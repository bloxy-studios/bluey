//! Settings blob, panel state, active mode, shortcuts, model provider configs
//! and the cached auth user. API keys are **never** stored here (keychain only).

use bluey_core::error::BlueyError;
use bluey_core::now_iso;
use bluey_core::types::ai::AiProviderConfig;
use bluey_core::types::auth::AuthUser;
use bluey_core::types::settings::{merge_json, PanelState, Settings, ShortcutBinding};
use rusqlite::{params, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::{from_enum_str, opt_from_json, opt_to_json, to_enum_str, to_json_string};
use crate::db::Database;
use crate::error::SqlExt;

const SETTINGS_KEY: &str = "settings";
const PANEL_STATE_KEY: &str = "panel_state";
const ACTIVE_MODE_KEY: &str = "active_mode_id";

/// Key/value JSON store (`settings` table) with typed accessors.
pub struct SettingsRepository;

impl SettingsRepository {
    /// Load the [`Settings`] blob. Missing or partially incompatible JSON never
    /// fails: the stored value is merged **over** `Settings::default()` so new
    /// fields pick up defaults; irrecoverable JSON falls back to the defaults.
    pub fn get(db: &Database) -> Result<Settings, BlueyError> {
        Self::get_merged_or_default::<Settings>(db, SETTINGS_KEY)
    }

    /// Persist the whole [`Settings`] blob.
    pub fn save(db: &Database, settings: &Settings) -> Result<(), BlueyError> {
        Self::set_json(db, SETTINGS_KEY, &serde_json::to_value(settings)?)
    }

    /// Load the persisted [`PanelState`] (defaults merged the same way as settings).
    pub fn get_panel_state(db: &Database) -> Result<PanelState, BlueyError> {
        Self::get_merged_or_default::<PanelState>(db, PANEL_STATE_KEY)
    }

    /// Persist the [`PanelState`].
    pub fn save_panel_state(db: &Database, state: &PanelState) -> Result<(), BlueyError> {
        Self::set_json(db, PANEL_STATE_KEY, &serde_json::to_value(state)?)
    }

    /// Currently active mode id, if one was persisted.
    pub fn get_active_mode_id(db: &Database) -> Result<Option<String>, BlueyError> {
        match Self::get_json(db, ACTIVE_MODE_KEY)? {
            Some(serde_json::Value::String(s)) if !s.is_empty() => Ok(Some(s)),
            _ => Ok(None),
        }
    }

    /// Persist the active mode id.
    pub fn set_active_mode_id(db: &Database, mode_id: &str) -> Result<(), BlueyError> {
        Self::set_json(
            db,
            ACTIVE_MODE_KEY,
            &serde_json::Value::String(mode_id.to_string()),
        )
    }

    /// Raw JSON accessor for any settings key.
    pub fn get_json(db: &Database, key: &str) -> Result<Option<serde_json::Value>, BlueyError> {
        let raw: Option<String> = db.with_conn(|conn| {
            conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .sql()
        })?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }

    /// Raw JSON upsert for any settings key.
    pub fn set_json(db: &Database, key: &str, value: &serde_json::Value) -> Result<(), BlueyError> {
        let json = to_json_string(value)?;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, json, now_iso()],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Deserialize `key` merged over `T::default()`; any failure returns the default.
    fn get_merged_or_default<T>(db: &Database, key: &str) -> Result<T, BlueyError>
    where
        T: Default + Serialize + DeserializeOwned,
    {
        let stored = Self::get_json(db, key)?;
        let default = T::default();
        let Some(stored) = stored else {
            return Ok(default);
        };
        let mut base = serde_json::to_value(&default)?;
        merge_json(&mut base, &stored);
        match serde_json::from_value::<T>(base) {
            Ok(value) => Ok(value),
            Err(e) => {
                tracing::warn!(key, error = %e, "stored JSON incompatible, using defaults");
                Ok(default)
            }
        }
    }
}

/// Persisted shortcut overrides (`shortcuts` table), merged over the defaults
/// from `bluey_core::shortcuts::default_bindings()`.
pub struct ShortcutRepository;

impl ShortcutRepository {
    /// All bindings in default order: stored accelerator/enabled override the
    /// defaults; rows for unknown ids are ignored, missing ids fall back to the
    /// default binding.
    pub fn list(db: &Database) -> Result<Vec<ShortcutBinding>, BlueyError> {
        let rows: Vec<(String, String, bool)> = db.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, accelerator, enabled FROM shortcuts")
                .sql()?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        let mut bindings = bluey_core::shortcuts::default_bindings();
        for (id, accelerator, enabled) in rows {
            let Ok(parsed) = from_enum_str::<bluey_core::types::settings::ShortcutId>(&id) else {
                continue; // unknown / legacy id
            };
            if let Some(binding) = bindings.iter_mut().find(|b| b.id == parsed) {
                binding.accelerator = accelerator;
                binding.enabled = enabled;
            }
        }
        Ok(bindings)
    }

    /// Replace all stored bindings.
    pub fn save_all(db: &Database, bindings: &[ShortcutBinding]) -> Result<(), BlueyError> {
        let rows: Vec<(String, String, bool)> = bindings
            .iter()
            .map(|b| Ok((to_enum_str(&b.id)?, b.accelerator.clone(), b.enabled)))
            .collect::<Result<_, BlueyError>>()?;
        db.transaction(|conn| {
            conn.execute("DELETE FROM shortcuts", []).sql()?;
            for (id, accelerator, enabled) in &rows {
                conn.execute(
                    "INSERT INTO shortcuts (id, accelerator, enabled, updated_at) VALUES (?1, ?2, ?3, ?4)",
                    params![id, accelerator, enabled, now_iso()],
                )
                .sql()?;
            }
            Ok(())
        })
    }

    /// Drop all overrides and return the default bindings.
    pub fn reset(db: &Database) -> Result<Vec<ShortcutBinding>, BlueyError> {
        db.with_conn(|conn| {
            conn.execute("DELETE FROM shortcuts", []).sql()?;
            Ok(())
        })?;
        Ok(bluey_core::shortcuts::default_bindings())
    }
}

/// AI provider configurations (`model_configs`). The API key itself lives in
/// the OS keychain; reads always report `has_api_key: false` and the app layer
/// overwrites the flag after checking the keychain.
pub struct ModelConfigRepository;

impl ModelConfigRepository {
    /// All provider configs (insertion order by `created_at`), `has_api_key` always false.
    pub fn list(db: &Database) -> Result<Vec<AiProviderConfig>, BlueyError> {
        type RawRow = (
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            bool,
        );
        let rows: Vec<RawRow> = db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, name, base_url, api_version, deployments, enabled
                           FROM model_configs ORDER BY created_at, id",
                )
                .sql()?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                })
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        rows.into_iter()
            .map(
                |(id, kind, name, base_url, api_version, deployments, enabled)| {
                    Ok(AiProviderConfig {
                        id,
                        kind: from_enum_str(&kind)?,
                        name,
                        base_url,
                        api_version,
                        deployments: opt_from_json(deployments)?,
                        enabled,
                        has_api_key: false,
                        auth_method: Default::default(),
                    })
                },
            )
            .collect()
    }

    /// Insert or update a provider config by id (`has_api_key` is not persisted).
    pub fn upsert(db: &Database, config: &AiProviderConfig) -> Result<(), BlueyError> {
        let kind = to_enum_str(&config.kind)?;
        let deployments = opt_to_json(&config.deployments)?;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO model_configs (id, kind, name, base_url, api_version, deployments, enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                   kind = excluded.kind,
                   name = excluded.name,
                   base_url = excluded.base_url,
                   api_version = excluded.api_version,
                   deployments = excluded.deployments,
                   enabled = excluded.enabled,
                   updated_at = excluded.updated_at",
                params![
                    config.id,
                    kind,
                    config.name,
                    config.base_url,
                    config.api_version,
                    deployments,
                    config.enabled,
                    now_iso()
                ],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Remove a provider config. Returns whether a row was deleted.
    pub fn delete(db: &Database, id: &str) -> Result<bool, BlueyError> {
        db.with_conn(|conn| {
            conn.execute("DELETE FROM model_configs WHERE id = ?1", [id])
                .map(|n| n > 0)
                .sql()
        })
    }
}

/// Cached Clerk user (`users` table). Auth tokens are **not** stored here.
pub struct UserRepository;

impl UserRepository {
    /// Insert or update the cached user profile.
    pub fn upsert(db: &Database, user: &AuthUser) -> Result<(), BlueyError> {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO users (id, email, first_name, last_name, image_url, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                   email = excluded.email,
                   first_name = excluded.first_name,
                   last_name = excluded.last_name,
                   image_url = excluded.image_url,
                   updated_at = excluded.updated_at",
                params![user.id, user.email, user.first_name, user.last_name, user.image_url, now_iso()],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Fetch a cached user by id.
    pub fn get(db: &Database, id: &str) -> Result<Option<AuthUser>, BlueyError> {
        db.with_conn(|conn| {
            conn.query_row(
                "SELECT id, email, first_name, last_name, image_url FROM users WHERE id = ?1",
                [id],
                |r| {
                    Ok(AuthUser {
                        id: r.get(0)?,
                        email: r.get(1)?,
                        first_name: r.get(2)?,
                        last_name: r.get(3)?,
                        image_url: r.get(4)?,
                    })
                },
            )
            .optional()
            .sql()
        })
    }

    /// Remove every cached user (sign-out).
    pub fn clear(db: &Database) -> Result<(), BlueyError> {
        db.with_conn(|conn| {
            conn.execute("DELETE FROM users", []).sql()?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;
    use bluey_core::types::ai::AiProviderKind;
    use pretty_assertions::assert_eq;

    #[test]
    fn settings_default_merge_and_round_trip() {
        let db = testutil::db();
        // Missing → defaults.
        let s = SettingsRepository::get(&db).unwrap();
        assert_eq!(s, Settings::default());

        // A partial stored blob is merged over the defaults (new fields appear).
        SettingsRepository::set_json(
            &db,
            "settings",
            &serde_json::json!({ "general": { "blueyName": "Blue" }, "privacy": { "storeScreenshots": true } }),
        )
        .unwrap();
        let s = SettingsRepository::get(&db).unwrap();
        assert_eq!(s.general.bluey_name, "Blue");
        assert!(s.privacy.store_screenshots);
        assert_eq!(s.appearance.width, 690, "unspecified fields keep defaults");

        // Full round trip.
        let mut edited = s.clone();
        edited.ai.context_token_budget = 9000;
        SettingsRepository::save(&db, &edited).unwrap();
        assert_eq!(SettingsRepository::get(&db).unwrap(), edited);

        // Incompatible JSON falls back to defaults instead of erroring.
        SettingsRepository::set_json(&db, "settings", &serde_json::json!({ "version": "one" }))
            .unwrap();
        assert_eq!(SettingsRepository::get(&db).unwrap(), Settings::default());
        db.with_conn(|c| {
            c.execute(
                "UPDATE settings SET value = 'not json' WHERE key = 'settings'",
                [],
            )
            .sql()
        })
        .unwrap();
        assert_eq!(SettingsRepository::get(&db).unwrap(), Settings::default());
    }

    #[test]
    fn panel_state_active_mode_and_raw_json() {
        let db = testutil::db();
        assert_eq!(
            SettingsRepository::get_panel_state(&db).unwrap(),
            PanelState::default()
        );
        let ps = PanelState {
            x: 120.0,
            pinned: true,
            ..PanelState::default()
        };
        SettingsRepository::save_panel_state(&db, &ps).unwrap();
        assert_eq!(SettingsRepository::get_panel_state(&db).unwrap(), ps);

        assert!(SettingsRepository::get_active_mode_id(&db)
            .unwrap()
            .is_none());
        SettingsRepository::set_active_mode_id(&db, "interview").unwrap();
        assert_eq!(
            SettingsRepository::get_active_mode_id(&db)
                .unwrap()
                .as_deref(),
            Some("interview")
        );

        SettingsRepository::set_json(&db, "custom", &serde_json::json!({"a": 1})).unwrap();
        assert_eq!(
            SettingsRepository::get_json(&db, "custom").unwrap(),
            Some(serde_json::json!({"a": 1}))
        );
        assert_eq!(SettingsRepository::get_json(&db, "missing").unwrap(), None);
    }

    #[test]
    fn shortcuts_merge_over_defaults() {
        let db = testutil::db();
        let defaults = ShortcutRepository::list(&db).unwrap();
        assert_eq!(defaults, bluey_core::shortcuts::default_bindings());

        let mut edited = defaults.clone();
        edited[0].accelerator = "Cmd+Alt+KeyB".to_string();
        edited[1].enabled = false;
        ShortcutRepository::save_all(&db, &edited).unwrap();
        let listed = ShortcutRepository::list(&db).unwrap();
        assert_eq!(listed[0].accelerator, "Cmd+Alt+KeyB");
        assert_eq!(
            listed[0].default_accelerator,
            defaults[0].default_accelerator
        );
        assert!(!listed[1].enabled);
        assert_eq!(listed.len(), 12);

        let reset = ShortcutRepository::reset(&db).unwrap();
        assert_eq!(reset, defaults);
        assert_eq!(ShortcutRepository::list(&db).unwrap(), defaults);
    }

    #[test]
    fn model_configs_never_report_api_keys() {
        let db = testutil::db();
        let cfg = AiProviderConfig {
            id: "azure-1".into(),
            kind: AiProviderKind::AzureFoundry,
            name: "Azure".into(),
            base_url: "https://example.openai.azure.com".into(),
            api_version: Some("2024-06-01".into()),
            deployments: Some(std::collections::BTreeMap::from([(
                "default".to_string(),
                "gpt".to_string(),
            )])),
            enabled: true,
            has_api_key: true, // must NOT persist
            auth_method: Default::default(),
        };
        ModelConfigRepository::upsert(&db, &cfg).unwrap();
        let listed = ModelConfigRepository::list(&db).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].has_api_key);
        assert_eq!(listed[0].deployments, cfg.deployments);

        let mut updated = cfg.clone();
        updated.name = "Azure Prod".into();
        ModelConfigRepository::upsert(&db, &updated).unwrap();
        assert_eq!(
            ModelConfigRepository::list(&db).unwrap()[0].name,
            "Azure Prod"
        );

        assert!(ModelConfigRepository::delete(&db, "azure-1").unwrap());
        assert!(!ModelConfigRepository::delete(&db, "azure-1").unwrap());
    }

    #[test]
    fn user_cache_round_trip() {
        let db = testutil::db();
        let user = AuthUser {
            id: "user_1".into(),
            email: Some("jane@example.com".into()),
            first_name: Some("Jane".into()),
            last_name: None,
            image_url: None,
        };
        UserRepository::upsert(&db, &user).unwrap();
        assert_eq!(
            UserRepository::get(&db, "user_1").unwrap(),
            Some(user.clone())
        );
        let mut updated = user.clone();
        updated.last_name = Some("Doe".into());
        UserRepository::upsert(&db, &updated).unwrap();
        assert_eq!(UserRepository::get(&db, "user_1").unwrap(), Some(updated));
        UserRepository::clear(&db).unwrap();
        assert_eq!(UserRepository::get(&db, "user_1").unwrap(), None);
    }
}
