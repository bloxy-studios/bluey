//! `responses_*` commands (stored answers + feedback).

use bluey_core::types::{BlueyResponse, FeedbackCategory, FeedbackRating};
use bluey_core::BlueyResult;
use bluey_storage::ResponseRepository;
use tauri::State;

use crate::state::AppCore;

#[tauri::command]
pub async fn responses_save(
    core: State<'_, AppCore>,
    response: BlueyResponse,
) -> BlueyResult<BlueyResponse> {
    core.storage
        .run(move |db| ResponseRepository::save(db, &response))
        .await
}

#[tauri::command]
pub async fn responses_list(
    core: State<'_, AppCore>,
    session_id: String,
    limit: Option<u32>,
) -> BlueyResult<Vec<BlueyResponse>> {
    core.storage
        .run(move |db| ResponseRepository::list(db, &session_id, limit))
        .await
}

#[tauri::command]
pub async fn responses_get(core: State<'_, AppCore>, id: String) -> BlueyResult<BlueyResponse> {
    core.storage
        .run(move |db| ResponseRepository::get(db, &id))
        .await
}

#[tauri::command]
pub async fn responses_feedback(
    core: State<'_, AppCore>,
    response_id: String,
    rating: FeedbackRating,
    categories: Option<Vec<FeedbackCategory>>,
    comment: Option<String>,
) -> BlueyResult<BlueyResponse> {
    core.storage
        .run(move |db| {
            ResponseRepository::set_feedback(db, &response_id, rating, categories, comment)
        })
        .await
}

#[tauri::command]
pub async fn responses_delete(core: State<'_, AppCore>, id: String) -> BlueyResult<()> {
    core.storage
        .run(move |db| ResponseRepository::delete(db, &id))
        .await
}
