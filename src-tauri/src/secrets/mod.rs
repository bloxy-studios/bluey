//! Secrets in the macOS Keychain (service `com.codewithabdul.bluey`, `….dev`
//! for development builds), behind [`backend::SecretBackend`] and an
//! in-process cache (ADR 0011). Values never appear in logs or errors.
//!
//! Two allow-lists, two layers:
//! * the **store** ([`SecretsStore`]) accepts every key Bluey owns —
//!   `provider:<id>:api_key`, the research / agent keys and the Rust-only
//!   sign-in tokens (`auth:clerk:*`);
//! * the **WebView** ([`validate_webview_key`], in front of `secrets_set` /
//!   `secrets_has` / `secrets_delete`) may touch only the keys of
//!   `SECRET_KEYS` in `src/lib/tauri/commands.ts` — API keys entered in
//!   Settings. Sign-in and (ADR 0009) subscription-account tokens never cross
//!   that boundary in either direction — except that `secrets_allow_access`
//!   ([`health::may_allow_access`]) may name them to trigger the one
//!   deliberate read of a locked item; it returns a state, never a value.

pub mod backend;
pub mod health;
pub mod keychain;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bluey_core::{BlueyError, BlueyResult};
use parking_lot::Mutex;
use zeroize::Zeroizing;

use self::backend::{KeychainStatus, SecretBackend};
use self::keychain::KeychainBackend;
#[cfg(not(debug_assertions))]
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

/// The Keychain service of Bluey's items. Development builds use their own
/// service so a `tauri dev` binary (a new code identity on every rebuild)
/// never touches — or re-owns — the installed app's items (ADR 0011).
#[cfg(debug_assertions)]
pub const SERVICE: &str = "com.codewithabdul.bluey.dev";
#[cfg(not(debug_assertions))]
pub const SERVICE: &str = BUNDLE_ID;

/// The kind of a Bluey-owned key — what diagnostics and the credential-health
/// list may name instead of the key itself.
pub fn key_category(key: &str) -> &'static str {
    match key {
        EXA_KEY | FIRECRAWL_KEY => "research_key",
        AGENT_ANTHROPIC_KEY => "agent_key",
        CLERK_TOKEN_KEY | CLERK_OAUTH_TOKENS_KEY => "sign_in",
        key if is_provider_api_key(key) => "provider_key",
        key if is_account_tokens_key(key) => "account_tokens",
        _ => "unknown",
    }
}

/// Whether a saved secret can be used, as far as this build can tell without
/// asking the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretState {
    /// Saved, and this build can read it (or already has).
    Present,
    /// Saved, but macOS wants the user's approval before this build reads it
    /// (an update or rebuild changed Bluey's code identity, or access was
    /// denied).
    Locked,
    /// Not saved.
    Absent,
}

/// What this process knows about the service's items. Values are the ones it
/// read or wrote itself; errors are never recorded.
#[derive(Default)]
struct Cache {
    values: HashMap<String, Zeroizing<String>>,
    /// Attribute-only knowledge of whether an item exists.
    presence: HashMap<String, bool>,
    /// `presence` covers every item (an enumeration ran): a key missing from
    /// it is absent.
    complete: bool,
}

impl Cache {
    fn presence(&self, key: &str) -> Option<bool> {
        if self.values.contains_key(key) {
            return Some(true);
        }
        match self.presence.get(key) {
            Some(present) => Some(*present),
            None if self.complete => Some(false),
            None => None,
        }
    }

    fn store(&mut self, key: &str, value: Zeroizing<String>) {
        self.presence.insert(key.to_string(), true);
        self.values.insert(key.to_string(), value);
    }

    fn mark(&mut self, key: &str, present: bool) {
        if !present {
            self.values.remove(key);
        }
        self.presence.insert(key.to_string(), present);
    }

    /// Forget `key` after a failed write or delete left its state unknown.
    fn forget(&mut self, key: &str) {
        self.values.remove(key);
        self.presence.remove(key);
        self.complete = false;
    }
}

/// Keychain-backed secret store with an in-process cache: each item is
/// decrypted at most once per process (a prompt at most once after an
/// update), presence comes from attribute-only lookups, and writes replace
/// the item only when the value changed.
pub struct SecretsStore {
    backend: Arc<dyn SecretBackend>,
    /// Serializes backend calls: concurrent misses of one key cost one read,
    /// and the non-interactive probe (which disables Keychain UI process-wide)
    /// never overlaps another call.
    io: Mutex<()>,
    cache: Mutex<Cache>,
}

impl Default for SecretsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretsStore {
    pub fn new() -> Self {
        // Unit tests take a fake backend (`with_backend`): the login keychain
        // would make their results depend on the machine's saved items.
        if cfg!(test) {
            panic!("unit tests must not use the real Keychain");
        }
        Self::with_backend(Arc::new(KeychainBackend::new(SERVICE)))
    }

    pub fn with_backend(backend: Arc<dyn SecretBackend>) -> Self {
        Self {
            backend,
            io: Mutex::new(()),
            cache: Mutex::new(Cache::default()),
        }
    }

    /// The storage-level allow-list: everything Bluey owns, including the
    /// Rust-only sign-in tokens the WebView must never reach.
    fn validate_key(key: &str) -> BlueyResult<()> {
        if Self::is_owned_key(key) {
            Ok(())
        } else {
            Err(BlueyError::invalid_params("unknown secret key"))
        }
    }

    fn is_owned_key(key: &str) -> bool {
        matches!(
            key,
            EXA_KEY
                | FIRECRAWL_KEY
                | AGENT_ANTHROPIC_KEY
                | CLERK_TOKEN_KEY
                | CLERK_OAUTH_TOKENS_KEY
        ) || is_provider_api_key(key)
            || is_account_tokens_key(key)
    }

    /// Log a failed backend call (category, operation, `OSStatus` — never the
    /// key or a value) and turn it into the user-facing error.
    fn failure(op: &str, key: &str, status: KeychainStatus) -> BlueyError {
        let category = key_category(key);
        tracing::warn!(category, op, status = status.0, "keychain request failed");
        status.into_error(op, category)
    }

    /// Enumerate the service's items once (attributes only, no prompt) so every
    /// presence question — `has_api_key`, the stored sign-in, research
    /// availability — is answered from memory. Boot calls this first.
    pub fn preload_presence(&self) -> BlueyResult<()> {
        let _io = self.io.lock();
        let accounts = self
            .backend
            .list()
            .map_err(|status| Self::failure("list", "*", status))?;
        let mut cache = self.cache.lock();
        for account in accounts {
            cache.presence.insert(account, true);
        }
        cache.complete = true;
        Ok(())
    }

    /// The cached value, if this process already read or wrote it.
    fn cached(&self, key: &str) -> Option<String> {
        self.cache
            .lock()
            .values
            .get(key)
            .map(|value| value.as_str().to_string())
    }

    /// Whether `value` is the cached value of `key` (no plain copy is made).
    fn is_cached_value(&self, key: &str, value: &str) -> bool {
        self.cache
            .lock()
            .values
            .get(key)
            .is_some_and(|cached| cached.as_str() == value)
    }

    /// The state memory alone can tell: `Present` once read or written,
    /// `Absent` when known missing.
    fn cached_state(&self, key: &str) -> Option<SecretState> {
        let cache = self.cache.lock();
        if cache.values.contains_key(key) {
            Some(SecretState::Present)
        } else if cache.presence(key) == Some(false) {
            Some(SecretState::Absent)
        } else {
            None
        }
    }

    /// Read a secret, decrypting the item at most once per process (the only
    /// call that can show the Keychain prompt). Blocking.
    pub fn get_sync(&self, key: &str) -> BlueyResult<Option<String>> {
        Self::validate_key(key)?;
        {
            let cache = self.cache.lock();
            if let Some(value) = cache.values.get(key) {
                return Ok(Some(value.as_str().to_string()));
            }
            if cache.presence(key) == Some(false) {
                return Ok(None);
            }
        }
        let _io = self.io.lock();
        // Another caller may have read it while this one waited.
        if let Some(value) = self.cached(key) {
            return Ok(Some(value));
        }
        match self.backend.read(key) {
            Ok(Some(value)) => {
                let plain = value.as_str().to_string();
                self.cache.lock().store(key, value);
                Ok(Some(plain))
            }
            Ok(None) => {
                self.cache.lock().mark(key, false);
                Ok(None)
            }
            Err(status) => Err(Self::failure("read", key, status)),
        }
    }

    /// Whether a secret is saved — from memory, or an attribute-only lookup
    /// that never prompts. A saved item this build may not read yet still
    /// counts as saved. Blocking only on a cache miss.
    pub fn has_sync(&self, key: &str) -> BlueyResult<bool> {
        Self::validate_key(key)?;
        if let Some(present) = self.cache.lock().presence(key) {
            return Ok(present);
        }
        let _io = self.io.lock();
        if let Some(present) = self.cache.lock().presence(key) {
            return Ok(present);
        }
        let present = self
            .backend
            .exists(key)
            .map_err(|status| Self::failure("exists", key, status))?;
        self.cache.lock().mark(key, present);
        Ok(present)
    }

    /// The value of `key` if this process already read or wrote it — never a
    /// Keychain call (best-effort work such as revoking tokens on sign-out
    /// must not prompt).
    pub fn peek(&self, key: &str) -> Option<String> {
        self.cached(key)
    }

    /// The cached presence of `key`, without touching the Keychain.
    pub fn known_presence(&self, key: &str) -> Option<bool> {
        self.cache.lock().presence(key)
    }

    /// Store a secret. A value equal to the cached one is not rewritten;
    /// otherwise the item is replaced (attribute-only delete, then add), so
    /// the running build owns it and an untrusted in-place modify can never
    /// lock it. Blocking.
    pub fn set_sync(&self, key: &str, value: &str) -> BlueyResult<()> {
        Self::validate_key(key)?;
        if value.is_empty() {
            return Err(BlueyError::invalid_params("secret value must not be empty"));
        }
        let _io = self.io.lock();
        if self.is_cached_value(key, value) {
            return Ok(());
        }
        let result = self
            .backend
            .remove(key)
            .and_then(|_| self.backend.add(key, value));
        let mut cache = self.cache.lock();
        match result {
            Ok(()) => {
                cache.store(key, Zeroizing::new(value.to_string()));
                Ok(())
            }
            Err(status) => {
                cache.forget(key);
                drop(cache);
                Err(Self::failure("write", key, status))
            }
        }
    }

    /// Delete a secret — attribute-only, never prompts; the status is checked.
    /// Missing items are not an error. Blocking.
    pub fn delete_sync(&self, key: &str) -> BlueyResult<()> {
        Self::validate_key(key)?;
        let _io = self.io.lock();
        match self.backend.remove(key) {
            Ok(_) => {
                self.cache.lock().mark(key, false);
                Ok(())
            }
            Err(status) => {
                self.cache.lock().forget(key);
                Err(Self::failure("delete", key, status))
            }
        }
    }

    /// Delete every Bluey-owned item of the service — including keys of
    /// providers that no longer exist. Returns the keys that could not be
    /// deleted, each with its error. Blocking.
    pub fn delete_all_sync(&self) -> BlueyResult<Vec<(String, BlueyError)>> {
        let _io = self.io.lock();
        let accounts = self
            .backend
            .list()
            .map_err(|status| Self::failure("list", "*", status))?;
        let mut failures = Vec::new();
        let mut cache = Cache {
            complete: true,
            ..Cache::default()
        };
        for account in accounts.into_iter().filter(|a| Self::is_owned_key(a)) {
            if let Err(status) = self.backend.remove(&account) {
                failures.push((account.clone(), Self::failure("delete", &account, status)));
                cache.presence.insert(account, true);
            }
        }
        *self.cache.lock() = cache;
        Ok(failures)
    }

    /// The state of `key` without asking the user: from memory, or a read with
    /// Keychain UI disabled (a silent read fills the cache). Blocking.
    pub fn state_sync(&self, key: &str) -> BlueyResult<SecretState> {
        Self::validate_key(key)?;
        if let Some(state) = self.cached_state(key) {
            return Ok(state);
        }
        let _io = self.io.lock();
        if let Some(state) = self.cached_state(key) {
            return Ok(state);
        }
        match self.backend.probe(key) {
            Ok(Some(value)) => {
                self.cache.lock().store(key, value);
                Ok(SecretState::Present)
            }
            Ok(None) => {
                self.cache.lock().mark(key, false);
                Ok(SecretState::Absent)
            }
            Err(status) if status.is_locked() => {
                self.cache.lock().mark(key, true);
                Ok(SecretState::Locked)
            }
            Err(status) => Err(Self::failure("probe", key, status)),
        }
    }

    /// Every saved Bluey-owned key with its [`SecretState`] (attribute-only
    /// enumeration, then [`Self::state_sync`] per key). Never prompts.
    pub fn states_sync(&self) -> BlueyResult<Vec<(String, SecretState)>> {
        self.preload_presence()?;
        let mut keys: Vec<String> = {
            let cache = self.cache.lock();
            cache
                .presence
                .iter()
                .filter(|(key, present)| **present && Self::is_owned_key(key))
                .map(|(key, _)| key.clone())
                .collect()
        };
        keys.sort();
        keys.into_iter()
            .map(|key| self.state_sync(&key).map(|state| (key, state)))
            .collect()
    }

    /// Async wrappers — Keychain access can block (a prompt waits for the
    /// user), so hop to the blocking pool unless memory answers.
    pub async fn get(self: &Arc<Self>, key: &str) -> BlueyResult<Option<String>> {
        if let Some(value) = self.cached(key) {
            return Ok(Some(value));
        }
        self.blocking(key, |this, key| this.get_sync(key)).await
    }

    pub async fn has(self: &Arc<Self>, key: &str) -> BlueyResult<bool> {
        if let Some(present) = self.known_presence(key) {
            Self::validate_key(key)?;
            return Ok(present);
        }
        self.blocking(key, |this, key| this.has_sync(key)).await
    }

    pub async fn set(self: &Arc<Self>, key: &str, value: String) -> BlueyResult<()> {
        let value = Zeroizing::new(value);
        self.blocking(key, move |this, key| this.set_sync(key, &value))
            .await
    }

    pub async fn delete(self: &Arc<Self>, key: &str) -> BlueyResult<()> {
        self.blocking(key, |this, key| this.delete_sync(key)).await
    }

    pub async fn state(self: &Arc<Self>, key: &str) -> BlueyResult<SecretState> {
        self.blocking(key, |this, key| this.state_sync(key)).await
    }

    pub async fn states(self: &Arc<Self>) -> BlueyResult<Vec<(String, SecretState)>> {
        self.blocking("", |this, _| this.states_sync()).await
    }

    pub async fn delete_all(self: &Arc<Self>) -> BlueyResult<Vec<(String, BlueyError)>> {
        self.blocking("", |this, _| this.delete_all_sync()).await
    }

    async fn blocking<T, F>(self: &Arc<Self>, key: &str, op: F) -> BlueyResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Self, &str) -> BlueyResult<T> + Send + 'static,
    {
        let this = self.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || op(&this, &key))
            .await
            .map_err(|e| BlueyError::internal(format!("keychain task panicked: {e}")))?
    }
}

/// Load `.env.local` and then `.env` into the process environment. Variables
/// that are already set win, and an earlier file wins over a later one (so
/// `.env.local` overrides `.env`, as with Vite and Bun). Files read, in order:
/// an explicit `BLUEY_ENV_FILE`; then, for development builds only, the
/// repository root and `src-tauri` — the Tauri CLI runs the app from
/// `src-tauri`, so a plain relative `.env` would miss the repository's files —
/// the current directory and the directory of the executable. A release build
/// never picks up an `.env` from wherever it was launched (SEC-015).
///
/// Values are never logged. The loaded paths are returned so the caller can log
/// them once logging is up (this runs first: `BLUEY_LOG_LEVEL` may live here).
pub fn load_dotenv() -> Vec<PathBuf> {
    let explicit = std::env::var_os("BLUEY_ENV_FILE").map(PathBuf::from);
    let dirs = dotenv_dirs(cfg!(debug_assertions));
    let files = dotenv_files(explicit, &dirs);

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut loaded = Vec::new();
    for path in files {
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
    loaded
}

/// Where a development build looks for `.env` files; a release build looks
/// nowhere (only `BLUEY_ENV_FILE`).
fn dotenv_dirs(development: bool) -> Vec<PathBuf> {
    if !development {
        return Vec::new();
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    manifest
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .chain([manifest.to_path_buf(), PathBuf::from(".")])
        .chain(exe_dir)
        .collect()
}

/// The env files [`load_dotenv`] tries, in priority order: the explicit file,
/// then `.env.local` and `.env` of each search directory.
fn dotenv_files(explicit: Option<PathBuf>, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let searched = dirs
        .iter()
        .flat_map(|dir| [dir.join(".env.local"), dir.join(".env")]);
    explicit.into_iter().chain(searched).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(debug_assertions)]
    fn debug_builds_never_share_the_installed_apps_keychain_service() {
        use crate::storage::BUNDLE_ID;
        assert_ne!(SERVICE, BUNDLE_ID);
        assert!(SERVICE.starts_with(BUNDLE_ID));
    }

    #[test]
    fn release_builds_read_only_an_explicit_env_file() {
        assert!(dotenv_dirs(false).is_empty(), "no cwd / exe-dir .env");
        let explicit = PathBuf::from("/tmp/bluey.env");
        assert_eq!(dotenv_files(Some(explicit.clone()), &[]), vec![explicit]);
        assert!(dotenv_dirs(true).contains(&PathBuf::from(".")));
        assert_eq!(
            dotenv_files(None, &[PathBuf::from("d")]),
            vec![PathBuf::from("d/.env.local"), PathBuf::from("d/.env")]
        );
    }

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

    use self::backend::fake::{CountingFake, Op};
    use self::backend::{ERR_AUTH_FAILED, ERR_INTERACTION_NOT_ALLOWED};

    const GEMINI: &str = "provider:gemini:api_key";

    fn store(items: &[(&str, &str)]) -> (Arc<CountingFake>, SecretsStore) {
        let fake = Arc::new(CountingFake::with_items(items));
        (fake.clone(), SecretsStore::with_backend(fake))
    }

    #[test]
    fn boot_presence_is_one_enumeration_and_no_reads() {
        let (fake, store) = store(&[(GEMINI, "g"), (CLERK_OAUTH_TOKENS_KEY, "{}")]);
        store.preload_presence().unwrap();
        assert!(store.has_sync(GEMINI).unwrap());
        assert!(store.has_sync(CLERK_OAUTH_TOKENS_KEY).unwrap());
        assert!(!store.has_sync(EXA_KEY).unwrap());
        assert_eq!(fake.count(Op::List), 1);
        assert_eq!(fake.reads(), 0);
        assert_eq!(fake.count(Op::Exists), 0);
    }

    #[test]
    fn a_key_is_decrypted_at_most_once_per_process() {
        let (fake, store) = store(&[(GEMINI, "g")]);
        for _ in 0..5 {
            assert_eq!(store.get_sync(GEMINI).unwrap().as_deref(), Some("g"));
        }
        assert_eq!(fake.count_for(Op::Read, GEMINI), 1);
    }

    #[test]
    fn a_known_missing_key_costs_no_read() {
        let (fake, store) = store(&[]);
        store.preload_presence().unwrap();
        assert_eq!(store.get_sync(EXA_KEY).unwrap(), None);
        assert_eq!(fake.reads(), 0);
    }

    #[test]
    fn set_replaces_by_delete_then_add_and_skips_unchanged_values() {
        let (fake, store) = store(&[(GEMINI, "old")]);
        store.set_sync(GEMINI, "new").unwrap();
        assert_eq!(fake.count(Op::Remove), 1);
        assert_eq!(fake.writes(), 1);
        assert_eq!(fake.value(GEMINI).as_deref(), Some("new"));
        store.set_sync(GEMINI, "new").unwrap();
        assert_eq!(fake.writes(), 1, "an unchanged value is not rewritten");
        assert_eq!(store.get_sync(GEMINI).unwrap().as_deref(), Some("new"));
        assert_eq!(fake.reads(), 0, "a write fills the cache");
    }

    #[test]
    fn a_locked_item_is_saved_not_missing() {
        let (fake, store) = store(&[(GEMINI, "g"), (EXA_KEY, "e")]);
        fake.lock_item(GEMINI, ERR_INTERACTION_NOT_ALLOWED);
        fake.lock_item(EXA_KEY, ERR_AUTH_FAILED);
        store.preload_presence().unwrap();
        assert!(store.has_sync(GEMINI).unwrap());
        assert_eq!(store.state_sync(GEMINI).unwrap(), SecretState::Locked);
        assert_eq!(
            store.state_sync(FIRECRAWL_KEY).unwrap(),
            SecretState::Absent
        );
        let denied = store.get_sync(EXA_KEY).unwrap_err();
        assert_eq!(denied.code, "storage.keychain_access_denied");
        let blocked = store.get_sync(GEMINI).unwrap_err();
        assert_eq!(blocked.code, "storage.keychain_interaction_not_allowed");
        assert_eq!(fake.reads(), 2);
        assert_eq!(fake.count(Op::Probe), 1, "the probe never prompts");
        // Errors are not cached: once allowed, the next read succeeds.
        fake.unlock(EXA_KEY);
        assert_eq!(store.get_sync(EXA_KEY).unwrap().as_deref(), Some("e"));
        assert_eq!(store.state_sync(EXA_KEY).unwrap(), SecretState::Present);
    }

    #[test]
    fn delete_is_attribute_only_and_invalidates_the_cache() {
        let (fake, store) = store(&[(GEMINI, "g")]);
        assert_eq!(store.get_sync(GEMINI).unwrap().as_deref(), Some("g"));
        fake.reset_counts();
        store.delete_sync(GEMINI).unwrap();
        assert!(!store.has_sync(GEMINI).unwrap());
        assert_eq!(store.get_sync(GEMINI).unwrap(), None);
        assert_eq!(fake.reads(), 0);
        assert_eq!(fake.count(Op::Remove), 1);
        // Deleting a missing key is not an error.
        store.delete_sync(GEMINI).unwrap();
    }

    #[test]
    fn a_failed_delete_is_reported_and_forgets_the_cached_value() {
        let (fake, store) = store(&[(GEMINI, "g")]);
        store.get_sync(GEMINI).unwrap();
        fake.fail_remove(GEMINI, ERR_AUTH_FAILED);
        let error = store.delete_sync(GEMINI).unwrap_err();
        assert_eq!(error.code, "storage.keychain_access_denied");
        assert_eq!(store.known_presence(GEMINI), None);
    }

    #[test]
    fn delete_all_removes_every_owned_item_of_the_service() {
        let orphan = "provider:removed-provider:api_key";
        let (fake, store) = store(&[
            (GEMINI, "g"),
            (orphan, "o"),
            (CLERK_OAUTH_TOKENS_KEY, "{}"),
            (&account_tokens_key("claude-1"), "{}"),
            ("not-ours", "x"),
        ]);
        store.get_sync(GEMINI).unwrap();
        fake.reset_counts();
        let failures = store.delete_all_sync().unwrap();
        assert!(failures.is_empty());
        assert_eq!(fake.accounts(), HashSet::from(["not-ours".to_string()]));
        assert_eq!(fake.reads(), 0);
        assert_eq!(store.get_sync(GEMINI).unwrap(), None);
        assert_eq!(fake.reads(), 0, "the reset left nothing cached");
    }

    #[test]
    fn credential_health_probes_without_prompting_and_lists_owned_items_only() {
        let (fake, store) = store(&[
            (GEMINI, "g"),
            (EXA_KEY, "e"),
            (&account_tokens_key("claude"), "{}"),
            ("not-ours", "x"),
        ]);
        fake.lock_item(EXA_KEY, ERR_AUTH_FAILED);
        let states = store.states_sync().unwrap();
        assert_eq!(
            states,
            vec![
                (account_tokens_key("claude"), SecretState::Present),
                (GEMINI.to_string(), SecretState::Present),
                (EXA_KEY.to_string(), SecretState::Locked),
            ]
        );
        assert_eq!(fake.reads(), 0, "health never shows the Keychain prompt");
        assert_eq!(fake.count(Op::Probe), 3);
        // A second look re-probes only the locked item (the user may have
        // allowed access meanwhile); the rest is answered from memory.
        store.states_sync().unwrap();
        assert_eq!(fake.count(Op::Probe), 4);
        assert_eq!(fake.reads(), 0);
    }
}
