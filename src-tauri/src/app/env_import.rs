//! `.env` → Keychain / settings import at boot (ADR 0007).
//!
//! The decision half is pure and unit-tested in `bluey_core::presets`
//! (`plan_env_import`). This module only performs the I/O the plan asks for:
//! Keychain writes for API keys the environment provides (provider keys plus
//! `EXA_API_KEY` / `FIRECRAWL_API_KEY`) — only when the Keychain has no entry
//! for that key, unless `BLUEY_ENV_OVERRIDES_KEYCHAIN=1` — and one settings
//! patch for provider configs, model assignments and the Gemini-related knobs
//! (`embeddingDimensions`, `researchBackend`, `audio.transcriptionProvider`,
//! `bootstrapProvider`).
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

use crate::secrets::{provider_key, SecretsStore, EXA_KEY, FIRECRAWL_KEY};
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

/// `.env` variables that seed the research (web search) keys (PROV-014).
const RESEARCH_KEYS: [(&str, &str); 2] = [
    (EXA_KEY, "EXA_API_KEY"),
    (FIRECRAWL_KEY, "FIRECRAWL_API_KEY"),
];

/// Write `value` to `key` unless the Keychain already holds an entry (an
/// attribute-only check — boot never shows a Keychain prompt) and
/// `override_keychain` is off. A Keychain failure never leads to an
/// overwrite. `label` names the provider or variable in logs; never values.
fn import_key(
    secrets: &SecretsStore,
    key: &str,
    value: Option<&str>,
    override_keychain: bool,
    label: &str,
) {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return;
    };
    match secrets.has_sync(key) {
        Ok(true) if !override_keychain => {
            tracing::debug!(%label, "keychain already holds a key; env value ignored");
            return;
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%label, error = %error, "keychain unavailable; env key not imported");
            return;
        }
    }
    match secrets.set_sync(key, value) {
        Ok(()) => tracing::info!("imported api key for {label}"),
        Err(error) => tracing::warn!(%label, error = %error, "failed to import api key"),
    }
}

fn apply(
    plan: &EnvImportPlan,
    secrets: &SecretsStore,
    settings: &SettingsManager,
) -> BlueyResult<()> {
    for warning in &plan.warnings {
        tracing::warn!("{warning}");
    }
    // Research keys are not provider-bound, so they bypass the plan.
    for (key, var) in RESEARCH_KEYS {
        let value = std::env::var(var).ok();
        import_key(secrets, key, value.as_deref(), plan.override_keychain, var);
    }
    if plan.is_empty() {
        return Ok(());
    }

    // 1. API keys → Keychain.
    for (provider_id, var) in &plan.keys {
        let value = std::env::var(var).ok();
        import_key(
            secrets,
            &provider_key(provider_id),
            value.as_deref(),
            plan.override_keychain,
            &format!("provider {provider_id}"),
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::backend::fake::{CountingFake, Op};

    fn store(items: &[(&str, &str)]) -> (Arc<CountingFake>, SecretsStore) {
        let fake = Arc::new(CountingFake::with_items(items));
        (fake.clone(), SecretsStore::with_backend(fake))
    }

    #[test]
    fn env_keys_fill_only_empty_entries_without_reading_the_keychain() {
        let (fake, secrets) = store(&[(EXA_KEY, "saved")]);
        import_key(&secrets, EXA_KEY, Some("from-env"), false, "EXA_API_KEY");
        import_key(
            &secrets,
            FIRECRAWL_KEY,
            Some(" fc "),
            false,
            "FIRECRAWL_API_KEY",
        );
        import_key(
            &secrets,
            &provider_key("gemini"),
            Some("  "),
            false,
            "gemini",
        );

        assert_eq!(fake.value(EXA_KEY).as_deref(), Some("saved"));
        assert_eq!(fake.value(FIRECRAWL_KEY).as_deref(), Some("fc"));
        assert_eq!(fake.value(&provider_key("gemini")), None);
        assert_eq!(fake.reads(), 0, "boot import never shows a Keychain prompt");
        assert_eq!(fake.count(Op::Add), 1);
    }

    #[test]
    fn override_replaces_a_saved_key_and_a_locked_entry_is_never_overwritten() {
        let (fake, secrets) = store(&[(EXA_KEY, "saved")]);
        import_key(&secrets, EXA_KEY, Some("from-env"), true, "EXA_API_KEY");
        assert_eq!(fake.value(EXA_KEY).as_deref(), Some("from-env"));

        let (fake, secrets) = store(&[(FIRECRAWL_KEY, "saved")]);
        fake.fail_lookups(-25308);
        import_key(
            &secrets,
            FIRECRAWL_KEY,
            Some("from-env"),
            false,
            "FIRECRAWL_API_KEY",
        );
        assert_eq!(fake.value(FIRECRAWL_KEY).as_deref(), Some("saved"));
    }

    #[test]
    fn research_variables_match_env_example() {
        let example = include_str!("../../../.env.example");
        for (_, var) in RESEARCH_KEYS {
            assert!(
                example.contains(&format!("{var}=")),
                "{var} missing from .env.example"
            );
        }
    }
}
