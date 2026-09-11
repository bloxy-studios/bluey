//! `accounts_*` commands — subscription sign-in (ADR 0009). The WebView only
//! ever sees [`ProviderAccount`] and [`ProviderModelCatalog`]; tokens stay in
//! Rust. Completion of a sign-in arrives as `accounts.changed`.

use bluey_core::types::{
    AccountConnectOptions, FingerprintProbe, ProviderAccount, ProviderModelCatalog,
};
use bluey_core::BlueyResult;
use tauri::{AppHandle, State};

use crate::state::AppCore;

#[tauri::command]
pub fn accounts_list(core: State<'_, AppCore>) -> BlueyResult<Vec<ProviderAccount>> {
    Ok(core.accounts.list())
}

/// Start a sign-in with a subscription provider (`chatgpt`, `claude`,
/// `antigravity`); returns the account in `connecting`.
#[tauri::command]
pub async fn accounts_connect(
    app: AppHandle,
    core: State<'_, AppCore>,
    provider_id: String,
    options: Option<AccountConnectOptions>,
) -> BlueyResult<ProviderAccount> {
    core.accounts
        .connect(&app, &provider_id, options.unwrap_or_default())
        .await
}

/// Import the official client's local sign-in on this Mac (read-only).
#[tauri::command]
pub async fn accounts_import(
    core: State<'_, AppCore>,
    provider_id: String,
) -> BlueyResult<ProviderAccount> {
    core.accounts.import(&provider_id).await
}

#[tauri::command]
pub async fn accounts_cancel_connect(
    core: State<'_, AppCore>,
    account_id: String,
) -> BlueyResult<ProviderAccount> {
    core.accounts.cancel_connect(&account_id).await
}

/// Deliver a pasted `code#state` for a manual-code flow.
#[tauri::command]
pub async fn accounts_submit_code(
    core: State<'_, AppCore>,
    account_id: String,
    code: String,
) -> BlueyResult<ProviderAccount> {
    core.accounts.submit_code(&account_id, code).await
}

#[tauri::command]
pub async fn accounts_disconnect(core: State<'_, AppCore>, account_id: String) -> BlueyResult<()> {
    core.accounts.disconnect(&account_id).await
}

#[tauri::command]
pub fn accounts_status(
    core: State<'_, AppCore>,
    account_id: String,
) -> BlueyResult<ProviderAccount> {
    core.accounts.status(&account_id)
}

/// The cached catalog, if one was fetched (no network).
#[tauri::command]
pub fn accounts_catalog(
    core: State<'_, AppCore>,
    account_id: String,
) -> BlueyResult<Option<ProviderModelCatalog>> {
    core.accounts.status(&account_id)?;
    Ok(core.accounts.catalog(&account_id))
}

#[tauri::command]
pub async fn accounts_refresh_catalog(
    core: State<'_, AppCore>,
    account_id: String,
    force: Option<bool>,
) -> BlueyResult<ProviderModelCatalog> {
    core.accounts
        .refresh_catalog(&account_id, force.unwrap_or(false))
        .await
}

/// Developer mode: one tiny request, reporting whether the plan paid for it.
#[tauri::command]
pub async fn accounts_probe_fingerprint(
    core: State<'_, AppCore>,
    account_id: String,
) -> BlueyResult<FingerprintProbe> {
    core.accounts.probe_fingerprint(&account_id).await
}
