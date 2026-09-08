//! Document ingestion: parse → chunk → insert (doc + chunks + FTS) and
//! re-indexing from stored text.

use std::path::Path;

use bluey_core::error::BlueyError;
use bluey_core::types::documents::{
    AddDocumentInput, BlueyDocument, DocumentChunk, DocumentFormat, DocumentIndexStatus,
};
use bluey_core::{new_id, now_iso};
use rusqlite::params;

use super::chunk::{chunk_text, ChunkDraft};
use super::parse::{detect_format, parse_document, ParsedDocument};
use crate::db::Database;
use crate::error::SqlExt;
use crate::repositories::DocumentRepository;

/// Default chunk size (≈ tokens) used for indexing.
pub const DEFAULT_CHUNK_TOKENS: u32 = 350;
/// Default overlap (≈ tokens) between consecutive chunks.
pub const DEFAULT_CHUNK_OVERLAP_TOKENS: u32 = 40;

/// Ingest a document described by `input`: bytes come from `input.content`
/// (inline text) or from `input.path` via the injected `read_file` (the Tauri
/// layer supplies a scoped file reader — this crate never touches the FS).
///
/// On success the stored document is `indexed` with its chunks in
/// `document_chunks` (+ FTS). Parse failures are still recorded — the document
/// row is kept with `index_status: failed` and the error message in metadata —
/// so the UI can show what went wrong. Only invalid input (neither content nor
/// path) is an error.
pub fn add_document(
    db: &Database,
    input: &AddDocumentInput,
    read_file: impl Fn(&Path) -> Result<Vec<u8>, BlueyError>,
) -> Result<BlueyDocument, BlueyError> {
    let path = input.path.as_deref().map(Path::new);
    let (bytes, format) = match (&input.content, path) {
        (Some(content), _) => {
            let format = input.format.unwrap_or(DocumentFormat::Text);
            (content.clone().into_bytes(), format)
        }
        (None, Some(path)) => {
            let format = input
                .format
                .or_else(|| detect_format(path))
                .ok_or_else(|| {
                    BlueyError::invalid_params("cannot detect the document format from the path")
                })?;
            (read_file(path)?, format)
        }
        (None, None) => {
            return Err(BlueyError::invalid_params(
                "either content or path is required to add a document",
            ))
        }
    };

    let parse_result = parse_document(&bytes, format);
    let now = now_iso();
    let id = new_id("doc");
    let fallback_title = || {
        input
            .title
            .clone()
            .or_else(|| {
                path.and_then(|p| p.file_stem())
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Untitled".to_string())
    };

    match parse_result {
        Ok(ParsedDocument {
            text,
            title,
            metadata,
        }) => {
            let title = input
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .or(title)
                .unwrap_or_else(fallback_title);
            let drafts = chunk_text(&text, DEFAULT_CHUNK_TOKENS, DEFAULT_CHUNK_OVERLAP_TOKENS);
            let doc = BlueyDocument {
                id: id.clone(),
                title,
                kind: input.kind,
                format,
                scope: input.scope,
                scope_id: input.scope_id.clone(),
                source_path: input.path.clone(),
                size_bytes: bytes.len() as u64,
                chunk_count: drafts.len() as u32,
                index_status: DocumentIndexStatus::Indexed,
                has_embeddings: false,
                embedding_model: None,
                embedding_dimensions: None,
                metadata: if metadata.is_empty() {
                    None
                } else {
                    Some(metadata)
                },
                created_at: now.clone(),
                updated_at: now,
            };
            let chunks = drafts_to_chunks(&id, &drafts);
            DocumentRepository::insert(db, &doc, &text, &chunks)?;
            Ok(doc)
        }
        Err(err) => {
            let mut metadata = serde_json::Map::new();
            metadata.insert("error".into(), serde_json::json!(err.message));
            metadata.insert("errorCode".into(), serde_json::json!(err.code));
            let doc = BlueyDocument {
                id,
                title: fallback_title(),
                kind: input.kind,
                format,
                scope: input.scope,
                scope_id: input.scope_id.clone(),
                source_path: input.path.clone(),
                size_bytes: bytes.len() as u64,
                chunk_count: 0,
                index_status: DocumentIndexStatus::Failed,
                has_embeddings: false,
                embedding_model: None,
                embedding_dimensions: None,
                metadata: Some(metadata),
                created_at: now.clone(),
                updated_at: now,
            };
            DocumentRepository::insert(db, &doc, "", &[])?;
            tracing::warn!(document = %doc.id, code = %err.code, "document parse failed; stored as failed");
            Ok(doc)
        }
    }
}

fn drafts_to_chunks(document_id: &str, drafts: &[ChunkDraft]) -> Vec<DocumentChunk> {
    drafts
        .iter()
        .enumerate()
        .map(|(i, draft)| DocumentChunk {
            id: new_id("chk"),
            document_id: document_id.to_string(),
            index: i as u32,
            content: draft.content.clone(),
            tokens: draft.tokens,
            heading: draft.heading.clone(),
        })
        .collect()
}

/// Re-chunk documents from their stored text (one document when `id` is given,
/// otherwise all). Existing chunks — and therefore their FTS rows and
/// embeddings — are replaced; `has_embeddings` resets to false. Documents with
/// no stored text (failed parses) are skipped. Returns how many documents were
/// re-indexed.
pub fn reindex(db: &Database, id: Option<&str>) -> Result<u32, BlueyError> {
    let targets: Vec<(String, String)> = db.with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, content FROM documents WHERE (?1 IS NULL OR id = ?1)")
            .sql()?;
        let rows = stmt
            .query_map(params![id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .sql()?;
        rows.collect::<Result<Vec<_>, _>>().sql()
    })?;
    if let Some(id) = id {
        if targets.is_empty() {
            return Err(BlueyError::storage(
                "not_found",
                format!("document '{id}' was not found"),
            ));
        }
    }

    let mut reindexed = 0u32;
    for (doc_id, content) in targets {
        if content.trim().is_empty() {
            continue; // failed parse — nothing to index
        }
        let drafts = chunk_text(&content, DEFAULT_CHUNK_TOKENS, DEFAULT_CHUNK_OVERLAP_TOKENS);
        let chunks = drafts_to_chunks(&doc_id, &drafts);
        db.transaction(|conn| {
            conn.execute("DELETE FROM document_chunks WHERE document_id = ?1", [&doc_id]).sql()?;
            for chunk in &chunks {
                conn.execute(
                    "INSERT INTO document_chunks (id, document_id, chunk_index, content, tokens, heading, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![chunk.id, chunk.document_id, chunk.index, chunk.content, chunk.tokens, chunk.heading, now_iso()],
                )
                .sql()?;
            }
            conn.execute(
                "UPDATE documents SET chunk_count = ?2, index_status = 'indexed',
                        has_embeddings = 0, embedding_model = NULL, embedding_dimensions = NULL,
                        updated_at = ?3
                  WHERE id = ?1",
                params![doc_id, chunks.len() as i64, now_iso()],
            )
            .sql()?;
            Ok(())
        })?;
        reindexed += 1;
    }
    Ok(reindexed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;
    use bluey_core::types::documents::{DocumentKind, DocumentScope};
    use pretty_assertions::assert_eq;

    #[test]
    fn add_document_from_inline_content_indexes_chunks() {
        let db = testutil::db();
        let input = AddDocumentInput {
            title: None,
            kind: DocumentKind::Resume,
            scope: DocumentScope::Global,
            scope_id: None,
            path: None,
            content: Some("# Jane Doe Resume\n\nRust experience at Acme.".into()),
            format: Some(DocumentFormat::Md),
        };
        let doc = add_document(&db, &input, |_| unreachable!()).unwrap();
        assert_eq!(doc.index_status, DocumentIndexStatus::Indexed);
        assert_eq!(doc.title, "Jane Doe Resume", "markdown title is picked up");
        assert!(doc.chunk_count >= 1);
        let chunks = DocumentRepository::chunks(&db, &doc.id).unwrap();
        assert_eq!(chunks.len() as u32, doc.chunk_count);
        let fts: i64 = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT count(*) FROM document_chunks_fts WHERE document_chunks_fts MATCH 'acme'",
                    [],
                    |r| r.get(0),
                )
                .sql()
            })
            .unwrap();
        assert_eq!(fts, 1);
    }

    #[test]
    fn add_document_reads_file_and_detects_format() {
        let db = testutil::db();
        let input = AddDocumentInput {
            title: None,
            kind: DocumentKind::Notes,
            scope: DocumentScope::Session,
            scope_id: Some("ses_x".into()),
            path: Some("/fake/meeting notes.txt".into()),
            content: None,
            format: None,
        };
        let doc = add_document(&db, &input, |p| {
            assert_eq!(p, Path::new("/fake/meeting notes.txt"));
            Ok(b"remember the deadline".to_vec())
        })
        .unwrap();
        assert_eq!(doc.format, DocumentFormat::Txt);
        assert_eq!(doc.title, "meeting notes", "file stem becomes the title");
        assert_eq!(doc.scope_id.as_deref(), Some("ses_x"));

        let err = add_document(
            &db,
            &AddDocumentInput {
                path: None,
                content: None,
                ..input.clone()
            },
            |_| unreachable!(),
        )
        .unwrap_err();
        assert_eq!(err.code, "internal.invalid_params");
    }

    #[test]
    fn parse_failure_is_stored_as_failed() {
        let db = testutil::db();
        let input = AddDocumentInput {
            title: Some("Broken".into()),
            kind: DocumentKind::Other,
            scope: DocumentScope::Global,
            scope_id: None,
            path: Some("/fake/broken.docx".into()),
            content: None,
            format: None,
        };
        let doc = add_document(&db, &input, |_| Ok(b"not a zip at all".to_vec())).unwrap();
        assert_eq!(doc.index_status, DocumentIndexStatus::Failed);
        assert_eq!(doc.chunk_count, 0);
        let meta = doc.metadata.clone().unwrap();
        assert_eq!(
            meta.get("errorCode"),
            Some(&serde_json::json!("storage.parse"))
        );
        // Stored and listable; text is empty.
        assert_eq!(DocumentRepository::get_text(&db, &doc.id).unwrap(), "");
        assert_eq!(DocumentRepository::list(&db, None, None).unwrap().len(), 1);
    }

    #[test]
    fn reindex_rebuilds_chunks_and_resets_embeddings() {
        let db = testutil::db();
        let doc = add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Resume,
                DocumentScope::Global,
                None,
                "Alpha paragraph.\n\nBeta paragraph.",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        let chunks = DocumentRepository::chunks(&db, &doc.id).unwrap();
        for c in &chunks {
            DocumentRepository::set_embedding(&db, &c.id, &[1.0, 2.0]).unwrap();
        }
        assert!(
            DocumentRepository::get(&db, &doc.id)
                .unwrap()
                .has_embeddings
        );

        assert_eq!(reindex(&db, Some(&doc.id)).unwrap(), 1);
        let after = DocumentRepository::get(&db, &doc.id).unwrap();
        assert!(!after.has_embeddings, "reindex invalidates embeddings");
        assert_eq!(after.index_status, DocumentIndexStatus::Indexed);
        let new_chunks = DocumentRepository::chunks(&db, &doc.id).unwrap();
        assert_eq!(new_chunks.len() as u32, after.chunk_count);
        assert!(
            new_chunks
                .iter()
                .all(|c| !chunks.iter().any(|old| old.id == c.id)),
            "chunk ids rotate"
        );

        // Reindex-all skips failed docs but counts the good ones.
        add_document(
            &db,
            &AddDocumentInput {
                title: Some("bad".into()),
                kind: DocumentKind::Other,
                scope: DocumentScope::Global,
                scope_id: None,
                path: Some("/fake/bad.pdf".into()),
                content: None,
                format: None,
            },
            |_| Ok(b"nope".to_vec()),
        )
        .unwrap();
        assert_eq!(reindex(&db, None).unwrap(), 1);
        assert!(reindex(&db, Some("missing")).is_err());
    }
}
