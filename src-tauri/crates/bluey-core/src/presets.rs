//! Provider presets and the pure half of the `.env` bootstrap import.
//!
//! One table drives three things:
//!
//! * the **reserved provider ids** (`gemini`, `azure-foundry`, `anthropic`,
//!   `openai`) and display names used when a provider is created from a
//!   preset (Settings → AI → "Add provider", or the `.env` import);
//! * the **recommended model per role** for each provider kind — applied by
//!   `ai_apply_provider_presets` and by the import;
//! * [`plan_env_import`], which turns the process environment + the current
//!   settings into an [`EnvImportPlan`]. The plan never contains secret
//!   *values*: it names the environment variable a key lives in and the app
//!   crate reads it at the moment it writes the Keychain.
//!
//! Everything here is deterministic and unit-tested; the Keychain and settings
//! I/O live in the app crate (`app::env_import`).

use std::collections::BTreeMap;

use crate::types::{
    AiProviderConfig, AiProviderKind, ModelAssignment, ModelRole, ModelRoleAssignments,
    ResearchBackend, Settings, TranscriptionProviderKind,
};
use crate::{BlueyError, BlueyResult};

/// Reserved provider ids (one per kind; users may add more with other ids).
pub const GEMINI_ID: &str = "gemini";
pub const AZURE_FOUNDRY_ID: &str = "azure-foundry";
pub const ANTHROPIC_ID: &str = "anthropic";
pub const OPENAI_ID: &str = "openai";

/// Environment variable naming the bootstrap provider (`gemini` default).
pub const ENV_AI_PROVIDER: &str = "BLUEY_AI_PROVIDER";
/// When set to `1`/`true`, `.env` keys replace existing Keychain entries.
pub const ENV_OVERRIDE_KEYCHAIN: &str = "BLUEY_ENV_OVERRIDES_KEYCHAIN";
pub const ENV_EMBEDDING_DIMENSIONS: &str = "BLUEY_EMBEDDING_DIMENSIONS";
pub const ENV_TRANSCRIPTION_PROVIDER: &str = "BLUEY_TRANSCRIPTION_PROVIDER";
pub const ENV_RESEARCH_BACKEND: &str = "RESEARCH_BACKEND";

/// Supported MRL sizes for `gemini-embedding-2` (anything else is rejected).
pub const EMBEDDING_DIMENSION_CHOICES: [u32; 3] = [768, 1536, 3072];

/// A provider kind's defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPreset {
    pub kind: AiProviderKind,
    pub id: &'static str,
    pub name: &'static str,
    /// Default base URL (`""` when the adapter knows the public endpoint or the
    /// user must supply one).
    pub base_url: &'static str,
    /// The provider cannot work without a user-supplied base URL (Foundry).
    pub requires_base_url: bool,
    /// Environment variables that may hold the API key, in priority order.
    pub key_env: &'static [&'static str],
    /// Environment variable that may hold the base URL.
    pub base_url_env: &'static str,
    /// Recommended model per role (`None` = the kind does not serve the role).
    pub models: [(ModelRole, Option<&'static str>); 7],
}

impl ProviderPreset {
    /// Recommended model for `role`.
    pub fn model_for(&self, role: ModelRole) -> Option<&'static str> {
        self.models
            .iter()
            .find(|(r, _)| *r == role)
            .and_then(|(_, model)| *model)
    }

    /// A fresh provider config for this preset (no key yet).
    pub fn config(&self) -> AiProviderConfig {
        AiProviderConfig {
            id: self.id.to_string(),
            kind: self.kind,
            name: self.name.to_string(),
            base_url: self.base_url.to_string(),
            api_version: None,
            deployments: None,
            enabled: true,
            has_api_key: false,
            auth_method: Default::default(),
        }
    }
}

pub static GEMINI: ProviderPreset = ProviderPreset {
    kind: AiProviderKind::GoogleGemini,
    id: GEMINI_ID,
    name: "Google Gemini",
    base_url: "",
    requires_base_url: false,
    key_env: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
    base_url_env: "GEMINI_BASE_URL",
    models: [
        (ModelRole::Default, Some("gemini-3.8-flash")),
        (ModelRole::Fast, Some("gemini-3.5-flash-lite")),
        (ModelRole::Reasoning, Some("gemini-3.8-flash")),
        (ModelRole::Vision, Some("gemini-3.8-flash")),
        (ModelRole::Research, Some("gemini-3.8-flash")),
        (ModelRole::Transcription, Some("gemini-3.5-transcribe")),
        (ModelRole::Embedding, Some("gemini-embedding-2")),
    ],
};

pub static AZURE_FOUNDRY: ProviderPreset = ProviderPreset {
    kind: AiProviderKind::AzureFoundry,
    id: AZURE_FOUNDRY_ID,
    name: "Microsoft Foundry",
    base_url: "",
    requires_base_url: true,
    key_env: &["AZURE_FOUNDRY_API_KEY"],
    base_url_env: "AZURE_FOUNDRY_ENDPOINT",
    models: [
        (ModelRole::Default, Some("gpt-5.6-terra")),
        (ModelRole::Fast, Some("gpt-5.6-luna")),
        (ModelRole::Reasoning, Some("gpt-6-astra")),
        (ModelRole::Vision, Some("gpt-6-astra")),
        (ModelRole::Research, Some("gpt-6-astra")),
        (ModelRole::Transcription, Some("MAI-Transcribe-1.5")),
        (ModelRole::Embedding, Some("text-embedding-3-small")),
    ],
};

pub static ANTHROPIC: ProviderPreset = ProviderPreset {
    kind: AiProviderKind::Anthropic,
    id: ANTHROPIC_ID,
    name: "Anthropic",
    base_url: "https://api.anthropic.com",
    requires_base_url: false,
    // ANTHROPIC_FOUNDRY_API_KEY is deliberately not accepted here: it belongs to
    // Claude-on-Foundry (research sidecar), whose endpoint is not api.anthropic.com.
    key_env: &["ANTHROPIC_API_KEY"],
    base_url_env: "ANTHROPIC_BASE_URL",
    models: [
        (ModelRole::Default, Some("claude-sonnet-5")),
        (ModelRole::Fast, Some("claude-haiku-4-5")),
        (ModelRole::Reasoning, Some("claude-opus-5")),
        (ModelRole::Vision, Some("claude-sonnet-5")),
        (ModelRole::Research, Some("claude-opus-5")),
        (ModelRole::Transcription, None),
        (ModelRole::Embedding, None),
    ],
};

pub static OPENAI: ProviderPreset = ProviderPreset {
    kind: AiProviderKind::OpenaiCompatible,
    id: OPENAI_ID,
    name: "OpenAI-compatible",
    base_url: "",
    requires_base_url: true,
    key_env: &["OPENAI_API_KEY"],
    base_url_env: "OPENAI_BASE_URL",
    // Model line-ups differ per endpoint; the user picks from `ai_list_models`.
    models: [
        (ModelRole::Default, None),
        (ModelRole::Fast, None),
        (ModelRole::Reasoning, None),
        (ModelRole::Vision, None),
        (ModelRole::Research, None),
        (ModelRole::Transcription, None),
        (ModelRole::Embedding, None),
    ],
};

/// Every preset, in the order the UI lists them (Gemini first).
pub fn all() -> [&'static ProviderPreset; 4] {
    [&GEMINI, &AZURE_FOUNDRY, &ANTHROPIC, &OPENAI]
}

/// The preset for a provider kind (`None` for mock).
pub fn by_kind(kind: AiProviderKind) -> Option<&'static ProviderPreset> {
    all().into_iter().find(|preset| preset.kind == kind)
}

/// The preset owning a reserved provider id.
pub fn by_id(id: &str) -> Option<&'static ProviderPreset> {
    all().into_iter().find(|preset| preset.id == id)
}

/// Parse a `BLUEY_AI_PROVIDER` value (aliases accepted, case-insensitive).
pub fn parse_provider_choice(value: &str) -> Option<&'static ProviderPreset> {
    match value.trim().to_ascii_lowercase().as_str() {
        "gemini" | "google" | "google_gemini" | "google-gemini" => Some(&GEMINI),
        "azure-foundry" | "azure_foundry" | "foundry" | "azure" => Some(&AZURE_FOUNDRY),
        "anthropic" | "claude" => Some(&ANTHROPIC),
        "openai" | "openai-compatible" | "openai_compatible" => Some(&OPENAI),
        _ => None,
    }
}

/// Parse a `RESEARCH_BACKEND` value (aliases accepted, case-insensitive).
pub fn parse_research_backend(value: &str) -> Option<ResearchBackend> {
    match value.trim().to_ascii_lowercase().as_str() {
        "gemini" | "google" => Some(ResearchBackend::Gemini),
        "claude" | "anthropic" => Some(ResearchBackend::Claude),
        _ => None,
    }
}

/// Parse `BLUEY_EMBEDDING_DIMENSIONS` (one of [`EMBEDDING_DIMENSION_CHOICES`]).
pub fn parse_embedding_dimensions(value: &str) -> Option<u32> {
    let dims: u32 = value.trim().parse().ok()?;
    EMBEDDING_DIMENSION_CHOICES.contains(&dims).then_some(dims)
}

/// Env variable carrying the model override for `role` (`BLUEY_MODEL_<ROLE>`).
pub fn model_override_env(role: ModelRole) -> &'static str {
    match role {
        ModelRole::Default => "BLUEY_MODEL_DEFAULT",
        ModelRole::Fast => "BLUEY_MODEL_FAST",
        ModelRole::Reasoning => "BLUEY_MODEL_REASONING",
        ModelRole::Vision => "BLUEY_MODEL_VISION",
        ModelRole::Research => "BLUEY_MODEL_RESEARCH",
        ModelRole::Transcription => "BLUEY_MODEL_TRANSCRIPTION",
        ModelRole::Embedding => "BLUEY_MODEL_EMBEDDING",
    }
}

/// Apply `provider`'s preset models to `assignments`.
///
/// * `overwrite = false` fills only roles that are unassigned;
/// * `overwrite = true` points every role the preset serves at this provider;
/// * `overrides` (role → model id) always win and always target this provider.
///
/// Returns the roles that changed. Errors when the provider kind has no preset
/// (mock) — the caller surfaces `config.no_preset`.
pub fn apply_presets(
    assignments: &mut ModelRoleAssignments,
    provider: &AiProviderConfig,
    overwrite: bool,
    overrides: &BTreeMap<ModelRole, String>,
) -> BlueyResult<Vec<ModelRole>> {
    let preset = by_kind(provider.kind).ok_or_else(|| {
        BlueyError::configuration("no_preset", "this provider kind has no recommended models")
    })?;
    let mut changed = Vec::new();
    for role in ModelRole::ALL {
        let model = match overrides.get(&role) {
            Some(model) if !model.trim().is_empty() => Some(model.trim().to_string()),
            _ => {
                let recommended = preset.model_for(role).map(str::to_string);
                match recommended {
                    Some(model) if overwrite || assignments.get(role).is_none() => Some(model),
                    _ => None,
                }
            }
        };
        let Some(model) = model else { continue };
        let next = ModelAssignment {
            provider_id: provider.id.clone(),
            model,
        };
        if assignments.get(role) != Some(&next) {
            assignments.set(role, Some(next));
            changed.push(role);
        }
    }
    Ok(changed)
}

/// The result of planning a `.env` import. Contains no secret values.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EnvImportPlan {
    /// Provider id the environment nominates as the default (`ai.bootstrapProvider`).
    pub bootstrap_provider: Option<String>,
    /// Provider configs that must exist afterwards (new or with an updated base URL).
    pub providers: Vec<AiProviderConfig>,
    /// `(provider id, environment variable holding its API key)`.
    pub keys: Vec<(String, &'static str)>,
    /// Model assignments after the bootstrap provider's presets/overrides.
    pub models: ModelRoleAssignments,
    pub changed_roles: Vec<ModelRole>,
    pub embedding_dimensions: Option<u32>,
    pub transcription_provider: Option<TranscriptionProviderKind>,
    pub research_backend: Option<ResearchBackend>,
    /// `BLUEY_ENV_OVERRIDES_KEYCHAIN` is on: env keys replace Keychain entries.
    pub override_keychain: bool,
    /// Provider ids whose base URL came from the environment (only those may
    /// replace a base URL the user edited in Settings).
    pub explicit_base_urls: Vec<String>,
    /// The nomination changed, so presets were re-applied over user edits.
    pub reapplied: bool,
    /// Human-readable notes for the log (never values).
    pub warnings: Vec<String>,
}

impl EnvImportPlan {
    /// Nothing in the environment asks for a change.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
            && self.keys.is_empty()
            && self.changed_roles.is_empty()
            && self.embedding_dimensions.is_none()
            && self.transcription_provider.is_none()
            && self.research_backend.is_none()
            && self.bootstrap_provider.is_none()
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn truthy(value: Option<String>) -> bool {
    matches!(
        non_empty(value)
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Plan the import. `env` reads one variable (e.g. `|n| std::env::var(n).ok()`).
///
/// * a provider is imported when one of its `key_env` variables is set;
/// * the bootstrap provider is `BLUEY_AI_PROVIDER` when valid, else the first
///   imported provider in preset order (Gemini first);
/// * base URLs come from the env when set, else the existing config's, else the preset;
/// * the bootstrap provider's recommended models fill unassigned roles and
///   `BLUEY_MODEL_*` overrides always apply to it.
pub fn plan_env_import(env: &dyn Fn(&str) -> Option<String>, settings: &Settings) -> EnvImportPlan {
    let mut plan = EnvImportPlan {
        models: settings.ai.models.clone(),
        override_keychain: truthy(env(ENV_OVERRIDE_KEYCHAIN)),
        ..EnvImportPlan::default()
    };

    let mut imported: Vec<&'static ProviderPreset> = Vec::new();
    for preset in all() {
        let Some(key_var) = preset
            .key_env
            .iter()
            .copied()
            .find(|var| non_empty(env(var)).is_some())
        else {
            continue;
        };
        let existing = settings.ai.providers.iter().find(|p| p.id == preset.id);
        let mut config = existing.cloned().unwrap_or_else(|| preset.config());
        config.kind = preset.kind;
        config.enabled = true;
        if let Some(base_url) = non_empty(env(preset.base_url_env)) {
            config.base_url = base_url.trim_end_matches('/').to_string();
            plan.explicit_base_urls.push(preset.id.to_string());
        }
        if preset.kind == AiProviderKind::AzureFoundry {
            if let Some(version) = non_empty(env("AZURE_FOUNDRY_API_VERSION")) {
                config.api_version = Some(version);
            }
        }
        plan.providers.push(config);
        plan.keys.push((preset.id.to_string(), key_var));
        imported.push(preset);
    }

    let previous = settings.ai.bootstrap_provider.as_deref();
    let first_import = previous.is_none();
    let explicit = non_empty(env(ENV_AI_PROVIDER)).and_then(|value| parse_provider_choice(&value));
    let explicit_configured = explicit
        .map(|preset| {
            plan.providers.iter().any(|p| p.id == preset.id)
                || settings.ai.providers.iter().any(|p| p.id == preset.id)
        })
        .unwrap_or(false);
    let chosen = match explicit {
        Some(preset) if explicit_configured => Some(preset),
        Some(preset) => {
            plan.warnings.push(format!(
                "{ENV_AI_PROVIDER}={} names a provider without a key or configuration; using the first keyed provider instead",
                preset.id
            ));
            imported.first().copied()
        }
        None => imported.first().copied(),
    };

    if let Some(preset) = chosen {
        let config = plan
            .providers
            .iter()
            .find(|p| p.id == preset.id)
            .cloned()
            .or_else(|| {
                settings
                    .ai
                    .providers
                    .iter()
                    .find(|p| p.id == preset.id)
                    .cloned()
            });
        if let Some(config) = config {
            let mut overrides = BTreeMap::new();
            for role in ModelRole::ALL {
                if let Some(model) = non_empty(env(model_override_env(role))) {
                    overrides.insert(role, model);
                }
            }
            // A changed nomination re-applies the presets over user edits; an
            // unchanged one only fills gaps, so Settings edits survive reboots.
            let overwrite = !first_import && previous != Some(config.id.as_str());
            if let Ok(changed) = apply_presets(&mut plan.models, &config, overwrite, &overrides) {
                plan.changed_roles = changed;
            }
            plan.reapplied = overwrite;
            plan.bootstrap_provider = Some(config.id);
        }
    }

    // Knobs follow the same rule: applied on the first import or when the
    // nomination changes, never on every boot (they would undo Settings edits).
    let nomination_changed = plan.bootstrap_provider.as_deref() != previous;
    if first_import || nomination_changed {
        plan.embedding_dimensions = non_empty(env(ENV_EMBEDDING_DIMENSIONS))
            .and_then(|v| parse_embedding_dimensions(&v))
            .filter(|dims| *dims != settings.ai.embedding_dimensions);
        plan.transcription_provider = non_empty(env(ENV_TRANSCRIPTION_PROVIDER))
            .and_then(|v| {
                serde_json::from_value::<TranscriptionProviderKind>(serde_json::Value::String(v))
                    .ok()
            })
            .filter(|kind| *kind != settings.audio.transcription_provider);
        plan.research_backend = non_empty(env(ENV_RESEARCH_BACKEND))
            .and_then(|v| parse_research_backend(&v))
            .filter(|backend| *backend != settings.ai.research_backend);
    }
    if plan.bootstrap_provider == settings.ai.bootstrap_provider {
        // Unchanged nomination is not a change worth persisting on its own.
        if plan.providers.is_empty() && plan.changed_roles.is_empty() {
            plan.bootstrap_provider = None;
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    #[test]
    fn presets_cover_every_kind_but_mock() {
        for kind in [
            AiProviderKind::GoogleGemini,
            AiProviderKind::AzureFoundry,
            AiProviderKind::Anthropic,
            AiProviderKind::OpenaiCompatible,
        ] {
            assert!(by_kind(kind).is_some(), "{kind:?}");
        }
        assert!(by_kind(AiProviderKind::Mock).is_none());
        assert_eq!(
            by_id("gemini").map(|p| p.kind),
            Some(AiProviderKind::GoogleGemini)
        );
        assert_eq!(all()[0].id, GEMINI_ID, "Gemini is listed first");
    }

    #[test]
    fn gemini_preset_matches_the_verified_model_ids() {
        assert_eq!(
            GEMINI.model_for(ModelRole::Default),
            Some("gemini-3.8-flash")
        );
        assert_eq!(
            GEMINI.model_for(ModelRole::Fast),
            Some("gemini-3.5-flash-lite")
        );
        assert_eq!(
            GEMINI.model_for(ModelRole::Reasoning),
            Some("gemini-3.8-flash")
        );
        assert_eq!(
            GEMINI.model_for(ModelRole::Vision),
            Some("gemini-3.8-flash")
        );
        assert_eq!(
            GEMINI.model_for(ModelRole::Research),
            Some("gemini-3.8-flash")
        );
        assert_eq!(
            GEMINI.model_for(ModelRole::Transcription),
            Some("gemini-3.5-transcribe")
        );
        assert_eq!(
            GEMINI.model_for(ModelRole::Embedding),
            Some("gemini-embedding-2")
        );
        assert!(!GEMINI.requires_base_url);
        assert!(AZURE_FOUNDRY.requires_base_url);
    }

    #[test]
    fn apply_presets_fills_only_empty_roles_unless_overwriting() {
        let gemini = GEMINI.config();
        let mut models = ModelRoleAssignments::default();
        models.set(
            ModelRole::Default,
            Some(ModelAssignment {
                provider_id: "azure-foundry".into(),
                model: "gpt-5.6-terra".into(),
            }),
        );

        let changed = apply_presets(&mut models, &gemini, false, &BTreeMap::new()).unwrap();
        assert_eq!(
            changed.len(),
            6,
            "default stays on Foundry, the other six roles are filled"
        );
        assert_eq!(
            models.default.as_ref().unwrap().provider_id,
            "azure-foundry"
        );
        assert_eq!(
            models.embedding.as_ref().unwrap().model,
            "gemini-embedding-2"
        );

        let changed = apply_presets(&mut models, &gemini, true, &BTreeMap::new()).unwrap();
        assert_eq!(changed, vec![ModelRole::Default]);
        assert_eq!(models.default.as_ref().unwrap().model, "gemini-3.8-flash");

        // Idempotent.
        assert!(apply_presets(&mut models, &gemini, true, &BTreeMap::new())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn apply_presets_skips_roles_the_kind_does_not_serve_and_honours_overrides() {
        let anthropic = ANTHROPIC.config();
        let mut models = ModelRoleAssignments::default();
        let mut overrides = BTreeMap::new();
        overrides.insert(ModelRole::Fast, "claude-haiku-5".to_string());
        let changed = apply_presets(&mut models, &anthropic, false, &overrides).unwrap();
        assert!(models.embedding.is_none() && models.transcription.is_none());
        assert_eq!(models.fast.as_ref().unwrap().model, "claude-haiku-5");
        assert_eq!(changed.len(), 5);

        let mock = AiProviderConfig {
            kind: AiProviderKind::Mock,
            ..GEMINI.config()
        };
        assert_eq!(
            apply_presets(&mut models, &mock, false, &BTreeMap::new())
                .unwrap_err()
                .code,
            "config.no_preset"
        );
    }

    #[test]
    fn plan_imports_gemini_from_the_google_alias_and_nominates_it() {
        let settings = Settings::default();
        let env = env_of(&[
            ("GOOGLE_API_KEY", "AIza-secret"),
            ("BLUEY_EMBEDDING_DIMENSIONS", "1536"),
        ]);
        let plan = plan_env_import(&env, &settings);

        assert_eq!(plan.bootstrap_provider.as_deref(), Some("gemini"));
        assert_eq!(plan.providers.len(), 1);
        assert_eq!(plan.providers[0].kind, AiProviderKind::GoogleGemini);
        assert_eq!(plan.providers[0].base_url, "");
        assert_eq!(plan.keys, vec![("gemini".to_string(), "GOOGLE_API_KEY")]);
        assert_eq!(plan.changed_roles.len(), 7);
        assert_eq!(
            plan.models.default.as_ref().unwrap().model,
            "gemini-3.8-flash"
        );
        assert_eq!(plan.embedding_dimensions, Some(1536));
        assert!(!plan.override_keychain);
        let debug = format!("{plan:?}");
        assert!(
            !debug.contains("AIza-secret"),
            "plans never carry secret values"
        );
    }

    #[test]
    fn plan_respects_the_explicit_provider_choice_and_model_overrides() {
        let settings = Settings::default();
        let env = env_of(&[
            ("GEMINI_API_KEY", "k1"),
            ("AZURE_FOUNDRY_API_KEY", "k2"),
            ("AZURE_FOUNDRY_ENDPOINT", "https://res.openai.azure.com/"),
            ("AZURE_FOUNDRY_API_VERSION", "preview"),
            ("BLUEY_AI_PROVIDER", "foundry"),
            ("BLUEY_MODEL_DEFAULT", "gpt-6-astra"),
            ("RESEARCH_BACKEND", "claude"),
            ("BLUEY_ENV_OVERRIDES_KEYCHAIN", "1"),
        ]);
        let plan = plan_env_import(&env, &settings);

        assert_eq!(plan.bootstrap_provider.as_deref(), Some("azure-foundry"));
        assert_eq!(plan.providers.len(), 2, "both keyed providers are imported");
        let foundry = plan
            .providers
            .iter()
            .find(|p| p.id == "azure-foundry")
            .unwrap();
        assert_eq!(foundry.base_url, "https://res.openai.azure.com");
        assert_eq!(foundry.api_version.as_deref(), Some("preview"));
        assert_eq!(plan.models.default.as_ref().unwrap().model, "gpt-6-astra");
        assert_eq!(
            plan.models.default.as_ref().unwrap().provider_id,
            "azure-foundry"
        );
        assert_eq!(plan.models.fast.as_ref().unwrap().model, "gpt-5.6-luna");
        assert_eq!(plan.research_backend, Some(ResearchBackend::Claude));
        assert!(plan.override_keychain);
    }

    #[test]
    fn plan_keeps_user_assignments_and_is_empty_without_env() {
        let mut settings = Settings::default();
        settings.ai.models.set(
            ModelRole::Default,
            Some(ModelAssignment {
                provider_id: "anthropic".into(),
                model: "claude-sonnet-5".into(),
            }),
        );
        let env = env_of(&[("GEMINI_API_KEY", "k")]);
        let plan = plan_env_import(&env, &settings);
        assert_eq!(
            plan.models.default.as_ref().unwrap().provider_id,
            "anthropic"
        );
        assert_eq!(plan.models.fast.as_ref().unwrap().provider_id, "gemini");

        let empty = plan_env_import(&env_of(&[]), &settings);
        assert!(empty.is_empty());
    }

    fn bootstrapped_settings(provider: &str) -> Settings {
        let mut settings = Settings::default();
        settings.ai.bootstrap_provider = Some(provider.to_string());
        settings.ai.providers = vec![GEMINI.config(), AZURE_FOUNDRY.config()];
        settings.ai.models.set(
            ModelRole::Default,
            Some(ModelAssignment {
                provider_id: "gemini".into(),
                model: "gemini-3.6-flash".into(),
            }),
        );
        settings
    }

    #[test]
    fn plan_reapplies_presets_only_when_the_nomination_changes() {
        // Same nomination as last boot: user edits are kept, nothing to persist.
        let settings = bootstrapped_settings("gemini");
        let env = env_of(&[("GEMINI_API_KEY", "k"), ("BLUEY_AI_PROVIDER", "gemini")]);
        let plan = plan_env_import(&env, &settings);
        assert!(!plan.reapplied);
        assert_eq!(
            plan.models.default.as_ref().unwrap().model,
            "gemini-3.6-flash",
            "an unchanged nomination fills gaps only"
        );
        assert_eq!(plan.changed_roles.len(), 6);

        // A different nomination re-applies that provider's presets over the edits.
        let env = env_of(&[
            ("GEMINI_API_KEY", "k"),
            ("AZURE_FOUNDRY_API_KEY", "f"),
            ("BLUEY_AI_PROVIDER", "azure-foundry"),
        ]);
        let plan = plan_env_import(&env, &settings);
        assert!(plan.reapplied);
        assert_eq!(plan.bootstrap_provider.as_deref(), Some("azure-foundry"));
        assert_eq!(
            plan.models.default.as_ref().unwrap().provider_id,
            "azure-foundry"
        );
    }

    #[test]
    fn plan_applies_knobs_on_first_import_or_nomination_change_only() {
        let knobs = [
            ("GEMINI_API_KEY", "k"),
            ("BLUEY_EMBEDDING_DIMENSIONS", "1536"),
            ("BLUEY_TRANSCRIPTION_PROVIDER", "apple"),
            ("RESEARCH_BACKEND", "claude"),
        ];
        let fresh = plan_env_import(&env_of(&knobs), &Settings::default());
        assert_eq!(fresh.embedding_dimensions, Some(1536));
        assert_eq!(
            fresh.transcription_provider,
            Some(TranscriptionProviderKind::Apple)
        );
        assert_eq!(fresh.research_backend, Some(ResearchBackend::Claude));

        // Already bootstrapped with the same provider: Settings edits win.
        let later = plan_env_import(&env_of(&knobs), &bootstrapped_settings("gemini"));
        assert!(later.embedding_dimensions.is_none());
        assert!(later.transcription_provider.is_none());
        assert!(later.research_backend.is_none());
    }

    #[test]
    fn plan_falls_back_with_a_warning_when_the_nominated_provider_has_no_key() {
        let env = env_of(&[
            ("AZURE_FOUNDRY_API_KEY", "f"),
            ("AZURE_FOUNDRY_ENDPOINT", "https://res.openai.azure.com"),
            ("BLUEY_AI_PROVIDER", "gemini"),
        ]);
        let plan = plan_env_import(&env, &Settings::default());
        assert_eq!(plan.bootstrap_provider.as_deref(), Some("azure-foundry"));
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("BLUEY_AI_PROVIDER=gemini"));
        assert_eq!(plan.explicit_base_urls, vec!["azure-foundry".to_string()]);
        assert!(plan.models.default.is_some());
    }

    #[test]
    fn a_foundry_claude_key_does_not_configure_the_anthropic_provider() {
        let env = env_of(&[("ANTHROPIC_FOUNDRY_API_KEY", "f")]);
        let plan = plan_env_import(&env, &Settings::default());
        assert!(plan.providers.is_empty());
        assert!(plan.keys.is_empty());
        assert!(plan.is_empty());
    }

    #[test]
    fn plan_ignores_invalid_dimension_transcription_and_backend_values() {
        let settings = Settings::default();
        let env = env_of(&[
            ("BLUEY_EMBEDDING_DIMENSIONS", "1000"),
            ("BLUEY_TRANSCRIPTION_PROVIDER", "whisper"),
            ("RESEARCH_BACKEND", "gpt"),
        ]);
        let plan = plan_env_import(&env, &settings);
        assert!(plan.embedding_dimensions.is_none());
        assert!(plan.transcription_provider.is_none());
        assert!(plan.research_backend.is_none());
        assert!(plan.is_empty());
    }

    #[test]
    fn parsers_accept_aliases() {
        assert_eq!(
            parse_provider_choice("Google").map(|p| p.id),
            Some("gemini")
        );
        assert_eq!(
            parse_provider_choice("azure_foundry").map(|p| p.id),
            Some("azure-foundry")
        );
        assert_eq!(
            parse_provider_choice("claude").map(|p| p.id),
            Some("anthropic")
        );
        assert_eq!(parse_provider_choice("nope"), None);
        assert_eq!(
            parse_research_backend("anthropic"),
            Some(ResearchBackend::Claude)
        );
        assert_eq!(parse_embedding_dimensions("768"), Some(768));
        assert_eq!(parse_embedding_dimensions("512"), None);
        assert_eq!(
            model_override_env(ModelRole::Embedding),
            "BLUEY_MODEL_EMBEDDING"
        );
    }
}
