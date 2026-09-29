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
    /// Sidebar group: absent keeps it, `null` (or blank) clears it.
    #[serde(
        default,
        deserialize_with = "present_or_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub group: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_style: Option<ResponseStylePatch>,
    /// Model role: absent keeps it, `null` clears it ("Auto model").
    #[serde(
        default,
        deserialize_with = "present_or_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub preferred_model_role: Option<Option<ModelRole>>,
}

/// Deserializes a field that is present — including an explicit `null` — as
/// `Some(..)`, so `Option<Option<T>>` tells "absent: keep" (`None`) from
/// "`null`: clear" (`Some(None)`).
fn present_or_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(json: &str) -> ModePatch {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn nullable_patch_fields_tell_absent_from_cleared() {
        let absent = patch("{}");
        assert_eq!(absent.group, None);
        assert_eq!(absent.preferred_model_role, None);

        let cleared = patch(r#"{"group":null,"preferredModelRole":null}"#);
        assert_eq!(cleared.group, Some(None));
        assert_eq!(cleared.preferred_model_role, Some(None));

        let set = patch(r#"{"group":"Work","preferredModelRole":"reasoning"}"#);
        assert_eq!(set.group, Some(Some("Work".into())));
        assert_eq!(set.preferred_model_role, Some(Some(ModelRole::Reasoning)));

        // A clear survives a serialize/deserialize round trip.
        let json = serde_json::to_string(&cleared).unwrap();
        assert_eq!(patch(&json), cleared);
    }
}
