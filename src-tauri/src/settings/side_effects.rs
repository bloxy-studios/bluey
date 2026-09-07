//! Side effects of settings changes: shortcut re-registration, panel
//! appearance, content protection, autostart, observation mode, log level and
//! retention enforcement.

use bluey_core::types::{DisplayMode, ObservationMode, Settings};
use tauri::State;

use crate::state::AppCore;

/// Apply every observable difference between `old` and `new`. Errors are
/// logged, never propagated — the settings write already succeeded.
pub async fn apply(core: &State<'_, AppCore>, old: &Settings, new: &Settings) {
    // Shortcuts: bindings changed → persist + re-register.
    if old.shortcuts != new.shortcuts {
        if let Err(e) = core.shortcuts.apply_bindings(new.shortcuts.clone()).await {
            tracing::warn!(error = %e, "failed to re-register shortcuts");
        }
    }

    // Panel appearance.
    if (old.appearance.opacity - new.appearance.opacity).abs() > f32::EPSILON {
        let _ = core.panel.set_opacity(new.appearance.opacity).await;
    }
    if old.appearance.width != new.appearance.width {
        let _ = core.panel.apply_width(new.appearance.width as f64).await;
    }
    if old.appearance.always_on_top != new.appearance.always_on_top {
        core.panel.set_always_on_top(new.appearance.always_on_top);
    }

    // Privacy: display mode → content protection on every Bluey window.
    if old.privacy.display_mode != new.privacy.display_mode {
        let enabled = new.privacy.display_mode == DisplayMode::Privacy;
        if let Err(e) = core.capture.set_protection(enabled) {
            tracing::warn!(error = %e, "failed to toggle content protection");
        }
    }

    // Launch at login.
    if old.general.launch_at_login != new.general.launch_at_login {
        crate::platform::set_autostart(&core.panel.app_handle(), new.general.launch_at_login);
    }

    // Observation mode.
    if old.screen.observation != new.screen.observation
        || old.screen.observation_interval_ms != new.screen.observation_interval_ms
    {
        match new.screen.observation {
            ObservationMode::Smart => {
                let _ = core
                    .capture
                    .observe_start(Some(new.screen.observation_interval_ms), None)
                    .await;
            }
            ObservationMode::Manual => {
                let _ = core.capture.observe_stop().await;
            }
        }
    }

    // Log level.
    if old.advanced.log_level != new.advanced.log_level {
        crate::app::set_log_level(new.advanced.log_level.as_str());
    }

    // Helper restart policy.
    core.helper
        .set_restart_on_crash(new.advanced.helper_restart_on_crash);

    // Retention: privacy toggles turned off → enforce on stored data.
    let retention_tightened = (!new.privacy.store_screenshots && old.privacy.store_screenshots)
        || (!new.privacy.store_transcripts && old.privacy.store_transcripts)
        || (!new.privacy.store_session_history && old.privacy.store_session_history);
    if retention_tightened {
        let privacy = new.privacy.clone();
        let storage = core.storage.clone();
        tauri::async_runtime::spawn(async move {
            let result = storage
                .run(move |db| bluey_storage::apply_retention(db, &privacy))
                .await;
            match result {
                Ok(report) => {
                    crate::storage::Storage::remove_files(&report.image_paths);
                    tracing::info!(
                        sessions = report.sessions_deleted,
                        screenshots = report.screenshots_deleted,
                        transcripts = report.transcripts_deleted,
                        "retention enforced after settings change"
                    );
                }
                Err(e) => tracing::warn!(error = %e, "retention sweep failed"),
            }
        });
    }
}
