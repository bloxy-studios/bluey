use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreferredLatency {
    UltraFast,
    Fast,
    Balanced,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextRequirement {
    Screen,
    Accessibility,
    Transcript,
    Resume,
    JobDescription,
    Documents,
    SessionMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResponseSchemaId {
    Answer,
    SuggestedResponse,
    Behavioral,
    Coding,
    SystemDesign,
    Case,
    Sales,
    Recruiting,
    Meeting,
    Lecture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ResponseLength {
    #[default]
    Concise,
    Balanced,
    Detailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ResponseTone {
    #[default]
    Natural,
    Professional,
    Technical,
    Conversational,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResponseStyle {
    pub length: ResponseLength,
    pub tone: ResponseTone,
}

/// Partial style override (mode-level).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResponseStylePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<ResponseLength>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<ResponseTone>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Default,
    Fast,
    Reasoning,
    Vision,
    Research,
    Transcription,
    Embedding,
}

impl ModelRole {
    pub const ALL: [ModelRole; 7] = [
        ModelRole::Default,
        ModelRole::Fast,
        ModelRole::Reasoning,
        ModelRole::Vision,
        ModelRole::Research,
        ModelRole::Transcription,
        ModelRole::Embedding,
    ];
}

/// Mirrors `BlueyMode`. Modes are data, not code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueyMode {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub system_instructions: String,
    pub response_schema: ResponseSchemaId,
    pub preferred_latency: PreferredLatency,
    pub context_requirements: Vec<ContextRequirement>,
    pub built_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_style: Option<ResponseStylePatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_model_role: Option<ModelRole>,
    #[serde(default)]
    pub attached_document_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Mirrors `ModeDraft` (all optional except name) and `ModePatch` (everything optional).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<ResponseSchemaId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_latency: Option<PreferredLatency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_requirements: Option<Vec<ContextRequirement>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_style: Option<ResponseStylePatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_model_role: Option<ModelRole>,
}

pub type ModeDraft = ModePatch;

pub const BUILT_IN_MODE_IDS: [&str; 10] = [
    "general",
    "interview",
    "behavioral-interview",
    "coding-interview",
    "system-design",
    "case-interview",
    "sales",
    "recruiting",
    "team-meeting",
    "lecture",
];

pub const DEFAULT_MODE_ID: &str = "general";
