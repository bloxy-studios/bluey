use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Resume,
    Cv,
    Bio,
    Portfolio,
    Skills,
    Experience,
    PersonalInstructions,
    JobDescription,
    CompanyNotes,
    RoleDescription,
    Notes,
    Other,
}

impl DocumentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Cv => "cv",
            Self::Bio => "bio",
            Self::Portfolio => "portfolio",
            Self::Skills => "skills",
            Self::Experience => "experience",
            Self::PersonalInstructions => "personal_instructions",
            Self::JobDescription => "job_description",
            Self::CompanyNotes => "company_notes",
            Self::RoleDescription => "role_description",
            Self::Notes => "notes",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentScope {
    Global,
    Session,
    Mode,
}

impl DocumentScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Session => "session",
            Self::Mode => "mode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentFormat {
    Pdf,
    Docx,
    Txt,
    Md,
    Text,
}

impl DocumentFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Txt => "txt",
            Self::Md => "md",
            Self::Text => "text",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "docx" => Some(Self::Docx),
            "txt" => Some(Self::Txt),
            "md" | "markdown" => Some(Self::Md),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentIndexStatus {
    Pending,
    Indexed,
    Failed,
}

/// Mirrors `BlueyDocument`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueyDocument {
    pub id: String,
    pub title: String,
    pub kind: DocumentKind,
    pub format: DocumentFormat,
    pub scope: DocumentScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub size_bytes: u64,
    pub chunk_count: u32,
    pub index_status: DocumentIndexStatus,
    pub has_embeddings: bool,
    /// `providerId/model` that produced the chunk vectors (`None` until embedded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    /// Vector length the chunks were embedded with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_dimensions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
    pub created_at: String,
    pub updated_at: String,
}

/// Mirrors `DocumentChunk`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentChunk {
    pub id: String,
    pub document_id: String,
    pub index: u32,
    pub content: String,
    pub tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
}

/// Mirrors `AddDocumentInput`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddDocumentInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub kind: DocumentKind,
    pub scope: DocumentScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<DocumentFormat>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeRef {
    pub scope: DocumentScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RetrievalStrategy {
    #[default]
    Auto,
    Keyword,
    Semantic,
}

/// Mirrors `RetrievalQuery`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalQuery {
    pub query: String,
    pub scopes: Vec<ScopeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<DocumentKind>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<RetrievalStrategy>,
}
