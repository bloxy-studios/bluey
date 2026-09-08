//! `sessions_*` commands.

use std::collections::BTreeMap;

use bluey_core::types::{
    Session, SessionDetail, SessionEvent, SessionEventType, SessionListItem, SessionNote,
    SessionSearchQuery, SessionSummary, SessionSummaryInput,
};
use bluey_core::BlueyResult;
use tauri::State;

use crate::state::AppCore;

#[tauri::command]
pub async fn sessions_start(
    core: State<'_, AppCore>,
    mode_id: Option<String>,
    title: Option<String>,
) -> BlueyResult<Session> {
    core.sessions.start(mode_id, title).await
}

#[tauri::command]
pub async fn sessions_pause(core: State<'_, AppCore>) -> BlueyResult<Session> {
    core.sessions.pause().await
}

#[tauri::command]
pub async fn sessions_resume(core: State<'_, AppCore>) -> BlueyResult<Session> {
    core.sessions.resume().await
}

#[tauri::command]
pub async fn sessions_end(core: State<'_, AppCore>) -> BlueyResult<Session> {
    core.sessions.end().await
}

#[tauri::command]
pub fn sessions_get_active(core: State<'_, AppCore>) -> BlueyResult<Option<Session>> {
    Ok(core.sessions.active())
}

#[tauri::command]
pub async fn sessions_list(
    core: State<'_, AppCore>,
    query: Option<SessionSearchQuery>,
) -> BlueyResult<Vec<SessionListItem>> {
    core.sessions.list(query.unwrap_or_default()).await
}

#[tauri::command]
pub async fn sessions_get(core: State<'_, AppCore>, id: String) -> BlueyResult<SessionDetail> {
    core.sessions.detail(id).await
}

#[tauri::command]
pub async fn sessions_search(
    core: State<'_, AppCore>,
    query: SessionSearchQuery,
) -> BlueyResult<Vec<SessionListItem>> {
    core.sessions.search(query).await
}

#[tauri::command]
pub async fn sessions_delete(core: State<'_, AppCore>, id: String) -> BlueyResult<()> {
    core.sessions.delete(id).await
}

#[tauri::command]
pub async fn sessions_delete_all(core: State<'_, AppCore>) -> BlueyResult<u64> {
    core.sessions.delete_all().await
}

#[tauri::command]
pub async fn sessions_rename(
    core: State<'_, AppCore>,
    id: String,
    title: String,
) -> BlueyResult<Session> {
    core.sessions.rename(id, title).await
}

#[tauri::command]
pub async fn sessions_add_event(
    core: State<'_, AppCore>,
    session_id: String,
    r#type: SessionEventType,
    title: String,
    detail: Option<String>,
    refs: Option<BTreeMap<String, String>>,
    confidence: Option<f32>,
) -> BlueyResult<SessionEvent> {
    core.sessions
        .add_event(session_id, r#type, title, detail, refs, confidence)
        .await
}

#[tauri::command]
pub async fn sessions_list_events(
    core: State<'_, AppCore>,
    session_id: String,
) -> BlueyResult<Vec<SessionEvent>> {
    core.sessions.list_events(session_id).await
}

#[tauri::command]
pub async fn sessions_add_note(
    core: State<'_, AppCore>,
    session_id: String,
    content: String,
) -> BlueyResult<SessionNote> {
    core.sessions.add_note(session_id, content).await
}

#[tauri::command]
pub async fn sessions_delete_note(core: State<'_, AppCore>, note_id: String) -> BlueyResult<()> {
    core.sessions.delete_note(note_id).await
}

#[tauri::command]
pub async fn sessions_save_summary(
    core: State<'_, AppCore>,
    summary: SessionSummaryInput,
) -> BlueyResult<SessionSummary> {
    core.sessions.save_summary(summary).await
}

#[tauri::command]
pub async fn sessions_get_summary(
    core: State<'_, AppCore>,
    session_id: String,
) -> BlueyResult<Option<SessionSummary>> {
    core.sessions.get_summary(session_id).await
}
