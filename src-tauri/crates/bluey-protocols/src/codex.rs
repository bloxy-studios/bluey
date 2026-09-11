//! ChatGPT through the Codex OAuth flow (ADR 0009 §4) — the pure half.
//!
//! Everything the ChatGPT account and adapter need that is not I/O: the OAuth
//! constants and bodies (PKCE loopback on 1455 → 1457, then the device-code
//! flow), identity from the ID token, `~/.codex/auth.json` import parsing, the
//! Responses API request builder, the SSE event parser and stream state, the
//! model catalog mapping, error mapping onto `account.*` codes, and
//! [`CodexShaper`] — the request fingerprint, built from the constants in
//! [`crate::fingerprints::codex`] so the shipped shaper and the documented
//! capture cannot disagree (a test diffs one against the other).
//!
//! Every wire fact here is a row of `docs/PROVIDER_ACCOUNTS.md › ChatGPT`
//! (verified 2026-09-11 against `openai/codex` rust-v0.154.0).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Map, Value};

use bluey_core::error::RecoveryAction;
use bluey_core::types::{
    AccountIdentity, AiContentPart, AiMessage, AiRole, BilledTo, CatalogModel, FinishReason,
    JsonSchemaSpec, LatencyBudget, ModelCapabilities, ModelRole, ReasoningLevel, UnavailableReason,
    CHATGPT_PROVIDER_ID,
};
use bluey_core::{BlueyError, BlueyErrorKind};

use crate::fingerprints::codex as fp;
use crate::oauth::{self, decode_jwt_payload};
use crate::request_shaper::{
    FingerprintInfo, ProviderHttpRequest, RequestShaper, ShapeContext, ShapeError,
};

// ─────────────────────────────────────────────────────────────────────────────
// OAuth
// ─────────────────────────────────────────────────────────────────────────────

/// The Codex CLI's public client id (no secret — PKCE proves the flow is ours).
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const REVOKE_URL: &str = "https://auth.openai.com/oauth/revoke";
pub const DEVICE_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
pub const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
/// Where the user types the device code (a page, not an API).
pub const DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";
/// The redirect URI of the code exchange after a device flow.
pub const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
/// The CLI's scope string (six scopes; the reference implementations send four — VERIFY).
pub const SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
/// Loopback ports the client registration allows, in the order Bluey tries them.
pub const LOOPBACK_PORTS: [u16; 2] = [1455, 1457];
pub const CALLBACK_PATH: &str = "/auth/callback";
pub const DEVICE_FLOW_EXPIRY_SECS: u64 = 15 * 60;
pub const DEFAULT_DEVICE_INTERVAL_SECS: u64 = 5;
/// PKCE verifier entropy of the CLI (64 bytes → 86 characters).
pub const PKCE_VERIFIER_BYTES: usize = 64;

pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CALLBACK_PATH}")
}

/// The authorization URL, parameters in the CLI's order (`login/src/server.rs:576-612`).
pub fn authorize_url(redirect_uri: &str, code_challenge: &str, state: &str) -> String {
    let mut url = url::Url::parse(AUTHORIZE_URL).expect("constant URL parses");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", fp::ORIGINATOR);
    url.to_string()
}

/// Form body of the code exchange (`application/x-www-form-urlencoded`).
pub fn token_exchange_form(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Vec<(&'static str, String)> {
    oauth::token_exchange_form(CLIENT_ID, code, redirect_uri, code_verifier)
}

/// JSON body of a refresh (the refresh grant is JSON, unlike the exchange).
pub fn refresh_body(refresh_token: &str) -> Value {
    json!({
        "client_id": CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
    })
}

pub fn revoke_body(token: &str) -> Value {
    json!({ "client_id": CLIENT_ID, "token": token })
}

pub fn device_usercode_body() -> Value {
    json!({ "client_id": CLIENT_ID })
}

pub fn device_poll_body(device_auth_id: &str, user_code: &str) -> Value {
    json!({ "device_auth_id": device_auth_id, "user_code": user_code })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeStart {
    pub device_auth_id: String,
    pub user_code: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `interval` arrives as a *string* of seconds; accept numbers too.
fn seconds_field(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
}

/// The `deviceauth/usercode` response.
pub fn parse_device_code_start(json: &str) -> Result<DeviceCodeStart, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(DeviceCodeStart {
        device_auth_id: str_field(&value, "device_auth_id")
            .ok_or_else(|| "no device_auth_id in the response".to_string())?,
        user_code: str_field(&value, "user_code")
            .ok_or_else(|| "no user_code in the response".to_string())?,
        interval_secs: seconds_field(value.get("interval")).unwrap_or(DEFAULT_DEVICE_INTERVAL_SECS),
        expires_in_secs: seconds_field(value.get("expires_in")).unwrap_or(DEVICE_FLOW_EXPIRY_SECS),
    })
}

/// What a successful device poll returns: the code plus the *server's* PKCE verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuthorization {
    pub authorization_code: String,
    pub code_verifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePollOutcome {
    /// 403 / 404 — the user has not confirmed yet.
    Pending,
    SlowDown,
    Complete(DeviceAuthorization),
    Failed(String),
}

/// Classify one poll of `deviceauth/token`.
pub fn device_poll_outcome(status: u16, body: &str) -> DevicePollOutcome {
    match status {
        200 => {
            let value: Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(_) => {
                    return DevicePollOutcome::Failed("malformed device token response".into())
                }
            };
            match (
                str_field(&value, "authorization_code"),
                str_field(&value, "code_verifier"),
            ) {
                (Some(authorization_code), Some(code_verifier)) => {
                    DevicePollOutcome::Complete(DeviceAuthorization {
                        authorization_code,
                        code_verifier,
                    })
                }
                _ => DevicePollOutcome::Failed(
                    "the device token response carried no authorization code".into(),
                ),
            }
        }
        403 | 404 => DevicePollOutcome::Pending,
        429 => DevicePollOutcome::SlowDown,
        _ => {
            let parsed = parse_error_body(body);
            if parsed.code.as_deref() == Some("slow_down") {
                return DevicePollOutcome::SlowDown;
            }
            DevicePollOutcome::Failed(
                parsed
                    .message
                    .or(parsed.code)
                    .unwrap_or_else(|| format!("HTTP {status}")),
            )
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Identity
// ─────────────────────────────────────────────────────────────────────────────

pub const AUTH_CLAIM: &str = "https://api.openai.com/auth";
pub const PROFILE_CLAIM: &str = "https://api.openai.com/profile";

/// What the ID token (and access token) say about the subscription. Display
/// claims taken on trust because the token came straight from the token
/// endpoint over TLS; `account_id` also rides on every backend request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Identity {
    pub account_id: Option<String>,
    /// `free`, `go`, `plus`, `pro`, `prolite`, `team`, `business`, `enterprise`, `edu`, …
    pub plan_type: Option<String>,
    pub user_id: Option<String>,
    pub email: Option<String>,
    pub fedramp: bool,
}

fn merge_claims(identity: &mut Identity, payload: &Value) {
    let auth = payload.get(AUTH_CLAIM);
    let pick = |key: &str| auth.and_then(|a| str_field(a, key));
    if identity.account_id.is_none() {
        identity.account_id = pick("chatgpt_account_id")
            .or_else(|| str_field(payload, "chatgpt_account_id"))
            .or_else(|| {
                payload
                    .get("organizations")
                    .and_then(Value::as_array)
                    .and_then(|orgs| orgs.first())
                    .and_then(|org| str_field(org, "id"))
            });
    }
    if identity.plan_type.is_none() {
        identity.plan_type = pick("chatgpt_plan_type");
    }
    if identity.user_id.is_none() {
        identity.user_id = pick("chatgpt_user_id");
    }
    if identity.email.is_none() {
        identity.email = str_field(payload, "email").or_else(|| {
            payload
                .get(PROFILE_CLAIM)
                .and_then(|profile| str_field(profile, "email"))
        });
    }
    identity.fedramp |= auth
        .and_then(|a| a.get("chatgpt_account_is_fedramp"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
}

/// Identity from the ID token first, the access token as a fallback (both are JWTs).
pub fn identity_from_tokens(id_token: Option<&str>, access_token: &str) -> Identity {
    let mut identity = Identity::default();
    for token in id_token.into_iter().chain(std::iter::once(access_token)) {
        if let Some(payload) = decode_jwt_payload(token) {
            merge_claims(&mut identity, &payload);
        }
    }
    identity
}

/// Human label for a plan type.
pub fn plan_label(plan_type: &str) -> String {
    match plan_type.trim().to_ascii_lowercase().as_str() {
        "free" => "ChatGPT Free".into(),
        "go" => "ChatGPT Go".into(),
        "plus" => "ChatGPT Plus".into(),
        "pro" => "ChatGPT Pro".into(),
        "prolite" => "ChatGPT Pro Lite".into(),
        "team" => "ChatGPT Team".into(),
        "business" => "ChatGPT Business".into(),
        "enterprise" => "ChatGPT Enterprise".into(),
        "edu" => "ChatGPT Edu".into(),
        "" => "ChatGPT".into(),
        other => {
            let mut chars = other.chars();
            let first = chars
                .next()
                .map(|c| c.to_ascii_uppercase())
                .unwrap_or_default();
            format!("ChatGPT {first}{}", chars.as_str())
        }
    }
}

/// The wire identity the WebView sees.
pub fn account_identity(identity: &Identity) -> AccountIdentity {
    AccountIdentity {
        email: identity.email.clone(),
        display_name: None,
        plan_tier: identity.plan_type.clone(),
        plan_label: Some(
            identity
                .plan_type
                .as_deref()
                .map(plan_label)
                .unwrap_or_else(|| "ChatGPT".to_string()),
        ),
        account_id: identity.account_id.clone(),
        project_id: None,
    }
}

/// `exp` of a JWT, Unix seconds.
pub fn jwt_exp(token: &str) -> Option<u64> {
    decode_jwt_payload(token)?.get("exp")?.as_u64()
}

/// When the access token expires: `expires_in` from the token response when
/// present (VERIFY — the Codex response may omit it), else the JWT's own `exp`.
pub fn access_token_expiry(
    access_token: &str,
    expires_in: Option<u64>,
    now_unix: u64,
) -> Option<u64> {
    oauth::expires_at(expires_in, now_unix).or_else(|| jwt_exp(access_token))
}

// ─────────────────────────────────────────────────────────────────────────────
// Refresh failures
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshFailure {
    /// The refresh token is dead: sign in again (`NeedsReauth`).
    Permanent(String),
    /// Try again later; the previous tokens stay.
    Transient(String),
}

pub const PERMANENT_REFRESH_CODES: &[&str] = &[
    "refresh_token_expired",
    "refresh_token_reused",
    "refresh_token_invalidated",
    "invalid_grant",
];

pub fn classify_refresh_failure(status: u16, body: &str) -> RefreshFailure {
    let parsed = parse_error_body(body);
    let code = parsed
        .code
        .clone()
        .unwrap_or_else(|| format!("http_{status}"));
    if status == 401 || PERMANENT_REFRESH_CODES.contains(&code.as_str()) {
        RefreshFailure::Permanent(code)
    } else {
        RefreshFailure::Transient(code)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// `~/.codex/auth.json` import (read-only)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub account_id: Option<String>,
    pub last_refresh: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Malformed(String),
    /// `auth_mode` is `apikey` — nothing to copy.
    NotChatgptMode(String),
    NoTokens,
}

/// `$CODEX_HOME/auth.json`, else `~/.codex/auth.json`.
pub fn auth_json_path(home: &Path, codex_home: Option<&str>) -> PathBuf {
    match codex_home.map(str::trim).filter(|s| !s.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("auth.json"),
        None => home.join(".codex").join("auth.json"),
    }
}

pub fn parse_auth_json(text: &str) -> Result<ImportedTokens, ImportError> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| ImportError::Malformed(e.to_string()))?;
    if let Some(mode) = str_field(&value, "auth_mode") {
        if !mode.eq_ignore_ascii_case("chatgpt") {
            return Err(ImportError::NotChatgptMode(mode));
        }
    }
    let tokens = value.get("tokens").ok_or(ImportError::NoTokens)?;
    let access_token = str_field(tokens, "access_token").ok_or(ImportError::NoTokens)?;
    Ok(ImportedTokens {
        access_token,
        refresh_token: str_field(tokens, "refresh_token"),
        id_token: str_field(tokens, "id_token"),
        account_id: str_field(tokens, "account_id"),
        last_refresh: str_field(&value, "last_refresh"),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Backend + fingerprint
// ─────────────────────────────────────────────────────────────────────────────

pub fn backend_base() -> String {
    format!("{}{}", fp::UPSTREAM, fp::BASE_PATH)
}

pub fn responses_url() -> String {
    format!("{}/responses", backend_base())
}

pub fn models_url(client_version: &str) -> String {
    format!("{}/models?client_version={client_version}", backend_base())
}

/// What the User-Agent carries about this machine. The terminal token is free
/// text in the CLI (`iTerm.app`, `WezTerm`, …); Bluey names itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexClientEnv {
    /// `sw_vers -productVersion` (`26.0.1`).
    pub os_version: String,
    /// `arm64` / `x86_64`.
    pub arch: String,
    pub terminal: String,
}

impl Default for CodexClientEnv {
    fn default() -> Self {
        Self {
            os_version: "26.0".into(),
            arch: "arm64".into(),
            terminal: "Bluey".into(),
        }
    }
}

pub fn user_agent(env: &CodexClientEnv) -> String {
    format!(
        "codex_cli_rs/{} (Mac OS {}; {}) {}",
        fp::CLIENT_VERSION,
        env.os_version,
        env.arch,
        env.terminal
    )
}

/// The Codex CLI's request fingerprint (`docs/PROVIDER_ACCOUNTS.md › ChatGPT › Headers / Body`).
#[derive(Debug, Clone, Default)]
pub struct CodexShaper {
    pub env: CodexClientEnv,
    /// `X-OpenAI-Fedramp: true` for FedRAMP accounts only (ID-token claim).
    pub fedramp: bool,
}

/// Why a Codex response says the fingerprint or the account no longer passes.
pub fn drift_reason(status: u16, body: &str) -> Option<UnavailableReason> {
    let lower = body.to_ascii_lowercase();
    match status {
        403 => {
            if lower.contains("credential")
                || lower.contains("client_version")
                || lower.contains("originator")
                || lower.contains("unsupported client")
            {
                Some(UnavailableReason::FingerprintDrift)
            } else {
                Some(UnavailableReason::PolicyBlocked)
            }
        }
        400 if lower.contains("unsupported parameter") || lower.contains("must be set to") => {
            // The backend no longer accepts the body shape the CLI sends.
            Some(UnavailableReason::FingerprintDrift)
        }
        _ => None,
    }
}

/// Bring a `/responses` body to the shape the CLI sends: stateless, streaming,
/// encrypted reasoning kept, no sampling knobs.
pub fn normalise_responses_body(body: &mut Value, prompt_cache_key: &str) {
    let Some(map) = body.as_object_mut() else {
        return;
    };
    map.insert("store".into(), Value::Bool(false));
    map.insert("stream".into(), Value::Bool(true));
    for key in [
        "max_output_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "previous_response_id",
    ] {
        map.remove(key);
    }
    let include = map
        .entry("include")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(items) = include.as_array_mut() {
        for wanted in fp::INCLUDE {
            if !items.iter().any(|i| i.as_str() == Some(wanted)) {
                items.push(Value::String(wanted.to_string()));
            }
        }
    }
    if !map.contains_key("prompt_cache_key") {
        map.insert(
            "prompt_cache_key".into(),
            Value::String(prompt_cache_key.into()),
        );
    }
    map.entry("tool_choice")
        .or_insert_with(|| Value::String("auto".into()));
    map.entry("parallel_tool_calls")
        .or_insert(Value::Bool(false));
    map.entry("tools")
        .or_insert_with(|| Value::Array(Vec::new()));
    map.entry("reasoning")
        .or_insert_with(|| json!({ "effort": "medium", "summary": "auto" }));
    match map.get_mut("text") {
        Some(Value::Object(text)) => {
            text.entry("verbosity")
                .or_insert_with(|| Value::String("medium".into()));
        }
        _ => {
            map.insert("text".into(), json!({ "verbosity": "medium" }));
        }
    }
}

impl RequestShaper for CodexShaper {
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
        let account = ctx.provider_account_id.ok_or(ShapeError::Missing(
            "provider_account_id (chatgpt_account_id)",
        ))?;
        let is_post = request.method.eq_ignore_ascii_case("POST");
        let streaming = is_post && request.url.contains("/responses");
        request.set_header("authorization", &format!("Bearer {token}"));
        request.set_header("chatgpt-account-id", account);
        request.set_header("originator", fp::ORIGINATOR);
        request.set_header("user-agent", &user_agent(&self.env));
        request.set_header("version", fp::CLIENT_VERSION);
        request.set_header(
            "accept",
            if streaming {
                "text/event-stream"
            } else {
                "application/json"
            },
        );
        if is_post {
            request.set_header("content-type", "application/json");
        } else {
            request.remove_header("content-type");
        }
        request.set_header("session-id", ctx.session_id);
        request.set_header("thread-id", ctx.session_id);
        request.set_header("x-client-request-id", ctx.request_id);
        // The CLI sends no OpenAI-Beta header on HTTP (DELTA in the doc).
        request.remove_header("openai-beta");
        if self.fedramp {
            request.set_header("x-openai-fedramp", "true");
        } else {
            request.remove_header("x-openai-fedramp");
        }
        if streaming {
            if !request.body.is_object() {
                return Err(ShapeError::InvalidBody(
                    "a /responses body must be a JSON object".into(),
                ));
            }
            normalise_responses_body(&mut request.body, ctx.session_id);
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

/// The shaped `GET /models` request.
pub fn models_request(
    shaper: &CodexShaper,
    ctx: &ShapeContext<'_>,
) -> Result<ProviderHttpRequest, ShapeError> {
    let mut request = ProviderHttpRequest::new("GET", &models_url(fp::CLIENT_VERSION), Value::Null);
    shaper.shape(&mut request, ctx)?;
    Ok(request)
}

// ─────────────────────────────────────────────────────────────────────────────
// Request body
// ─────────────────────────────────────────────────────────────────────────────

/// What goes into `instructions`. The CLI sends the model's
/// `instructions_template` from the catalog (13–21 KB); whether `""` or a
/// short prompt also passes is the replay experiment of the doc — until then
/// Bluey ships the template and carries its own prompt as a `developer` item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionsPolicy<'a> {
    /// The catalog's template as `instructions`; Bluey's system messages become
    /// `developer` input items.
    Template(&'a str),
    /// Bluey's system messages as `instructions` (never empty — the backend rejects that).
    Own,
}

/// Used when there is no system message and no template.
pub const DEFAULT_INSTRUCTIONS: &str =
    "You are Bluey, a discreet on-screen copilot. Answer directly and concisely.";
pub const DEFAULT_IMAGE_DETAIL: &str = "auto";
pub const REASONING_SUMMARY: &str = "auto";

#[derive(Debug, Clone)]
pub struct ResponsesBodyOptions<'a> {
    pub model: &'a str,
    pub messages: &'a [AiMessage],
    pub instructions: InstructionsPolicy<'a>,
    /// `none | low | medium | high | xhigh`, see [`reasoning_effort`].
    pub effort: &'a str,
    /// `low | medium | high`, see [`verbosity_for`].
    pub verbosity: &'a str,
    /// Stable per conversation (the CLI uses its session id).
    pub prompt_cache_key: &'a str,
    pub output_schema: Option<&'a JsonSchemaSpec>,
    /// `auto | low | high | original` for `input_image` parts.
    pub image_detail: &'a str,
}

fn message_item(role: &str, content: Vec<Value>) -> Value {
    json!({ "type": "message", "role": role, "content": content })
}

fn input_parts(message: &AiMessage, image_detail: &str) -> Vec<Value> {
    message
        .content
        .iter()
        .map(|part| match part {
            AiContentPart::Text { text } => json!({ "type": "input_text", "text": text }),
            AiContentPart::Image { media_type, data } => json!({
                "type": "input_image",
                "image_url": format!("data:{};base64,{data}", media_type.as_str()),
                "detail": image_detail,
            }),
        })
        .collect()
}

/// Build the `/responses` body. Roles map to Responses API items: system →
/// `developer` (template policy) or `instructions`, user → `input_text` /
/// `input_image`, assistant → `output_text`.
pub fn build_responses_body(opts: &ResponsesBodyOptions<'_>) -> Value {
    let system_text = opts
        .messages
        .iter()
        .filter(|m| m.role == AiRole::System)
        .map(AiMessage::text_content)
        .filter(|t| !t.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut input: Vec<Value> = Vec::new();
    let instructions = match opts.instructions {
        InstructionsPolicy::Template(template) => {
            if !system_text.is_empty() {
                input.push(message_item(
                    "developer",
                    vec![json!({ "type": "input_text", "text": system_text })],
                ));
            }
            template.to_string()
        }
        InstructionsPolicy::Own => {
            if system_text.is_empty() {
                DEFAULT_INSTRUCTIONS.to_string()
            } else {
                system_text
            }
        }
    };
    for message in opts.messages {
        match message.role {
            AiRole::System => {}
            AiRole::User => input.push(message_item(
                "user",
                input_parts(message, opts.image_detail),
            )),
            AiRole::Assistant => {
                let text = message.text_content();
                input.push(message_item(
                    "assistant",
                    vec![json!({ "type": "output_text", "text": text })],
                ));
            }
        }
    }
    let mut text = json!({ "verbosity": opts.verbosity });
    if let Some(schema) = opts.output_schema {
        // Strict mode wants every property required (optionals nullable) and
        // `additionalProperties: false` on every object — zod's output has
        // neither, and the backend answers such a schema with HTTP 400.
        let strict = schema.strict.unwrap_or(true);
        text["format"] = json!({
            "type": "json_schema",
            "name": schema.name,
            "schema": if strict {
                crate::json_schema::strict_variant(&schema.schema)
            } else {
                crate::json_schema::strip_meta(&schema.schema)
            },
            "strict": strict,
        });
    }
    json!({
        "model": opts.model,
        "instructions": instructions,
        "input": input,
        "tools": [],
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "reasoning": { "effort": opts.effort, "summary": REASONING_SUMMARY },
        "store": false,
        "stream": true,
        "include": fp::INCLUDE,
        "prompt_cache_key": opts.prompt_cache_key,
        "text": text,
    })
}

pub const EFFORT_ORDER: [&str; 5] = ["none", "low", "medium", "high", "xhigh"];

fn effort_rank(effort: &str) -> usize {
    EFFORT_ORDER.iter().position(|e| *e == effort).unwrap_or(2)
}

/// The `reasoning.effort` for a request: Bluey's level and latency budget
/// clamped to what the model supports (`supported` empty = `low..=high`).
pub fn reasoning_effort(
    level: ReasoningLevel,
    latency: LatencyBudget,
    supported: &[String],
    default: Option<&str>,
) -> String {
    let allowed: Vec<&str> = if supported.is_empty() {
        vec!["low", "medium", "high"]
    } else {
        EFFORT_ORDER
            .iter()
            .copied()
            .filter(|e| supported.iter().any(|s| s == e))
            .collect()
    };
    let allowed = if allowed.is_empty() {
        vec!["medium"]
    } else {
        allowed
    };
    let clamp = |wanted: &str| -> String {
        let rank = effort_rank(wanted);
        allowed
            .iter()
            .min_by_key(|e| (effort_rank(e) as i64 - rank as i64).abs())
            .map(|e| e.to_string())
            .unwrap_or_else(|| "medium".into())
    };
    let default = default.filter(|d| allowed.contains(d)).unwrap_or("medium");
    match level {
        ReasoningLevel::None => match latency {
            LatencyBudget::UltraFast | LatencyBudget::Fast => allowed[0].to_string(),
            LatencyBudget::Balanced | LatencyBudget::Deep => clamp(default),
        },
        ReasoningLevel::Light => clamp(default),
        ReasoningLevel::Deep => {
            let cap = if latency == LatencyBudget::Deep {
                "xhigh"
            } else {
                "high"
            };
            allowed
                .iter()
                .rev()
                .find(|e| effort_rank(e) <= effort_rank(cap))
                .map(|e| e.to_string())
                .unwrap_or_else(|| clamp(cap))
        }
    }
}

pub fn verbosity_for(latency: LatencyBudget) -> &'static str {
    match latency {
        LatencyBudget::UltraFast => "low",
        _ => "medium",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SSE
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponsesEvent {
    Created,
    OutputTextDelta(String),
    ReasoningSummaryDelta(String),
    /// A finished output item (message, reasoning, …).
    OutputItemDone(Value),
    Completed {
        usage: Option<Usage>,
    },
    Incomplete {
        reason: Option<String>,
    },
    Failed {
        code: Option<String>,
        message: Option<String>,
    },
    Error {
        code: Option<String>,
        message: Option<String>,
    },
    Other(String),
}

fn u32_field(value: &Value, key: &str) -> Option<u32> {
    value.get(key).and_then(Value::as_u64).map(|n| n as u32)
}

fn parse_usage(response: &Value) -> Option<Usage> {
    let usage = response.get("usage")?;
    Some(Usage {
        input_tokens: u32_field(usage, "input_tokens"),
        output_tokens: u32_field(usage, "output_tokens"),
        cached_tokens: usage
            .get("input_tokens_details")
            .and_then(|d| u32_field(d, "cached_tokens")),
        reasoning_tokens: usage
            .get("output_tokens_details")
            .and_then(|d| u32_field(d, "reasoning_tokens")),
    })
}

fn error_fields(error: &Value) -> (Option<String>, Option<String>) {
    (
        str_field(error, "code").or_else(|| str_field(error, "type")),
        str_field(error, "message"),
    )
}

/// One `data:` payload of the `/responses` stream.
pub fn parse_event(data: &str) -> Result<ResponsesEvent, serde_json::Error> {
    let value: Value = serde_json::from_str(data)?;
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    Ok(match kind {
        "response.created" => ResponsesEvent::Created,
        "response.output_text.delta" => {
            ResponsesEvent::OutputTextDelta(str_field(&value, "delta").unwrap_or_default())
        }
        "response.reasoning_summary_text.delta" => {
            ResponsesEvent::ReasoningSummaryDelta(str_field(&value, "delta").unwrap_or_default())
        }
        "response.output_item.done" => {
            ResponsesEvent::OutputItemDone(value.get("item").cloned().unwrap_or(Value::Null))
        }
        "response.completed" => ResponsesEvent::Completed {
            usage: value.get("response").and_then(parse_usage),
        },
        "response.incomplete" => ResponsesEvent::Incomplete {
            reason: value
                .pointer("/response/incomplete_details/reason")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        "response.failed" => {
            let (code, message) = value
                .pointer("/response/error")
                .map(error_fields)
                .unwrap_or((None, None));
            ResponsesEvent::Failed { code, message }
        }
        "error" => {
            let (code, message) = error_fields(&value);
            ResponsesEvent::Error { code, message }
        }
        other => ResponsesEvent::Other(other.to_string()),
    })
}

/// What the adapter forwards.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    Delta(String),
    Usage(Usage),
    Finished(FinishReason),
    Failed(BlueyError),
}

/// Stream bookkeeping: deltas, the terminal event, and the message text of a
/// finished item when no deltas were streamed.
#[derive(Debug, Default)]
pub struct StreamState {
    finished: bool,
    saw_delta: bool,
}

impl StreamState {
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn on_event(&mut self, event: ResponsesEvent) -> Vec<StreamItem> {
        match event {
            ResponsesEvent::OutputTextDelta(text) => {
                if text.is_empty() {
                    return Vec::new();
                }
                self.saw_delta = true;
                vec![StreamItem::Delta(text)]
            }
            ResponsesEvent::OutputItemDone(item) => {
                if self.saw_delta || item.get("type").and_then(Value::as_str) != Some("message") {
                    return Vec::new();
                }
                let text: String = item
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter(|p| {
                                p.get("type").and_then(Value::as_str) == Some("output_text")
                            })
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("")
                    })
                    .unwrap_or_default();
                if text.is_empty() {
                    Vec::new()
                } else {
                    self.saw_delta = true;
                    vec![StreamItem::Delta(text)]
                }
            }
            ResponsesEvent::Completed { usage } => {
                self.finished = true;
                let mut items = Vec::new();
                if let Some(usage) = usage {
                    items.push(StreamItem::Usage(usage));
                }
                items.push(StreamItem::Finished(FinishReason::Stop));
                items
            }
            ResponsesEvent::Incomplete { reason } => {
                self.finished = true;
                match reason.as_deref() {
                    Some("max_output_tokens") | None => {
                        vec![StreamItem::Finished(FinishReason::Length)]
                    }
                    Some("content_filter") => vec![StreamItem::Failed(BlueyError::ai(
                        "blocked_content_filter",
                        "ChatGPT stopped the answer (content filter)",
                    ))],
                    Some(other) => vec![StreamItem::Failed(BlueyError::ai(
                        "incomplete",
                        format!("ChatGPT ended the answer early ({other})"),
                    ))],
                }
            }
            ResponsesEvent::Failed { code, message } | ResponsesEvent::Error { code, message } => {
                self.finished = true;
                vec![StreamItem::Failed(map_stream_error(
                    code.as_deref(),
                    message.as_deref(),
                ))]
            }
            ResponsesEvent::Created
            | ResponsesEvent::ReasoningSummaryDelta(_)
            | ResponsesEvent::Other(_) => Vec::new(),
        }
    }

    /// The stream ended: an error unless `response.completed` (or a terminal
    /// event) arrived — exactly as the Gemini adapter treats a cut-off stream.
    pub fn on_end(&mut self) -> Option<StreamItem> {
        if self.finished {
            return None;
        }
        self.finished = true;
        Some(StreamItem::Failed(BlueyError::network(
            "stream",
            "the ChatGPT response ended before it completed",
        )))
    }
}

/// Map a mid-stream `response.failed` / `error` onto the contract.
pub fn map_stream_error(code: Option<&str>, message: Option<&str>) -> BlueyError {
    let detail = message.unwrap_or("").to_string();
    match code.unwrap_or("") {
        "rate_limit_exceeded" | "usage_limit_reached" => rate_limited(
            RateLimitInfo::default(),
            "ChatGPT plan limit reached during the answer",
        ),
        "insufficient_quota" => rate_limited(
            RateLimitInfo::default(),
            "the ChatGPT plan's Codex credits are used up",
        ),
        "usage_not_included" => policy_blocked("this ChatGPT plan does not include Codex"),
        "context_length_exceeded" => BlueyError::ai(
            "context_length_exceeded",
            "the request is too long for this ChatGPT model",
        ),
        "invalid_prompt" => BlueyError::ai("blocked_invalid_prompt", "ChatGPT refused the prompt"),
        "server_is_overloaded" | "slow_down" => {
            BlueyError::network("http_5xx", "ChatGPT is overloaded — try again in a moment")
        }
        other => BlueyError::ai(
            "provider_error",
            if detail.is_empty() {
                format!("ChatGPT reported an error ({other})")
            } else {
                format!("ChatGPT reported an error: {detail}")
            },
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Catalog
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct ReasoningLevelEntry {
    pub effort: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct ModelMessages {
    #[serde(default)]
    pub instructions_template: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct ModelEntry {
    pub slug: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_reasoning_level: Option<String>,
    #[serde(default)]
    pub supported_reasoning_levels: Vec<ReasoningLevelEntry>,
    /// `list` | `hide` | `none`.
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub supported_in_api: Option<bool>,
    /// Lower = offered first.
    #[serde(default)]
    pub priority: Option<i64>,
    /// A version string or `[major, minor, patch]`.
    #[serde(default)]
    pub minimal_client_version: Option<Value>,
    #[serde(default)]
    pub context_window: Option<u32>,
    /// `text` | `image` | `audio`.
    #[serde(default)]
    pub input_modalities: Option<Vec<String>>,
    #[serde(default)]
    pub model_messages: Option<ModelMessages>,
    #[serde(default)]
    pub available_in_plans: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct ModelsResponse {
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

pub fn parse_models(json: &str) -> Result<ModelsResponse, serde_json::Error> {
    serde_json::from_str(json)
}

impl ModelEntry {
    /// `image` in `input_modalities`; models without the field are assumed to read images.
    pub fn vision(&self) -> bool {
        match &self.input_modalities {
            Some(modalities) => modalities.iter().any(|m| m == "image"),
            None => true,
        }
    }

    pub fn reasoning_levels(&self) -> Vec<String> {
        self.supported_reasoning_levels
            .iter()
            .map(|l| l.effort.clone())
            .collect()
    }

    pub fn template(&self) -> Option<&str> {
        self.model_messages
            .as_ref()
            .and_then(|m| m.instructions_template.as_deref())
            .filter(|t| !t.is_empty())
    }
}

/// The models a plan can pick from: listed, available in the plan, by priority.
pub fn listed_models<'a>(response: &'a ModelsResponse, plan: Option<&str>) -> Vec<&'a ModelEntry> {
    let mut listed: Vec<&ModelEntry> = response
        .models
        .iter()
        .filter(|m| !m.slug.is_empty())
        .filter(|m| matches!(m.visibility.as_deref(), None | Some("list")))
        .filter(|m| match (plan, &m.available_in_plans) {
            (Some(plan), Some(plans)) if !plans.is_empty() => {
                plans.iter().any(|p| p.eq_ignore_ascii_case(plan))
            }
            _ => true,
        })
        .collect();
    listed.sort_by(|a, b| {
        a.priority
            .unwrap_or(i64::MAX)
            .cmp(&b.priority.unwrap_or(i64::MAX))
            .then_with(|| a.slug.cmp(&b.slug))
    });
    listed
}

/// A model that answers quickly: a small variant, or one that defaults to
/// little or no reasoning.
pub fn is_fast_model(entry: &ModelEntry) -> bool {
    let slug = entry.slug.to_ascii_lowercase();
    ["mini", "nano", "luna", "lite", "flash"]
        .iter()
        .any(|hint| slug.contains(hint))
        || matches!(
            entry.default_reasoning_level.as_deref(),
            Some("none") | Some("low") | Some("minimal")
        )
}

/// The catalog as Bluey stores it, with the roles each model is suggested for
/// (§3.7): `default`/`vision` = the first listed model that reads images,
/// `reasoning`/`research` = the first with a `high`/`xhigh` level, `fast` = the
/// first small model (else the last listed one).
pub fn catalog_models(response: &ModelsResponse, plan: Option<&str>) -> Vec<CatalogModel> {
    let listed = listed_models(response, plan);
    if listed.is_empty() {
        return Vec::new();
    }
    let vision_idx = listed.iter().position(|m| m.vision());
    let default_idx = vision_idx.unwrap_or(0);
    let reasoning_idx = listed
        .iter()
        .position(|m| {
            m.reasoning_levels()
                .iter()
                .any(|l| l == "high" || l == "xhigh")
        })
        .unwrap_or(default_idx);
    let fast_idx = listed
        .iter()
        .position(|m| is_fast_model(m))
        .unwrap_or(if listed.len() > 1 {
            listed.len() - 1
        } else {
            default_idx
        });
    listed
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mut suggested_roles = Vec::new();
            if i == default_idx {
                suggested_roles.push(ModelRole::Default);
            }
            if i == fast_idx {
                suggested_roles.push(ModelRole::Fast);
            }
            if i == reasoning_idx {
                suggested_roles.push(ModelRole::Reasoning);
                suggested_roles.push(ModelRole::Research);
            }
            if Some(i) == vision_idx {
                suggested_roles.push(ModelRole::Vision);
            }
            CatalogModel {
                id: m.slug.clone(),
                label: m.display_name.clone().unwrap_or_else(|| m.slug.clone()),
                capabilities: ModelCapabilities {
                    vision: m.vision(),
                    tools: true,
                    reasoning_levels: m.reasoning_levels(),
                    streaming: true,
                    context_window: m.context_window,
                },
                quota_pool: None,
                suggested_roles,
            }
        })
        .collect()
}

/// `(slug, instructions_template)` for every model that ships one.
pub fn instructions_templates(response: &ModelsResponse) -> Vec<(String, String)> {
    response
        .models
        .iter()
        .filter_map(|m| m.template().map(|t| (m.slug.clone(), t.to_string())))
        .collect()
}

/// Supported efforts and the default effort of a model, from a stored catalog.
pub fn model_reasoning(models: &[CatalogModel], slug: &str) -> Vec<String> {
    models
        .iter()
        .find(|m| m.id == slug)
        .map(|m| m.capabilities.reasoning_levels.clone())
        .unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Responses,
    Models,
}

/// First header value, case-insensitively.
pub fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ErrorBody {
    pub code: Option<String>,
    pub message: Option<String>,
    pub plan_type: Option<String>,
    pub resets_at: Option<u64>,
    pub resets_in_seconds: Option<u64>,
    /// The backend's validation shape: `{"detail": "Store must be set to false"}`.
    pub detail: Option<String>,
}

/// `{"error": {...}}`, `{"error": "code"}` or `{"detail": "..."}`.
pub fn parse_error_body(body: &str) -> ErrorBody {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return ErrorBody::default();
    };
    let mut parsed = ErrorBody {
        detail: str_field(&value, "detail"),
        ..ErrorBody::default()
    };
    match value.get("error") {
        Some(Value::Object(error)) => {
            let error = Value::Object(error.clone());
            let (code, message) = error_fields(&error);
            parsed.code = code;
            parsed.message = message;
            parsed.plan_type = str_field(&error, "plan_type");
            parsed.resets_at = error.get("resets_at").and_then(Value::as_u64);
            parsed.resets_in_seconds = error.get("resets_in_seconds").and_then(Value::as_u64);
        }
        Some(Value::String(code)) => {
            parsed.code = Some(code.clone());
            parsed.message = str_field(&value, "error_description");
        }
        _ => {}
    }
    parsed
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RateLimitInfo {
    /// ISO 8601 reset time.
    pub until: Option<String>,
    /// `5h`, `weekly`, `30 min` …
    pub window: Option<String>,
    pub plan_type: Option<String>,
}

/// `300` → `5h`, `10080` → `weekly`, else minutes / hours.
pub fn window_label(minutes: u64) -> String {
    match minutes {
        0 => "unknown window".into(),
        10080 => "weekly".into(),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m} min"),
    }
}

/// Unix seconds → `YYYY-MM-DDTHH:MM:SSZ` (civil-from-days, no chrono here).
pub fn iso_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, minute, second) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Reset time and window from a 429: the body's `resets_at` / `resets_in_seconds`
/// first, then the `x-codex-primary-*` headers.
pub fn rate_limit_info(
    headers: &[(String, String)],
    body: &ErrorBody,
    now_unix: u64,
) -> RateLimitInfo {
    let header_secs = |name: &str| header(headers, name).and_then(|v| v.trim().parse::<u64>().ok());
    let until = body
        .resets_at
        .or_else(|| body.resets_in_seconds.map(|s| now_unix.saturating_add(s)))
        .or_else(|| header_secs("x-codex-primary-reset-at"))
        .map(iso_from_unix);
    let window = header_secs("x-codex-primary-window-minutes")
        .filter(|m| *m > 0)
        .map(window_label);
    RateLimitInfo {
        until,
        window,
        plan_type: body.plan_type.clone(),
    }
}

pub fn needs_reauth() -> BlueyError {
    BlueyError::new(
        BlueyErrorKind::Authentication,
        bluey_core::accounts::codes::NEEDS_REAUTH,
        "ChatGPT rejected the sign-in — reconnect the account",
    )
    .recoverable(RecoveryAction::reconnect_account(
        CHATGPT_PROVIDER_ID,
        CHATGPT_PROVIDER_ID,
    ))
}

pub fn policy_blocked(message: impl Into<String>) -> BlueyError {
    BlueyError::account("policy_blocked", message).recoverable(RecoveryAction::UseApiKey)
}

pub fn fingerprint_drift(message: impl Into<String>) -> BlueyError {
    BlueyError::account("fingerprint_drift", message).recoverable(RecoveryAction::UseApiKey)
}

pub fn catalog_unavailable(message: impl Into<String>) -> BlueyError {
    BlueyError::account("catalog_unavailable", message).recoverable(RecoveryAction::UseApiKey)
}

/// `account.rate_limited`; `details.until` / `details.window` drive the status
/// and the HUD copy (no `until` = the status stays, the toast still shows).
pub fn rate_limited(info: RateLimitInfo, message: impl Into<String>) -> BlueyError {
    let mut details = Map::new();
    if let Some(until) = &info.until {
        details.insert("until".into(), Value::String(until.clone()));
    }
    if let Some(window) = &info.window {
        details.insert("window".into(), Value::String(window.clone()));
    }
    if let Some(plan) = &info.plan_type {
        details.insert("planType".into(), Value::String(plan.clone()));
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

/// Map a non-2xx Codex response onto the contract. Bodies are quoted only when
/// they are the provider's own words (policy / validation messages), never a prompt.
pub fn map_error(
    status: u16,
    headers: &[(String, String)],
    body: &str,
    endpoint: Endpoint,
    now_unix: u64,
) -> BlueyError {
    let parsed = parse_error_body(body);
    let code = parsed.code.clone().unwrap_or_default();
    let message = parsed
        .message
        .clone()
        .or_else(|| parsed.detail.clone())
        .map(|m| short(&m));
    match status {
        401 => needs_reauth(),
        403 => {
            let text = message.unwrap_or_else(|| "ChatGPT refused the account (HTTP 403)".into());
            match drift_reason(status, body) {
                Some(UnavailableReason::FingerprintDrift) => fingerprint_drift(format!(
                    "ChatGPT stopped recognising Bluey as the Codex CLI: {text}"
                )),
                _ => policy_blocked(format!("ChatGPT blocked the account: {text}")),
            }
        }
        404 if endpoint == Endpoint::Models => catalog_unavailable(format!(
            "the Codex model catalog moved or rejected client version {} (HTTP 404)",
            fp::CLIENT_VERSION
        )),
        429 => match code.as_str() {
            "usage_not_included" => policy_blocked("this ChatGPT plan does not include Codex"),
            "insufficient_quota" => rate_limited(
                RateLimitInfo {
                    plan_type: parsed.plan_type.clone(),
                    ..RateLimitInfo::default()
                },
                "the ChatGPT plan's Codex credits are used up",
            ),
            _ => {
                let info = rate_limit_info(headers, &parsed, now_unix);
                let text = match (&info.until, &info.window) {
                    (Some(until), Some(window)) => {
                        format!("ChatGPT plan limit reached ({window} window) — resets at {until}")
                    }
                    (Some(until), None) => {
                        format!("ChatGPT plan limit reached — resets at {until}")
                    }
                    _ => "ChatGPT plan limit reached".to_string(),
                };
                rate_limited(info, text)
            }
        },
        400 => {
            let lower = body.to_ascii_lowercase();
            match code.as_str() {
                "context_length_exceeded" => BlueyError::ai(
                    "context_length_exceeded",
                    "the request is too long for this ChatGPT model",
                ),
                "invalid_prompt" => BlueyError::ai("blocked_invalid_prompt", "ChatGPT refused the prompt"),
                c if c.starts_with("cyber") => {
                    BlueyError::ai("blocked_cyber_policy", "ChatGPT refused the prompt (cyber policy)")
                }
                _ if drift_reason(status, body).is_some() => fingerprint_drift(format!(
                    "the Codex backend no longer accepts the request shape Bluey sends: {}",
                    message.unwrap_or_default()
                )),
                _ if lower.contains("model") && (lower.contains("not found") || lower.contains("does not exist") || lower.contains("unsupported model") || lower.contains("unknown model")) => {
                    BlueyError::configuration(
                        "model_not_found",
                        "ChatGPT does not offer this model to the plan — refresh the account's models",
                    )
                }
                _ => BlueyError::ai(
                    "invalid_request",
                    message
                        .map(|m| format!("ChatGPT rejected the request: {m}"))
                        .unwrap_or_else(|| "ChatGPT rejected the request (HTTP 400)".into()),
                ),
            }
        }
        500..=599 => BlueyError::network(
            "http_5xx",
            if matches!(code.as_str(), "server_is_overloaded" | "slow_down") {
                "ChatGPT is overloaded — try again in a moment".to_string()
            } else {
                format!("ChatGPT returned HTTP {status}")
            },
        ),
        _ => BlueyError::ai(
            &format!("http_{status}"),
            format!("ChatGPT returned HTTP {status}"),
        ),
    }
}

/// What a probe request says about billing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub ok: bool,
    pub billed_to: BilledTo,
    pub message: String,
}

pub fn probe_outcome(status: u16, headers: &[(String, String)], body: &str) -> ProbeOutcome {
    let has_usage_headers = header(headers, "x-codex-primary-used-percent").is_some()
        || header(headers, "x-codex-primary-window-minutes").is_some()
        || header(headers, "x-codex-primary-reset-at").is_some();
    match status {
        200 if has_usage_headers => ProbeOutcome {
            ok: true,
            billed_to: BilledTo::Plan,
            message: "billed to the ChatGPT plan (Codex usage headers present)".into(),
        },
        200 => ProbeOutcome {
            ok: true,
            billed_to: BilledTo::Unknown,
            message: "200 OK without Codex usage headers — billing pool unknown".into(),
        },
        429 => {
            let parsed = parse_error_body(body);
            ProbeOutcome {
                ok: false,
                billed_to: BilledTo::Plan,
                message: format!(
                    "plan limit reached ({}) — the request counted against the plan",
                    parsed.code.unwrap_or_else(|| "429".into())
                ),
            }
        }
        _ => {
            let parsed = parse_error_body(body);
            ProbeOutcome {
                ok: false,
                billed_to: BilledTo::Unknown,
                message: format!(
                    "HTTP {status}{}",
                    parsed
                        .message
                        .or(parsed.detail)
                        .or(parsed.code)
                        .map(|m| format!(": {}", short(&m)))
                        .unwrap_or_default()
                ),
            }
        }
    }
}

/// The one-token probe body (`accounts_probe_fingerprint`).
pub fn probe_body(
    model: &str,
    prompt_cache_key: &str,
    effort: &str,
    template: Option<&str>,
) -> Value {
    let messages = [AiMessage::text(
        AiRole::User,
        "Reply with the single word: ok",
    )];
    build_responses_body(&ResponsesBodyOptions {
        model,
        messages: &messages,
        instructions: match template {
            Some(t) => InstructionsPolicy::Template(t),
            None => InstructionsPolicy::Own,
        },
        effort,
        verbosity: "low",
        prompt_cache_key,
        output_schema: None,
        image_detail: DEFAULT_IMAGE_DETAIL,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprints::{
        self, diff, scrub_capture, Body, Capture, CaptureSource, CapturedRequest, FingerprintStamp,
        Header, Provider, SCHEMA_VERSION,
    };
    use bluey_core::types::ImageMediaType;
    use pretty_assertions::assert_eq;

    fn jwt(payload: Value) -> String {
        let header = oauth::base64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = oauth::base64url(payload.to_string().as_bytes());
        format!("{header}.{payload}.sig")
    }

    fn id_token() -> String {
        jwt(json!({
            "email": "owner@example.com",
            "exp": 1_800_000_000u64,
            AUTH_CLAIM: {
                "chatgpt_account_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
                "chatgpt_plan_type": "plus",
                "chatgpt_user_id": "user-abc",
                "chatgpt_account_is_fedramp": false
            }
        }))
    }

    #[test]
    fn the_authorize_url_carries_the_clis_parameters_in_its_order() {
        let url = authorize_url(&redirect_uri(1455), "chal", "st");
        let parsed = url::Url::parse(&url).unwrap();
        let keys: Vec<String> = parsed.query_pairs().map(|(k, _)| k.into_owned()).collect();
        assert_eq!(
            keys,
            vec![
                "response_type",
                "client_id",
                "redirect_uri",
                "scope",
                "code_challenge",
                "code_challenge_method",
                "id_token_add_organizations",
                "codex_cli_simplified_flow",
                "state",
                "originator"
            ]
        );
        let value = |k: &str| {
            parsed
                .query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
                .unwrap()
        };
        assert_eq!(value("client_id"), CLIENT_ID);
        assert_eq!(value("redirect_uri"), "http://localhost:1455/auth/callback");
        assert_eq!(value("scope"), SCOPES);
        assert_eq!(value("originator"), "codex_cli_rs");
        assert_eq!(value("codex_cli_simplified_flow"), "true");
        assert_eq!(redirect_uri(1457), "http://localhost:1457/auth/callback");
        assert_eq!(LOOPBACK_PORTS, [1455, 1457]);
        let (verifier, _) = oauth::pkce_pair_from(&[9u8; PKCE_VERIFIER_BYTES]);
        assert_eq!(verifier.len(), 86, "the CLI's 64-byte verifier");
    }

    #[test]
    fn token_bodies_use_form_for_the_exchange_and_json_for_the_refresh() {
        let form = token_exchange_form("code", "http://localhost:1455/auth/callback", "ver");
        assert_eq!(form[0], ("grant_type", "authorization_code".to_string()));
        assert_eq!(form[3], ("client_id", CLIENT_ID.to_string()));
        assert_eq!(
            refresh_body("rt"),
            json!({ "client_id": CLIENT_ID, "grant_type": "refresh_token", "refresh_token": "rt" })
        );
        assert_eq!(
            revoke_body("rt"),
            json!({ "client_id": CLIENT_ID, "token": "rt" })
        );
        assert_eq!(device_usercode_body(), json!({ "client_id": CLIENT_ID }));
        assert_eq!(
            device_poll_body("dev-1", "ABCD-1234"),
            json!({ "device_auth_id": "dev-1", "user_code": "ABCD-1234" })
        );
    }

    #[test]
    fn device_flow_responses_are_classified() {
        let start = parse_device_code_start(
            r#"{"device_auth_id":"dev-1","user_code":"ABCD-1234","interval":"5"}"#,
        )
        .unwrap();
        assert_eq!(start.interval_secs, 5);
        assert_eq!(start.expires_in_secs, DEVICE_FLOW_EXPIRY_SECS);
        assert_eq!(start.user_code, "ABCD-1234");
        let numeric = parse_device_code_start(
            r#"{"device_auth_id":"d","user_code":"u","interval":7,"expires_in":600}"#,
        )
        .unwrap();
        assert_eq!((numeric.interval_secs, numeric.expires_in_secs), (7, 600));
        assert!(parse_device_code_start(r#"{"user_code":"u"}"#).is_err());

        assert_eq!(device_poll_outcome(403, ""), DevicePollOutcome::Pending);
        assert_eq!(device_poll_outcome(404, "{}"), DevicePollOutcome::Pending);
        assert_eq!(device_poll_outcome(429, ""), DevicePollOutcome::SlowDown);
        assert_eq!(
            device_poll_outcome(400, r#"{"error":{"code":"slow_down"}}"#),
            DevicePollOutcome::SlowDown
        );
        assert_eq!(
            device_poll_outcome(
                200,
                r#"{"authorization_code":"ac","code_verifier":"cv","code_challenge":"cc"}"#
            ),
            DevicePollOutcome::Complete(DeviceAuthorization {
                authorization_code: "ac".into(),
                code_verifier: "cv".into(),
            })
        );
        assert!(matches!(
            device_poll_outcome(200, "{}"),
            DevicePollOutcome::Failed(_)
        ));
        assert_eq!(
            device_poll_outcome(
                400,
                r#"{"error":{"code":"access_denied","message":"nope"}}"#
            ),
            DevicePollOutcome::Failed("nope".into())
        );
    }

    #[test]
    fn identity_comes_from_the_namespaced_claims_with_fallbacks() {
        let identity = identity_from_tokens(Some(&id_token()), "not.a.jwt");
        assert_eq!(
            identity,
            Identity {
                account_id: Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e".into()),
                plan_type: Some("plus".into()),
                user_id: Some("user-abc".into()),
                email: Some("owner@example.com".into()),
                fedramp: false,
            }
        );
        let wire = account_identity(&identity);
        assert_eq!(wire.plan_label.as_deref(), Some("ChatGPT Plus"));
        assert_eq!(wire.plan_tier.as_deref(), Some("plus"));
        assert_eq!(
            wire.account_id.as_deref(),
            Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e")
        );
        assert_eq!(wire.email.as_deref(), Some("owner@example.com"));

        let access_only = jwt(json!({
            PROFILE_CLAIM: { "email": "p@example.com" },
            "organizations": [{ "id": "org-1" }],
            AUTH_CLAIM: { "chatgpt_account_is_fedramp": true }
        }));
        let fallback = identity_from_tokens(None, &access_only);
        assert_eq!(fallback.email.as_deref(), Some("p@example.com"));
        assert_eq!(fallback.account_id.as_deref(), Some("org-1"));
        assert!(fallback.fedramp);
        assert_eq!(
            account_identity(&fallback).plan_label.as_deref(),
            Some("ChatGPT")
        );
        assert_eq!(plan_label("pro"), "ChatGPT Pro");
        assert_eq!(plan_label("prolite"), "ChatGPT Pro Lite");
        assert_eq!(plan_label("team"), "ChatGPT Team");
        assert_eq!(plan_label("edu"), "ChatGPT Edu");
        assert_eq!(plan_label("mystery"), "ChatGPT Mystery");
        assert_eq!(jwt_exp(&id_token()), Some(1_800_000_000));
        assert_eq!(
            access_token_expiry(&id_token(), Some(3600), 100),
            Some(3700)
        );
        assert_eq!(
            access_token_expiry(&id_token(), None, 100),
            Some(1_800_000_000)
        );
        assert_eq!(access_token_expiry("opaque", None, 100), None);
    }

    #[test]
    fn refresh_failures_are_permanent_on_401_and_the_known_codes() {
        assert_eq!(
            classify_refresh_failure(401, r#"{"error":{"code":"refresh_token_invalidated"}}"#),
            RefreshFailure::Permanent("refresh_token_invalidated".into())
        );
        assert_eq!(
            classify_refresh_failure(400, r#"{"error":"invalid_grant"}"#),
            RefreshFailure::Permanent("invalid_grant".into())
        );
        assert_eq!(
            classify_refresh_failure(400, r#"{"error":{"code":"invalid_request"}}"#),
            RefreshFailure::Transient("invalid_request".into())
        );
        assert_eq!(
            classify_refresh_failure(503, "upstream down"),
            RefreshFailure::Transient("http_503".into())
        );
    }

    #[test]
    fn auth_json_import_reads_the_clis_store_without_writing() {
        let text = r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"i.d.t","access_token":"a.c.t","refresh_token":"rt","account_id":"acct"},"last_refresh":"2026-09-10T10:00:00Z"}"#;
        let imported = parse_auth_json(text).unwrap();
        assert_eq!(imported.access_token, "a.c.t");
        assert_eq!(imported.refresh_token.as_deref(), Some("rt"));
        assert_eq!(imported.id_token.as_deref(), Some("i.d.t"));
        assert_eq!(imported.account_id.as_deref(), Some("acct"));
        assert_eq!(
            imported.last_refresh.as_deref(),
            Some("2026-09-10T10:00:00Z")
        );
        assert_eq!(
            parse_auth_json(r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-x"}"#),
            Err(ImportError::NotChatgptMode("apikey".into()))
        );
        assert_eq!(
            parse_auth_json(r#"{"tokens":{}}"#),
            Err(ImportError::NoTokens)
        );
        assert!(matches!(
            parse_auth_json("nope"),
            Err(ImportError::Malformed(_))
        ));
        let home = Path::new("/Users/owner");
        assert_eq!(
            auth_json_path(home, None),
            PathBuf::from("/Users/owner/.codex/auth.json")
        );
        assert_eq!(
            auth_json_path(home, Some("/tmp/codex-home")),
            PathBuf::from("/tmp/codex-home/auth.json")
        );
        assert_eq!(
            auth_json_path(home, Some("  ")),
            PathBuf::from("/Users/owner/.codex/auth.json")
        );
    }

    fn messages() -> Vec<AiMessage> {
        vec![
            AiMessage::text(AiRole::System, "You are Bluey."),
            AiMessage {
                role: AiRole::User,
                content: vec![
                    AiContentPart::Text {
                        text: "What is on my screen?".into(),
                    },
                    AiContentPart::Image {
                        media_type: ImageMediaType::Png,
                        data: "iVBORw0KGgo=".into(),
                    },
                ],
            },
            AiMessage::text(AiRole::Assistant, "A spreadsheet."),
            AiMessage::text(AiRole::User, "Summarise it."),
        ]
    }

    #[test]
    fn the_responses_body_follows_the_clis_shape() {
        let body = build_responses_body(&ResponsesBodyOptions {
            model: "gpt-6-astra",
            messages: &messages(),
            instructions: InstructionsPolicy::Template("TEMPLATE"),
            effort: "medium",
            verbosity: "medium",
            prompt_cache_key: "cache-key",
            output_schema: None,
            image_detail: "auto",
        });
        assert_eq!(body["instructions"], "TEMPLATE");
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["input"][0]["content"][0]["text"], "You are Bluey.");
        assert_eq!(body["input"][1]["role"], "user");
        assert_eq!(body["input"][1]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["content"][1]["type"], "input_image");
        assert_eq!(
            body["input"][1]["content"][1]["image_url"],
            "data:image/png;base64,iVBORw0KGgo="
        );
        assert_eq!(body["input"][1]["content"][1]["detail"], "auto");
        assert_eq!(body["input"][2]["role"], "assistant");
        assert_eq!(body["input"][2]["content"][0]["type"], "output_text");
        assert_eq!(body["input"][3]["content"][0]["text"], "Summarise it.");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(
            body["reasoning"],
            json!({ "effort": "medium", "summary": "auto" })
        );
        assert_eq!(body["prompt_cache_key"], "cache-key");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["text"], json!({ "verbosity": "medium" }));
        for forbidden in [
            "max_output_tokens",
            "temperature",
            "top_p",
            "previous_response_id",
        ] {
            assert!(
                body.get(forbidden).is_none(),
                "{forbidden} must never be sent"
            );
        }

        let own = build_responses_body(&ResponsesBodyOptions {
            model: "gpt-6-astra",
            messages: &messages(),
            instructions: InstructionsPolicy::Own,
            effort: "low",
            verbosity: "low",
            prompt_cache_key: "k",
            output_schema: Some(&JsonSchemaSpec {
                name: "answer".into(),
                schema: json!({ "type": "object" }),
                strict: None,
            }),
            image_detail: "high",
        });
        assert_eq!(own["instructions"], "You are Bluey.");
        assert_eq!(
            own["input"][0]["role"], "user",
            "no developer item under the own policy"
        );
        assert_eq!(own["input"][0]["content"][1]["detail"], "high");
        assert_eq!(own["text"]["format"]["type"], "json_schema");
        assert_eq!(own["text"]["format"]["strict"], true);
        assert_eq!(
            own["text"]["format"]["schema"],
            json!({ "type": "object", "required": [], "additionalProperties": false }),
            "strict mode needs additionalProperties: false on every object"
        );
        let no_system = build_responses_body(&ResponsesBodyOptions {
            model: "m",
            messages: &[AiMessage::text(AiRole::User, "hi")],
            instructions: InstructionsPolicy::Own,
            effort: "low",
            verbosity: "low",
            prompt_cache_key: "k",
            output_schema: None,
            image_detail: "auto",
        });
        assert_eq!(
            no_system["instructions"], DEFAULT_INSTRUCTIONS,
            "never empty"
        );
    }

    #[test]
    fn a_zod_schema_with_optional_fields_is_sent_in_its_strict_form() {
        // What `outputSchemaFor("answer")` emits: optionals missing from `required`,
        // a top-level `$schema` — the shape the Responses API rejects with HTTP 400.
        let zod = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "responseType": { "type": "string", "const": "answer" },
                "title": { "type": "string" },
                "content": { "type": "string" },
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
            },
            "required": ["responseType", "content"],
            "additionalProperties": false
        });
        let strict = build_responses_body(&ResponsesBodyOptions {
            model: "gpt-6-astra",
            messages: &[AiMessage::text(AiRole::User, "hi")],
            instructions: InstructionsPolicy::Own,
            effort: "low",
            verbosity: "low",
            prompt_cache_key: "k",
            output_schema: Some(&JsonSchemaSpec {
                name: "bluey_answer".into(),
                schema: zod.clone(),
                strict: Some(true),
            }),
            image_detail: "auto",
        });
        let format = &strict["text"]["format"];
        assert_eq!(format["name"], "bluey_answer");
        assert!(format["schema"].get("$schema").is_none());
        assert_eq!(
            format["schema"]["required"],
            json!(["confidence", "content", "responseType", "title"]),
            "every property (serde_json keeps keys sorted)"
        );
        assert_eq!(
            format["schema"]["properties"]["title"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(
            format["schema"]["properties"]["confidence"]["type"],
            json!(["number", "null"])
        );
        assert_eq!(format["schema"]["properties"]["confidence"]["maximum"], 1);

        let lenient = build_responses_body(&ResponsesBodyOptions {
            model: "gpt-6-astra",
            messages: &[AiMessage::text(AiRole::User, "hi")],
            instructions: InstructionsPolicy::Own,
            effort: "low",
            verbosity: "low",
            prompt_cache_key: "k",
            output_schema: Some(&JsonSchemaSpec {
                name: "bluey_answer".into(),
                schema: zod,
                strict: Some(false),
            }),
            image_detail: "auto",
        });
        let format = &lenient["text"]["format"];
        assert_eq!(format["strict"], false);
        assert!(format["schema"].get("$schema").is_none());
        assert_eq!(
            format["schema"]["required"],
            json!(["responseType", "content"]),
            "non-strict keeps the caller's optionals"
        );
    }

    #[test]
    fn reasoning_effort_clamps_to_the_models_levels() {
        let levels = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let full = levels(&["none", "low", "medium", "high", "xhigh"]);
        assert_eq!(
            reasoning_effort(
                ReasoningLevel::None,
                LatencyBudget::UltraFast,
                &full,
                Some("medium")
            ),
            "none"
        );
        assert_eq!(
            reasoning_effort(
                ReasoningLevel::None,
                LatencyBudget::Balanced,
                &full,
                Some("medium")
            ),
            "medium"
        );
        assert_eq!(
            reasoning_effort(
                ReasoningLevel::Light,
                LatencyBudget::Fast,
                &full,
                Some("high")
            ),
            "high"
        );
        assert_eq!(
            reasoning_effort(ReasoningLevel::Deep, LatencyBudget::Balanced, &full, None),
            "high"
        );
        assert_eq!(
            reasoning_effort(ReasoningLevel::Deep, LatencyBudget::Deep, &full, None),
            "xhigh"
        );
        let limited = levels(&["low", "medium"]);
        assert_eq!(
            reasoning_effort(ReasoningLevel::Deep, LatencyBudget::Deep, &limited, None),
            "medium"
        );
        assert_eq!(
            reasoning_effort(ReasoningLevel::None, LatencyBudget::Fast, &limited, None),
            "low"
        );
        assert_eq!(
            reasoning_effort(ReasoningLevel::None, LatencyBudget::Fast, &[], None),
            "low",
            "unknown catalog: low..=high"
        );
        assert_eq!(
            reasoning_effort(
                ReasoningLevel::Light,
                LatencyBudget::Fast,
                &[],
                Some("xhigh")
            ),
            "medium",
            "an unsupported default falls back"
        );
        assert_eq!(verbosity_for(LatencyBudget::UltraFast), "low");
        assert_eq!(verbosity_for(LatencyBudget::Deep), "medium");
    }

    #[test]
    fn sse_events_drive_the_stream_state() {
        let mut state = StreamState::default();
        assert_eq!(
            state.on_event(
                parse_event(r#"{"type":"response.created","response":{"id":"resp_1"}}"#).unwrap()
            ),
            vec![]
        );
        assert_eq!(
            state.on_event(
                parse_event(r#"{"type":"response.output_text.delta","delta":"Hel"}"#).unwrap()
            ),
            vec![StreamItem::Delta("Hel".into())]
        );
        assert_eq!(
            state.on_event(
                parse_event(
                    r#"{"type":"response.reasoning_summary_text.delta","delta":"thinking"}"#
                )
                .unwrap()
            ),
            vec![]
        );
        assert_eq!(
            state.on_event(
                parse_event(r#"{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"Hello"}]}}"#)
                    .unwrap()
            ),
            vec![],
            "deltas were streamed — the finished item is not repeated"
        );
        let completed = parse_event(
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":120,"output_tokens":8,"input_tokens_details":{"cached_tokens":100},"output_tokens_details":{"reasoning_tokens":3}}}}"#,
        )
        .unwrap();
        assert_eq!(
            state.on_event(completed),
            vec![
                StreamItem::Usage(Usage {
                    input_tokens: Some(120),
                    output_tokens: Some(8),
                    cached_tokens: Some(100),
                    reasoning_tokens: Some(3),
                }),
                StreamItem::Finished(FinishReason::Stop),
            ]
        );
        assert!(state.is_finished());
        assert_eq!(state.on_end(), None);

        // A stream that only sends the finished item still yields its text.
        let mut quiet = StreamState::default();
        assert_eq!(
            quiet.on_event(
                parse_event(r#"{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"Hello"}]}}"#)
                    .unwrap()
            ),
            vec![StreamItem::Delta("Hello".into())]
        );
        let cut = quiet.on_end().expect("EOF without completed is an error");
        match cut {
            StreamItem::Failed(error) => assert_eq!(error.code, "network.stream"),
            other => panic!("{other:?}"),
        }

        let mut failing = StreamState::default();
        let failed = parse_event(
            r#"{"type":"response.failed","response":{"error":{"code":"usage_limit_reached","message":"limit"}}}"#,
        )
        .unwrap();
        match failing.on_event(failed).remove(0) {
            StreamItem::Failed(error) => assert_eq!(error.code, "account.rate_limited"),
            other => panic!("{other:?}"),
        }
        let mut incomplete = StreamState::default();
        assert_eq!(
            incomplete.on_event(
                parse_event(r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}"#)
                    .unwrap()
            ),
            vec![StreamItem::Finished(FinishReason::Length)]
        );
        let mut erroring = StreamState::default();
        match erroring
            .on_event(
                parse_event(r#"{"type":"error","code":"server_is_overloaded","message":"busy"}"#)
                    .unwrap(),
            )
            .remove(0)
        {
            StreamItem::Failed(error) => assert_eq!(error.code, "network.http_5xx"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            parse_event(r#"{"type":"response.in_progress"}"#).unwrap(),
            ResponsesEvent::Other(_)
        ));
        assert!(parse_event("not json").is_err());
    }

    const CATALOG: &str = r#"{"models":[
        {"slug":"gpt-5.6-luna","display_name":"GPT-5.6 Luna","visibility":"list","priority":3,"default_reasoning_level":"low","supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"}],"input_modalities":["text","image"],"context_window":272000,"available_in_plans":["free","plus","pro"]},
        {"slug":"gpt-6-astra","display_name":"GPT-6 Astra","visibility":"list","priority":1,"default_reasoning_level":"medium","supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"},{"effort":"high"},{"effort":"xhigh"}],"input_modalities":["text","image"],"context_window":400000,"model_messages":{"instructions_template":"You are Codex."},"available_in_plans":["plus","pro"]},
        {"slug":"gpt-5.6-sol","display_name":"GPT-5.6 Sol","visibility":"list","priority":2,"supported_reasoning_levels":[{"effort":"medium"},{"effort":"high"}],"input_modalities":["text"]},
        {"slug":"codex-auto-review","display_name":"Auto review","visibility":"hide","priority":0},
        {"slug":"gpt-daybreak-alpha","visibility":"none","priority":0}
    ]}"#;

    #[test]
    fn the_catalog_lists_visible_models_by_priority_with_suggested_roles() {
        let response = parse_models(CATALOG).unwrap();
        let models = catalog_models(&response, Some("plus"));
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["gpt-6-astra", "gpt-5.6-sol", "gpt-5.6-luna"]);
        let astra = &models[0];
        assert!(astra.capabilities.vision);
        assert_eq!(astra.capabilities.context_window, Some(400_000));
        assert_eq!(
            astra.capabilities.reasoning_levels,
            vec!["low", "medium", "high", "xhigh"]
        );
        assert_eq!(
            astra.suggested_roles,
            vec![
                ModelRole::Default,
                ModelRole::Reasoning,
                ModelRole::Research,
                ModelRole::Vision
            ]
        );
        assert!(!models[1].capabilities.vision);
        assert_eq!(models[1].suggested_roles, vec![]);
        assert_eq!(models[2].suggested_roles, vec![ModelRole::Fast]);
        assert_eq!(models[2].label, "GPT-5.6 Luna");

        let free = catalog_models(&response, Some("free"));
        assert_eq!(
            free.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["gpt-5.6-sol", "gpt-5.6-luna"],
            "plan filter drops models the plan does not list"
        );
        assert_eq!(
            instructions_templates(&response),
            vec![("gpt-6-astra".to_string(), "You are Codex.".to_string())]
        );
        assert_eq!(
            model_reasoning(&models, "gpt-5.6-luna"),
            vec!["low", "medium"]
        );
        assert!(model_reasoning(&models, "nope").is_empty());
        assert!(catalog_models(&ModelsResponse::default(), None).is_empty());
    }

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn errors_map_onto_account_codes() {
        let now = 1_700_000_000;
        let reauth = map_error(401, &[], "", Endpoint::Responses, now);
        assert_eq!(reauth.code, "account.needs_reauth");
        assert_eq!(reauth.kind, BlueyErrorKind::Authentication);
        assert_eq!(
            reauth.recovery,
            Some(RecoveryAction::reconnect_account("chatgpt", "chatgpt"))
        );

        let blocked = map_error(
            403,
            &[],
            r#"{"error":{"code":"misalignment_policy_violation","message":"Your account was flagged"}}"#,
            Endpoint::Responses,
            now,
        );
        assert_eq!(blocked.code, "account.policy_blocked");
        assert!(blocked.message.contains("flagged"));
        assert_eq!(blocked.recovery, Some(RecoveryAction::UseApiKey));
        let drift = map_error(
            403,
            &[],
            r#"{"detail":"invalid or disabled credential"}"#,
            Endpoint::Responses,
            now,
        );
        assert_eq!(drift.code, "account.fingerprint_drift");

        let limited = map_error(
            429,
            &headers(&[
                ("x-codex-primary-window-minutes", "300"),
                ("x-codex-primary-reset-at", "1700003600"),
            ]),
            r#"{"error":{"type":"usage_limit_reached","plan_type":"plus","resets_at":1700007200}}"#,
            Endpoint::Responses,
            now,
        );
        assert_eq!(limited.code, "account.rate_limited");
        let details = limited.details.clone().unwrap();
        assert_eq!(
            details["until"], "2023-11-15T00:13:20Z",
            "the body's resets_at wins"
        );
        assert_eq!(details["window"], "5h");
        assert_eq!(details["planType"], "plus");
        assert!(limited.message.contains("5h window"));
        let header_only = map_error(
            429,
            &headers(&[
                ("x-codex-primary-reset-at", "1700003600"),
                ("x-codex-primary-window-minutes", "10080"),
            ]),
            r#"{"error":{"type":"rate_limit_exceeded"}}"#,
            Endpoint::Responses,
            now,
        );
        assert_eq!(
            header_only.details.clone().unwrap()["until"],
            "2023-11-14T23:13:20Z"
        );
        assert_eq!(header_only.details.clone().unwrap()["window"], "weekly");
        let relative = map_error(
            429,
            &[],
            r#"{"error":{"type":"usage_limit_reached","resets_in_seconds":60}}"#,
            Endpoint::Responses,
            now,
        );
        assert_eq!(
            relative.details.clone().unwrap()["until"],
            "2023-11-14T22:14:20Z"
        );
        let no_reset = map_error(429, &[], "{}", Endpoint::Responses, now);
        assert_eq!(no_reset.code, "account.rate_limited");
        assert!(no_reset.details.is_none());
        assert_eq!(
            map_error(
                429,
                &[],
                r#"{"error":{"code":"usage_not_included"}}"#,
                Endpoint::Responses,
                now
            )
            .code,
            "account.policy_blocked"
        );
        assert_eq!(
            map_error(
                429,
                &[],
                r#"{"error":{"code":"insufficient_quota"}}"#,
                Endpoint::Responses,
                now
            )
            .code,
            "account.rate_limited"
        );

        assert_eq!(
            map_error(404, &[], "", Endpoint::Models, now).code,
            "account.catalog_unavailable"
        );
        assert_eq!(
            map_error(404, &[], "", Endpoint::Responses, now).code,
            "ai.http_404"
        );
        assert_eq!(
            map_error(
                400,
                &[],
                r#"{"error":{"code":"context_length_exceeded"}}"#,
                Endpoint::Responses,
                now
            )
            .code,
            "ai.context_length_exceeded"
        );
        assert_eq!(
            map_error(
                400,
                &[],
                r#"{"detail":"Unsupported parameter: max_output_tokens"}"#,
                Endpoint::Responses,
                now
            )
            .code,
            "account.fingerprint_drift"
        );
        assert_eq!(
            map_error(
                400,
                &[],
                r#"{"error":{"message":"The model `gpt-9` does not exist"}}"#,
                Endpoint::Responses,
                now
            )
            .code,
            "config.model_not_found"
        );
        assert_eq!(
            map_error(
                400,
                &[],
                r#"{"error":{"code":"cyber_policy_violation"}}"#,
                Endpoint::Responses,
                now
            )
            .code,
            "ai.blocked_cyber_policy"
        );
        let invalid = map_error(
            400,
            &[],
            r#"{"error":{"message":"bad image"}}"#,
            Endpoint::Responses,
            now,
        );
        assert_eq!(invalid.code, "ai.invalid_request");
        assert!(invalid.message.contains("bad image"));
        let overloaded = map_error(
            503,
            &[],
            r#"{"error":{"code":"server_is_overloaded"}}"#,
            Endpoint::Responses,
            now,
        );
        assert_eq!(overloaded.code, "network.http_5xx");
        assert_eq!(overloaded.recovery, Some(RecoveryAction::Retry));
        assert_eq!(
            map_error(418, &[], "", Endpoint::Responses, now).code,
            "ai.http_418"
        );

        assert_eq!(iso_from_unix(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(window_label(300), "5h");
        assert_eq!(window_label(10080), "weekly");
        assert_eq!(window_label(45), "45 min");
        assert_eq!(window_label(0), "unknown window");
    }

    #[test]
    fn probe_outcomes_read_the_usage_headers() {
        let plan = probe_outcome(200, &headers(&[("x-codex-primary-used-percent", "12")]), "");
        assert_eq!(plan.billed_to, BilledTo::Plan);
        assert!(plan.ok);
        let unknown = probe_outcome(200, &[], "");
        assert_eq!(unknown.billed_to, BilledTo::Unknown);
        assert!(unknown.ok);
        let limited = probe_outcome(429, &[], r#"{"error":{"type":"usage_limit_reached"}}"#);
        assert!(!limited.ok);
        assert_eq!(limited.billed_to, BilledTo::Plan);
        let refused = probe_outcome(403, &[], r#"{"detail":"blocked"}"#);
        assert!(!refused.ok);
        assert_eq!(refused.message, "HTTP 403: blocked");
        let body = probe_body("gpt-5.6-luna", "k", "low", Some("T"));
        assert_eq!(body["instructions"], "T");
        assert_eq!(
            body["input"][0]["content"][0]["text"],
            "Reply with the single word: ok"
        );
        assert_eq!(body["text"]["verbosity"], "low");
    }

    fn shaped(shaper: &CodexShaper, request: &mut ProviderHttpRequest, model: &str) {
        shaper
            .shape(
                request,
                &ShapeContext {
                    account_id: "chatgpt",
                    provider_account_id: Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e"),
                    device_id: "",
                    session_id: "6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b",
                    request_id: "7a1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7c",
                    model,
                    access_token: Some("eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln"),
                },
            )
            .unwrap();
    }

    /// A shaped request as the capture proxy would record it (reqwest adds `host`).
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
            provider: "chatgpt".into(),
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
        scrub_capture(&mut capture, Provider::Chatgpt.rules());
        capture
    }

    fn documented(endpoint: &str) -> Capture {
        Provider::Chatgpt
            .documented()
            .into_iter()
            .find(|c| {
                Provider::Chatgpt
                    .rules()
                    .endpoint_for(&c.request.method, &c.request.path())
                    .is_some_and(|e| e.name == endpoint)
            })
            .expect("documented capture")
    }

    #[test]
    fn the_shaper_reproduces_the_documented_fingerprint() {
        let shaper = CodexShaper::default();
        let body = build_responses_body(&ResponsesBodyOptions {
            model: "gpt-6-astra",
            messages: &[AiMessage::text(AiRole::User, "hello")],
            instructions: InstructionsPolicy::Template("You are Codex."),
            effort: "medium",
            verbosity: "medium",
            prompt_cache_key: "6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b",
            output_schema: None,
            image_detail: DEFAULT_IMAGE_DETAIL,
        });
        let mut request = ProviderHttpRequest::new("POST", &responses_url(), body);
        request
            .headers
            .push(("OpenAI-Beta".into(), "responses=experimental".into()));
        shaped(&shaper, &mut request, "gpt-6-astra");
        assert_eq!(
            request.header("openai-beta"),
            None,
            "the CLI sends no OpenAI-Beta header"
        );
        assert_eq!(request.header("accept"), Some("text/event-stream"));
        assert_eq!(request.header("originator"), Some("codex_cli_rs"));
        assert_eq!(request.header("version"), Some(fp::CLIENT_VERSION));
        assert_eq!(
            request.header("user-agent"),
            Some("codex_cli_rs/0.154.0 (Mac OS 26.0; arm64) Bluey")
        );
        assert_eq!(request.header("x-openai-fedramp"), None);
        assert_eq!(shaper.fingerprint(), fp::INFO);

        let capture = capture_of(&request);
        let report = diff(
            Provider::Chatgpt.rules(),
            &documented("responses"),
            &capture,
            "documented",
            "shaper",
        );
        assert!(!report.has_drift(), "{}", report.render());
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.location == "header user-agent"),
            "the terminal token differs within the documented format: {}",
            report.render()
        );

        let models = models_request(
            &shaper,
            &ShapeContext {
                account_id: "chatgpt",
                provider_account_id: Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e"),
                device_id: "",
                session_id: "6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b",
                request_id: "7a1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7c",
                model: "",
                access_token: Some("eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln"),
            },
        )
        .unwrap();
        assert_eq!(models.method, "GET");
        assert_eq!(models.header("accept"), Some("application/json"));
        assert_eq!(models.header("content-type"), None);
        assert!(models.url.ends_with("/models?client_version=0.154.0"));
        let report = diff(
            Provider::Chatgpt.rules(),
            &documented("models"),
            &capture_of(&models),
            "documented",
            "shaper",
        );
        assert!(!report.has_drift(), "{}", report.render());

        let fedramp = CodexShaper {
            fedramp: true,
            ..CodexShaper::default()
        };
        let mut request = ProviderHttpRequest::new(
            "POST",
            &responses_url(),
            json!({ "model": "m", "max_output_tokens": 5, "temperature": 0.2 }),
        );
        shaped(&fedramp, &mut request, "m");
        assert_eq!(request.header("x-openai-fedramp"), Some("true"));
        assert_eq!(request.body["store"], false);
        assert_eq!(request.body["stream"], true);
        assert!(request.body.get("max_output_tokens").is_none());
        assert!(request.body.get("temperature").is_none());
        assert_eq!(
            request.body["prompt_cache_key"],
            "6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b"
        );
        assert_eq!(
            request.body["include"],
            json!(["reasoning.encrypted_content"])
        );
        assert_eq!(request.body["reasoning"]["effort"], "medium");

        let mut no_token = ProviderHttpRequest::new("POST", &responses_url(), json!({}));
        let missing = shaper.shape(
            &mut no_token,
            &ShapeContext {
                account_id: "chatgpt",
                provider_account_id: Some("acct"),
                device_id: "",
                session_id: "s",
                request_id: "r",
                model: "m",
                access_token: None,
            },
        );
        assert_eq!(missing, Err(ShapeError::Missing("access_token")));
        assert_eq!(
            shaper.detect_drift(403, r#"{"detail":"invalid or disabled credential"}"#, &[]),
            Some(UnavailableReason::FingerprintDrift)
        );
        assert_eq!(
            shaper.detect_drift(403, "blocked by policy", &[]),
            Some(UnavailableReason::PolicyBlocked)
        );
        assert_eq!(shaper.detect_drift(429, "", &[]), None);
        assert_eq!(
            shaper.detect_drift(400, r#"{"detail":"Store must be set to false"}"#, &[]),
            Some(UnavailableReason::FingerprintDrift)
        );
    }
}
