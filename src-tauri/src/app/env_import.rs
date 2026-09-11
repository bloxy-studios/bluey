//! `.env` → Keychain / settings import at boot (ADR 0007).
//!
//! The decision half is pure and unit-tested in `bluey_core::presets`
//! (`plan_env_import`). This module only performs the I/O the plan asks for:
//! Keychain writes for API keys the environment provides — only when the
//! Keychain has no entry for that provider, unless `BLUEY_ENV_OVERRIDES_KEYCHAIN=1`
//! — and one settings patch for provider configs, model assignments and the
//! Gemini-related knobs (`embeddingDimensions`, `researchBackend`,
//! `audio.transcriptionProvider`, `bootstrapProvider`).
//!
//! Settings edits survive reboots: an existing provider keeps its enabled
//! state and base URL (unless the base URL is set in the environment), an
//! unchanged `.env` nomination only fills unassigned roles, and nothing is
//! written when the plan changes nothing. The provider the environment
//! nominated last time is remembered in the settings table
//! ([`ENV_NOMINATION_KEY`]) so a default provider switched in Settings is
//! not undone at the next boot — `ai.bootstrapProvider` is the user's choice,
//! not the environment's.
//!
//! Logging names the provider only: never values, never lengths.

use std::sync::Arc;

use bluey_core::presets::{self, EnvImportPlan};
use bluey_core::BlueyResult;
use bluey_storage::SettingsRepository;
use serde_json::json;

use crate::secrets::{provider_key, SecretsStore};
use crate::settings::SettingsManager;
use crate::storage::Storage;

/// Settings-table key of the provider id `BLUEY_AI_PROVIDER` (or the first keyed
/// provider) nominated at the last import.
pub const ENV_NOMINATION_KEY: &str = "env_import:bootstrap_provider";

/// Import from the process environment (after `load_dotenv`). Failures are
/// reported to the caller but must never abort the boot.
pub fn import_env(
    secrets: &Arc<SecretsStore>,
    settings: &Arc<SettingsManager>,
    storage: &Arc<Storage>,
) -> BlueyResult<()> {
    let last_nomination = storage
        .run_sync(|db| SettingsRepository::get_json(db, ENV_NOMINATION_KEY))?
        .and_then(|value| value.as_str().map(str::to_string));
    let plan = presets::plan_env_import(
        &|name| std::env::var(name).ok(),
        &settings.get(),
        last_nomination.as_deref(),
    );
    let applied = apply(&plan, secrets, settings);
    if let Some(nomination) = &plan.env_nomination {
        if last_nomination.as_deref() != Some(nomination.as_str()) {
            let value = serde_json::Value::String(nomination.clone());
            storage
                .run_sync(move |db| SettingsRepository::set_json(db, ENV_NOMINATION_KEY, &value))?;
        }
    }
    applied
}

fn apply(
    plan: &EnvImportPlan,
    secrets: &SecretsStore,
    settings: &SettingsManager,
) -> BlueyResult<()> {
    for warning in &plan.warnings {
        tracing::warn!("{warning}");
    }
    if plan.is_empty() {
        return Ok(());
    }

    // 1. API keys → Keychain (Keychain-first; a Keychain failure never leads to
    //    an overwrite).
    for (provider_id, var) in &plan.keys {
        let key = provider_key(provider_id);
        match secrets.get_sync(&key) {
            Ok(Some(_)) if !plan.override_keychain => {
                tracing::debug!(provider = %provider_id, "keychain already holds a key; env value ignored");
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(provider = %provider_id, error = %error, "keychain unavailable; env key not imported");
                continue;
            }
        }
        let Ok(value) = std::env::var(var) else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match secrets.set_sync(&key, value) {
            Ok(()) => tracing::info!("imported api key for provider {provider_id}"),
            Err(error) => {
                tracing::warn!(provider = %provider_id, error = %error, "failed to import api key")
            }
        }
    }

    // 2. Providers: new ones are added; existing ones keep the user's kind,
    //    enabled state and base URL unless the base URL came from the environment.
    let current = settings.get();
    let mut providers = current.ai.providers.clone();
    for imported in &plan.providers {
        match providers.iter_mut().find(|p| p.id == imported.id) {
            Some(existing) => {
                if plan.explicit_base_urls.iter().any(|id| id == &imported.id) {
                    existing.base_url = imported.base_url.clone();
                    existing.api_version = imported.api_version.clone();
                }
                if existing.name.trim().is_empty() {
                    existing.name = imported.name.clone();
                }
            }
            None => providers.push(imported.clone()),
        }
    }

    // 3. One settings patch, only for what actually changed.
    let providers_changed = providers != current.ai.providers;
    let models_changed = plan.models != current.ai.models;
    let bootstrap_changed = plan.bootstrap_provider.is_some()
        && plan.bootstrap_provider != current.ai.bootstrap_provider;
    let knobs_changed = plan.embedding_dimensions.is_some()
        || plan.research_backend.is_some()
        || plan.transcription_provider.is_some();
    if !(providers_changed || models_changed || bootstrap_changed || knobs_changed) {
        tracing::debug!("env import: settings already match the environment");
        return Ok(());
    }
    let mut ai = json!({});
    if providers_changed {
        ai["providers"] = json!(providers);
    }
    if models_changed {
        ai["models"] = json!(plan.models);
    }
    if bootstrap_changed {
        ai["bootstrapProvider"] = json!(plan.bootstrap_provider);
    }
    if let Some(dims) = plan.embedding_dimensions {
        ai["embeddingDimensions"] = json!(dims);
    }
    if let Some(backend) = plan.research_backend {
        ai["researchBackend"] = json!(backend);
    }
    let mut patch = json!({ "ai": ai });
    if let Some(kind) = plan.transcription_provider {
        patch["audio"] = json!({ "transcriptionProvider": kind });
    }
    settings.update_sync(patch)?;
    tracing::info!(
        providers = plan.providers.len(),
        roles = plan.changed_roles.len(),
        reapplied = plan.reapplied,
        bootstrap = plan.bootstrap_provider.as_deref().unwrap_or("-"),
        "applied .env import"
    );
    Ok(())
}
