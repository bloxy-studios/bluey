//! `responses_*` commands (stored answers + feedback).

use bluey_core::types::{BlueyResponse, FeedbackCategory, FeedbackRating};
use bluey_core::BlueyResult;
use bluey_storage::ResponseRepository;
use tauri::State;

use crate::state::AppCore;

/// Persist an answer. An answer asked outside a session is not kept while
/// session history is off: no session end would ever prune it (DATA-003).
#[tauri::command]
pub async fn responses_save(
    core: State<'_, AppCore>,
    response: BlueyResponse,
) -> BlueyResult<BlueyResponse> {
    if !keeps_response(&response, core.settings.get().privacy.store_session_history) {
        return Ok(response);
    }
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

/// Whether `responses_save` stores the answer: an answer that belongs to no
/// session is only kept while session history is on.
fn keeps_response(response: &BlueyResponse, store_session_history: bool) -> bool {
    response.session_id.is_some() || store_session_history
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sessionless_answer_is_not_kept_while_history_is_off() {
        let mut response: BlueyResponse = serde_json::from_value(serde_json::json!({
            "id": "res_1", "requestId": "req_1", "modeId": "general",
            "type": "answer", "content": "hi", "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        assert!(keeps_response(&response, true));
        assert!(!keeps_response(&response, false));
        response.session_id = Some("ses_1".into());
        assert!(
            keeps_response(&response, false),
            "history-off sessions prune at end"
        );
    }
}
