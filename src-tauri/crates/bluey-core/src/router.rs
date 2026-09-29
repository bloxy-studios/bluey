//! Model routing policy (spec §35): pick a provider+model for a request from
//! the task type, latency budget, reasoning level, context size and vision
//! requirement — plus the user's role assignments. Pure decision logic, no I/O.

use crate::error::BlueyError;
use crate::presets;
use crate::types::{
    AiProviderConfig, AiProviderKind, AiTask, LatencyBudget, ModelAssignment, ModelRole,
    ModelRoleAssignments, ModelSelection, ProviderAuthMethod, ReasoningLevel,
};

/// Above this context size, summarization is routed to the default role
/// instead of the fast role (fast models tend to have smaller windows).
pub const SUMMARIZATION_FAST_MAX_CONTEXT_TOKENS: u32 = 24_000;

/// Everything the router needs to make a decision.
#[derive(Debug, Clone)]
pub struct RoutingInput<'a> {
    /// What kind of work the request is.
    pub task: AiTask,
    /// How fast the answer must feel.
    pub latency: LatencyBudget,
    /// How much explicit reasoning the request wants.
    pub reasoning: ReasoningLevel,
    /// Estimated prompt/context size in tokens.
    pub context_tokens: u32,
    /// The request includes images that the model must be able to read.
    pub vision_required: bool,
    /// Mode-level preference (e.g. system-design mode prefers `reasoning`).
    pub preferred_role: Option<ModelRole>,
    /// Explicit per-request override; used verbatim when its provider is usable.
    pub model_override: Option<&'a ModelAssignment>,
}

/// Whether models on this provider kind can read images. All real provider
/// kinds can host vision-capable deployments; the mock provider pretends to.
pub fn provider_supports_vision(kind: AiProviderKind) -> bool {
    match kind {
        AiProviderKind::GoogleGemini
        | AiProviderKind::AzureFoundry
        | AiProviderKind::Anthropic
        | AiProviderKind::OpenaiCompatible
        | AiProviderKind::ChatgptCodex
        | AiProviderKind::ClaudeSubscription
        | AiProviderKind::AntigravityGoogle
        | AiProviderKind::Mock => true,
    }
}

/// Pick the provider+model for `input`.
///
/// Policy: classification→fast; answer→fast (default at balanced/deep
/// latency); coding→default; system_design→reasoning when reasoning ≥ light,
/// else default; deep_reasoning→reasoning; research→research;
/// summarization→fast (default above 24k context tokens); vision→vision;
/// embedding→embedding; transcription→transcription. A mode's
/// `preferred_role` overrides the derived role for text-generation tasks;
/// `vision_required` forces the vision role. Unassigned roles fall back:
/// fast→default, reasoning→default, vision→default, research→reasoning→default.
/// Providers that are disabled or lack an API key are skipped (mock never
/// needs a key). When nothing usable remains: `config.no_model`.
pub fn select(
    input: &RoutingInput,
    assignments: &ModelRoleAssignments,
    providers: &[AiProviderConfig],
) -> Result<ModelSelection, BlueyError> {
    let needs_vision = input.vision_required || input.task == AiTask::Vision;
    let (role, mut reason) = desired_role(input, needs_vision);

    // Explicit override wins when its provider is usable.
    if let Some(overridden) = input.model_override {
        return match providers.iter().find(|p| p.id == overridden.provider_id) {
            Some(p) if provider_usable(p) && (!needs_vision || provider_supports_vision(p.kind)) => {
                Ok(ModelSelection {
                    provider_id: p.id.clone(),
                    provider_kind: p.kind,
                    model: overridden.model.clone(),
                    role,
                    reason: format!(
                        "model override → provider {} model {}",
                        overridden.provider_id, overridden.model
                    ),
                })
            }
            _ => Err(BlueyError::configuration(
                "no_model",
                "the model override references a provider that is missing, disabled or has no API key",
            )),
        };
    }

    // The first assigned provider the chain had to skip: it names the failure
    // and decides whether the API-key fallback below applies.
    let mut blocked: Option<Blocked> = None;
    for candidate in fallback_chain(role) {
        let Some(assignment) = assignments.get(*candidate) else {
            reason.push_str(&format!("; role {} unassigned", role_str(*candidate)));
            continue;
        };
        let Some(provider) = providers.iter().find(|p| p.id == assignment.provider_id) else {
            reason.push_str(&format!(
                "; provider {} for role {} not configured",
                assignment.provider_id,
                role_str(*candidate)
            ));
            blocked
                .get_or_insert_with(|| Blocked::new(&assignment.provider_id, None, needs_vision));
            continue;
        };
        if !provider_usable(provider) {
            reason.push_str(&format!(
                "; provider {} for role {} disabled or missing API key",
                provider.id,
                role_str(*candidate)
            ));
            blocked.get_or_insert_with(|| Blocked::new(&provider.id, Some(provider), needs_vision));
            continue;
        }
        if needs_vision && !provider_supports_vision(provider.kind) {
            reason.push_str(&format!(
                "; provider {} for role {} cannot read images",
                provider.id,
                role_str(*candidate)
            ));
            blocked.get_or_insert_with(|| Blocked::new(&provider.id, Some(provider), needs_vision));
            continue;
        }
        if *candidate != role {
            reason.push_str(&format!(" → fallback {}", role_str(*candidate)));
        }
        return Ok(ModelSelection {
            provider_id: provider.id.clone(),
            provider_kind: provider.kind,
            model: assignment.model.clone(),
            role: *candidate,
            reason,
        });
    }

    let Some(blocked) = blocked else {
        return Err(BlueyError::configuration(
            "no_model",
            format!(
                "no usable model for role {}; add a provider and assign models in Settings → AI",
                role_str(role)
            ),
        ));
    };

    // A subscription account that stopped serving (or a provider that is gone)
    // must not strand the role: the same role runs on a usable API-key provider.
    if blocked.allows_api_key_fallback() {
        if let Some((provider, model)) =
            api_key_fallback(role, needs_vision, assignments, providers)
        {
            reason.push_str(&format!(
                " → fallback {} ({} unavailable)",
                provider.id, blocked.provider_id
            ));
            return Ok(ModelSelection {
                provider_id: provider.id.clone(),
                provider_kind: provider.kind,
                model,
                role,
                reason,
            });
        }
    }
    Err(blocked.into_error(role))
}

/// Derive the desired role from the routing input, with a human-readable reason.
fn desired_role(input: &RoutingInput, needs_vision: bool) -> (ModelRole, String) {
    let (mut role, mut reason) = match input.task {
        AiTask::Classification => (
            ModelRole::Fast,
            "task=classification → role fast".to_string(),
        ),
        AiTask::Answer => match input.latency {
            LatencyBudget::Balanced | LatencyBudget::Deep => (
                ModelRole::Default,
                format!(
                    "task=answer latency={} → role default",
                    latency_str(input.latency)
                ),
            ),
            LatencyBudget::UltraFast | LatencyBudget::Fast => (
                ModelRole::Fast,
                format!(
                    "task=answer latency={} → role fast",
                    latency_str(input.latency)
                ),
            ),
        },
        AiTask::Coding => (ModelRole::Default, "task=coding → role default".to_string()),
        AiTask::SystemDesign => match input.reasoning {
            ReasoningLevel::Light | ReasoningLevel::Deep => (
                ModelRole::Reasoning,
                format!(
                    "task=system_design reasoning={} → role reasoning",
                    reasoning_str(input.reasoning)
                ),
            ),
            ReasoningLevel::None => (
                ModelRole::Default,
                "task=system_design reasoning=none → role default".to_string(),
            ),
        },
        AiTask::DeepReasoning => (
            ModelRole::Reasoning,
            "task=deep_reasoning → role reasoning".to_string(),
        ),
        AiTask::Research => (
            ModelRole::Research,
            "task=research → role research".to_string(),
        ),
        AiTask::Summarization => {
            if input.context_tokens > SUMMARIZATION_FAST_MAX_CONTEXT_TOKENS {
                (
                    ModelRole::Default,
                    format!(
                        "task=summarization context_tokens={} > 24k → role default",
                        input.context_tokens
                    ),
                )
            } else {
                (
                    ModelRole::Fast,
                    "task=summarization → role fast".to_string(),
                )
            }
        }
        AiTask::Vision => (ModelRole::Vision, "task=vision → role vision".to_string()),
        AiTask::Embedding => (
            ModelRole::Embedding,
            "task=embedding → role embedding".to_string(),
        ),
        AiTask::Transcription => (
            ModelRole::Transcription,
            "task=transcription → role transcription".to_string(),
        ),
    };

    // Mode preference overrides the derived role for text-generation tasks only.
    if let Some(preferred) = input.preferred_role {
        let text_generation = matches!(
            input.task,
            AiTask::Answer
                | AiTask::Coding
                | AiTask::SystemDesign
                | AiTask::DeepReasoning
                | AiTask::Summarization
        );
        if text_generation && preferred != role {
            role = preferred;
            reason.push_str(&format!("; mode prefers role {}", role_str(preferred)));
        }
    }

    // A vision requirement trumps everything else.
    if needs_vision && role != ModelRole::Vision {
        role = ModelRole::Vision;
        reason.push_str("; vision required → role vision");
    }

    (role, reason)
}

/// Roles to try in order when the desired role has no usable assignment.
fn fallback_chain(role: ModelRole) -> &'static [ModelRole] {
    match role {
        ModelRole::Fast => &[ModelRole::Fast, ModelRole::Default],
        ModelRole::Reasoning => &[ModelRole::Reasoning, ModelRole::Default],
        ModelRole::Vision => &[ModelRole::Vision, ModelRole::Default],
        ModelRole::Research => &[
            ModelRole::Research,
            ModelRole::Reasoning,
            ModelRole::Default,
        ],
        ModelRole::Default => &[ModelRole::Default],
        ModelRole::Transcription => &[ModelRole::Transcription],
        ModelRole::Embedding => &[ModelRole::Embedding],
    }
}

fn provider_usable(p: &AiProviderConfig) -> bool {
    p.enabled && (p.has_api_key || p.kind == AiProviderKind::Mock)
}

/// The API-key provider that stands in for an unusable account: the first
/// usable one in settings order (the reserved Gemini provider first), with its
/// kind's recommended model for the role — or, for kinds without presets
/// (OpenAI-compatible), the model the user already assigned it for another
/// text role.
fn api_key_fallback<'p>(
    role: ModelRole,
    needs_vision: bool,
    assignments: &ModelRoleAssignments,
    providers: &'p [AiProviderConfig],
) -> Option<(&'p AiProviderConfig, String)> {
    let usable = |p: &&AiProviderConfig| {
        p.auth_method == ProviderAuthMethod::ApiKey
            && p.kind != AiProviderKind::Mock
            && provider_usable(p)
            && (!needs_vision || provider_supports_vision(p.kind))
    };
    let model_for = |p: &AiProviderConfig| {
        let preset = presets::by_kind(p.kind).and_then(|preset| {
            fallback_chain(role)
                .iter()
                .find_map(|r| preset.model_for(*r))
        });
        let text_role = TEXT_ROLES.contains(&role);
        preset.map(str::to_string).or_else(|| {
            TEXT_ROLES
                .iter()
                .filter(|_| text_role)
                .filter_map(|r| assignments.get(*r))
                .find(|a| a.provider_id == p.id)
                .map(|a| a.model.clone())
        })
    };
    let gemini_first = providers
        .iter()
        .filter(|p| p.id == presets::GEMINI_ID)
        .chain(providers.iter().filter(|p| p.id != presets::GEMINI_ID));
    gemini_first
        .filter(usable)
        .find_map(|p| model_for(p).map(|model| (p, model)))
}

/// Roles whose models generate text (a stand-in model must be one of these).
const TEXT_ROLES: [ModelRole; 5] = [
    ModelRole::Default,
    ModelRole::Fast,
    ModelRole::Reasoning,
    ModelRole::Vision,
    ModelRole::Research,
];

/// An assigned provider the chain skipped, and why (`cause` is the
/// `config.provider_unusable` detail the WebView words its copy from).
struct Blocked {
    provider_id: String,
    provider_name: Option<String>,
    is_account: bool,
    cause: &'static str,
}

impl Blocked {
    fn new(provider_id: &str, provider: Option<&AiProviderConfig>, needs_vision: bool) -> Self {
        let cause = match provider {
            None => "not_configured",
            Some(p) if p.auth_method == ProviderAuthMethod::OauthSubscription => {
                "account_unavailable"
            }
            Some(p) if !p.enabled => "disabled",
            Some(p) if !provider_usable(p) => "missing_key",
            Some(_) if needs_vision => "no_vision",
            Some(_) => "unusable",
        };
        Self {
            provider_id: provider_id.to_string(),
            provider_name: provider.map(|p| p.name.clone()),
            is_account: provider
                .is_some_and(|p| p.auth_method == ProviderAuthMethod::OauthSubscription),
            cause,
        }
    }

    /// Only an account (or a provider that no longer exists) reroutes on its
    /// own; a disabled or keyless API-key provider is the user's to fix.
    fn allows_api_key_fallback(&self) -> bool {
        self.is_account || self.cause == "not_configured"
    }

    fn into_error(self, role: ModelRole) -> BlueyError {
        let name = self.provider_name.as_deref().unwrap_or(&self.provider_id);
        BlueyError::configuration(
            "provider_unusable",
            format!(
                "role {} is assigned to {name}, which cannot serve requests ({})",
                role_str(role),
                self.cause
            ),
        )
        .with_details(serde_json::json!({
            "providerId": self.provider_id,
            "providerName": name,
            "cause": self.cause,
            "role": role_str(role),
        }))
    }
}

fn role_str(role: ModelRole) -> &'static str {
    match role {
        ModelRole::Default => "default",
        ModelRole::Fast => "fast",
        ModelRole::Reasoning => "reasoning",
        ModelRole::Vision => "vision",
        ModelRole::Research => "research",
        ModelRole::Transcription => "transcription",
        ModelRole::Embedding => "embedding",
    }
}

fn latency_str(latency: LatencyBudget) -> &'static str {
    match latency {
        LatencyBudget::UltraFast => "ultra-fast",
        LatencyBudget::Fast => "fast",
        LatencyBudget::Balanced => "balanced",
        LatencyBudget::Deep => "deep",
    }
}

fn reasoning_str(level: ReasoningLevel) -> &'static str {
    match level {
        ReasoningLevel::None => "none",
        ReasoningLevel::Light => "light",
        ReasoningLevel::Deep => "deep",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BlueyErrorKind;
    use pretty_assertions::assert_eq;

    fn provider(id: &str, kind: AiProviderKind, enabled: bool, has_key: bool) -> AiProviderConfig {
        AiProviderConfig {
            id: id.into(),
            kind,
            name: id.into(),
            base_url: "https://example.invalid".into(),
            api_version: None,
            deployments: None,
            enabled,
            has_api_key: has_key,
            auth_method: Default::default(),
        }
    }

    fn assignment(provider: &str, model: &str) -> ModelAssignment {
        ModelAssignment {
            provider_id: provider.into(),
            model: model.into(),
        }
    }

    fn full_assignments() -> ModelRoleAssignments {
        ModelRoleAssignments {
            default: Some(assignment("p1", "gpt-default")),
            fast: Some(assignment("p1", "gpt-fast")),
            reasoning: Some(assignment("p1", "gpt-reasoning")),
            vision: Some(assignment("p1", "gpt-vision")),
            research: Some(assignment("p1", "gpt-research")),
            transcription: Some(assignment("p1", "whisper")),
            embedding: Some(assignment("p1", "embed")),
        }
    }

    fn providers() -> Vec<AiProviderConfig> {
        vec![provider("p1", AiProviderKind::OpenaiCompatible, true, true)]
    }

    fn input(task: AiTask) -> RoutingInput<'static> {
        RoutingInput {
            task,
            latency: LatencyBudget::Fast,
            reasoning: ReasoningLevel::None,
            context_tokens: 2_000,
            vision_required: false,
            preferred_role: None,
            model_override: None,
        }
    }

    fn pick(i: &RoutingInput) -> ModelSelection {
        select(i, &full_assignments(), &providers()).expect("selection")
    }

    #[test]
    fn routes_each_task_to_its_role() {
        assert_eq!(pick(&input(AiTask::Classification)).role, ModelRole::Fast);
        assert_eq!(pick(&input(AiTask::Answer)).role, ModelRole::Fast);
        assert_eq!(pick(&input(AiTask::Coding)).role, ModelRole::Default);
        assert_eq!(
            pick(&input(AiTask::DeepReasoning)).role,
            ModelRole::Reasoning
        );
        assert_eq!(pick(&input(AiTask::Research)).role, ModelRole::Research);
        assert_eq!(pick(&input(AiTask::Summarization)).role, ModelRole::Fast);
        assert_eq!(pick(&input(AiTask::Vision)).role, ModelRole::Vision);
        assert_eq!(pick(&input(AiTask::Embedding)).role, ModelRole::Embedding);
        assert_eq!(
            pick(&input(AiTask::Transcription)).role,
            ModelRole::Transcription
        );
    }

    #[test]
    fn answer_latency_controls_role() {
        let mut i = input(AiTask::Answer);
        i.latency = LatencyBudget::UltraFast;
        assert_eq!(pick(&i).role, ModelRole::Fast);
        i.latency = LatencyBudget::Balanced;
        assert_eq!(pick(&i).role, ModelRole::Default);
        i.latency = LatencyBudget::Deep;
        assert_eq!(pick(&i).role, ModelRole::Default);
        assert!(pick(&i).reason.contains("latency=deep"));
    }

    #[test]
    fn system_design_needs_reasoning_level() {
        let mut i = input(AiTask::SystemDesign);
        assert_eq!(pick(&i).role, ModelRole::Default);
        i.reasoning = ReasoningLevel::Light;
        assert_eq!(pick(&i).role, ModelRole::Reasoning);
        i.reasoning = ReasoningLevel::Deep;
        assert_eq!(pick(&i).role, ModelRole::Reasoning);
    }

    #[test]
    fn big_summarization_context_uses_default_role() {
        let mut i = input(AiTask::Summarization);
        i.context_tokens = 30_000;
        let s = pick(&i);
        assert_eq!(s.role, ModelRole::Default);
        assert!(s.reason.contains("24k"));
    }

    #[test]
    fn unassigned_roles_fall_back() {
        let mut a = full_assignments();
        a.fast = None;
        let s = select(&input(AiTask::Answer), &a, &providers()).expect("fallback");
        assert_eq!(s.role, ModelRole::Default);
        assert!(s.reason.contains("unassigned"));

        a.vision = None;
        let s = select(&input(AiTask::Vision), &a, &providers()).expect("vision fallback");
        assert_eq!(s.role, ModelRole::Default);

        // research → reasoning → default
        a.research = None;
        let s = select(&input(AiTask::Research), &a, &providers()).expect("research fallback");
        assert_eq!(s.role, ModelRole::Reasoning);
        a.reasoning = None;
        let s = select(&input(AiTask::Research), &a, &providers()).expect("research fallback 2");
        assert_eq!(s.role, ModelRole::Default);
    }

    #[test]
    fn disabled_or_keyless_providers_are_skipped() {
        let providers = vec![
            provider("p1", AiProviderKind::OpenaiCompatible, false, true), // disabled
            provider("p2", AiProviderKind::Anthropic, true, false),        // no key
            provider("p3", AiProviderKind::Anthropic, true, true),
        ];
        let a = ModelRoleAssignments {
            fast: Some(assignment("p1", "m-fast")),
            default: Some(assignment("p3", "m-default")),
            ..Default::default()
        };
        let s = select(&input(AiTask::Answer), &a, &providers).expect("skip to default");
        assert_eq!(s.provider_id, "p3");
        assert_eq!(s.role, ModelRole::Default);

        // Keyless non-mock is unusable even as the last resort.
        let a2 = ModelRoleAssignments {
            default: Some(assignment("p2", "m")),
            ..Default::default()
        };
        let e = select(&input(AiTask::Answer), &a2, &providers).expect_err("no usable model");
        assert_eq!(e.kind, BlueyErrorKind::Configuration);
        assert_eq!(e.code, "config.provider_unusable");
        let details = e.details.expect("details");
        assert_eq!(details["providerId"], "p2");
        assert_eq!(details["cause"], "missing_key");
    }

    fn account(id: &str, kind: AiProviderKind, usable: bool) -> AiProviderConfig {
        AiProviderConfig {
            auth_method: ProviderAuthMethod::OauthSubscription,
            ..provider(id, kind, true, usable)
        }
    }

    fn default_on(provider_id: &str) -> ModelRoleAssignments {
        ModelRoleAssignments {
            default: Some(assignment(provider_id, "claude-sonnet-5")),
            ..Default::default()
        }
    }

    #[test]
    fn an_unusable_account_falls_back_to_an_api_key_provider() {
        let providers = vec![
            provider("azure-foundry", AiProviderKind::AzureFoundry, true, false),
            provider("gemini", AiProviderKind::GoogleGemini, true, true),
            account("claude", AiProviderKind::ClaudeSubscription, false),
        ];
        let mut i = input(AiTask::Answer);
        i.latency = LatencyBudget::Balanced;
        let s = select(&i, &default_on("claude"), &providers).expect("api-key fallback");
        assert_eq!(s.provider_id, "gemini");
        assert_eq!(s.model, "gemini-3.8-flash");
        assert_eq!(s.role, ModelRole::Default);
        assert!(
            s.reason.contains("→ fallback gemini (claude unavailable)"),
            "{}",
            s.reason
        );

        // The account layer switched off: the provider is gone, same fallback.
        let without_account = &providers[..2];
        let s = select(&i, &default_on("claude"), without_account).expect("fallback");
        assert_eq!(s.provider_id, "gemini");
    }

    #[test]
    fn an_unusable_account_without_an_api_key_provider_names_the_account() {
        let providers = vec![
            provider("gemini", AiProviderKind::GoogleGemini, true, false),
            account("claude", AiProviderKind::ClaudeSubscription, false),
        ];
        let e = select(&input(AiTask::Coding), &default_on("claude"), &providers)
            .expect_err("nothing usable");
        assert_eq!(e.code, "config.provider_unusable");
        let details = e.details.expect("details");
        assert_eq!(details["providerId"], "claude");
        assert_eq!(details["cause"], "account_unavailable");
        assert_eq!(details["role"], "default");
    }

    #[test]
    fn a_disabled_api_key_provider_is_reported_not_rerouted() {
        let providers = vec![
            provider("gemini", AiProviderKind::GoogleGemini, true, true),
            provider("anthropic", AiProviderKind::Anthropic, false, true),
        ];
        let e = select(&input(AiTask::Coding), &default_on("anthropic"), &providers)
            .expect_err("the user's choice stays visible");
        assert_eq!(e.code, "config.provider_unusable");
        assert_eq!(e.details.expect("details")["cause"], "disabled");
    }

    #[test]
    fn mock_provider_needs_no_key() {
        let providers = vec![provider("mock", AiProviderKind::Mock, true, false)];
        let a = ModelRoleAssignments {
            default: Some(assignment("mock", "mock-model")),
            ..Default::default()
        };
        let s = select(&input(AiTask::Coding), &a, &providers).expect("mock usable");
        assert_eq!(s.provider_kind, AiProviderKind::Mock);
    }

    #[test]
    fn nothing_assigned_is_a_configuration_error() {
        let e = select(
            &input(AiTask::Answer),
            &ModelRoleAssignments::default(),
            &providers(),
        )
        .expect_err("no model");
        assert_eq!(e.code, "config.no_model");
        assert!(e.recoverable);
    }

    #[test]
    fn override_wins_and_unusable_override_errors() {
        let ov = assignment("p1", "my-exact-model");
        let mut i = input(AiTask::Answer);
        i.model_override = Some(&ov);
        let s = select(&i, &ModelRoleAssignments::default(), &providers()).expect("override");
        assert_eq!(s.model, "my-exact-model");
        assert!(s.reason.contains("override"));

        let bad = assignment("missing", "m");
        i.model_override = Some(&bad);
        let e = select(&i, &full_assignments(), &providers()).expect_err("bad override");
        assert_eq!(e.code, "config.no_model");
    }

    #[test]
    fn preferred_role_applies_to_generation_tasks_only() {
        let mut i = input(AiTask::Answer);
        i.preferred_role = Some(ModelRole::Reasoning);
        assert_eq!(pick(&i).role, ModelRole::Reasoning);

        let mut i = input(AiTask::Classification);
        i.preferred_role = Some(ModelRole::Reasoning);
        assert_eq!(
            pick(&i).role,
            ModelRole::Fast,
            "classification ignores mode preference"
        );
    }

    #[test]
    fn vision_requirement_forces_vision_role() {
        let mut i = input(AiTask::Answer);
        i.vision_required = true;
        let s = pick(&i);
        assert_eq!(s.role, ModelRole::Vision);
        assert!(s.reason.contains("vision required"));
    }

    #[test]
    fn all_provider_kinds_support_vision() {
        for kind in [
            AiProviderKind::GoogleGemini,
            AiProviderKind::AzureFoundry,
            AiProviderKind::Anthropic,
            AiProviderKind::OpenaiCompatible,
            AiProviderKind::Mock,
        ] {
            assert!(provider_supports_vision(kind));
        }
    }
}
