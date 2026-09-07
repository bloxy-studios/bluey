//! Repositories — one per aggregate. All methods are stateless associated
//! functions taking `&Database`; the public API speaks `bluey_core` types and
//! SQL never leaves this module tree.

mod cache;
pub(crate) mod documents;
mod modes;
mod responses;
mod sessions;
mod settings;
mod snapshots;
mod transcript;

pub use cache::AiCacheRepository;
pub use documents::{DocumentRepository, EmbeddedChunk};
pub use modes::ModeRepository;
pub use responses::{AiRequestRecord, AiRequestRepository, ResponseRepository};
pub use sessions::{
    SessionEventRepository, SessionNoteRepository, SessionRepository, SummaryRepository,
};
pub use settings::{ModelConfigRepository, SettingsRepository, ShortcutRepository, UserRepository};
pub use snapshots::SnapshotRepository;
pub use transcript::TranscriptRepository;

use bluey_core::error::BlueyError;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Serialize a unit enum to its serde string tag (e.g. `ResponseSchemaId::Answer` → `"answer"`).
pub(crate) fn to_enum_str<T: Serialize>(value: &T) -> Result<String, BlueyError> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(s) => Ok(s),
        other => Err(BlueyError::internal(format!(
            "expected string-serializable enum, got {other}"
        ))),
    }
}

/// Parse a unit enum from its serde string tag.
pub(crate) fn from_enum_str<T: DeserializeOwned>(s: &str) -> Result<T, BlueyError> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|e| BlueyError::storage("query", format!("invalid enum value '{s}': {e}")))
}

/// Serialize any value to a JSON string column.
pub(crate) fn to_json_string<T: Serialize>(value: &T) -> Result<String, BlueyError> {
    serde_json::to_string(value).map_err(BlueyError::from)
}

/// Serialize an optional value to an optional JSON string column.
pub(crate) fn opt_to_json<T: Serialize>(value: &Option<T>) -> Result<Option<String>, BlueyError> {
    value.as_ref().map(to_json_string).transpose()
}

/// Deserialize an optional JSON string column.
pub(crate) fn opt_from_json<T: DeserializeOwned>(
    value: Option<String>,
) -> Result<Option<T>, BlueyError> {
    match value {
        None => Ok(None),
        Some(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| BlueyError::storage("query", format!("invalid stored JSON: {e}"))),
    }
}

/// Deserialize a required JSON string column.
pub(crate) fn from_json_str<T: DeserializeOwned>(s: &str) -> Result<T, BlueyError> {
    serde_json::from_str(s)
        .map_err(|e| BlueyError::storage("query", format!("invalid stored JSON: {e}")))
}

/// `storage.not_found` error for a missing row.
pub(crate) fn not_found(entity: &str, id: &str) -> BlueyError {
    BlueyError::storage("not_found", format!("{entity} '{id}' was not found"))
}
