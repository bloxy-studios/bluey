//! `auth_*` commands — browser sign-in (ADR 0008). The WebView only ever sees
//! [`AuthStatus`]; tokens stay in Rust.

use bluey_core::types::{AuthStatus, SignInStart};
use bluey_core::BlueyResult;
use tauri::{AppHandle, State};

use crate::state::AppCore;

#[tauri::command]
pub async fn auth_get_status(core: State<'_, AppCore>) -> BlueyResult<AuthStatus> {
    core.auth.status().await
}

/// Open the system browser on Clerk's authorization page and wait for the
/// redirect (deep link or loopback). The calling window is brought to the
/// front once the sign-in completes.
#[tauri::command]
pub async fn auth_begin_sign_in(
    app: AppHandle,
    window: tauri::Window,
    core: State<'_, AppCore>,
) -> BlueyResult<SignInStart> {
    core.auth
        .begin_sign_in(&app, Some(window.label().to_string()))
        .await
}

/// Abandon a pending browser sign-in.
#[tauri::command]
pub async fn auth_cancel_sign_in(core: State<'_, AppCore>) -> BlueyResult<AuthStatus> {
    core.auth.cancel_sign_in().await
}

/// Sign out: revoke (best effort) and forget the tokens and the cached user.
#[tauri::command]
pub async fn auth_clear_session(core: State<'_, AppCore>) -> BlueyResult<AuthStatus> {
    core.auth.clear_session().await
}

/// Open Clerk's hosted Account Portal (profile & security) in the browser.
#[tauri::command]
pub async fn auth_open_account_portal(app: AppHandle, core: State<'_, AppCore>) -> BlueyResult<()> {
    core.auth.open_account_portal(&app)
}
