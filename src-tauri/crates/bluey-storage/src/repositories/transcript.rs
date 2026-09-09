//! Transcript segments. `transcript_fts` is kept in sync by triggers
//! (migration `0002_fts_sync`) and only indexes finalized segments.

use bluey_core::error::BlueyError;
use bluey_core::types::transcript::{AudioSource, TranscriptSegment};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Row};

use crate::db::Database;
use crate::error::SqlExt;

fn source_str(source: AudioSource) -> &'static str {
    match source {
        AudioSource::Microphone => "microphone",
        AudioSource::System => "system",
    }
}

fn source_from_str(s: &str) -> Result<AudioSource, BlueyError> {
    match s {
        "microphone" => Ok(AudioSource::Microphone),
        "system" => Ok(AudioSource::System),
        other => Err(BlueyError::storage(
            "query",
            format!("invalid audio source '{other}'"),
        )),
    }
}

struct SegmentRow {
    id: String,
    session_id: Option<String>,
    speaker: Option<String>,
    speaker_confidence: Option<f32>,
    source: String,
    text: String,
    start_time_ms: i64,
    end_time_ms: i64,
    confidence: Option<f32>,
    finalized: bool,
    language: Option<String>,
    created_at: String,
}

impl SegmentRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            session_id: row.get(1)?,
            speaker: row.get(2)?,
            speaker_confidence: row.get(3)?,
            source: row.get(4)?,
            text: row.get(5)?,
            start_time_ms: row.get(6)?,
            end_time_ms: row.get(7)?,
            confidence: row.get(8)?,
            finalized: row.get(9)?,
            language: row.get(10)?,
            created_at: row.get(11)?,
        })
    }

    fn into_segment(self) -> Result<TranscriptSegment, BlueyError> {
        Ok(TranscriptSegment {
            id: self.id,
            session_id: self.session_id,
            speaker: self.speaker,
            speaker_confidence: self.speaker_confidence,
            source: source_from_str(&self.source)?,
            text: self.text,
            start_time: self.start_time_ms.max(0) as u64,
            end_time: self.end_time_ms.max(0) as u64,
            confidence: self.confidence,
            finalized: self.finalized,
            language: self.language,
            created_at: self.created_at,
        })
    }
}

const SEGMENT_COLS: &str = "id, session_id, speaker, speaker_confidence, source, text,
    start_time_ms, end_time_ms, confidence, finalized, language, created_at";

/// Persistence for `transcript_segments` (+ `transcript_fts`).
pub struct TranscriptRepository;

impl TranscriptRepository {
    /// Insert a segment. Finalized segments are indexed into `transcript_fts`
    /// by trigger; partials are not.
    pub fn insert(db: &Database, segment: &TranscriptSegment) -> Result<(), BlueyError> {
        db.with_conn(|conn| Self::insert_with(conn, segment))
    }

    fn insert_with(
        conn: &rusqlite::Connection,
        segment: &TranscriptSegment,
    ) -> Result<(), BlueyError> {
        conn.execute(
            "INSERT INTO transcript_segments
               (id, session_id, speaker, speaker_confidence, source, text,
                start_time_ms, end_time_ms, confidence, finalized, language, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                segment.id,
                segment.session_id,
                segment.speaker,
                segment.speaker_confidence,
                source_str(segment.source),
                segment.text,
                segment.start_time as i64,
                segment.end_time as i64,
                segment.confidence,
                segment.finalized,
                segment.language,
                segment.created_at
            ],
        )
        .sql()?;
        Ok(())
    }

    /// Insert-or-replace by id: streaming transcription repeatedly updates the
    /// same partial segment and eventually finalizes it. The FTS index follows
    /// the `finalized` flag via triggers.
    pub fn upsert_partial(db: &Database, segment: &TranscriptSegment) -> Result<(), BlueyError> {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO transcript_segments
                   (id, session_id, speaker, speaker_confidence, source, text,
                    start_time_ms, end_time_ms, confidence, finalized, language, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET
                   session_id = excluded.session_id,
                   speaker = excluded.speaker,
                   speaker_confidence = excluded.speaker_confidence,
                   source = excluded.source,
                   text = excluded.text,
                   start_time_ms = excluded.start_time_ms,
                   end_time_ms = excluded.end_time_ms,
                   confidence = excluded.confidence,
                   finalized = excluded.finalized,
                   language = excluded.language",
                params![
                    segment.id,
                    segment.session_id,
                    segment.speaker,
                    segment.speaker_confidence,
                    source_str(segment.source),
                    segment.text,
                    segment.start_time as i64,
                    segment.end_time as i64,
                    segment.confidence,
                    segment.finalized,
                    segment.language,
                    segment.created_at
                ],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Segments of a session ordered by start time. `since_ms` filters
    /// `start_time >= since_ms`; when `limit` is set, the **most recent**
    /// `limit` segments are returned (still in chronological order).
    pub fn list(
        db: &Database,
        session_id: Option<&str>,
        since_ms: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<TranscriptSegment>, BlueyError> {
        let rows = db.with_conn(|conn| {
            let sql = format!(
                "SELECT {SEGMENT_COLS} FROM transcript_segments
                  WHERE (?1 IS NULL OR session_id = ?1)
                    AND (?2 IS NULL OR start_time_ms >= ?2)
                  ORDER BY start_time_ms DESC, id DESC
                  LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql).sql()?;
            let rows = stmt
                .query_map(
                    params![
                        session_id,
                        since_ms.map(|v| v as i64),
                        limit.map(i64::from).unwrap_or(-1)
                    ],
                    SegmentRow::read,
                )
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        let mut segments = rows
            .into_iter()
            .map(SegmentRow::into_segment)
            .collect::<Result<Vec<_>, _>>()?;
        segments.reverse(); // chronological order
        Ok(segments)
    }

    /// Segments created within the last `window_seconds`, judged by `created_at`.
    /// `now_ms_cursor` (Unix epoch milliseconds) overrides "now" — useful for
    /// tests and replay; `None` means the current time.
    pub fn recent(
        db: &Database,
        window_seconds: u32,
        now_ms_cursor: Option<i64>,
    ) -> Result<Vec<TranscriptSegment>, BlueyError> {
        let now: DateTime<Utc> = match now_ms_cursor {
            Some(ms) => DateTime::<Utc>::from_timestamp_millis(ms)
                .ok_or_else(|| BlueyError::invalid_params("now_ms_cursor out of range"))?,
            None => Utc::now(),
        };
        let cutoff = (now - chrono::Duration::seconds(i64::from(window_seconds)))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let now_str = now.to_rfc3339_opts(SecondsFormat::Millis, true);
        let rows = db.with_conn(|conn| {
            let sql = format!(
                "SELECT {SEGMENT_COLS} FROM transcript_segments
                  WHERE created_at >= ?1 AND created_at <= ?2
                  ORDER BY start_time_ms, created_at, id"
            );
            let mut stmt = conn.prepare(&sql).sql()?;
            let rows = stmt
                .query_map(params![cutoff, now_str], SegmentRow::read)
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        rows.into_iter().map(SegmentRow::into_segment).collect()
    }

    /// Number of segments (optionally per session).
    pub fn count(db: &Database, session_id: Option<&str>) -> Result<u64, BlueyError> {
        db.with_conn(|conn| {
            conn.query_row(
                "SELECT count(*) FROM transcript_segments WHERE (?1 IS NULL OR session_id = ?1)",
                params![session_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as u64)
            .sql()
        })
    }

    /// End of the last stored segment of a session in ms (0 when none) — the
    /// offset at which an imported recording is appended.
    pub fn last_end_ms(db: &Database, session_id: &str) -> Result<u64, BlueyError> {
        db.with_conn(|conn| {
            conn.query_row(
                "SELECT MAX(end_time_ms) FROM transcript_segments WHERE session_id = ?1",
                params![session_id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map(|v| v.unwrap_or(0).max(0) as u64)
            .sql()
        })
    }

    /// Delete segments (all of them, or one session's). Returns the number of
    /// deleted rows; the FTS index is cleaned by trigger.
    pub fn clear(db: &Database, session_id: Option<&str>) -> Result<u64, BlueyError> {
        db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM transcript_segments WHERE (?1 IS NULL OR session_id = ?1)",
                params![session_id],
            )
            .map(|n| n as u64)
            .sql()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::SessionRepository;
    use crate::testutil;
    use pretty_assertions::assert_eq;

    fn fts_count(db: &Database) -> i64 {
        db.with_conn(|c| {
            c.query_row("SELECT count(*) FROM transcript_fts", [], |r| r.get(0))
                .sql()
        })
        .unwrap()
    }

    #[test]
    fn insert_list_and_round_trip() {
        let db = testutil::db();
        let s = SessionRepository::create(&db, "general", None).unwrap();
        let a = testutil::segment(Some(&s.id), "first words", 0, true);
        let b = testutil::segment(Some(&s.id), "second words", 5000, true);
        let free = testutil::segment(None, "no session", 0, true);
        TranscriptRepository::insert(&db, &b).unwrap();
        TranscriptRepository::insert(&db, &a).unwrap();
        TranscriptRepository::insert(&db, &free).unwrap();

        let listed = TranscriptRepository::list(&db, Some(&s.id), None, None).unwrap();
        assert_eq!(listed, vec![a.clone(), b.clone()]);

        let since = TranscriptRepository::list(&db, Some(&s.id), Some(1000), None).unwrap();
        assert_eq!(since, vec![b.clone()]);

        let last_one = TranscriptRepository::list(&db, Some(&s.id), None, Some(1)).unwrap();
        assert_eq!(last_one, vec![b.clone()]);

        assert_eq!(TranscriptRepository::count(&db, Some(&s.id)).unwrap(), 2);
        assert_eq!(TranscriptRepository::count(&db, None).unwrap(), 3);
    }

    #[test]
    fn recent_uses_created_at_window() {
        let db = testutil::db();
        let mut old = testutil::segment(None, "old", 0, true);
        old.created_at = "2020-01-01T00:00:00.000Z".into();
        let fresh = testutil::segment(None, "fresh", 10, true);
        TranscriptRepository::insert(&db, &old).unwrap();
        TranscriptRepository::insert(&db, &fresh).unwrap();

        let recent = TranscriptRepository::recent(&db, 120, None).unwrap();
        assert_eq!(recent, vec![fresh.clone()]);

        // With an explicit cursor placed in 2020 the old segment is in the window.
        let cursor = chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:30Z")
            .unwrap()
            .timestamp_millis();
        let then = TranscriptRepository::recent(&db, 60, Some(cursor)).unwrap();
        assert_eq!(then, vec![old]);
    }

    #[test]
    fn fts_only_indexes_finalized_and_stays_in_sync() {
        let db = testutil::db();
        let s = SessionRepository::create(&db, "general", None).unwrap();
        let done = testutil::segment(Some(&s.id), "rust ownership question", 0, true);
        let mut partial = testutil::segment(Some(&s.id), "still talki", 1000, false);
        TranscriptRepository::insert(&db, &done).unwrap();
        TranscriptRepository::insert(&db, &partial).unwrap();
        assert_eq!(fts_count(&db), 1);

        // Finalizing the partial via upsert adds it to the index.
        partial.text = "still talking about lifetimes".into();
        partial.finalized = true;
        TranscriptRepository::upsert_partial(&db, &partial).unwrap();
        assert_eq!(fts_count(&db), 2);
        let hits: i64 = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT count(*) FROM transcript_fts WHERE transcript_fts MATCH 'lifetimes'",
                    [],
                    |r| r.get(0),
                )
                .sql()
            })
            .unwrap();
        assert_eq!(hits, 1);

        // Re-upserting as partial removes it again.
        partial.finalized = false;
        TranscriptRepository::upsert_partial(&db, &partial).unwrap();
        assert_eq!(fts_count(&db), 1);

        // clear() and cascading session deletes clean the index.
        TranscriptRepository::clear(&db, Some(&s.id)).unwrap();
        assert_eq!(fts_count(&db), 0);
        let other = testutil::segment(Some(&s.id), "cascade me", 0, true);
        TranscriptRepository::insert(&db, &other).unwrap();
        assert_eq!(fts_count(&db), 1);
        SessionRepository::delete(&db, &s.id).unwrap();
        assert_eq!(
            fts_count(&db),
            0,
            "cascade delete must clean transcript_fts"
        );
    }

    #[test]
    fn clear_all_really_deletes() {
        let db = testutil::db();
        TranscriptRepository::insert(&db, &testutil::segment(None, "a", 0, true)).unwrap();
        TranscriptRepository::insert(&db, &testutil::segment(None, "b", 1, true)).unwrap();
        assert_eq!(TranscriptRepository::clear(&db, None).unwrap(), 2);
        assert_eq!(testutil::count(&db, "transcript_segments"), 0);
        assert_eq!(fts_count(&db), 0);
    }
}
