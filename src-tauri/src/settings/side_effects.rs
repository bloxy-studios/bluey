//! Side effects of settings changes: shortcut re-registration, panel
//! appearance, content protection, autostart, observation mode, log level and
//! retention enforcement.

use bluey_core::error::{BlueyError, RecoveryAction};
use bluey_core::events::BlueyEvent;
use bluey_core::types::{DisplayMode, Settings};
use tauri::State;

use crate::events::EventBus;
use crate::state::AppCore;

/// Apply every observable difference between `old` and `new`. Errors are
/// never propagated — the settings write already succeeded — but the ones the
/// user must act on are published as `app.error` (see [`report`]).
pub async fn apply(core: &State<'_, AppCore>, old: &Settings, new: &Settings) {
    // Shortcuts: bindings changed → persist + re-register.
    if old.shortcuts != new.shortcuts {
        if let Err(e) = core.shortcuts.apply_bindings(new.shortcuts.clone()).await {
            report(&core.bus, "keybinds", e);
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
            report(&core.bus, "privacy", e);
        }
    }

    // Privacy: Cloud AI off → live transcription leaves the cloud route.
    if old.privacy.cloud_ai_enabled && !new.privacy.cloud_ai_enabled {
        core.audio
            .fall_back_to_apple(crate::audio::CLOUD_AI_OFF_REASON);
    }

    // Launch at login.
    if old.general.launch_at_login != new.general.launch_at_login {
        let app = core.panel.app_handle();
        if let Err(e) = crate::platform::set_autostart(&app, new.general.launch_at_login) {
            report(&core.bus, "general", e);
        }
    }

    // In-app updates: a new channel re-checks; automatic on installs a waiting update.
    if old.updates != new.updates {
        core.updates.on_settings_changed(&old.updates, &new.updates);
    }

    // Screen observation: Smart has no consumer yet (FEATURE-002), so no
    // setting starts the sampling stream.

    // Embedding model / size changed → vectors from the old space are stale.
    let embedding_changed = old.ai.models.embedding != new.ai.models.embedding
        || old.ai.embedding_dimensions != new.ai.embedding_dimensions
        || (!old.ai.embeddings_enabled && new.ai.embeddings_enabled);
    if embedding_changed {
        let documents = core.documents.clone();
        tauri::async_runtime::spawn(async move {
            match documents.reembed_stale().await {
                Ok(count) if count > 0 => {
                    tracing::info!(count, "documents re-embedded after settings change")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "re-embedding after settings change failed"),
            }
        });
    }

    // A removed provider takes its API key with it (FEATURE-001) — an
    // attribute-only delete, so no Keychain prompt.
    for provider_id in removed_provider_ids(old, new) {
        let key = crate::secrets::provider_key(&provider_id);
        if let Err(e) = core.secrets.delete(&key).await {
            tracing::warn!(error = %e, "failed to delete a removed provider's API key");
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
        let bus = core.bus.clone();
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
                Err(e) => report(&bus, "privacy", e),
            }
        });
    }
}

/// A side effect the user must act on failed (UX-037): publish it as
/// `app.error`, whose toast action opens the settings tab that controls it —
/// unless the error already carries its own recovery (a permission error
/// opens System Settings). Only the code is logged.
fn report(bus: &EventBus, tab: &str, error: BlueyError) {
    tracing::warn!(code = %error.code, tab, "settings side effect failed");
    bus.publish(BlueyEvent::AppError(with_settings_recovery(error, tab)));
}

fn with_settings_recovery(error: BlueyError, tab: &str) -> BlueyError {
    match error.recovery {
        Some(RecoveryAction::None) | None => {
            error.recoverable(RecoveryAction::OpenSettings { tab: tab.into() })
        }
        Some(_) => error,
    }
}

/// Providers present in `old` but gone from `new`.
fn removed_provider_ids(old: &Settings, new: &Settings) -> Vec<String> {
    old.ai
        .providers
        .iter()
        .filter(|p| !new.ai.providers.iter().any(|n| n.id == p.id))
        .map(|p| p.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use bluey_core::types::{AiProviderConfig, AiProviderKind, PermissionKind};

    fn provider(id: &str) -> AiProviderConfig {
        AiProviderConfig {
            id: id.into(),
            kind: AiProviderKind::OpenaiCompatible,
            name: id.into(),
            base_url: "https://example.invalid".into(),
            api_version: None,
            deployments: None,
            enabled: true,
            has_api_key: true,
            auth_method: Default::default(),
        }
    }

    #[test]
    fn a_provider_dropped_from_settings_is_reported_for_key_cleanup() {
        let mut old = Settings::default();
        old.ai.providers = vec![provider("gemini"), provider("custom-1")];
        let mut new = old.clone();
        new.ai.providers.retain(|p| p.id == "gemini");
        assert_eq!(
            removed_provider_ids(&old, &new),
            vec!["custom-1".to_string()]
        );
        assert!(removed_provider_ids(&old, &old).is_empty());
    }

    #[test]
    fn a_failed_side_effect_is_published_with_a_way_to_fix_it() {
        let bus = EventBus::new();
        let mut events = bus.subscribe();

        report(
            &bus,
            "privacy",
            BlueyError::storage("sweep_failed", "disk I/O error"),
        );
        let Ok(BlueyEvent::AppError(error)) = events.try_recv() else {
            panic!("expected an app.error event");
        };
        assert_eq!(error.code, "storage.sweep_failed");
        assert_eq!(
            error.recovery,
            Some(RecoveryAction::OpenSettings {
                tab: "privacy".into()
            })
        );

        // A permission error keeps its own System Settings action.
        let denied = BlueyError::permission(PermissionKind::ScreenRecording, "not granted");
        report(&bus, "screen", denied.clone());
        let Ok(BlueyEvent::AppError(error)) = events.try_recv() else {
            panic!("expected an app.error event");
        };
        assert_eq!(error.recovery, denied.recovery);
    }
}
