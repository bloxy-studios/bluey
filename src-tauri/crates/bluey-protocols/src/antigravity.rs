//! Google AI Pro / Ultra through the Antigravity OAuth client on Cloud Code
//! `v1internal` (ADR 0009 §4c, PR 3c) — the pure codec.
//!
//! Every value here is a row of `docs/PROVIDER_ACCOUNTS.md › Google AI`
//! (verified 2026-09-11 against CLIProxyAPI @ `09a29bd`; the User-Agent family
//! and the generation host are disputed between the references and settle with a
//! capture of the real app). The module owns:
//!
//! * the Google OAuth constants and form bodies (five scopes, PKCE, loopback
//!   port 51121, the public client secret supplied by the build — never
//!   committed), `userinfo` → e-mail;
//! * `loadCodeAssist` / `onboardUser` parsing: project, tier → plan label,
//!   `VALIDATION_REQUIRED` links, the onboarding tier and the LRO state;
//! * the Hub update manifest → client version (floored) → User-Agent;
//! * `fetchAvailableModels` → catalog with Bluey's preset roles, and the
//!   curated pool list as the fallback;
//! * the request normalisation per model family (Gemini `thinkingLevel`, Claude
//!   `thinkingBudget`, none for GPT-OSS) and the Cloud Code wrapper
//!   (`model` / `project` / `request{…, sessionId}` / `userAgent` /
//!   `requestType` / `requestId`) as [`AntigravityShaper`], reproducing the
//!   documented capture — the thin header set the native client sends;
//! * the response envelope (`{response, traceId}`) and the error mapper: a
//!   Terms-of-Service 403 stops the account, `VALIDATION_REQUIRED` carries the
//!   link, "no longer supported" is fingerprint drift, `QUOTA_EXHAUSTED` is a
//!   plan window, capacity is a short retry;
//! * the Keychain payload the standalone app / `agy` keep (read-only import).
//!
//! No I/O, no tokens kept: the app crate's `accounts/antigravity.rs` and
//! `ai/providers/antigravity.rs` do the talking.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use bluey_core::error::RecoveryAction;
use bluey_core::types::{
    AccountIdentity, BilledTo, CatalogModel, ModelCapabilities, ModelRole, ReasoningLevel,
    UnavailableReason, ANTIGRAVITY_PROVIDER_ID,
};
use bluey_core::{BlueyError, BlueyErrorKind};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::codex::iso_from_unix;
use crate::fingerprints::antigravity as fp;
use crate::gemini;
use crate::request_shaper::{
    FingerprintInfo, ProviderHttpRequest, RequestShaper, ShapeContext, ShapeError,
};

// ── OAuth ────────────────────────────────────────────────────────────────────

/// Antigravity's public OAuth client (all four references; gemini-cli's is a
/// different client whose consumer path is shut down — never use it).
pub const CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
/// The five scopes, in the order CLIProxyAPI sends them.
pub const SCOPES: [&str; 5] = [
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/cclog",
    "https://www.googleapis.com/auth/experimentsandconfigs",
];
pub const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
pub const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo?alt=json";
/// The fixed loopback port every reference registers; any free port is the
/// fallback (installed-app clients accept every `http://localhost:<port>`).
pub const LOOPBACK_PORT: u16 = 51121;
pub const CALLBACK_PATH: &str = "/oauth-callback";
/// "Real Antigravity uses Go's default User-Agent for OAuth token refresh" (CLIProxyAPI).
pub const REFRESH_USER_AGENT: &str = "Go-http-client/2.0";
/// How long the browser sign-in may take.
pub const BROWSER_FLOW_SECS: u64 = 600;

// ── Cloud Code ───────────────────────────────────────────────────────────────

/// `onboardUser` adds this header; nothing else does.
pub const ONBOARD_API_CLIENT: &str = "gl-node/22.21.1";
/// `onboardUser`'s User-Agent carries the Node client suffix.
pub const ONBOARD_USER_AGENT_SUFFIX: &str = " google-api-nodejs-client/10.3.0";
/// The Hub update manifest the User-Agent version comes from (cached 6 h).
pub const MANIFEST_URL: &str =
    "https://antigravity-hub-auto-updater-974169037036.us-central1.run.app/manifest/latest-arm64-mac.yml";
pub const MANIFEST_USER_AGENT: &str = "electron-builder";
/// "Cloud Code rejects newer models for clients below 2.9.0" — never go under this.
pub const VERSION_FLOOR: &str = "2.9.1";
/// The identity text the archived plugin and Antigravity-Manager prepend; the
/// maintained reference ships without it, so Bluey sends it only in the probe's A/B.
pub const IDENTITY_TEXT: &str = "You are Antigravity, a powerful agentic AI coding assistant designed by the Google DeepMind team working on Advanced Agentic Coding.\nYou are pair programming with a USER to solve their coding task. The task may require creating a new codebase, modifying or debugging an existing codebase, or simply answering a question.\n**Absolute paths only**\n**Proactiveness**";
/// The cheapest model to probe billing with.
pub const PROBE_MODEL: &str = "gemini-3.1-flash-lite";
/// A `QUOTA_EXHAUSTED` without a delay: wait this long before the account is retried.
pub const DEFAULT_COOLDOWN_SECS: u64 = 30 * 60;
/// A retry delay this long or longer is a plan window, not a capacity blip.
pub const LONG_DELAY_SECS: u64 = 300;
/// Standalone Antigravity / `agy` keep their sign-in here (generic password).
pub const KEYCHAIN_SERVICE: &str = "gemini";
pub const KEYCHAIN_ACCOUNT: &str = "antigravity";
const KEYCHAIN_PREFIX: &str = "go-keyring-base64:";

/// Internal ids `fetchAvailableModels` lists that no client offers.
pub const INTERNAL_MODEL_IDS: &[&str] = &[
    "chat_20706",
    "chat_23310",
    "tab_flash_lite_preview",
    "tab_jump_flash_lite_preview",
    "gemini-2.5-flash-thinking",
    "gemini-2.5-pro",
];

/// The Antigravity pool on 2026-09-11 (CLIProxyAPI `models.json`), the fallback
/// when `fetchAvailableModels` refuses the token.
pub const CURATED_MODELS: &[(&str, &str)] = &[
    ("claude-opus-4-6-thinking", "Claude Opus 4.6 (Thinking)"),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("gemini-3-flash", "Gemini 3 Flash"),
    ("gemini-3.1-flash-lite", "Gemini 3.1 Flash-Lite"),
    ("gemini-3.1-flash-image", "Gemini 3.1 Flash Image"),
    ("gemini-3.1-pro-low", "Gemini 3.1 Pro (Low)"),
    ("gemini-pro-agent", "Gemini 3.1 Pro (High)"),
    ("gemini-3.6-flash-high", "Gemini 3.6 Flash (High)"),
    ("gemini-3.7-flash-high", "Gemini 3.7 Flash (High)"),
    ("gemini-3.8-flash-high", "Gemini 3.8 Flash (High)"),
    ("gpt-oss-120b-medium", "GPT-OSS 120B (Medium)"),
];

pub fn scope_string() -> String {
    SCOPES.join(" ")
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CALLBACK_PATH}")
}

/// The authorize URL: `access_type=offline` + `prompt=consent` for a refresh
/// token, PKCE S256 (the plugin's variant; CLIProxyAPI omits it — both work).
pub fn authorize_url(redirect_uri: &str, code_challenge: &str, state: &str) -> String {
    let mut url = url::Url::parse(AUTHORIZE_URL).expect("static url");
    url.query_pairs_mut()
        .append_pair("access_type", "offline")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("prompt", "consent")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &scope_string())
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
    url.to_string()
}

/// Form body of the code exchange — Google's installed-app clients send the
/// (public) client secret with it.
pub fn token_exchange_form(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    client_secret: &str,
) -> Vec<(&'static str, String)> {
    vec![
        ("code", code.to_string()),
        ("client_id", CLIENT_ID.to_string()),
        ("client_secret", client_secret.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("grant_type", "authorization_code".to_string()),
        ("code_verifier", code_verifier.to_string()),
    ]
}

/// Form body of a refresh — **includes the client secret**.
pub fn refresh_form(refresh_token: &str, client_secret: &str) -> Vec<(&'static str, String)> {
    vec![
        ("client_id", CLIENT_ID.to_string()),
        ("client_secret", client_secret.to_string()),
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
    ]
}

/// Form body of a revocation (`token` = the refresh token, which also kills the access token).
pub fn revoke_form(token: &str) -> Vec<(&'static str, String)> {
    vec![("token", token.to_string())]
}

// ── Identity ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct UserInfo {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

pub fn parse_userinfo(json: &str) -> Result<UserInfo, serde_json::Error> {
    serde_json::from_str(json)
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tier {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub user_defined_cloudaicompanion_project: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IneligibleTier {
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub reason_message: Option<String>,
    #[serde(default)]
    pub validation_url: Option<String>,
}

/// What `loadCodeAssist` says about the account.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodeAssistInfo {
    pub project: Option<String>,
    pub current_tier: Option<Tier>,
    pub paid_tier: Option<Tier>,
    pub allowed_tiers: Vec<Tier>,
    pub ineligible_tiers: Vec<IneligibleTier>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireCodeAssist {
    #[serde(default)]
    cloudaicompanion_project: Option<Value>,
    #[serde(default)]
    current_tier: Option<Tier>,
    #[serde(default)]
    paid_tier: Option<Tier>,
    #[serde(default)]
    allowed_tiers: Vec<Tier>,
    #[serde(default)]
    ineligible_tiers: Vec<IneligibleTier>,
}

fn project_of(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Object(map) => map
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string()),
        _ => None,
    }
}

pub fn parse_load_code_assist(body: &str) -> Result<CodeAssistInfo, serde_json::Error> {
    let wire: WireCodeAssist = serde_json::from_str(body)?;
    Ok(CodeAssistInfo {
        project: project_of(wire.cloudaicompanion_project.as_ref()),
        current_tier: wire.current_tier,
        paid_tier: wire.paid_tier,
        allowed_tiers: wire.allowed_tiers,
        ineligible_tiers: wire.ineligible_tiers,
    })
}

impl CodeAssistInfo {
    /// The tier the plan label comes from: paid, then current, then the default allowed one.
    pub fn plan(&self) -> Option<&Tier> {
        self.paid_tier
            .as_ref()
            .or(self.current_tier.as_ref())
            .or_else(|| self.allowed_tiers.iter().find(|t| t.is_default))
            .or_else(|| self.allowed_tiers.first())
    }

    /// The tier `onboardUser` is asked for when no project exists yet
    /// (CLIProxyAPI: default allowed → current → `free-tier`).
    pub fn onboarding_tier(&self) -> String {
        self.allowed_tiers
            .iter()
            .find(|t| t.is_default)
            .and_then(|t| t.id.clone())
            .or_else(|| self.current_tier.as_ref().and_then(|t| t.id.clone()))
            .unwrap_or_else(|| "free-tier".to_string())
    }

    /// Google wants the user to verify in a browser first.
    pub fn validation_url(&self) -> Option<String> {
        self.ineligible_tiers
            .iter()
            .find(|t| t.reason_code.as_deref() == Some("VALIDATION_REQUIRED"))
            .map(|t| {
                t.validation_url
                    .clone()
                    .unwrap_or_else(|| "https://antigravity.google".to_string())
            })
    }
}

/// `Google AI Pro` / `Google AI Ultra` / … from a tier's name or id (the Pro /
/// Ultra ids are not documented; Antigravity-Manager labels the same way).
pub fn plan_label(tier: &Tier) -> String {
    let haystack = format!(
        "{} {}",
        tier.name.as_deref().unwrap_or(""),
        tier.id.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase();
    if haystack.contains("ultra") {
        "Google AI Ultra".into()
    } else if haystack.contains("pro") {
        "Google AI Pro".into()
    } else if haystack.contains("enterprise") {
        "Gemini Code Assist Enterprise".into()
    } else if haystack.contains("standard") {
        "Gemini Code Assist Standard".into()
    } else if haystack.contains("free") {
        "Google AI (free)".into()
    } else if haystack.contains("legacy") {
        "Gemini Code Assist".into()
    } else {
        tier.name
            .clone()
            .or_else(|| tier.id.clone())
            .unwrap_or_else(|| "Google AI".into())
    }
}

/// The identity shown on the card. `project` overrides what `loadCodeAssist` returned.
pub fn account_identity(
    user: Option<&UserInfo>,
    info: &CodeAssistInfo,
    project: Option<&str>,
) -> AccountIdentity {
    let plan = info.plan();
    AccountIdentity {
        email: user.and_then(|u| u.email.clone()),
        display_name: user.and_then(|u| u.name.clone()),
        plan_tier: plan.and_then(|t| t.id.clone()),
        plan_label: Some(plan.map(plan_label).unwrap_or_else(|| "Google AI".into())),
        account_id: user.and_then(|u| u.id.clone()),
        project_id: project.map(str::to_string).or_else(|| info.project.clone()),
    }
}

/// `{"metadata":{"ideType":"ANTIGRAVITY"}}`, plus the project when the user supplied one.
pub fn load_code_assist_body(project_hint: Option<&str>) -> Value {
    let mut body = Map::new();
    if let Some(project) = project_hint.filter(|p| !p.trim().is_empty()) {
        body.insert("cloudaicompanionProject".into(), json!(project.trim()));
    }
    body.insert("metadata".into(), json!({ "ideType": fp::IDE_TYPE }));
    Value::Object(body)
}

/// `onboardUser` body — snake_case, as CLIProxyAPI sends it for this client.
pub fn onboard_user_body(tier_id: &str, version: &str) -> Value {
    json!({
        "tier_id": tier_id,
        "metadata": {
            "ide_type": fp::IDE_TYPE,
            "ide_version": version,
            "ide_name": "antigravity"
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OnboardState {
    pub done: bool,
    pub project: Option<String>,
}

/// The long-running operation: `done` and `response.cloudaicompanionProject.id`.
pub fn parse_onboard_response(body: &str) -> Result<OnboardState, serde_json::Error> {
    let value: Value = serde_json::from_str(body)?;
    let done = value.get("done").and_then(Value::as_bool).unwrap_or(false);
    let project = value
        .get("response")
        .and_then(|r| r.get("cloudaicompanionProject"))
        .and_then(|p| project_of(Some(p)))
        .or_else(|| project_of(value.get("cloudaicompanionProject")));
    Ok(OnboardState { done, project })
}

pub fn load_code_assist_url() -> String {
    format!("{}/v1internal:loadCodeAssist", fp::PROD_UPSTREAM)
}

pub fn onboard_user_url(host: &str) -> String {
    format!("{}/v1internal:onboardUser", host.trim_end_matches('/'))
}

pub fn models_url(host: &str) -> String {
    format!(
        "{}/v1internal:fetchAvailableModels",
        host.trim_end_matches('/')
    )
}

pub fn quota_url(host: &str) -> String {
    format!(
        "{}/v1internal:retrieveUserQuota",
        host.trim_end_matches('/')
    )
}

pub fn stream_url(host: &str) -> String {
    format!(
        "{}/v1internal:streamGenerateContent?alt=sse",
        host.trim_end_matches('/')
    )
}

pub fn generate_url(host: &str) -> String {
    format!("{}/v1internal:generateContent", host.trim_end_matches('/'))
}

// ── Client version / User-Agent ──────────────────────────────────────────────

/// `version: 2.12.2` out of the electron-builder manifest.
pub fn parse_manifest_version(yaml: &str) -> Option<String> {
    yaml.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("version:")?;
        let version = rest.trim().trim_matches(|c| c == '"' || c == '\'');
        (!version.is_empty()
            && version
                .split('.')
                .all(|p| p.chars().all(|c| c.is_ascii_digit())))
        .then(|| version.to_string())
    })
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|p| p.trim().parse::<u64>().unwrap_or(0))
        .collect()
}

pub fn version_at_least(version: &str, floor: &str) -> bool {
    let (a, b) = (version_parts(version), version_parts(floor));
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    true
}

/// The version the User-Agent carries: the manifest's when it parses and is at
/// least the floor, else the fingerprint's own.
pub fn effective_version(manifest: Option<&str>) -> String {
    manifest
        .and_then(parse_manifest_version)
        .filter(|v| version_at_least(v, VERSION_FLOOR))
        .unwrap_or_else(|| fp::CLIENT_VERSION.to_string())
}

/// `antigravity/hub/<version> darwin/<arch>` — CLIProxyAPI's family (disputed; capture settles).
pub fn user_agent(version: &str, arch: &str) -> String {
    format!("antigravity/hub/{version} darwin/{arch}")
}

/// The architecture label of this build for the User-Agent.
pub fn arch_label() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    }
}

// ── Catalog ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ModelEntry {
    pub id: String,
    pub display_name: Option<String>,
    pub max_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub remaining_fraction: Option<f64>,
    pub reset_time: Option<String>,
}

/// `fetchAvailableModels` → entries (internal ids skipped, sorted by id).
pub fn parse_models_response(body: &str) -> Result<Vec<ModelEntry>, serde_json::Error> {
    let value: Value = serde_json::from_str(body)?;
    let mut out = Vec::new();
    if let Some(models) = value.get("models").and_then(Value::as_object) {
        for (id, info) in models {
            if INTERNAL_MODEL_IDS.contains(&id.as_str()) || id.trim().is_empty() {
                continue;
            }
            let quota = info.get("quotaInfo");
            out.push(ModelEntry {
                id: id.clone(),
                display_name: info
                    .get("displayName")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string),
                max_tokens: info
                    .get("maxTokens")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
                max_output_tokens: info
                    .get("maxOutputTokens")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
                remaining_fraction: quota
                    .and_then(|q| q.get("remainingFraction"))
                    .and_then(Value::as_f64),
                reset_time: quota
                    .and_then(|q| q.get("resetTime"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Which pool line a model id belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Gemini,
    Claude,
    GptOss,
    Other,
}

pub fn family(model: &str) -> Family {
    let m = model.trim().to_ascii_lowercase();
    if m.starts_with("gemini") {
        Family::Gemini
    } else if m.starts_with("claude") {
        Family::Claude
    } else if m.starts_with("gpt-oss") {
        Family::GptOss
    } else {
        Family::Other
    }
}

/// `thinkingLevel: minimal` is accepted only by the bare Flash lines of the pool.
fn accepts_minimal(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    m == "gemini-3-flash" || m.starts_with("gemini-3.1-flash-lite") || m.contains("flash-image")
}

/// The reasoning levels the card offers per model (CLIProxyAPI `models.json`).
pub fn reasoning_levels(model: &str) -> Vec<String> {
    let levels: &[&str] = match family(model) {
        Family::Gemini if model.contains("flash-image") => &["minimal", "high"],
        Family::Gemini if accepts_minimal(model) => &["minimal", "low", "medium", "high"],
        Family::Gemini => &["low", "medium", "high"],
        Family::Claude => &["low", "medium", "high"],
        Family::GptOss | Family::Other => &[],
    };
    levels.iter().map(|l| l.to_string()).collect()
}

fn default_context_window(model: &str) -> u32 {
    match family(model) {
        Family::Gemini => 1_048_576,
        Family::Claude => 200_000,
        Family::GptOss => 114_000,
        Family::Other => 128_000,
    }
}

fn curated_label(id: &str) -> Option<&'static str> {
    CURATED_MODELS
        .iter()
        .find(|(model, _)| *model == id)
        .map(|(_, label)| *label)
}

/// `gemini-3.8-flash-high` → 3.8; `gemini-pro-agent` → none.
fn gemini_version(id: &str) -> Option<f64> {
    let rest = id.strip_prefix("gemini-")?;
    let number: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    number.parse::<f64>().ok()
}

fn newest(ids: &[String], keep: impl Fn(&str) -> bool) -> Option<&String> {
    ids.iter().filter(|id| keep(id)).max_by(|a, b| {
        gemini_version(a)
            .unwrap_or(0.0)
            .partial_cmp(&gemini_version(b).unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(b))
    })
}

/// The preset roles (docs: `fast` = 3.1 Flash-Lite, `default`/`vision` = the newest
/// Flash High, `reasoning`/`research` = Opus thinking, else Gemini Pro High).
fn preset_roles(ids: &[String]) -> Vec<(String, Vec<ModelRole>)> {
    let mut presets: Vec<(String, Vec<ModelRole>)> = Vec::new();
    let mut add = |id: Option<&String>, roles: &[ModelRole]| {
        if let Some(id) = id {
            match presets.iter_mut().find(|(m, _)| m == id) {
                Some((_, existing)) => existing.extend_from_slice(roles),
                None => presets.push((id.clone(), roles.to_vec())),
            }
        }
    };
    let has = |wanted: &str| ids.iter().find(|id| id.as_str() == wanted);
    let fast = has("gemini-3.1-flash-lite")
        .or_else(|| newest(ids, |id| id.contains("flash-lite")))
        .or_else(|| newest(ids, |id| id.contains("flash") && !id.contains("image")));
    add(fast, &[ModelRole::Fast]);
    let default = newest(ids, |id| {
        id.starts_with("gemini-") && id.ends_with("-flash-high")
    })
    .or_else(|| {
        newest(ids, |id| {
            id.starts_with("gemini-") && id.contains("flash") && !id.contains("image")
        })
    })
    .or_else(|| has("claude-sonnet-4-6"));
    add(default, &[ModelRole::Default, ModelRole::Vision]);
    let reasoning = has("claude-opus-4-6-thinking")
        .or_else(|| has("gemini-pro-agent"))
        .or_else(|| newest(ids, |id| id.contains("pro")))
        .or_else(|| ids.iter().find(|id| id.contains("thinking")));
    add(reasoning, &[ModelRole::Reasoning, ModelRole::Research]);
    presets
}

/// Catalog models with capabilities, the `antigravity` quota pool and Bluey's presets.
pub fn catalog_models(entries: &[ModelEntry]) -> Vec<CatalogModel> {
    let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
    let presets = preset_roles(&ids);
    entries
        .iter()
        .map(|entry| CatalogModel {
            id: entry.id.clone(),
            label: entry
                .display_name
                .clone()
                .or_else(|| curated_label(&entry.id).map(str::to_string))
                .unwrap_or_else(|| entry.id.clone()),
            capabilities: ModelCapabilities {
                vision: family(&entry.id) != Family::GptOss,
                tools: true,
                reasoning_levels: reasoning_levels(&entry.id),
                streaming: true,
                context_window: Some(
                    entry
                        .max_tokens
                        .unwrap_or_else(|| default_context_window(&entry.id)),
                ),
            },
            quota_pool: Some("antigravity".into()),
            suggested_roles: presets
                .iter()
                .find(|(id, _)| *id == entry.id)
                .map(|(_, roles)| roles.clone())
                .unwrap_or_default(),
        })
        .collect()
}

/// The curated pool as a catalog (`CatalogSource::Curated`).
pub fn curated_catalog() -> Vec<CatalogModel> {
    let entries: Vec<ModelEntry> = CURATED_MODELS
        .iter()
        .map(|(id, label)| ModelEntry {
            id: id.to_string(),
            display_name: Some(label.to_string()),
            max_tokens: None,
            max_output_tokens: None,
            remaining_fraction: None,
            reset_time: None,
        })
        .collect();
    catalog_models(&entries)
}

// ── Request normalisation & the wrapper ──────────────────────────────────────

fn thinking_budget(reasoning: ReasoningLevel) -> Option<u32> {
    match reasoning {
        ReasoningLevel::None => None,
        ReasoningLevel::Light => Some(4096),
        ReasoningLevel::Deep => Some(16_000),
    }
}

/// Make a Gemini `generateContent` body fit the pool's model line: `safetySettings`
/// gone; Gemini keeps `thinkingLevel` (`minimal` → `low` where the line rejects it);
/// Claude takes a `thinkingBudget` below `maxOutputTokens` (≥ 1024 or none) and
/// no `temperature` next to it; GPT-OSS and unknown lines carry no thinking
/// config; `systemInstruction` gets the `user` role the references send.
pub fn normalise_request(inner: &mut Value, model: &str, reasoning: ReasoningLevel) {
    let Some(body) = inner.as_object_mut() else {
        return;
    };
    body.remove("safetySettings");
    if let Some(system) = body
        .get_mut("systemInstruction")
        .and_then(Value::as_object_mut)
    {
        system.insert("role".into(), json!("user"));
    }
    let mut config = body
        .remove("generationConfig")
        .and_then(|c| c.as_object().cloned())
        .unwrap_or_default();
    match family(model) {
        Family::Gemini => {
            let level = config
                .get("thinkingConfig")
                .and_then(|t| t.get("thinkingLevel"))
                .and_then(Value::as_str)
                .map(str::to_string);
            if level.as_deref() == Some("minimal") && !accepts_minimal(model) {
                config.insert("thinkingConfig".into(), json!({ "thinkingLevel": "low" }));
            }
        }
        Family::Claude => {
            config.remove("thinkingConfig");
            let max_out = config
                .get("maxOutputTokens")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            let budget = thinking_budget(reasoning).map(|wanted| match max_out {
                Some(max) => wanted.min(max.saturating_sub(1)),
                None => wanted,
            });
            if let Some(budget) = budget.filter(|b| *b >= 1024) {
                config.insert(
                    "thinkingConfig".into(),
                    json!({ "thinkingBudget": budget, "includeThoughts": false }),
                );
                config.remove("temperature");
            }
        }
        Family::GptOss | Family::Other => {
            config.remove("thinkingConfig");
        }
    }
    if !config.is_empty() {
        body.insert("generationConfig".into(), Value::Object(config));
    }
}

/// `request.sessionId`: `-<63-bit int>` derived from the conversation id (stable
/// per Bluey session, as CLIProxyAPI derives it from the first user text).
pub fn session_id_for(conversation: &str) -> String {
    let digest = Sha256::digest(conversation.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    format!("-{}", u64::from_be_bytes(bytes) >> 1)
}

/// `agent`, or `image_gen` for the image line.
pub fn request_type(model: &str) -> &'static str {
    if model.to_ascii_lowercase().contains("image") {
        "image_gen"
    } else {
        fp::REQUEST_TYPE
    }
}

/// The Cloud Code wrapper around a Gemini request.
pub fn wrap_request(
    mut inner: Value,
    model: &str,
    project: &str,
    conversation_id: &str,
    request_id: &str,
) -> Value {
    if let Some(map) = inner.as_object_mut() {
        map.insert("sessionId".into(), json!(session_id_for(conversation_id)));
    }
    json!({
        "model": model,
        "project": project,
        "request": inner,
        "userAgent": fp::WRAPPER_USER_AGENT,
        "requestType": request_type(model),
        "requestId": format!("agent-{request_id}")
    })
}

/// Prepend the Antigravity identity text to the system instruction (probe A/B only).
pub fn inject_identity(inner: &mut Value) {
    let Some(body) = inner.as_object_mut() else {
        return;
    };
    let entry = body
        .entry("systemInstruction")
        .or_insert_with(|| json!({ "role": "user", "parts": [] }));
    if let Some(parts) = entry.get_mut("parts").and_then(Value::as_array_mut) {
        parts.insert(0, json!({ "text": IDENTITY_TEXT }));
    }
}

fn is_wrapped(body: &Value) -> bool {
    body.get("request").is_some() && body.get("userAgent").is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudCodeEndpoint {
    Generation,
    CountTokens,
    LoadCodeAssist,
    OnboardUser,
    Other,
}

fn endpoint_of(url: &str) -> CloudCodeEndpoint {
    let Some(idx) = url.find("/v1internal:") else {
        return CloudCodeEndpoint::Other;
    };
    let name = &url[idx + "/v1internal:".len()..];
    let name = name.split('?').next().unwrap_or("");
    match name {
        "streamGenerateContent" | "generateContent" => CloudCodeEndpoint::Generation,
        "countTokens" => CloudCodeEndpoint::CountTokens,
        "loadCodeAssist" => CloudCodeEndpoint::LoadCodeAssist,
        "onboardUser" => CloudCodeEndpoint::OnboardUser,
        _ => CloudCodeEndpoint::Other,
    }
}

/// The Antigravity request fingerprint: the thin header set (`Content-Type`,
/// `Authorization`, `User-Agent` — nothing Google-SDK-shaped) and the wrapper.
#[derive(Debug, Clone)]
pub struct AntigravityShaper {
    version: String,
    arch: &'static str,
}

impl AntigravityShaper {
    pub fn new(version: impl Into<String>, arch: &'static str) -> Self {
        Self {
            version: version.into(),
            arch,
        }
    }

    pub fn user_agent(&self) -> String {
        user_agent(&self.version, self.arch)
    }
}

impl Default for AntigravityShaper {
    fn default() -> Self {
        Self::new(fp::CLIENT_VERSION, arch_label())
    }
}

impl RequestShaper for AntigravityShaper {
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
        let endpoint = endpoint_of(&request.url);
        if endpoint == CloudCodeEndpoint::Generation && !is_wrapped(&request.body) {
            if !request.body.is_object() {
                return Err(ShapeError::InvalidBody(
                    "a Gemini generateContent object".into(),
                ));
            }
            if ctx.model.is_empty() {
                return Err(ShapeError::Missing("model"));
            }
            let project = ctx
                .provider_account_id
                .filter(|p| !p.trim().is_empty())
                .ok_or(ShapeError::Missing("the Cloud Code project id"))?;
            let inner = std::mem::take(&mut request.body);
            request.body = wrap_request(inner, ctx.model, project, ctx.session_id, ctx.request_id);
        }
        // Whitelist: everything the caller set goes; the native client sends exactly these.
        request.headers.clear();
        request.set_header("content-type", "application/json");
        request.set_header("authorization", &format!("Bearer {token}"));
        match endpoint {
            CloudCodeEndpoint::LoadCodeAssist => {
                request.set_header("user-agent", &self.user_agent());
                request.set_header("accept", "*/*");
            }
            CloudCodeEndpoint::OnboardUser => {
                request.set_header(
                    "user-agent",
                    &format!("{}{ONBOARD_USER_AGENT_SUFFIX}", self.user_agent()),
                );
                request.set_header("accept", "*/*");
                request.set_header("x-goog-api-client", ONBOARD_API_CLIENT);
            }
            CloudCodeEndpoint::Generation
            | CloudCodeEndpoint::CountTokens
            | CloudCodeEndpoint::Other => {
                request.set_header("user-agent", &self.user_agent());
            }
        }
        Ok(())
    }

    fn detect_drift(
        &self,
        status: u16,
        body: &str,
        _headers: &[(String, String)],
    ) -> Option<UnavailableReason> {
        drift_reason(status, body)
    }
}

// ── Responses & errors ───────────────────────────────────────────────────────

/// `error.message` of a Google error envelope (kept for the account card; never a prompt).
pub fn error_message(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .get("error")?
        .get("message")?
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The verification link of a `VALIDATION_REQUIRED` 403 (Help detail, else `ErrorInfo.metadata`).
pub fn validation_link(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let details = value.get("error")?.get("details")?.as_array()?;
    let from_help = details.iter().find_map(|d| {
        d.get("links")?
            .as_array()?
            .first()?
            .get("url")?
            .as_str()
            .map(str::to_string)
    });
    from_help.or_else(|| {
        details.iter().find_map(|d| {
            d.get("metadata")?
                .get("validation_link")?
                .as_str()
                .map(str::to_string)
        })
    })
}

fn lower(text: &str) -> String {
    text.to_ascii_lowercase()
}

fn is_tos_ban(message: &str) -> bool {
    let m = lower(message);
    m.contains("violation of terms of service")
        || m.contains("disabled in this account")
        || m.contains("submit an appeal")
}

fn is_validation_required(body: &str, message: &str) -> bool {
    lower(message).contains("validation_required")
        || gemini::parse_error_body(body)
            .and_then(|e| e.reason)
            .as_deref()
            == Some("VALIDATION_REQUIRED")
}

fn is_version_rejection(message: &str) -> bool {
    lower(message).contains("no longer supported")
}

fn is_wrapper_complaint(message: &str) -> bool {
    let m = lower(message);
    m.contains("unknown name")
        || m.contains("invalid json payload")
        || m.contains("invalid value at")
        || m.contains("cannot find field")
}

/// Which stop signal a response carries, if any (the `RequestShaper` hook).
pub fn drift_reason(status: u16, body: &str) -> Option<UnavailableReason> {
    let message = error_message(body).unwrap_or_default();
    if is_version_rejection(&message) {
        return Some(UnavailableReason::FingerprintDrift);
    }
    match status {
        403 => Some(UnavailableReason::PolicyBlocked),
        400 if is_wrapper_complaint(&message) => Some(UnavailableReason::FingerprintDrift),
        _ => None,
    }
}

pub fn needs_reauth() -> BlueyError {
    BlueyError::new(
        BlueyErrorKind::Authentication,
        bluey_core::accounts::codes::NEEDS_REAUTH,
        "Google rejected the sign-in — reconnect the account",
    )
    .recoverable(RecoveryAction::reconnect_account(
        ANTIGRAVITY_PROVIDER_ID,
        ANTIGRAVITY_PROVIDER_ID,
    ))
}

pub fn fingerprint_drift(message: impl Into<String>) -> BlueyError {
    BlueyError::account("fingerprint_drift", message).recoverable(RecoveryAction::UseApiKey)
}

pub fn policy_blocked(message: impl Into<String>) -> BlueyError {
    BlueyError::account("policy_blocked", message).recoverable(RecoveryAction::UseApiKey)
}

/// `account.rate_limited` with the reset in `details.until` (no window label:
/// Cloud Code names none; the pools roll over 5-hour and weekly windows).
pub fn rate_limited(until_iso: &str, message: impl Into<String>) -> BlueyError {
    BlueyError::account("rate_limited", message)
        .with_details(json!({ "until": until_iso }))
        .recoverable(RecoveryAction::UseApiKey)
}

fn with_message(prefix: &str, message: &str) -> String {
    if message.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {message}")
    }
}

/// Map a non-2xx Cloud Code response. `now_unix` dates the rate-limit reset.
pub fn map_error(status: u16, body: &str, now_unix: u64) -> BlueyError {
    let message = error_message(body).unwrap_or_default();
    if is_version_rejection(&message) {
        return fingerprint_drift(format!(
            "Google no longer accepts this Antigravity client version (fingerprint {}, captured {}) — reconnect to pick up the current version from the update manifest",
            fp::VERSION,
            fp::CAPTURED_ON
        ));
    }
    match status {
        401 => needs_reauth(),
        403 if is_tos_ban(&message) => policy_blocked(with_message(
            "Google has suspended this account for third-party use and Bluey stopped using it",
            &message,
        )),
        403 if is_validation_required(body, &message) => policy_blocked(format!(
            "Google asks you to verify this account in the browser first: {} — then reconnect",
            validation_link(body).unwrap_or_else(|| "https://antigravity.google".into())
        )),
        403 => policy_blocked(with_message(
            "Google refused the request for this account (HTTP 403)",
            &message,
        )),
        404 => gemini::map_gemini_error(404, None),
        429 => {
            let info = gemini::parse_error_body(body);
            let reason = info.as_ref().and_then(|e| e.reason.clone()).unwrap_or_default();
            let delay = info.as_ref().and_then(|e| e.retry_after);
            let exhausted = matches!(
                reason.as_str(),
                "QUOTA_EXHAUSTED" | "INSUFFICIENT_G1_CREDITS_BALANCE"
            ) || info.as_ref().is_some_and(gemini::GeminiError::is_daily_quota)
                || delay.is_some_and(|d| d.as_secs() >= LONG_DELAY_SECS);
            if exhausted {
                let wait = delay.map(|d| d.as_secs()).unwrap_or(DEFAULT_COOLDOWN_SECS);
                let until = iso_from_unix(now_unix.saturating_add(wait.max(1)));
                rate_limited(
                    &until,
                    with_message("your Google AI plan window is used up", &message),
                )
            } else {
                gemini::map_gemini_error(429, info.as_ref())
            }
        }
        400 if is_wrapper_complaint(&message) => fingerprint_drift(format!(
            "Cloud Code rejected the request shape Antigravity {} sends (captured {}) — the fingerprint needs a re-capture: {message}",
            fp::CLIENT_VERSION,
            fp::CAPTURED_ON
        )),
        400 if lower(&message).contains("token count exceeds") => BlueyError::ai(
            "context_length_exceeded",
            "the request is longer than the model's context window",
        ),
        400 => BlueyError::ai(
            "invalid_request",
            with_message("Google AI rejected the request (HTTP 400)", &message),
        ),
        500..=599 => gemini::map_gemini_error(status, None),
        other => BlueyError::ai(
            &format!("http_{other}"),
            format!("Cloud Code returned HTTP {other}"),
        ),
    }
}

/// A mid-stream `data:` frame that is an error envelope, mapped; `None` for content.
pub fn map_stream_error(frame: &str, now_unix: u64) -> Option<BlueyError> {
    let value: Value = serde_json::from_str(frame).ok()?;
    let error = value.get("error")?.as_object()?;
    let status = error
        .get("code")
        .and_then(Value::as_u64)
        .and_then(|c| u16::try_from(c).ok())
        .unwrap_or(500);
    Some(map_error(status, frame, now_unix))
}

/// The `GenerateContentResponse` inside a Cloud Code frame (`{response, traceId}`);
/// `Ok(None)` when the frame carries no `response`.
pub fn envelope_response(data: &str) -> Result<Option<Value>, serde_json::Error> {
    let mut value: Value = serde_json::from_str(data)?;
    Ok(value
        .as_object_mut()
        .and_then(|map| map.remove("response"))
        .filter(Value::is_object))
}

/// `traceId` of a frame, for the developer log.
pub fn trace_id(data: &str) -> Option<String> {
    let value: Value = serde_json::from_str(data).ok()?;
    value.get("traceId")?.as_str().map(str::to_string)
}

// ── Time ─────────────────────────────────────────────────────────────────────

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 (`2026-09-11T12:34:56Z`, fractional seconds, `±HH:MM` offsets) → unix seconds.
pub fn unix_from_iso(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', 't', ' '])?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let offset_pos = rest
        .find(['Z', 'z', '+'])
        .or_else(|| rest.rfind('-').filter(|&i| i >= 5));
    let (time, zone) = match offset_pos {
        Some(pos) => rest.split_at(pos),
        None => (rest, ""),
    };
    let time = time.split('.').next().unwrap_or(time);
    let mut clock = time.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next().unwrap_or("0").parse().ok()?;
    let offset_secs: i64 = match zone {
        "" | "Z" | "z" => 0,
        signed => {
            let sign = if signed.starts_with('-') { -1 } else { 1 };
            let mut hm = signed[1..].split(':');
            let h: i64 = hm.next()?.parse().ok()?;
            let m: i64 = hm.next().unwrap_or("0").parse().ok()?;
            sign * (h * 3600 + m * 60)
        }
    };
    let total = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
        - offset_secs;
    u64::try_from(total).ok()
}

// ── Local sign-in (read-only import) ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Malformed(String),
    NoTokens,
}

/// The Keychain item of the standalone app / `agy`: `go-keyring-base64:<base64 JSON
/// {"token":{"access_token","token_type","refresh_token","expiry"},"auth_method"}>`.
pub fn parse_keychain_payload(raw: &str) -> Result<ImportedTokens, ImportError> {
    let raw = raw.trim();
    let json = match raw.strip_prefix(KEYCHAIN_PREFIX) {
        Some(encoded) => {
            let bytes = BASE64
                .decode(encoded.trim())
                .map_err(|_| ImportError::Malformed("the Keychain item is not base64".into()))?;
            String::from_utf8(bytes)
                .map_err(|_| ImportError::Malformed("the Keychain item is not UTF-8".into()))?
        }
        None => raw.to_string(),
    };
    let value: Value = serde_json::from_str(&json)
        .map_err(|_| ImportError::Malformed("the Keychain item is not JSON".into()))?;
    let token = value.get("token").unwrap_or(&value);
    let access_token = token
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or(ImportError::NoTokens)?
        .to_string();
    let refresh_token = token
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);
    let expires_at = token
        .get("expiry")
        .and_then(Value::as_str)
        .and_then(unix_from_iso)
        .or_else(|| {
            token
                .get("expiry_date")
                .and_then(Value::as_u64)
                .map(|ms| ms / 1000)
        });
    Ok(ImportedTokens {
        access_token,
        refresh_token,
        expires_at,
    })
}

// ── Probe ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub ok: bool,
    pub billed_to: BilledTo,
    pub message: String,
}

/// What a unary `generateContent` probe says: the pool answered (billed to the
/// plan — Cloud Code has no extra-usage tier), or why it did not.
pub fn probe_outcome(status: u16, body: &str, now_unix: u64) -> ProbeOutcome {
    if (200..300).contains(&status) {
        let trace = trace_id(body)
            .map(|t| format!(" (trace {t})"))
            .unwrap_or_default();
        return ProbeOutcome {
            ok: true,
            billed_to: BilledTo::Plan,
            message: format!("Cloud Code answered on the Antigravity pool{trace}"),
        };
    }
    let error = map_error(status, body, now_unix);
    ProbeOutcome {
        ok: false,
        billed_to: BilledTo::Unknown,
        message: format!("HTTP {status} → {}: {}", error.code, error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprints::{
        self, diff, scrub_capture, Body, Capture, CaptureSource, CapturedRequest, FingerprintStamp,
        Header, Provider, SCHEMA_VERSION,
    };
    use bluey_core::types::{AiMessage, AiRole};
    use pretty_assertions::assert_eq;

    const NOW: u64 = 1_788_000_000;

    #[test]
    fn authorize_url_and_token_forms_follow_the_references() {
        let url = authorize_url("http://localhost:51121/oauth-callback", "chal", "st8");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains(&format!("client_id={CLIENT_ID}")));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A51121%2Foauth-callback"));
        assert!(url.contains("code_challenge=chal&code_challenge_method=S256&state=st8"));
        let scope = url
            .split("scope=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        assert_eq!(scope.matches("googleapis.com").count(), 5);
        assert!(scope.contains("cclog") && scope.contains("experimentsandconfigs"));
        assert_eq!(
            redirect_uri(LOOPBACK_PORT),
            "http://localhost:51121/oauth-callback"
        );

        let exchange = token_exchange_form(
            "c0de",
            "http://localhost:51121/oauth-callback",
            "ver",
            "GOCSPX-x",
        );
        assert_eq!(exchange[0], ("code", "c0de".to_string()));
        assert!(exchange.contains(&("client_secret", "GOCSPX-x".to_string())));
        assert!(exchange.contains(&("grant_type", "authorization_code".to_string())));
        assert!(exchange.contains(&("code_verifier", "ver".to_string())));
        let refresh = refresh_form("1//refresh", "GOCSPX-x");
        assert!(refresh.contains(&("client_secret", "GOCSPX-x".to_string())));
        assert!(refresh.contains(&("grant_type", "refresh_token".to_string())));
        assert_eq!(revoke_form("tok"), vec![("token", "tok".to_string())]);
    }

    #[test]
    fn load_code_assist_yields_project_plan_and_validation() {
        let info = parse_load_code_assist(
            r#"{"cloudaicompanionProject":"proj-123","currentTier":{"id":"g1-pro","name":"Google AI Pro","isDefault":false},
                "allowedTiers":[{"id":"free-tier","isDefault":false},{"id":"g1-pro","isDefault":true}],
                "paidTier":{"id":"g1-ultra","name":"Google AI Ultra","availableCredits":[{"creditType":"GOOGLE_ONE_AI","creditAmount":"25000"}]}}"#,
        )
        .unwrap();
        assert_eq!(info.project.as_deref(), Some("proj-123"));
        assert_eq!(plan_label(info.plan().unwrap()), "Google AI Ultra");
        assert_eq!(info.onboarding_tier(), "g1-pro");
        assert_eq!(info.validation_url(), None);
        let user =
            parse_userinfo(r#"{"id":"1029","email":"o@example.com","name":"Owner"}"#).unwrap();
        let identity = account_identity(Some(&user), &info, None);
        assert_eq!(identity.email.as_deref(), Some("o@example.com"));
        assert_eq!(identity.plan_label.as_deref(), Some("Google AI Ultra"));
        assert_eq!(identity.plan_tier.as_deref(), Some("g1-ultra"));
        assert_eq!(identity.project_id.as_deref(), Some("proj-123"));
        assert_eq!(identity.account_id.as_deref(), Some("1029"));
        let overridden = account_identity(None, &info, Some("workspace-proj"));
        assert_eq!(overridden.project_id.as_deref(), Some("workspace-proj"));

        let object_project = parse_load_code_assist(
            r#"{"cloudaicompanionProject":{"id":"proj-obj"},"currentTier":{"id":"standard-tier"}}"#,
        )
        .unwrap();
        assert_eq!(object_project.project.as_deref(), Some("proj-obj"));
        assert_eq!(
            plan_label(object_project.plan().unwrap()),
            "Gemini Code Assist Standard"
        );

        let blocked = parse_load_code_assist(
            r#"{"ineligibleTiers":[{"reasonCode":"VALIDATION_REQUIRED","validationUrl":"https://g.co/verify","tierId":"free-tier"}]}"#,
        )
        .unwrap();
        assert_eq!(blocked.project, None);
        assert_eq!(
            blocked.validation_url().as_deref(),
            Some("https://g.co/verify")
        );
        assert_eq!(blocked.onboarding_tier(), "free-tier");
        assert_eq!(
            plan_label(&Tier {
                id: Some("free-tier".into()),
                ..Tier::default()
            }),
            "Google AI (free)"
        );
        assert_eq!(
            plan_label(&Tier {
                name: Some("Legacy".into()),
                ..Tier::default()
            }),
            "Gemini Code Assist"
        );
    }

    #[test]
    fn setup_bodies_and_the_onboarding_lro() {
        assert_eq!(
            load_code_assist_body(None),
            json!({ "metadata": { "ideType": "ANTIGRAVITY" } })
        );
        assert_eq!(
            load_code_assist_body(Some(" my-proj ")),
            json!({ "cloudaicompanionProject": "my-proj", "metadata": { "ideType": "ANTIGRAVITY" } })
        );
        assert_eq!(
            onboard_user_body("free-tier", "2.12.2"),
            json!({ "tier_id": "free-tier", "metadata": { "ide_type": "ANTIGRAVITY", "ide_version": "2.12.2", "ide_name": "antigravity" } })
        );
        let pending = parse_onboard_response(r#"{"name":"operations/abc","done":false}"#).unwrap();
        assert_eq!(
            pending,
            OnboardState {
                done: false,
                project: None
            }
        );
        let done = parse_onboard_response(
            r#"{"done":true,"response":{"cloudaicompanionProject":{"id":"rising-fact-p41fc","name":"x"}}}"#,
        )
        .unwrap();
        assert_eq!(done.project.as_deref(), Some("rising-fact-p41fc"));
        assert!(done.done);
        assert_eq!(
            load_code_assist_url(),
            "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist"
        );
        assert_eq!(
            onboard_user_url(fp::UPSTREAM),
            "https://daily-cloudcode-pa.googleapis.com/v1internal:onboardUser"
        );
        assert_eq!(
            stream_url(fp::UPSTREAM),
            "https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            generate_url("https://h/"),
            "https://h/v1internal:generateContent"
        );
    }

    #[test]
    fn manifest_version_is_read_and_floored() {
        let yaml = "version: 2.12.2\nfiles:\n  - url: antigravity-hub/2.12.2-6298742303883264/darwin-arm/Antigravity.zip\npath: x\n";
        assert_eq!(parse_manifest_version(yaml).as_deref(), Some("2.12.2"));
        assert_eq!(
            parse_manifest_version("version: '3.0.1'").as_deref(),
            Some("3.0.1")
        );
        assert_eq!(parse_manifest_version("nope"), None);
        assert!(version_at_least("2.12.2", "2.9.1"));
        assert!(version_at_least("2.9.1", "2.9.1"));
        assert!(!version_at_least("2.8.9", "2.9.1"));
        assert!(!version_at_least("2.9", "2.9.1"));
        assert_eq!(effective_version(Some(yaml)), "2.12.2");
        assert_eq!(
            effective_version(Some("version: 2.0.6")),
            fp::CLIENT_VERSION,
            "below the floor → ours"
        );
        assert_eq!(effective_version(None), fp::CLIENT_VERSION);
        assert_eq!(user_agent("2.12.2", "arm64"), fp::USER_AGENT_EXAMPLE);
        assert!(regex::Regex::new(fp::USER_AGENT_PATTERN)
            .unwrap()
            .is_match(&user_agent("2.13.0", "x86_64")));
    }

    #[test]
    fn models_response_maps_to_a_catalog_with_presets() {
        let entries = parse_models_response(
            r#"{"models":{
                "gemini-3.8-flash-high":{"displayName":"Gemini 3.8 Flash","maxTokens":1048576,"maxOutputTokens":65536,"quotaInfo":{"remainingFraction":0.9,"resetTime":"2026-09-11T15:00:00Z"}},
                "gemini-3.7-flash-high":{"displayName":"Gemini 3.7 Flash"},
                "gemini-3.1-flash-lite":{"displayName":"Gemini 3.1 Flash-Lite"},
                "gemini-pro-agent":{"displayName":"Gemini 3.1 Pro (High)"},
                "claude-opus-4-6-thinking":{"displayName":"Claude Opus 4.6 (Thinking)","maxTokens":200000},
                "claude-sonnet-4-6":{},
                "gpt-oss-120b-medium":{"maxTokens":114000},
                "chat_20706":{"displayName":"internal"}
            },"webSearchModelIds":["gemini-3.8-flash-high"]}"#,
        )
        .unwrap();
        assert_eq!(entries.len(), 7, "internal ids are skipped");
        let catalog = catalog_models(&entries);
        let roles = |id: &str| {
            catalog
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.suggested_roles.clone())
                .unwrap()
        };
        assert_eq!(
            roles("gemini-3.8-flash-high"),
            vec![ModelRole::Default, ModelRole::Vision]
        );
        assert_eq!(roles("gemini-3.7-flash-high"), Vec::<ModelRole>::new());
        assert_eq!(roles("gemini-3.1-flash-lite"), vec![ModelRole::Fast]);
        assert_eq!(
            roles("claude-opus-4-6-thinking"),
            vec![ModelRole::Reasoning, ModelRole::Research]
        );
        let sonnet = catalog
            .iter()
            .find(|m| m.id == "claude-sonnet-4-6")
            .unwrap();
        assert_eq!(
            sonnet.label, "Claude Sonnet 4.6",
            "curated label fills a missing displayName"
        );
        assert_eq!(sonnet.capabilities.context_window, Some(200_000));
        assert_eq!(sonnet.quota_pool.as_deref(), Some("antigravity"));
        let oss = catalog
            .iter()
            .find(|m| m.id == "gpt-oss-120b-medium")
            .unwrap();
        assert!(!oss.capabilities.vision);
        assert!(oss.capabilities.reasoning_levels.is_empty());
        assert_eq!(
            catalog
                .iter()
                .find(|m| m.id == "gemini-3.1-flash-lite")
                .unwrap()
                .capabilities
                .reasoning_levels,
            vec!["minimal", "low", "medium", "high"]
        );
        assert_eq!(
            catalog
                .iter()
                .find(|m| m.id == "gemini-3.8-flash-high")
                .unwrap()
                .capabilities
                .reasoning_levels,
            vec!["low", "medium", "high"]
        );

        let curated = curated_catalog();
        assert_eq!(curated.len(), CURATED_MODELS.len());
        let curated_roles = |id: &str| {
            curated
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .suggested_roles
                .clone()
        };
        assert_eq!(
            curated_roles("gemini-3.8-flash-high"),
            vec![ModelRole::Default, ModelRole::Vision]
        );
        assert_eq!(
            curated_roles("gemini-3.1-flash-lite"),
            vec![ModelRole::Fast]
        );
        assert_eq!(
            curated_roles("claude-opus-4-6-thinking"),
            vec![ModelRole::Reasoning, ModelRole::Research]
        );

        // Without Opus, Pro High takes reasoning; without a Flash High line, the newest Flash.
        let gemini_only: Vec<ModelEntry> = ["gemini-3-flash", "gemini-pro-agent"]
            .iter()
            .map(|id| ModelEntry {
                id: id.to_string(),
                display_name: None,
                max_tokens: None,
                max_output_tokens: None,
                remaining_fraction: None,
                reset_time: None,
            })
            .collect();
        let small = catalog_models(&gemini_only);
        assert_eq!(
            small
                .iter()
                .find(|m| m.id == "gemini-pro-agent")
                .unwrap()
                .suggested_roles,
            vec![ModelRole::Reasoning, ModelRole::Research]
        );
        assert_eq!(
            small
                .iter()
                .find(|m| m.id == "gemini-3-flash")
                .unwrap()
                .suggested_roles,
            vec![ModelRole::Fast, ModelRole::Default, ModelRole::Vision]
        );
    }

    fn gemini_body(model: &str, level: Option<gemini::ThinkingLevel>, max: Option<u32>) -> Value {
        gemini::build_generate_body(&gemini::GenerateBodyOptions {
            model,
            messages: &[
                AiMessage::text(AiRole::System, "You are Bluey, a discreet copilot."),
                AiMessage::text(AiRole::User, "What is on my screen?"),
            ],
            max_output_tokens: max,
            temperature: Some(0.3),
            output_schema: None,
            thinking_level: level,
        })
    }

    #[test]
    fn requests_are_normalised_per_model_line() {
        let mut flash_high = gemini_body(
            "gemini-3.8-flash-high",
            Some(gemini::ThinkingLevel::Minimal),
            Some(400),
        );
        flash_high["safetySettings"] = json!([]);
        normalise_request(
            &mut flash_high,
            "gemini-3.8-flash-high",
            ReasoningLevel::None,
        );
        assert_eq!(
            flash_high["generationConfig"]["thinkingConfig"]["thinkingLevel"], "low",
            "minimal → low on the High line"
        );
        assert!(flash_high.get("safetySettings").is_none());
        assert_eq!(flash_high["systemInstruction"]["role"], "user");
        assert_eq!(
            flash_high["generationConfig"]["maxOutputTokens"], 400,
            "the cap stays"
        );
        assert!(
            flash_high["generationConfig"].get("temperature").is_none(),
            "3.x bodies never carry it"
        );

        let mut lite = gemini_body(
            "gemini-3.1-flash-lite",
            Some(gemini::ThinkingLevel::Minimal),
            None,
        );
        normalise_request(&mut lite, "gemini-3.1-flash-lite", ReasoningLevel::None);
        assert_eq!(
            lite["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "minimal"
        );

        let mut opus = gemini_body("claude-opus-4-6-thinking", None, Some(2048));
        normalise_request(&mut opus, "claude-opus-4-6-thinking", ReasoningLevel::Deep);
        assert_eq!(
            opus["generationConfig"]["thinkingConfig"],
            json!({ "thinkingBudget": 2047, "includeThoughts": false }),
            "budget stays below maxOutputTokens"
        );
        assert!(
            opus["generationConfig"].get("temperature").is_none(),
            "no temperature next to thinking"
        );

        let mut sonnet = gemini_body("claude-sonnet-4-6", None, Some(400));
        normalise_request(&mut sonnet, "claude-sonnet-4-6", ReasoningLevel::Light);
        assert!(
            sonnet["generationConfig"].get("thinkingConfig").is_none(),
            "a 399-token budget is below the 1024 minimum → none"
        );
        let temperature = sonnet["generationConfig"]["temperature"].as_f64().unwrap();
        assert!((temperature - 0.3).abs() < 1e-6, "{temperature}");

        let mut sonnet_free = gemini_body("claude-sonnet-4-6", None, None);
        normalise_request(&mut sonnet_free, "claude-sonnet-4-6", ReasoningLevel::Light);
        assert_eq!(
            sonnet_free["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            4096
        );

        let mut oss = gemini_body("gpt-oss-120b-medium", None, None);
        oss["generationConfig"] = json!({ "thinkingConfig": { "thinkingLevel": "high" } });
        normalise_request(&mut oss, "gpt-oss-120b-medium", ReasoningLevel::Deep);
        assert!(
            oss.get("generationConfig").is_none(),
            "an emptied config is dropped"
        );

        assert_eq!(request_type("gemini-3.1-flash-image"), "image_gen");
        assert_eq!(request_type("gemini-3.8-flash-high"), "agent");
        assert_eq!(family("gpt-oss-120b-medium"), Family::GptOss);
        assert_eq!(family("mystery"), Family::Other);
        let session = session_id_for("6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b");
        assert!(session.starts_with('-') && session[1..].chars().all(|c| c.is_ascii_digit()));
        assert_eq!(
            session,
            session_id_for("6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b"),
            "stable per conversation"
        );
        assert_ne!(session, session_id_for("other"));
        let mut with_identity = json!({ "contents": [] });
        inject_identity(&mut with_identity);
        assert_eq!(
            with_identity["systemInstruction"]["parts"][0]["text"],
            IDENTITY_TEXT
        );
        assert_eq!(with_identity["systemInstruction"]["role"], "user");
    }

    fn ctx<'a>(model: &'a str, project: Option<&'a str>) -> ShapeContext<'a> {
        ShapeContext {
            account_id: "antigravity",
            provider_account_id: project,
            device_id: "d",
            session_id: "6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b",
            request_id: "7a1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7c",
            model,
            access_token: Some("ya29.SECRETSECRETSECRETSECRET"),
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
            provider: "antigravity".into(),
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
        scrub_capture(&mut capture, Provider::Antigravity.rules());
        capture
    }

    fn documented(endpoint: &str) -> Capture {
        Provider::Antigravity
            .documented()
            .into_iter()
            .find(|c| {
                Provider::Antigravity
                    .rules()
                    .endpoint_for(&c.request.method, &c.request.path())
                    .is_some_and(|e| e.name == endpoint)
            })
            .expect("documented capture")
    }

    #[test]
    fn the_shaper_reproduces_the_documented_fingerprint() {
        let shaper = AntigravityShaper::new("2.12.2", "arm64");
        let mut body = gemini_body(
            "gemini-3.8-flash-high",
            Some(gemini::ThinkingLevel::Medium),
            None,
        );
        normalise_request(&mut body, "gemini-3.8-flash-high", ReasoningLevel::None);
        let mut request = ProviderHttpRequest::new("POST", &stream_url(fp::UPSTREAM), body);
        request
            .headers
            .push(("x-goog-api-client".into(), "gl-node/22".into()));
        request
            .headers
            .push(("x-api-key".into(), "AIza-should-go".into()));
        shaper
            .shape(
                &mut request,
                &ctx("gemini-3.8-flash-high", Some("proj-123")),
            )
            .unwrap();

        assert_eq!(
            request.header("x-goog-api-client"),
            None,
            "whitelist: nothing SDK-shaped"
        );
        assert_eq!(request.header("x-api-key"), None);
        assert_eq!(
            request.header("authorization"),
            Some("Bearer ya29.SECRETSECRETSECRETSECRET")
        );
        assert_eq!(request.header("user-agent"), Some(fp::USER_AGENT_EXAMPLE));
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert_eq!(request.headers.len(), 3);
        assert_eq!(request.body["model"], "gemini-3.8-flash-high");
        assert_eq!(request.body["project"], "proj-123");
        assert_eq!(request.body["userAgent"], "antigravity");
        assert_eq!(request.body["requestType"], "agent");
        assert_eq!(
            request.body["requestId"],
            "agent-7a1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7c"
        );
        assert_eq!(
            request.body["request"]["sessionId"],
            session_id_for("6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b")
        );
        assert_eq!(request.body["request"]["contents"][0]["role"], "user");
        assert_eq!(request.body["request"]["systemInstruction"]["role"], "user");
        assert_eq!(
            request.body["request"]["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "medium"
        );
        assert!(request.body["request"].get("safetySettings").is_none());

        let report = diff(
            Provider::Antigravity.rules(),
            &documented("stream"),
            &capture_of(&request),
            "documented",
            "shaper",
        );
        assert!(!report.has_drift(), "{}", report.render());

        // Shaping twice does not wrap twice.
        let before = request.body.clone();
        shaper
            .shape(
                &mut request,
                &ctx("gemini-3.8-flash-high", Some("proj-123")),
            )
            .unwrap();
        assert_eq!(request.body, before);

        // loadCodeAssist and the model list keep their bodies and get their headers.
        let mut load =
            ProviderHttpRequest::new("POST", &load_code_assist_url(), load_code_assist_body(None));
        shaper.shape(&mut load, &ctx("", Some("proj-123"))).unwrap();
        assert_eq!(load.header("accept"), Some("*/*"));
        assert_eq!(load.header("x-goog-api-client"), None);
        assert_eq!(
            load.body,
            json!({ "metadata": { "ideType": "ANTIGRAVITY" } })
        );
        let report = diff(
            Provider::Antigravity.rules(),
            &documented("load_code_assist"),
            &capture_of(&load),
            "documented",
            "shaper",
        );
        assert!(!report.has_drift(), "{}", report.render());

        let mut models = ProviderHttpRequest::new(
            "POST",
            &models_url(fp::UPSTREAM),
            json!({ "project": "proj-123" }),
        );
        shaper.shape(&mut models, &ctx("", None)).unwrap();
        assert_eq!(models.body, json!({ "project": "proj-123" }));
        let report = diff(
            Provider::Antigravity.rules(),
            &documented("models"),
            &capture_of(&models),
            "documented",
            "shaper",
        );
        assert!(!report.has_drift(), "{}", report.render());

        let mut onboard = ProviderHttpRequest::new(
            "POST",
            &onboard_user_url(fp::UPSTREAM),
            onboard_user_body("free-tier", "2.12.2"),
        );
        shaper.shape(&mut onboard, &ctx("", None)).unwrap();
        assert_eq!(
            onboard.header("x-goog-api-client"),
            Some(ONBOARD_API_CLIENT)
        );
        assert_eq!(
            onboard.header("user-agent"),
            Some("antigravity/hub/2.12.2 darwin/arm64 google-api-nodejs-client/10.3.0")
        );

        // What the shaper needs.
        let mut no_project =
            ProviderHttpRequest::new("POST", &stream_url(fp::UPSTREAM), json!({ "contents": [] }));
        assert_eq!(
            shaper.shape(&mut no_project, &ctx("gemini-3-flash", None)),
            Err(ShapeError::Missing("the Cloud Code project id"))
        );
        let mut no_token =
            ProviderHttpRequest::new("POST", &stream_url(fp::UPSTREAM), json!({ "contents": [] }));
        let mut without = ctx("gemini-3-flash", Some("p"));
        without.access_token = None;
        assert_eq!(
            shaper.shape(&mut no_token, &without),
            Err(ShapeError::Missing("access_token"))
        );
        assert_eq!(shaper.fingerprint(), fp::INFO);
        assert_eq!(
            AntigravityShaper::default().user_agent(),
            user_agent(fp::CLIENT_VERSION, arch_label())
        );
    }

    #[test]
    fn errors_map_to_account_statuses() {
        assert_eq!(map_error(401, "", NOW).code, "account.needs_reauth");

        let tos = r#"{"error":{"code":403,"message":"This service has been disabled in this account for violation of Terms of Service. If you believe this is an error, contact gemini-code-assist-user-feedback@google.com.","status":"PERMISSION_DENIED"}}"#;
        let banned = map_error(403, tos, NOW);
        assert_eq!(banned.code, "account.policy_blocked");
        assert!(
            banned
                .message
                .contains("gemini-code-assist-user-feedback@google.com"),
            "{}",
            banned.message
        );
        assert_eq!(
            drift_reason(403, tos),
            Some(UnavailableReason::PolicyBlocked)
        );

        let validation = r#"{"error":{"code":403,"message":"Verification required","status":"PERMISSION_DENIED","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"VALIDATION_REQUIRED","domain":"cloudcode-pa.googleapis.com"},{"@type":"type.googleapis.com/google.rpc.Help","links":[{"description":"verify","url":"https://g.co/verify"}]}]}}"#;
        let verify = map_error(403, validation, NOW);
        assert_eq!(verify.code, "account.policy_blocked");
        assert!(
            verify.message.contains("https://g.co/verify"),
            "{}",
            verify.message
        );
        assert_eq!(
            validation_link(validation).as_deref(),
            Some("https://g.co/verify")
        );

        let generic = map_error(
            403,
            r#"{"error":{"code":403,"message":"The caller does not have permission","status":"PERMISSION_DENIED"}}"#,
            NOW,
        );
        assert_eq!(generic.code, "account.policy_blocked");
        assert!(generic
            .message
            .contains("The caller does not have permission"));

        let old = r#"{"error":{"code":400,"message":"This version of Antigravity is no longer supported","status":"FAILED_PRECONDITION"}}"#;
        assert_eq!(map_error(400, old, NOW).code, "account.fingerprint_drift");
        assert_eq!(
            drift_reason(400, old),
            Some(UnavailableReason::FingerprintDrift)
        );
        let wrapper = r#"{"error":{"code":400,"message":"Invalid JSON payload received. Unknown name \"userAgent\": Cannot find field.","status":"INVALID_ARGUMENT"}}"#;
        assert_eq!(
            map_error(400, wrapper, NOW).code,
            "account.fingerprint_drift"
        );
        assert_eq!(
            drift_reason(400, wrapper),
            Some(UnavailableReason::FingerprintDrift)
        );
        let too_long = r#"{"error":{"code":400,"message":"The input token count exceeds the maximum number of tokens allowed","status":"INVALID_ARGUMENT"}}"#;
        assert_eq!(
            map_error(400, too_long, NOW).code,
            "ai.context_length_exceeded"
        );
        assert_eq!(drift_reason(400, too_long), None);
        let plain = map_error(
            400,
            r#"{"error":{"code":400,"message":"Request contains an invalid argument.","status":"INVALID_ARGUMENT"}}"#,
            NOW,
        );
        assert_eq!(plain.code, "ai.invalid_request");
        assert!(plain.message.contains("invalid argument"));

        let exhausted = r#"{"error":{"code":429,"message":"Quota exhausted","status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"3600s"},{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"QUOTA_EXHAUSTED","domain":"cloudcode-pa.googleapis.com"}]}}"#;
        let limited = map_error(429, exhausted, NOW);
        assert_eq!(limited.code, "account.rate_limited");
        assert_eq!(
            limited.details.as_ref().unwrap()["until"],
            iso_from_unix(NOW + 3600)
        );
        assert_eq!(
            bluey_core::accounts::status_after_error(&limited),
            Some(bluey_core::types::AccountStatus::RateLimited {
                until: iso_from_unix(NOW + 3600),
                window: None
            })
        );
        let no_delay = map_error(
            429,
            r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"QUOTA_EXHAUSTED"}]}}"#,
            NOW,
        );
        assert_eq!(
            no_delay.details.as_ref().unwrap()["until"],
            iso_from_unix(NOW + DEFAULT_COOLDOWN_SECS)
        );
        let long_delay = map_error(
            429,
            r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"900s"},{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"RATE_LIMIT_EXCEEDED"}]}}"#,
            NOW,
        );
        assert_eq!(
            long_delay.code, "account.rate_limited",
            "≥ 5 min is a window"
        );
        let capacity = map_error(
            429,
            r#"{"error":{"code":429,"message":"You have exhausted your capacity on this model. Your quota will reset after 3s.","status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"3.957525076s"},{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"MODEL_CAPACITY_EXHAUSTED"}]}}"#,
            NOW,
        );
        assert_eq!(
            capacity.code, "network.http_429",
            "capacity is a short retry, not a window"
        );
        assert_eq!(capacity.details.as_ref().unwrap()["retryAfterMs"], 3957);
        assert_eq!(drift_reason(429, ""), None);

        assert_eq!(
            map_error(
                404,
                r#"{"error":{"code":404,"message":"model not found"}}"#,
                NOW
            )
            .code,
            "config.model_not_found"
        );
        assert_eq!(map_error(503, "", NOW).code, "network.http_5xx");
        assert_eq!(map_error(418, "", NOW).code, "ai.http_418");

        let mid_stream = map_stream_error(r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"QUOTA_EXHAUSTED"}]}}"#, NOW).unwrap();
        assert_eq!(mid_stream.code, "account.rate_limited");
        assert!(map_stream_error(r#"{"response":{"candidates":[]}}"#, NOW).is_none());
        for error in [map_error(403, tos, NOW), map_error(400, old, NOW), limited] {
            assert!(
                !error.message.contains("ya29."),
                "never a token in a message"
            );
        }
    }

    #[test]
    fn stream_frames_unwrap_the_cloud_code_envelope() {
        let frame = r#"{"response":{"candidates":[{"content":{"role":"model","parts":[{"text":"Hello"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":1},"modelVersion":"gemini-3.8-flash-high"},"traceId":"abc123"}"#;
        let inner = envelope_response(frame).unwrap().unwrap();
        let parsed = gemini::parse_response(&inner.to_string()).unwrap();
        assert_eq!(parsed.text, "Hello");
        assert_eq!(parsed.finish.as_deref(), Some("STOP"));
        assert_eq!(parsed.usage.as_ref().and_then(|u| u.prompt), Some(12));
        assert_eq!(trace_id(frame).as_deref(), Some("abc123"));
        assert_eq!(envelope_response(r#"{"traceId":"x"}"#).unwrap(), None);
        assert!(envelope_response("not json").is_err());
        let probe = probe_outcome(200, frame, NOW);
        assert!(probe.ok);
        assert_eq!(probe.billed_to, BilledTo::Plan);
        assert!(probe.message.contains("abc123"));
        let refused = probe_outcome(
            403,
            r#"{"error":{"code":403,"message":"The caller does not have permission"}}"#,
            NOW,
        );
        assert!(!refused.ok);
        assert_eq!(refused.billed_to, BilledTo::Unknown);
        assert!(refused.message.contains("account.policy_blocked"));
    }

    #[test]
    fn keychain_payloads_and_timestamps_parse() {
        let json = r#"{"token":{"access_token":"ya29.abc","token_type":"Bearer","refresh_token":"1//refresh","expiry":"2026-09-11T14:30:00.123456Z"},"auth_method":"consumer"}"#;
        let raw = format!("{KEYCHAIN_PREFIX}{}", BASE64.encode(json));
        let imported = parse_keychain_payload(&raw).unwrap();
        assert_eq!(imported.access_token, "ya29.abc");
        assert_eq!(imported.refresh_token.as_deref(), Some("1//refresh"));
        assert_eq!(imported.expires_at, unix_from_iso("2026-09-11T14:30:00Z"));
        assert_eq!(
            iso_from_unix(imported.expires_at.unwrap()),
            "2026-09-11T14:30:00Z"
        );
        assert_eq!(
            parse_keychain_payload(json).unwrap(),
            imported,
            "plain JSON works too"
        );
        assert_eq!(
            parse_keychain_payload(r#"{"token":{"token_type":"Bearer"}}"#),
            Err(ImportError::NoTokens)
        );
        assert!(matches!(
            parse_keychain_payload("go-keyring-base64:!!!"),
            Err(ImportError::Malformed(_))
        ));
        assert!(matches!(
            parse_keychain_payload("nope"),
            Err(ImportError::Malformed(_))
        ));
        let gemini_cli =
            r#"{"access_token":"ya29.x","refresh_token":"1//y","expiry_date":1788000000000}"#;
        assert_eq!(
            parse_keychain_payload(gemini_cli).unwrap().expires_at,
            Some(1_788_000_000)
        );

        assert_eq!(unix_from_iso("2023-11-14T22:13:20Z"), Some(1_700_000_000));
        assert_eq!(
            unix_from_iso("2023-11-14T23:13:20+01:00"),
            Some(1_700_000_000)
        );
        assert_eq!(
            unix_from_iso("2023-11-14T17:13:20.5-05:00"),
            Some(1_700_000_000)
        );
        assert_eq!(unix_from_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_from_iso("2026-13-01T00:00:00Z"), None);
        assert_eq!(unix_from_iso("garbage"), None);
        for secs in [0u64, 951_782_400, 1_700_000_000, 4_102_444_800] {
            assert_eq!(
                unix_from_iso(&iso_from_unix(secs)),
                Some(secs),
                "round trip {secs}"
            );
        }
    }
}
