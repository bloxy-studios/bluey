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
    /// Grants an earlier Bluey version had that macOS no longer reports since
    /// this version launched (MAC-001). Only filled in the first run after an
    /// update; a kind drops out as soon as it is granted again.
    #[serde(default)]
    pub lost_after_update: Vec<PermissionKind>,
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
            lost_after_update: Vec::new(),
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

/// Privacy grants TCC ties to the app's code identity. An update signed with
/// a different identity (every ad-hoc build) no longer matches them (MAC-001).
/// Notifications are keyed by bundle id and survive.
pub const IDENTITY_BOUND_PERMISSIONS: [PermissionKind; 4] = [
    PermissionKind::Microphone,
    PermissionKind::ScreenRecording,
    PermissionKind::Accessibility,
    PermissionKind::SpeechRecognition,
];

/// The last-known granted permissions and the Bluey version that saw them,
/// persisted so the first launch after an update can tell which grants the
/// update cost (MAC-001).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PermissionSnapshot {
    pub version: String,
    pub granted: Vec<PermissionKind>,
}

impl PermissionSnapshot {
    /// Identity-bound grants this snapshot had that `current` reports as off,
    /// when it was taken by another version. `Unknown` (the helper is not up
    /// yet) never counts as lost.
    pub fn lost_after_update(
        &self,
        version: &str,
        current: &PermissionState,
    ) -> Vec<PermissionKind> {
        if self.version == version {
            return Vec::new();
        }
        IDENTITY_BOUND_PERMISSIONS
            .into_iter()
            .filter(|kind| self.granted.contains(kind))
            .filter(|kind| {
                matches!(
                    current.get(*kind),
                    PermissionStatus::Denied
                        | PermissionStatus::NotDetermined
                        | PermissionStatus::Restricted
                )
            })
            .collect()
    }

    /// The snapshot to persist after observing `current` in `version`: a
    /// kind that could not be checked keeps its previous entry.
    pub fn observe(&self, version: &str, current: &PermissionState) -> Self {
        let granted = PermissionKind::ALL
            .into_iter()
            .filter(|kind| match current.get(*kind) {
                PermissionStatus::Granted => true,
                PermissionStatus::Unknown => self.granted.contains(kind),
                _ => false,
            })
            .collect();
        Self {
            version: version.to_string(),
            granted,
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

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    fn state(granted: &[PermissionKind], unknown: &[PermissionKind]) -> PermissionState {
        let mut s = PermissionState::unknown("t".into());
        for kind in PermissionKind::ALL {
            let status = if granted.contains(&kind) {
                PermissionStatus::Granted
            } else if unknown.contains(&kind) {
                PermissionStatus::Unknown
            } else {
                PermissionStatus::Denied
            };
            s.set(kind, status);
        }
        s
    }

    fn snapshot(version: &str, granted: &[PermissionKind]) -> PermissionSnapshot {
        PermissionSnapshot {
            version: version.into(),
            granted: granted.to_vec(),
        }
    }

    #[test]
    fn an_update_that_drops_grants_reports_them() {
        use PermissionKind::*;
        let before = snapshot("0.1.0", &[ScreenRecording, Accessibility, Microphone]);
        let now = state(&[Microphone], &[]);
        assert_eq!(
            before.lost_after_update("0.1.1", &now),
            vec![ScreenRecording, Accessibility]
        );
    }

    #[test]
    fn same_version_unknown_and_notifications_are_not_lost() {
        use PermissionKind::*;
        let before = snapshot("0.1.0", &[ScreenRecording, Microphone, Notifications]);
        // Same version: the user revoked it on purpose, no update involved.
        assert!(before
            .lost_after_update("0.1.0", &state(&[], &[]))
            .is_empty());
        // Microphone unknown (helper not up yet), notifications survive updates.
        assert_eq!(
            before.lost_after_update("0.2.0", &state(&[ScreenRecording], &[Microphone])),
            Vec::<PermissionKind>::new()
        );
    }

    #[test]
    fn observe_records_the_version_and_keeps_unchecked_grants() {
        use PermissionKind::*;
        let before = snapshot("0.1.0", &[Microphone, Accessibility]);
        let next = before.observe("0.2.0", &state(&[ScreenRecording], &[Microphone]));
        assert_eq!(next, snapshot("0.2.0", &[Microphone, ScreenRecording]));
    }
}
