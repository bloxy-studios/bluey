//! Mode manager: built-in seeding, CRUD (repo-backed), active/default mode
//! tracking, and `mode.changed`/`modes.changed` events.

use std::sync::Arc;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{AppEvent, BlueyMode, ModePatch, Settings};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_storage::{ModeRepository, SessionRepository, SettingsRepository};

use crate::events::EventBus;
use crate::settings::SettingsManager;
use crate::state::StateHub;
use crate::storage::Storage;

pub struct ModeManager {
    storage: Arc<Storage>,
    settings: Arc<SettingsManager>,
    bus: Arc<EventBus>,
    hub: Arc<StateHub>,
    active_id: parking_lot::Mutex<String>,
    /// Cache of the active mode for hot paths (router preferred role).
    active_cache: parking_lot::Mutex<Option<BlueyMode>>,
}

impl ModeManager {
    /// Seed built-ins and resolve the active mode (bootstrap, synchronous):
    /// Bluey launches in the default mode, except that a session being
    /// resumed keeps the mode it was running in. A mode that no longer
    /// exists falls back to the built-in `general`.
    pub fn load(
        storage: Arc<Storage>,
        settings: Arc<SettingsManager>,
        bus: Arc<EventBus>,
        hub: Arc<StateHub>,
    ) -> BlueyResult<Self> {
        let built_ins = bluey_core::modes::built_in_modes(&now_iso());
        storage.run_sync(|db| ModeRepository::seed_built_in(db, &built_ins))?;
        let stored_active = storage.run_sync(SettingsRepository::get_active_mode_id)?;
        let resuming_session = storage.run_sync(SessionRepository::get_active)?.is_some();
        let default_id = settings.get().general.default_mode_id;
        let preferred = match stored_active.clone() {
            Some(active) if resuming_session => active,
            _ => default_id,
        };
        let active_id = existing_or_general(&storage, preferred);
        if stored_active.as_deref() != Some(active_id.as_str()) {
            storage.run_sync(|db| SettingsRepository::set_active_mode_id(db, &active_id))?;
        }
        let manager = Self {
            storage,
            settings,
            bus,
            hub,
            active_id: parking_lot::Mutex::new(active_id),
            active_cache: parking_lot::Mutex::new(None),
        };
        manager.refresh_active_cache_sync();
        Ok(manager)
    }

    /// Current active mode id.
    pub fn active_id(&self) -> String {
        self.active_id.lock().clone()
    }

    /// Current active mode (cached; falls back to the built-in general mode).
    pub fn active_mode(&self) -> BlueyMode {
        if let Some(mode) = self.active_cache.lock().clone() {
            return mode;
        }
        bluey_core::modes::mode_by_id(&self.active_id())
            .or_else(|| bluey_core::modes::mode_by_id(bluey_core::types::DEFAULT_MODE_ID))
            .expect("built-in default mode exists")
    }

    fn refresh_active_cache_sync(&self) {
        let id = self.active_id();
        let mode = self
            .storage
            .run_sync(|db| ModeRepository::get(db, &id))
            .ok();
        *self.active_cache.lock() = mode;
    }

    async fn refresh_active_cache(&self) {
        let id = self.active_id();
        let mode = self
            .storage
            .run(move |db| ModeRepository::get(db, &id))
            .await
            .ok();
        *self.active_cache.lock() = mode;
    }

    pub async fn list(&self) -> BlueyResult<Vec<BlueyMode>> {
        self.storage.run(ModeRepository::list).await
    }

    pub async fn get(&self, id: String) -> BlueyResult<BlueyMode> {
        self.storage
            .run(move |db| ModeRepository::get(db, &id))
            .await
    }

    async fn publish_modes_changed(&self) -> BlueyResult<()> {
        let modes = self.list().await?;
        self.bus.publish(BlueyEvent::ModesChanged(modes));
        Ok(())
    }

    pub async fn create(&self, draft: ModePatch) -> BlueyResult<BlueyMode> {
        let mode = self
            .storage
            .run(move |db| ModeRepository::create(db, &draft))
            .await?;
        self.publish_modes_changed().await?;
        Ok(mode)
    }

    pub async fn update(&self, id: String, patch: ModePatch) -> BlueyResult<BlueyMode> {
        let update_id = id.clone();
        let mode = self
            .storage
            .run(move |db| ModeRepository::update(db, &update_id, &patch))
            .await?;
        if id == self.active_id() {
            self.refresh_active_cache().await;
        }
        self.publish_modes_changed().await?;
        Ok(mode)
    }

    pub async fn delete(&self, id: String) -> BlueyResult<()> {
        if bluey_core::modes::is_built_in(&id) {
            return Err(BlueyError::invalid_params(
                "built-in modes cannot be deleted",
            ));
        }
        // The default falls back to `general` when its mode goes away.
        let mut default_id = self.settings.get().general.default_mode_id;
        if default_id == id {
            default_id = bluey_core::types::DEFAULT_MODE_ID.to_string();
            self.settings.set_default_mode(&default_id).await?;
        }
        if id == self.active_id() {
            // Switch to the default mode (when it still exists) first.
            let fallback = if self.get(default_id.clone()).await.is_ok() {
                default_id
            } else {
                bluey_core::types::DEFAULT_MODE_ID.to_string()
            };
            self.set_active(fallback).await?;
        }
        let delete_id = id.clone();
        self.storage
            .run(move |db| ModeRepository::delete(db, &delete_id))
            .await?;
        self.publish_modes_changed().await?;
        Ok(())
    }

    pub async fn duplicate(&self, id: String) -> BlueyResult<BlueyMode> {
        let mode = self
            .storage
            .run(move |db| ModeRepository::duplicate(db, &id))
            .await?;
        self.publish_modes_changed().await?;
        Ok(mode)
    }

    pub async fn reset_built_in(&self, id: String) -> BlueyResult<BlueyMode> {
        let reference = bluey_core::modes::mode_by_id(&id)
            .ok_or_else(|| BlueyError::invalid_params("not a built-in mode"))?;
        let reset_id = id.clone();
        let mode = self
            .storage
            .run(move |db| ModeRepository::reset_built_in(db, &reset_id, &reference))
            .await?;
        if id == self.active_id() {
            self.refresh_active_cache().await;
        }
        self.publish_modes_changed().await?;
        Ok(mode)
    }

    /// Make `id` the default mode. With no session running Bluey also
    /// switches to it now (a running session keeps its mode; the default
    /// applies from the next launch).
    pub async fn set_default(&self, id: String) -> BlueyResult<Settings> {
        self.get(id.clone()).await?;
        let settings = self.settings.set_default_mode(&id).await?;
        if self.hub.status().session_id.is_none() && self.active_id() != id {
            self.set_active(id).await?;
        }
        Ok(settings)
    }

    /// Switch the active mode: persists, updates the state machine and
    /// publishes `mode.changed`. Returns the new app status.
    pub async fn set_active(&self, id: String) -> BlueyResult<bluey_core::types::AppStatus> {
        let fetch_id = id.clone();
        let mode = self
            .storage
            .run(move |db| ModeRepository::get(db, &fetch_id))
            .await?;
        let persist_id = id.clone();
        self.storage
            .run(move |db| SettingsRepository::set_active_mode_id(db, &persist_id))
            .await?;
        *self.active_id.lock() = id.clone();
        *self.active_cache.lock() = Some(mode.clone());
        let status = self.hub.transition(AppEvent::ModeChanged { mode_id: id })?;
        self.bus.publish(BlueyEvent::ModeChanged {
            mode: mode.clone(),
            session_id: status.session_id.clone(),
        });
        Ok(status)
    }
}

/// `id` when that mode exists, else the built-in `general`.
fn existing_or_general(storage: &Storage, id: String) -> String {
    match storage.run_sync(|db| ModeRepository::get(db, &id)) {
        Ok(_) => id,
        Err(_) => bluey_core::types::DEFAULT_MODE_ID.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SecretsStore;
    use crate::storage::AppPaths;
    use bluey_core::types::DEFAULT_MODE_ID;

    /// A throwaway data directory with the managers `ModeManager` needs.
    struct Harness {
        dir: std::path::PathBuf,
        storage: Arc<Storage>,
        settings: Arc<SettingsManager>,
        bus: Arc<EventBus>,
        hub: Arc<StateHub>,
    }

    impl Harness {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(bluey_core::new_id("bluey-modes-test"));
            std::fs::create_dir_all(&dir).unwrap();
            let paths = Arc::new(AppPaths {
                db_path: dir.join("bluey.db"),
                frames_dir: dir.join("frames"),
                logs_dir: dir.join("logs"),
                data_dir: dir.clone(),
            });
            let storage = Arc::new(Storage::open(paths).unwrap());
            let bus = Arc::new(EventBus::new());
            let secrets = Arc::new(SecretsStore::new());
            let settings =
                Arc::new(SettingsManager::load(storage.clone(), secrets, bus.clone()).unwrap());
            let hub = Arc::new(StateHub::new(DEFAULT_MODE_ID, bus.clone()));
            Self {
                dir,
                storage,
                settings,
                bus,
                hub,
            }
        }

        /// A freshly launched manager over the same data.
        fn launch(&self) -> ModeManager {
            ModeManager::load(
                self.storage.clone(),
                self.settings.clone(),
                self.bus.clone(),
                self.hub.clone(),
            )
            .unwrap()
        }

        fn default_mode_id(&self) -> String {
            self.settings.get().general.default_mode_id
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn custom_mode(modes: &ModeManager, name: &str) -> String {
        let draft = ModePatch {
            name: Some(name.into()),
            ..Default::default()
        };
        modes.create(draft).await.unwrap().id
    }

    #[tokio::test]
    async fn setting_the_default_switches_to_it_when_no_session_runs() {
        let h = Harness::new();
        let modes = h.launch();
        let pitch = custom_mode(&modes, "Pitch").await;

        let settings = modes.set_default(pitch.clone()).await.unwrap();

        assert_eq!(settings.general.default_mode_id, pitch);
        assert_eq!(modes.active_id(), pitch);
        assert_eq!(h.hub.status().mode_id, pitch);
    }

    #[tokio::test]
    async fn a_running_session_keeps_its_mode_when_the_default_changes() {
        let h = Harness::new();
        let modes = h.launch();
        let pitch = custom_mode(&modes, "Pitch").await;
        h.hub.transition_soft(AppEvent::SessionChanged {
            session_id: Some("ses_running".into()),
        });

        modes.set_default(pitch.clone()).await.unwrap();

        assert_eq!(h.default_mode_id(), pitch);
        assert_eq!(modes.active_id(), DEFAULT_MODE_ID);
    }

    #[tokio::test]
    async fn launch_uses_the_default_mode_unless_a_session_is_resumed() {
        let h = Harness::new();
        let modes = h.launch();
        let pitch = custom_mode(&modes, "Pitch").await;
        let notes = custom_mode(&modes, "Notes").await;
        modes.set_default(pitch.clone()).await.unwrap();
        modes.set_active(notes.clone()).await.unwrap();

        assert_eq!(h.launch().active_id(), pitch);

        // A session left active resumes in the mode it was running in.
        modes.set_active(notes.clone()).await.unwrap();
        let mode_id = notes.clone();
        h.storage
            .run_sync(|db| SessionRepository::create(db, &mode_id, None))
            .unwrap();
        assert_eq!(h.launch().active_id(), notes);
    }

    #[tokio::test]
    async fn deleting_the_default_mode_resets_default_and_active_to_general() {
        let h = Harness::new();
        let modes = h.launch();
        let pitch = custom_mode(&modes, "Pitch").await;
        modes.set_default(pitch.clone()).await.unwrap();

        modes.delete(pitch).await.unwrap();

        assert_eq!(h.default_mode_id(), DEFAULT_MODE_ID);
        assert_eq!(modes.active_id(), DEFAULT_MODE_ID);
        assert_eq!(h.launch().active_id(), DEFAULT_MODE_ID);
    }

    #[tokio::test]
    async fn deleting_the_active_mode_after_its_default_was_deleted_succeeds() {
        let h = Harness::new();
        let modes = h.launch();
        let a = custom_mode(&modes, "A").await;
        let b = custom_mode(&modes, "B").await;
        modes.set_default(a.clone()).await.unwrap();
        modes.set_active(b.clone()).await.unwrap();

        modes.delete(a).await.unwrap();
        assert_eq!(h.default_mode_id(), DEFAULT_MODE_ID);
        modes.delete(b).await.unwrap();

        assert_eq!(modes.active_id(), DEFAULT_MODE_ID);
    }

    #[tokio::test]
    async fn a_default_that_no_longer_exists_launches_in_general() {
        let h = Harness::new();
        h.settings
            .update(serde_json::json!({ "general": { "defaultModeId": "mode_gone" } }))
            .await
            .unwrap();

        assert_eq!(h.launch().active_id(), DEFAULT_MODE_ID);
    }
}
