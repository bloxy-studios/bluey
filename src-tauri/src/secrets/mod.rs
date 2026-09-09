//! Secrets in the macOS Keychain via the `keyring` crate
//! (service `com.codewithabdul.bluey`). Values never appear in logs or errors.
//!
//! Allowed keys mirror `SECRET_KEYS` in `src/lib/tauri/commands.ts`:
//! `provider:<id>:api_key`, `research:exa:api_key`, `research:firecrawl:api_key`,
//! `agent:anthropic:api_key`, `auth:clerk:client_token`.

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

    fn validate_key(key: &str) -> BlueyResult<()> {
        let allowed = matches!(
            key,
            EXA_KEY
                | FIRECRAWL_KEY
                | AGENT_ANTHROPIC_KEY
                | CLERK_TOKEN_KEY
                | CLERK_OAUTH_TOKENS_KEY
        ) || (key.starts_with("provider:")
            && key.ends_with(":api_key")
            && key.len() > "provider::api_key".len());
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

/// Read `KEY=value` pairs from a `.env` file next to the executable or the
/// current directory into the process environment (existing variables win).
/// Values are never logged.
pub fn load_dotenv() {
    let mut candidates: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from(".env")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(".env"));
        }
    }
    for path in candidates {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !key.is_empty() && std::env::var_os(key).is_none() {
                std::env::set_var(key, value);
            }
        }
        tracing::info!(path = %path.display(), "loaded .env file");
        break;
    }
}
