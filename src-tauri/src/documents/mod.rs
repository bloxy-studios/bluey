//! Documents ("My Context", session and mode files): scoped file reading,
//! ingestion through `bluey_storage::add_document`, optional embeddings via the
//! embedding role, retrieval (keyword / semantic / hybrid), re-indexing and the
//! native file picker. The storage crate never touches the file system — the
//! reader here is the only place documents are read from disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{
    AddDocumentInput, BlueyDocument, DocumentIndexStatus, DocumentScope, RetrievalQuery,
    RetrievalStrategy, RetrievedChunk,
};
use bluey_core::{BlueyError, BlueyResult};
use bluey_storage::{DocumentRepository, ModeRepository, MAX_DOCUMENT_BYTES};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::ai::AiManager;
use crate::events::EventBus;
use crate::settings::SettingsManager;
use crate::storage::Storage;

/// File extensions offered by the picker (matches `DocumentFormat`).
const PICKER_EXTENSIONS: &[&str] = &["pdf", "docx", "txt", "md", "markdown"];

pub struct DocumentsManager {
    app: AppHandle,
    storage: Arc<Storage>,
    settings: Arc<SettingsManager>,
    ai: Arc<AiManager>,
    bus: Arc<EventBus>,
}

impl DocumentsManager {
    pub fn new(
        app: AppHandle,
        storage: Arc<Storage>,
        settings: Arc<SettingsManager>,
        ai: Arc<AiManager>,
        bus: Arc<EventBus>,
    ) -> Self {
        Self {
            app,
            storage,
            settings,
            ai,
            bus,
        }
    }

    /// Ingest a document (inline text or a file path), attach it to its mode
    /// when mode-scoped and embed it when the embedding role is usable.
    pub async fn add(&self, input: AddDocumentInput) -> BlueyResult<BlueyDocument> {
        if let Some(path) = input.path.as_deref() {
            validate_path(Path::new(path))?;
        }
        let stored_input = input.clone();
        let doc = self
            .storage
            .run(move |db| bluey_storage::add_document(db, &stored_input, read_scoped))
            .await?;

        if doc.scope == DocumentScope::Mode {
            if let Some(mode_id) = doc.scope_id.clone() {
                let doc_id = doc.id.clone();
                self.storage
                    .run(move |db| ModeRepository::attach_document(db, &mode_id, &doc_id))
                    .await?;
                self.publish_modes_changed().await;
            }
        }

        let mut doc = doc;
        if doc.index_status == DocumentIndexStatus::Indexed && self.ai.embeddings_ready() {
            match self.embed_document(&doc.id).await {
                Ok(()) => doc = self.get(doc.id.clone()).await?,
                Err(e) => {
                    tracing::warn!(document = %doc.id, error = %e, "embedding failed; keyword retrieval only")
                }
            }
        }
        Ok(doc)
    }

    pub async fn list(
        &self,
        scope: Option<DocumentScope>,
        scope_id: Option<String>,
    ) -> BlueyResult<Vec<BlueyDocument>> {
        self.storage
            .run(move |db| DocumentRepository::list(db, scope, scope_id.as_deref()))
            .await
    }

    pub async fn get(&self, id: String) -> BlueyResult<BlueyDocument> {
        self.storage
            .run(move |db| DocumentRepository::get(db, &id))
            .await
    }

    pub async fn get_text(&self, id: String) -> BlueyResult<String> {
        self.storage
            .run(move |db| DocumentRepository::get_text(db, &id))
            .await
    }

    /// Delete one document (chunks, FTS rows and mode attachments cascade).
    pub async fn delete(&self, id: String) -> BlueyResult<()> {
        let doc = self.get(id.clone()).await?;
        let deleted = self
            .storage
            .run(move |db| DocumentRepository::delete(db, &id))
            .await?;
        if !deleted {
            return Err(BlueyError::storage("not_found", "document was not found"));
        }
        if doc.scope == DocumentScope::Mode {
            self.publish_modes_changed().await;
        }
        Ok(())
    }

    /// Delete every document (optionally one scope). Returns rows removed.
    pub async fn delete_all(&self, scope: Option<DocumentScope>) -> BlueyResult<u64> {
        let removed = self
            .storage
            .run(move |db| DocumentRepository::delete_all(db, scope))
            .await?;
        if removed > 0 && scope.is_none_or(|s| s == DocumentScope::Mode) {
            self.publish_modes_changed().await;
        }
        Ok(removed)
    }

    /// Retrieve relevant chunks; the query is embedded when semantic retrieval
    /// is possible (embedding role usable and not keyword-only).
    pub async fn retrieve(&self, query: RetrievalQuery) -> BlueyResult<Vec<RetrievedChunk>> {
        let wants_semantic = !matches!(query.strategy, Some(RetrievalStrategy::Keyword));
        let embedding = if wants_semantic && self.ai.embeddings_ready() {
            match self
                .ai
                .embed(
                    std::slice::from_ref(&query.query),
                    &crate::ai::EmbedPurpose::Query,
                )
                .await
            {
                Ok(mut vectors) if !vectors.is_empty() => Some(vectors.remove(0)),
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(error = %e, "query embedding failed; falling back to keyword retrieval");
                    None
                }
            }
        } else {
            None
        };
        self.storage
            .run(move |db| bluey_storage::retrieve(db, &query, embedding.as_deref()))
            .await
    }

    /// Re-chunk (one or all documents) and re-embed when possible. Returns the
    /// number of documents re-indexed.
    pub async fn reindex(&self, id: Option<String>) -> BlueyResult<u32> {
        let target = id.clone();
        let count = self
            .storage
            .run(move |db| bluey_storage::reindex(db, target.as_deref()))
            .await?;
        if self.ai.embeddings_ready() {
            let docs = match id {
                Some(id) => vec![self.get(id).await?],
                None => self.list(None, None).await?,
            };
            for doc in docs
                .iter()
                .filter(|d| d.index_status == DocumentIndexStatus::Indexed)
            {
                if let Err(e) = self.embed_document(&doc.id).await {
                    tracing::warn!(document = %doc.id, error = %e, "re-embedding failed");
                }
            }
        }
        Ok(count)
    }

    /// Embed every chunk of a document with the embedding role.
    pub async fn embed_document(&self, document_id: &str) -> BlueyResult<()> {
        let id = document_id.to_string();
        let chunks = self
            .storage
            .run(move |db| DocumentRepository::chunks(db, &id))
            .await?;
        if chunks.is_empty() {
            return Ok(());
        }
        let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
        let title = self
            .get(document_id.to_string())
            .await
            .ok()
            .map(|d| d.title);
        let purpose = crate::ai::EmbedPurpose::Document { title };
        let vectors = self.ai.embed(&texts, &purpose).await?;
        if vectors.len() != chunks.len() {
            return Err(BlueyError::ai(
                "embeddings_parse",
                "the provider returned a different number of vectors than chunks",
            ));
        }
        let pairs: Vec<(String, Vec<f32>)> =
            chunks.into_iter().map(|c| c.id).zip(vectors).collect();
        self.storage
            .run(move |db| {
                for (chunk_id, vector) in &pairs {
                    DocumentRepository::set_embedding(db, chunk_id, vector)?;
                }
                Ok(())
            })
            .await
    }

    /// Native open dialog (multi-select, document extensions). Returns absolute
    /// paths; an empty list when the user cancels.
    pub async fn pick_files(&self) -> BlueyResult<Vec<String>> {
        let app = self.app.clone();
        let picked = tokio::task::spawn_blocking(move || {
            app.dialog()
                .file()
                .add_filter("Documents", PICKER_EXTENSIONS)
                .blocking_pick_files()
        })
        .await
        .map_err(|e| BlueyError::internal(format!("file dialog task failed: {e}")))?;
        Ok(picked
            .unwrap_or_default()
            .into_iter()
            .filter_map(|file| file.into_path().ok())
            .map(|path: PathBuf| path.to_string_lossy().into_owned())
            .collect())
    }

    /// Whether embeddings are enabled in settings (used by setup checks / UI).
    pub fn embeddings_enabled(&self) -> bool {
        self.settings.get().ai.embeddings_enabled
    }

    async fn publish_modes_changed(&self) {
        match self.storage.run(ModeRepository::list).await {
            Ok(modes) => self.bus.publish(BlueyEvent::ModesChanged(modes)),
            Err(e) => tracing::warn!(error = %e, "cannot list modes after document change"),
        }
    }
}

/// Reject anything that is not an existing, regular, reasonably sized file.
fn validate_path(path: &Path) -> BlueyResult<()> {
    if !path.is_absolute() {
        return Err(BlueyError::invalid_params(
            "document paths must be absolute",
        ));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|_| BlueyError::invalid_params("the document file does not exist"))?;
    if !metadata.is_file() {
        return Err(BlueyError::invalid_params(
            "the document path is not a file",
        ));
    }
    if metadata.len() as usize > MAX_DOCUMENT_BYTES {
        return Err(BlueyError::invalid_params(format!(
            "the document is larger than {} MB",
            MAX_DOCUMENT_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

/// The scoped reader handed to the storage crate.
fn read_scoped(path: &Path) -> BlueyResult<Vec<u8>> {
    validate_path(path)?;
    std::fs::read(path)
        .map_err(|e| BlueyError::storage("io", format!("cannot read the document: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_path_rejects_relative_missing_and_directories() {
        assert!(validate_path(Path::new("relative.txt")).is_err());
        assert!(validate_path(Path::new("/definitely/missing/file.txt")).is_err());
        let dir = std::env::temp_dir();
        assert!(validate_path(&dir).is_err());
    }

    #[test]
    fn read_scoped_reads_regular_files() {
        let dir = std::env::temp_dir().join(format!("bluey-doc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.md");
        std::fs::write(&file, b"# hello").unwrap();
        assert_eq!(read_scoped(&file).unwrap(), b"# hello");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
