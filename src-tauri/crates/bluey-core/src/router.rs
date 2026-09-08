//! Model routing policy (spec §35): pick a provider+model for a request from
//! the task type, latency budget, reasoning level, context size and vision
//! requirement — plus the user's role assignments. Pure decision logic, no I/O.

use crate::error::BlueyError;
use crate::types::{
    AiProviderConfig, AiProviderKind, AiTask, LatencyBudget, ModelAssignment, ModelRole,
    ModelRoleAssignments, ModelSelection, ReasoningLevel,
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
            continue;
        };
        if !provider_usable(provider) {
            reason.push_str(&format!(
                "; provider {} for role {} disabled or missing API key",
                provider.id,
                role_str(*candidate)
            ));
            continue;
        }
        if needs_vision && !provider_supports_vision(provider.kind) {
            reason.push_str(&format!(
                "; provider {} for role {} cannot read images",
                provider.id,
                role_str(*candidate)
            ));
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

    Err(BlueyError::configuration(
        "no_model",
        format!(
            "no usable model for role {}; add a provider and assign models in Settings → AI",
            role_str(role)
        ),
    ))
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
        assert_eq!(e.code, "config.no_model");
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
