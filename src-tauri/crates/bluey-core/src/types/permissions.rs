use serde::{Deserialize, Serialize};

use crate::error::RecoveryAction;

/// Mirrors `PermissionKind` (camelCase strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionKind {
    Microphone,
    ScreenRecording,
    Accessibility,
    Notifications,
    SpeechRecognition,
}

impl PermissionKind {
    pub const ALL: [PermissionKind; 5] = [
        PermissionKind::Microphone,
        PermissionKind::ScreenRecording,
        PermissionKind::Accessibility,
        PermissionKind::Notifications,
        PermissionKind::SpeechRecognition,
    ];

    /// snake_case suffix used in error codes.
    pub fn code_suffix(&self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::ScreenRecording => "screen_recording",
            Self::Accessibility => "accessibility",
            Self::Notifications => "notifications",
            Self::SpeechRecognition => "speech_recognition",
        }
    }

    /// macOS System Settings deep link for the privacy pane.
    pub fn system_settings_url(&self) -> &'static str {
        match self {
            Self::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            Self::ScreenRecording => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            Self::Accessibility => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            Self::Notifications => "x-apple.systempreferences:com.apple.preference.notifications",
            Self::SpeechRecognition => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_SpeechRecognition"
            }
        }
    }
}

/// Mirrors `PermissionStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionStatus {
    Granted,
    Denied,
    NotDetermined,
    Restricted,
    #[default]
    Unknown,
}

impl PermissionStatus {
    pub fn is_granted(&self) -> bool {
        matches!(self, PermissionStatus::Granted)
    }
}

/// Mirrors `PermissionState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionState {
    pub microphone: PermissionStatus,
    pub screen_recording: PermissionStatus,
    pub accessibility: PermissionStatus,
    pub notifications: PermissionStatus,
    pub speech_recognition: PermissionStatus,
    pub checked_at: String,
}

impl PermissionState {
    pub fn unknown(checked_at: String) -> Self {
        Self {
            microphone: PermissionStatus::Unknown,
            screen_recording: PermissionStatus::Unknown,
            accessibility: PermissionStatus::Unknown,
            notifications: PermissionStatus::Unknown,
            speech_recognition: PermissionStatus::Unknown,
            checked_at,
        }
    }

    pub fn get(&self, kind: PermissionKind) -> PermissionStatus {
        match kind {
            PermissionKind::Microphone => self.microphone,
            PermissionKind::ScreenRecording => self.screen_recording,
            PermissionKind::Accessibility => self.accessibility,
            PermissionKind::Notifications => self.notifications,
            PermissionKind::SpeechRecognition => self.speech_recognition,
        }
    }

    pub fn set(&mut self, kind: PermissionKind, status: PermissionStatus) {
        match kind {
            PermissionKind::Microphone => self.microphone = status,
            PermissionKind::ScreenRecording => self.screen_recording = status,
            PermissionKind::Accessibility => self.accessibility = status,
            PermissionKind::Notifications => self.notifications = status,
            PermissionKind::SpeechRecognition => self.speech_recognition = status,
        }
    }
}

/// Mirrors `CaptureProtection`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureProtection {
    pub supported: bool,
    pub enabled: bool,
    pub note: String,
}

/// Mirrors `SetupCheck`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupCheck {
    pub id: SetupCheckId,
    pub label: String,
    pub ok: bool,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SetupCheckId {
    Screen,
    Microphone,
    Accessibility,
    Ai,
    Helper,
    SystemAudio,
}
