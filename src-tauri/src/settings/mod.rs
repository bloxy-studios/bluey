//! Settings: load/merge on boot, deep-partial updates, persistence split
//! across the settings blob (`SettingsRepository`), provider configs
//! (`ModelConfigRepository`, `has_api_key` filled from the keychain) and
//! shortcuts (`ShortcutRepository`). Side effects of changes are applied by
//! [`side_effects::apply`].

pub mod side_effects;

use std::sync::Arc;

use bluey_core::events::BlueyEvent;
use bluey_core::types::Settings;
use bluey_core::BlueyResult;
use bluey_storage::{ModelConfigRepository, SettingsRepository, ShortcutRepository};

use crate::events::EventBus;
use crate::secrets::{provider_key, SecretsStore};
use crate::storage::Storage;

/// In-memory settings with persistence.
pub struct SettingsManager {
    storage: Arc<Storage>,
    secrets: Arc<SecretsStore>,
    bus: Arc<EventBus>,
    current: parking_lot::RwLock<Settings>,
}

impl SettingsManager {
    /// Load composed settings synchronously (bootstrap).
    pub fn load(
        storage: Arc<Storage>,
        secrets: Arc<SecretsStore>,
        bus: Arc<EventBus>,
    ) -> BlueyResult<Self> {
        let mut settings = storage.run_sync(SettingsRepository::get)?;
        let mut providers = storage.run_sync(ModelConfigRepository::list)?;
        for provider in &mut providers {
            // Boot, before the runtime serves requests: memory (the presence
            // enumeration), else an attribute-only lookup that never prompts.
            let key = provider_key(&provider.id);
            provider.has_api_key = secrets
                .known_presence(&key)
                .unwrap_or_else(|| secrets.has_sync(&key).unwrap_or(false));
        }
        settings.ai.providers = providers;
        settings.shortcuts = storage.run_sync(ShortcutRepository::list)?;
        Ok(Self {
            storage,
            secrets,
            bus,
            current: parking_lot::RwLock::new(settings),
        })
    }

    /// Current settings snapshot.
    pub fn get(&self) -> Settings {
        self.current.read().clone()
    }

    /// Apply a deep-partial JSON patch, persist, publish `settings.changed`
    /// and return `(old, new)` so the caller can apply side effects.
    pub async fn update(&self, patch: serde_json::Value) -> BlueyResult<(Settings, Settings)> {
        let old = self.get();
        let mut new = old
            .apply_patch(&patch)
            .map_err(|e| bluey_core::BlueyError::invalid_params(format!("invalid patch: {e}")))?;
        validate(&new)?;
        // Normalise shortcuts (drop unknown ids, fix accelerators).
        new.shortcuts = bluey_core::shortcuts::reconcile(&new.shortcuts);
        self.persist(&new).await?;
        self.replace(new.clone());
        Ok((old, new))
    }

    /// Synchronous variant of [`Self::update`] for the bootstrap path (the
    /// `.env` import runs before any async context exists).
    pub fn update_sync(&self, patch: serde_json::Value) -> BlueyResult<Settings> {
        let old = self.get();
        let mut new = old
            .apply_patch(&patch)
            .map_err(|e| bluey_core::BlueyError::invalid_params(format!("invalid patch: {e}")))?;
        validate(&new)?;
        new.shortcuts = bluey_core::shortcuts::reconcile(&new.shortcuts);
        let mut blob = new.clone();
        let providers = std::mem::take(&mut blob.ai.providers);
        let shortcuts = blob.shortcuts.clone();
        let old_ids: Vec<String> = old.ai.providers.iter().map(|p| p.id.clone()).collect();
        let new_ids: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
        self.storage.run_sync(move |db| {
            SettingsRepository::save(db, &blob)?;
            for provider in &providers {
                ModelConfigRepository::upsert(db, provider)?;
            }
            for stale in old_ids.iter().filter(|id| !new_ids.contains(id)) {
                ModelConfigRepository::delete(db, stale)?;
            }
            ShortcutRepository::save_all(db, &shortcuts)?;
            Ok(())
        })?;
        self.replace(new.clone());
        Ok(new)
    }

    /// Reset everything to defaults (providers and shortcuts included),
    /// publish `settings.changed` and return `(old, new)`.
    pub async fn reset(&self) -> BlueyResult<(Settings, Settings)> {
        let old = self.get();
        let new = Settings::default();
        self.persist(&new).await?;
        // Delete providers that no longer exist.
        let stale: Vec<String> = old.ai.providers.iter().map(|p| p.id.clone()).collect();
        self.storage
            .run(move |db| {
                for id in &stale {
                    ModelConfigRepository::delete(db, id)?;
                }
                ShortcutRepository::reset(db)?;
                Ok(())
            })
            .await?;
        self.replace(new.clone());
        Ok((old, new))
    }

    /// Persist a settings value (blob w/o providers + provider table + shortcuts).
    async fn persist(&self, settings: &Settings) -> BlueyResult<()> {
        let mut blob = settings.clone();
        let providers = std::mem::take(&mut blob.ai.providers);
        let shortcuts = blob.shortcuts.clone();
        let old_ids: Vec<String> = self
            .get()
            .ai
            .providers
            .iter()
            .map(|p| p.id.clone())
            .collect();
        let new_ids: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
        self.storage
            .run(move |db| {
                SettingsRepository::save(db, &blob)?;
                for provider in &providers {
                    ModelConfigRepository::upsert(db, provider)?;
                }
                for stale in old_ids.iter().filter(|id| !new_ids.contains(id)) {
                    ModelConfigRepository::delete(db, stale)?;
                }
                ShortcutRepository::save_all(db, &shortcuts)?;
                Ok(())
            })
            .await
    }

    /// `has_api_key` of `provider` as the secrets cache knows it — seeded at
    /// boot by one attribute-only enumeration and kept current by every set /
    /// delete — so no settings path touches the Keychain, blocks a runtime
    /// worker or holds the settings lock across a Keychain call. A key whose
    /// state is unknown (after a failed write) keeps the flag it had.
    fn key_flag(&self, provider_id: &str, current: &Settings) -> bool {
        self.secrets
            .known_presence(&provider_key(provider_id))
            .unwrap_or_else(|| {
                current
                    .ai
                    .providers
                    .iter()
                    .any(|p| p.id == provider_id && p.has_api_key)
            })
    }

    /// Swap the in-memory settings and publish `settings.changed`. `has_api_key`
    /// flags come from the secrets cache ([`Self::key_flag`]).
    fn replace(&self, mut settings: Settings) {
        let current = self.get();
        for provider in &mut settings.ai.providers {
            provider.has_api_key = self.key_flag(&provider.id, &current);
        }
        *self.current.write() = settings.clone();
        self.bus.publish(BlueyEvent::SettingsChanged(settings));
    }

    /// Refresh `has_api_key` flags after a keychain change and publish
    /// `settings.changed` when anything flipped.
    pub fn refresh_provider_keys(&self) {
        let mut changed = false;
        let snapshot = {
            let mut settings = self.current.write();
            let current = settings.clone();
            for provider in &mut settings.ai.providers {
                let has = self.key_flag(&provider.id, &current);
                if has != provider.has_api_key {
                    provider.has_api_key = has;
                    changed = true;
                }
            }
            settings.clone()
        };
        if changed {
            self.bus.publish(BlueyEvent::SettingsChanged(snapshot));
        }
    }

    /// Replace the in-memory shortcut list (already persisted by the caller)
    /// and publish `settings.changed`.
    pub fn set_shortcuts(&self, shortcuts: Vec<bluey_core::types::ShortcutBinding>) {
        let snapshot = {
            let mut settings = self.current.write();
            settings.shortcuts = shortcuts;
            settings.clone()
        };
        self.bus.publish(BlueyEvent::SettingsChanged(snapshot));
    }

    /// Update one field in memory + blob without going through a patch (used
    /// by `modes_set_default`).
    pub async fn set_default_mode(&self, mode_id: &str) -> BlueyResult<Settings> {
        let (_, new) = self
            .update(serde_json::json!({ "general": { "defaultModeId": mode_id } }))
            .await?;
        Ok(new)
    }
}

/// Cross-field rules a patch from the WebView (or the `.env` import) must meet.
fn validate(settings: &Settings) -> BlueyResult<()> {
    if !bluey_core::presets::EMBEDDING_DIMENSION_CHOICES.contains(&settings.ai.embedding_dimensions)
    {
        return Err(bluey_core::BlueyError::invalid_params(
            "ai.embeddingDimensions must be 768, 1536 or 3072",
        ));
    }
    if !OBSERVATION_INTERVAL_MS.contains(&settings.screen.observation_interval_ms) {
        return Err(bluey_core::BlueyError::invalid_params(
            "screen.observationIntervalMs must be between 1000 and 60000",
        ));
    }
    Ok(())
}

/// Screen sampling interval bounds (UX-031).
const OBSERVATION_INTERVAL_MS: std::ops::RangeInclusive<u32> = 1_000..=60_000;

#[cfg(test)]
mod tests {
    use bluey_core::types::{AiProviderConfig, AiProviderKind};

    use super::*;
    use crate::secrets::backend::fake::{CountingFake, Op};

    fn provider(id: &str) -> AiProviderConfig {
        AiProviderConfig {
            id: id.into(),
            kind: AiProviderKind::GoogleGemini,
            name: id.into(),
            base_url: "https://example.invalid".into(),
            api_version: None,
            deployments: None,
            enabled: true,
            has_api_key: false,
            auth_method: Default::default(),
        }
    }

    fn manager(fake: &Arc<CountingFake>) -> (Arc<SecretsStore>, SettingsManager) {
        let storage = Arc::new(Storage::in_memory());
        storage
            .run_sync(|db| {
                ModelConfigRepository::upsert(db, &provider("gemini"))?;
                ModelConfigRepository::upsert(db, &provider("openai"))
            })
            .unwrap();
        let secrets = Arc::new(SecretsStore::with_backend(fake.clone()));
        secrets.preload_presence().unwrap();
        let bus = Arc::new(EventBus::new());
        let settings = SettingsManager::load(storage, secrets.clone(), bus).unwrap();
        (secrets, settings)
    }

    fn flag(settings: &SettingsManager, id: &str) -> bool {
        let current = settings.get();
        current
            .ai
            .providers
            .iter()
            .any(|p| p.id == id && p.has_api_key)
    }

    #[tokio::test]
    async fn loading_and_saving_settings_never_decrypts_a_key() {
        let fake = Arc::new(CountingFake::with_items(&[(
            "provider:gemini:api_key",
            "g",
        )]));
        let (_, settings) = manager(&fake);
        assert!(flag(&settings, "gemini"));
        assert!(!flag(&settings, "openai"));
        settings
            .update(serde_json::json!({ "general": { "launchAtLogin": true } }))
            .await
            .unwrap();
        assert!(flag(&settings, "gemini"));
        assert_eq!(fake.reads(), 0);
        assert_eq!(fake.count(Op::Exists), 0, "presence comes from memory");
        assert_eq!(fake.count(Op::List), 1, "one enumeration at boot");
    }

    #[tokio::test]
    async fn key_flags_follow_set_and_delete_from_memory() {
        let fake = Arc::new(CountingFake::default());
        let (secrets, settings) = manager(&fake);
        secrets
            .set("provider:openai:api_key", "o".into())
            .await
            .unwrap();
        settings.refresh_provider_keys();
        assert!(flag(&settings, "openai"));
        secrets.delete("provider:openai:api_key").await.unwrap();
        settings.refresh_provider_keys();
        assert!(!flag(&settings, "openai"));
        assert_eq!(fake.reads(), 0);
    }

    #[test]
    fn the_observation_interval_is_bounded() {
        let with = |ms: u32| {
            let mut s = Settings::default();
            s.screen.observation_interval_ms = ms;
            validate(&s)
        };
        assert!(with(Settings::default().screen.observation_interval_ms).is_ok());
        assert!(with(1_000).is_ok());
        assert!(with(60_000).is_ok());
        assert!(with(0).is_err(), "UX-031");
        assert!(with(1_000_000_000).is_err(), "UX-031");
    }
}
