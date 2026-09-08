//! `ai_*` commands: streaming generations over a `Channel`, cancellation,
//! embeddings, connection tests and model listing.

use bluey_core::types::{AiChunk, AiRequest, ConnectionTestResult};
use bluey_core::BlueyResult;
use tauri::ipc::Channel;
use tauri::State;

use crate::state::AppCore;

/// Validation and routing happen before this returns; chunks flow through
/// `on_chunk` (and mirror onto `ai.*` bus events).
#[tauri::command]
pub fn ai_stream(
    core: State<'_, AppCore>,
    request: AiRequest,
    on_chunk: Channel<AiChunk>,
) -> BlueyResult<()> {
    core.ai.stream(request, on_chunk)
}

#[tauri::command]
pub fn ai_cancel(core: State<'_, AppCore>, request_id: String) -> BlueyResult<bool> {
    Ok(core.ai.cancel(&request_id))
}

#[tauri::command]
pub fn ai_cancel_all(core: State<'_, AppCore>) -> BlueyResult<u32> {
    Ok(core.ai.cancel_all())
}

#[tauri::command]
pub async fn ai_embed(core: State<'_, AppCore>, texts: Vec<String>) -> BlueyResult<Vec<Vec<f32>>> {
    core.ai.embed(&texts).await
}

#[tauri::command]
pub async fn ai_test_connection(
    core: State<'_, AppCore>,
    provider_id: String,
    model: Option<String>,
) -> BlueyResult<ConnectionTestResult> {
    core.ai.test_connection(&provider_id, model).await
}

#[tauri::command]
pub async fn ai_list_models(
    core: State<'_, AppCore>,
    provider_id: String,
) -> BlueyResult<Vec<String>> {
    core.ai.list_models(&provider_id).await
}
