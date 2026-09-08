//! Setup checks (onboarding / Settings → Permissions) and developer info.

use bluey_core::error::RecoveryAction;
use bluey_core::types::{
    BuildProfile, DevInfo, PermissionKind, PermissionState, PermissionStatus, SetupCheck,
    SetupCheckId,
};
use tauri::AppHandle;

use crate::agent::AgentManager;
use crate::state::AppCore;

/// Run every setup check. Permission checks refresh the live state; the AI
/// check is configuration-only (no network) so the wizard stays fast.
pub async fn setup_checks(core: &AppCore) -> Vec<SetupCheck> {
    let permissions = core
        .permissions
        .refresh()
        .await
        .unwrap_or_else(|_| PermissionState::unknown(bluey_core::now_iso()));
    let helper_running = core.helper.is_running();
    let helper_version = core.helper.version();
    let system_audio_capable = helper_version
        .as_ref()
        .map(|v| v.has_capability("audio.system"))
        .unwrap_or(false);
    let providers = core.ai.providers();
    let has_provider = providers.iter().any(|p| p.enabled && p.has_api_key);
    let has_default_model = core.settings.get().ai.models.default.is_some();

    let mut checks = vec![
        permission_check(
            SetupCheckId::Screen,
            "Screen recording",
            PermissionKind::ScreenRecording,
            permissions.screen_recording,
            "Bluey can capture your screen.",
            "Screen Recording permission is not granted.",
        ),
        permission_check(
            SetupCheckId::Microphone,
            "Microphone",
            PermissionKind::Microphone,
            permissions.microphone,
            "Microphone access is granted.",
            "Microphone permission is not granted.",
        ),
        permission_check(
            SetupCheckId::Accessibility,
            "Accessibility",
            PermissionKind::Accessibility,
            permissions.accessibility,
            "Bluey can read on-screen structure.",
            "Accessibility permission is not granted.",
        ),
        SetupCheck {
            id: SetupCheckId::Ai,
            label: "AI provider".into(),
            ok: has_provider && has_default_model,
            detail: match (has_provider, has_default_model) {
                (true, true) => "A provider with a stored key serves the default model.".into(),
                (true, false) => {
                    "A provider is configured but no default model is assigned.".into()
                }
                _ => "No enabled provider has an API key.".into(),
            },
            fix: Some("Add a provider key and assign models in Settings → AI.".into()),
            recovery: Some(RecoveryAction::OpenSettings { tab: "ai".into() }),
        },
        SetupCheck {
            id: SetupCheckId::Helper,
            label: "Native helper".into(),
            ok: helper_running,
            detail: match &helper_version {
                Some(v) if helper_running => format!("Helper {} is running.", v.version),
                _ => "The native helper is not running.".into(),
            },
            fix: (!helper_running).then(|| "Restart the helper from Settings → Advanced.".into()),
            recovery: (!helper_running).then_some(RecoveryAction::RestartHelper),
        },
    ];
    let system_audio_ok = system_audio_capable && permissions.screen_recording.is_granted();
    checks.push(SetupCheck {
        id: SetupCheckId::SystemAudio,
        label: "System audio".into(),
        ok: system_audio_ok,
        detail: if system_audio_ok {
            "System audio can be captured alongside the microphone.".into()
        } else if !system_audio_capable {
            "The helper does not report system-audio capture.".into()
        } else {
            "System audio needs Screen Recording permission.".into()
        },
        fix: (!system_audio_ok)
            .then(|| "Grant Screen Recording; system audio is part of it.".into()),
        recovery: (!system_audio_ok).then_some(RecoveryAction::OpenSystemSettings {
            pane: PermissionKind::ScreenRecording,
        }),
    });
    checks
}

fn permission_check(
    id: SetupCheckId,
    label: &str,
    kind: PermissionKind,
    status: PermissionStatus,
    ok_detail: &str,
    missing_detail: &str,
) -> SetupCheck {
    let ok = status.is_granted();
    SetupCheck {
        id,
        label: label.into(),
        ok,
        detail: if ok {
            ok_detail.into()
        } else {
            missing_detail.into()
        },
        fix: (!ok).then(|| format!("Grant {label} in System Settings → Privacy & Security.")),
        recovery: (!ok).then_some(RecoveryAction::OpenSystemSettings { pane: kind }),
    }
}

/// Information for the developer overlay / About tab.
pub fn dev_info(core: &AppCore, app: &AppHandle) -> DevInfo {
    DevInfo {
        version: app.package_info().version.to_string(),
        build_profile: if cfg!(debug_assertions) {
            BuildProfile::Debug
        } else {
            BuildProfile::Release
        },
        helper_version: core.helper.version().map(|v| v.version),
        helper_running: core.helper.is_running(),
        agent_sidecar_available: AgentManager::binary_path().is_some(),
        db_path: core.paths.db_path.to_string_lossy().into_owned(),
        log_path: core.paths.logs_dir.to_string_lossy().into_owned(),
        mock_transport: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_check_shapes_fix_and_recovery() {
        let granted = permission_check(
            SetupCheckId::Microphone,
            "Microphone",
            PermissionKind::Microphone,
            PermissionStatus::Granted,
            "ok",
            "missing",
        );
        assert!(granted.ok);
        assert_eq!(granted.detail, "ok");
        assert!(granted.fix.is_none() && granted.recovery.is_none());

        let denied = permission_check(
            SetupCheckId::Screen,
            "Screen recording",
            PermissionKind::ScreenRecording,
            PermissionStatus::Denied,
            "ok",
            "missing",
        );
        assert!(!denied.ok);
        assert_eq!(
            denied.recovery,
            Some(RecoveryAction::OpenSystemSettings {
                pane: PermissionKind::ScreenRecording
            })
        );
        assert!(denied.fix.unwrap().contains("Screen recording"));
    }
}
