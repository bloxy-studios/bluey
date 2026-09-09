//! The central event enum: one variant per entry in the frontend contract
//! `src/lib/tauri/events.ts`. Rust code publishes [`BlueyEvent`]s through an
//! [`EventSink`]; the Tauri layer forwards them as `emit(tauri_event_name(),
//! payload())`. Payload shapes mirror the TypeScript `EventMap` exactly
//! (camelCase fields, same nesting).

use serde::{Deserialize, Serialize};

use crate::error::BlueyError;
use crate::types::{
    AccessibilityContext, AiChunk, AiTask, AppStatus, AudioDevice, AudioSource, AudioStatus,
    AuthStatus, BlueyMode, BlueyResponse, ContextSnapshot, DeepResearchEvent, DetectedEvent,
    LatencyMetrics, OcrContext, PanelState, PermissionState, ScreenFrame, Session, SessionEvent,
    Settings, ShortcutId, TranscriptSegment,
};

/// Prefix every Tauri event name carries (`bluey:` + dotted name).
pub const EVENT_PREFIX: &str = "bluey:";

/// Scroll direction for `panel.scroll`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrollDirection {
    Up,
    Down,
}

/// Every event Bluey can emit, with its payload. Serialization (untagged)
/// yields exactly the payload the frontend expects for [`BlueyEvent::name`].
//
// Events are transient bus values that are serialized and dropped, so the size
// difference between unit variants and snapshot-carrying variants is fine.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum BlueyEvent {
    // ── app ─────────────────────────────────────────────────────────────────
    /// `app.state`
    AppState(AppStatus),
    /// `app.error`
    AppError(BlueyError),
    /// `settings.changed`
    SettingsChanged(Settings),
    /// `permissions.changed`
    PermissionsChanged(PermissionState),
    /// Sign-in state changed (browser sign-in started / finished, sign-out).
    AuthChanged(AuthStatus),
    /// `helper.status`
    #[serde(rename_all = "camelCase")]
    HelperStatus {
        running: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        restarted: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<BlueyError>,
    },

    // ── screen ──────────────────────────────────────────────────────────────
    /// `screen.changed`
    #[serde(rename_all = "camelCase")]
    ScreenChanged {
        hash: String,
        /// Fraction of the screen that changed, 0.0–1.0.
        delta: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        display_id: Option<String>,
        at: String,
    },
    /// `screen.captured`
    ScreenCaptured(ScreenFrame),
    /// `ocr.completed`
    OcrCompleted(OcrContext),
    /// `accessibility.updated`
    AccessibilityUpdated(AccessibilityContext),
    /// `activeApp.changed`
    #[serde(rename_all = "camelCase")]
    ActiveAppChanged {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        bundle_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pid: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        window_title: Option<String>,
    },

    // ── audio / transcript ──────────────────────────────────────────────────
    /// `audio.started`
    AudioStarted(AudioStatus),
    /// `audio.stopped`
    AudioStopped(AudioStatus),
    /// `audio.paused`
    AudioPaused(AudioStatus),
    /// `audio.resumed`
    AudioResumed(AudioStatus),
    /// `audio.level`
    AudioLevel { microphone: f32, system: f32 },
    /// `audio.chunk`
    #[serde(rename_all = "camelCase")]
    AudioChunk {
        source: AudioSource,
        start_ms: u64,
        end_ms: u64,
        is_speech: bool,
        rms: f32,
    },
    /// `audio.deviceChanged`
    #[serde(rename_all = "camelCase")]
    AudioDeviceChanged {
        devices: Vec<AudioDevice>,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_input: Option<AudioDevice>,
    },
    /// `audio.error`
    AudioError(BlueyError),
    /// `transcript.partial`
    TranscriptPartial(TranscriptSegment),
    /// `transcript.final`
    TranscriptFinal(TranscriptSegment),
    /// `transcript.cleared`
    #[serde(rename_all = "camelCase")]
    TranscriptCleared {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },

    // ── intelligence ────────────────────────────────────────────────────────
    /// `question.detected`
    QuestionDetected(DetectedEvent),
    /// `context.updated`
    ContextUpdated {
        snapshot: ContextSnapshot,
        reason: String,
    },
    /// `response.prepared`
    ResponsePrepared(BlueyResponse),

    // ── ai ──────────────────────────────────────────────────────────────────
    /// `ai.requested`
    #[serde(rename_all = "camelCase")]
    AiRequested {
        request_id: String,
        task: AiTask,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// `ai.started`
    #[serde(rename_all = "camelCase")]
    AiStarted {
        request_id: String,
        provider: String,
        model: String,
    },
    /// `ai.chunk`
    AiChunk(AiChunk),
    /// `ai.completed`
    #[serde(rename_all = "camelCase")]
    AiCompleted {
        request_id: String,
        total_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        time_to_first_token_ms: Option<u64>,
    },
    /// `ai.failed`
    #[serde(rename_all = "camelCase")]
    AiFailed {
        request_id: String,
        error: BlueyError,
    },
    /// `ai.cancelled`
    #[serde(rename_all = "camelCase")]
    AiCancelled { request_id: String },

    // ── research ────────────────────────────────────────────────────────────
    /// `research.event`
    ResearchEvent(DeepResearchEvent),

    // ── sessions & modes ────────────────────────────────────────────────────
    /// `session.started`
    SessionStarted(Session),
    /// `session.paused`
    SessionPaused(Session),
    /// `session.resumed`
    SessionResumed(Session),
    /// `session.ended`
    SessionEnded(Session),
    /// `session.event`
    SessionEvent(SessionEvent),
    /// `mode.changed`
    #[serde(rename_all = "camelCase")]
    ModeChanged {
        mode: BlueyMode,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// `modes.changed`
    ModesChanged(Vec<BlueyMode>),

    // ── shortcuts / panel ───────────────────────────────────────────────────
    /// `shortcut.triggered`
    ShortcutTriggered { id: ShortcutId, at: String },
    /// `panel.state`
    PanelState(PanelState),
    /// `panel.scroll`
    PanelScroll { direction: ScrollDirection },
    /// `panel.focusInput` (empty payload)
    PanelFocusInput,
    /// `panel.newChat` (empty payload)
    PanelNewChat,

    // ── dev ─────────────────────────────────────────────────────────────────
    /// `dev.metrics`
    DevMetrics(LatencyMetrics),
    /// `dev.log`
    DevLog {
        level: String,
        target: String,
        message: String,
        at: String,
    },
}

impl BlueyEvent {
    /// Dotted event name exactly as in `events.ts` (e.g. `"transcript.final"`).
    pub fn name(&self) -> &'static str {
        match self {
            Self::AppState(_) => "app.state",
            Self::AppError(_) => "app.error",
            Self::SettingsChanged(_) => "settings.changed",
            Self::PermissionsChanged(_) => "permissions.changed",
            Self::AuthChanged(_) => "auth.changed",
            Self::HelperStatus { .. } => "helper.status",
            Self::ScreenChanged { .. } => "screen.changed",
            Self::ScreenCaptured(_) => "screen.captured",
            Self::OcrCompleted(_) => "ocr.completed",
            Self::AccessibilityUpdated(_) => "accessibility.updated",
            Self::ActiveAppChanged { .. } => "activeApp.changed",
            Self::AudioStarted(_) => "audio.started",
            Self::AudioStopped(_) => "audio.stopped",
            Self::AudioPaused(_) => "audio.paused",
            Self::AudioResumed(_) => "audio.resumed",
            Self::AudioLevel { .. } => "audio.level",
            Self::AudioChunk { .. } => "audio.chunk",
            Self::AudioDeviceChanged { .. } => "audio.deviceChanged",
            Self::AudioError(_) => "audio.error",
            Self::TranscriptPartial(_) => "transcript.partial",
            Self::TranscriptFinal(_) => "transcript.final",
            Self::TranscriptCleared { .. } => "transcript.cleared",
            Self::QuestionDetected(_) => "question.detected",
            Self::ContextUpdated { .. } => "context.updated",
            Self::ResponsePrepared(_) => "response.prepared",
            Self::AiRequested { .. } => "ai.requested",
            Self::AiStarted { .. } => "ai.started",
            Self::AiChunk(_) => "ai.chunk",
            Self::AiCompleted { .. } => "ai.completed",
            Self::AiFailed { .. } => "ai.failed",
            Self::AiCancelled { .. } => "ai.cancelled",
            Self::ResearchEvent(_) => "research.event",
            Self::SessionStarted(_) => "session.started",
            Self::SessionPaused(_) => "session.paused",
            Self::SessionResumed(_) => "session.resumed",
            Self::SessionEnded(_) => "session.ended",
            Self::SessionEvent(_) => "session.event",
            Self::ModeChanged { .. } => "mode.changed",
            Self::ModesChanged(_) => "modes.changed",
            Self::ShortcutTriggered { .. } => "shortcut.triggered",
            Self::PanelState(_) => "panel.state",
            Self::PanelScroll { .. } => "panel.scroll",
            Self::PanelFocusInput => "panel.focusInput",
            Self::PanelNewChat => "panel.newChat",
            Self::DevMetrics(_) => "dev.metrics",
            Self::DevLog { .. } => "dev.log",
        }
    }

    /// Full Tauri event name: `"bluey:" + name()`.
    pub fn tauri_event_name(&self) -> String {
        format!("{EVENT_PREFIX}{}", self.name())
    }

    /// JSON payload in the exact shape the frontend expects (camelCase).
    /// Events with an empty payload (`panel.focusInput`, `panel.newChat`)
    /// serialize as `{}`.
    pub fn payload(&self) -> serde_json::Value {
        match self {
            Self::PanelFocusInput | Self::PanelNewChat => {
                serde_json::Value::Object(serde_json::Map::new())
            }
            _ => serde_json::to_value(self).unwrap_or(serde_json::Value::Null),
        }
    }
}

/// Anything that can deliver [`BlueyEvent`]s (the Tauri emitter in the app,
/// recording/null sinks in tests). Implementations must be cheap and must not
/// block the caller.
pub trait EventSink: Send + Sync {
    /// Publish one event.
    fn publish(&self, event: BlueyEvent);
}

/// Sink that drops every event. Useful default for headless code paths.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullSink;

impl EventSink for NullSink {
    fn publish(&self, _event: BlueyEvent) {}
}

/// Sink that records every published event for assertions in tests.
#[derive(Debug, Default)]
pub struct RecordingSink {
    events: std::sync::Mutex<Vec<BlueyEvent>>,
}

impl RecordingSink {
    /// New empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, Vec<BlueyEvent>> {
        // A panic while holding the lock only poisons test state; recover it.
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Snapshot of everything published so far.
    pub fn events(&self) -> Vec<BlueyEvent> {
        self.guard().clone()
    }

    /// Drain and return everything published so far.
    pub fn take(&self) -> Vec<BlueyEvent> {
        std::mem::take(&mut *self.guard())
    }

    /// Names (dotted) of everything published so far, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.guard().iter().map(BlueyEvent::name).collect()
    }
}

impl EventSink for RecordingSink {
    fn publish(&self, event: BlueyEvent) {
        self.guard().push(event);
    }
}

#[cfg(test)]
mod tests;
