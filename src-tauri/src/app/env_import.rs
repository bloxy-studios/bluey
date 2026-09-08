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
//! Logging names the provider only: never values, never lengths.

use std::sync::Arc;

use bluey_core::presets::{self, EnvImportPlan};
use bluey_core::BlueyResult;
use serde_json::json;

use crate::secrets::{provider_key, SecretsStore};
use crate::settings::SettingsManager;

/// Import from the process environment (after `load_dotenv`). Failures are
/// reported to the caller but must never abort the boot.
pub fn import_env(secrets: &Arc<SecretsStore>, settings: &Arc<SettingsManager>) -> BlueyResult<()> {
    let plan = presets::plan_env_import(&|name| std::env::var(name).ok(), &settings.get());
    apply(&plan, secrets, settings)
}

fn apply(
    plan: &EnvImportPlan,
    secrets: &SecretsStore,
    settings: &SettingsManager,
) -> BlueyResult<()> {
    if plan.is_empty() {
        return Ok(());
    }

    // 1. API keys → Keychain.
    for (provider_id, var) in &plan.keys {
        let key = provider_key(provider_id);
        if secrets.has_sync(&key) && !plan.override_keychain {
            tracing::debug!(provider = %provider_id, "keychain already holds a key; env value ignored");
            continue;
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

    // 2. Providers, model assignments and knobs → one settings patch.
    let current = settings.get();
    let mut providers = current.ai.providers.clone();
    for imported in &plan.providers {
        match providers.iter_mut().find(|p| p.id == imported.id) {
            Some(existing) => {
                existing.kind = imported.kind;
                existing.base_url = imported.base_url.clone();
                existing.api_version = imported.api_version.clone();
                existing.enabled = true;
                if existing.name.trim().is_empty() {
                    existing.name = imported.name.clone();
                }
            }
            None => providers.push(imported.clone()),
        }
    }
    let mut ai = json!({ "providers": providers, "models": plan.models });
    if let Some(id) = &plan.bootstrap_provider {
        ai["bootstrapProvider"] = json!(id);
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
        bootstrap = plan.bootstrap_provider.as_deref().unwrap_or("-"),
        "applied .env import"
    );
    Ok(())
}
