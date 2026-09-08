//! `auth_*` commands (Clerk session persistence + Frontend API proxy).

use std::collections::HashMap;

use bluey_core::types::{AuthStatus, AuthUser};
use bluey_core::BlueyResult;
use tauri::State;

use crate::auth::FapiResponse;
use crate::state::AppCore;

#[tauri::command]
pub async fn auth_get_status(core: State<'_, AppCore>) -> BlueyResult<AuthStatus> {
    core.auth.status().await
}

#[tauri::command]
pub async fn auth_store_session(
    core: State<'_, AppCore>,
    client_token: String,
    user: AuthUser,
) -> BlueyResult<AuthStatus> {
    core.auth.store_session(client_token, user).await
}

#[tauri::command]
pub async fn auth_store_token(core: State<'_, AppCore>, client_token: String) -> BlueyResult<()> {
    core.auth.store_token(client_token).await
}

#[tauri::command]
pub async fn auth_load_client_token(core: State<'_, AppCore>) -> BlueyResult<Option<String>> {
    core.auth.load_client_token().await
}

#[tauri::command]
pub async fn auth_clear_session(core: State<'_, AppCore>) -> BlueyResult<AuthStatus> {
    core.auth.clear_session().await
}

#[tauri::command]
pub async fn auth_fapi_fetch(
    core: State<'_, AppCore>,
    url: String,
    method: String,
    headers: HashMap<String, String>,
    body: Option<String>,
) -> BlueyResult<FapiResponse> {
    core.auth.fapi_fetch(url, method, headers, body).await
}
