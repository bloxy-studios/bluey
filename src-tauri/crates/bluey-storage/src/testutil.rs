//! Shared fixtures for unit tests (compiled only with `cfg(test)`).

use bluey_core::types::documents::{AddDocumentInput, DocumentFormat, DocumentKind, DocumentScope};
use bluey_core::types::mode::{BlueyMode, PreferredLatency, ResponseSchemaId};
use bluey_core::types::response::{BlueyResponse, ResponseType};
use bluey_core::types::session::{SessionEvent, SessionEventType, SessionSummaryInput};
use bluey_core::types::transcript::{AudioSource, TranscriptSegment};
use bluey_core::{new_id, now_iso};
use rusqlite::params;

use crate::db::Database;
use crate::error::SqlExt;
use crate::repositories::ModeRepository;

/// Fresh in-memory database with the `general` built-in mode seeded.
pub(crate) fn db() -> Database {
    let db = Database::in_memory().expect("in-memory db");
    ModeRepository::seed_built_in(&db, &[mode("general", "General")]).expect("seed general mode");
    db
}

/// Minimal built-in style mode fixture.
pub(crate) fn mode(id: &str, name: &str) -> BlueyMode {
    BlueyMode {
        id: id.to_string(),
        name: name.to_string(),
        description: format!("{name} mode"),
        icon: "sparkles".into(),
        system_instructions: format!("You are in {name} mode."),
        response_schema: ResponseSchemaId::Answer,
        preferred_latency: PreferredLatency::Fast,
        context_requirements: vec![],
        built_in: true,
        group: None,
        response_style: None,
        preferred_model_role: None,
        attached_document_ids: vec![],
        created_at: now_iso(),
        updated_at: now_iso(),
    }
}

/// Seed an extra built-in mode.
pub(crate) fn seed_mode(db: &Database, id: &str, name: &str) {
    ModeRepository::seed_built_in(db, &[mode(id, name)]).expect("seed mode");
}

/// Session event fixture.
pub(crate) fn event(session_id: &str, title: &str) -> SessionEvent {
    SessionEvent {
        id: new_id("evt"),
        session_id: session_id.to_string(),
        event_type: SessionEventType::QuestionDetected,
        title: title.to_string(),
        detail: None,
        refs: None,
        confidence: None,
        created_at: now_iso(),
    }
}

/// Summary input fixture.
pub(crate) fn summary_input(session_id: &str) -> SessionSummaryInput {
    SessionSummaryInput {
        session_id: session_id.to_string(),
        mode_id: "general".into(),
        overview: "We talked about things".into(),
        topics: vec!["things".into()],
        questions: vec![],
        answers: vec![],
        decisions: vec![],
        action_items: vec![],
        open_items: vec![],
        improvements: vec![],
        sections: None,
    }
}

/// Transcript segment fixture (`created_at` = now).
pub(crate) fn segment(
    session_id: Option<&str>,
    text: &str,
    start_ms: u64,
    finalized: bool,
) -> TranscriptSegment {
    TranscriptSegment {
        id: new_id("seg"),
        session_id: session_id.map(str::to_string),
        speaker: Some("You".into()),
        speaker_confidence: None,
        source: AudioSource::Microphone,
        text: text.to_string(),
        start_time: start_ms,
        end_time: start_ms + 1000,
        confidence: Some(0.95),
        finalized,
        language: Some("en".into()),
        created_at: now_iso(),
    }
}

/// AI response fixture.
pub(crate) fn response(session_id: Option<&str>, content: &str) -> BlueyResponse {
    BlueyResponse {
        id: new_id("res"),
        request_id: new_id("req"),
        session_id: session_id.map(str::to_string),
        mode_id: "general".into(),
        response_type: ResponseType::Answer,
        title: None,
        content: content.to_string(),
        code: None,
        sections: None,
        citations: None,
        confidence: None,
        prompt: None,
        diagram: None,
        metrics: None,
        feedback: None,
        prepared: None,
        truncated: None,
        created_at: now_iso(),
    }
}

/// Document input fixture with inline content.
pub(crate) fn doc_input(
    kind: DocumentKind,
    scope: DocumentScope,
    scope_id: Option<&str>,
    content: &str,
) -> AddDocumentInput {
    AddDocumentInput {
        title: Some("Fixture".into()),
        kind,
        scope,
        scope_id: scope_id.map(str::to_string),
        path: None,
        content: Some(content.to_string()),
        format: Some(DocumentFormat::Txt),
    }
}

/// Insert a raw screen snapshot row (bypasses the repository on purpose).
pub(crate) fn insert_screen_snapshot(
    db: &Database,
    session_id: Option<&str>,
    image_path: Option<&str>,
) {
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO screen_snapshots (id, session_id, display_id, width, height, mime_type, image_path, captured_at)
             VALUES (?1, ?2, NULL, 100, 100, 'image/jpeg', ?3, ?4)",
            params![new_id("snap"), session_id, image_path, now_iso()],
        )
        .sql()?;
        Ok(())
    })
    .expect("insert snapshot");
}

/// Count rows of a table.
pub(crate) fn count(db: &Database, table: &str) -> i64 {
    db.with_conn(|c| {
        c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .sql()
    })
    .expect("count")
}
