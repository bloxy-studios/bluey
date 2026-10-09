//! `settings_*` and `secrets_*` commands. Settings writes apply their side
//! effects (shortcuts, panel, protection, autostart, observation, log level,
//! retention) after persisting.

use bluey_core::types::Settings;
use bluey_core::BlueyResult;
use tauri::State;

use crate::secrets::health::{self, CredentialHealth, CredentialLabels};
use crate::secrets::SecretState;
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

/// Saved / locked / not set, for an API-key field. Never prompts.
#[tauri::command]
pub async fn secrets_state(core: State<'_, AppCore>, key: String) -> BlueyResult<SecretState> {
    crate::secrets::validate_webview_key(&key)?;
    core.secrets.state(&key).await
}

/// Settings → Privacy → Saved credentials: every saved Bluey-owned item with
/// its state and what it is for — names only, never values (ADR 0011).
#[tauri::command]
pub async fn secrets_health(core: State<'_, AppCore>) -> BlueyResult<Vec<CredentialHealth>> {
    let states = core.secrets.states().await?;
    Ok(health::describe(states, &Labels(&core)))
}

/// "Allow access": the one read that may show macOS's Keychain prompt, so the
/// user answers it once, on purpose — only for a listed credential macOS
/// holds back ([`health::allow_access`]); returns the new state.
#[tauri::command]
pub async fn secrets_allow_access(
    core: State<'_, AppCore>,
    key: String,
) -> BlueyResult<SecretState> {
    let state = health::allow_access(&core.secrets, &key).await?;
    core.settings.refresh_provider_keys();
    Ok(state)
}

struct Labels<'a>(&'a AppCore);

impl CredentialLabels for Labels<'_> {
    fn provider(&self, provider_id: &str) -> Option<String> {
        let settings = self.0.settings.get();
        let provider = settings.ai.providers.iter().find(|p| p.id == provider_id)?;
        Some(provider.name.clone())
    }

    fn account(&self, account_id: &str) -> Option<String> {
        let account = self
            .0
            .accounts
            .list()
            .into_iter()
            .find(|account| account.account_id == account_id)?;
        Some(
            self.provider(&account.provider_id)
                .unwrap_or(account.provider_id),
        )
    }
}
