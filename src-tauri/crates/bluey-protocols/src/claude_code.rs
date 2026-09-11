//! Claude Pro / Max through claude.ai OAuth in the Claude Code wire format
//! (ADR 0009 §4b) — the pure half.
//!
//! The OAuth constants and bodies (PKCE loopback on Claude Code's port 54545,
//! any free port, then the pasted `code#state`), the profile endpoint → identity,
//! Claude Code's credential store parsing (read-only import), the model catalog
//! (endpoint or the curated list), the thinking / effort knobs, [`ClaudeCodeShaper`]
//! — the request fingerprint built from [`crate::fingerprints::claude_code`] (a
//! test diffs it against the documented capture) — and the error mapper whose
//! centrepiece is the **extra-usage guard**: a response saying the request is
//! billed outside the plan stops the account, never a retry.
//!
//! Every wire fact here is a row of `docs/PROVIDER_ACCOUNTS.md › Claude`.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use bluey_core::error::RecoveryAction;
use bluey_core::types::{
    AccountIdentity, BilledTo, CatalogModel, LatencyBudget, ModelCapabilities, ModelRole,
    ReasoningLevel, UnavailableReason, CLAUDE_PROVIDER_ID,
};
use bluey_core::{BlueyError, BlueyErrorKind};

use crate::codex::iso_from_unix;
use crate::fingerprints::claude_code as fp;
use crate::request_shaper::{
    FingerprintInfo, ProviderHttpRequest, RequestShaper, ShapeContext, ShapeError,
};

// ─────────────────────────────────────────────────────────────────────────────
// OAuth
// ─────────────────────────────────────────────────────────────────────────────

pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
/// JSON token endpoint (the `console.anthropic.com` one sits behind a challenge).
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
/// The redirect whose page shows `code#state` for pasting.
pub const MANUAL_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
/// The six scopes of the real 2.1.267 authorize URL.
pub const SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
/// Claude Code's fixed loopback port; any free port is accepted too (client-chosen).
pub const LOOPBACK_PORT: u16 = 54545;
pub const CALLBACK_PATH: &str = "/callback";
pub const OAUTH_BETA: &str = "oauth-2025-04-20";
/// The token endpoint sees axios: these three headers ride on every token call.
pub const TOKEN_HEADERS: [(&str, &str); 3] = [
    ("content-type", "application/json"),
    ("accept", "application/json, text/plain, */*"),
    ("user-agent", "axios/1.15.2"),
];
pub const BROWSER_FLOW_SECS: u64 = 10 * 60;

pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CALLBACK_PATH}")
}

/// The authorization URL, parameters in the reference order (`anthropic_auth.go:324-333`).
pub fn authorize_url(redirect_uri: &str, code_challenge: &str, state: &str) -> String {
    let mut url = url::Url::parse(AUTHORIZE_URL).expect("constant URL parses");
    url.query_pairs_mut()
        .append_pair("code", "true")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
    url.to_string()
}

/// JSON body of the code exchange (`state` included, as the references send it).
pub fn token_exchange_body(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    state: &str,
) -> Value {
    json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": redirect_uri,
        "client_id": CLIENT_ID,
        "code_verifier": code_verifier,
        "state": state,
    })
}

pub fn refresh_body(refresh_token: &str) -> Value {
    json!({
        "client_id": CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "scope": SCOPES,
    })
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// What a token response says beyond the tokens (presence is VERIFY — refetch the profile anyway).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenExtras {
    pub account_uuid: Option<String>,
    pub email: Option<String>,
    pub organization_uuid: Option<String>,
    pub organization_name: Option<String>,
}

pub fn parse_token_extras(json: &str) -> TokenExtras {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return TokenExtras::default();
    };
    let account = value.get("account");
    let organization = value.get("organization");
    TokenExtras {
        account_uuid: account.and_then(|a| str_field(a, "uuid")),
        email: account
            .and_then(|a| str_field(a, "email_address").or_else(|| str_field(a, "email"))),
        organization_uuid: organization.and_then(|o| str_field(o, "uuid")),
        organization_name: organization.and_then(|o| str_field(o, "name")),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Profile → identity
// ─────────────────────────────────────────────────────────────────────────────

/// `GET /api/oauth/profile`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Profile {
    pub account_uuid: Option<String>,
    pub email: Option<String>,
    pub has_pro: bool,
    pub has_max: bool,
    pub organization_uuid: Option<String>,
    pub organization_name: Option<String>,
    /// `default_claude_pro`, `default_claude_max_5x`, `default_claude_max_20x`.
    pub rate_limit_tier: Option<String>,
    /// `claude_pro`, `claude_max`, `claude_team`, `claude_enterprise`.
    pub organization_type: Option<String>,
    pub subscription_status: Option<String>,
}

pub fn parse_profile(json: &str) -> Result<Profile, serde_json::Error> {
    let value: Value = serde_json::from_str(json)?;
    let account = value.get("account");
    let organization = value.get("organization");
    let flag = |key: &str| {
        account
            .and_then(|a| a.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    Ok(Profile {
        account_uuid: account.and_then(|a| str_field(a, "uuid")),
        email: account
            .and_then(|a| str_field(a, "email").or_else(|| str_field(a, "email_address"))),
        has_pro: flag("has_claude_pro"),
        has_max: flag("has_claude_max"),
        organization_uuid: organization.and_then(|o| str_field(o, "uuid")),
        organization_name: organization.and_then(|o| str_field(o, "name")),
        rate_limit_tier: organization.and_then(|o| str_field(o, "rate_limit_tier")),
        organization_type: organization.and_then(|o| str_field(o, "organization_type")),
        subscription_status: organization.and_then(|o| str_field(o, "subscription_status")),
    })
}

/// Human label for the plan.
pub fn plan_label(profile: &Profile) -> String {
    match profile.rate_limit_tier.as_deref() {
        Some("default_claude_pro") => return "Claude Pro".into(),
        Some("default_claude_max_5x") => return "Claude Max 5×".into(),
        Some("default_claude_max_20x") => return "Claude Max 20×".into(),
        _ => {}
    }
    match profile.organization_type.as_deref() {
        Some("claude_team") => "Claude Team".into(),
        Some("claude_enterprise") => "Claude Enterprise".into(),
        Some("claude_max") => "Claude Max".into(),
        Some("claude_pro") => "Claude Pro".into(),
        _ if profile.has_max => "Claude Max".into(),
        _ if profile.has_pro => "Claude Pro".into(),
        _ => "Claude".into(),
    }
}

/// The wire identity the WebView sees.
pub fn account_identity(profile: &Profile) -> AccountIdentity {
    AccountIdentity {
        email: profile.email.clone(),
        display_name: profile.organization_name.clone(),
        plan_tier: profile.rate_limit_tier.clone(),
        plan_label: Some(plan_label(profile)),
        account_id: profile.account_uuid.clone(),
        project_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Claude Code's credential store (read-only import)
// ─────────────────────────────────────────────────────────────────────────────

/// macOS Keychain generic password service of Claude Code's sign-in.
pub const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// Keychain account names to try, in order (the macOS username, then the
/// values older versions wrote).
pub fn keychain_accounts(username: &str) -> Vec<String> {
    let mut accounts = Vec::new();
    for candidate in [username, "default", "unknown"] {
        if !candidate.is_empty() && !accounts.iter().any(|a| a == candidate) {
            accounts.push(candidate.to_string());
        }
    }
    accounts
}

/// `$CLAUDE_CONFIG_DIR/.credentials.json`, else `~/.claude/.credentials.json`.
pub fn credentials_file(home: &Path, config_dir: Option<&str>) -> PathBuf {
    match config_dir.map(str::trim).filter(|s| !s.is_empty()) {
        Some(dir) => PathBuf::from(dir).join(".credentials.json"),
        None => home.join(".claude").join(".credentials.json"),
    }
}

/// `~/.claude.json` — carries `oauthAccount.accountUuid` / `emailAddress`.
pub fn claude_json_path(home: &Path) -> PathBuf {
    home.join(".claude.json")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedCredentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds (the store keeps milliseconds).
    pub expires_at: Option<u64>,
    pub scopes: Vec<String>,
    /// `pro` | `max`.
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Malformed(String),
    NoTokens,
}

/// `{"claudeAiOauth": {"accessToken", "refreshToken", "expiresAt": <ms>, "scopes", "subscriptionType", "rateLimitTier"}}`.
pub fn parse_credentials(text: &str) -> Result<ImportedCredentials, ImportError> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| ImportError::Malformed(e.to_string()))?;
    let oauth = value.get("claudeAiOauth").ok_or(ImportError::NoTokens)?;
    let access_token = str_field(oauth, "accessToken").ok_or(ImportError::NoTokens)?;
    Ok(ImportedCredentials {
        access_token,
        refresh_token: str_field(oauth, "refreshToken"),
        expires_at: oauth
            .get("expiresAt")
            .and_then(Value::as_u64)
            .map(|ms| ms / 1000),
        scopes: oauth
            .get("scopes")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        subscription_type: str_field(oauth, "subscriptionType"),
        rate_limit_tier: str_field(oauth, "rateLimitTier"),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OauthAccountInfo {
    pub account_uuid: Option<String>,
    pub email: Option<String>,
    pub organization_uuid: Option<String>,
}

pub fn parse_claude_json(text: &str) -> OauthAccountInfo {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return OauthAccountInfo::default();
    };
    let Some(account) = value.get("oauthAccount") else {
        return OauthAccountInfo::default();
    };
    OauthAccountInfo {
        account_uuid: str_field(account, "accountUuid"),
        email: str_field(account, "emailAddress"),
        organization_uuid: str_field(account, "organizationUuid"),
    }
}

/// An identity from the local store alone (no network): the profile call refines it.
pub fn identity_from_import(
    credentials: &ImportedCredentials,
    info: &OauthAccountInfo,
) -> AccountIdentity {
    let profile = Profile {
        account_uuid: info.account_uuid.clone(),
        email: info.email.clone(),
        has_pro: credentials.subscription_type.as_deref() == Some("pro"),
        has_max: credentials.subscription_type.as_deref() == Some("max"),
        organization_uuid: info.organization_uuid.clone(),
        organization_name: None,
        rate_limit_tier: credentials.rate_limit_tier.clone(),
        organization_type: None,
        subscription_status: None,
    };
    account_identity(&profile)
}

// ─────────────────────────────────────────────────────────────────────────────
// Backend + catalog
// ─────────────────────────────────────────────────────────────────────────────

/// `POST …/v1/messages?beta=true`.
pub fn messages_url(base_url: &str) -> String {
    format!("{}/v1/messages?beta=true", base_url.trim_end_matches('/'))
}

pub fn models_url(base_url: &str) -> String {
    format!("{}/v1/models?limit=100", base_url.trim_end_matches('/'))
}

/// The ids the doc lists (2026-09-11), when `/v1/models` refuses an OAuth token.
pub const CURATED_MODELS: &[(&str, &str)] = &[
    ("claude-opus-5", "Claude Opus 5"),
    ("claude-sonnet-5", "Claude Sonnet 5"),
    ("claude-fable-5-1", "Claude Fable 5.1"),
    ("claude-haiku-4-5-20251001", "Claude Haiku 4.5"),
    ("claude-opus-4-8", "Claude Opus 4.8"),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Opus,
    Sonnet,
    Haiku,
    Fable,
    Other,
}

pub fn family(model: &str) -> Family {
    let lower = model.to_ascii_lowercase();
    if lower.contains("haiku") {
        Family::Haiku
    } else if lower.contains("fable") {
        Family::Fable
    } else if lower.contains("opus") {
        Family::Opus
    } else if lower.contains("sonnet") {
        Family::Sonnet
    } else {
        Family::Other
    }
}

/// Effort (`output_config.effort`) is for effort-capable models with thinking on — never Haiku.
pub fn is_effort_capable(model: &str) -> bool {
    family(model) != Family::Haiku
}

/// The `max_tokens` Claude Code sends per model line.
pub fn max_tokens_for(model: &str) -> u32 {
    let lower = model.to_ascii_lowercase();
    if lower.contains("opus-5") || lower.contains("sonnet-5") || lower.contains("fable-5") {
        128_000
    } else {
        64_000
    }
}

pub const EFFORT_LEVELS: [&str; 4] = ["low", "medium", "high", "max"];

/// `(id, display name)` pairs → catalog models with the roles of `docs/PROVIDER_ACCOUNTS.md`:
/// `default`/`vision` = the newest Sonnet, `reasoning`/`research` = the newest Opus,
/// `fast` = the newest Haiku.
pub fn catalog_models(entries: &[(String, String)]) -> Vec<CatalogModel> {
    let pick = |wanted: Family| entries.iter().position(|(id, _)| family(id) == wanted);
    let sonnet = pick(Family::Sonnet)
        .or_else(|| pick(Family::Fable))
        .or((!entries.is_empty()).then_some(0));
    let opus = pick(Family::Opus).or(sonnet);
    let haiku = pick(Family::Haiku).or(sonnet);
    entries
        .iter()
        .enumerate()
        .map(|(i, (id, label))| {
            let mut suggested_roles = Vec::new();
            if Some(i) == sonnet {
                suggested_roles.push(ModelRole::Default);
            }
            if Some(i) == haiku {
                suggested_roles.push(ModelRole::Fast);
            }
            if Some(i) == opus {
                suggested_roles.push(ModelRole::Reasoning);
                suggested_roles.push(ModelRole::Research);
            }
            if Some(i) == sonnet {
                suggested_roles.push(ModelRole::Vision);
            }
            CatalogModel {
                id: id.clone(),
                label: if label.is_empty() {
                    id.clone()
                } else {
                    label.clone()
                },
                capabilities: ModelCapabilities {
                    vision: true,
                    tools: true,
                    reasoning_levels: if is_effort_capable(id) {
                        EFFORT_LEVELS.iter().map(|s| s.to_string()).collect()
                    } else {
                        Vec::new()
                    },
                    streaming: true,
                    context_window: Some(200_000),
                },
                quota_pool: None,
                suggested_roles,
            }
        })
        .collect()
}

/// `{"data":[{"id","display_name"}], "has_more"}` → catalog models, newest first as listed.
pub fn catalog_from_models_response(json: &str) -> Result<Vec<CatalogModel>, serde_json::Error> {
    let value: Value = serde_json::from_str(json)?;
    let entries: Vec<(String, String)> = value
        .get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let id = str_field(row, "id")?;
                    let label = str_field(row, "display_name").unwrap_or_else(|| id.clone());
                    Some((id, label))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(catalog_models(&entries))
}

pub fn curated_catalog() -> Vec<CatalogModel> {
    let entries: Vec<(String, String)> = CURATED_MODELS
        .iter()
        .map(|(id, label)| (id.to_string(), label.to_string()))
        .collect();
    catalog_models(&entries)
}

// ─────────────────────────────────────────────────────────────────────────────
// Thinking / effort
// ─────────────────────────────────────────────────────────────────────────────

/// `output_config.effort` from Bluey's reasoning level and latency budget.
pub fn effort_for(reasoning: ReasoningLevel, latency: LatencyBudget) -> Option<&'static str> {
    match reasoning {
        ReasoningLevel::None => None,
        ReasoningLevel::Light => Some(match latency {
            LatencyBudget::UltraFast | LatencyBudget::Fast => "low",
            _ => "medium",
        }),
        ReasoningLevel::Deep => Some(match latency {
            LatencyBudget::Deep => "max",
            _ => "high",
        }),
    }
}

/// Adaptive thinking the way the CLI sends it, when Bluey asked for reasoning
/// and the model takes it: `thinking`, `output_config.effort`,
/// `context_management` — and no sampling knob next to thinking.
pub fn apply_thinking(
    body: &mut Value,
    model: &str,
    reasoning: ReasoningLevel,
    latency: LatencyBudget,
) {
    let Some(map) = body.as_object_mut() else {
        return;
    };
    let effort = effort_for(reasoning, latency).filter(|_| is_effort_capable(model));
    match effort {
        Some(effort) => {
            map.insert(
                "thinking".into(),
                json!({ "type": "adaptive", "display": "summarized" }),
            );
            let output_config = map
                .entry("output_config")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(config) = output_config.as_object_mut() {
                config.insert("effort".into(), Value::String(effort.into()));
            }
            map.insert(
                "context_management".into(),
                json!({ "edits": [ { "type": "clear_thinking_20251015", "keep": "all" } ] }),
            );
            map.remove("temperature");
            map.remove("top_p");
            map.remove("top_k");
        }
        None => {
            map.remove("thinking");
            map.remove("context_management");
            if let Some(config) = map.get_mut("output_config").and_then(Value::as_object_mut) {
                config.remove("effort");
            }
            if map
                .get("output_config")
                .and_then(Value::as_object)
                .is_some_and(|c| c.is_empty())
            {
                map.remove("output_config");
            }
        }
    }
}

/// `max_tokens` as the CLI sends it unless the caller asked for a specific cap.
pub fn normalise_max_tokens(body: &mut Value, model: &str, explicit: Option<u32>) {
    if let Some(map) = body.as_object_mut() {
        let value = explicit.unwrap_or_else(|| max_tokens_for(model));
        map.insert("max_tokens".into(), json!(value));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fingerprint
// ─────────────────────────────────────────────────────────────────────────────

const VERSION_SUFFIX_SEED: &str = "59cf53e54c78";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The `.<3hex>` build suffix of `cc_version`:
/// `SHA256(seed + text[4] + text[7] + text[20] + version)[:3]`, missing characters
/// padded with `"0"` (three references agree).
pub fn version_suffix(first_user_text: &str, version: &str) -> String {
    let chars: Vec<char> = first_user_text.chars().collect();
    let pick = |i: usize| {
        chars
            .get(i)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "0".to_string())
    };
    let input = format!(
        "{VERSION_SUFFIX_SEED}{}{}{}{version}",
        pick(4),
        pick(7),
        pick(20)
    );
    hex(&Sha256::digest(input.as_bytes()))[..3].to_string()
}

/// `cch`: `SHA256(first user text)[:5]` — the griffinmartin / hermes reading of a
/// disputed value (CLIProxyAPI hashes the normalised body instead; the §4b.1
/// capture settles it). A wrong `cch` alone has not been seen to flip billing.
pub fn cch(first_user_text: &str) -> String {
    hex(&Sha256::digest(first_user_text.as_bytes()))[..5].to_string()
}

pub fn billing_header(first_user_text: &str) -> String {
    format!(
        "x-anthropic-billing-header: cc_version={}.{}; cc_entrypoint=cli; cch={};",
        fp::CLIENT_VERSION,
        version_suffix(first_user_text, fp::CLIENT_VERSION),
        cch(first_user_text)
    )
}

/// `metadata.user_id` — a JSON string, keys in this order.
pub fn user_id(device_id: &str, account_uuid: &str, session_id: &str) -> String {
    format!(
        r#"{{"device_id":"{device_id}","account_uuid":"{account_uuid}","session_id":"{session_id}"}}"#
    )
}

/// Bluey's own instructions travel the way Claude Code carries CLAUDE.md content:
/// a `<system-reminder>` block at the top of the first user turn — never in `system[]`.
pub fn system_reminder(text: &str) -> String {
    format!("<system-reminder>\n{}\n</system-reminder>", text.trim())
}

/// Betas the OAuth transport always carries (the CLI's always-on set plus the two
/// OAuth-conditional ones), then per body: `effort` and `structured-outputs`.
pub fn betas_for(body: &Value) -> Vec<&'static str> {
    let mut betas: Vec<&'static str> = fp::BETAS_ALWAYS.to_vec();
    betas.push("fallback-credit-2026-06-01");
    betas.push("extended-cache-ttl-2025-04-11");
    let output_config = body.get("output_config");
    if output_config.and_then(|c| c.get("effort")).is_some() {
        betas.push("effort-2025-11-24");
    }
    if output_config.and_then(|c| c.get("format")).is_some() {
        betas.push("structured-outputs-2025-12-15");
    }
    betas
}

fn text_of(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => blocks.iter().find_map(|block| {
            (block.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| str_field(block, "text"))
                .flatten()
        }),
        _ => None,
    }
}

/// Move the caller's `system` into a `<system-reminder>` block on the first user
/// turn and return the first user text as sent (the billing hash input).
fn relocate_system(map: &mut Map<String, Value>) -> String {
    let caller_system: Option<String> = match map.remove("system") {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text),
        Some(Value::Array(blocks)) => {
            let texts: Vec<String> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter(|b| {
                    str_field(b, "text").is_some_and(|t| {
                        t != fp::IDENTITY && !t.starts_with("x-anthropic-billing-header:")
                    })
                })
                .filter_map(|b| str_field(b, "text"))
                .collect();
            (!texts.is_empty()).then(|| texts.join("\n\n"))
        }
        _ => None,
    };
    let Some(messages) = map.get_mut("messages").and_then(Value::as_array_mut) else {
        return String::new();
    };
    let first_user = messages
        .iter_mut()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"));
    let Some(first_user) = first_user else {
        if let Some(system) = caller_system {
            messages.insert(
                0,
                json!({ "role": "user", "content": [ { "type": "text", "text": system_reminder(&system) } ] }),
            );
            return system_reminder(&system);
        }
        return String::new();
    };
    let Some(user) = first_user.as_object_mut() else {
        return String::new();
    };
    let mut blocks: Vec<Value> = match user.remove("content") {
        Some(Value::String(text)) => vec![json!({ "type": "text", "text": text })],
        Some(Value::Array(blocks)) => blocks,
        _ => Vec::new(),
    };
    if let Some(system) = caller_system {
        let reminder = system_reminder(&system);
        match blocks
            .iter_mut()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        {
            Some(block) => {
                let existing = str_field(block, "text").unwrap_or_default();
                if !existing.starts_with("<system-reminder>") {
                    block["text"] = Value::String(format!("{reminder}\n\n{existing}"));
                }
            }
            None => blocks.insert(0, json!({ "type": "text", "text": reminder })),
        }
    }
    let first_text = blocks
        .iter()
        .find_map(|b| {
            (b.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| str_field(b, "text"))
                .flatten()
        })
        .unwrap_or_default();
    user.insert("content".into(), Value::Array(blocks));
    let _ = text_of; // kept for callers that only need the text
    first_text
}

/// The Claude Code request fingerprint (`docs/PROVIDER_ACCOUNTS.md › Claude`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCodeShaper;

impl RequestShaper for ClaudeCodeShaper {
    fn fingerprint(&self) -> FingerprintInfo {
        fp::INFO
    }

    fn shape(
        &self,
        request: &mut ProviderHttpRequest,
        ctx: &ShapeContext<'_>,
    ) -> Result<(), ShapeError> {
        let token = ctx
            .access_token
            .ok_or(ShapeError::Missing("access_token"))?;
        let is_post = request.method.eq_ignore_ascii_case("POST");
        let is_messages = is_post && request.url.contains("/v1/messages");
        request.set_header("authorization", &format!("Bearer {token}"));
        request.remove_header("x-api-key");
        request.set_header("anthropic-version", fp::ANTHROPIC_VERSION);
        request.set_header("user-agent", fp::USER_AGENT);
        request.set_header("x-app", "cli");
        request.set_header("anthropic-dangerous-direct-browser-access", "true");
        request.set_header("x-claude-code-session-id", ctx.session_id);
        request.set_header("x-client-request-id", ctx.request_id);
        request.set_header("x-stainless-lang", "js");
        request.set_header("x-stainless-package-version", fp::STAINLESS_PACKAGE_VERSION);
        request.set_header("x-stainless-os", "MacOS");
        request.set_header("x-stainless-arch", "arm64");
        request.set_header("x-stainless-runtime", "node");
        request.set_header("x-stainless-runtime-version", fp::STAINLESS_RUNTIME_VERSION);
        request.set_header("x-stainless-retry-count", "0");
        request.set_header("x-stainless-timeout", "600");
        request.set_header("accept", "application/json");
        request.set_header("accept-encoding", "gzip, deflate, br, zstd");
        if is_post {
            request.set_header("content-type", "application/json");
        } else {
            request.remove_header("content-type");
        }
        if !is_messages {
            request.set_header("anthropic-beta", OAUTH_BETA);
            return Ok(());
        }
        let account_uuid = ctx
            .provider_account_id
            .ok_or(ShapeError::Missing("provider_account_id (account uuid)"))?;
        if ctx.device_id.is_empty() {
            return Err(ShapeError::Missing("device_id"));
        }
        let Some(map) = request.body.as_object_mut() else {
            return Err(ShapeError::InvalidBody(
                "a /v1/messages body must be a JSON object".into(),
            ));
        };
        let first_user_text = relocate_system(map);
        map.insert(
            "system".into(),
            json!([
                { "type": "text", "text": billing_header(&first_user_text) },
                { "type": "text", "text": fp::IDENTITY, "cache_control": { "type": "ephemeral", "ttl": "1h" } }
            ]),
        );
        map.insert(
            "metadata".into(),
            json!({ "user_id": user_id(ctx.device_id, account_uuid, ctx.session_id) }),
        );
        map.insert("stream".into(), Value::Bool(true));
        if !map.contains_key("max_tokens") {
            map.insert("max_tokens".into(), json!(max_tokens_for(ctx.model)));
        }
        if family(ctx.model) == Family::Fable && !map.contains_key("fallbacks") {
            map.insert("fallbacks".into(), json!([ { "model": "claude-opus-5" } ]));
        }
        let betas = betas_for(&request.body).join(",");
        request.set_header("anthropic-beta", &betas);
        Ok(())
    }

    fn detect_drift(
        &self,
        status: u16,
        body: &str,
        headers: &[(String, String)],
    ) -> Option<UnavailableReason> {
        drift_reason(status, body, headers)
    }
}

/// The shaped `GET /v1/models?limit=100`.
pub fn models_request(
    shaper: &ClaudeCodeShaper,
    base_url: &str,
    ctx: &ShapeContext<'_>,
) -> Result<ProviderHttpRequest, ShapeError> {
    let mut request = ProviderHttpRequest::new("GET", &models_url(base_url), Value::Null);
    shaper.shape(&mut request, ctx)?;
    Ok(request)
}

// ─────────────────────────────────────────────────────────────────────────────
// Errors — the extra-usage guard
// ─────────────────────────────────────────────────────────────────────────────

pub fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn has_unified_headers(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(k, _)| {
        k.to_ascii_lowercase()
            .starts_with("anthropic-ratelimit-unified-")
    })
}

/// `{"type":"error","error":{"type","message"}}` or `{"type","message"}`.
pub fn error_fields(body: &str) -> (Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return (None, None);
    };
    let error = value.get("error").unwrap_or(&value);
    (str_field(error, "type"), str_field(error, "message"))
}

pub fn mentions_extra_usage(message: &str) -> bool {
    message.to_ascii_lowercase().contains("extra usage")
}

pub fn mentions_long_context(message: &str) -> bool {
    message.to_ascii_lowercase().contains("long context")
}

/// Why a response says the fingerprint or the account no longer passes.
pub fn drift_reason(
    status: u16,
    body: &str,
    headers: &[(String, String)],
) -> Option<UnavailableReason> {
    match status {
        400 => {
            let (_, message) = error_fields(body);
            let text = message.unwrap_or_else(|| body.to_string());
            (mentions_extra_usage(&text) && !mentions_long_context(&text))
                .then_some(UnavailableReason::ExtraUsageBilling)
        }
        429 if !has_unified_headers(headers) => Some(UnavailableReason::FingerprintDrift),
        403 => Some(UnavailableReason::PolicyBlocked),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RateLimitInfo {
    pub until: Option<String>,
    pub window: Option<String>,
}

fn reset_to_iso(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.chars().all(|c| c.is_ascii_digit()) {
        return value.parse::<u64>().ok().map(iso_from_unix);
    }
    Some(value.to_string())
}

fn window_label(window: &str) -> &'static str {
    match window {
        "5h" | "five_hour" => "5h",
        "7d" | "seven_day" => "7d",
        "7d_oi" => "7d (model)",
        _ => "plan",
    }
}

/// The exhausted window and its reset from the unified rate-limit headers.
pub fn rate_limit_info(headers: &[(String, String)]) -> RateLimitInfo {
    let rejected: Option<&str> = ["5h", "7d", "7d_oi"].into_iter().find(|w| {
        header(headers, &format!("anthropic-ratelimit-unified-{w}-status")) == Some("rejected")
    });
    let claim = header(headers, "anthropic-ratelimit-unified-representative-claim");
    let window_key = rejected.or_else(|| {
        claim.map(|c| match c {
            "five_hour" => "5h",
            "seven_day" => "7d",
            other => other,
        })
    });
    let until = window_key
        .and_then(|w| header(headers, &format!("anthropic-ratelimit-unified-{w}-reset")))
        .or_else(|| header(headers, "anthropic-ratelimit-unified-reset"))
        .and_then(reset_to_iso);
    RateLimitInfo {
        until,
        window: window_key.or(claim).map(|w| window_label(w).to_string()),
    }
}

pub fn needs_reauth() -> BlueyError {
    BlueyError::new(
        BlueyErrorKind::Authentication,
        bluey_core::accounts::codes::NEEDS_REAUTH,
        "Claude rejected the sign-in — reconnect the account",
    )
    .recoverable(RecoveryAction::reconnect_account(
        CLAUDE_PROVIDER_ID,
        CLAUDE_PROVIDER_ID,
    ))
}

pub fn extra_usage_blocked(provider_message: &str) -> BlueyError {
    BlueyError::account(
        "extra_usage_blocked",
        format!(
            "Anthropic is billing this request outside the plan — Bluey stopped: {}",
            short(provider_message)
        ),
    )
    .recoverable(RecoveryAction::UseApiKey)
}

pub fn fingerprint_drift(message: impl Into<String>) -> BlueyError {
    BlueyError::account("fingerprint_drift", message).recoverable(RecoveryAction::UseApiKey)
}

pub fn policy_blocked(message: impl Into<String>) -> BlueyError {
    BlueyError::account("policy_blocked", message).recoverable(RecoveryAction::UseApiKey)
}

pub fn rate_limited(info: RateLimitInfo, message: impl Into<String>) -> BlueyError {
    let mut details = Map::new();
    if let Some(until) = &info.until {
        details.insert("until".into(), Value::String(until.clone()));
    }
    if let Some(window) = &info.window {
        details.insert("window".into(), Value::String(window.clone()));
    }
    let error = BlueyError::account("rate_limited", message);
    if details.is_empty() {
        error
    } else {
        error.with_details(Value::Object(details))
    }
}

fn short(text: &str) -> String {
    let trimmed = text.trim();
    let mut out: String = trimmed.chars().take(200).collect();
    if trimmed.chars().count() > 200 {
        out.push('…');
    }
    out
}

/// Map a non-2xx response. The extra-usage guard comes first: a 400 saying the
/// request draws from extra usage, or a bare 429 without the unified headers,
/// stops the account (`Unavailable`) — the router falls back, nothing retries.
pub fn map_error(status: u16, headers: &[(String, String)], body: &str) -> BlueyError {
    let (error_type, message) = error_fields(body);
    let message = message.unwrap_or_default();
    match status {
        401 => needs_reauth(),
        400 => {
            if mentions_extra_usage(&message) && !mentions_long_context(&message) {
                return extra_usage_blocked(&message);
            }
            if mentions_long_context(&message) {
                return BlueyError::ai(
                    "context_length_exceeded",
                    "this request needs Claude's long-context tier, which the plan does not include",
                );
            }
            BlueyError::ai(
                "invalid_request",
                if message.is_empty() {
                    "Claude rejected the request (HTTP 400)".to_string()
                } else {
                    format!("Claude rejected the request: {}", short(&message))
                },
            )
        }
        403 => policy_blocked(format!(
            "Claude blocked the account{}",
            if message.is_empty() {
                String::new()
            } else {
                format!(": {}", short(&message))
            }
        )),
        429 => {
            if !has_unified_headers(headers) {
                return fingerprint_drift(
                    "Claude answered a bare 429 without plan headers — the third-party detection response; Bluey stopped",
                );
            }
            let info = rate_limit_info(headers);
            let text = match (&info.window, &info.until) {
                (Some(window), Some(until)) => {
                    format!("Claude {window} limit reached — resets at {until}")
                }
                (Some(window), None) => format!("Claude {window} limit reached"),
                _ => "Claude plan limit reached".to_string(),
            };
            rate_limited(info, text)
        }
        529 => BlueyError::network("overloaded", "Claude is overloaded — try again in a moment"),
        500..=599 => BlueyError::network(
            "http_5xx",
            format!(
                "Claude returned HTTP {status}{}",
                if error_type.is_some() {
                    " (server error)"
                } else {
                    ""
                }
            ),
        ),
        _ => BlueyError::ai(
            &format!("http_{status}"),
            format!("Claude returned HTTP {status}"),
        ),
    }
}

/// A mid-stream `error` event.
pub fn map_stream_error(error_type: &str, message: &str) -> BlueyError {
    match error_type {
        "overloaded_error" => {
            BlueyError::network("overloaded", "Claude is overloaded — try again in a moment")
        }
        "rate_limit_error" => rate_limited(
            RateLimitInfo::default(),
            "Claude plan limit reached during the answer",
        ),
        "authentication_error" => needs_reauth(),
        "permission_error" => {
            policy_blocked(format!("Claude blocked the request: {}", short(message)))
        }
        "invalid_request_error" if mentions_extra_usage(message) => extra_usage_blocked(message),
        other => BlueyError::ai(other, short(message)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub ok: bool,
    pub billed_to: BilledTo,
    pub message: String,
}

pub fn probe_outcome(status: u16, headers: &[(String, String)], body: &str) -> ProbeOutcome {
    let unified = has_unified_headers(headers);
    match status {
        200 if unified => ProbeOutcome {
            ok: true,
            billed_to: BilledTo::Plan,
            message: "billed to the Claude plan (unified rate-limit headers present)".into(),
        },
        200 => ProbeOutcome {
            ok: true,
            billed_to: BilledTo::Unknown,
            message: "200 OK without unified rate-limit headers — billing pool unknown".into(),
        },
        400 => {
            let (_, message) = error_fields(body);
            let message = message.unwrap_or_default();
            if mentions_extra_usage(&message) {
                ProbeOutcome {
                    ok: false,
                    billed_to: BilledTo::ExtraUsage,
                    message: format!("extra-usage signal: {}", short(&message)),
                }
            } else {
                ProbeOutcome {
                    ok: false,
                    billed_to: BilledTo::Unknown,
                    message: format!("HTTP 400: {}", short(&message)),
                }
            }
        }
        429 if unified => ProbeOutcome {
            ok: false,
            billed_to: BilledTo::Plan,
            message: "plan window exhausted — the request counted against the plan".into(),
        },
        429 => ProbeOutcome {
            ok: false,
            billed_to: BilledTo::Unknown,
            message: "bare 429 without plan headers — the third-party detection response".into(),
        },
        _ => {
            let (_, message) = error_fields(body);
            ProbeOutcome {
                ok: false,
                billed_to: BilledTo::Unknown,
                message: format!(
                    "HTTP {status}{}",
                    message
                        .map(|m| format!(": {}", short(&m)))
                        .unwrap_or_default()
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprints::{
        self, diff, scrub_capture, Body, Capture, CaptureSource, CapturedRequest, FingerprintStamp,
        Header, Provider, SCHEMA_VERSION,
    };
    use pretty_assertions::assert_eq;

    #[test]
    fn the_authorize_url_and_token_bodies_follow_the_references() {
        let url = authorize_url(&redirect_uri(LOOPBACK_PORT), "chal", "st");
        let parsed = url::Url::parse(&url).unwrap();
        let keys: Vec<String> = parsed.query_pairs().map(|(k, _)| k.into_owned()).collect();
        assert_eq!(
            keys,
            vec![
                "code",
                "client_id",
                "response_type",
                "redirect_uri",
                "scope",
                "code_challenge",
                "code_challenge_method",
                "state"
            ]
        );
        let value = |k: &str| {
            parsed
                .query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
                .unwrap()
        };
        assert_eq!(value("code"), "true");
        assert_eq!(value("client_id"), CLIENT_ID);
        assert_eq!(value("redirect_uri"), "http://localhost:54545/callback");
        assert_eq!(value("scope"), SCOPES);
        assert_eq!(SCOPES.split(' ').count(), 6);
        let exchange = token_exchange_body("c", MANUAL_REDIRECT_URI, "v", "st");
        assert_eq!(exchange["grant_type"], "authorization_code");
        assert_eq!(exchange["state"], "st");
        assert_eq!(exchange["redirect_uri"], MANUAL_REDIRECT_URI);
        let refresh = refresh_body("rt");
        assert_eq!(refresh["grant_type"], "refresh_token");
        assert_eq!(refresh["scope"], SCOPES);
        assert_eq!(TOKEN_HEADERS[2], ("user-agent", "axios/1.15.2"));
        assert_eq!(
            parse_token_extras(
                r#"{"access_token":"a","account":{"uuid":"acc-1","email_address":"o@example.com"},"organization":{"uuid":"org-1","name":"Owner"}}"#
            ),
            TokenExtras {
                account_uuid: Some("acc-1".into()),
                email: Some("o@example.com".into()),
                organization_uuid: Some("org-1".into()),
                organization_name: Some("Owner".into()),
            }
        );
        assert_eq!(parse_token_extras("nope"), TokenExtras::default());
    }

    #[test]
    fn the_profile_becomes_the_identity_with_a_plan_label() {
        let profile = parse_profile(
            r#"{"account":{"uuid":"acc-1","email":"o@example.com","has_claude_pro":false,"has_claude_max":true},"organization":{"uuid":"org-1","name":"Owner's org","rate_limit_tier":"default_claude_max_5x","organization_type":"claude_max","subscription_status":"active"}}"#,
        )
        .unwrap();
        assert!(profile.has_max);
        assert_eq!(plan_label(&profile), "Claude Max 5×");
        let identity = account_identity(&profile);
        assert_eq!(identity.plan_label.as_deref(), Some("Claude Max 5×"));
        assert_eq!(identity.plan_tier.as_deref(), Some("default_claude_max_5x"));
        assert_eq!(identity.account_id.as_deref(), Some("acc-1"));
        assert_eq!(identity.display_name.as_deref(), Some("Owner's org"));
        let pro = Profile {
            rate_limit_tier: Some("default_claude_pro".into()),
            ..Profile::default()
        };
        assert_eq!(plan_label(&pro), "Claude Pro");
        let twenty = Profile {
            rate_limit_tier: Some("default_claude_max_20x".into()),
            ..Profile::default()
        };
        assert_eq!(plan_label(&twenty), "Claude Max 20×");
        let team = Profile {
            organization_type: Some("claude_team".into()),
            ..Profile::default()
        };
        assert_eq!(plan_label(&team), "Claude Team");
        let flags_only = Profile {
            has_pro: true,
            ..Profile::default()
        };
        assert_eq!(plan_label(&flags_only), "Claude Pro");
        assert_eq!(plan_label(&Profile::default()), "Claude");
    }

    #[test]
    fn claude_codes_credential_store_is_read_without_being_written() {
        let creds = parse_credentials(
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-abc","refreshToken":"sk-ant-ort01-def","expiresAt":1757600000000,"scopes":["user:inference","user:profile"],"subscriptionType":"max","rateLimitTier":"default_claude_max_5x","refreshTokenExpiresAt":1760000000000}}"#,
        )
        .unwrap();
        assert_eq!(creds.access_token, "sk-ant-oat01-abc");
        assert_eq!(creds.refresh_token.as_deref(), Some("sk-ant-ort01-def"));
        assert_eq!(
            creds.expires_at,
            Some(1_757_600_000),
            "milliseconds → seconds"
        );
        assert_eq!(creds.scopes.len(), 2);
        assert_eq!(creds.subscription_type.as_deref(), Some("max"));
        assert_eq!(
            parse_credentials(r#"{"claudeAiOauth":{}}"#),
            Err(ImportError::NoTokens)
        );
        assert_eq!(
            parse_credentials(r#"{"other":1}"#),
            Err(ImportError::NoTokens)
        );
        assert!(matches!(
            parse_credentials("x"),
            Err(ImportError::Malformed(_))
        ));
        assert_eq!(
            keychain_accounts("jordan"),
            vec!["jordan", "default", "unknown"]
        );
        assert_eq!(keychain_accounts("default"), vec!["default", "unknown"]);
        assert_eq!(keychain_accounts(""), vec!["default", "unknown"]);
        let home = Path::new("/Users/owner");
        assert_eq!(
            credentials_file(home, None),
            PathBuf::from("/Users/owner/.claude/.credentials.json")
        );
        assert_eq!(
            credentials_file(home, Some("/tmp/cc")),
            PathBuf::from("/tmp/cc/.credentials.json")
        );
        assert_eq!(
            claude_json_path(home),
            PathBuf::from("/Users/owner/.claude.json")
        );
        let info = parse_claude_json(
            r#"{"oauthAccount":{"accountUuid":"acc-1","emailAddress":"o@example.com","organizationUuid":"org-1"}}"#,
        );
        assert_eq!(info.account_uuid.as_deref(), Some("acc-1"));
        let identity = identity_from_import(&creds, &info);
        assert_eq!(identity.plan_label.as_deref(), Some("Claude Max 5×"));
        assert_eq!(identity.email.as_deref(), Some("o@example.com"));
        assert_eq!(identity.account_id.as_deref(), Some("acc-1"));
        assert_eq!(parse_claude_json("{}"), OauthAccountInfo::default());
    }

    #[test]
    fn catalogs_carry_the_documented_roles() {
        let models = catalog_from_models_response(
            r#"{"data":[{"type":"model","id":"claude-opus-5","display_name":"Claude Opus 5","created_at":"2026-05-01T00:00:00Z"},{"type":"model","id":"claude-sonnet-5","display_name":"Claude Sonnet 5"},{"type":"model","id":"claude-haiku-4-5-20251001","display_name":"Claude Haiku 4.5"},{"type":"model","id":"claude-fable-5-1","display_name":"Claude Fable 5.1"}],"has_more":false}"#,
        )
        .unwrap();
        let roles = |id: &str| {
            models
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.suggested_roles.clone())
                .unwrap()
        };
        assert_eq!(
            roles("claude-opus-5"),
            vec![ModelRole::Reasoning, ModelRole::Research]
        );
        assert_eq!(
            roles("claude-sonnet-5"),
            vec![ModelRole::Default, ModelRole::Vision]
        );
        assert_eq!(roles("claude-haiku-4-5-20251001"), vec![ModelRole::Fast]);
        assert!(roles("claude-fable-5-1").is_empty());
        assert!(models
            .iter()
            .find(|m| m.id == "claude-haiku-4-5-20251001")
            .unwrap()
            .capabilities
            .reasoning_levels
            .is_empty());
        assert_eq!(
            models
                .iter()
                .find(|m| m.id == "claude-opus-5")
                .unwrap()
                .capabilities
                .reasoning_levels,
            vec!["low", "medium", "high", "max"]
        );
        let curated = curated_catalog();
        assert_eq!(curated.len(), CURATED_MODELS.len());
        assert!(curated
            .iter()
            .any(|m| m.id == "claude-sonnet-5" && m.suggested_roles.contains(&ModelRole::Default)));
        assert!(catalog_from_models_response(r#"{"data":[]}"#)
            .unwrap()
            .is_empty());
        assert_eq!(max_tokens_for("claude-sonnet-5"), 128_000);
        assert_eq!(max_tokens_for("claude-haiku-4-5-20251001"), 64_000);
        assert_eq!(max_tokens_for("claude-sonnet-4-6"), 64_000);
        assert_eq!(family("claude-fable-5-1"), Family::Fable);
    }

    #[test]
    fn thinking_follows_the_reasoning_level_and_never_reaches_haiku() {
        let mut body = json!({ "model": "claude-sonnet-5", "temperature": 0.2, "max_tokens": 64, "messages": [] });
        apply_thinking(
            &mut body,
            "claude-sonnet-5",
            ReasoningLevel::Deep,
            LatencyBudget::Balanced,
        );
        assert_eq!(
            body["thinking"],
            json!({ "type": "adaptive", "display": "summarized" })
        );
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(
            body["context_management"]["edits"][0]["type"],
            "clear_thinking_20251015"
        );
        assert!(
            body.get("temperature").is_none(),
            "no sampling knob next to thinking"
        );
        apply_thinking(
            &mut body,
            "claude-sonnet-5",
            ReasoningLevel::None,
            LatencyBudget::Fast,
        );
        assert!(body.get("thinking").is_none());
        assert!(body.get("output_config").is_none());
        assert!(body.get("context_management").is_none());
        let mut haiku = json!({ "model": "claude-haiku-4-5-20251001", "messages": [] });
        apply_thinking(
            &mut haiku,
            "claude-haiku-4-5-20251001",
            ReasoningLevel::Deep,
            LatencyBudget::Deep,
        );
        assert!(haiku.get("thinking").is_none());
        assert_eq!(
            effort_for(ReasoningLevel::Deep, LatencyBudget::Deep),
            Some("max")
        );
        assert_eq!(
            effort_for(ReasoningLevel::Light, LatencyBudget::Fast),
            Some("low")
        );
        assert_eq!(
            effort_for(ReasoningLevel::Light, LatencyBudget::Balanced),
            Some("medium")
        );
        assert_eq!(effort_for(ReasoningLevel::None, LatencyBudget::Deep), None);
        let mut with_schema = json!({ "output_config": { "format": { "type": "json_schema" } } });
        apply_thinking(
            &mut with_schema,
            "claude-sonnet-5",
            ReasoningLevel::Light,
            LatencyBudget::Fast,
        );
        assert_eq!(
            with_schema["output_config"]["format"]["type"], "json_schema",
            "the format survives"
        );
        assert_eq!(with_schema["output_config"]["effort"], "low");
        let mut capped = json!({ "model": "m" });
        normalise_max_tokens(&mut capped, "claude-opus-5", None);
        assert_eq!(capped["max_tokens"], 128_000);
        normalise_max_tokens(&mut capped, "claude-opus-5", Some(64));
        assert_eq!(capped["max_tokens"], 64);
    }

    #[test]
    fn the_billing_header_hashes_the_first_user_text_as_the_references_do() {
        let text = "Summarise what is on my screen, please, and keep it short.";
        let suffix = version_suffix(text, "2.1.258");
        let expected_input = format!("59cf53e54c78{}{}{}2.1.258", "a", "s", " ");
        assert_eq!(&text.chars().nth(4).unwrap().to_string(), "a");
        assert_eq!(&text.chars().nth(7).unwrap().to_string(), "s");
        assert_eq!(&text.chars().nth(20).unwrap().to_string(), " ");
        assert_eq!(suffix, hex(&Sha256::digest(expected_input.as_bytes()))[..3]);
        let padded = version_suffix("hi", "2.1.258");
        assert_eq!(
            padded,
            hex(&Sha256::digest(b"59cf53e54c780002.1.258"))[..3],
            "missing characters pad with 0"
        );
        assert_eq!(cch(text), hex(&Sha256::digest(text.as_bytes()))[..5]);
        let header_line = billing_header(text);
        assert!(
            regex::Regex::new(fp::BILLING_HEADER_PATTERN)
                .unwrap()
                .is_match(&header_line),
            "{header_line}"
        );
        assert_eq!(
            user_id("d".repeat(64).as_str(), "acc", "sess"),
            format!(
                r#"{{"device_id":"{}","account_uuid":"acc","session_id":"sess"}}"#,
                "d".repeat(64)
            )
        );
        assert_eq!(
            system_reminder(" You are Bluey. "),
            "<system-reminder>\nYou are Bluey.\n</system-reminder>"
        );
    }

    fn ctx<'a>(model: &'a str, device: &'a str) -> ShapeContext<'a> {
        ShapeContext {
            account_id: "claude",
            provider_account_id: Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e"),
            device_id: device,
            session_id: "6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b",
            request_id: "7a1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7c",
            model,
            access_token: Some("sk-ant-oat01-SECRETSECRETSECRETSECRET"),
        }
    }

    fn capture_of(request: &ProviderHttpRequest) -> Capture {
        let host = url::Url::parse(&request.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        let mut headers: Vec<Header> = request
            .headers
            .iter()
            .map(|(n, v)| Header::new(n, v.clone()))
            .collect();
        headers.push(Header::new("host", host));
        let mut capture = Capture {
            schema: SCHEMA_VERSION,
            provider: "claude".into(),
            source: CaptureSource::Probe,
            captured_at: "2026-09-11T12:00:00Z".into(),
            fingerprint: FingerprintStamp {
                version: fp::VERSION.into(),
                captured_on: fp::CAPTURED_ON.into(),
            },
            client: fingerprints::client_from_user_agent(
                request.header("user-agent").unwrap_or(""),
            ),
            request: CapturedRequest {
                method: request.method.clone(),
                url: request.url.clone(),
                headers,
                body: if request.body.is_null() {
                    Body::Empty
                } else {
                    Body::Json {
                        value: request.body.clone(),
                    }
                },
            },
            response: None,
            scrubbed: Vec::new(),
            notes: Vec::new(),
        };
        scrub_capture(&mut capture, Provider::Claude.rules());
        capture
    }

    fn documented(endpoint: &str) -> Capture {
        Provider::Claude
            .documented()
            .into_iter()
            .find(|c| {
                Provider::Claude
                    .rules()
                    .endpoint_for(&c.request.method, &c.request.path())
                    .is_some_and(|e| e.name == endpoint)
            })
            .expect("documented capture")
    }

    #[test]
    fn the_shaper_reproduces_the_documented_fingerprint() {
        let device = "a".repeat(64);
        let mut body =
            crate::anthropic::build_messages_body(&crate::anthropic::MessagesBodyOptions {
                model: "claude-sonnet-5",
                messages: &[
                    bluey_core::types::AiMessage::text(
                        bluey_core::types::AiRole::System,
                        "You are Bluey, a discreet copilot.",
                    ),
                    bluey_core::types::AiMessage::text(
                        bluey_core::types::AiRole::User,
                        "What is on my screen?",
                    ),
                ],
                stream: true,
                max_output_tokens: None,
                temperature: Some(0.3),
                output_schema: None,
                schema_as_prompt_fallback: false,
            });
        apply_thinking(
            &mut body,
            "claude-sonnet-5",
            ReasoningLevel::Light,
            LatencyBudget::Balanced,
        );
        normalise_max_tokens(&mut body, "claude-sonnet-5", None);
        let mut request = ProviderHttpRequest::new("POST", &messages_url(fp::UPSTREAM), body);
        request
            .headers
            .push(("x-api-key".into(), "sk-ant-api03-should-go".into()));
        ClaudeCodeShaper
            .shape(&mut request, &ctx("claude-sonnet-5", &device))
            .unwrap();

        assert_eq!(request.header("x-api-key"), None, "OAuth is Bearer-only");
        assert_eq!(
            request.header("authorization"),
            Some("Bearer sk-ant-oat01-SECRETSECRETSECRETSECRET")
        );
        assert_eq!(request.header("user-agent"), Some(fp::USER_AGENT));
        assert_eq!(request.header("accept"), Some("application/json"));
        let betas = request.header("anthropic-beta").unwrap();
        for beta in fp::BETAS_ALWAYS {
            assert!(betas.contains(beta), "{beta} missing from {betas}");
        }
        assert!(
            betas.contains("effort-2025-11-24"),
            "effort beta rides with output_config.effort"
        );
        assert!(betas.contains("extended-cache-ttl-2025-04-11"));
        assert!(!betas.contains("structured-outputs"), "no schema, no beta");
        assert!(!betas.contains("context-1m"), "never by default");

        let system = request.body["system"].as_array().unwrap();
        assert_eq!(system.len(), 2, "exactly two blocks");
        assert!(system[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("x-anthropic-billing-header: cc_version=2.1.258."));
        assert_eq!(system[1]["text"], fp::IDENTITY);
        assert_eq!(
            system[1]["cache_control"],
            json!({ "type": "ephemeral", "ttl": "1h" })
        );
        let first_user = &request.body["messages"][0];
        assert_eq!(first_user["role"], "user");
        let text = first_user["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("<system-reminder>\nYou are Bluey, a discreet copilot.\n</system-reminder>\n\nWhat is on my screen?"), "{text}");
        assert_eq!(
            request.body["metadata"]["user_id"],
            format!(
                r#"{{"device_id":"{device}","account_uuid":"9d1c250a-e61b-44d9-88ed-5944d1962f5e","session_id":"6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b"}}"#
            )
        );
        assert_eq!(request.body["max_tokens"], 128_000);
        assert_eq!(request.body["stream"], true);
        assert!(request.body.get("temperature").is_none());
        assert!(request.body.get("fallbacks").is_none());

        let report = diff(
            Provider::Claude.rules(),
            &documented("messages"),
            &capture_of(&request),
            "documented",
            "shaper",
        );
        assert!(!report.has_drift(), "{}", report.render());

        let models = models_request(&ClaudeCodeShaper, fp::UPSTREAM, &ctx("", &device)).unwrap();
        assert_eq!(models.header("anthropic-beta"), Some(OAUTH_BETA));
        assert_eq!(models.header("content-type"), None);
        let report = diff(
            Provider::Claude.rules(),
            &documented("models"),
            &capture_of(&models),
            "documented",
            "shaper",
        );
        assert!(!report.has_drift(), "{}", report.render());

        let mut fable = ProviderHttpRequest::new(
            "POST",
            &messages_url(fp::UPSTREAM),
            json!({ "model": "claude-fable-5-1", "messages": [ { "role": "user", "content": "hi" } ] }),
        );
        ClaudeCodeShaper
            .shape(&mut fable, &ctx("claude-fable-5-1", &device))
            .unwrap();
        assert_eq!(
            fable.body["fallbacks"],
            json!([ { "model": "claude-opus-5" } ])
        );
        assert_eq!(fable.body["max_tokens"], 128_000);
        assert_eq!(
            fable.body["messages"][0]["content"][0]["text"], "hi",
            "string content becomes a block; no system → no reminder"
        );

        let mut without_device = ProviderHttpRequest::new(
            "POST",
            &messages_url(fp::UPSTREAM),
            json!({ "messages": [] }),
        );
        assert_eq!(
            ClaudeCodeShaper.shape(&mut without_device, &ctx("m", "")),
            Err(ShapeError::Missing("device_id"))
        );
        assert_eq!(ClaudeCodeShaper.fingerprint(), fp::INFO);
    }

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_extra_usage_guard_stops_and_never_retries() {
        let extra = map_error(
            400,
            &[],
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"Third-party apps now draw from your extra usage, not your plan limits. Add more at claude.ai/settings/usage and keep going."}}"#,
        );
        assert_eq!(extra.code, "account.extra_usage_blocked");
        assert_eq!(extra.recovery, Some(RecoveryAction::UseApiKey));
        assert!(extra.message.contains("Third-party apps"));
        assert_eq!(
            bluey_core::accounts::status_after_error(&extra),
            Some(bluey_core::types::AccountStatus::Unavailable {
                reason: UnavailableReason::ExtraUsageBilling,
                detail: Some(extra.message.clone()),
            })
        );
        let out = map_error(
            400,
            &[],
            r#"{"error":{"type":"invalid_request_error","message":"You're out of extra usage. Add more at claude.ai/settings/usage and keep going."}}"#,
        );
        assert_eq!(out.code, "account.extra_usage_blocked");
        let long = map_error(
            400,
            &[],
            r#"{"error":{"type":"invalid_request_error","message":"Extra usage is required for long context requests."}}"#,
        );
        assert_eq!(
            long.code, "ai.context_length_exceeded",
            "the 1M entitlement is not the classifier"
        );
        let bare = map_error(
            429,
            &headers(&[("request-id", "req_1")]),
            r#"{"type":"rate_limit_error","message":"Error"}"#,
        );
        assert_eq!(bare.code, "account.fingerprint_drift");
        assert_eq!(
            ClaudeCodeShaper.detect_drift(429, "", &headers(&[("request-id", "req_1")])),
            Some(UnavailableReason::FingerprintDrift)
        );
        assert_eq!(
            ClaudeCodeShaper.detect_drift(
                400,
                r#"{"error":{"message":"Third-party apps now draw from your extra usage"}}"#,
                &[]
            ),
            Some(UnavailableReason::ExtraUsageBilling)
        );
        assert_eq!(
            ClaudeCodeShaper.detect_drift(
                400,
                r#"{"error":{"message":"Extra usage is required for long context requests."}}"#,
                &[]
            ),
            None
        );
        assert_eq!(
            ClaudeCodeShaper.detect_drift(403, "", &[]),
            Some(UnavailableReason::PolicyBlocked)
        );

        let limited = map_error(
            429,
            &headers(&[
                ("anthropic-ratelimit-unified-status", "rejected"),
                ("anthropic-ratelimit-unified-5h-status", "rejected"),
                ("anthropic-ratelimit-unified-7d-status", "allowed"),
                ("anthropic-ratelimit-unified-5h-reset", "1700003600"),
                (
                    "anthropic-ratelimit-unified-representative-claim",
                    "five_hour",
                ),
            ]),
            r#"{"type":"rate_limit_error","message":"Rate limited"}"#,
        );
        assert_eq!(limited.code, "account.rate_limited");
        let details = limited.details.clone().unwrap();
        assert_eq!(details["until"], "2023-11-14T23:13:20Z");
        assert_eq!(details["window"], "5h");
        assert!(limited.message.contains("5h limit"));
        let weekly = map_error(
            429,
            &headers(&[
                ("anthropic-ratelimit-unified-7d-status", "rejected"),
                (
                    "anthropic-ratelimit-unified-7d-reset",
                    "2026-09-14T00:00:00Z",
                ),
            ]),
            "",
        );
        assert_eq!(
            weekly.details.clone().unwrap()["until"],
            "2026-09-14T00:00:00Z"
        );
        assert_eq!(weekly.details.clone().unwrap()["window"], "7d");
        let claim_only = map_error(
            429,
            &headers(&[
                (
                    "anthropic-ratelimit-unified-representative-claim",
                    "seven_day",
                ),
                ("anthropic-ratelimit-unified-reset", "1700003600"),
            ]),
            "",
        );
        assert_eq!(claim_only.details.clone().unwrap()["window"], "7d");
        assert_eq!(
            claim_only.details.clone().unwrap()["until"],
            "2023-11-14T23:13:20Z"
        );

        assert_eq!(map_error(401, &[], "").code, "account.needs_reauth");
        assert_eq!(
            map_error(403, &[], r#"{"error":{"message":"forbidden"}}"#).code,
            "account.policy_blocked"
        );
        let overloaded = map_error(529, &[], r#"{"error":{"type":"overloaded_error"}}"#);
        assert_eq!(overloaded.code, "network.overloaded");
        assert_eq!(overloaded.recovery, Some(RecoveryAction::Retry));
        assert_eq!(map_error(500, &[], "").code, "network.http_5xx");
        assert_eq!(
            map_error(400, &[], r#"{"error":{"message":"max_tokens: too large"}}"#).code,
            "ai.invalid_request"
        );
        assert_eq!(map_error(418, &[], "").code, "ai.http_418");
        assert_eq!(
            map_stream_error("rate_limit_error", "Error").code,
            "account.rate_limited"
        );
        assert_eq!(
            map_stream_error(
                "invalid_request_error",
                "Third-party apps now draw from your extra usage"
            )
            .code,
            "account.extra_usage_blocked"
        );
        assert_eq!(
            map_stream_error("authentication_error", "x").code,
            "account.needs_reauth"
        );
        assert_eq!(
            map_stream_error("overloaded_error", "x").code,
            "network.overloaded"
        );
        assert_eq!(map_stream_error("api_error", "boom").code, "ai.api_error");
    }

    #[test]
    fn probe_outcomes_read_the_unified_headers() {
        let plan = probe_outcome(
            200,
            &headers(&[("anthropic-ratelimit-unified-status", "allowed")]),
            "",
        );
        assert_eq!((plan.ok, plan.billed_to), (true, BilledTo::Plan));
        let unknown = probe_outcome(200, &[], "");
        assert_eq!((unknown.ok, unknown.billed_to), (true, BilledTo::Unknown));
        let extra = probe_outcome(
            400,
            &[],
            r#"{"error":{"message":"Third-party apps now draw from your extra usage"}}"#,
        );
        assert_eq!((extra.ok, extra.billed_to), (false, BilledTo::ExtraUsage));
        let bare = probe_outcome(429, &[], "");
        assert!(!bare.ok && bare.message.contains("third-party detection"));
        let window = probe_outcome(
            429,
            &headers(&[("anthropic-ratelimit-unified-5h-status", "rejected")]),
            "",
        );
        assert_eq!(window.billed_to, BilledTo::Plan);
        assert!(probe_outcome(503, &[], r#"{"error":{"message":"down"}}"#)
            .message
            .contains("HTTP 503: down"));
    }
}
