//! `documents_*` commands.

use bluey_core::types::{
    AddDocumentInput, BlueyDocument, DocumentScope, RetrievalQuery, RetrievedChunk,
};
use bluey_core::BlueyResult;
use tauri::State;

use crate::state::AppCore;

#[tauri::command]
pub async fn documents_add(
    core: State<'_, AppCore>,
    input: AddDocumentInput,
) -> BlueyResult<BlueyDocument> {
    core.documents.add(input).await
}

#[tauri::command]
pub async fn documents_list(
    core: State<'_, AppCore>,
    scope: Option<DocumentScope>,
    scope_id: Option<String>,
) -> BlueyResult<Vec<BlueyDocument>> {
    core.documents.list(scope, scope_id).await
}

#[tauri::command]
pub async fn documents_get(core: State<'_, AppCore>, id: String) -> BlueyResult<BlueyDocument> {
    core.documents.get(id).await
}

#[tauri::command]
pub async fn documents_get_text(core: State<'_, AppCore>, id: String) -> BlueyResult<String> {
    core.documents.get_text(id).await
}

#[tauri::command]
pub async fn documents_delete(core: State<'_, AppCore>, id: String) -> BlueyResult<()> {
    core.documents.delete(id).await
}

#[tauri::command]
pub async fn documents_delete_all(
    core: State<'_, AppCore>,
    scope: Option<DocumentScope>,
) -> BlueyResult<u64> {
    core.documents.delete_all(scope).await
}

#[tauri::command]
pub async fn documents_retrieve(
    core: State<'_, AppCore>,
    query: RetrievalQuery,
) -> BlueyResult<Vec<RetrievedChunk>> {
    core.documents.retrieve(query).await
}

#[tauri::command]
pub async fn documents_reindex(core: State<'_, AppCore>, id: Option<String>) -> BlueyResult<u32> {
    core.documents.reindex(id).await
}

#[tauri::command]
pub async fn documents_pick_files(core: State<'_, AppCore>) -> BlueyResult<Vec<String>> {
    core.documents.pick_files().await
}
