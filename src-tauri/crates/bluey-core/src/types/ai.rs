use serde::{Deserialize, Serialize};

use super::mode::ModelRole;
use super::response::Citation;
use crate::error::BlueyError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiTask {
    Vision,
    Classification,
    Transcription,
    Answer,
    Coding,
    SystemDesign,
    Summarization,
    Research,
    DeepReasoning,
    Embedding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LatencyBudget {
    UltraFast,
    #[default]
    Fast,
    Balanced,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningLevel {
    #[default]
    None,
    Light,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    /// Google AI Studio (Gemini API) — the default provider (ADR 0007).
    GoogleGemini,
    AzureFoundry,
    Anthropic,
    OpenaiCompatible,
    Mock,
}

/// Mirrors `AIProviderConfig`. Never contains the API key (kept in the OS keychain).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderConfig {
    pub id: String,
    pub kind: AiProviderKind,
    pub name: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployments: Option<std::collections::BTreeMap<String, String>>,
    pub enabled: bool,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelAssignment {
    pub provider_id: String,
    pub model: String,
}

/// Mirrors `ModelRoleAssignments` (`Record<ModelRole, ModelAssignment | null>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelRoleAssignments {
    pub default: Option<ModelAssignment>,
    pub fast: Option<ModelAssignment>,
    pub reasoning: Option<ModelAssignment>,
    pub vision: Option<ModelAssignment>,
    pub research: Option<ModelAssignment>,
    pub transcription: Option<ModelAssignment>,
    pub embedding: Option<ModelAssignment>,
}

impl ModelRoleAssignments {
    pub fn get(&self, role: ModelRole) -> Option<&ModelAssignment> {
        match role {
            ModelRole::Default => self.default.as_ref(),
            ModelRole::Fast => self.fast.as_ref(),
            ModelRole::Reasoning => self.reasoning.as_ref(),
            ModelRole::Vision => self.vision.as_ref(),
            ModelRole::Research => self.research.as_ref(),
            ModelRole::Transcription => self.transcription.as_ref(),
            ModelRole::Embedding => self.embedding.as_ref(),
        }
    }

    pub fn set(&mut self, role: ModelRole, assignment: Option<ModelAssignment>) {
        match role {
            ModelRole::Default => self.default = assignment,
            ModelRole::Fast => self.fast = assignment,
            ModelRole::Reasoning => self.reasoning = assignment,
            ModelRole::Vision => self.vision = assignment,
            ModelRole::Research => self.research = assignment,
            ModelRole::Transcription => self.transcription = assignment,
            ModelRole::Embedding => self.embedding = assignment,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageMediaType {
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/webp")]
    Webp,
}

impl ImageMediaType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }
}

/// Mirrors `AIContentPart` (tag = "type").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AiContentPart {
    Text {
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Image {
        media_type: ImageMediaType,
        data: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiMessage {
    pub role: AiRole,
    pub content: Vec<AiContentPart>,
}

impl AiMessage {
    pub fn text(role: AiRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![AiContentPart::Text { text: text.into() }],
        }
    }

    pub fn has_images(&self) -> bool {
        self.content
            .iter()
            .any(|p| matches!(p, AiContentPart::Image { .. }))
    }

    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|p| match p {
                AiContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonSchemaSpec {
    pub name: String,
    pub schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// Mirrors `AIRequest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRequest {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub generation: u64,
    pub task: AiTask,
    pub latency_budget: LatencyBudget,
    pub reasoning: ReasoningLevel,
    pub vision_required: bool,
    pub context_tokens: u32,
    pub messages: Vec<AiMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<JsonSchemaSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<ModelAssignment>,
    pub created_at: String,
}

/// Mirrors `ModelSelection`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelection {
    pub provider_id: String,
    pub provider_kind: AiProviderKind,
    pub model: String,
    pub role: ModelRole,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FinishReason {
    Stop,
    Length,
    Cancelled,
    Error,
}

/// Mirrors `AIChunk` (tag = "type").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiChunk {
    #[serde(rename_all = "camelCase")]
    Started {
        request_id: String,
        selection: ModelSelection,
    },
    #[serde(rename_all = "camelCase")]
    Delta { request_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    Usage {
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Completed {
        request_id: String,
        finish_reason: FinishReason,
        total_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        time_to_first_token_ms: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        request_id: String,
        error: BlueyError,
    },
}

/// Mirrors `AIResponse` (non-streaming convenience).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiResponse {
    pub request_id: String,
    pub selection: ModelSelection,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<serde_json::Value>,
    pub finish_reason: FinishReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
    pub total_ms: u64,
}

/// Mirrors `ConnectionTestResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestResult {
    pub ok: bool,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<BlueyError>,
}

// ── Research ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResearchDepth {
    #[default]
    None,
    Search,
    SearchScrape,
    DeepAgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchSource {
    Exa,
    Firecrawl,
    Mock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    pub source: SearchSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeResult {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub markdown: String,
    pub source: SearchSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchTool {
    ExaSearch,
    FirecrawlScrape,
    DocumentRead,
}

/// Mirrors `DeepResearchRequest`. `query`/`goal` must be public (no private context).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepResearchRequest {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub query: String,
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    pub tools: Vec<ResearchTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_document_ids: Option<Vec<String>>,
}

/// Mirrors `DeepResearchEvent` (tag = "type").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeepResearchEvent {
    #[serde(rename_all = "camelCase")]
    Started { job_id: String },
    #[serde(rename_all = "camelCase")]
    Progress { job_id: String, message: String },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        job_id: String,
        tool: String,
        input: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    TextDelta { job_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    Completed {
        job_id: String,
        report: String,
        citations: Vec<Citation>,
        total_ms: u64,
        turns: u32,
    },
    #[serde(rename_all = "camelCase")]
    Failed { job_id: String, error: BlueyError },
}
