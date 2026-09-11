//! `ProviderProfile` — what one subscription provider contributes to the
//! accounts layer (ADR 0009 §3.4): how to start and complete a sign-in, how to
//! import the official client's local sign-in, how to fetch the model catalog,
//! refresh, revoke and probe. The manager owns everything else (Keychain,
//! persistence, status, events), so a provider is one file in PR 3a–3c.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use bluey_core::error::RecoveryAction;
use bluey_core::types::{
    AccountConnectOptions, AccountIdentity, AiProviderKind, ConnectFlow, FingerprintProbe,
    ProviderModelCatalog,
};
use bluey_core::{BlueyError, BlueyResult};
use bluey_oauth::TokenSet;
use bluey_protocols::fingerprints;
use bluey_protocols::request_shaper::FingerprintInfo;
use tokio_util::sync::CancellationToken;

/// A finished sign-in: the tokens to keep (Keychain) and who the user is.
pub struct Connected {
    pub tokens: TokenSet,
    pub identity: AccountIdentity,
}

pub type CompletionFuture = Pin<Box<dyn Future<Output = BlueyResult<Connected>> + Send>>;

/// A sign-in that has been started: what the UI shows, the future that
/// resolves when the browser / device / pasted code comes back, a token to
/// abandon it, and — for manual-code flows — where to deliver the pasted code.
pub struct ConnectStart {
    pub flow: ConnectFlow,
    pub completion: CompletionFuture,
    pub cancel: CancellationToken,
    pub manual_code: Option<tokio::sync::oneshot::Sender<String>>,
}

#[async_trait]
pub trait ProviderProfile: Send + Sync {
    /// Reserved provider id (`chatgpt`, `claude`, `antigravity`).
    fn provider_id(&self) -> &'static str;
    fn kind(&self) -> AiProviderKind;
    fn display_name(&self) -> &'static str;
    /// The request fingerprint this build reproduces, once the provider is real.
    fn fingerprint(&self) -> Option<FingerprintInfo>;

    /// Open the browser (or start the device / manual flow) and return the
    /// pending sign-in.
    async fn begin_connect(
        &self,
        http: &reqwest::Client,
        options: &AccountConnectOptions,
    ) -> BlueyResult<ConnectStart>;

    /// Read the official client's local sign-in on this Mac (read-only) and
    /// return it as if the user had just connected.
    async fn import(&self, http: &reqwest::Client) -> BlueyResult<Connected>;

    /// The models this subscription exposes.
    async fn catalog(
        &self,
        http: &reqwest::Client,
        tokens: &TokenSet,
        identity: &AccountIdentity,
    ) -> BlueyResult<ProviderModelCatalog>;

    /// Renew the access token. `account.needs_reauth` when the refresh token is dead.
    async fn refresh(&self, http: &reqwest::Client, tokens: &TokenSet) -> BlueyResult<TokenSet>;

    /// Best-effort revocation on disconnect.
    async fn revoke(&self, http: &reqwest::Client, tokens: &TokenSet) -> BlueyResult<()>;

    /// Developer mode: one tiny request, reporting whether the plan paid for it.
    async fn probe(
        &self,
        http: &reqwest::Client,
        tokens: &TokenSet,
    ) -> BlueyResult<FingerprintProbe>;
}

/// Placeholder for a provider this build does not carry (a build without the
/// `subscription-accounts` feature): the account card exists, and every action
/// explains what is missing instead of pretending. Feature builds ship the real
/// profiles — ChatGPT (PR 3a), Claude (PR 3b), Google AI (PR 3c).
pub struct PendingProfile {
    provider_id: &'static str,
    kind: AiProviderKind,
    display_name: &'static str,
    lands_in: &'static str,
}

impl PendingProfile {
    fn pending(&self) -> BlueyError {
        BlueyError::account(
            "provider_pending",
            format!(
                "{} sign-in is not built into this version of Bluey yet ({}). Use an API key for now.",
                self.display_name, self.lands_in
            ),
        )
        .recoverable(RecoveryAction::UseApiKey)
    }
}

#[async_trait]
impl ProviderProfile for PendingProfile {
    fn provider_id(&self) -> &'static str {
        self.provider_id
    }

    fn kind(&self) -> AiProviderKind {
        self.kind
    }

    fn display_name(&self) -> &'static str {
        self.display_name
    }

    fn fingerprint(&self) -> Option<FingerprintInfo> {
        // The documented fingerprint (`bluey_protocols::fingerprints`) that PR 3a–3c reproduce;
        // the Accounts card shows its version and capture date.
        Some(match self.kind {
            AiProviderKind::ChatgptCodex => fingerprints::codex::INFO,
            AiProviderKind::ClaudeSubscription => fingerprints::claude_code::INFO,
            AiProviderKind::AntigravityGoogle => fingerprints::antigravity::INFO,
            _ => return None,
        })
    }

    async fn begin_connect(
        &self,
        _http: &reqwest::Client,
        _options: &AccountConnectOptions,
    ) -> BlueyResult<ConnectStart> {
        Err(self.pending())
    }

    async fn import(&self, _http: &reqwest::Client) -> BlueyResult<Connected> {
        Err(self.pending())
    }

    async fn catalog(
        &self,
        _http: &reqwest::Client,
        _tokens: &TokenSet,
        _identity: &AccountIdentity,
    ) -> BlueyResult<ProviderModelCatalog> {
        Err(self.pending())
    }

    async fn refresh(&self, _http: &reqwest::Client, _tokens: &TokenSet) -> BlueyResult<TokenSet> {
        Err(self.pending())
    }

    async fn revoke(&self, _http: &reqwest::Client, _tokens: &TokenSet) -> BlueyResult<()> {
        Ok(())
    }

    async fn probe(
        &self,
        _http: &reqwest::Client,
        _tokens: &TokenSet,
    ) -> BlueyResult<FingerprintProbe> {
        Err(self.pending())
    }
}

/// ChatGPT is real since PR 3a — in builds with the `subscription-accounts` feature.
#[cfg(feature = "subscription-accounts")]
fn chatgpt_profile() -> Arc<dyn ProviderProfile> {
    Arc::new(super::chatgpt::ChatgptProfile)
}

#[cfg(not(feature = "subscription-accounts"))]
fn chatgpt_profile() -> Arc<dyn ProviderProfile> {
    Arc::new(PendingProfile {
        provider_id: bluey_core::types::CHATGPT_PROVIDER_ID,
        kind: AiProviderKind::ChatgptCodex,
        display_name: "ChatGPT",
        lands_in: "a build with the subscription-accounts feature",
    })
}

/// Claude is real since PR 3b — in builds with the `subscription-accounts` feature.
#[cfg(feature = "subscription-accounts")]
fn claude_profile() -> Arc<dyn ProviderProfile> {
    Arc::new(super::claude::ClaudeProfile)
}

#[cfg(not(feature = "subscription-accounts"))]
fn claude_profile() -> Arc<dyn ProviderProfile> {
    Arc::new(PendingProfile {
        provider_id: bluey_core::types::CLAUDE_PROVIDER_ID,
        kind: AiProviderKind::ClaudeSubscription,
        display_name: "Claude",
        lands_in: "a build with the subscription-accounts feature",
    })
}

/// Google AI is real since PR 3c — in builds with the `subscription-accounts` feature.
#[cfg(feature = "subscription-accounts")]
fn antigravity_profile() -> Arc<dyn ProviderProfile> {
    Arc::new(super::antigravity::AntigravityProfile)
}

#[cfg(not(feature = "subscription-accounts"))]
fn antigravity_profile() -> Arc<dyn ProviderProfile> {
    Arc::new(PendingProfile {
        provider_id: bluey_core::types::ANTIGRAVITY_PROVIDER_ID,
        kind: AiProviderKind::AntigravityGoogle,
        display_name: "Google AI",
        lands_in: "a build with the subscription-accounts feature",
    })
}

/// The profiles this build ships, in UI order.
pub fn profiles() -> Vec<Arc<dyn ProviderProfile>> {
    vec![chatgpt_profile(), claude_profile(), antigravity_profile()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reserved_provider_has_a_profile_with_matching_kind() {
        let profiles = profiles();
        assert_eq!(profiles.len(), 3);
        for profile in &profiles {
            assert_eq!(
                bluey_core::accounts::kind_for_provider_id(profile.provider_id()),
                Some(profile.kind())
            );
            assert_eq!(
                bluey_core::accounts::provider_display_name(profile.provider_id()),
                Some(profile.display_name())
            );
            assert!(
                profile.fingerprint().is_some(),
                "every profile reports the documented fingerprint its build reproduces"
            );
        }
    }

    #[test]
    fn pending_errors_point_at_the_api_key_path() {
        let profile = PendingProfile {
            provider_id: bluey_core::types::ANTIGRAVITY_PROVIDER_ID,
            kind: AiProviderKind::AntigravityGoogle,
            display_name: "Google AI",
            lands_in: "a build with the subscription-accounts feature",
        };
        let error = profile.pending();
        assert_eq!(error.code, "account.provider_pending");
        assert!(error.message.contains("subscription-accounts"));
        assert_eq!(error.recovery, Some(RecoveryAction::UseApiKey));
        assert!(!error.message.contains("token"));
    }
}
