//! Typed error contract. Serialized shape mirrors `src/lib/types/errors.ts`:
//! `{ kind, code, message, recoverable, recovery?, details? }`.

use serde::{Deserialize, Serialize};

use crate::types::permissions::PermissionKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueyErrorKind {
    Permission,
    Capture,
    Audio,
    Transcription,
    Ai,
    Storage,
    Authentication,
    Configuration,
    Sidecar,
    Network,
    Research,
    Cancelled,
    NotSupported,
    Internal,
}

impl BlueyErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Permission => "permission",
            Self::Capture => "capture",
            Self::Audio => "audio",
            Self::Transcription => "transcription",
            Self::Ai => "ai",
            Self::Storage => "storage",
            Self::Authentication => "authentication",
            Self::Configuration => "configuration",
            Self::Sidecar => "sidecar",
            Self::Network => "network",
            Self::Research => "research",
            Self::Cancelled => "cancelled",
            Self::NotSupported => "not_supported",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecoveryAction {
    OpenSystemSettings { pane: PermissionKind },
    OpenSettings { tab: String },
    Retry,
    SignIn,
    RestartHelper,
    ConfigureProvider,
    None,
}

/// The single error type returned by every Bluey subsystem.
///
/// `message` must never contain secrets, transcript text, screenshots or resume content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("[{}] {code}: {message}", kind.as_str())]
pub struct BlueyError {
    pub kind: BlueyErrorKind,
    pub code: String,
    pub message: String,
    pub recoverable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

pub type BlueyResult<T> = Result<T, BlueyError>;

impl BlueyError {
    pub fn new(kind: BlueyErrorKind, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
            recoverable: false,
            recovery: None,
            details: None,
        }
    }

    pub fn recoverable(mut self, recovery: RecoveryAction) -> Self {
        self.recoverable = true;
        self.recovery = Some(recovery);
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    // ── Constructors for the common cases ──────────────────────────────────

    pub fn permission(kind: PermissionKind, message: impl Into<String>) -> Self {
        Self::new(
            BlueyErrorKind::Permission,
            format!("permission.{}", kind.code_suffix()),
            message,
        )
        .recoverable(RecoveryAction::OpenSystemSettings { pane: kind })
    }

    pub fn capture(code: &str, message: impl Into<String>) -> Self {
        Self::new(BlueyErrorKind::Capture, format!("capture.{code}"), message)
    }

    pub fn audio(code: &str, message: impl Into<String>) -> Self {
        Self::new(BlueyErrorKind::Audio, format!("audio.{code}"), message)
    }

    pub fn transcription(code: &str, message: impl Into<String>) -> Self {
        Self::new(
            BlueyErrorKind::Transcription,
            format!("transcription.{code}"),
            message,
        )
    }

    pub fn ai(code: &str, message: impl Into<String>) -> Self {
        Self::new(BlueyErrorKind::Ai, format!("ai.{code}"), message)
    }

    pub fn storage(code: &str, message: impl Into<String>) -> Self {
        Self::new(BlueyErrorKind::Storage, format!("storage.{code}"), message)
    }

    pub fn authentication(code: &str, message: impl Into<String>) -> Self {
        Self::new(
            BlueyErrorKind::Authentication,
            format!("auth.{code}"),
            message,
        )
        .recoverable(RecoveryAction::SignIn)
    }

    pub fn configuration(code: &str, message: impl Into<String>) -> Self {
        Self::new(
            BlueyErrorKind::Configuration,
            format!("config.{code}"),
            message,
        )
        .recoverable(RecoveryAction::OpenSettings { tab: "ai".into() })
    }

    pub fn sidecar(code: &str, message: impl Into<String>) -> Self {
        Self::new(BlueyErrorKind::Sidecar, format!("sidecar.{code}"), message)
            .recoverable(RecoveryAction::RestartHelper)
    }

    pub fn network(code: &str, message: impl Into<String>) -> Self {
        Self::new(BlueyErrorKind::Network, format!("network.{code}"), message)
            .recoverable(RecoveryAction::Retry)
    }

    pub fn research(code: &str, message: impl Into<String>) -> Self {
        Self::new(
            BlueyErrorKind::Research,
            format!("research.{code}"),
            message,
        )
    }

    pub fn cancelled() -> Self {
        Self::new(
            BlueyErrorKind::Cancelled,
            "cancelled",
            "The operation was cancelled",
        )
    }

    pub fn not_supported(code: &str, message: impl Into<String>) -> Self {
        Self::new(
            BlueyErrorKind::NotSupported,
            format!("not_supported.{code}"),
            message,
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(BlueyErrorKind::Internal, "internal.unexpected", message)
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(BlueyErrorKind::Internal, "internal.invalid_params", message)
    }

    pub fn is_cancelled(&self) -> bool {
        self.kind == BlueyErrorKind::Cancelled
    }
}

impl From<serde_json::Error> for BlueyError {
    fn from(e: serde_json::Error) -> Self {
        BlueyError::internal(format!("serialization error: {e}"))
    }
}

impl From<std::io::Error> for BlueyError {
    fn from(e: std::io::Error) -> Self {
        BlueyError::new(BlueyErrorKind::Internal, "internal.io", e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_frontend_shape() {
        let e = BlueyError::permission(
            PermissionKind::ScreenRecording,
            "Screen Recording not granted",
        );
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["kind"], "permission");
        assert_eq!(json["code"], "permission.screen_recording");
        assert_eq!(json["recoverable"], true);
        assert_eq!(json["recovery"]["type"], "open_system_settings");
        assert_eq!(json["recovery"]["pane"], "screenRecording");
    }

    #[test]
    fn round_trips() {
        let e = BlueyError::ai("timeout", "took too long")
            .with_details(serde_json::json!({"ms": 3000}));
        let back: BlueyError = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(e, back);
    }
}
