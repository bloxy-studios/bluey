//! Provider accounts (ADR 0009): the owner's AI subscriptions as credential
//! sources next to API keys.
//!
//! [`AccountsManager`] owns everything a provider does not: the list of
//! accounts and their status (SQLite settings table, `accounts:list`), the
//! cached model catalogs (`accounts:catalog:<id>`), the tokens (Keychain
//! `account:<id>:oauth_tokens`, read and written **only** here), a
//! single-flight refresh per account, the pending sign-in flows, and the
//! `accounts.changed` / `accounts.catalog` events. What a provider *is* — its
//! OAuth flow, catalog, refresh, revoke, probe — is a [`profile::ProviderProfile`];
//! this PR ships placeholders for the three reserved ids, PR 3a–3c replace them.
//!
//! Two switches: the Cargo feature `subscription-accounts` (compile the
//! feature out entirely: every account shows as unavailable) and
//! `settings.experimental.subscriptionAccounts` (hide the section and refuse
//! new sign-ins without a rebuild). Nothing here logs a token, ever.

#[cfg(feature = "subscription-accounts")]
pub mod chatgpt;
#[cfg(feature = "subscription-accounts")]
pub mod claude;
pub mod profile;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use bluey_core::accounts::{self as rules, codes};
use bluey_core::error::RecoveryAction;
use bluey_core::events::BlueyEvent;
use bluey_core::types::{
    AccountConnectOptions, AccountIdentity, AccountStatus, AiProviderConfig, ConnectFlowKind,
    FingerprintProbe, ModelRole, ProviderAccount, ProviderAuthMethod, ProviderModelCatalog,
    Settings, UnavailableReason,
};
use bluey_core::{now_iso, BlueyError, BlueyErrorKind, BlueyResult};
use bluey_oauth::{unix_now, RefreshError, TokenCache, TokenSet, DEFAULT_REFRESH_LEEWAY};
use bluey_storage::SettingsRepository;
use chrono::Utc;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;

use crate::events::EventBus;
use crate::secrets::{account_tokens_key, SecretsStore};
use crate::settings::SettingsManager;
use crate::state::AppCore;
use crate::storage::Storage;
use profile::{Connected, ProviderProfile};

/// Settings-table key of the account list (statuses, identities — no tokens).
const ACCOUNTS_KEY: &str = "accounts:list";
/// Settings-table key of the stable per-install device id (64 hex; not a secret —
/// it identifies this install in `metadata.user_id`-style fields).
const DEVICE_ID_KEY: &str = "accounts:device_id";

static DEVICE_ID: OnceLock<String> = OnceLock::new();

/// The install's device id — the one `AccountsManager::load` persisted, or a
/// process-local one when the manager has not loaded (tests).
pub fn process_device_id() -> String {
    DEVICE_ID
        .get_or_init(|| hex(&bluey_oauth::random_bytes::<32>()))
        .clone()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn load_or_create_device_id(storage: &Storage) -> BlueyResult<String> {
    let stored = storage
        .run_sync(|db| SettingsRepository::get_json(db, DEVICE_ID_KEY))?
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|id| id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()));
    if let Some(id) = stored {
        return Ok(id);
    }
    let fresh = hex(&bluey_oauth::random_bytes::<32>());
    let value = serde_json::Value::String(fresh.clone());
    storage.run_sync(move |db| SettingsRepository::set_json(db, DEVICE_ID_KEY, &value))?;
    Ok(fresh)
}

fn catalog_key(account_id: &str) -> String {
    format!("accounts:catalog:{account_id}")
}

/// A sign-in in flight: how to abandon it and, for manual-code flows, where
/// the pasted code goes.
struct PendingConnect {
    cancel: CancellationToken,
    manual_code: Option<tokio::sync::oneshot::Sender<String>>,
}

pub struct AccountsManager {
    secrets: Arc<SecretsStore>,
    storage: Arc<Storage>,
    bus: Arc<EventBus>,
    settings: Arc<SettingsManager>,
    http: reqwest::Client,
    profiles: Vec<Arc<dyn ProviderProfile>>,
    accounts: parking_lot::RwLock<Vec<ProviderAccount>>,
    catalogs: parking_lot::RwLock<HashMap<String, ProviderModelCatalog>>,
    tokens: parking_lot::Mutex<HashMap<String, Arc<TokenCache>>>,
    pending: parking_lot::Mutex<HashMap<String, PendingConnect>>,
    device_id: String,
}

impl AccountsManager {
    /// Load the persisted accounts and catalogs (bootstrap, synchronous). A
    /// `Connecting` status does not survive a restart; every profile gets a
    /// card even when nothing is stored yet.
    pub fn load(
        secrets: Arc<SecretsStore>,
        storage: Arc<Storage>,
        bus: Arc<EventBus>,
        settings: Arc<SettingsManager>,
        http: reqwest::Client,
    ) -> BlueyResult<Self> {
        let profiles = profile::profiles();
        let mut accounts: Vec<ProviderAccount> = storage
            .run_sync(|db| SettingsRepository::get_json(db, ACCOUNTS_KEY))?
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        for account in &mut accounts {
            rules::normalise_after_restart(account);
        }
        for profile in &profiles {
            match accounts
                .iter_mut()
                .find(|account| account.provider_id == profile.provider_id())
            {
                Some(account) => Self::stamp_fingerprint(account, profile.as_ref()),
                None => accounts.push(Self::blank_account(profile.as_ref())),
            }
        }
        let mut catalogs = HashMap::new();
        for account in &accounts {
            let key = catalog_key(&account.account_id);
            if let Some(catalog) = storage
                .run_sync(|db| SettingsRepository::get_json(db, &key))?
                .and_then(|value| serde_json::from_value::<ProviderModelCatalog>(value).ok())
            {
                catalogs.insert(account.account_id.clone(), catalog);
            }
        }
        let device_id = load_or_create_device_id(&storage)?;
        let _ = DEVICE_ID.set(device_id.clone());
        Ok(Self {
            secrets,
            storage,
            bus,
            settings,
            http,
            profiles,
            accounts: parking_lot::RwLock::new(accounts),
            catalogs: parking_lot::RwLock::new(catalogs),
            tokens: parking_lot::Mutex::new(HashMap::new()),
            pending: parking_lot::Mutex::new(HashMap::new()),
            device_id,
        })
    }

    /// Stable per-install device id (64 hex) for the shapers' `metadata.user_id` fields.
    pub fn device_id(&self) -> String {
        self.device_id.clone()
    }

    /// Whether this build includes subscription accounts at all.
    pub fn build_enabled() -> bool {
        cfg!(feature = "subscription-accounts")
    }

    /// Build feature **and** the runtime flag.
    pub fn enabled(&self) -> bool {
        Self::build_enabled() && self.settings.get().experimental.subscription_accounts
    }

    fn ensure_enabled(&self) -> BlueyResult<()> {
        if !Self::build_enabled() {
            return Err(BlueyError::account(
                "disabled",
                "this build of Bluey was made without subscription accounts",
            )
            .recoverable(RecoveryAction::UseApiKey));
        }
        if !self.settings.get().experimental.subscription_accounts {
            return Err(BlueyError::account(
                "disabled",
                "subscription accounts are switched off in Settings → AI",
            )
            .recoverable(RecoveryAction::OpenSettings { tab: "ai".into() }));
        }
        Ok(())
    }

    fn blank_account(profile: &dyn ProviderProfile) -> ProviderAccount {
        let mut account = ProviderAccount {
            account_id: profile.provider_id().to_string(),
            provider_id: profile.provider_id().to_string(),
            kind: profile.kind(),
            method: ProviderAuthMethod::OauthSubscription,
            status: AccountStatus::Disconnected,
            identity: None,
            connected_at: None,
            expires_at: None,
            catalog_fetched_at: None,
            fingerprint_version: None,
            fingerprint_captured_on: None,
        };
        Self::stamp_fingerprint(&mut account, profile);
        account
    }

    /// The fingerprint columns come from the build, not from storage.
    fn stamp_fingerprint(account: &mut ProviderAccount, profile: &dyn ProviderProfile) {
        let info = profile.fingerprint();
        account.fingerprint_version = info.map(|i| i.version.to_string());
        account.fingerprint_captured_on = info.map(|i| i.captured_on.to_string());
        account.kind = profile.kind();
    }

    /// Every account, in UI order. A build without the feature reports each
    /// one as unavailable so the UI can say why.
    pub fn list(&self) -> Vec<ProviderAccount> {
        let mut accounts = self.accounts.read().clone();
        if !Self::build_enabled() {
            for account in &mut accounts {
                account.status = AccountStatus::Unavailable {
                    reason: UnavailableReason::Disabled,
                    detail: Some("built without subscription accounts".into()),
                };
            }
        }
        accounts
    }

    pub fn status(&self, account_id: &str) -> BlueyResult<ProviderAccount> {
        self.list()
            .into_iter()
            .find(|account| account.account_id == account_id)
            .ok_or_else(|| not_found(account_id))
    }

    fn stored(&self, account_id: &str) -> BlueyResult<ProviderAccount> {
        self.accounts
            .read()
            .iter()
            .find(|account| account.account_id == account_id)
            .cloned()
            .ok_or_else(|| not_found(account_id))
    }

    fn profile_for(&self, provider_id: &str) -> BlueyResult<Arc<dyn ProviderProfile>> {
        self.profiles
            .iter()
            .find(|profile| profile.provider_id() == provider_id)
            .cloned()
            .ok_or_else(|| {
                BlueyError::account(
                    "unknown_provider",
                    format!("`{provider_id}` is not a subscription provider"),
                )
            })
    }

    /// The cached catalog of an account, if one was fetched.
    pub fn catalog(&self, account_id: &str) -> Option<ProviderModelCatalog> {
        self.catalogs.read().get(account_id).cloned()
    }

    /// Store an account, persist the list and publish `accounts.changed`.
    async fn set_account(&self, account: ProviderAccount) -> BlueyResult<ProviderAccount> {
        let snapshot = {
            let mut accounts = self.accounts.write();
            rules::upsert(&mut accounts, account.clone());
            accounts.clone()
        };
        let value = serde_json::to_value(&snapshot)
            .map_err(|_| BlueyError::internal("cannot serialise the account list"))?;
        self.storage
            .run(move |db| SettingsRepository::set_json(db, ACCOUNTS_KEY, &value))
            .await?;
        self.bus
            .publish(BlueyEvent::AccountsChanged(account.clone()));
        Ok(account)
    }

    async fn store_catalog(&self, catalog: ProviderModelCatalog) -> BlueyResult<()> {
        let key = catalog_key(&catalog.account_id);
        let value = serde_json::to_value(&catalog)
            .map_err(|_| BlueyError::internal("cannot serialise the model catalog"))?;
        self.storage
            .run(move |db| SettingsRepository::set_json(db, &key, &value))
            .await?;
        self.catalogs
            .write()
            .insert(catalog.account_id.clone(), catalog.clone());
        self.bus.publish(BlueyEvent::AccountsCatalog(catalog));
        Ok(())
    }

    async fn drop_catalog(&self, account_id: &str) -> BlueyResult<()> {
        self.catalogs.write().remove(account_id);
        let key = catalog_key(account_id);
        self.storage
            .run(move |db| SettingsRepository::set_json(db, &key, &serde_json::Value::Null))
            .await
    }

    fn cancel_pending(&self, account_id: &str) -> bool {
        match self.pending.lock().remove(account_id) {
            Some(pending) => {
                pending.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Start a sign-in with `provider_id`. Returns the account in `Connecting`;
    /// completion arrives as `accounts.changed` (and `accounts.catalog`).
    pub async fn connect(
        &self,
        app: &AppHandle,
        provider_id: &str,
        options: AccountConnectOptions,
    ) -> BlueyResult<ProviderAccount> {
        self.ensure_enabled()?;
        let profile = self.profile_for(provider_id)?;
        // One account per provider until PR 3c adds Google multi-account.
        let account_id = provider_id.to_string();
        self.cancel_pending(&account_id);
        let mut account = self.stored(&account_id)?;
        let start = match profile.begin_connect(&self.http, &options).await {
            Ok(start) => start,
            Err(error) => {
                if let Some(status) = rules::status_after_error(&error) {
                    account.status = status;
                    self.set_account(account).await?;
                }
                return Err(error);
            }
        };
        // Profiles have no window handle: the browser (or the device-code page) opens here.
        let to_open = match start.flow.kind {
            ConnectFlowKind::Browser | ConnectFlowKind::ManualCode => start.flow.url.clone(),
            ConnectFlowKind::DeviceCode => start
                .flow
                .verification_url
                .clone()
                .or_else(|| start.flow.url.clone()),
        };
        if let Some(url) = to_open {
            if let Err(error) = app.opener().open_url(&url, None::<&str>) {
                start.cancel.cancel();
                return Err(BlueyError::account(
                    "browser_open_failed",
                    format!("could not open the browser: {error}"),
                )
                .recoverable(RecoveryAction::Retry));
            }
        }
        account.status = AccountStatus::Connecting {
            flow: start.flow.clone(),
        };
        self.pending.lock().insert(
            account_id.clone(),
            PendingConnect {
                cancel: start.cancel.clone(),
                manual_code: start.manual_code,
            },
        );
        let account = self.set_account(account).await?;
        let cancel = start.cancel;
        let completion = start.completion;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let outcome = tokio::select! {
                _ = cancel.cancelled() => return,
                outcome = completion => outcome,
            };
            app.state::<AppCore>()
                .accounts
                .finish_connect(&account_id, outcome)
                .await;
        });
        tracing::info!(provider = provider_id, "subscription sign-in started");
        Ok(account)
    }

    /// Import the official client's local sign-in (read-only) for `provider_id`.
    pub async fn import(&self, provider_id: &str) -> BlueyResult<ProviderAccount> {
        self.ensure_enabled()?;
        let profile = self.profile_for(provider_id)?;
        let account_id = provider_id.to_string();
        self.cancel_pending(&account_id);
        let outcome = profile.import(&self.http).await;
        let failed = outcome.as_ref().err().cloned();
        self.finish_connect(&account_id, outcome).await;
        match failed {
            Some(error) => Err(error),
            None => self.stored(&account_id),
        }
    }

    /// A sign-in completed (or failed): keep the tokens, resolve the catalog,
    /// publish; a failure moves the account to the matching status and
    /// surfaces through the error toast channel.
    async fn finish_connect(&self, account_id: &str, outcome: BlueyResult<Connected>) {
        self.pending.lock().remove(account_id);
        let Ok(mut account) = self.stored(account_id) else {
            return;
        };
        match outcome {
            Ok(connected) => {
                let Ok(raw) = serde_json::to_string(&connected.tokens) else {
                    tracing::warn!(account = account_id, "cannot serialise the account tokens");
                    return;
                };
                if let Err(error) = self.secrets.set(&account_tokens_key(account_id), raw).await {
                    tracing::warn!(account = account_id, code = %error.code, "cannot store the account tokens");
                    account.status = AccountStatus::Disconnected;
                    let _ = self.set_account(account).await;
                    self.bus.publish(BlueyEvent::AppError(error));
                    return;
                }
                self.tokens.lock().insert(
                    account_id.to_string(),
                    Arc::new(TokenCache::new(
                        Some(connected.tokens.clone()),
                        DEFAULT_REFRESH_LEEWAY,
                    )),
                );
                account.identity = Some(connected.identity);
                account.status = AccountStatus::Connected;
                account.connected_at = Some(now_iso());
                account.expires_at = connected.tokens.expires_at.map(iso_from_unix);
                if let Err(error) = self.set_account(account.clone()).await {
                    tracing::warn!(account = account_id, code = %error.code, "cannot persist the account");
                }
                tracing::info!(account = account_id, "subscription account connected");
                if let Err(error) = self.refresh_catalog(account_id, true).await {
                    tracing::debug!(account = account_id, code = %error.code, "catalog fetch deferred");
                }
            }
            Err(error) => {
                if error.is_cancelled() {
                    account.status = AccountStatus::Disconnected;
                } else {
                    account.status =
                        rules::status_after_error(&error).unwrap_or(AccountStatus::Disconnected);
                    tracing::warn!(account = account_id, code = %error.code, "subscription sign-in failed");
                    self.bus.publish(BlueyEvent::AppError(error));
                }
                let _ = self.set_account(account).await;
            }
        }
    }

    /// Abandon a pending sign-in.
    pub async fn cancel_connect(&self, account_id: &str) -> BlueyResult<ProviderAccount> {
        self.cancel_pending(account_id);
        let mut account = self.stored(account_id)?;
        if account.status.is_connecting() {
            account.status = AccountStatus::Disconnected;
            account = self.set_account(account).await?;
        }
        Ok(account)
    }

    /// Deliver a pasted `code#state` (manual-code flows).
    pub async fn submit_code(
        &self,
        account_id: &str,
        code: String,
    ) -> BlueyResult<ProviderAccount> {
        let sender = {
            let mut pending = self.pending.lock();
            match pending.get_mut(account_id) {
                Some(entry) => entry.manual_code.take(),
                None => None,
            }
        };
        let Some(sender) = sender else {
            return Err(BlueyError::account(
                "no_pending_flow",
                "no sign-in is waiting for a code — start again from the account card",
            ));
        };
        if sender.send(code).is_err() {
            return Err(BlueyError::account(
                "no_pending_flow",
                "the sign-in already finished — check the account card",
            ));
        }
        self.stored(account_id)
    }

    /// Revoke (best effort), forget the tokens and the catalog, unassign the
    /// roles that pointed at this provider, publish.
    pub async fn disconnect(&self, account_id: &str) -> BlueyResult<()> {
        self.cancel_pending(account_id);
        let mut account = self.stored(account_id)?;
        if let Ok(profile) = self.profile_for(&account.provider_id) {
            if let Some(tokens) = self.load_tokens(account_id).await {
                let _ = profile.revoke(&self.http, &tokens).await;
            }
        }
        self.secrets.delete(&account_tokens_key(account_id)).await?;
        self.tokens.lock().remove(account_id);
        self.drop_catalog(account_id).await?;
        self.unassign_roles(&account.provider_id).await?;
        account.status = AccountStatus::Disconnected;
        account.identity = None;
        account.connected_at = None;
        account.expires_at = None;
        account.catalog_fetched_at = None;
        self.set_account(account).await?;
        tracing::info!(account = account_id, "subscription account disconnected");
        Ok(())
    }

    /// Roles assigned to `provider_id` go back to unassigned (the router then
    /// uses its fallback chain).
    async fn unassign_roles(&self, provider_id: &str) -> BlueyResult<()> {
        let models = self.settings.get().ai.models;
        let mut patch = serde_json::Map::new();
        for role in ModelRole::ALL {
            if models
                .get(role)
                .is_some_and(|assignment| assignment.provider_id == provider_id)
            {
                patch.insert(role_key(role).to_string(), serde_json::Value::Null);
            }
        }
        if patch.is_empty() {
            return Ok(());
        }
        self.settings
            .update(serde_json::json!({ "ai": { "models": patch } }))
            .await
            .map(|_| ())
    }

    /// Fetch (or serve the fresh cached) catalog of a connected account.
    pub async fn refresh_catalog(
        &self,
        account_id: &str,
        force: bool,
    ) -> BlueyResult<ProviderModelCatalog> {
        self.ensure_enabled()?;
        let mut account = self.stored(account_id)?;
        if !account.status.is_connected() {
            return Err(BlueyError::account(
                "not_connected",
                "connect the account before fetching its models",
            ));
        }
        if !force {
            if let Some(cached) = self.catalog(account_id) {
                if rules::catalog_is_fresh(&cached, Utc::now()) {
                    return Ok(cached);
                }
            }
        }
        let profile = self.profile_for(&account.provider_id)?;
        let tokens = self.credential_for(account_id).await?;
        let identity = account.identity.clone().unwrap_or_default();
        match profile.catalog(&self.http, &tokens, &identity).await {
            Ok(catalog) => {
                self.store_catalog(catalog.clone()).await?;
                account.catalog_fetched_at = Some(catalog.fetched_at.clone());
                self.set_account(account).await?;
                self.apply_catalog_presets_after_fetch(&catalog).await;
                Ok(catalog)
            }
            Err(error) => {
                if let Some(status) = rules::status_after_error(&error) {
                    account.status = status;
                    self.set_account(account).await?;
                }
                Err(error)
            }
        }
    }

    /// Developer mode: one tiny request through the account's fingerprint,
    /// reporting whether the plan paid for it.
    pub async fn probe_fingerprint(&self, account_id: &str) -> BlueyResult<FingerprintProbe> {
        self.ensure_enabled()?;
        let account = self.stored(account_id)?;
        if !account.status.is_connected() {
            return Err(BlueyError::account(
                "not_connected",
                "connect the account before probing it",
            ));
        }
        let profile = self.profile_for(&account.provider_id)?;
        let tokens = self.credential_for(account_id).await?;
        profile.probe(&self.http, &tokens).await
    }

    /// After a catalog fetch: fill unassigned roles (and roles pointing at a
    /// model this catalog no longer lists) with the suggested models (§3.7).
    async fn apply_catalog_presets_after_fetch(&self, catalog: &ProviderModelCatalog) {
        let mut models = self.settings.get().ai.models;
        let changed = rules::apply_catalog_presets(&mut models, catalog, false);
        if changed.is_empty() {
            return;
        }
        match self
            .settings
            .update(serde_json::json!({ "ai": { "models": models } }))
            .await
        {
            Ok(_) => {
                tracing::info!(provider = %catalog.provider_id, roles = changed.len(), "assigned catalog presets")
            }
            Err(error) => {
                tracing::warn!(provider = %catalog.provider_id, code = %error.code, "cannot assign catalog presets")
            }
        }
    }

    /// Point roles at the account's recommended models (`ai_apply_provider_presets`
    /// for a subscription provider). `overwrite = false` fills only unassigned roles.
    pub async fn apply_presets(&self, provider_id: &str, overwrite: bool) -> BlueyResult<Settings> {
        self.ensure_enabled()?;
        let account = self.stored(provider_id)?;
        if !account.status.is_connected() {
            return Err(BlueyError::account(
                "not_connected",
                "connect the account before applying its models",
            ));
        }
        let catalog = match self.catalog(provider_id) {
            Some(catalog) => catalog,
            None => self.refresh_catalog(provider_id, true).await?,
        };
        let mut models = self.settings.get().ai.models;
        let changed = rules::apply_catalog_presets(&mut models, &catalog, overwrite);
        tracing::info!(
            provider = provider_id,
            roles = changed.len(),
            overwrite,
            "applied catalog presets"
        );
        let (_, new) = self
            .settings
            .update(serde_json::json!({ "ai": { "models": models } }))
            .await?;
        Ok(new)
    }

    /// The accounts as the router sees them (§3.6): one provider per account,
    /// keyed while usable. Empty when the layer is switched off.
    pub fn provider_configs(&self) -> Vec<AiProviderConfig> {
        if !self.enabled() {
            return Vec::new();
        }
        let now = Utc::now();
        self.list()
            .iter()
            .map(|account| rules::provider_config(account, now, true))
            .collect()
    }

    /// The signed-in identity of an account, if any.
    pub fn identity(&self, account_id: &str) -> Option<AccountIdentity> {
        self.stored(account_id)
            .ok()
            .and_then(|account| account.identity)
    }

    /// A request through the account failed: move it to the status the error
    /// names (401 → `NeedsReauth`, plan limit → `RateLimited`, block or drifted
    /// fingerprint → `Unavailable`). Plain provider errors leave the status alone.
    pub async fn note_request_error(&self, account_id: &str, error: &BlueyError) {
        let Some(status) = rules::status_after_error(error) else {
            return;
        };
        let Ok(mut account) = self.stored(account_id) else {
            return;
        };
        if account.status == status {
            return;
        }
        tracing::warn!(account = account_id, code = %error.code, status = status.state_name(), "account status changed after a request error");
        account.status = status;
        let _ = self.set_account(account).await;
    }

    /// A request through the account succeeded: a rate-limit window it was
    /// waiting out is over.
    pub async fn note_request_success(&self, account_id: &str) {
        let Ok(mut account) = self.stored(account_id) else {
            return;
        };
        if matches!(account.status, AccountStatus::RateLimited { .. }) {
            account.status = AccountStatus::Connected;
            let _ = self.set_account(account).await;
        }
    }

    async fn load_tokens(&self, account_id: &str) -> Option<TokenSet> {
        let raw = self
            .secrets
            .get(&account_tokens_key(account_id))
            .await
            .ok()
            .flatten()?;
        serde_json::from_str(&raw).ok()
    }

    async fn token_cache(&self, account_id: &str) -> Arc<TokenCache> {
        if let Some(cache) = self.tokens.lock().get(account_id).cloned() {
            return cache;
        }
        let tokens = self.load_tokens(account_id).await;
        let cache = Arc::new(TokenCache::new(tokens, DEFAULT_REFRESH_LEEWAY));
        self.tokens
            .lock()
            .entry(account_id.to_string())
            .or_insert_with(|| cache.clone())
            .clone()
    }

    /// A fresh access token for `account_id`, refreshing under a single-flight
    /// lock when it is about to expire. A dead refresh token flips the account
    /// to `NeedsReauth`; the adapters then fall back to the API-key provider.
    pub async fn credential_for(&self, account_id: &str) -> BlueyResult<TokenSet> {
        let account = self.stored(account_id)?;
        let profile = self.profile_for(&account.provider_id)?;
        let cache = self.token_cache(account_id).await;
        let http = self.http.clone();
        let result = cache
            .fresh(unix_now(), |current, _refresh_token| {
                let profile = profile.clone();
                async move { profile.refresh(&http, &current).await }
            })
            .await;
        match result {
            Ok(tokens) => {
                if let Some(expires_at) = tokens.expires_at {
                    let iso = iso_from_unix(expires_at);
                    if account.expires_at.as_deref() != Some(iso.as_str()) {
                        let mut updated = account;
                        updated.expires_at = Some(iso);
                        let _ = self.set_account(updated).await;
                    }
                    if let Ok(raw) = serde_json::to_string(&tokens) {
                        let _ = self.secrets.set(&account_tokens_key(account_id), raw).await;
                    }
                }
                Ok(tokens)
            }
            Err(RefreshError::NoTokens) => Err(BlueyError::account(
                "not_connected",
                "the account has no stored sign-in",
            )),
            Err(RefreshError::NoRefreshToken) => {
                self.mark_needs_reauth(account).await;
                Err(needs_reauth(account_id, &account_id_provider(account_id)))
            }
            Err(RefreshError::Refresh(error)) => {
                if error.kind == BlueyErrorKind::Authentication {
                    self.mark_needs_reauth(account.clone()).await;
                    return Err(needs_reauth(account_id, &account.provider_id));
                }
                Err(error)
            }
        }
    }

    async fn mark_needs_reauth(&self, mut account: ProviderAccount) {
        if account.status != AccountStatus::NeedsReauth {
            account.status = AccountStatus::NeedsReauth;
            let _ = self.set_account(account).await;
        }
    }

    /// Boot: validate the stored tokens of connected accounts in the
    /// background — refresh when expiring, `NeedsReauth` when rejected, keep
    /// the cached state when merely offline.
    pub async fn restore(&self) {
        if !self.enabled() {
            return;
        }
        let connected: Vec<String> = self
            .accounts
            .read()
            .iter()
            .filter(|account| account.status.is_connected())
            .map(|account| account.account_id.clone())
            .collect();
        for account_id in connected {
            match self.credential_for(&account_id).await {
                Ok(_) => {}
                Err(error) if error.kind == BlueyErrorKind::Authentication => {
                    tracing::warn!(account = %account_id, code = %error.code, "stored account sign-in rejected");
                }
                Err(error) => {
                    tracing::debug!(account = %account_id, code = %error.code, "account check deferred (offline?)");
                }
            }
        }
    }

    /// `data_reset_all`: forget every account's tokens, catalog and status.
    /// Continues past failures and returns them for the caller's report.
    pub async fn reset_all(&self) -> Vec<(String, BlueyError)> {
        let mut failures = Vec::new();
        let accounts = self.accounts.read().clone();
        for account in &accounts {
            self.cancel_pending(&account.account_id);
            if let Err(error) = self
                .secrets
                .delete(&account_tokens_key(&account.account_id))
                .await
            {
                failures.push((format!("account tokens {}", account.account_id), error));
            }
            if let Err(error) = self.drop_catalog(&account.account_id).await {
                failures.push((format!("account catalog {}", account.account_id), error));
            }
        }
        self.tokens.lock().clear();
        let blanks: Vec<ProviderAccount> = self
            .profiles
            .iter()
            .map(|profile| Self::blank_account(profile.as_ref()))
            .collect();
        *self.accounts.write() = blanks.clone();
        let value = serde_json::to_value(&blanks).unwrap_or(serde_json::Value::Array(vec![]));
        if let Err(error) = self
            .storage
            .run(move |db| SettingsRepository::set_json(db, ACCOUNTS_KEY, &value))
            .await
        {
            failures.push(("account list".into(), error));
        }
        for account in blanks {
            self.bus.publish(BlueyEvent::AccountsChanged(account));
        }
        failures
    }
}

fn not_found(account_id: &str) -> BlueyError {
    BlueyError::account("not_found", format!("no account `{account_id}`"))
}

fn needs_reauth(account_id: &str, provider_id: &str) -> BlueyError {
    BlueyError::new(
        BlueyErrorKind::Authentication,
        codes::NEEDS_REAUTH,
        "the account's sign-in expired — reconnect it",
    )
    .recoverable(RecoveryAction::reconnect_account(account_id, provider_id))
}

/// Accounts are keyed by provider id in this version (one per provider).
fn account_id_provider(account_id: &str) -> String {
    account_id.to_string()
}

fn iso_from_unix(secs: u64) -> String {
    chrono::DateTime::<Utc>::from_timestamp(secs as i64, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn role_key(role: ModelRole) -> &'static str {
    match role {
        ModelRole::Default => "default",
        ModelRole::Fast => "fast",
        ModelRole::Reasoning => "reasoning",
        ModelRole::Vision => "vision",
        ModelRole::Research => "research",
        ModelRole::Transcription => "transcription",
        ModelRole::Embedding => "embedding",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_timestamps_are_stable() {
        assert_eq!(catalog_key("chatgpt"), "accounts:catalog:chatgpt");
        assert_eq!(iso_from_unix(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(role_key(ModelRole::Transcription), "transcription");
    }

    #[test]
    fn needs_reauth_carries_the_reconnect_recovery() {
        let error = needs_reauth("claude", "claude");
        assert_eq!(error.code, codes::NEEDS_REAUTH);
        assert_eq!(
            error.recovery,
            Some(RecoveryAction::reconnect_account("claude", "claude"))
        );
        assert!(!error.message.contains("sk-"));
    }
}
