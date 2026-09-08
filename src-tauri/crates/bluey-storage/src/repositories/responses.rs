//! AI responses (+ feedback + `responses_fts`) and the `ai_requests` metrics table.

use bluey_core::error::BlueyError;
use bluey_core::now_iso;
use bluey_core::types::response::{
    BlueyResponse, FeedbackCategory, FeedbackRating, ResponseFeedback,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{from_json_str, not_found, opt_to_json, to_enum_str, to_json_string};
use crate::db::Database;
use crate::error::SqlExt;

/// Persistence for `ai_responses` / `response_feedback`.
///
/// The full [`BlueyResponse`] JSON is the source of truth (`response_json`);
/// the extracted columns exist for filtering and full-text search.
pub struct ResponseRepository;

impl ResponseRepository {
    /// Insert or replace a response by id. `responses_fts` follows via triggers.
    pub fn save(db: &Database, response: &BlueyResponse) -> Result<BlueyResponse, BlueyError> {
        let response_type = to_enum_str(&response.response_type)?;
        let json = to_json_string(response)?;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO ai_responses (id, request_id, session_id, mode_id, response_type,
                    title, content, prompt, response_json, prepared, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                   request_id = excluded.request_id,
                   session_id = excluded.session_id,
                   mode_id = excluded.mode_id,
                   response_type = excluded.response_type,
                   title = excluded.title,
                   content = excluded.content,
                   prompt = excluded.prompt,
                   response_json = excluded.response_json,
                   prepared = excluded.prepared",
                params![
                    response.id,
                    response.request_id,
                    response.session_id,
                    response.mode_id,
                    response_type,
                    response.title,
                    response.content,
                    response.prompt,
                    json,
                    response.prepared.unwrap_or(false),
                    response.created_at
                ],
            )
            .sql()?;
            Ok(())
        })?;
        Ok(response.clone())
    }

    /// Responses of a session in chronological order; when `limit` is set only
    /// the most recent `limit` responses are returned.
    pub fn list(
        db: &Database,
        session_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<BlueyResponse>, BlueyError> {
        let rows = db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT response_json FROM ai_responses
                      WHERE session_id = ?1
                      ORDER BY created_at DESC, id DESC
                      LIMIT ?2",
                )
                .sql()?;
            let rows = stmt
                .query_map(
                    params![session_id, limit.map(i64::from).unwrap_or(-1)],
                    |r| r.get::<_, String>(0),
                )
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        let mut responses = rows
            .iter()
            .map(|json| from_json_str::<BlueyResponse>(json))
            .collect::<Result<Vec<_>, _>>()?;
        responses.reverse();
        Ok(responses)
    }

    /// Fetch one response (`storage.not_found` when missing).
    pub fn get(db: &Database, id: &str) -> Result<BlueyResponse, BlueyError> {
        let json: Option<String> = db.with_conn(|conn| {
            conn.query_row(
                "SELECT response_json FROM ai_responses WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .sql()
        })?;
        from_json_str(&json.ok_or_else(|| not_found("response", id))?)
    }

    /// Delete a response (feedback cascades, FTS cleaned by trigger).
    pub fn delete(db: &Database, id: &str) -> Result<(), BlueyError> {
        let changed = db.with_conn(|conn| {
            conn.execute("DELETE FROM ai_responses WHERE id = ?1", [id])
                .sql()
        })?;
        if changed == 0 {
            return Err(not_found("response", id));
        }
        Ok(())
    }

    /// Record (or replace) user feedback for a response and return the updated
    /// response with `feedback` embedded.
    pub fn set_feedback(
        db: &Database,
        response_id: &str,
        rating: FeedbackRating,
        categories: Option<Vec<FeedbackCategory>>,
        comment: Option<String>,
    ) -> Result<BlueyResponse, BlueyError> {
        let mut response = Self::get(db, response_id)?;
        let feedback = ResponseFeedback {
            response_id: response_id.to_string(),
            rating,
            categories,
            comment,
            created_at: now_iso(),
        };
        let rating_str = to_enum_str(&rating)?;
        let categories_json = opt_to_json(&feedback.categories)?;
        response.feedback = Some(feedback.clone());
        let json = to_json_string(&response)?;
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO response_feedback (response_id, rating, categories, comment, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(response_id) DO UPDATE SET
                   rating = excluded.rating,
                   categories = excluded.categories,
                   comment = excluded.comment,
                   created_at = excluded.created_at",
                params![feedback.response_id, rating_str, categories_json, feedback.comment, feedback.created_at],
            )
            .sql()?;
            conn.execute(
                "UPDATE ai_responses SET response_json = ?2 WHERE id = ?1",
                params![response_id, json],
            )
            .sql()?;
            Ok(())
        })?;
        Ok(response)
    }

    /// Number of responses in a session.
    pub fn count(db: &Database, session_id: &str) -> Result<u64, BlueyError> {
        db.with_conn(|conn| {
            conn.query_row(
                "SELECT count(*) FROM ai_responses WHERE session_id = ?1",
                [session_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as u64)
            .sql()
        })
    }
}

/// One row of the `ai_requests` latency/token metrics table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiRequestRecord {
    /// The `requestId` of the originating `AiRequest`.
    pub id: String,
    pub session_id: Option<String>,
    /// Task tag, e.g. `answer`, `vision` (serde string of `AiTask`).
    pub task: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    /// Latency budget tag, e.g. `fast` (serde string of `LatencyBudget`).
    pub latency_budget: Option<String>,
    pub context_tokens: Option<u32>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub ttft_ms: Option<u64>,
    pub total_ms: Option<u64>,
    pub finish_reason: Option<String>,
    pub error_code: Option<String>,
    /// Defaults to now when empty.
    pub created_at: String,
}

/// Metrics writer for `ai_requests`.
pub struct AiRequestRepository;

impl AiRequestRepository {
    /// Insert or replace one request record (keyed by request id).
    pub fn record(db: &Database, record: &AiRequestRecord) -> Result<(), BlueyError> {
        if record.id.trim().is_empty() {
            return Err(BlueyError::invalid_params(
                "ai request record id is required",
            ));
        }
        let created_at = if record.created_at.is_empty() {
            now_iso()
        } else {
            record.created_at.clone()
        };
        db.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO ai_requests (id, session_id, task, provider_id, model,
                    latency_budget, context_tokens, input_tokens, output_tokens, ttft_ms,
                    total_ms, finish_reason, error_code, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    record.id,
                    record.session_id,
                    record.task,
                    record.provider_id,
                    record.model,
                    record.latency_budget,
                    record.context_tokens,
                    record.input_tokens,
                    record.output_tokens,
                    record.ttft_ms.map(|v| v as i64),
                    record.total_ms.map(|v| v as i64),
                    record.finish_reason,
                    record.error_code,
                    created_at
                ],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Most recent request records (newest first), for the dev overlay.
    pub fn recent(db: &Database, limit: u32) -> Result<Vec<AiRequestRecord>, BlueyError> {
        db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, task, provider_id, model, latency_budget,
                            context_tokens, input_tokens, output_tokens, ttft_ms, total_ms,
                            finish_reason, error_code, created_at
                       FROM ai_requests ORDER BY created_at DESC, id DESC LIMIT ?1",
                )
                .sql()?;
            let rows = stmt
                .query_map([i64::from(limit)], |r| {
                    Ok(AiRequestRecord {
                        id: r.get(0)?,
                        session_id: r.get(1)?,
                        task: r.get(2)?,
                        provider_id: r.get(3)?,
                        model: r.get(4)?,
                        latency_budget: r.get(5)?,
                        context_tokens: r.get(6)?,
                        input_tokens: r.get(7)?,
                        output_tokens: r.get(8)?,
                        ttft_ms: r.get::<_, Option<i64>>(9)?.map(|v| v.max(0) as u64),
                        total_ms: r.get::<_, Option<i64>>(10)?.map(|v| v.max(0) as u64),
                        finish_reason: r.get(11)?,
                        error_code: r.get(12)?,
                        created_at: r.get(13)?,
                    })
                })
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
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
            c.query_row("SELECT count(*) FROM responses_fts", [], |r| r.get(0))
                .sql()
        })
        .unwrap()
    }

    #[test]
    fn save_get_list_round_trip_and_fts_sync() {
        let db = testutil::db();
        let s = SessionRepository::create(&db, "general", None).unwrap();
        let mut r1 = testutil::response(Some(&s.id), "Binary search runs in O(log n)");
        r1.title = Some("Complexity".into());
        // Distinct timestamps: ordering ties on created_at would fall back to random ids.
        r1.created_at = "2026-09-08T09:00:00.000Z".into();
        let mut r2 = testutil::response(Some(&s.id), "Second answer");
        r2.created_at = "2026-09-08T09:00:01.000Z".into();
        ResponseRepository::save(&db, &r1).unwrap();
        ResponseRepository::save(&db, &r2).unwrap();
        assert_eq!(fts_count(&db), 2);

        assert_eq!(ResponseRepository::get(&db, &r1.id).unwrap(), r1);
        assert_eq!(ResponseRepository::count(&db, &s.id).unwrap(), 2);
        let listed = ResponseRepository::list(&db, &s.id, None).unwrap();
        assert_eq!(listed, vec![r1.clone(), r2.clone()]);
        let last = ResponseRepository::list(&db, &s.id, Some(1)).unwrap();
        assert_eq!(last, vec![r2.clone()]);

        // Upsert replaces content and keeps a single FTS row.
        r1.content = "Binary search is O(log n) on sorted arrays".into();
        ResponseRepository::save(&db, &r1).unwrap();
        assert_eq!(fts_count(&db), 2);
        let hits: i64 = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT count(*) FROM responses_fts WHERE responses_fts MATCH 'sorted'",
                    [],
                    |r| r.get(0),
                )
                .sql()
            })
            .unwrap();
        assert_eq!(hits, 1);

        ResponseRepository::delete(&db, &r2.id).unwrap();
        assert!(ResponseRepository::get(&db, &r2.id).is_err());
        assert_eq!(fts_count(&db), 1);

        // Session delete cascades responses + FTS.
        SessionRepository::delete(&db, &s.id).unwrap();
        assert_eq!(fts_count(&db), 0);
        assert_eq!(testutil::count(&db, "ai_responses"), 0);
    }

    #[test]
    fn feedback_upserts_and_embeds_in_response() {
        let db = testutil::db();
        let s = SessionRepository::create(&db, "general", None).unwrap();
        let r = testutil::response(Some(&s.id), "answer");
        ResponseRepository::save(&db, &r).unwrap();

        let with_feedback = ResponseRepository::set_feedback(
            &db,
            &r.id,
            FeedbackRating::Down,
            Some(vec![FeedbackCategory::TooLong]),
            Some("way too long".into()),
        )
        .unwrap();
        let fb = with_feedback.feedback.clone().unwrap();
        assert_eq!(fb.rating, FeedbackRating::Down);
        assert_eq!(fb.categories, Some(vec![FeedbackCategory::TooLong]));

        // Reading back includes the feedback; a second rating replaces the first.
        assert_eq!(
            ResponseRepository::get(&db, &r.id)
                .unwrap()
                .feedback
                .unwrap()
                .rating,
            FeedbackRating::Down
        );
        ResponseRepository::set_feedback(&db, &r.id, FeedbackRating::Up, None, None).unwrap();
        assert_eq!(
            ResponseRepository::get(&db, &r.id)
                .unwrap()
                .feedback
                .unwrap()
                .rating,
            FeedbackRating::Up
        );
        assert_eq!(testutil::count(&db, "response_feedback"), 1);

        assert!(
            ResponseRepository::set_feedback(&db, "missing", FeedbackRating::Up, None, None)
                .is_err()
        );
    }

    #[test]
    fn ai_request_metrics_record_and_recent() {
        let db = testutil::db();
        let rec = AiRequestRecord {
            id: "req_1".into(),
            task: "answer".into(),
            provider_id: Some("azure".into()),
            model: Some("gpt-5".into()),
            latency_budget: Some("fast".into()),
            input_tokens: Some(120),
            output_tokens: Some(80),
            ttft_ms: Some(420),
            total_ms: Some(1650),
            finish_reason: Some("stop".into()),
            ..Default::default()
        };
        AiRequestRepository::record(&db, &rec).unwrap();
        // Replacing by id updates in place.
        let mut rec2 = rec.clone();
        rec2.total_ms = Some(1700);
        AiRequestRepository::record(&db, &rec2).unwrap();
        let recent = AiRequestRepository::recent(&db, 10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].total_ms, Some(1700));
        assert!(!recent[0].created_at.is_empty());
        assert!(AiRequestRepository::record(&db, &AiRequestRecord::default()).is_err());
    }
}
