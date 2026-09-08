//! Documents + chunks. `document_chunks_fts` is synced by triggers; embeddings
//! are stored as little-endian `f32` blobs on the chunk rows.

use bluey_core::error::BlueyError;
use bluey_core::now_iso;
use bluey_core::types::documents::{
    BlueyDocument, DocumentChunk, DocumentIndexStatus, DocumentKind, DocumentScope, ScopeRef,
};
use rusqlite::{params, OptionalExtension, Row};

use super::{from_enum_str, not_found, opt_from_json, opt_to_json, to_enum_str};
use crate::db::Database;
use crate::error::SqlExt;

/// Encode an embedding vector as little-endian `f32` bytes.
pub fn f32s_to_le_bytes(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode little-endian `f32` bytes back into a vector (trailing partial floats
/// are ignored).
pub fn le_bytes_to_f32s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// A chunk with its decoded embedding and document context, used by semantic retrieval.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedChunk {
    pub chunk_id: String,
    pub document_id: String,
    pub document_title: String,
    pub document_kind: DocumentKind,
    pub scope: DocumentScope,
    pub content: String,
    pub heading: Option<String>,
    pub embedding: Vec<f32>,
}

struct DocRow {
    id: String,
    title: String,
    kind: String,
    format: String,
    scope: String,
    scope_id: Option<String>,
    source_path: Option<String>,
    size_bytes: i64,
    chunk_count: i64,
    index_status: String,
    has_embeddings: bool,
    metadata: Option<String>,
    created_at: String,
    updated_at: String,
    embedding_model: Option<String>,
    embedding_dimensions: Option<i64>,
}

impl DocRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            title: row.get(1)?,
            kind: row.get(2)?,
            format: row.get(3)?,
            scope: row.get(4)?,
            scope_id: row.get(5)?,
            source_path: row.get(6)?,
            size_bytes: row.get(7)?,
            chunk_count: row.get(8)?,
            index_status: row.get(9)?,
            has_embeddings: row.get(10)?,
            metadata: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
            embedding_model: row.get(14)?,
            embedding_dimensions: row.get(15)?,
        })
    }

    fn into_document(self) -> Result<BlueyDocument, BlueyError> {
        Ok(BlueyDocument {
            id: self.id,
            title: self.title,
            kind: from_enum_str(&self.kind)?,
            format: from_enum_str(&self.format)?,
            scope: from_enum_str(&self.scope)?,
            scope_id: self.scope_id,
            source_path: self.source_path,
            size_bytes: self.size_bytes.max(0) as u64,
            chunk_count: self.chunk_count.max(0) as u32,
            index_status: from_enum_str(&self.index_status)?,
            has_embeddings: self.has_embeddings,
            embedding_model: self.embedding_model,
            embedding_dimensions: self
                .embedding_dimensions
                .filter(|d| *d > 0)
                .map(|d| d as u32),
            metadata: opt_from_json(self.metadata)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

const DOC_COLS: &str = "id, title, kind, format, scope, scope_id, source_path, size_bytes,
    chunk_count, index_status, has_embeddings, metadata, created_at, updated_at,
    embedding_model, embedding_dimensions";

/// Build a `WHERE` fragment restricting `documents d` to the given scopes.
/// Empty scope list = no restriction. Returns the SQL and its parameters.
pub(crate) fn scope_filter_sql(scopes: &[ScopeRef]) -> Result<(String, Vec<String>), BlueyError> {
    if scopes.is_empty() {
        return Ok(("1=1".to_string(), Vec::new()));
    }
    let mut clauses = Vec::new();
    let mut params_out = Vec::new();
    for scope_ref in scopes {
        let scope = to_enum_str(&scope_ref.scope)?;
        match &scope_ref.scope_id {
            Some(id) => {
                clauses.push("(d.scope = ? AND d.scope_id = ?)".to_string());
                params_out.push(scope);
                params_out.push(id.clone());
            }
            None => {
                clauses.push("(d.scope = ?)".to_string());
                params_out.push(scope);
            }
        }
    }
    Ok((format!("({})", clauses.join(" OR ")), params_out))
}

/// Build a `d.kind IN (…)` fragment. `None` = no restriction.
pub(crate) fn kind_filter_sql(
    kinds: Option<&[DocumentKind]>,
) -> Result<(String, Vec<String>), BlueyError> {
    match kinds {
        None | Some([]) => Ok(("1=1".to_string(), Vec::new())),
        Some(kinds) => {
            let placeholders = vec!["?"; kinds.len()].join(", ");
            let values = kinds
                .iter()
                .map(to_enum_str)
                .collect::<Result<Vec<_>, _>>()?;
            Ok((format!("d.kind IN ({placeholders})"), values))
        }
    }
}

/// Persistence for `documents` / `document_chunks` (+ FTS via triggers).
pub struct DocumentRepository;

impl DocumentRepository {
    /// Insert a document with its full text and chunks in one transaction.
    pub fn insert(
        db: &Database,
        doc: &BlueyDocument,
        content: &str,
        chunks: &[DocumentChunk],
    ) -> Result<(), BlueyError> {
        let kind = to_enum_str(&doc.kind)?;
        let format = to_enum_str(&doc.format)?;
        let scope = to_enum_str(&doc.scope)?;
        let status = to_enum_str(&doc.index_status)?;
        let metadata = opt_to_json(&doc.metadata)?;
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO documents (id, title, kind, format, scope, scope_id, source_path,
                    content, size_bytes, chunk_count, index_status, has_embeddings, metadata,
                    created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    doc.id,
                    doc.title,
                    kind,
                    format,
                    scope,
                    doc.scope_id,
                    doc.source_path,
                    content,
                    doc.size_bytes as i64,
                    doc.chunk_count as i64,
                    status,
                    doc.has_embeddings,
                    metadata,
                    doc.created_at,
                    doc.updated_at
                ],
            )
            .sql()?;
            for chunk in chunks {
                conn.execute(
                    "INSERT INTO document_chunks (id, document_id, chunk_index, content, tokens, heading, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![chunk.id, chunk.document_id, chunk.index, chunk.content, chunk.tokens, chunk.heading, now_iso()],
                )
                .sql()?;
            }
            Ok(())
        })
    }

    /// Fetch a document (`storage.not_found` when missing).
    pub fn get(db: &Database, id: &str) -> Result<BlueyDocument, BlueyError> {
        let row = db.with_conn(|conn| {
            conn.query_row(
                &format!("SELECT {DOC_COLS} FROM documents WHERE id = ?1"),
                [id],
                DocRow::read,
            )
            .optional()
            .sql()
        })?;
        row.ok_or_else(|| not_found("document", id))?
            .into_document()
    }

    /// Full normalized text of a document.
    pub fn get_text(db: &Database, id: &str) -> Result<String, BlueyError> {
        let text: Option<String> = db.with_conn(|conn| {
            conn.query_row("SELECT content FROM documents WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .sql()
        })?;
        text.ok_or_else(|| not_found("document", id))
    }

    /// Documents, newest first, optionally restricted to a scope (and scope id).
    pub fn list(
        db: &Database,
        scope: Option<DocumentScope>,
        scope_id: Option<&str>,
    ) -> Result<Vec<BlueyDocument>, BlueyError> {
        let scope_str = scope.map(|s| to_enum_str(&s)).transpose()?;
        let rows = db.with_conn(|conn| {
            let sql = format!(
                "SELECT {DOC_COLS} FROM documents
                  WHERE (?1 IS NULL OR scope = ?1)
                    AND (?2 IS NULL OR scope_id = ?2)
                  ORDER BY created_at DESC, id DESC"
            );
            let mut stmt = conn.prepare(&sql).sql()?;
            let rows = stmt
                .query_map(params![scope_str, scope_id], DocRow::read)
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })?;
        rows.into_iter().map(DocRow::into_document).collect()
    }

    /// Delete a document (chunks + FTS + mode attachments cascade). Returns
    /// whether a row was deleted.
    pub fn delete(db: &Database, id: &str) -> Result<bool, BlueyError> {
        db.with_conn(|conn| {
            conn.execute("DELETE FROM documents WHERE id = ?1", [id])
                .map(|n| n > 0)
                .sql()
        })
    }

    /// Delete all documents (optionally only one scope). Returns rows removed.
    pub fn delete_all(db: &Database, scope: Option<DocumentScope>) -> Result<u64, BlueyError> {
        let scope_str = scope.map(|s| to_enum_str(&s)).transpose()?;
        db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM documents WHERE (?1 IS NULL OR scope = ?1)",
                params![scope_str],
            )
            .map(|n| n as u64)
            .sql()
        })
    }

    /// Update the indexing status of a document.
    pub fn set_index_status(
        db: &Database,
        id: &str,
        status: DocumentIndexStatus,
    ) -> Result<(), BlueyError> {
        let status = to_enum_str(&status)?;
        let changed = db.with_conn(|conn| {
            conn.execute(
                "UPDATE documents SET index_status = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, status, now_iso()],
            )
            .sql()
        })?;
        if changed == 0 {
            return Err(not_found("document", id));
        }
        Ok(())
    }

    /// Chunks of a document in order.
    pub fn chunks(db: &Database, document_id: &str) -> Result<Vec<DocumentChunk>, BlueyError> {
        db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, document_id, chunk_index, content, tokens, heading
                       FROM document_chunks WHERE document_id = ?1 ORDER BY chunk_index",
                )
                .sql()?;
            let rows = stmt
                .query_map([document_id], |r| {
                    Ok(DocumentChunk {
                        id: r.get(0)?,
                        document_id: r.get(1)?,
                        index: r.get::<_, i64>(2)?.max(0) as u32,
                        content: r.get(3)?,
                        tokens: r.get::<_, i64>(4)?.max(0) as u32,
                        heading: r.get(5)?,
                    })
                })
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })
    }

    /// Store the embedding vector for a chunk (little-endian `f32` bytes) and
    /// flip the parent document's `has_embeddings` flag once every chunk has one.
    pub fn set_embedding(
        db: &Database,
        chunk_id: &str,
        embedding: &[f32],
    ) -> Result<(), BlueyError> {
        let bytes = f32s_to_le_bytes(embedding);
        db.transaction(|conn| {
            let document_id: Option<String> = conn
                .query_row("SELECT document_id FROM document_chunks WHERE id = ?1", [chunk_id], |r| r.get(0))
                .optional()
                .sql()?;
            let Some(document_id) = document_id else {
                return Err(not_found("document chunk", chunk_id));
            };
            conn.execute("UPDATE document_chunks SET embedding = ?2 WHERE id = ?1", params![chunk_id, bytes])
                .sql()?;
            let missing: i64 = conn
                .query_row(
                    "SELECT count(*) FROM document_chunks WHERE document_id = ?1 AND embedding IS NULL",
                    [&document_id],
                    |r| r.get(0),
                )
                .sql()?;
            conn.execute(
                "UPDATE documents SET has_embeddings = ?2 WHERE id = ?1",
                params![document_id, missing == 0],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Record which embedding model (`providerId/model`) and vector size the
    /// document's chunks were embedded with. Call after every chunk has a vector.
    pub fn mark_embedded(
        db: &Database,
        document_id: &str,
        model_tag: &str,
        dimensions: u32,
    ) -> Result<(), BlueyError> {
        let changed = db.with_conn(|conn| {
            conn.execute(
                "UPDATE documents SET embedding_model = ?2, embedding_dimensions = ?3, updated_at = ?4
                  WHERE id = ?1",
                params![document_id, model_tag, dimensions as i64, now_iso()],
            )
            .sql()
        })?;
        if changed == 0 {
            return Err(not_found("document", document_id));
        }
        Ok(())
    }

    /// Indexed documents whose vectors were produced by a different model or
    /// size than `model_tag`/`dimensions` (or that have none yet). These need
    /// re-embedding before semantic retrieval can trust them.
    pub fn stale_embeddings(
        db: &Database,
        model_tag: &str,
        dimensions: u32,
    ) -> Result<Vec<String>, BlueyError> {
        db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM documents
                      WHERE index_status = 'indexed'
                        AND (has_embeddings = 0
                             OR embedding_model IS NULL
                             OR embedding_model <> ?1
                             OR embedding_dimensions IS NULL
                             OR embedding_dimensions <> ?2)
                      ORDER BY created_at, id",
                )
                .sql()?;
            let rows = stmt
                .query_map(params![model_tag, dimensions as i64], |r| {
                    r.get::<_, String>(0)
                })
                .sql()?;
            rows.collect::<Result<Vec<_>, _>>().sql()
        })
    }

    /// Chunks that have embeddings, restricted to `scopes` (empty = all) and
    /// optional `kinds`, joined with their document context.
    pub fn chunks_with_embeddings(
        db: &Database,
        scopes: &[ScopeRef],
        kinds: Option<&[DocumentKind]>,
    ) -> Result<Vec<EmbeddedChunk>, BlueyError> {
        let (scope_sql, scope_params) = scope_filter_sql(scopes)?;
        let (kind_sql, kind_params) = kind_filter_sql(kinds)?;
        type RawRow = (
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Vec<u8>,
        );
        let rows: Vec<RawRow> =
            db.with_conn(|conn| {
                let sql = format!(
                    "SELECT c.id, c.document_id, d.title, d.kind, d.scope, c.content, c.heading, c.embedding
                       FROM document_chunks c
                       JOIN documents d ON d.id = c.document_id
                      WHERE c.embedding IS NOT NULL AND {scope_sql} AND {kind_sql}
                      ORDER BY c.document_id, c.chunk_index"
                );
                let mut stmt = conn.prepare(&sql).sql()?;
                let all_params: Vec<&str> =
                    scope_params.iter().map(String::as_str).chain(kind_params.iter().map(String::as_str)).collect();
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(all_params), |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                        ))
                    })
                    .sql()?;
                rows.collect::<Result<Vec<_>, _>>().sql()
            })?;
        rows.into_iter()
            .map(
                |(chunk_id, document_id, title, kind, scope, content, heading, blob)| {
                    Ok(EmbeddedChunk {
                        chunk_id,
                        document_id,
                        document_title: title,
                        document_kind: from_enum_str(&kind)?,
                        scope: from_enum_str(&scope)?,
                        content,
                        heading,
                        embedding: le_bytes_to_f32s(&blob),
                    })
                },
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::index::add_document;
    use crate::testutil;
    use pretty_assertions::assert_eq;

    fn fts_count(db: &Database) -> i64 {
        db.with_conn(|c| {
            c.query_row("SELECT count(*) FROM document_chunks_fts", [], |r| r.get(0))
                .sql()
        })
        .unwrap()
    }

    #[test]
    fn embedding_bytes_round_trip() {
        let v = vec![0.5f32, -1.25, 3.0, f32::MIN_POSITIVE];
        assert_eq!(le_bytes_to_f32s(&f32s_to_le_bytes(&v)), v);
        assert_eq!(le_bytes_to_f32s(&[1, 2, 3]), Vec::<f32>::new());
    }

    #[test]
    fn insert_get_list_delete_with_fts() {
        let db = testutil::db();
        let doc = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Resume,
                DocumentScope::Global,
                None,
                "Rust engineer with systems experience.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        assert_eq!(doc.index_status, DocumentIndexStatus::Indexed);
        assert!(doc.chunk_count >= 1);
        assert_eq!(fts_count(&db), doc.chunk_count as i64);

        assert_eq!(DocumentRepository::get(&db, &doc.id).unwrap(), doc);
        assert_eq!(
            DocumentRepository::get_text(&db, &doc.id).unwrap(),
            "Rust engineer with systems experience."
        );
        assert_eq!(
            DocumentRepository::list(&db, Some(DocumentScope::Global), None)
                .unwrap()
                .len(),
            1
        );
        assert!(
            DocumentRepository::list(&db, Some(DocumentScope::Session), None)
                .unwrap()
                .is_empty()
        );

        assert!(DocumentRepository::delete(&db, &doc.id).unwrap());
        assert_eq!(fts_count(&db), 0, "chunk FTS rows must cascade away");
        assert_eq!(testutil::count(&db, "document_chunks"), 0);
        assert!(!DocumentRepository::delete(&db, &doc.id).unwrap());
        assert!(DocumentRepository::get(&db, &doc.id).is_err());
    }

    #[test]
    fn delete_all_scoped_and_status() {
        let db = testutil::db();
        let g = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Notes,
                DocumentScope::Global,
                None,
                "global doc",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Notes,
                DocumentScope::Mode,
                Some("general"),
                "mode doc",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        assert_eq!(
            DocumentRepository::delete_all(&db, Some(DocumentScope::Mode)).unwrap(),
            1
        );
        assert_eq!(DocumentRepository::delete_all(&db, None).unwrap(), 1);

        assert!(
            DocumentRepository::set_index_status(&db, &g.id, DocumentIndexStatus::Failed).is_err()
        );
    }

    #[test]
    fn embedding_model_tracking_and_stale_detection() {
        let db = testutil::db();
        let doc = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Notes,
                DocumentScope::Global,
                None,
                "Alpha paragraph about Rust.\n\nBeta paragraph about Python.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        let tag = "gemini/gemini-embedding-2";

        // Never embedded → stale for any model.
        assert_eq!(
            DocumentRepository::stale_embeddings(&db, tag, 768).unwrap(),
            vec![doc.id.clone()]
        );

        for chunk in DocumentRepository::chunks(&db, &doc.id).unwrap() {
            DocumentRepository::set_embedding(&db, &chunk.id, &[0.1, 0.2, 0.3, 0.4]).unwrap();
        }
        DocumentRepository::mark_embedded(&db, &doc.id, tag, 768).unwrap();
        let stored = DocumentRepository::get(&db, &doc.id).unwrap();
        assert!(stored.has_embeddings);
        assert_eq!(stored.embedding_model.as_deref(), Some(tag));
        assert_eq!(stored.embedding_dimensions, Some(768));

        // Same model and size → fresh; another size or model → stale.
        assert!(DocumentRepository::stale_embeddings(&db, tag, 768)
            .unwrap()
            .is_empty());
        assert_eq!(
            DocumentRepository::stale_embeddings(&db, tag, 1536).unwrap(),
            vec![doc.id.clone()]
        );
        assert_eq!(
            DocumentRepository::stale_embeddings(&db, "azure-foundry/text-embedding-3-small", 768)
                .unwrap(),
            vec![doc.id.clone()]
        );

        // Re-indexing wipes the vectors and forgets the model.
        crate::documents::index::reindex(&db, Some(&doc.id)).unwrap();
        let reindexed = DocumentRepository::get(&db, &doc.id).unwrap();
        assert!(!reindexed.has_embeddings);
        assert_eq!(reindexed.embedding_model, None);
        assert_eq!(reindexed.embedding_dimensions, None);
        assert_eq!(
            DocumentRepository::stale_embeddings(&db, tag, 768).unwrap(),
            vec![doc.id.clone()]
        );

        assert!(DocumentRepository::mark_embedded(&db, "missing", tag, 768).is_err());
    }

    #[test]
    fn embeddings_flow_and_scope_kind_filters() {
        let db = testutil::db();
        let doc = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Resume,
                DocumentScope::Global,
                None,
                "First paragraph about Rust.\n\nSecond paragraph about Python.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        let chunks = DocumentRepository::chunks(&db, &doc.id).unwrap();
        assert!(!chunks.is_empty());
        assert!(
            !DocumentRepository::get(&db, &doc.id)
                .unwrap()
                .has_embeddings
        );

        for (i, c) in chunks.iter().enumerate() {
            DocumentRepository::set_embedding(&db, &c.id, &[i as f32 + 1.0, 0.0]).unwrap();
        }
        assert!(
            DocumentRepository::get(&db, &doc.id)
                .unwrap()
                .has_embeddings
        );

        let all = DocumentRepository::chunks_with_embeddings(&db, &[], None).unwrap();
        assert_eq!(all.len(), chunks.len());
        assert_eq!(all[0].embedding, vec![1.0, 0.0]);
        assert_eq!(all[0].document_kind, DocumentKind::Resume);

        let scoped = DocumentRepository::chunks_with_embeddings(
            &db,
            &[ScopeRef {
                scope: DocumentScope::Session,
                scope_id: Some("nope".into()),
            }],
            None,
        )
        .unwrap();
        assert!(scoped.is_empty());

        let kind_miss = DocumentRepository::chunks_with_embeddings(
            &db,
            &[],
            Some(&[DocumentKind::JobDescription]),
        )
        .unwrap();
        assert!(kind_miss.is_empty());

        assert!(DocumentRepository::set_embedding(&db, "missing", &[1.0]).is_err());
    }
}
