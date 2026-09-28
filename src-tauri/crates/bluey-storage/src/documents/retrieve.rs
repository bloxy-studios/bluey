//! Chunk retrieval: FTS5 `bm25` keyword search, cosine-similarity semantic
//! search over stored embeddings, or a 50/50 hybrid (`Auto`), with scope- and
//! kind-aware boosting — plus `Leading`, which lists the first chunks of every
//! document in scope without matching. Scores are normalized to `0..=1`.

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

/// Raw cosine similarity below which a chunk is unrelated to the query.
/// Modern embedding models put unrelated text around 0.0–0.2, and the
/// `(cos+1)/2` mapping would otherwise score it ~0.5–0.6.
const SEMANTIC_MIN_COSINE: f32 = 0.25;

/// Score of a `Leading` chunk: included because of what it is, not because
/// it matched, so it sits below any real match.
const LEADING_SCORE: f32 = 0.5;

/// Smallest raw bm25 rank (sign-flipped) that counts as a real keyword match.
/// FTS5 floors a term's IDF to 1e-6 when it occurs in half or more of all
/// chunks, so a best rank below this means no matched term discriminates.
const MIN_KEYWORD_RANK: f64 = 1e-3;

/// Flat score for keyword matches below [`MIN_KEYWORD_RANK`]: weak, instead
/// of normalized up to 1.0 by the best (equally weak) row.
const WEAK_KEYWORD_SCORE: f32 = 0.25;

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
///
/// `Leading` ignores the query text: it returns the first `limit` chunks in
/// document order (session, then mode, then global documents; newest document
/// first), each scored [`LEADING_SCORE`].
///
/// `embedding_model` is the `providerId/model` tag of `query_embedding`; when
/// given, only chunks embedded by that model are compared.
pub fn retrieve(
    db: &Database,
    query: &RetrievalQuery,
    query_embedding: Option<&[f32]>,
    embedding_model: Option<&str>,
) -> Result<Vec<RetrievedChunk>, BlueyError> {
    let strategy = query.strategy.unwrap_or_default();
    let limit = query
        .limit
        .map(|l| l as usize)
        .filter(|l| *l > 0)
        .unwrap_or(DEFAULT_LIMIT);
    let kinds = query.kinds.as_deref();

    if strategy == RetrievalStrategy::Leading {
        return leading_chunks(db, &query.scopes, kinds, limit);
    }

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
            let embedded =
                semantic_candidates(db, embedding, embedding_model, &query.scopes, kinds)?;
            for (chunk, score) in embedded {
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
                RetrievalStrategy::Leading => unreachable!("handled before matching"),
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
    if best < MIN_KEYWORD_RANK {
        // No matched term discriminates (or no rows): a flat weak score.
        return rows
            .into_iter()
            .map(|row| build_chunk(row).map(|c| (c, WEAK_KEYWORD_SCORE)))
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

/// Cosine similarity candidates over stored embeddings, mapped from `-1..1` to
/// `0..1`. Chunks below [`SEMANTIC_MIN_COSINE`] are unrelated and dropped.
fn semantic_candidates(
    db: &Database,
    query_embedding: &[f32],
    embedding_model: Option<&str>,
    scopes: &[ScopeRef],
    kinds: Option<&[DocumentKind]>,
) -> Result<Vec<(RetrievedChunk, f32)>, BlueyError> {
    let embedded = DocumentRepository::chunks_with_embeddings(db, scopes, kinds, embedding_model)?;
    let mut out = Vec::new();
    for chunk in embedded {
        // Vectors from another model / MRL size live in a different space: skip
        // them instead of scoring them at zero (they are re-embedded separately).
        if chunk.embedding.len() != query_embedding.len() {
            continue;
        }
        let similarity = cosine(query_embedding, &chunk.embedding);
        if similarity < SEMANTIC_MIN_COSINE {
            continue;
        }
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

/// The first `limit` chunks in document order: session, mode, then global
/// documents, newest document first, chunks in position order.
fn leading_chunks(
    db: &Database,
    scopes: &[ScopeRef],
    kinds: Option<&[DocumentKind]>,
    limit: usize,
) -> Result<Vec<RetrievedChunk>, BlueyError> {
    let (scope_sql, scope_params) = scope_filter_sql(scopes)?;
    let (kind_sql, kind_params) = kind_filter_sql(kinds)?;
    type RawRow = (String, String, String, String, String, String, f64);
    let rows: Vec<RawRow> = db.with_conn(|conn| {
        let sql = format!(
            "SELECT c.id, c.document_id, d.title, d.kind, d.scope, c.content, 0.0
               FROM document_chunks c
               JOIN documents d ON d.id = c.document_id
              WHERE d.index_status = 'indexed'
                AND {scope_sql} AND {kind_sql}
              ORDER BY CASE d.scope WHEN 'session' THEN 0 WHEN 'mode' THEN 1 ELSE 2 END,
                       d.updated_at DESC, d.id, c.chunk_index
              LIMIT {limit}"
        );
        let mut stmt = conn.prepare(&sql).sql()?;
        let params: Vec<&str> = scope_params
            .iter()
            .chain(&kind_params)
            .map(String::as_str)
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
    rows.into_iter()
        .map(|row| {
            build_chunk(row).map(|chunk| RetrievedChunk {
                score: LEADING_SCORE,
                ..chunk
            })
        })
        .collect()
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
        let hits = retrieve(&db, &query, None, None).unwrap();
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
        let hits = retrieve(&db, &query, None, None).unwrap();
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
        let hits = retrieve(&db, &query, None, None).unwrap();
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
        let hits = retrieve(&db, &semantic_query, Some(&[1.0, 0.0]), None).unwrap();
        assert_eq!(hits.len(), 1, "the orthogonal JD is unrelated: {hits:#?}");
        assert_eq!(hits[0].document_id, resume_id);
        assert!(hits.iter().all(|h| h.document_id != jd_id));
        // A query unrelated to every chunk returns nothing, not a ranked list.
        assert!(retrieve(&db, &semantic_query, Some(&[-1.0, -1.0]), None)
            .unwrap()
            .is_empty());

        // Semantic without an embedding yields nothing.
        assert!(retrieve(&db, &semantic_query, None, None)
            .unwrap()
            .is_empty());
        // Vectors from another embedding size are skipped, not scored at zero.
        assert!(retrieve(&db, &semantic_query, Some(&[1.0, 0.0, 0.0]), None)
            .unwrap()
            .is_empty());

        // Hybrid: keyword agrees with the embedding → resume stays on top.
        let auto_query = RetrievalQuery {
            query: "rust experience".into(),
            scopes: vec![],
            kinds: None,
            limit: Some(3),
            strategy: None, // Auto
        };
        let hits = retrieve(&db, &auto_query, Some(&[1.0, 0.0]), None).unwrap();
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
        let hits = retrieve(&db, &query, None, None).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].document_id, resume.id,
            "resume must outrank notes for candidate questions"
        );
        assert!(hits[0].score > hits[1].score);
        let _ = notes;
    }

    #[test]
    fn keyword_matches_without_a_discriminating_term_stay_weak() {
        let db = testutil::db();
        // One chunk in the whole index: every term occurs in all chunks.
        add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Notes,
                DocumentScope::Global,
                None,
                "Quarterly planning notes about the billing migration.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        let query = RetrievalQuery {
            query: "billing".into(),
            scopes: vec![],
            kinds: None,
            limit: None,
            strategy: Some(RetrievalStrategy::Keyword),
        };
        let hits = retrieve(&db, &query, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].score, WEAK_KEYWORD_SCORE,
            "not normalized up to 1.0"
        );

        // Once the term is rare in the index, the best match scores 1.0.
        for text in [
            "Team offsite agenda and travel.",
            "Hiring loop feedback template.",
            "Incident review for the search outage.",
        ] {
            add_document(
                &db,
                &testutil::doc_input(DocumentKind::Notes, DocumentScope::Global, None, text),
                |_| unreachable!(),
            )
            .unwrap();
        }
        let hits = retrieve(&db, &query, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].score, 1.0);
    }

    #[test]
    fn leading_returns_first_chunks_in_scope_order_without_matching() {
        let db = testutil::db();
        let paragraphs: Vec<String> = (0..30)
            .map(|i| format!("Section {i}. Led the payments platform rewrite across {i} regions, owning reliability, hiring and the on-call rotation for the billing services."))
            .collect();
        let resume = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Resume,
                DocumentScope::Global,
                None,
                &paragraphs.join("\n\n"),
            ),
            |_| unreachable!(),
        )
        .unwrap();
        let resume_chunks =
            crate::repositories::DocumentRepository::chunks(&db, &resume.id).unwrap();
        assert!(resume_chunks.len() > 2, "fixture must span several chunks");
        let session_pi = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::PersonalInstructions,
                DocumentScope::Session,
                Some("ses_1"),
                "Answer in British English.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        let global_pi = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::PersonalInstructions,
                DocumentScope::Global,
                None,
                "Keep answers under thirty seconds.",
            ),
            |_| unreachable!(),
        )
        .unwrap();

        // No word overlap with the résumé: keyword finds nothing…
        let keyword = RetrievalQuery {
            query: "tell me about yourself".into(),
            scopes: vec![],
            kinds: Some(vec![DocumentKind::Resume]),
            limit: Some(2),
            strategy: Some(RetrievalStrategy::Keyword),
        };
        assert!(retrieve(&db, &keyword, None, None).unwrap().is_empty());
        // …while `Leading` returns the first chunks in document order.
        let leading = RetrievalQuery {
            strategy: Some(RetrievalStrategy::Leading),
            ..keyword
        };
        let hits = retrieve(&db, &leading, Some(&[1.0, 0.0]), None).unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.chunk_id.as_str()).collect();
        let expected: Vec<&str> = resume_chunks[..2].iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, expected);
        assert!(hits.iter().all(|h| h.score == LEADING_SCORE));

        // Scope order: session before global; out-of-scope sessions excluded.
        let instructions = RetrievalQuery {
            query: String::new(),
            scopes: vec![
                ScopeRef {
                    scope: DocumentScope::Session,
                    scope_id: Some("ses_1".into()),
                },
                ScopeRef {
                    scope: DocumentScope::Global,
                    scope_id: None,
                },
            ],
            kinds: Some(vec![DocumentKind::PersonalInstructions]),
            limit: Some(6),
            strategy: Some(RetrievalStrategy::Leading),
        };
        let hits = retrieve(&db, &instructions, None, None).unwrap();
        let docs: Vec<&str> = hits.iter().map(|h| h.document_id.as_str()).collect();
        assert_eq!(docs, vec![session_pi.id.as_str(), global_pi.id.as_str()]);
        let other_session = RetrievalQuery {
            scopes: vec![ScopeRef {
                scope: DocumentScope::Session,
                scope_id: Some("ses_2".into()),
            }],
            ..instructions
        };
        assert!(retrieve(&db, &other_session, None, None)
            .unwrap()
            .is_empty());
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
