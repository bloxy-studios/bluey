//! `ai_*` commands: streaming generations over a `Channel`, cancellation,
//! embeddings, connection tests, model listing and provider presets.

use bluey_core::types::{AiChunk, AiRequest, ConnectionTestResult, ModelRole, Settings};
use bluey_core::BlueyResult;
use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::ai::providers::EmbedPurpose;
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

/// What the frontend embeds texts for (`document` default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbedPurposeArg {
    Document,
    Query,
}

#[tauri::command]
pub async fn ai_embed(
    core: State<'_, AppCore>,
    texts: Vec<String>,
    purpose: Option<EmbedPurposeArg>,
) -> BlueyResult<Vec<Vec<f32>>> {
    let purpose = match purpose.unwrap_or(EmbedPurposeArg::Document) {
        EmbedPurposeArg::Document => EmbedPurpose::Document { title: None },
        EmbedPurposeArg::Query => EmbedPurpose::Query,
    };
    core.ai.embed(&texts, &purpose).await
}

#[tauri::command]
pub async fn ai_test_connection(
    core: State<'_, AppCore>,
    provider_id: String,
    model: Option<String>,
) -> BlueyResult<ConnectionTestResult> {
    core.ai.test_connection(&provider_id, model).await
}

/// Models one provider can serve; `role` narrows the list to models fit for it
/// (embeddings, transcription, or text generation).
#[tauri::command]
pub async fn ai_list_models(
    core: State<'_, AppCore>,
    provider_id: String,
    role: Option<ModelRole>,
) -> BlueyResult<Vec<String>> {
    core.ai.list_models(&provider_id, role).await
}

/// Point roles at the provider's recommended models. `overwrite = false` fills
/// only unassigned roles. Returns the new settings.
#[tauri::command]
pub async fn ai_apply_provider_presets(
    core: State<'_, AppCore>,
    provider_id: String,
    overwrite: bool,
) -> BlueyResult<Settings> {
    core.ai
        .apply_provider_presets(&provider_id, overwrite)
        .await
}
