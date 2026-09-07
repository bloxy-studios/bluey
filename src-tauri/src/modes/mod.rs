//! Mode manager: built-in seeding, CRUD (repo-backed), active/default mode
//! tracking, and `mode.changed`/`modes.changed` events.

use std::sync::Arc;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{AppEvent, BlueyMode, ModePatch};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_storage::{ModeRepository, SettingsRepository};

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
    /// Seed built-ins and resolve the active mode (bootstrap, synchronous).
    pub fn load(
        storage: Arc<Storage>,
        settings: Arc<SettingsManager>,
        bus: Arc<EventBus>,
        hub: Arc<StateHub>,
    ) -> BlueyResult<Self> {
        let built_ins = bluey_core::modes::built_in_modes(&now_iso());
        storage.run_sync(|db| ModeRepository::seed_built_in(db, &built_ins))?;
        let stored_active = storage.run_sync(SettingsRepository::get_active_mode_id)?;
        let default_id = settings.get().general.default_mode_id;
        let active_id = stored_active.unwrap_or(default_id);
        // Fall back to the built-in default when the stored mode vanished.
        let active_id = match storage.run_sync(|db| ModeRepository::get(db, &active_id)) {
            Ok(_) => active_id,
            Err(_) => bluey_core::types::DEFAULT_MODE_ID.to_string(),
        };
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
            return Err(BlueyError::invalid_params("built-in modes cannot be deleted"));
        }
        if id == self.active_id() {
            // Switch to the default mode before deleting the active one.
            let fallback = self.settings.get().general.default_mode_id;
            let fallback = if fallback == id {
                bluey_core::types::DEFAULT_MODE_ID.to_string()
            } else {
                fallback
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
