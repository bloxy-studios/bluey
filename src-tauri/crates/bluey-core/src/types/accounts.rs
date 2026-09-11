//! Provider accounts (ADR 0009): the owner's paid AI subscriptions as
//! credential sources next to API keys. Mirrors `src/lib/types/accounts.ts`
//! byte-for-byte on the wire.
//!
//! Nothing here carries a token. The WebView sees an account's status, plan and
//! identity; the tokens live in the Keychain under
//! `account:<account_id>:oauth_tokens` and are read only by the Rust
//! `AccountsManager`.

use serde::{Deserialize, Serialize};

use super::ai::AiProviderKind;
use super::mode::ModelRole;

/// Reserved provider ids of the subscription providers (one account each,
/// several for Google once PR 3c lands multi-account rotation).
pub const CHATGPT_PROVIDER_ID: &str = "chatgpt";
pub const CLAUDE_PROVIDER_ID: &str = "claude";
pub const ANTIGRAVITY_PROVIDER_ID: &str = "antigravity";

/// How a provider config authenticates: an API key in the Keychain, or an
/// OAuth subscription account (`ProviderAccount`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMethod {
    #[default]
    ApiKey,
    OauthSubscription,
}

/// Why an account cannot serve requests right now (`AccountStatus::Unavailable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The provider stopped recognising Bluey's request fingerprint.
    FingerprintDrift,
    /// The provider refused the account (terms, region, entitlement).
    PolicyBlocked,
    /// The model catalog endpoint is gone or rejected the pinned client version.
    CatalogUnavailable,
    /// The provider bills requests outside the plan — a stop signal, never a retry.
    ExtraUsageBilling,
    /// The provider's integration is not built into this version yet.
    ProviderPending,
    /// Subscription accounts are switched off (feature flag or build feature).
    Disabled,
    Other,
}

/// How the browser hands the result back during `AccountStatus::Connecting`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectFlowKind {
    /// PKCE in the system browser with a loopback redirect.
    Browser,
    /// RFC 8628: the user enters `user_code` at `verification_url`.
    DeviceCode,
    /// The redirect page shows a code the user pastes back (`code#state`).
    ManualCode,
}

/// Mirrors `ConnectFlow` — what the UI shows while a connection is pending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectFlow {
    pub kind: ConnectFlowKind,
    /// The authorization URL that was opened (for "copy link").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Device-code flows: the code to type in the browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    /// Device-code flows: where to type it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_url: Option<String>,
    /// When the pending flow expires (ISO 8601).
    pub expires_at: String,
}

/// Mirrors `AccountStatus` (tag `state`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AccountStatus {
    #[default]
    Disconnected,
    Connecting {
        flow: ConnectFlow,
    },
    Connected,
    /// The refresh token is gone or was rejected; the user must reconnect.
    NeedsReauth,
    /// A plan window is exhausted; the router skips the account until `until`.
    RateLimited {
        /// ISO 8601 reset time.
        until: String,
        /// The window that tripped (`5h`, `7d`, …), when the provider names it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window: Option<String>,
    },
    Unavailable {
        reason: UnavailableReason,
        /// Human-readable detail from the provider (never a token, never a prompt).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

impl AccountStatus {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }

    pub fn is_connecting(&self) -> bool {
        matches!(self, Self::Connecting { .. })
    }

    /// Stable machine-readable name of the state (the wire tag).
    pub fn state_name(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting { .. } => "connecting",
            Self::Connected => "connected",
            Self::NeedsReauth => "needs_reauth",
            Self::RateLimited { .. } => "rate_limited",
            Self::Unavailable { .. } => "unavailable",
        }
    }
}

/// Mirrors `AccountIdentity` — what the provider says about the signed-in
/// subscription. Display only; nothing here is a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Provider-native tier id (`plus`, `default_claude_max_5x`, `g1-pro`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_tier: Option<String>,
    /// Human label for the tier (`ChatGPT Plus`, `Claude Max 5×`, `Google AI Pro`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_label: Option<String>,
    /// Provider account / organisation id (Codex `chatgpt_account_id`, Claude org uuid).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Antigravity: the Cloud Code companion project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// Mirrors `ProviderAccount` — everything the WebView may know about a
/// subscription account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccount {
    /// Internal id (`chatgpt`, `claude`, `antigravity`, `antigravity-2`, …).
    pub account_id: String,
    /// The reserved provider id this account serves.
    pub provider_id: String,
    pub kind: AiProviderKind,
    pub method: ProviderAuthMethod,
    pub status: AccountStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<AccountIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
    /// Access-token expiry (ISO 8601), for the dev overlay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_fetched_at: Option<String>,
    /// The request-fingerprint module version this build ships for the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_version: Option<String>,
    /// When that fingerprint was captured from the official client (ISO date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_captured_on: Option<String>,
}

/// Mirrors `ModelCapabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    pub vision: bool,
    pub tools: bool,
    /// Reasoning / thinking levels the model accepts (`minimal`, `low`, …).
    #[serde(default)]
    pub reasoning_levels: Vec<String>,
    pub streaming: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

/// Mirrors `CatalogModel` — one model a subscription exposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub id: String,
    pub label: String,
    pub capabilities: ModelCapabilities,
    /// Antigravity: `antigravity` vs `gemini_cli`; other providers omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_pool: Option<String>,
    /// Roles the provider profile recommends this model for.
    #[serde(default)]
    pub suggested_roles: Vec<ModelRole>,
}

/// Mirrors `CatalogSource` (tag `type`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CatalogSource {
    /// Fetched from the provider's models endpoint.
    Endpoint,
    /// Built by probing each candidate model.
    Probed,
    /// Copied from a dated capture of the official client.
    Curated { version: String },
    /// Development fixture (mock provider).
    Fixture,
}

/// Mirrors `ProviderModelCatalog`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelCatalog {
    pub account_id: String,
    pub provider_id: String,
    pub fetched_at: String,
    pub source: CatalogSource,
    pub models: Vec<CatalogModel>,
}

/// Mirrors `AccountConnectOptions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountConnectOptions {
    /// Antigravity on a Workspace account: the Google Cloud project to use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Skip the loopback listener and start with the device-code flow.
    #[serde(default)]
    pub prefer_device_code: bool,
}

/// Which pool a probe request was billed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BilledTo {
    Plan,
    ExtraUsage,
    Unknown,
}

/// Mirrors `FingerprintProbe` — the result of `accounts_probe_fingerprint`
/// (developer mode): one tiny request, and whether the plan paid for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintProbe {
    pub account_id: String,
    pub ok: bool,
    pub billed_to: BilledTo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub checked_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_serialise_with_the_state_tag() {
        assert_eq!(
            serde_json::to_value(AccountStatus::Disconnected).unwrap(),
            serde_json::json!({ "state": "disconnected" })
        );
        assert_eq!(
            serde_json::to_value(AccountStatus::RateLimited {
                until: "2026-09-11T14:32:00Z".into(),
                window: Some("5h".into()),
            })
            .unwrap(),
            serde_json::json!({ "state": "rate_limited", "until": "2026-09-11T14:32:00Z", "window": "5h" })
        );
        assert_eq!(
            serde_json::to_value(AccountStatus::Unavailable {
                reason: UnavailableReason::FingerprintDrift,
                detail: None,
            })
            .unwrap(),
            serde_json::json!({ "state": "unavailable", "reason": "fingerprint_drift" })
        );
        let connecting: AccountStatus = serde_json::from_value(serde_json::json!({
            "state": "connecting",
            "flow": { "kind": "device_code", "userCode": "ABCD-1234", "verificationUrl": "https://auth.openai.com/codex/device", "expiresAt": "2026-09-11T14:47:00Z" }
        }))
        .unwrap();
        assert!(connecting.is_connecting());
        assert_eq!(connecting.state_name(), "connecting");
    }

    #[test]
    fn kinds_and_methods_use_the_wire_names() {
        assert_eq!(
            serde_json::to_value(ProviderAuthMethod::OauthSubscription).unwrap(),
            "oauth_subscription"
        );
        assert_eq!(
            serde_json::to_value(AiProviderKind::ChatgptCodex).unwrap(),
            "chatgpt_codex"
        );
        assert_eq!(
            serde_json::to_value(AiProviderKind::ClaudeSubscription).unwrap(),
            "claude_subscription"
        );
        assert_eq!(
            serde_json::to_value(AiProviderKind::AntigravityGoogle).unwrap(),
            "antigravity_google"
        );
        assert_eq!(
            serde_json::to_value(CatalogSource::Curated {
                version: "2.1.268".into()
            })
            .unwrap(),
            serde_json::json!({ "type": "curated", "version": "2.1.268" })
        );
    }

    #[test]
    fn a_provider_account_round_trips_in_camel_case() {
        let account = ProviderAccount {
            account_id: "claude".into(),
            provider_id: "claude".into(),
            kind: AiProviderKind::ClaudeSubscription,
            method: ProviderAuthMethod::OauthSubscription,
            status: AccountStatus::Connected,
            identity: Some(AccountIdentity {
                email: Some("owner@example.com".into()),
                plan_label: Some("Claude Max 5×".into()),
                ..AccountIdentity::default()
            }),
            connected_at: Some("2026-09-11T08:00:00Z".into()),
            expires_at: None,
            catalog_fetched_at: None,
            fingerprint_version: Some("claude_code/2.1.268".into()),
            fingerprint_captured_on: Some("2026-09-11".into()),
        };
        let json = serde_json::to_value(&account).unwrap();
        assert_eq!(json["accountId"], "claude");
        assert_eq!(json["status"]["state"], "connected");
        assert_eq!(json["identity"]["planLabel"], "Claude Max 5×");
        assert_eq!(json["fingerprintCapturedOn"], "2026-09-11");
        assert!(json.get("expiresAt").is_none());
        let back: ProviderAccount = serde_json::from_value(json).unwrap();
        assert_eq!(back, account);
    }
}
