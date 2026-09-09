//! Sessions, session events, session notes and session summaries.

use std::collections::BTreeMap;
use std::path::PathBuf;

use bluey_core::error::BlueyError;
use bluey_core::types::session::{
    Session, SessionEvent, SessionListItem, SessionNote, SessionSearchQuery, SessionStatus,
    SessionSummary, SessionSummaryInput,
};
use bluey_core::{new_id, now_iso};
use rusqlite::{params, OptionalExtension, Row};

use super::{from_enum_str, from_json_str, not_found, opt_from_json, opt_to_json, to_enum_str};
use crate::db::Database;
use crate::error::SqlExt;

/// Default page size for [`SessionRepository::list`].
const DEFAULT_LIST_LIMIT: u32 = 50;

fn status_str(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Paused => "paused",
        SessionStatus::Completed => "completed",
    }
}

fn status_from_str(s: &str) -> Result<SessionStatus, BlueyError> {
    match s {
        "active" => Ok(SessionStatus::Active),
        "paused" => Ok(SessionStatus::Paused),
        "completed" => Ok(SessionStatus::Completed),
        other => Err(BlueyError::storage(
            "query",
            format!("invalid session status '{other}'"),
        )),
    }
}

/// Raw sessions row (strings only, converted outside the rusqlite closure).
struct SessionRow {
    id: String,
    mode_id: String,
    title: Option<String>,
    status: String,
    started_at: String,
    ended_at: Option<String>,
    metadata: Option<String>,
}

impl SessionRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            mode_id: row.get(1)?,
            title: row.get(2)?,
            status: row.get(3)?,
            started_at: row.get(4)?,
            ended_at: row.get(5)?,
            metadata: row.get(6)?,
        })
    }

    fn into_session(self) -> Result<Session, BlueyError> {
        Ok(Session {
            id: self.id,
            mode_id: self.mode_id,
            started_at: self.started_at,
            ended_at: self.ended_at,
            status: status_from_str(&self.status)?,
            title: self.title,
            metadata: opt_from_json(self.metadata)?,
        })
    }
}

const SESSION_COLS: &str = "id, mode_id, title, status, started_at, ended_at, metadata";

/// Raw list-item row: session columns + mode name + counts.
struct ItemRow {
    session: SessionRow,
    mode_name: String,
    event_count: u32,
    response_count: u32,
    transcript_segment_count: u32,
    has_summary: bool,
}

impl ItemRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            session: SessionRow::read(row)?,
            mode_name: row.get(7)?,
            event_count: row.get::<_, i64>(8)?.max(0) as u32,
            response_count: row.get::<_, i64>(9)?.max(0) as u32,
            transcript_segment_count: row.get::<_, i64>(10)?.max(0) as u32,
            has_summary: row.get(11)?,
        })
    }

    fn into_item(self) -> Result<SessionListItem, BlueyError> {
        Ok(SessionListItem {
            session: self.session.into_session()?,
            mode_name: self.mode_name,
            event_count: self.event_count,
            response_count: self.response_count,
            transcript_segment_count: self.transcript_segment_count,
            has_summary: self.has_summary,
            snippet: None,
        })
    }
}

const ITEM_SELECT: &str =
    "SELECT s.id, s.mode_id, s.title, s.status, s.started_at, s.ended_at, s.metadata,
        coalesce(m.name, s.mode_id),
        (SELECT count(*) FROM session_events e WHERE e.session_id = s.id),
        (SELECT count(*) FROM ai_responses r WHERE r.session_id = s.id),
        (SELECT count(*) FROM transcript_segments t WHERE t.session_id = s.id),
        EXISTS(SELECT 1 FROM session_summaries su WHERE su.session_id = s.id)
   FROM sessions s LEFT JOIN modes m ON m.id = s.mode_id";

/// CRUD + queries for the `sessions` table.
pub struct SessionRepository;

impl SessionRepository {
    /// Create a new active session for `mode_id` (the mode must exist).
    pub fn create(
        db: &Database,
        mode_id: &str,
        title: Option<String>,
    ) -> Result<Session, BlueyError> {
        let session = Session {
            id: new_id("ses"),
            mode_id: mode_id.to_string(),
            started_at: now_iso(),
            ended_at: None,
            status: SessionStatus::Active,
            title,
            metadata: None,
        };
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, mode_id, title, status, started_at, ended_at, metadata, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?5)",
                params![
                    session.id,
                    session.mode_id,
                    session.title,
                    status_str(session.status),
                    session.started_at
                ],
            )
            .sql()?;
            Ok(())
        })?;
        Ok(session)
    }

    /// Insert an already-completed session (imported recordings): a single
    /// statement, so it can never be observed as `active` and never disturbs
    /// the live session.
    pub fn create_completed(
        db: &Database,
        mode_id: &str,
        title: Option<String>,
    ) -> Result<Session, BlueyError> {
        let now = now_iso();
        let session = Session {
            id: new_id("ses"),
            mode_id: mode_id.to_string(),
            started_at: now.clone(),
            ended_at: Some(now),
            status: SessionStatus::Completed,
            title,
            metadata: None,
        };
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, mode_id, title, status, started_at, ended_at, metadata, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, NULL, ?5)",
                params![
                    session.id,
                    session.mode_id,
                    session.title,
                    status_str(session.status),
                    session.started_at
                ],
            )
            .sql()?;
            Ok(())
        })?;
        Ok(session)
    }

    /// Fetch a session by id (`storage.not_found` when missing).
    pub fn get(db: &Database, id: &str) -> Result<Session, BlueyError> {
        let row = db.with_conn(|conn| {
            conn.query_row(
                &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
                [id],
                SessionRow::read,
            )
            .optional()
            .sql()
        })?;
        row.ok_or_else(|| not_found("session", id))?.into_session()
    }

    /// The single active or paused session, if any (newest wins if state was corrupted).
    pub fn get_active(db: &Database) -> Result<Option<Session>, BlueyError> {
        let row = db.with_conn(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {SESSION_COLS} FROM sessions
                      WHERE status IN ('active','paused')
                      ORDER BY started_at DESC LIMIT 1"
                ),
                [],
                SessionRow::read,
            )
            .optional()
            .sql()
        })?;
        row.map(SessionRow::into_session).transpose()
    }

    /// Update the lifecycle status (and optionally `ended_at`), returning the updated session.
    pub fn set_status(
        db: &Database,
        id: &str,
        status: SessionStatus,
        ended_at: Option<String>,
    ) -> Result<Session, BlueyError> {
        let changed = db.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET status = ?2, ended_at = coalesce(?3, ended_at) WHERE id = ?1",
                params![id, status_str(status), ended_at],
            )
            .sql()
        })?;
        if changed == 0 {
            return Err(not_found("session", id));
        }
        Self::get(db, id)
    }

    /// Rename a session.
    pub fn rename(db: &Database, id: &str, title: &str) -> Result<Session, BlueyError> {
        let changed = db.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET title = ?2 WHERE id = ?1",
                params![id, title],
            )
            .sql()
        })?;
        if changed == 0 {
            return Err(not_found("session", id));
        }
        Self::get(db, id)
    }

    /// Replace the free-form metadata object.
    pub fn update_metadata(
        db: &Database,
        id: &str,
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<Session, BlueyError> {
        let json = opt_to_json(&metadata)?;
        let changed = db.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET metadata = ?2 WHERE id = ?1",
                params![id, json],
            )
            .sql()
        })?;
        if changed == 0 {
            return Err(not_found("session", id));
        }
        Self::get(db, id)
    }

    /// List sessions (newest first) with mode name, counts and summary flag.
    /// Honours `mode_id` / `from` / `to` / `limit` / `offset`; the free-text field
    /// is ignored here (see [`crate::search::search_sessions`]).
    pub fn list(
        db: &Database,
        query: &SessionSearchQuery,
    ) -> Result<Vec<SessionListItem>, BlueyError> {
        let limit = i64::from(query.limit.unwrap_or(DEFAULT_LIST_LIMIT));
        let offset = i64::from(query.offset.unwrap_or(0));
        let rows = db.with_conn(|conn| {
            let sql = format!(
                "{ITEM_SELECT}
                  WHERE (?1 IS NULL OR s.mode_id = ?1)
                    AND (?2 IS NULL OR s.started_at >= ?2)
                    AND (?3 IS NULL OR s.started_at <= ?3)
                  ORDER BY s.started_at DESC, s.rowid DESC
                  LIMIT ?4 OFFSET ?5"
            );
            let mut stmt = conn.prepare(&sql).sql()?;
            let rows = stmt
                .query_map(
                    params![query.mode_id, query.from, query.to, limit, offset],
                    ItemRow::read,
                )
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        rows.into_iter().map(ItemRow::into_item).collect()
    }

    /// Fetch list items for specific session ids, preserving the input order.
    /// Unknown ids are skipped. Used by full-text session search.
    pub fn items_by_ids(db: &Database, ids: &[String]) -> Result<Vec<SessionListItem>, BlueyError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; ids.len()].join(", ");
        let rows = db.with_conn(|conn| {
            let sql = format!("{ITEM_SELECT} WHERE s.id IN ({placeholders})");
            let mut stmt = conn.prepare(&sql).sql()?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(ids.iter()), ItemRow::read)
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        let mut by_id: BTreeMap<String, SessionListItem> = BTreeMap::new();
        for row in rows {
            let item = row.into_item()?;
            by_id.insert(item.session.id.clone(), item);
        }
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }

    /// Delete a session (children cascade). Returns the screenshot image paths
    /// that were referenced by this session so the app can remove the files.
    pub fn delete(db: &Database, id: &str) -> Result<Vec<PathBuf>, BlueyError> {
        db.transaction(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT image_path FROM screen_snapshots
                      WHERE session_id = ?1 AND image_path IS NOT NULL",
                )
                .sql()?;
            let paths = stmt
                .query_map([id], |r| r.get::<_, String>(0))
                .sql()?
                .collect::<Result<Vec<_>, _>>()
                .sql()?;
            let changed = conn
                .execute("DELETE FROM sessions WHERE id = ?1", [id])
                .sql()?;
            if changed == 0 {
                return Err(not_found("session", id));
            }
            Ok(paths.into_iter().map(PathBuf::from).collect())
        })
    }

    /// Delete every session. Returns `(deleted_sessions, screenshot_image_paths)`.
    pub fn delete_all(db: &Database) -> Result<(u64, Vec<PathBuf>), BlueyError> {
        db.transaction(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT image_path FROM screen_snapshots
                      WHERE session_id IS NOT NULL AND image_path IS NOT NULL",
                )
                .sql()?;
            let paths = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .sql()?
                .collect::<Result<Vec<_>, _>>()
                .sql()?;
            let deleted = conn.execute("DELETE FROM sessions", []).sql()?;
            Ok((
                deleted as u64,
                paths.into_iter().map(PathBuf::from).collect(),
            ))
        })
    }
}

/// Timeline events (`session_events`).
pub struct SessionEventRepository;

impl SessionEventRepository {
    /// Insert an event exactly as given (callers construct it with `new_id`/`now_iso`).
    pub fn add(db: &Database, event: &SessionEvent) -> Result<SessionEvent, BlueyError> {
        let refs = opt_to_json(&event.refs)?;
        let event_type = to_enum_str(&event.event_type)?;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO session_events (id, session_id, type, title, detail, refs, confidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    event.id,
                    event.session_id,
                    event_type,
                    event.title,
                    event.detail,
                    refs,
                    event.confidence,
                    event.created_at
                ],
            )
            .sql()?;
            Ok(())
        })?;
        Ok(event.clone())
    }

    /// All events of a session in chronological order.
    pub fn list(db: &Database, session_id: &str) -> Result<Vec<SessionEvent>, BlueyError> {
        let raw = db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, type, title, detail, refs, confidence, created_at
                       FROM session_events WHERE session_id = ?1 ORDER BY created_at, id",
                )
                .sql()?;
            let rows = stmt
                .query_map([session_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<f32>>(6)?,
                        r.get::<_, String>(7)?,
                    ))
                })
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        raw.into_iter()
            .map(
                |(id, session_id, event_type, title, detail, refs, confidence, created_at)| {
                    Ok(SessionEvent {
                        id,
                        session_id,
                        event_type: from_enum_str(&event_type)?,
                        title,
                        detail,
                        refs: opt_from_json(refs)?,
                        confidence,
                        created_at,
                    })
                },
            )
            .collect()
    }

    /// Number of events recorded for a session.
    pub fn count(db: &Database, session_id: &str) -> Result<u64, BlueyError> {
        db.with_conn(|conn| {
            conn.query_row(
                "SELECT count(*) FROM session_events WHERE session_id = ?1",
                [session_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as u64)
            .sql()
        })
    }
}

/// User notes (`session_notes`).
pub struct SessionNoteRepository;

impl SessionNoteRepository {
    /// Add a note to a session.
    pub fn add(db: &Database, session_id: &str, content: &str) -> Result<SessionNote, BlueyError> {
        let now = now_iso();
        let note = SessionNote {
            id: new_id("note"),
            session_id: session_id.to_string(),
            content: content.to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO session_notes (id, session_id, content, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    note.id,
                    note.session_id,
                    note.content,
                    note.created_at,
                    note.updated_at
                ],
            )
            .sql()?;
            Ok(())
        })?;
        Ok(note)
    }

    /// All notes of a session in chronological order.
    pub fn list(db: &Database, session_id: &str) -> Result<Vec<SessionNote>, BlueyError> {
        db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, content, created_at, updated_at
                       FROM session_notes WHERE session_id = ?1 ORDER BY created_at, id",
                )
                .sql()?;
            let rows = stmt
                .query_map([session_id], |r| {
                    Ok(SessionNote {
                        id: r.get(0)?,
                        session_id: r.get(1)?,
                        content: r.get(2)?,
                        created_at: r.get(3)?,
                        updated_at: r.get(4)?,
                    })
                })
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })
    }

    /// Delete a note by id (`storage.not_found` when missing).
    pub fn delete(db: &Database, note_id: &str) -> Result<(), BlueyError> {
        let changed = db.with_conn(|conn| {
            conn.execute("DELETE FROM session_notes WHERE id = ?1", [note_id])
                .sql()
        })?;
        if changed == 0 {
            return Err(not_found("session note", note_id));
        }
        Ok(())
    }
}

/// One summary per session (`session_summaries`).
pub struct SummaryRepository;

impl SummaryRepository {
    /// Upsert the summary for a session (there is at most one). A re-generated
    /// summary keeps the original id but refreshes `created_at`.
    pub fn save(db: &Database, input: &SessionSummaryInput) -> Result<SessionSummary, BlueyError> {
        let existing_id: Option<String> = db.with_conn(|conn| {
            conn.query_row(
                "SELECT id FROM session_summaries WHERE session_id = ?1",
                [&input.session_id],
                |r| r.get(0),
            )
            .optional()
            .sql()
        })?;
        let summary = SessionSummary {
            id: existing_id.unwrap_or_else(|| new_id("sum")),
            session_id: input.session_id.clone(),
            mode_id: input.mode_id.clone(),
            overview: input.overview.clone(),
            topics: input.topics.clone(),
            questions: input.questions.clone(),
            answers: input.answers.clone(),
            decisions: input.decisions.clone(),
            action_items: input.action_items.clone(),
            open_items: input.open_items.clone(),
            improvements: input.improvements.clone(),
            sections: input.sections.clone(),
            created_at: now_iso(),
        };
        let json = super::to_json_string(&summary)?;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO session_summaries (id, session_id, mode_id, summary_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(session_id) DO UPDATE SET
                   mode_id = excluded.mode_id,
                   summary_json = excluded.summary_json,
                   created_at = excluded.created_at",
                params![
                    summary.id,
                    summary.session_id,
                    summary.mode_id,
                    json,
                    summary.created_at
                ],
            )
            .sql()?;
            Ok(())
        })?;
        Ok(summary)
    }

    /// Fetch the stored summary for a session, if any.
    pub fn get(db: &Database, session_id: &str) -> Result<Option<SessionSummary>, BlueyError> {
        let json: Option<String> = db.with_conn(|conn| {
            conn.query_row(
                "SELECT summary_json FROM session_summaries WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .optional()
            .sql()
        })?;
        json.map(|s| from_json_str(&s)).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;
    use pretty_assertions::assert_eq;

    #[test]
    fn create_get_active_and_lifecycle() {
        let db = testutil::db();
        assert!(SessionRepository::get_active(&db).unwrap().is_none());
        let s = SessionRepository::create(&db, "general", Some("Standup".into())).unwrap();
        assert_eq!(s.status, SessionStatus::Active);
        assert_eq!(
            SessionRepository::get_active(&db).unwrap().unwrap().id,
            s.id
        );

        let paused =
            SessionRepository::set_status(&db, &s.id, SessionStatus::Paused, None).unwrap();
        assert_eq!(paused.status, SessionStatus::Paused);
        assert_eq!(
            SessionRepository::get_active(&db).unwrap().unwrap().id,
            s.id
        );

        let ended =
            SessionRepository::set_status(&db, &s.id, SessionStatus::Completed, Some(now_iso()))
                .unwrap();
        assert_eq!(ended.status, SessionStatus::Completed);
        assert!(ended.ended_at.is_some());
        assert!(SessionRepository::get_active(&db).unwrap().is_none());

        let renamed = SessionRepository::rename(&db, &s.id, "Renamed").unwrap();
        assert_eq!(renamed.title.as_deref(), Some("Renamed"));

        let mut meta = serde_json::Map::new();
        meta.insert("k".into(), serde_json::json!(1));
        let with_meta = SessionRepository::update_metadata(&db, &s.id, Some(meta.clone())).unwrap();
        assert_eq!(with_meta.metadata, Some(meta));

        assert!(SessionRepository::get(&db, "missing").is_err());
    }

    #[test]
    fn create_requires_existing_mode() {
        let db = testutil::db();
        let err = SessionRepository::create(&db, "no-such-mode", None).unwrap_err();
        assert_eq!(err.code, "storage.constraint");
    }

    #[test]
    fn list_filters_and_counts() {
        let db = testutil::db();
        testutil::seed_mode(&db, "interview", "Interview");
        let a = SessionRepository::create(&db, "general", Some("A".into())).unwrap();
        let b = SessionRepository::create(&db, "interview", Some("B".into())).unwrap();

        let ev = testutil::event(&a.id, "Question detected");
        SessionEventRepository::add(&db, &ev).unwrap();
        SessionNoteRepository::add(&db, &a.id, "note").unwrap();
        SummaryRepository::save(&db, &testutil::summary_input(&a.id)).unwrap();

        let all = SessionRepository::list(&db, &SessionSearchQuery::default()).unwrap();
        assert_eq!(all.len(), 2);
        // Newest first: b was created after a.
        assert_eq!(all[0].session.id, b.id);
        let item_a = all.iter().find(|i| i.session.id == a.id).unwrap();
        assert_eq!(item_a.event_count, 1);
        assert!(item_a.has_summary);
        assert_eq!(item_a.mode_name, "General");

        let only_interview = SessionRepository::list(
            &db,
            &SessionSearchQuery {
                mode_id: Some("interview".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(only_interview.len(), 1);
        assert_eq!(only_interview[0].session.id, b.id);

        let none = SessionRepository::list(
            &db,
            &SessionSearchQuery {
                to: Some("2000-01-01".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(none.is_empty());

        let ordered =
            SessionRepository::items_by_ids(&db, &[a.id.clone(), "nope".into(), b.id.clone()])
                .unwrap();
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].session.id, a.id);
        assert_eq!(ordered[1].session.id, b.id);
    }

    #[test]
    fn events_notes_summary_round_trip() {
        let db = testutil::db();
        let s = SessionRepository::create(&db, "general", None).unwrap();
        let mut ev = testutil::event(&s.id, "Coding problem");
        ev.refs = Some(std::collections::BTreeMap::from([(
            "responseId".to_string(),
            "res_1".to_string(),
        )]));
        ev.confidence = Some(0.9);
        SessionEventRepository::add(&db, &ev).unwrap();
        let events = SessionEventRepository::list(&db, &s.id).unwrap();
        assert_eq!(events, vec![ev]);
        assert_eq!(SessionEventRepository::count(&db, &s.id).unwrap(), 1);

        let note = SessionNoteRepository::add(&db, &s.id, "remember this").unwrap();
        assert_eq!(
            SessionNoteRepository::list(&db, &s.id).unwrap(),
            vec![note.clone()]
        );
        SessionNoteRepository::delete(&db, &note.id).unwrap();
        assert!(SessionNoteRepository::list(&db, &s.id).unwrap().is_empty());
        assert!(SessionNoteRepository::delete(&db, &note.id).is_err());

        assert!(SummaryRepository::get(&db, &s.id).unwrap().is_none());
        let first = SummaryRepository::save(&db, &testutil::summary_input(&s.id)).unwrap();
        let mut input = testutil::summary_input(&s.id);
        input.overview = "updated".into();
        let second = SummaryRepository::save(&db, &input).unwrap();
        assert_eq!(first.id, second.id, "summary id is stable across upserts");
        let got = SummaryRepository::get(&db, &s.id).unwrap().unwrap();
        assert_eq!(got.overview, "updated");
        // Still exactly one row.
        let n: i64 = db
            .with_conn(|c| {
                c.query_row("SELECT count(*) FROM session_summaries", [], |r| r.get(0))
                    .sql()
            })
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn delete_cascades_and_returns_screenshot_paths() {
        let db = testutil::db();
        let s = SessionRepository::create(&db, "general", None).unwrap();
        SessionNoteRepository::add(&db, &s.id, "n").unwrap();
        SessionEventRepository::add(&db, &testutil::event(&s.id, "e")).unwrap();
        testutil::insert_screen_snapshot(&db, Some(&s.id), Some("/tmp/shot1.jpg"));
        testutil::insert_screen_snapshot(&db, Some(&s.id), None);

        let paths = SessionRepository::delete(&db, &s.id).unwrap();
        assert_eq!(paths, vec![PathBuf::from("/tmp/shot1.jpg")]);
        for table in [
            "sessions",
            "session_notes",
            "session_events",
            "screen_snapshots",
        ] {
            let n: i64 = db
                .with_conn(|c| {
                    c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                        .sql()
                })
                .unwrap();
            assert_eq!(n, 0, "{table} should be empty");
        }
        assert!(SessionRepository::delete(&db, &s.id).is_err());

        let s2 = SessionRepository::create(&db, "general", None).unwrap();
        testutil::insert_screen_snapshot(&db, Some(&s2.id), Some("/tmp/shot2.jpg"));
        let (count, paths) = SessionRepository::delete_all(&db).unwrap();
        assert_eq!(count, 1);
        assert_eq!(paths, vec![PathBuf::from("/tmp/shot2.jpg")]);
    }
}
