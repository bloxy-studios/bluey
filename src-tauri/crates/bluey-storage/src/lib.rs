//! `bluey-storage` — SQLite persistence for Bluey.
//!
//! * [`db`] — connection management (WAL, foreign keys, busy timeout), migrations.
//! * [`repositories`] — one repository per aggregate (sessions, transcript, modes, documents,
//!   responses, settings, snapshots, events, feedback).
//! * [`search`] — FTS5-backed session search.
//! * [`documents`] — parsing (PDF/DOCX/TXT/MD), chunking, indexing, keyword + embedding retrieval.
//! * [`retention`] — deletion & retention policies (deletion must really delete).
//!
//! SQL never leaves this crate. All public APIs speak `bluey_core` types.

pub mod db;
pub mod documents;
pub mod repositories;
pub mod retention;
pub mod search;

mod error;
mod fts;
#[cfg(test)]
pub(crate) mod testutil;

pub use db::{Database, MIGRATIONS};
pub use documents::chunk::{chunk_text, estimate_tokens, ChunkDraft};
pub use documents::index::{
    add_document, reindex, DEFAULT_CHUNK_OVERLAP_TOKENS, DEFAULT_CHUNK_TOKENS,
};
pub use documents::parse::{detect_format, parse_document, ParsedDocument, MAX_DOCUMENT_BYTES};
pub use documents::retrieve::{cosine, retrieve};
pub use repositories::{
    AiCacheRepository, AiRequestRecord, AiRequestRepository, DocumentRepository, EmbeddedChunk,
    ModeRepository, ModelConfigRepository, ResponseRepository, SessionEventRepository,
    SessionNoteRepository, SessionRepository, SettingsRepository, ShortcutRepository,
    SnapshotRepository, SummaryRepository, TranscriptRepository, UserRepository,
};
pub use retention::{
    apply_retention, clear_ai_cache, clear_transcripts, delete_all_sessions, delete_screenshots,
    delete_session, prune_finished_sessions_without_history, reset_all, usage_stats,
    RetentionReport, UsageStats,
};
pub use search::search_sessions;
