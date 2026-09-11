//! `settings_*` and `secrets_*` commands. Settings writes apply their side
//! effects (shortcuts, panel, protection, autostart, observation, log level,
//! retention) after persisting.

use bluey_core::types::Settings;
use bluey_core::BlueyResult;
use tauri::State;

use crate::settings::side_effects;
use crate::state::AppCore;

#[tauri::command]
pub fn settings_get(core: State<'_, AppCore>) -> BlueyResult<Settings> {
    Ok(core.settings.get())
}

#[tauri::command]
pub async fn settings_update(
    core: State<'_, AppCore>,
    patch: serde_json::Value,
) -> BlueyResult<Settings> {
    let (old, new) = core.settings.update(patch).await?;
    side_effects::apply(&core, &old, &new).await;
    Ok(new)
}

#[tauri::command]
pub async fn settings_reset(core: State<'_, AppCore>) -> BlueyResult<Settings> {
    let (old, new) = core.settings.reset().await?;
    side_effects::apply(&core, &old, &new).await;
    Ok(new)
}

/// The WebView manages API keys only (`SECRET_KEYS` in `commands.ts`);
/// sign-in and account tokens are Rust-only and rejected here, before the
/// store is touched (`crate::secrets::validate_webview_key`).
#[tauri::command]
pub async fn secrets_set(core: State<'_, AppCore>, key: String, value: String) -> BlueyResult<()> {
    crate::secrets::validate_webview_key(&key)?;
    core.secrets.set(&key, value).await?;
    core.settings.refresh_provider_keys();
    Ok(())
}

#[tauri::command]
pub async fn secrets_has(core: State<'_, AppCore>, key: String) -> BlueyResult<bool> {
    crate::secrets::validate_webview_key(&key)?;
    core.secrets.has(&key).await
}

#[tauri::command]
pub async fn secrets_delete(core: State<'_, AppCore>, key: String) -> BlueyResult<()> {
    crate::secrets::validate_webview_key(&key)?;
    core.secrets.delete(&key).await?;
    core.settings.refresh_provider_keys();
    Ok(())
}
