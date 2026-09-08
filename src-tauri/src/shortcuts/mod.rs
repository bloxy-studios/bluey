//! Global shortcuts: registration through `tauri-plugin-global-shortcut`,
//! persistence (`ShortcutRepository`), conflict detection (`bluey_core::shortcuts`)
//! and the native side of every trigger (`shortcut.triggered` on the bus plus the
//! panel/audio actions Rust owns). Registration failures never crash the app —
//! they are logged, published as `dev.log` and reported via `check_conflict`.

use std::str::FromStr;
use std::sync::Arc;

use bluey_core::events::{BlueyEvent, ScrollDirection};
use bluey_core::shortcuts as rules;
use bluey_core::types::{ShortcutBinding, ShortcutConflict, ShortcutConflictKind, ShortcutId};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_protocols::panel::MoveDirection;
use bluey_storage::ShortcutRepository;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::audio::AudioManager;
use crate::events::EventBus;
use crate::overlay::PanelManager;
use crate::settings::SettingsManager;
use crate::storage::Storage;

/// Everything a trigger needs; cloned into each registered closure.
#[derive(Clone)]
struct TriggerContext {
    app: AppHandle,
    bus: Arc<EventBus>,
    panel: Arc<PanelManager>,
    audio: Arc<AudioManager>,
}

pub struct ShortcutManager {
    app: AppHandle,
    bus: Arc<EventBus>,
    storage: Arc<Storage>,
    settings: Arc<SettingsManager>,
    panel: Arc<PanelManager>,
    audio: Arc<AudioManager>,
    /// Accelerators that failed to register in the last `apply_bindings`.
    failed: parking_lot::Mutex<Vec<(ShortcutId, String)>>,
}

impl ShortcutManager {
    pub fn new(
        app: AppHandle,
        bus: Arc<EventBus>,
        storage: Arc<Storage>,
        settings: Arc<SettingsManager>,
        panel: Arc<PanelManager>,
        audio: Arc<AudioManager>,
    ) -> Self {
        Self {
            app,
            bus,
            storage,
            settings,
            panel,
            audio,
            failed: parking_lot::Mutex::new(Vec::new()),
        }
    }

    fn context(&self) -> TriggerContext {
        TriggerContext {
            app: self.app.clone(),
            bus: self.bus.clone(),
            panel: self.panel.clone(),
            audio: self.audio.clone(),
        }
    }

    /// Current bindings (defaults merged with stored overrides).
    pub fn list(&self) -> Vec<ShortcutBinding> {
        self.settings.get().shortcuts
    }

    /// Re-register every enabled binding. Individual failures (accelerator
    /// already taken by another app, unparsable string) are collected and
    /// reported, the rest keep working.
    pub async fn apply_bindings(&self, bindings: Vec<ShortcutBinding>) -> BlueyResult<()> {
        let shortcuts = self.app.global_shortcut();
        if let Err(e) = shortcuts.unregister_all() {
            tracing::warn!(error = %e, "failed to unregister shortcuts");
        }
        let mut failed = Vec::new();
        for binding in bindings.iter().filter(|b| b.enabled) {
            let shortcut = match Shortcut::from_str(&binding.accelerator) {
                Ok(shortcut) => shortcut,
                Err(e) => {
                    tracing::warn!(id = ?binding.id, error = %e, "invalid accelerator");
                    failed.push((binding.id, format!("invalid accelerator: {e}")));
                    continue;
                }
            };
            let id = binding.id;
            let ctx = self.context();
            let result = shortcuts.on_shortcut(shortcut, move |_app, _shortcut, event| {
                if matches!(event.state(), ShortcutState::Pressed) {
                    trigger(&ctx, id);
                }
            });
            if let Err(e) = result {
                tracing::warn!(id = ?binding.id, accelerator = %binding.accelerator, error = %e, "shortcut registration failed");
                failed.push((binding.id, format!("registration failed: {e}")));
            }
        }
        if !failed.is_empty() {
            let message = failed
                .iter()
                .map(|(id, reason)| format!("{}: {reason}", id_str(*id)))
                .collect::<Vec<_>>()
                .join("; ");
            self.bus.publish(BlueyEvent::DevLog {
                level: "warn".into(),
                target: "bluey::shortcuts".into(),
                message: format!("some shortcuts could not be registered — {message}"),
                at: now_iso(),
            });
        }
        *self.failed.lock() = failed;
        Ok(())
    }

    /// Change one binding: validates and normalises the accelerator, rejects
    /// clashes with other Bluey shortcuts, persists and re-registers.
    pub async fn update(
        &self,
        id: ShortcutId,
        accelerator: String,
        enabled: Option<bool>,
    ) -> BlueyResult<Vec<ShortcutBinding>> {
        let normalized = rules::normalize_accelerator(&accelerator).ok_or_else(|| {
            BlueyError::invalid_params(format!("`{accelerator}` is not a valid shortcut"))
        })?;
        let mut bindings = self.list();
        if let Some(conflict) = rules::detect_conflict(&normalized, &bindings, Some(id)) {
            if conflict.conflicts_with == ShortcutConflictKind::Bluey {
                let details = serde_json::to_value(&conflict).unwrap_or_default();
                return Err(BlueyError::invalid_params(conflict.detail).with_details(details));
            }
            tracing::warn!(accelerator = %normalized, detail = %conflict.detail, "shortcut shadows a system shortcut");
        }
        let binding = bindings
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| BlueyError::invalid_params("unknown shortcut id"))?;
        binding.accelerator = normalized;
        if let Some(enabled) = enabled {
            binding.enabled = enabled;
        }
        self.persist_and_apply(bindings).await
    }

    /// Restore the default bindings.
    pub async fn reset(&self) -> BlueyResult<Vec<ShortcutBinding>> {
        let defaults = self.storage.run(ShortcutRepository::reset).await?;
        self.settings.set_shortcuts(defaults.clone());
        self.apply_bindings(defaults.clone()).await?;
        Ok(defaults)
    }

    /// Conflict check for the keybind recorder. Registration failures of the
    /// current bindings are reported as `registration_failed`.
    pub fn check_conflict(
        &self,
        accelerator: &str,
        ignore: Option<ShortcutId>,
    ) -> Option<ShortcutConflict> {
        let bindings = self.list();
        if let Some(conflict) = rules::detect_conflict(accelerator, &bindings, ignore) {
            return Some(conflict);
        }
        let normalized = rules::normalize_accelerator(accelerator)?;
        let failed = self.failed.lock();
        let failed_here = failed.iter().find(|(id, _)| {
            Some(*id) != ignore
                && bindings
                    .iter()
                    .any(|b| b.id == *id && b.accelerator == normalized)
        });
        failed_here.map(|(_, reason)| ShortcutConflict {
            accelerator: normalized.clone(),
            conflicts_with: ShortcutConflictKind::RegistrationFailed,
            detail: reason.clone(),
        })
    }

    async fn persist_and_apply(
        &self,
        bindings: Vec<ShortcutBinding>,
    ) -> BlueyResult<Vec<ShortcutBinding>> {
        let reconciled = rules::reconcile(&bindings);
        let to_store = reconciled.clone();
        self.storage
            .run(move |db| ShortcutRepository::save_all(db, &to_store))
            .await?;
        self.settings.set_shortcuts(reconciled.clone());
        self.apply_bindings(reconciled.clone()).await?;
        Ok(reconciled)
    }
}

/// Native reaction to a shortcut press. Everything the frontend handles itself
/// (⌘↵ capture, ⌘⇧↵ prepared response) only gets the event.
fn trigger(ctx: &TriggerContext, id: ShortcutId) {
    ctx.bus
        .publish(BlueyEvent::ShortcutTriggered { id, at: now_iso() });
    let ctx = ctx.clone();
    tauri::async_runtime::spawn(async move {
        let result: BlueyResult<()> = match id {
            ShortcutId::TogglePanel => ctx.panel.toggle().await.map(|_| ()),
            ShortcutId::CaptureAnalyze | ShortcutId::GenerateResponse => {
                // Make sure the HUD is visible for the answer.
                ctx.panel.show().await.map(|_| ())
            }
            ShortcutId::ToggleListening => {
                if ctx.audio.is_running() {
                    ctx.audio.stop().await.map(|_| ())
                } else {
                    ctx.audio.start(None).await.map(|_| ())
                }
            }
            ShortcutId::NewChat => {
                ctx.bus.publish(BlueyEvent::PanelNewChat);
                ctx.panel.show().await.map(|_| ())
            }
            ShortcutId::OpenSettings => crate::platform::open_window(&ctx.app, "settings", None),
            ShortcutId::MoveUp => ctx
                .panel
                .move_step(MoveDirection::Up, None)
                .await
                .map(|_| ()),
            ShortcutId::MoveDown => ctx
                .panel
                .move_step(MoveDirection::Down, None)
                .await
                .map(|_| ()),
            ShortcutId::MoveLeft => ctx
                .panel
                .move_step(MoveDirection::Left, None)
                .await
                .map(|_| ()),
            ShortcutId::MoveRight => ctx
                .panel
                .move_step(MoveDirection::Right, None)
                .await
                .map(|_| ()),
            ShortcutId::ScrollUp => {
                ctx.bus.publish(BlueyEvent::PanelScroll {
                    direction: ScrollDirection::Up,
                });
                Ok(())
            }
            ShortcutId::ScrollDown => {
                ctx.bus.publish(BlueyEvent::PanelScroll {
                    direction: ScrollDirection::Down,
                });
                Ok(())
            }
        };
        if let Err(e) = result {
            tracing::warn!(shortcut = id_str(id), error = %e, "shortcut action failed");
        }
    });
}

fn id_str(id: ShortcutId) -> &'static str {
    match id {
        ShortcutId::TogglePanel => "toggle_panel",
        ShortcutId::CaptureAnalyze => "capture_analyze",
        ShortcutId::GenerateResponse => "generate_response",
        ShortcutId::ToggleListening => "toggle_listening",
        ShortcutId::NewChat => "new_chat",
        ShortcutId::OpenSettings => "open_settings",
        ShortcutId::MoveUp => "move_up",
        ShortcutId::MoveDown => "move_down",
        ShortcutId::MoveLeft => "move_left",
        ShortcutId::MoveRight => "move_right",
        ShortcutId::ScrollUp => "scroll_up",
        ShortcutId::ScrollDown => "scroll_down",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_accelerator_parses_for_the_plugin() {
        for binding in rules::default_bindings() {
            assert!(
                Shortcut::from_str(&binding.accelerator).is_ok(),
                "{} does not parse",
                binding.accelerator
            );
        }
    }

    #[test]
    fn id_strings_match_serde_tags() {
        for id in ShortcutId::ALL {
            let tag = serde_json::to_value(id).unwrap();
            assert_eq!(tag, id_str(id));
        }
    }
}
