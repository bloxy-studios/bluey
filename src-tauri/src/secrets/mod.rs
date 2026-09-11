//! Secrets in the macOS Keychain via the `keyring` crate
//! (service `com.codewithabdul.bluey`). Values never appear in logs or errors.
//!
//! Two allow-lists, two layers:
//! * the **store** ([`SecretsStore`]) accepts every key Bluey owns —
//!   `provider:<id>:api_key`, the research / agent keys and the Rust-only
//!   sign-in tokens (`auth:clerk:*`);
//! * the **WebView** ([`validate_webview_key`], in front of `secrets_set` /
//!   `secrets_has` / `secrets_delete`) may touch only the keys of
//!   `SECRET_KEYS` in `src/lib/tauri/commands.ts` — API keys entered in
//!   Settings. Sign-in and (ADR 0009) subscription-account tokens never cross
//!   that boundary in either direction.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bluey_core::{BlueyError, BlueyResult};

use crate::storage::BUNDLE_ID;

/// Keychain key for a provider API key.
pub fn provider_key(provider_id: &str) -> String {
    format!("provider:{provider_id}:api_key")
}

/// Fixed keys.
pub const EXA_KEY: &str = "research:exa:api_key";
pub const FIRECRAWL_KEY: &str = "research:firecrawl:api_key";
pub const AGENT_ANTHROPIC_KEY: &str = "agent:anthropic:api_key";
/// Legacy (clerk-js in the WebView, ADR 0003): deleted at boot when present.
pub const CLERK_TOKEN_KEY: &str = "auth:clerk:client_token";
/// OAuth tokens of the browser sign-in (ADR 0008): JSON `{access_token, refresh_token, expires_at, id_token}`.
pub const CLERK_OAUTH_TOKENS_KEY: &str = "auth:clerk:oauth_tokens";

/// Keychain key of a subscription account's OAuth tokens (ADR 0009): JSON
/// `{access_token, refresh_token, expires_at, id_token}` (`bluey_oauth::TokenSet`),
/// read and written only by `AccountsManager`.
pub fn account_tokens_key(account_id: &str) -> String {
    format!("account:{account_id}:oauth_tokens")
}

/// `provider:<id>:api_key` with a non-empty id.
fn is_provider_api_key(key: &str) -> bool {
    key.strip_prefix("provider:")
        .and_then(|rest| rest.strip_suffix(":api_key"))
        .is_some_and(|id| !id.is_empty())
}

/// `account:<id>:oauth_tokens` with a non-empty id — Rust-only, never the WebView.
fn is_account_tokens_key(key: &str) -> bool {
    key.strip_prefix("account:")
        .and_then(|rest| rest.strip_suffix(":oauth_tokens"))
        .is_some_and(|id| !id.is_empty())
}

/// Whether the WebView may manage `key` through `secrets_set` /
/// `secrets_has` / `secrets_delete`: exactly the `SECRET_KEYS` of
/// `src/lib/tauri/commands.ts` — provider API keys and the research / agent
/// keys entered in Settings. Sign-in tokens (`auth:*`) and subscription-account
/// tokens (`account:*`, ADR 0009) are Rust-only and never pass here.
pub fn is_webview_secret_key(key: &str) -> bool {
    matches!(key, EXA_KEY | FIRECRAWL_KEY | AGENT_ANTHROPIC_KEY) || is_provider_api_key(key)
}

/// The command-layer gate in front of [`SecretsStore`] for WebView requests.
pub fn validate_webview_key(key: &str) -> BlueyResult<()> {
    if is_webview_secret_key(key) {
        Ok(())
    } else {
        Err(BlueyError::invalid_params(
            "this secret is not managed from the settings UI",
        ))
    }
}

/// Keychain-backed secret store.
pub struct SecretsStore {
    service: String,
}

impl Default for SecretsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretsStore {
    pub fn new() -> Self {
        Self {
            service: BUNDLE_ID.to_string(),
        }
    }

    /// The storage-level allow-list: everything Bluey owns, including the
    /// Rust-only sign-in tokens the WebView must never reach.
    fn validate_key(key: &str) -> BlueyResult<()> {
        let allowed = matches!(
            key,
            EXA_KEY
                | FIRECRAWL_KEY
                | AGENT_ANTHROPIC_KEY
                | CLERK_TOKEN_KEY
                | CLERK_OAUTH_TOKENS_KEY
        ) || is_provider_api_key(key)
            || is_account_tokens_key(key);
        if allowed {
            Ok(())
        } else {
            Err(BlueyError::invalid_params("unknown secret key"))
        }
    }

    fn entry(&self, key: &str) -> BlueyResult<keyring::Entry> {
        keyring::Entry::new(&self.service, key)
            .map_err(|_| BlueyError::storage("keychain", "cannot open the keychain entry"))
    }

    /// Read a secret synchronously (bootstrap paths only).
    pub fn get_sync(&self, key: &str) -> BlueyResult<Option<String>> {
        Self::validate_key(key)?;
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(BlueyError::storage(
                "keychain",
                "failed to read from the keychain",
            )),
        }
    }

    /// Whether a secret exists (synchronous).
    pub fn has_sync(&self, key: &str) -> bool {
        matches!(self.get_sync(key), Ok(Some(_)))
    }

    /// Store a secret (synchronous; used from bootstrap env import).
    pub fn set_sync(&self, key: &str, value: &str) -> BlueyResult<()> {
        Self::validate_key(key)?;
        if value.is_empty() {
            return Err(BlueyError::invalid_params("secret value must not be empty"));
        }
        self.entry(key)?
            .set_password(value)
            .map_err(|_| BlueyError::storage("keychain", "failed to write to the keychain"))
    }

    /// Delete a secret (synchronous). Missing entries are not an error.
    pub fn delete_sync(&self, key: &str) -> BlueyResult<()> {
        Self::validate_key(key)?;
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(BlueyError::storage(
                "keychain",
                "failed to delete from the keychain",
            )),
        }
    }

    /// Async wrappers — Keychain access can block, so hop to the blocking pool.
    pub async fn get(self: &std::sync::Arc<Self>, key: &str) -> BlueyResult<Option<String>> {
        let this = self.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || this.get_sync(&key))
            .await
            .map_err(|e| BlueyError::internal(format!("keychain task panicked: {e}")))?
    }

    pub async fn has(self: &std::sync::Arc<Self>, key: &str) -> BlueyResult<bool> {
        Ok(self.get(key).await?.is_some())
    }

    pub async fn set(self: &std::sync::Arc<Self>, key: &str, value: String) -> BlueyResult<()> {
        let this = self.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || this.set_sync(&key, &value))
            .await
            .map_err(|e| BlueyError::internal(format!("keychain task panicked: {e}")))?
    }

    pub async fn delete(self: &std::sync::Arc<Self>, key: &str) -> BlueyResult<()> {
        let this = self.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || this.delete_sync(&key))
            .await
            .map_err(|e| BlueyError::internal(format!("keychain task panicked: {e}")))?
    }
}

/// Load `.env.local` and then `.env` into the process environment. Variables
/// that are already set win, and an earlier file wins over a later one (so
/// `.env.local` overrides `.env`, as with Vite and Bun). Directories searched,
/// in order: for development builds the repository root and `src-tauri` — the
/// Tauri CLI runs the app from `src-tauri`, so a plain relative `.env` would
/// miss the repository's files — then the current directory and the directory
/// of the executable.
///
/// Values are never logged. The loaded paths are returned so the caller can log
/// them once logging is up (this runs first: `BLUEY_LOG_LEVEL` may live here).
pub fn load_dotenv() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(debug_assertions)]
    {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Some(root) = manifest.parent() {
            dirs.push(root.to_path_buf());
        }
        dirs.push(manifest.to_path_buf());
    }
    dirs.push(PathBuf::from("."));
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        dirs.push(dir);
    }

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut loaded = Vec::new();
    for dir in dirs {
        for name in [".env.local", ".env"] {
            let path = dir.join(name);
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            // The current directory may be one of the directories above.
            if !seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())) {
                continue;
            }
            for (key, value) in crate::dotenv::parse(&contents) {
                if std::env::var_os(&key).is_none() {
                    std::env::set_var(&key, &value);
                }
            }
            loaded.push(path);
        }
    }
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_webview_gate_mirrors_secret_keys_in_commands_ts() {
        for key in [
            "provider:gemini:api_key",
            "provider:azure-foundry:api_key",
            EXA_KEY,
            FIRECRAWL_KEY,
            AGENT_ANTHROPIC_KEY,
        ] {
            assert!(
                is_webview_secret_key(key),
                "{key} must be manageable from Settings"
            );
            assert!(validate_webview_key(key).is_ok());
        }
        for key in [
            CLERK_TOKEN_KEY,
            CLERK_OAUTH_TOKENS_KEY,
            "auth:anything:else",
            "account:chatgpt-1:oauth_tokens",
            "provider::api_key",
            "provider:gemini:oauth_tokens",
            "provider:gemini",
            "",
        ] {
            assert!(!is_webview_secret_key(key), "{key} must stay Rust-only");
            assert!(validate_webview_key(key).is_err());
        }
    }

    #[test]
    fn the_store_admits_the_rust_only_keys_the_webview_cannot() {
        assert!(SecretsStore::validate_key(CLERK_OAUTH_TOKENS_KEY).is_ok());
        assert!(SecretsStore::validate_key(CLERK_TOKEN_KEY).is_ok());
        assert!(SecretsStore::validate_key("provider:gemini:api_key").is_ok());
        assert!(SecretsStore::validate_key(EXA_KEY).is_ok());
        assert!(SecretsStore::validate_key(&account_tokens_key("chatgpt")).is_ok());
        assert!(SecretsStore::validate_key("account::oauth_tokens").is_err());
        assert!(SecretsStore::validate_key("account:chatgpt:api_key").is_err());
        assert!(SecretsStore::validate_key("provider::api_key").is_err());
        assert!(SecretsStore::validate_key("random").is_err());
        // The gate in front of the WebView never admits account tokens.
        assert!(validate_webview_key(&account_tokens_key("chatgpt")).is_err());
    }
}
