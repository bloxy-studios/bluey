use serde::{Deserialize, Serialize};

use crate::error::BlueyError;

/// Mirrors `AppState` in `src/lib/types/app-state.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppState {
    Booting,
    AuthRequired,
    Ready,
    Listening,
    Capturing,
    Analyzing,
    Thinking,
    ResponseReady,
    Error,
    Paused,
}

impl AppState {
    pub fn is_idle(&self) -> bool {
        matches!(self, AppState::Ready | AppState::Listening)
    }
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            AppState::Capturing | AppState::Analyzing | AppState::Thinking
        )
    }
}

/// Mirrors `AppStatus`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub state: AppState,
    pub audio_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub mode_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<BlueyError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_state: Option<AppState>,
    pub updated_at: String,
}

/// Mirrors `AppStateEvent` (tag = "type").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    BootCompleted {
        authenticated: bool,
    },
    Authenticated,
    SignedOut,
    AudioStarted,
    AudioStopped,
    CaptureStarted,
    CaptureFinished,
    AnalysisStarted,
    ThinkingStarted,
    ResponseReady,
    ResponseDismissed,
    Failed {
        error: BlueyError,
    },
    Recovered,
    Paused,
    Resumed,
    #[serde(rename_all = "camelCase")]
    SessionChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ModeChanged {
        mode_id: String,
    },
}
