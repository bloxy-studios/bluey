//! Chunk retrieval: FTS5 `bm25` keyword search, cosine-similarity semantic
//! search over stored embeddings, or a 50/50 hybrid (`Auto`), with scope- and
//! kind-aware boosting. Scores are normalized to `0..=1`.

use std::collections::HashMap;

use bluey_core::error::BlueyError;
use bluey_core::types::context::RetrievedChunk;
use bluey_core::types::documents::{
    DocumentKind, DocumentScope, RetrievalQuery, RetrievalStrategy, ScopeRef,
};

use crate::db::Database;
use crate::error::SqlExt;
use crate::fts::fts_match_query;
use crate::repositories::documents::{kind_filter_sql, scope_filter_sql};
use crate::repositories::DocumentRepository;

/// Default number of chunks returned when the query has no `limit`.
const DEFAULT_LIMIT: usize = 8;

/// Cosine similarity in `-1..=1`; `0.0` for empty or mismatched vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a <= 0.0 || norm_b <= 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

struct Candidate {
    chunk: RetrievedChunk,
    keyword: Option<f32>,  // normalized 0..1
    semantic: Option<f32>, // normalized 0..1
}

/// Retrieve the most relevant document chunks for `query`.
///
/// * `Keyword` — FTS5 `bm25` over `document_chunks_fts` with a sanitized match
///   expression (quotes stripped, terms OR-ed, stop words dropped).
/// * `Semantic` — cosine similarity over chunks that have embeddings; requires
///   `query_embedding` (returns nothing without it).
/// * `Auto` (default) — hybrid: both signals normalized to `0..1` and combined
///   50/50 when embeddings exist, keyword-only otherwise.
///
/// Results are boosted by scope priority (session > mode > global) and by a
/// document-kind relevance table keyed off the query intent (candidate-style
/// questions boost resume/CV/skills/experience docs; role-style questions boost
/// job/role descriptions). `query.kinds` acts as a hard filter.
pub fn retrieve(
    db: &Database,
    query: &RetrievalQuery,
    query_embedding: Option<&[f32]>,
) -> Result<Vec<RetrievedChunk>, BlueyError> {
    let strategy = query.strategy.unwrap_or_default();
    let limit = query
        .limit
        .map(|l| l as usize)
        .filter(|l| *l > 0)
        .unwrap_or(DEFAULT_LIMIT);
    let kinds = query.kinds.as_deref();

    let mut candidates: HashMap<String, Candidate> = HashMap::new();

    if matches!(
        strategy,
        RetrievalStrategy::Auto | RetrievalStrategy::Keyword
    ) {
        for (chunk, score) in keyword_candidates(db, &query.query, &query.scopes, kinds)? {
            candidates
                .entry(chunk.chunk_id.clone())
                .and_modify(|c| c.keyword = Some(score))
                .or_insert(Candidate {
                    chunk,
                    keyword: Some(score),
                    semantic: None,
                });
        }
    }
    if matches!(
        strategy,
        RetrievalStrategy::Auto | RetrievalStrategy::Semantic
    ) {
        if let Some(embedding) = query_embedding {
            for (chunk, score) in semantic_candidates(db, embedding, &query.scopes, kinds)? {
                candidates
                    .entry(chunk.chunk_id.clone())
                    .and_modify(|c| c.semantic = Some(score))
                    .or_insert(Candidate {
                        chunk,
                        keyword: None,
                        semantic: Some(score),
                    });
            }
        }
    }

    let has_semantic = candidates.values().any(|c| c.semantic.is_some());
    let intent = detect_intent(&query.query);

    let mut scored: Vec<RetrievedChunk> = candidates
        .into_values()
        .map(|c| {
            let base = match strategy {
                RetrievalStrategy::Keyword => c.keyword.unwrap_or(0.0),
                RetrievalStrategy::Semantic => c.semantic.unwrap_or(0.0),
                RetrievalStrategy::Auto => {
                    if has_semantic {
                        0.5 * c.keyword.unwrap_or(0.0) + 0.5 * c.semantic.unwrap_or(0.0)
                    } else {
                        c.keyword.unwrap_or(0.0)
                    }
                }
            };
            let boosted =
                base * scope_boost(c.chunk.scope) * kind_boost(c.chunk.document_kind, intent);
            RetrievedChunk {
                score: boosted,
                ..c.chunk
            }
        })
        .collect();

    // Renormalize back into 0..1 if boosting pushed anything above 1.
    let max = scored.iter().map(|c| c.score).fold(0.0f32, f32::max);
    if max > 1.0 {
        for c in &mut scored {
            c.score /= max;
        }
    }
    for c in &mut scored {
        c.score = c.score.clamp(0.0, 1.0);
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.chunk_id.cmp(&b.chunk_id))
    });
    scored.truncate(limit);
    Ok(scored)
}

/// FTS5 keyword candidates with bm25 rank normalized to `0..1`.
fn keyword_candidates(
    db: &Database,
    query_text: &str,
    scopes: &[ScopeRef],
    kinds: Option<&[DocumentKind]>,
) -> Result<Vec<(RetrievedChunk, f32)>, BlueyError> {
    let Some(match_expr) = fts_match_query(query_text) else {
        return Ok(Vec::new());
    };
    let (scope_sql, scope_params) = scope_filter_sql(scopes)?;
    let (kind_sql, kind_params) = kind_filter_sql(kinds)?;
    type RawRow = (String, String, String, String, String, String, f64);
    let rows: Vec<RawRow> = db.with_conn(|conn| {
        let sql = format!(
            "SELECT c.id, c.document_id, d.title, d.kind, d.scope, c.content,
                    bm25(document_chunks_fts, 1.0, 1.5) AS rank
               FROM document_chunks_fts f
               JOIN document_chunks c ON c.id = f.chunk_id
               JOIN documents d ON d.id = c.document_id
              WHERE document_chunks_fts MATCH ?1
                AND d.index_status = 'indexed'
                AND {scope_sql} AND {kind_sql}
              ORDER BY rank
              LIMIT 64"
        );
        let mut stmt = conn.prepare(&sql).sql()?;
        let params: Vec<&str> = std::iter::once(match_expr.as_str())
            .chain(scope_params.iter().map(String::as_str))
            .chain(kind_params.iter().map(String::as_str))
            .collect();
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params), |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })
            .sql()?;
        rows.collect::<Result<Vec<_>, _>>().sql()
    })?;

    // bm25() is more-negative-is-better; flip the sign and normalize by the best.
    let best = rows.iter().map(|r| -r.6).fold(0.0f64, f64::max);
    if best <= 0.0 {
        // All ranks zero (or no rows) — give surviving matches a flat mid score.
        return rows
            .into_iter()
            .map(|row| build_chunk(row).map(|c| (c, 0.5)))
            .collect();
    }
    rows.into_iter()
        .map(|row| {
            let weight = ((-row.6).max(0.0) / best) as f32;
            build_chunk(row).map(|c| (c, weight.clamp(0.0, 1.0)))
        })
        .collect()
}

fn build_chunk(
    (chunk_id, document_id, title, kind, scope, content, _rank): (
        String,
        String,
        String,
        String,
        String,
        String,
        f64,
    ),
) -> Result<RetrievedChunk, BlueyError> {
    Ok(RetrievedChunk {
        chunk_id,
        document_id,
        document_title: title,
        document_kind: crate::repositories::from_enum_str(&kind)?,
        content,
        score: 0.0,
        scope: crate::repositories::from_enum_str(&scope)?,
    })
}

/// Cosine similarity candidates over stored embeddings, mapped from `-1..1` to `0..1`.
fn semantic_candidates(
    db: &Database,
    query_embedding: &[f32],
    scopes: &[ScopeRef],
    kinds: Option<&[DocumentKind]>,
) -> Result<Vec<(RetrievedChunk, f32)>, BlueyError> {
    let embedded = DocumentRepository::chunks_with_embeddings(db, scopes, kinds)?;
    let mut out = Vec::new();
    for chunk in embedded {
        let similarity = cosine(query_embedding, &chunk.embedding);
        let score = ((similarity + 1.0) / 2.0).clamp(0.0, 1.0);
        out.push((
            RetrievedChunk {
                chunk_id: chunk.chunk_id,
                document_id: chunk.document_id,
                document_title: chunk.document_title,
                document_kind: chunk.document_kind,
                content: chunk.content,
                score: 0.0,
                scope: chunk.scope,
            },
            score,
        ));
    }
    Ok(out)
}

// ── Boosting ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryIntent {
    Candidate,
    Role,
    Both,
    Neutral,
}

const CANDIDATE_HINTS: &[&str] = &[
    "my",
    "me",
    "myself",
    "yourself",
    "resume",
    "cv",
    "experience",
    "background",
    "skills",
    "worked",
    "strength",
    "strengths",
    "weakness",
    "weaknesses",
    "career",
    "accomplishment",
    "accomplishments",
    "projects",
];
const ROLE_HINTS: &[&str] = &[
    "role",
    "job",
    "position",
    "company",
    "team",
    "responsibilities",
    "responsibility",
    "requirements",
    "requirement",
    "salary",
    "hiring",
    "opening",
];

fn detect_intent(query: &str) -> QueryIntent {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let candidate = words.iter().any(|w| CANDIDATE_HINTS.contains(w));
    let role = words.iter().any(|w| ROLE_HINTS.contains(w));
    match (candidate, role) {
        (true, true) => QueryIntent::Both,
        (true, false) => QueryIntent::Candidate,
        (false, true) => QueryIntent::Role,
        (false, false) => QueryIntent::Neutral,
    }
}

/// Session-scoped context beats mode-scoped beats global.
fn scope_boost(scope: DocumentScope) -> f32 {
    match scope {
        DocumentScope::Session => 1.15,
        DocumentScope::Mode => 1.08,
        DocumentScope::Global => 1.0,
    }
}

/// Simple kind relevance table per query intent.
fn kind_boost(kind: DocumentKind, intent: QueryIntent) -> f32 {
    let candidate = matches!(
        kind,
        DocumentKind::Resume
            | DocumentKind::Cv
            | DocumentKind::Experience
            | DocumentKind::Skills
            | DocumentKind::Bio
            | DocumentKind::Portfolio
    );
    let role = matches!(
        kind,
        DocumentKind::JobDescription | DocumentKind::RoleDescription | DocumentKind::CompanyNotes
    );
    match intent {
        QueryIntent::Candidate if candidate => 1.25,
        QueryIntent::Role if role => 1.25,
        QueryIntent::Both if candidate || role => 1.2,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::index::add_document;
    use crate::testutil;
    use bluey_core::types::documents::DocumentScope;
    use pretty_assertions::assert_eq;

    fn seed_docs(db: &Database) -> (String, String) {
        let resume = add_document(
            db,
            &testutil::doc_input(
                DocumentKind::Resume,
                DocumentScope::Global,
                None,
                "EXPERIENCE\n\nStaff engineer at Acme. Deep Rust experience: async services, SQLite, profiling.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        let jd = add_document(
            db,
            &testutil::doc_input(
                DocumentKind::JobDescription,
                DocumentScope::Mode,
                Some("general"),
                "About the role: we need a platform engineer. Responsibilities include Kafka pipelines and on-call.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        (resume.id, jd.id)
    }

    #[test]
    fn cosine_basics() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), 1.0);
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        assert_eq!(cosine(&[1.0, 0.0], &[-1.0, 0.0]), -1.0);
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0, "length mismatch");
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0, "zero norm");
    }

    #[test]
    fn keyword_retrieval_finds_and_scores() {
        let db = testutil::db();
        let (resume_id, jd_id) = seed_docs(&db);
        let query = RetrievalQuery {
            query: "tell me about your rust experience".into(),
            scopes: vec![],
            kinds: None,
            limit: None,
            strategy: Some(RetrievalStrategy::Keyword),
        };
        let hits = retrieve(&db, &query, None).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(
            hits[0].document_id, resume_id,
            "resume chunk should win: {hits:#?}"
        );
        assert!(hits[0].score > 0.0 && hits[0].score <= 1.0);
        assert!(hits.iter().all(|h| h.score >= 0.0 && h.score <= 1.0));
        let _ = jd_id;

        // Garbage query → no candidates.
        let none = retrieve(
            &db,
            &RetrievalQuery {
                query: "the of and".into(),
                ..query.clone()
            },
            None,
        )
        .unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn kind_filter_and_role_boost() {
        let db = testutil::db();
        let (_resume_id, jd_id) = seed_docs(&db);
        // Hard filter to job descriptions only.
        let query = RetrievalQuery {
            query: "what are the responsibilities of the role".into(),
            scopes: vec![],
            kinds: Some(vec![
                DocumentKind::JobDescription,
                DocumentKind::RoleDescription,
            ]),
            limit: Some(4),
            strategy: Some(RetrievalStrategy::Keyword),
        };
        let hits = retrieve(&db, &query, None).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.document_id == jd_id));
    }

    #[test]
    fn scope_filter_restricts_results() {
        let db = testutil::db();
        seed_docs(&db);
        let query = RetrievalQuery {
            query: "rust experience kafka".into(),
            scopes: vec![ScopeRef {
                scope: DocumentScope::Mode,
                scope_id: Some("general".into()),
            }],
            kinds: None,
            limit: None,
            strategy: Some(RetrievalStrategy::Keyword),
        };
        let hits = retrieve(&db, &query, None).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.scope == DocumentScope::Mode));
    }

    #[test]
    fn semantic_and_hybrid_paths() {
        let db = testutil::db();
        let (resume_id, jd_id) = seed_docs(&db);
        // Give every chunk an embedding: resume → [1,0], jd → [0,1].
        for (doc_id, emb) in [(&resume_id, [1.0f32, 0.0]), (&jd_id, [0.0f32, 1.0])] {
            for chunk in crate::repositories::DocumentRepository::chunks(&db, doc_id).unwrap() {
                crate::repositories::DocumentRepository::set_embedding(&db, &chunk.id, &emb)
                    .unwrap();
            }
        }
        let semantic_query = RetrievalQuery {
            query: "anything at all".into(),
            scopes: vec![],
            kinds: None,
            limit: Some(2),
            strategy: Some(RetrievalStrategy::Semantic),
        };
        let hits = retrieve(&db, &semantic_query, Some(&[1.0, 0.0])).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].document_id, resume_id);
        assert!(hits[0].score > hits[1].score);

        // Semantic without an embedding yields nothing.
        assert!(retrieve(&db, &semantic_query, None).unwrap().is_empty());

        // Hybrid: keyword agrees with the embedding → resume stays on top.
        let auto_query = RetrievalQuery {
            query: "rust experience".into(),
            scopes: vec![],
            kinds: None,
            limit: Some(3),
            strategy: None, // Auto
        };
        let hits = retrieve(&db, &auto_query, Some(&[1.0, 0.0])).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].document_id, resume_id);
        assert!(
            hits[0].score > 0.5,
            "hybrid winner should score high: {}",
            hits[0].score
        );
    }

    #[test]
    fn candidate_intent_boosts_resume_over_notes() {
        let db = testutil::db();
        // Two global docs with the same matching term; only the kind differs.
        let resume = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Resume,
                DocumentScope::Global,
                None,
                "Kubernetes production work.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        let notes = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Notes,
                DocumentScope::Global,
                None,
                "Kubernetes production work.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        let query = RetrievalQuery {
            query: "tell me about my kubernetes experience".into(),
            scopes: vec![],
            kinds: None,
            limit: None,
            strategy: Some(RetrievalStrategy::Keyword),
        };
        let hits = retrieve(&db, &query, None).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].document_id, resume.id,
            "resume must outrank notes for candidate questions"
        );
        assert!(hits[0].score > hits[1].score);
        let _ = notes;
    }

    #[test]
    fn intent_detection() {
        assert_eq!(
            detect_intent("tell me about my skills"),
            QueryIntent::Candidate
        );
        assert_eq!(
            detect_intent("what does the job require"),
            QueryIntent::Role
        );
        assert_eq!(
            detect_intent("how does my experience fit the role"),
            QueryIntent::Both
        );
        assert_eq!(detect_intent("what is a b-tree"), QueryIntent::Neutral);
    }
}
