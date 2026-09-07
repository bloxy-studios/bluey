//! Full-text session search across titles, mode names, `transcript_fts` and
//! `responses_fts`, merged per session with the best snippet.

use std::collections::HashMap;

use bluey_core::error::BlueyError;
use bluey_core::types::session::{SessionListItem, SessionSearchQuery};

use crate::db::Database;
use crate::error::SqlExt;
use crate::fts::{fts_match_query, like_contains_pattern};
use crate::repositories::SessionRepository;

/// Rank assigned to title / mode-name matches — always ahead of FTS matches
/// (FTS bm25 ranks are negative but bounded well above this).
const TITLE_RANK: f64 = -1.0e9;

/// Search sessions.
///
/// Without `query.text` this delegates to [`SessionRepository::list`]. With
/// text, sessions are matched on title + mode name (`LIKE`), transcript
/// segments (`transcript_fts`) and responses (`responses_fts` over
/// title/content/prompt); results are merged per session, ordered by best
/// match (title matches first, then bm25 rank, then recency), carry an FTS
/// `snippet()` where available, honour the `mode_id`/`from`/`to` filters and
/// are paged with `limit`/`offset`.
pub fn search_sessions(
    db: &Database,
    q: &SessionSearchQuery,
) -> Result<Vec<SessionListItem>, BlueyError> {
    let Some(text) = q.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
        return SessionRepository::list(db, q);
    };

    // session id -> (best rank, snippet from the best-ranked source)
    let mut candidates: HashMap<String, (f64, Option<String>)> = HashMap::new();
    let mut merge = |id: String, rank: f64, snippet: Option<String>| {
        candidates
            .entry(id)
            .and_modify(|entry| {
                if rank < entry.0 {
                    *entry = (rank, snippet.clone());
                }
            })
            .or_insert((rank, snippet));
    };

    // 1. Session titles and mode names (LIKE, case-insensitive for ASCII).
    let pattern = like_contains_pattern(text);
    let title_hits: Vec<String> = db.with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT s.id FROM sessions s LEFT JOIN modes m ON m.id = s.mode_id
                  WHERE s.title LIKE ?1 ESCAPE '\\' OR m.name LIKE ?1 ESCAPE '\\'",
            )
            .sql()?;
        let rows = stmt.query_map([&pattern], |r| r.get(0)).sql()?;
        rows.collect::<Result<Vec<String>, _>>().sql()
    })?;
    for id in title_hits {
        merge(id, TITLE_RANK, None);
    }

    // 2 + 3. FTS over transcripts and responses.
    if let Some(match_expr) = fts_match_query(text) {
        let transcript_hits: Vec<(String, f64, String)> = db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, bm25(transcript_fts) AS rank,
                            snippet(transcript_fts, 0, '', '', '…', 12)
                       FROM transcript_fts
                      WHERE transcript_fts MATCH ?1 AND session_id != ''
                      ORDER BY rank LIMIT 200",
                )
                .sql()?;
            let rows = stmt
                .query_map([&match_expr], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        for (id, rank, snippet) in transcript_hits {
            merge(id, rank, Some(snippet));
        }

        let response_hits: Vec<(String, f64, String)> = db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, bm25(responses_fts, 2.0, 1.0, 1.0) AS rank,
                            snippet(responses_fts, 1, '', '', '…', 12)
                       FROM responses_fts
                      WHERE responses_fts MATCH ?1 AND session_id != ''
                      ORDER BY rank LIMIT 200",
                )
                .sql()?;
            let rows = stmt
                .query_map([&match_expr], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        for (id, rank, snippet) in response_hits {
            merge(id, rank, Some(snippet));
        }
    }

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // Fetch items, apply the structured filters, attach snippets.
    let ids: Vec<String> = candidates.keys().cloned().collect();
    let mut items = SessionRepository::items_by_ids(db, &ids)?;
    items.retain(|item| {
        let s = &item.session;
        q.mode_id.as_deref().is_none_or(|m| s.mode_id == m)
            && q.from
                .as_deref()
                .is_none_or(|from| s.started_at.as_str() >= from)
            && q.to.as_deref().is_none_or(|to| s.started_at.as_str() <= to)
    });
    for item in &mut items {
        if let Some((_, snippet)) = candidates.get(&item.session.id) {
            item.snippet = snippet.clone().filter(|s| !s.is_empty());
        }
    }

    // Order: best rank first, then most recent.
    items.sort_by(|a, b| {
        let ra = candidates.get(&a.session.id).map(|c| c.0).unwrap_or(0.0);
        let rb = candidates.get(&b.session.id).map(|c| c.0).unwrap_or(0.0);
        ra.partial_cmp(&rb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.session.started_at.cmp(&a.session.started_at))
            .then_with(|| a.session.id.cmp(&b.session.id))
    });

    let offset = q.offset.unwrap_or(0) as usize;
    let limit = q.limit.map(|l| l as usize).unwrap_or(50);
    Ok(items.into_iter().skip(offset).take(limit).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::{ResponseRepository, SessionRepository, TranscriptRepository};
    use crate::testutil;
    use pretty_assertions::assert_eq;

    #[test]
    fn empty_text_delegates_to_list() {
        let db = testutil::db();
        SessionRepository::create(&db, "general", Some("Alpha".into())).unwrap();
        let all = search_sessions(&db, &SessionSearchQuery::default()).unwrap();
        assert_eq!(all.len(), 1);
        let blank = search_sessions(
            &db,
            &SessionSearchQuery {
                text: Some("   ".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(blank.len(), 1);
    }

    #[test]
    fn matches_titles_transcripts_and_responses_with_snippets() {
        let db = testutil::db();
        let by_title =
            SessionRepository::create(&db, "general", Some("Kubernetes planning".into())).unwrap();
        let by_transcript =
            SessionRepository::create(&db, "general", Some("Untitled".into())).unwrap();
        let by_response = SessionRepository::create(&db, "general", Some("Other".into())).unwrap();
        let unrelated = SessionRepository::create(&db, "general", Some("Nothing".into())).unwrap();

        TranscriptRepository::insert(
            &db,
            &testutil::segment(
                Some(&by_transcript.id),
                "we should move the cluster to kubernetes next month",
                0,
                true,
            ),
        )
        .unwrap();
        ResponseRepository::save(
            &db,
            &testutil::response(
                Some(&by_response.id),
                "Kubernetes uses declarative manifests for deployment.",
            ),
        )
        .unwrap();
        TranscriptRepository::insert(
            &db,
            &testutil::segment(Some(&unrelated.id), "totally different topic", 0, true),
        )
        .unwrap();

        let hits = search_sessions(
            &db,
            &SessionSearchQuery {
                text: Some("kubernetes".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.session.id.as_str()).collect();
        assert_eq!(hits.len(), 3, "unrelated session must not match: {ids:?}");
        assert_eq!(ids[0], by_title.id, "title match ranks first");
        assert!(ids.contains(&by_transcript.id.as_str()));
        assert!(ids.contains(&by_response.id.as_str()));

        let transcript_item = hits
            .iter()
            .find(|h| h.session.id == by_transcript.id)
            .unwrap();
        let snippet = transcript_item.snippet.as_deref().unwrap();
        assert!(
            snippet.to_lowercase().contains("kubernetes"),
            "snippet: {snippet}"
        );
        let response_item = hits
            .iter()
            .find(|h| h.session.id == by_response.id)
            .unwrap();
        assert!(response_item
            .snippet
            .as_deref()
            .unwrap()
            .contains("manifests"));
    }

    #[test]
    fn honours_filters_and_paging() {
        let db = testutil::db();
        testutil::seed_mode(&db, "interview", "Interview");
        let a = SessionRepository::create(&db, "general", Some("rust chat".into())).unwrap();
        let b = SessionRepository::create(&db, "interview", Some("rust interview".into())).unwrap();

        let q = SessionSearchQuery {
            text: Some("rust".into()),
            mode_id: Some("interview".into()),
            ..Default::default()
        };
        let hits = search_sessions(&db, &q).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session.id, b.id);

        let q = SessionSearchQuery {
            text: Some("rust".into()),
            to: Some("2000-01-01".into()),
            ..Default::default()
        };
        assert!(search_sessions(&db, &q).unwrap().is_empty());

        let page1 = search_sessions(
            &db,
            &SessionSearchQuery {
                text: Some("rust".into()),
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        let page2 = search_sessions(
            &db,
            &SessionSearchQuery {
                text: Some("rust".into()),
                limit: Some(1),
                offset: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page1.len(), 1);
        assert_eq!(page2.len(), 1);
        assert_ne!(page1[0].session.id, page2[0].session.id);
        let _ = a;
    }

    #[test]
    fn mode_name_matches_and_like_escaping() {
        let db = testutil::db();
        testutil::seed_mode(&db, "sales", "Sales Calls");
        let s = SessionRepository::create(&db, "sales", None).unwrap();
        let hits = search_sessions(
            &db,
            &SessionSearchQuery {
                text: Some("Sales".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session.id, s.id);

        // LIKE wildcards in user input must not match everything.
        SessionRepository::create(&db, "general", Some("plain".into())).unwrap();
        let hits = search_sessions(
            &db,
            &SessionSearchQuery {
                text: Some("%".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            hits.is_empty(),
            "escaped wildcard must not match: {hits:#?}"
        );
    }
}
