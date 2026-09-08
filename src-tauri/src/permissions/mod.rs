//! Permission manager: Screen Recording (`CGPreflightScreenCaptureAccess`, in
//! Rust so TCC attributes it to Bluey.app), Accessibility (`AXIsProcessTrusted`,
//! Rust), Microphone + Speech Recognition (helper `permissions.status` /
//! `permissions.request`) and Notifications (`tauri-plugin-notification`).
//! Every refresh that changes something publishes `permissions.changed`.

use std::sync::Arc;
use std::time::Duration;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{PermissionKind, PermissionState, PermissionStatus};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_protocols::helper::{permission_wire_kind, WirePermissions};
use serde_json::json;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::events::EventBus;
use crate::sidecar::HelperClient;
use crate::state::StateHub;

/// How often permissions are re-checked while listening (spec: 30 s).
const ACTIVE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

pub struct PermissionManager {
    app: AppHandle,
    helper: Arc<HelperClient>,
    bus: Arc<EventBus>,
    hub: Arc<StateHub>,
    last: parking_lot::Mutex<Option<PermissionState>>,
}

impl PermissionManager {
    pub fn new(
        app: AppHandle,
        helper: Arc<HelperClient>,
        bus: Arc<EventBus>,
        hub: Arc<StateHub>,
    ) -> Self {
        Self {
            app,
            helper,
            bus,
            hub,
            last: parking_lot::Mutex::new(None),
        }
    }

    /// The last observed state without touching the system (may be `None`
    /// before the first refresh).
    pub fn cached(&self) -> Option<PermissionState> {
        self.last.lock().clone()
    }

    /// Re-check every permission and publish `permissions.changed` when the
    /// result differs from the previous snapshot.
    pub async fn refresh(&self) -> BlueyResult<PermissionState> {
        let mut state = PermissionState::unknown(now_iso());
        state.screen_recording = screen_recording_status();
        state.accessibility = accessibility_status();
        state.notifications = self.notification_status();

        // Microphone + speech live in the helper (AVCaptureDevice / SFSpeechRecognizer).
        match self.helper.call("permissions.status", json!({})).await {
            Ok(value) => {
                let wire: WirePermissions = serde_json::from_value(value).unwrap_or_default();
                state.microphone = wire.microphone.into();
                state.speech_recognition = wire.speech_recognition.into();
                // The helper's own view of Screen Recording / Accessibility is
                // only used when the in-process check could not run.
                if state.screen_recording == PermissionStatus::Unknown {
                    state.screen_recording = wire.screen_recording.into();
                }
                if state.accessibility == PermissionStatus::Unknown {
                    state.accessibility = wire.accessibility.into();
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "helper permission status unavailable");
            }
        }

        self.store(state.clone());
        Ok(state)
    }

    /// Request one permission (shows the system prompt where macOS allows it)
    /// and return the refreshed state.
    pub async fn request(&self, kind: PermissionKind) -> BlueyResult<PermissionState> {
        match kind {
            PermissionKind::ScreenRecording => {
                // Runs in-process so the TCC entry is attributed to Bluey.app.
                let _ = tokio::task::spawn_blocking(request_screen_recording).await;
            }
            PermissionKind::Notifications => {
                let _ = self.app.notification().request_permission();
            }
            PermissionKind::Microphone
            | PermissionKind::Accessibility
            | PermissionKind::SpeechRecognition => {
                let wire = permission_wire_kind(kind)
                    .ok_or_else(|| BlueyError::internal("permission kind has no helper mapping"))?;
                let result = self
                    .helper
                    .call("permissions.request", json!({ "kind": wire }))
                    .await;
                if let Err(e) = result {
                    tracing::warn!(kind = wire, error = %e, "permission request via helper failed");
                }
            }
        }
        self.refresh().await
    }

    /// Open the System Settings pane for `kind`.
    pub fn open_settings(&self, kind: PermissionKind) -> BlueyResult<()> {
        self.app
            .opener()
            .open_url(kind.system_settings_url(), None::<&str>)
            .map_err(|e| BlueyError::internal(format!("cannot open System Settings: {e}")))
    }

    /// Whether `kind` is currently granted according to the last snapshot.
    pub fn is_granted(&self, kind: PermissionKind) -> bool {
        self.cached()
            .map(|s| s.get(kind).is_granted())
            .unwrap_or(false)
    }

    fn notification_status(&self) -> PermissionStatus {
        use tauri_plugin_notification::PermissionState as N;
        match self.app.notification().permission_state() {
            Ok(N::Granted) => PermissionStatus::Granted,
            Ok(N::Denied) => PermissionStatus::Denied,
            Ok(_) => PermissionStatus::NotDetermined,
            Err(_) => PermissionStatus::Unknown,
        }
    }

    fn store(&self, state: PermissionState) {
        let changed = {
            let mut last = self.last.lock();
            let changed = match last.as_ref() {
                Some(previous) => !same_statuses(previous, &state),
                None => true,
            };
            *last = Some(state.clone());
            changed
        };
        if changed {
            self.bus.publish(BlueyEvent::PermissionsChanged(state));
        }
    }

    /// Periodic refresh while an audio session is active (revocations must stop
    /// dependent subsystems quickly). Runs for the lifetime of the app.
    pub fn spawn_refresh_loop(self: &Arc<Self>) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(ACTIVE_REFRESH_INTERVAL);
            loop {
                interval.tick().await;
                if this.hub.audio_active() {
                    if let Err(e) = this.refresh().await {
                        tracing::debug!(error = %e, "periodic permission refresh failed");
                    }
                }
            }
        });
    }
}

/// Compare every status field (ignoring `checked_at`).
fn same_statuses(a: &PermissionState, b: &PermissionState) -> bool {
    PermissionKind::ALL
        .iter()
        .all(|kind| a.get(*kind) == b.get(*kind))
}

#[cfg(target_os = "macos")]
fn screen_recording_status() -> PermissionStatus {
    if core_graphics::access::ScreenCaptureAccess.preflight() {
        PermissionStatus::Granted
    } else {
        // macOS does not distinguish "never asked" from "denied" here.
        PermissionStatus::Denied
    }
}

#[cfg(not(target_os = "macos"))]
fn screen_recording_status() -> PermissionStatus {
    PermissionStatus::Unknown
}

#[cfg(target_os = "macos")]
fn request_screen_recording() -> bool {
    core_graphics::access::ScreenCaptureAccess.request()
}

#[cfg(not(target_os = "macos"))]
fn request_screen_recording() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn accessibility_status() -> PermissionStatus {
    // SAFETY: plain C call without arguments; returns a Boolean.
    let trusted = unsafe { accessibility_sys::AXIsProcessTrusted() };
    if trusted {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    }
}

#[cfg(not(target_os = "macos"))]
fn accessibility_status() -> PermissionStatus {
    PermissionStatus::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_protocols::helper::WireStatus;

    #[test]
    fn same_statuses_ignores_timestamp() {
        let mut a = PermissionState::unknown("t1".into());
        a.microphone = PermissionStatus::Granted;
        let mut b = a.clone();
        b.checked_at = "t2".into();
        assert!(same_statuses(&a, &b));
        b.microphone = PermissionStatus::Denied;
        assert!(!same_statuses(&a, &b));
    }

    #[test]
    fn wire_status_maps_onto_contract_status() {
        assert_eq!(
            PermissionStatus::from(WireStatus::Granted),
            PermissionStatus::Granted
        );
        assert_eq!(
            PermissionStatus::from(WireStatus::Unknown),
            PermissionStatus::Unknown
        );
    }
}
