//! Pure rules of the provider-accounts layer (ADR 0009): which provider ids
//! and kinds exist, how an account's status changes on a provider error, when
//! a catalog is stale, and which role each catalog model is recommended for.
//! The I/O — Keychain, browser, HTTP — lives in the app crate's
//! `AccountsManager`; everything here is unit-tested on any host.

use chrono::{DateTime, Utc};

use crate::error::BlueyError;
use crate::types::{
    AccountStatus, AiProviderConfig, AiProviderKind, CatalogModel, CatalogSource, ModelAssignment,
    ModelRole, ModelRoleAssignments, ProviderAccount, ProviderAuthMethod, ProviderModelCatalog,
    UnavailableReason, ANTIGRAVITY_PROVIDER_ID, CHATGPT_PROVIDER_ID, CLAUDE_PROVIDER_ID,
};

/// Endpoint / curated catalogs are refreshed after this long.
pub const CATALOG_TTL_SECS: i64 = 10 * 60;
/// Probed catalogs (Antigravity) are expensive; keep them for a day.
pub const PROBED_CATALOG_TTL_SECS: i64 = 24 * 60 * 60;

/// Stable error codes of the accounts layer (`BlueyError::account(code, …)`
/// produces `account.<code>`); `src/lib/errors/present.ts` maps them to copy.
pub mod codes {
    pub const NEEDS_REAUTH: &str = "account.needs_reauth";
    pub const FINGERPRINT_DRIFT: &str = "account.fingerprint_drift";
    pub const EXTRA_USAGE_BLOCKED: &str = "account.extra_usage_blocked";
    pub const POLICY_BLOCKED: &str = "account.policy_blocked";
    pub const RATE_LIMITED: &str = "account.rate_limited";
    pub const CATALOG_UNAVAILABLE: &str = "account.catalog_unavailable";
    pub const UNAVAILABLE: &str = "account.unavailable";
    pub const PROVIDER_PENDING: &str = "account.provider_pending";
    pub const DISABLED: &str = "account.disabled";
    pub const NOT_FOUND: &str = "account.not_found";
    pub const UNKNOWN_PROVIDER: &str = "account.unknown_provider";
    pub const FLOW_PENDING: &str = "account.flow_pending";
    pub const NO_PENDING_FLOW: &str = "account.no_pending_flow";
    pub const DENIED: &str = "account.denied";
    pub const NOT_CONNECTED: &str = "account.not_connected";
}

/// The three subscription providers, in UI order.
pub const SUBSCRIPTION_PROVIDER_IDS: [&str; 3] = [
    CHATGPT_PROVIDER_ID,
    CLAUDE_PROVIDER_ID,
    ANTIGRAVITY_PROVIDER_ID,
];

/// Whether a provider kind is served by a subscription account rather than an API key.
pub fn is_subscription_kind(kind: AiProviderKind) -> bool {
    matches!(
        kind,
        AiProviderKind::ChatgptCodex
            | AiProviderKind::ClaudeSubscription
            | AiProviderKind::AntigravityGoogle
    )
}

/// The kind behind a reserved subscription provider id.
pub fn kind_for_provider_id(provider_id: &str) -> Option<AiProviderKind> {
    match provider_id {
        CHATGPT_PROVIDER_ID => Some(AiProviderKind::ChatgptCodex),
        CLAUDE_PROVIDER_ID => Some(AiProviderKind::ClaudeSubscription),
        ANTIGRAVITY_PROVIDER_ID => Some(AiProviderKind::AntigravityGoogle),
        _ => None,
    }
}

/// The reserved provider id of a subscription kind.
pub fn provider_id_for_kind(kind: AiProviderKind) -> Option<&'static str> {
    match kind {
        AiProviderKind::ChatgptCodex => Some(CHATGPT_PROVIDER_ID),
        AiProviderKind::ClaudeSubscription => Some(CLAUDE_PROVIDER_ID),
        AiProviderKind::AntigravityGoogle => Some(ANTIGRAVITY_PROVIDER_ID),
        _ => None,
    }
}

/// Display name of a subscription provider (`None` for anything else).
pub fn provider_display_name(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        CHATGPT_PROVIDER_ID => Some("ChatGPT"),
        CLAUDE_PROVIDER_ID => Some("Claude"),
        ANTIGRAVITY_PROVIDER_ID => Some("Google AI"),
        _ => None,
    }
}

/// Whether the router may send requests through this account right now.
pub fn is_usable(account: &ProviderAccount, now: DateTime<Utc>) -> bool {
    match &account.status {
        AccountStatus::Connected => true,
        AccountStatus::RateLimited { until, .. } => !rate_limit_active(until, now),
        _ => false,
    }
}

/// A rate-limit window is active until its reset time; an unparsable reset
/// time counts as active (the provider said stop — believe it).
pub fn rate_limit_active(until: &str, now: DateTime<Utc>) -> bool {
    match DateTime::parse_from_rfc3339(until) {
        Ok(reset) => reset.with_timezone(&Utc) > now,
        Err(_) => true,
    }
}

/// The status an account moves to when a request through it fails with
/// `error`. `None` means the error is not about the account (a plain provider
/// error) and the status stays as it is.
pub fn status_after_error(error: &BlueyError) -> Option<AccountStatus> {
    let detail = || Some(error.message.clone()).filter(|m| !m.is_empty());
    match error.code.as_str() {
        codes::NEEDS_REAUTH => Some(AccountStatus::NeedsReauth),
        codes::FINGERPRINT_DRIFT => Some(AccountStatus::Unavailable {
            reason: UnavailableReason::FingerprintDrift,
            detail: detail(),
        }),
        codes::EXTRA_USAGE_BLOCKED => Some(AccountStatus::Unavailable {
            reason: UnavailableReason::ExtraUsageBilling,
            detail: detail(),
        }),
        codes::POLICY_BLOCKED => Some(AccountStatus::Unavailable {
            reason: UnavailableReason::PolicyBlocked,
            detail: detail(),
        }),
        codes::CATALOG_UNAVAILABLE => Some(AccountStatus::Unavailable {
            reason: UnavailableReason::CatalogUnavailable,
            detail: detail(),
        }),
        codes::PROVIDER_PENDING => Some(AccountStatus::Unavailable {
            reason: UnavailableReason::ProviderPending,
            detail: detail(),
        }),
        codes::UNAVAILABLE => Some(AccountStatus::Unavailable {
            reason: UnavailableReason::Other,
            detail: detail(),
        }),
        codes::RATE_LIMITED => {
            let details = error.details.as_ref();
            let until = details
                .and_then(|d| d.get("until"))
                .and_then(|v| v.as_str())
                .map(str::to_string)?;
            let window = details
                .and_then(|d| d.get("window"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            Some(AccountStatus::RateLimited { until, window })
        }
        _ => None,
    }
}

/// Whether a catalog is fresh enough to serve without refetching.
pub fn catalog_is_fresh(catalog: &ProviderModelCatalog, now: DateTime<Utc>) -> bool {
    let ttl = match catalog.source {
        CatalogSource::Probed => PROBED_CATALOG_TTL_SECS,
        _ => CATALOG_TTL_SECS,
    };
    match DateTime::parse_from_rfc3339(&catalog.fetched_at) {
        Ok(fetched) => {
            now.signed_duration_since(fetched.with_timezone(&Utc))
                .num_seconds()
                < ttl
        }
        Err(_) => false,
    }
}

/// Recommended model per role from a catalog: the first model that lists the
/// role in `suggested_roles`. Roles no model claims are absent — the router
/// falls back to the API-key provider for those (§3.7 of the brief).
pub fn preset_assignments(catalog: &ProviderModelCatalog) -> Vec<(ModelRole, &CatalogModel)> {
    ModelRole::ALL
        .iter()
        .filter_map(|role| {
            catalog
                .models
                .iter()
                .find(|model| model.suggested_roles.contains(role))
                .map(|model| (*role, model))
        })
        .collect()
}

/// The provider the router sees for an account (§3.6): enabled while the
/// accounts layer is on, "has a key" while the account is usable — connected,
/// or past its rate-limit window. Not `Connected` = keyless = the router's
/// existing fallback chain reaches the API-key providers.
pub fn provider_config(
    account: &ProviderAccount,
    now: DateTime<Utc>,
    enabled: bool,
) -> AiProviderConfig {
    AiProviderConfig {
        id: account.provider_id.clone(),
        kind: account.kind,
        name: provider_display_name(&account.provider_id)
            .unwrap_or(account.provider_id.as_str())
            .to_string(),
        base_url: String::new(),
        api_version: None,
        deployments: None,
        enabled,
        has_api_key: enabled && is_usable(account, now),
        auth_method: ProviderAuthMethod::OauthSubscription,
    }
}

/// Point roles at the catalog's suggested models (§3.7). `overwrite` re-points
/// every role the catalog suggests; otherwise only unassigned roles and roles
/// that point at this provider with a model the catalog no longer lists.
/// Never assigns a model the catalog did not return. Returns the roles changed.
pub fn apply_catalog_presets(
    assignments: &mut ModelRoleAssignments,
    catalog: &ProviderModelCatalog,
    overwrite: bool,
) -> Vec<ModelRole> {
    let mut changed = Vec::new();
    for (role, model) in preset_assignments(catalog) {
        let current = assignments.get(role);
        let stale = current.is_some_and(|assignment| {
            assignment.provider_id == catalog.provider_id
                && !catalog.models.iter().any(|m| m.id == assignment.model)
        });
        if !(overwrite || current.is_none() || stale) {
            continue;
        }
        let next = ModelAssignment {
            provider_id: catalog.provider_id.clone(),
            model: model.id.clone(),
        };
        if current != Some(&next) {
            assignments.set(role, Some(next));
            changed.push(role);
        }
    }
    changed
}

/// Insert or replace an account (by `account_id`), keeping the list order.
pub fn upsert(accounts: &mut Vec<ProviderAccount>, account: ProviderAccount) {
    match accounts
        .iter_mut()
        .find(|existing| existing.account_id == account.account_id)
    {
        Some(existing) => *existing = account,
        None => accounts.push(account),
    }
}

/// A `Connecting` status must not survive a restart: the browser round-trip is
/// gone with the process. Everything else is kept.
pub fn normalise_after_restart(account: &mut ProviderAccount) {
    if account.status.is_connecting() {
        account.status = AccountStatus::Disconnected;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AccountIdentity, ModelCapabilities, ProviderAuthMethod};

    fn account(status: AccountStatus) -> ProviderAccount {
        ProviderAccount {
            account_id: "chatgpt".into(),
            provider_id: "chatgpt".into(),
            kind: AiProviderKind::ChatgptCodex,
            method: ProviderAuthMethod::OauthSubscription,
            status,
            identity: Some(AccountIdentity::default()),
            connected_at: None,
            expires_at: None,
            catalog_fetched_at: None,
            fingerprint_version: None,
            fingerprint_captured_on: None,
        }
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn provider_ids_kinds_and_names_agree() {
        for id in SUBSCRIPTION_PROVIDER_IDS {
            let kind = kind_for_provider_id(id).expect("reserved id has a kind");
            assert!(is_subscription_kind(kind));
            assert_eq!(provider_id_for_kind(kind), Some(id));
            assert!(provider_display_name(id).is_some());
        }
        assert!(!is_subscription_kind(AiProviderKind::GoogleGemini));
        assert_eq!(kind_for_provider_id("gemini"), None);
        assert_eq!(provider_id_for_kind(AiProviderKind::Anthropic), None);
        assert_eq!(provider_display_name("openai"), None);
    }

    #[test]
    fn only_connected_and_expired_rate_limits_are_usable() {
        let now = at("2026-09-11T12:00:00Z");
        assert!(is_usable(&account(AccountStatus::Connected), now));
        assert!(!is_usable(&account(AccountStatus::Disconnected), now));
        assert!(!is_usable(&account(AccountStatus::NeedsReauth), now));
        assert!(!is_usable(
            &account(AccountStatus::Unavailable {
                reason: UnavailableReason::FingerprintDrift,
                detail: None
            }),
            now
        ));
        assert!(!is_usable(
            &account(AccountStatus::RateLimited {
                until: "2026-09-11T14:32:00Z".into(),
                window: Some("5h".into())
            }),
            now
        ));
        assert!(is_usable(
            &account(AccountStatus::RateLimited {
                until: "2026-09-11T11:59:00Z".into(),
                window: None
            }),
            now
        ));
        assert!(
            rate_limit_active("not a time", now),
            "unparsable reset counts as active"
        );
    }

    #[test]
    fn provider_errors_move_the_account_to_the_matching_status() {
        let drift = BlueyError::account(
            "fingerprint_drift",
            "Claude stopped recognising Bluey as Claude Code",
        );
        assert_eq!(
            status_after_error(&drift),
            Some(AccountStatus::Unavailable {
                reason: UnavailableReason::FingerprintDrift,
                detail: Some("Claude stopped recognising Bluey as Claude Code".into()),
            })
        );
        assert_eq!(
            status_after_error(&BlueyError::account("needs_reauth", "expired")),
            Some(AccountStatus::NeedsReauth)
        );
        assert_eq!(
            status_after_error(&BlueyError::account("extra_usage_blocked", "")),
            Some(AccountStatus::Unavailable {
                reason: UnavailableReason::ExtraUsageBilling,
                detail: None,
            })
        );
        let limited = BlueyError::account("rate_limited", "5h window")
            .with_details(serde_json::json!({ "until": "2026-09-11T14:32:00Z", "window": "5h" }));
        assert_eq!(
            status_after_error(&limited),
            Some(AccountStatus::RateLimited {
                until: "2026-09-11T14:32:00Z".into(),
                window: Some("5h".into()),
            })
        );
        // A rate limit without a reset time cannot be scheduled — leave the status alone.
        assert_eq!(
            status_after_error(&BlueyError::account("rate_limited", "no reset")),
            None
        );
        assert_eq!(
            status_after_error(&BlueyError::network("http_5xx", "down")),
            None
        );
    }

    fn catalog(source: CatalogSource, fetched_at: &str) -> ProviderModelCatalog {
        ProviderModelCatalog {
            account_id: "chatgpt".into(),
            provider_id: "chatgpt".into(),
            fetched_at: fetched_at.into(),
            source,
            models: vec![
                CatalogModel {
                    id: "gpt-6-astra".into(),
                    label: "GPT-6 Astra".into(),
                    capabilities: ModelCapabilities {
                        vision: true,
                        tools: true,
                        reasoning_levels: vec!["low".into(), "medium".into(), "high".into()],
                        streaming: true,
                        context_window: Some(272_000),
                    },
                    quota_pool: None,
                    suggested_roles: vec![
                        ModelRole::Default,
                        ModelRole::Vision,
                        ModelRole::Reasoning,
                    ],
                },
                CatalogModel {
                    id: "gpt-5.6-luna".into(),
                    label: "GPT-5.6 Luna".into(),
                    capabilities: ModelCapabilities::default(),
                    quota_pool: None,
                    suggested_roles: vec![ModelRole::Fast, ModelRole::Default],
                },
            ],
        }
    }

    #[test]
    fn catalog_freshness_depends_on_the_source() {
        let now = at("2026-09-11T12:00:00Z");
        let endpoint = catalog(CatalogSource::Endpoint, "2026-09-11T11:55:00Z");
        assert!(catalog_is_fresh(&endpoint, now));
        let stale = catalog(CatalogSource::Endpoint, "2026-09-11T11:40:00Z");
        assert!(!catalog_is_fresh(&stale, now));
        let probed = catalog(CatalogSource::Probed, "2026-09-10T13:00:00Z");
        assert!(
            catalog_is_fresh(&probed, now),
            "probed catalogs live for a day"
        );
        let old_probe = catalog(CatalogSource::Probed, "2026-09-10T11:00:00Z");
        assert!(!catalog_is_fresh(&old_probe, now));
        let broken = catalog(CatalogSource::Fixture, "yesterday");
        assert!(!catalog_is_fresh(&broken, now));
    }

    #[test]
    fn presets_take_the_first_model_suggested_for_each_role() {
        let catalog = catalog(CatalogSource::Endpoint, "2026-09-11T11:55:00Z");
        let presets = preset_assignments(&catalog);
        let pick = |role: ModelRole| {
            presets
                .iter()
                .find(|(r, _)| *r == role)
                .map(|(_, m)| m.id.as_str())
        };
        assert_eq!(pick(ModelRole::Default), Some("gpt-6-astra"));
        assert_eq!(pick(ModelRole::Fast), Some("gpt-5.6-luna"));
        assert_eq!(pick(ModelRole::Reasoning), Some("gpt-6-astra"));
        assert_eq!(
            pick(ModelRole::Transcription),
            None,
            "no model claims transcription"
        );
        assert_eq!(pick(ModelRole::Embedding), None);
    }

    #[test]
    fn the_router_sees_a_usable_account_as_a_keyed_provider() {
        let now = at("2026-09-11T12:00:00Z");
        let connected = provider_config(&account(AccountStatus::Connected), now, true);
        assert_eq!(connected.id, "chatgpt");
        assert_eq!(connected.kind, AiProviderKind::ChatgptCodex);
        assert_eq!(connected.name, "ChatGPT");
        assert_eq!(connected.auth_method, ProviderAuthMethod::OauthSubscription);
        assert!(connected.enabled && connected.has_api_key);
        let expired = provider_config(&account(AccountStatus::NeedsReauth), now, true);
        assert!(
            expired.enabled && !expired.has_api_key,
            "keyless → fallback chain"
        );
        let limited = provider_config(
            &account(AccountStatus::RateLimited {
                until: "2026-09-11T14:32:00Z".into(),
                window: None,
            }),
            now,
            true,
        );
        assert!(!limited.has_api_key);
        let off = provider_config(&account(AccountStatus::Connected), now, false);
        assert!(!off.enabled && !off.has_api_key);
    }

    #[test]
    fn catalog_presets_fill_unassigned_and_stale_roles_unless_overwriting() {
        let catalog = catalog(CatalogSource::Endpoint, "2026-09-11T11:55:00Z");
        let mut assignments = ModelRoleAssignments {
            default: Some(ModelAssignment {
                provider_id: "gemini".into(),
                model: "gemini-3.8-flash".into(),
            }),
            reasoning: Some(ModelAssignment {
                provider_id: "chatgpt".into(),
                model: "gpt-retired".into(),
            }),
            ..ModelRoleAssignments::default()
        };
        let mut changed = apply_catalog_presets(&mut assignments, &catalog, false);
        changed.sort_by_key(|r| format!("{r:?}"));
        assert_eq!(
            changed,
            vec![ModelRole::Fast, ModelRole::Reasoning, ModelRole::Vision]
        );
        assert_eq!(
            assignments.default.as_ref().unwrap().provider_id,
            "gemini",
            "kept"
        );
        assert_eq!(
            assignments.reasoning.as_ref().unwrap().model,
            "gpt-6-astra",
            "stale → replaced"
        );
        assert_eq!(assignments.fast.as_ref().unwrap().model, "gpt-5.6-luna");
        assert!(assignments.embedding.is_none(), "no model claims embedding");
        let again = apply_catalog_presets(&mut assignments, &catalog, false);
        assert!(again.is_empty(), "idempotent");
        let overwritten = apply_catalog_presets(&mut assignments, &catalog, true);
        assert_eq!(overwritten, vec![ModelRole::Default]);
        assert_eq!(assignments.default.as_ref().unwrap().provider_id, "chatgpt");
    }

    #[test]
    fn upsert_replaces_in_place_and_restarts_drop_pending_flows() {
        let mut accounts = vec![account(AccountStatus::Disconnected)];
        let mut claude = account(AccountStatus::Connected);
        claude.account_id = "claude".into();
        claude.provider_id = "claude".into();
        upsert(&mut accounts, claude.clone());
        upsert(&mut accounts, account(AccountStatus::Connected));
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].account_id, "chatgpt");
        assert!(accounts[0].status.is_connected());
        assert_eq!(accounts[1], claude);

        let mut pending = account(AccountStatus::Connecting {
            flow: crate::types::ConnectFlow {
                kind: crate::types::ConnectFlowKind::Browser,
                url: Some("https://auth.openai.com/oauth/authorize?x".into()),
                user_code: None,
                verification_url: None,
                expires_at: "2026-09-11T12:10:00Z".into(),
            },
        });
        normalise_after_restart(&mut pending);
        assert_eq!(pending.status, AccountStatus::Disconnected);
        let mut connected = account(AccountStatus::Connected);
        normalise_after_restart(&mut connected);
        assert!(connected.status.is_connected());
    }
}
