//! Keychain op counts of subscription-account tokens (ADR 0011) and the
//! refresh rules: persist only after a refresh, never refresh a rotating
//! import, force one refresh on a 401, never prompt to disconnect.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use bluey_core::types::{
    AccountConnectOptions, AccountIdentity, AccountStatus, AiProviderKind, FingerprintProbe,
    ProviderModelCatalog,
};
use bluey_core::{BlueyError, BlueyResult};
use bluey_oauth::{unix_now, TokenSet};
use bluey_protocols::request_shaper::FingerprintInfo;

use super::profile::{ConnectStart, Connected, ProviderProfile};
use super::{AccountsManager, Origin};
use crate::events::EventBus;
use crate::secrets::account_tokens_key;
use crate::secrets::backend::fake::{CountingFake, Op};
use crate::secrets::SecretsStore;
use crate::settings::SettingsManager;
use crate::storage::Storage;

/// A `claude` profile whose refresh hands out `access-<n>` and counts.
#[derive(Default)]
struct FakeProfile {
    refreshes: AtomicUsize,
}

impl FakeProfile {
    fn refreshes(&self) -> usize {
        self.refreshes.load(Ordering::SeqCst)
    }
}

fn unused() -> BlueyError {
    BlueyError::internal("not used by these tests")
}

#[async_trait]
impl ProviderProfile for FakeProfile {
    fn provider_id(&self) -> &'static str {
        "claude"
    }
    fn kind(&self) -> AiProviderKind {
        AiProviderKind::ClaudeSubscription
    }
    fn display_name(&self) -> &'static str {
        "Claude"
    }
    fn fingerprint(&self) -> Option<FingerprintInfo> {
        None
    }
    async fn begin_connect(
        &self,
        _: &reqwest::Client,
        _: &AccountConnectOptions,
    ) -> BlueyResult<ConnectStart> {
        Err(unused())
    }
    async fn import(&self, _: &reqwest::Client) -> BlueyResult<Connected> {
        Err(unused())
    }
    async fn catalog(
        &self,
        _: &reqwest::Client,
        _: &TokenSet,
        _: &AccountIdentity,
    ) -> BlueyResult<ProviderModelCatalog> {
        Err(unused())
    }
    async fn refresh(&self, _: &reqwest::Client, _: &TokenSet) -> BlueyResult<TokenSet> {
        let n = self.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(tokens(&format!("access-{n}"), unix_now() + 3_600))
    }
    async fn revoke(&self, _: &reqwest::Client, _: &TokenSet) -> BlueyResult<()> {
        Ok(())
    }
    async fn probe(&self, _: &reqwest::Client, _: &TokenSet) -> BlueyResult<FingerprintProbe> {
        Err(unused())
    }
}

fn tokens(access: &str, expires_at: u64) -> TokenSet {
    TokenSet {
        access_token: access.into(),
        refresh_token: Some("refresh".into()),
        expires_at: Some(expires_at),
        id_token: None,
    }
}

const KEY: &str = "account:claude:oauth_tokens";
const BROWSER: Option<Origin> = Some(Origin::Browser);

/// A connected `claude` account whose Keychain item holds `stored`; counts
/// start at zero after setup.
async fn connected(
    stored: &TokenSet,
    origin: Option<Origin>,
) -> (Arc<CountingFake>, Arc<FakeProfile>, AccountsManager) {
    assert_eq!(account_tokens_key("claude"), KEY);
    let raw = serde_json::to_string(stored).unwrap();
    let fake = Arc::new(CountingFake::with_items(&[(KEY, raw.as_str())]));
    let secrets = Arc::new(SecretsStore::with_backend(fake.clone()));
    secrets.preload_presence().unwrap();
    let storage = Arc::new(Storage::in_memory());
    let bus = Arc::new(EventBus::new());
    let settings =
        Arc::new(SettingsManager::load(storage.clone(), secrets.clone(), bus.clone()).unwrap());
    let mut manager =
        AccountsManager::load(secrets, storage, bus, settings, reqwest::Client::new()).unwrap();
    let profile = Arc::new(FakeProfile::default());
    manager.profiles = vec![profile.clone()];
    let mut account = manager.stored("claude").unwrap();
    account.status = AccountStatus::Connected;
    manager.set_account(account).await.unwrap();
    manager.set_origin("claude", origin).await.unwrap();
    fake.reset_counts();
    (fake, profile, manager)
}

fn status(manager: &AccountsManager) -> AccountStatus {
    manager.stored("claude").unwrap().status
}

#[tokio::test]
async fn requests_with_a_valid_token_never_write_the_keychain() {
    let (fake, profile, manager) = connected(&tokens("a", unix_now() + 3_600), BROWSER).await;
    for _ in 0..20 {
        assert_eq!(
            manager.credential_for("claude").await.unwrap().access_token,
            "a"
        );
    }
    assert_eq!(fake.reads(), 1, "decrypted once per process");
    assert_eq!(fake.writes(), 0);
    assert_eq!(fake.count(Op::Remove), 0);
    assert_eq!(profile.refreshes(), 0);
}

#[tokio::test]
async fn a_refresh_rewrites_the_item_exactly_once() {
    let (fake, profile, manager) = connected(&tokens("a", unix_now() - 10), BROWSER).await;
    for _ in 0..5 {
        assert_eq!(
            manager.credential_for("claude").await.unwrap().access_token,
            "access-1"
        );
    }
    assert_eq!(profile.refreshes(), 1);
    assert_eq!(fake.writes(), 1);
    assert!(fake.value(KEY).unwrap().contains("access-1"));
}

#[tokio::test]
async fn an_imported_rotating_session_is_never_refreshed() {
    let (fake, profile, manager) =
        connected(&tokens("a", unix_now() - 10), Some(Origin::Import)).await;
    let error = manager.credential_for("claude").await.unwrap_err();
    assert_eq!(error.code, "account.needs_reauth");
    assert!(error.message.contains("import it again"));
    assert_eq!(error.details.as_ref().unwrap()["imported"], true);
    assert_eq!(
        profile.refreshes(),
        0,
        "a refresh would sign Claude Code out"
    );
    assert_eq!(fake.writes(), 0);
    assert_eq!(status(&manager), AccountStatus::NeedsReauth);
    assert!(!manager.refresh_rejected("claude", "a").await);
    assert_eq!(profile.refreshes(), 0);
}

#[tokio::test]
async fn a_401_on_a_valid_token_forces_one_shared_refresh() {
    let (fake, profile, manager) = connected(&tokens("a", unix_now() + 3_600), BROWSER).await;
    assert_eq!(
        manager.credential_for("claude").await.unwrap().access_token,
        "a"
    );
    // Two requests saw `a` rejected: one refresh, both may retry.
    assert!(manager.refresh_rejected("claude", "a").await);
    assert!(manager.refresh_rejected("claude", "a").await);
    assert_eq!(profile.refreshes(), 1);
    assert_eq!(fake.writes(), 1);
    assert_eq!(
        manager.credential_for("claude").await.unwrap().access_token,
        "access-1"
    );
    assert_eq!(status(&manager), AccountStatus::Connected);
}

#[tokio::test]
async fn a_locked_token_item_is_not_reported_as_disconnected() {
    let (fake, _, manager) = connected(&tokens("a", unix_now() + 3_600), BROWSER).await;
    fake.lock_item(KEY, -25293);
    let error = manager.credential_for("claude").await.unwrap_err();
    assert_eq!(error.code, "storage.keychain_access_denied");
    assert_eq!(status(&manager), AccountStatus::Connected);
    // The refusal is not cached: once allowed, the next request reads it.
    fake.unlock(KEY);
    assert_eq!(
        manager.credential_for("claude").await.unwrap().access_token,
        "a"
    );
}

#[tokio::test]
async fn disconnect_never_decrypts_and_completes_when_the_delete_fails() {
    let (fake, _, manager) = connected(&tokens("a", unix_now() + 3_600), BROWSER).await;
    fake.fail_remove(KEY, -25293);
    let outcome = manager.disconnect("claude").await;
    assert_eq!(outcome.unwrap_err().code, "storage.keychain_access_denied");
    assert_eq!(status(&manager), AccountStatus::Disconnected);
    assert_eq!(fake.reads(), 0, "revocation uses in-memory tokens only");
    assert_eq!(manager.origins.read().get("claude"), None);
}

#[tokio::test]
async fn an_account_without_a_recorded_origin_is_never_refreshed() {
    // Connected before origins were recorded: it may be a Claude Code import,
    // and a refresh would rotate the token and sign Claude Code out.
    let (fake, profile, manager) = connected(&tokens("a", unix_now() - 10), None).await;
    let error = manager.credential_for("claude").await.unwrap_err();
    assert_eq!(error.code, "account.needs_reauth");
    // It may equally be a browser sign-in: the copy must not claim an import.
    assert!(error.details.is_none(), "{:?}", error.details);
    assert!(!error.message.contains("import"), "{}", error.message);
    assert!(!manager.refresh_rejected("claude", "a").await);
    assert_eq!(profile.refreshes(), 0);
    assert_eq!(fake.writes(), 0);
}

#[tokio::test]
async fn the_origin_is_recorded_before_the_tokens_and_survives_a_restart() {
    let (fake, _, manager) = connected(&tokens("a", unix_now() + 3_600), None).await;
    let connected = Connected {
        tokens: tokens("b", unix_now() + 3_600),
        identity: AccountIdentity::default(),
    };
    manager
        .finish_connect("claude", Ok(connected), Origin::Import)
        .await;
    assert_eq!(fake.writes(), 1);
    assert_eq!(status(&manager), AccountStatus::Connected);
    let restarted = AccountsManager::load(
        manager.secrets.clone(),
        manager.storage.clone(),
        manager.bus.clone(),
        manager.settings.clone(),
        reqwest::Client::new(),
    )
    .unwrap();
    assert_eq!(
        restarted.origins.read().get("claude"),
        Some(&Origin::Import)
    );
    assert!(!restarted.signed_in_by_bluey("claude"));
}

#[tokio::test]
async fn boot_never_reads_a_locked_or_unprobeable_account_item() {
    let (fake, profile, manager) = connected(&tokens("a", unix_now() - 10), BROWSER).await;
    fake.lock_item(KEY, -25308);
    manager.restore().await;
    assert_eq!(
        fake.reads(),
        0,
        "a read here is a Keychain prompt at launch"
    );
    assert_eq!(profile.refreshes(), 0);
    assert_eq!(status(&manager), AccountStatus::Connected);
    // A probe that fails outright defers the check the same way.
    fake.unlock(KEY);
    fake.fail_lookups(-25291);
    manager.restore().await;
    assert_eq!(fake.reads(), 0);
    assert_eq!(status(&manager), AccountStatus::Connected);
}
