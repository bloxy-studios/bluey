//! The explicit application state machine — the single owner of "what is the
//! app doing right now". No scattered booleans: the pipeline state lives in
//! [`AppState`], and `audio_active` is the one orthogonal region (listening can
//! be on or off independently of the capture→think pipeline).
//!
//! Rules implemented here (spec):
//! * Pipeline: idle → `CaptureStarted` → Capturing → `CaptureFinished` /
//!   `AnalysisStarted` → Analyzing → `ThinkingStarted` → Thinking →
//!   `ResponseReady` → ResponseReady → `ResponseDismissed` → idle.
//!   `ThinkingStarted` is also allowed straight from idle or ResponseReady
//!   (typed question without a capture), and `CaptureStarted` from ResponseReady
//!   (⌘↵ follow-up while a response is shown). `ResponseDismissed` additionally
//!   cancels out of Capturing/Analyzing/Thinking (Escape while busy).
//! * "idle" means Listening when `audio_active`, otherwise Ready.
//! * `AudioStarted`/`AudioStopped` toggle `audio_active` and only move the
//!   state between Ready and Listening; during a busy state they just flip the
//!   flag, so the pipeline returns to the right idle state afterwards.
//! * `Failed` → Error (remembering the idle state to resume to), `Recovered` →
//!   back to idle. `Paused`/`Resumed` likewise. Pausing keeps `audio_active`
//!   as-is: the app layer stops the audio engine itself without telling the
//!   machine, so resuming can go straight back to Listening.
//! * `SignedOut` clears the session and turns audio off.
//! * `SessionChanged`/`ModeChanged` update fields in any state without a state
//!   change.

use serde::{Deserialize, Serialize};

use crate::error::{BlueyError, BlueyErrorKind};
use crate::now_iso;
use crate::types::app_state::{AppEvent, AppState, AppStatus};

/// A rejected transition: `event` is the snake_case event tag, `from` the state
/// it was rejected in. Never contains user content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("invalid transition: event `{event}` is not allowed in state `{from:?}`")]
pub struct TransitionError {
    /// State the machine was in when the event was rejected.
    pub from: AppState,
    /// snake_case tag of the rejected event (e.g. `capture_started`).
    pub event: String,
}

impl From<TransitionError> for BlueyError {
    fn from(e: TransitionError) -> Self {
        let from = state_name(e.from);
        BlueyError::new(
            BlueyErrorKind::Internal,
            "state.invalid_transition",
            format!("event `{}` is not allowed in state `{from}`", e.event),
        )
        .with_details(serde_json::json!({ "from": from, "event": e.event }))
    }
}

/// snake_case name of a state, matching the serde representation.
pub fn state_name(state: AppState) -> &'static str {
    match state {
        AppState::Booting => "booting",
        AppState::AuthRequired => "auth_required",
        AppState::Ready => "ready",
        AppState::Listening => "listening",
        AppState::Capturing => "capturing",
        AppState::Analyzing => "analyzing",
        AppState::Thinking => "thinking",
        AppState::ResponseReady => "response_ready",
        AppState::Error => "error",
        AppState::Paused => "paused",
    }
}

/// snake_case tag of an event, matching the serde representation.
pub fn event_name(event: &AppEvent) -> &'static str {
    match event {
        AppEvent::BootCompleted { .. } => "boot_completed",
        AppEvent::Authenticated => "authenticated",
        AppEvent::SignedOut => "signed_out",
        AppEvent::AudioStarted => "audio_started",
        AppEvent::AudioStopped => "audio_stopped",
        AppEvent::CaptureStarted => "capture_started",
        AppEvent::CaptureFinished => "capture_finished",
        AppEvent::AnalysisStarted => "analysis_started",
        AppEvent::ThinkingStarted => "thinking_started",
        AppEvent::ResponseReady => "response_ready",
        AppEvent::ResponseDismissed => "response_dismissed",
        AppEvent::Failed { .. } => "failed",
        AppEvent::Recovered => "recovered",
        AppEvent::Paused => "paused",
        AppEvent::Resumed => "resumed",
        AppEvent::SessionChanged { .. } => "session_changed",
        AppEvent::ModeChanged { .. } => "mode_changed",
    }
}

/// The application state machine. Owns an [`AppStatus`] and mutates it only
/// through [`AppStateMachine::transition`], so every state change is validated,
/// timestamped and observable.
#[derive(Debug, Clone, PartialEq)]
pub struct AppStateMachine {
    status: AppStatus,
}

impl AppStateMachine {
    /// New machine in `Booting` with audio off and no session.
    pub fn new(mode_id: impl Into<String>) -> Self {
        Self {
            status: AppStatus {
                state: AppState::Booting,
                audio_active: false,
                session_id: None,
                mode_id: mode_id.into(),
                error: None,
                resume_state: None,
                updated_at: now_iso(),
            },
        }
    }

    /// Current status (state + orthogonal fields).
    pub fn status(&self) -> &AppStatus {
        &self.status
    }

    /// Current pipeline state.
    pub fn state(&self) -> AppState {
        self.status.state
    }

    /// Whether the audio/listening region is active.
    pub fn audio_active(&self) -> bool {
        self.status.audio_active
    }

    /// The idle state this machine returns to when a cycle ends:
    /// `Listening` when audio is active, `Ready` otherwise.
    pub fn idle_state(&self) -> AppState {
        if self.status.audio_active {
            AppState::Listening
        } else {
            AppState::Ready
        }
    }

    /// Apply `event`. On success returns a clone of the updated status; on a
    /// rejected transition returns a [`TransitionError`] and leaves the status
    /// untouched.
    pub fn transition(&mut self, event: AppEvent) -> Result<AppStatus, TransitionError> {
        use AppState as S;
        let from = self.status.state;
        let name = event_name(&event);
        let reject = || TransitionError {
            from,
            event: name.to_string(),
        };

        match event {
            // ── Field updates, allowed in any state ─────────────────────────
            AppEvent::SessionChanged { session_id } => {
                self.status.session_id = session_id;
            }
            AppEvent::ModeChanged { mode_id } => {
                self.status.mode_id = mode_id;
            }

            // ── Boot & auth ─────────────────────────────────────────────────
            AppEvent::BootCompleted { authenticated } => {
                if from != S::Booting {
                    return Err(reject());
                }
                self.status.state = if authenticated {
                    S::Ready
                } else {
                    S::AuthRequired
                };
            }
            AppEvent::Authenticated => {
                if from != S::AuthRequired {
                    return Err(reject());
                }
                self.status.state = S::Ready;
            }
            AppEvent::SignedOut => {
                if from == S::Booting {
                    return Err(reject());
                }
                self.status.state = S::AuthRequired;
                self.status.session_id = None;
                self.status.audio_active = false;
                self.status.error = None;
                self.status.resume_state = None;
            }

            // ── Orthogonal audio region ─────────────────────────────────────
            AppEvent::AudioStarted => {
                if !audio_toggle_allowed(from) {
                    return Err(reject());
                }
                self.status.audio_active = true;
                if from == S::Ready {
                    self.status.state = S::Listening;
                }
            }
            AppEvent::AudioStopped => {
                if !audio_toggle_allowed(from) {
                    return Err(reject());
                }
                self.status.audio_active = false;
                if from == S::Listening {
                    self.status.state = S::Ready;
                }
            }

            // ── Capture → analyze → think pipeline ──────────────────────────
            AppEvent::CaptureStarted => {
                // A new capture may start from idle or straight from a shown response
                // (⌘↵ follow-up) without bouncing through idle first.
                if !(from.is_idle() || from == S::ResponseReady) {
                    return Err(reject());
                }
                self.status.state = S::Capturing;
            }
            AppEvent::CaptureFinished | AppEvent::AnalysisStarted => {
                if from != S::Capturing {
                    return Err(reject());
                }
                self.status.state = S::Analyzing;
            }
            AppEvent::ThinkingStarted => {
                if !(from.is_idle() || from == S::Analyzing || from == S::ResponseReady) {
                    return Err(reject());
                }
                self.status.state = S::Thinking;
            }
            AppEvent::ResponseReady => {
                if from != S::Thinking {
                    return Err(reject());
                }
                self.status.state = S::ResponseReady;
            }
            AppEvent::ResponseDismissed => {
                if !matches!(
                    from,
                    S::Capturing | S::Analyzing | S::Thinking | S::ResponseReady
                ) {
                    return Err(reject());
                }
                self.status.state = self.idle_state();
            }

            // ── Error & pause regions ───────────────────────────────────────
            AppEvent::Failed { error } => {
                if matches!(from, S::Booting | S::AuthRequired) {
                    return Err(reject());
                }
                self.status.resume_state = Some(self.idle_state());
                self.status.error = Some(error);
                self.status.state = S::Error;
            }
            AppEvent::Recovered => {
                if from != S::Error {
                    return Err(reject());
                }
                self.status.state = self.idle_state();
                self.status.error = None;
                self.status.resume_state = None;
            }
            AppEvent::Paused => {
                if matches!(from, S::Booting | S::AuthRequired | S::Paused) {
                    return Err(reject());
                }
                self.status.resume_state = Some(self.idle_state());
                self.status.error = None;
                self.status.state = S::Paused;
            }
            AppEvent::Resumed => {
                if from != S::Paused {
                    return Err(reject());
                }
                self.status.state = self.idle_state();
                self.status.resume_state = None;
            }
        }

        self.status.updated_at = now_iso();
        Ok(self.status.clone())
    }
}

/// Audio start/stop is meaningful while the app runs normally; it is rejected
/// during boot/auth and while paused or errored (the app layer manages audio
/// hardware itself in those states).
fn audio_toggle_allowed(state: AppState) -> bool {
    matches!(
        state,
        AppState::Ready
            | AppState::Listening
            | AppState::Capturing
            | AppState::Analyzing
            | AppState::Thinking
            | AppState::ResponseReady
    )
}

#[cfg(test)]
mod tests;
