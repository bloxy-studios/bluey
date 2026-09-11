//! ChatGPT (Codex OAuth) — the real `ProviderProfile` (ADR 0009 §4, PR 3a).
//!
//! Sign-in: PKCE in the system browser with the Codex CLI's loopback redirect
//! (port 1455, then 1457), falling back to the device-code flow when both ports
//! are taken or the caller asks for it. Import: the CLI's `auth.json`, read-only
//! (Bluey never refreshes at import time — OpenAI rotates refresh tokens, so an
//! eager refresh would sign the CLI out). Requests are shaped by
//! `bluey_protocols::codex::CodexShaper`; the pure logic lives in
//! `bluey_protocols::codex`, this file only does I/O.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use bluey_core::types::{
    AccountConnectOptions, AccountIdentity, AiProviderKind, CatalogSource, ConnectFlow,
    ConnectFlowKind, FingerprintProbe, LatencyBudget, ModelRole, ProviderModelCatalog,
    ReasoningLevel, CHATGPT_PROVIDER_ID,
};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_oauth::{
    poll_device_code, random_bytes, random_token, unix_now, DeviceFlowError, DevicePoll,
    LoopbackError, LoopbackListener, LoopbackPort, TokenSet,
};
use bluey_protocols::codex;
use bluey_protocols::fingerprints;
use bluey_protocols::oauth::{self, CallbackOutcome};
use bluey_protocols::request_shaper::{
    FingerprintInfo, ProviderHttpRequest, RequestShaper, ShapeContext,
};
use tokio_util::sync::CancellationToken;

use super::profile::{ConnectStart, Connected, ProviderProfile};

/// How long the browser may take to come back.
pub const BROWSER_FLOW_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);

/// Instruction templates from the last catalog fetch, per model slug — the
/// adapter sends the model's template as `instructions`, like the CLI does.
/// Process-wide: the profile fills it, the adapter reads it.
#[derive(Default)]
pub struct TemplateCache {
    inner: parking_lot::Mutex<HashMap<String, String>>,
}

impl TemplateCache {
    pub fn get(&self, slug: &str) -> Option<String> {
        self.inner.lock().get(slug).cloned()
    }

    pub fn replace(&self, templates: Vec<(String, String)>) {
        *self.inner.lock() = templates.into_iter().collect();
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
    }
}

pub fn templates() -> &'static TemplateCache {
    static CELL: OnceLock<TemplateCache> = OnceLock::new();
    CELL.get_or_init(TemplateCache::default)
}

/// `sw_vers -productVersion`, the CPU architecture and Bluey as the terminal
/// token of the User-Agent — read once per process.
pub fn client_environment() -> codex::CodexClientEnv {
    static CELL: OnceLock<codex::CodexClientEnv> = OnceLock::new();
    CELL.get_or_init(|| codex::CodexClientEnv {
        os_version: mac_os_version().unwrap_or_else(|| "26.0".to_string()),
        arch: match std::env::consts::ARCH {
            "aarch64" => "arm64".to_string(),
            other => other.to_string(),
        },
        terminal: "Bluey".to_string(),
    })
    .clone()
}

fn mac_os_version() -> Option<String> {
    let output = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    parse_sw_vers(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn parse_sw_vers(output: &str) -> Option<String> {
    let version = output.trim();
    (!version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .then(|| version.to_string())
}

/// Where the Codex CLI keeps its sign-in (`$CODEX_HOME/auth.json`, else `~/.codex/auth.json`).
pub fn codex_auth_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(codex::auth_json_path(
        &home,
        std::env::var("CODEX_HOME").ok().as_deref(),
    ))
}

fn shaper(fedramp: bool) -> codex::CodexShaper {
    codex::CodexShaper {
        env: client_environment(),
        fedramp,
    }
}

fn network(what: &str, error: &reqwest::Error) -> BlueyError {
    if error.is_timeout() {
        BlueyError::network("timeout", format!("{what} timed out"))
    } else if error.is_connect() {
        BlueyError::network("connect", format!("could not connect for {what}"))
    } else {
        BlueyError::network("request", format!("{what} failed to complete"))
    }
}

fn denied(message: impl Into<String>) -> BlueyError {
    BlueyError::account("denied", message)
}

fn import_not_found(message: impl Into<String>) -> BlueyError {
    BlueyError::account("import_not_found", message)
}

fn iso_after(secs: u64) -> String {
    codex::iso_from_unix(unix_now().saturating_add(secs))
}

/// A token set from a token-endpoint response; the expiry falls back to the
/// access token's own `exp` when the response carries no `expires_in`.
fn token_set(response: oauth::TokenResponse, previous_refresh: Option<String>) -> TokenSet {
    let now = unix_now();
    let expires_at = codex::access_token_expiry(&response.access_token, response.expires_in, now);
    let mut set = TokenSet::from_response(response, previous_refresh, now);
    set.expires_at = expires_at;
    set
}

fn connected_from(tokens: TokenSet) -> Connected {
    let identity = codex::identity_from_tokens(tokens.id_token.as_deref(), &tokens.access_token);
    Connected {
        identity: codex::account_identity(&identity),
        tokens,
    }
}

/// The provider-side account id every backend request carries.
fn provider_account_id(tokens: &TokenSet, identity: &AccountIdentity) -> BlueyResult<String> {
    identity
        .account_id
        .clone()
        .or_else(|| {
            codex::identity_from_tokens(tokens.id_token.as_deref(), &tokens.access_token).account_id
        })
        .ok_or_else(|| {
            BlueyError::account(
                "not_connected",
                "the stored ChatGPT sign-in carries no account id — reconnect the account",
            )
        })
}

/// Send a shaped request and read the whole response (status, headers, body text).
pub(crate) async fn send_shaped(
    http: &reqwest::Client,
    request: &ProviderHttpRequest,
    what: &str,
    timeout: Duration,
) -> BlueyResult<(u16, Vec<(String, String)>, String)> {
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| BlueyError::internal("invalid HTTP method"))?;
    let mut builder = http.request(method, &request.url).timeout(timeout);
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    if !request.body.is_null() {
        let bytes = serde_json::to_vec(&request.body)
            .map_err(|_| BlueyError::internal("cannot serialise the request body"))?;
        builder = builder.body(bytes);
    }
    let response = builder.send().await.map_err(|e| network(what, &e))?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = response.text().await.unwrap_or_default();
    Ok((status, headers, body))
}

async fn exchange(
    http: &reqwest::Client,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> BlueyResult<Connected> {
    let response = http
        .post(codex::TOKEN_URL)
        .timeout(HTTP_TIMEOUT)
        .form(&codex::token_exchange_form(code, redirect_uri, verifier))
        .send()
        .await
        .map_err(|e| network("the ChatGPT token exchange", &e))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status >= 400 {
        let parsed = codex::parse_error_body(&body);
        return Err(denied(format!(
            "ChatGPT did not complete the sign-in (HTTP {status}{})",
            parsed.code.map(|c| format!(", {c}")).unwrap_or_default()
        )));
    }
    let parsed = oauth::parse_token_response(&body)
        .map_err(|_| denied("ChatGPT returned an unexpected token response"))?;
    Ok(connected_from(token_set(parsed, None)))
}

/// Bind the CLI's callback port, or its allow-listed fallback.
async fn bind_loopback() -> Result<LoopbackListener, LoopbackError> {
    let mut last = LoopbackError::PortInUse(codex::LOOPBACK_PORTS[0]);
    for port in codex::LOOPBACK_PORTS {
        match LoopbackListener::bind(LoopbackPort::Fixed(port)).await {
            Ok(listener) => return Ok(listener),
            Err(LoopbackError::PortInUse(port)) => {
                tracing::info!(port, "codex callback port busy");
                last = LoopbackError::PortInUse(port);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

fn browser_flow(
    http: reqwest::Client,
    listener: LoopbackListener,
    cancel: CancellationToken,
) -> ConnectStart {
    let redirect = codex::redirect_uri(listener.port());
    let state = random_token();
    let (verifier, challenge) =
        oauth::pkce_pair_from(&random_bytes::<{ codex::PKCE_VERIFIER_BYTES }>());
    let url = codex::authorize_url(&redirect, &challenge, &state);
    let flow = ConnectFlow {
        kind: ConnectFlowKind::Browser,
        url: Some(url),
        user_code: None,
        verification_url: None,
        expires_at: iso_after(BROWSER_FLOW_TIMEOUT.as_secs()),
    };
    let completion_cancel = cancel.clone();
    let completion = Box::pin(async move {
        let accepted = match tokio::time::timeout(
            BROWSER_FLOW_TIMEOUT,
            listener.accept_one(&completion_cancel),
        )
        .await
        {
            Ok(Ok(accepted)) => accepted,
            Ok(Err(LoopbackError::Cancelled)) => return Err(BlueyError::cancelled()),
            Ok(Err(error)) => {
                return Err(BlueyError::network(
                    "connect",
                    format!("the sign-in callback failed: {error}"),
                ))
            }
            Err(_) => return Err(denied("the browser sign-in did not complete in time")),
        };
        let outcome = accepted
            .target
            .as_deref()
            .and_then(|target| url::Url::parse(&format!("http://localhost{target}")).ok())
            .map(|redirected| oauth::callback_outcome(&redirected));
        match outcome {
            Some(CallbackOutcome::Code {
                code,
                state: returned,
            }) if returned == state => {
                accepted
                    .responder
                    .respond_html(&oauth::loopback_html(true))
                    .await;
                exchange(&http, &code, &redirect, &verifier).await
            }
            Some(CallbackOutcome::Code { .. }) => {
                accepted
                    .responder
                    .respond_html(&oauth::loopback_html(false))
                    .await;
                Err(denied(
                    "the sign-in callback did not match the flow Bluey started",
                ))
            }
            Some(CallbackOutcome::Denied {
                error, description, ..
            }) => {
                accepted
                    .responder
                    .respond_html(&oauth::loopback_html(false))
                    .await;
                Err(denied(format!(
                    "ChatGPT sign-in was not completed ({error}{})",
                    description.map(|d| format!(": {d}")).unwrap_or_default()
                )))
            }
            None => {
                accepted
                    .responder
                    .respond_html(&oauth::loopback_html(false))
                    .await;
                Err(denied("the sign-in callback carried no code"))
            }
        }
    });
    ConnectStart {
        flow,
        completion,
        cancel,
        manual_code: None,
    }
}

async fn device_flow(
    http: &reqwest::Client,
    cancel: CancellationToken,
) -> BlueyResult<ConnectStart> {
    let response = http
        .post(codex::DEVICE_USERCODE_URL)
        .timeout(HTTP_TIMEOUT)
        .json(&codex::device_usercode_body())
        .send()
        .await
        .map_err(|e| network("the ChatGPT device sign-in", &e))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status == 404 {
        return Err(denied(
            "ChatGPT device sign-in is not available for this client right now — free the callback port (1455) and try again",
        ));
    }
    if status >= 400 {
        return Err(denied(format!(
            "ChatGPT refused to start the device sign-in (HTTP {status})"
        )));
    }
    let start = codex::parse_device_code_start(&body)
        .map_err(|e| denied(format!("unexpected device sign-in response: {e}")))?;
    let flow = ConnectFlow {
        kind: ConnectFlowKind::DeviceCode,
        url: Some(codex::DEVICE_VERIFY_URL.to_string()),
        user_code: Some(start.user_code.clone()),
        verification_url: Some(codex::DEVICE_VERIFY_URL.to_string()),
        expires_at: iso_after(start.expires_in_secs),
    };
    let http = http.clone();
    let poll_cancel = cancel.clone();
    let completion = Box::pin(async move {
        let poll_http = http.clone();
        let device_auth_id = start.device_auth_id.clone();
        let user_code = start.user_code.clone();
        let outcome = poll_device_code(
            Duration::from_secs(start.interval_secs),
            Duration::from_secs(start.expires_in_secs),
            &poll_cancel,
            move || {
                let http = poll_http.clone();
                let body = codex::device_poll_body(&device_auth_id, &user_code);
                async move {
                    match http
                        .post(codex::DEVICE_TOKEN_URL)
                        .timeout(HTTP_TIMEOUT)
                        .json(&body)
                        .send()
                        .await
                    {
                        Ok(response) => {
                            let status = response.status().as_u16();
                            let text = response.text().await.unwrap_or_default();
                            match codex::device_poll_outcome(status, &text) {
                                codex::DevicePollOutcome::Pending => DevicePoll::Pending,
                                codex::DevicePollOutcome::SlowDown => DevicePoll::SlowDown,
                                codex::DevicePollOutcome::Complete(auth) => {
                                    DevicePoll::Complete(auth)
                                }
                                codex::DevicePollOutcome::Failed(message) => DevicePoll::Failed(
                                    denied(format!("ChatGPT device sign-in failed: {message}")),
                                ),
                            }
                        }
                        // A network blip: keep polling until the code expires.
                        Err(_) => DevicePoll::Pending,
                    }
                }
            },
        )
        .await;
        let auth = match outcome {
            Ok(auth) => auth,
            Err(DeviceFlowError::Expired) => {
                return Err(denied(
                    "the device code expired before the sign-in finished",
                ))
            }
            Err(DeviceFlowError::Cancelled) => return Err(BlueyError::cancelled()),
            Err(DeviceFlowError::Failed(error)) => return Err(error),
        };
        exchange(
            &http,
            &auth.authorization_code,
            codex::DEVICE_REDIRECT_URI,
            &auth.code_verifier,
        )
        .await
    });
    Ok(ConnectStart {
        flow,
        completion,
        cancel,
        manual_code: None,
    })
}

#[derive(Default)]
pub struct ChatgptProfile;

#[async_trait]
impl ProviderProfile for ChatgptProfile {
    fn provider_id(&self) -> &'static str {
        CHATGPT_PROVIDER_ID
    }

    fn kind(&self) -> AiProviderKind {
        AiProviderKind::ChatgptCodex
    }

    fn display_name(&self) -> &'static str {
        "ChatGPT"
    }

    fn fingerprint(&self) -> Option<FingerprintInfo> {
        Some(fingerprints::codex::INFO)
    }

    async fn begin_connect(
        &self,
        http: &reqwest::Client,
        options: &AccountConnectOptions,
    ) -> BlueyResult<ConnectStart> {
        let cancel = CancellationToken::new();
        if !options.prefer_device_code {
            match bind_loopback().await {
                Ok(listener) => return Ok(browser_flow(http.clone(), listener, cancel)),
                Err(LoopbackError::PortInUse(port)) => {
                    tracing::info!(
                        port,
                        "codex callback ports busy — using the device-code flow"
                    );
                }
                Err(error) => {
                    return Err(BlueyError::network(
                        "connect",
                        format!("cannot open the sign-in callback port: {error}"),
                    ))
                }
            }
        }
        device_flow(http, cancel).await
    }

    async fn import(&self, _http: &reqwest::Client) -> BlueyResult<Connected> {
        let path =
            codex_auth_path().ok_or_else(|| import_not_found("cannot find your home directory"))?;
        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(import_not_found(
                    "the Codex CLI is not signed in on this Mac (no ~/.codex/auth.json) — run `codex login` first, or connect in the browser",
                ))
            }
            Err(error) => {
                return Err(import_not_found(format!(
                    "cannot read the Codex CLI sign-in: {error}"
                )))
            }
        };
        let imported = codex::parse_auth_json(&text).map_err(|error| match error {
            codex::ImportError::NotChatgptMode(mode) => import_not_found(format!(
                "the Codex CLI is signed in with an API key ({mode}), not a ChatGPT account"
            )),
            codex::ImportError::NoTokens => {
                import_not_found("the Codex CLI store holds no ChatGPT tokens")
            }
            codex::ImportError::Malformed(detail) => {
                import_not_found(format!("the Codex CLI store could not be read: {detail}"))
            }
        })?;
        let expires_at = codex::access_token_expiry(&imported.access_token, None, unix_now());
        let tokens = TokenSet {
            access_token: imported.access_token,
            refresh_token: imported.refresh_token,
            expires_at,
            id_token: imported.id_token,
        };
        // Copy only. Refreshing here would rotate the CLI's refresh token and sign it out;
        // Bluey refreshes when its own access token expires.
        tracing::info!("imported the Codex CLI sign-in (read-only)");
        Ok(connected_from(tokens))
    }

    async fn catalog(
        &self,
        http: &reqwest::Client,
        tokens: &TokenSet,
        identity: &AccountIdentity,
    ) -> BlueyResult<ProviderModelCatalog> {
        let account_id = provider_account_id(tokens, identity)?;
        let fedramp =
            codex::identity_from_tokens(tokens.id_token.as_deref(), &tokens.access_token).fedramp;
        let session_id = uuid::Uuid::new_v4().to_string();
        let request_id = uuid::Uuid::new_v4().to_string();
        let request = codex::models_request(
            &shaper(fedramp),
            &ShapeContext {
                account_id: CHATGPT_PROVIDER_ID,
                provider_account_id: Some(&account_id),
                device_id: "",
                session_id: &session_id,
                request_id: &request_id,
                model: "",
                access_token: Some(&tokens.access_token),
            },
        )
        .map_err(|e| BlueyError::internal(e.to_string()))?;
        let (status, headers, body) =
            send_shaped(http, &request, "the ChatGPT model catalog", HTTP_TIMEOUT).await?;
        if status >= 400 {
            return Err(codex::map_error(
                status,
                &headers,
                &body,
                codex::Endpoint::Models,
                unix_now(),
            ));
        }
        let response = codex::parse_models(&body).map_err(|_| {
            codex::catalog_unavailable("the Codex model catalog returned an unexpected shape")
        })?;
        templates().replace(codex::instructions_templates(&response));
        let models = codex::catalog_models(&response, identity.plan_tier.as_deref());
        if models.is_empty() {
            return Err(codex::catalog_unavailable(format!(
                "the Codex model catalog lists no models for this plan with client version {}",
                fingerprints::codex::CLIENT_VERSION
            )));
        }
        tracing::info!(models = models.len(), "fetched the ChatGPT model catalog");
        Ok(ProviderModelCatalog {
            account_id: CHATGPT_PROVIDER_ID.to_string(),
            provider_id: CHATGPT_PROVIDER_ID.to_string(),
            fetched_at: now_iso(),
            source: CatalogSource::Endpoint,
            models,
        })
    }

    async fn refresh(&self, http: &reqwest::Client, tokens: &TokenSet) -> BlueyResult<TokenSet> {
        let refresh_token = tokens
            .refresh_token
            .clone()
            .ok_or_else(codex::needs_reauth)?;
        let response = http
            .post(codex::TOKEN_URL)
            .timeout(HTTP_TIMEOUT)
            .json(&codex::refresh_body(&refresh_token))
            .send()
            .await
            .map_err(|e| network("the ChatGPT token refresh", &e))?;
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        if status >= 400 {
            return Err(match codex::classify_refresh_failure(status, &body) {
                codex::RefreshFailure::Permanent(code) => {
                    tracing::warn!(code, "chatgpt refresh token rejected");
                    codex::needs_reauth()
                }
                codex::RefreshFailure::Transient(code) => BlueyError::network(
                    "request",
                    format!("the ChatGPT token refresh failed ({code}) — will retry"),
                ),
            });
        }
        let parsed = oauth::parse_token_response(&body).map_err(|_| {
            BlueyError::network("request", "ChatGPT returned an unexpected refresh response")
        })?;
        let mut set = token_set(parsed, Some(refresh_token));
        if set.id_token.is_none() {
            set.id_token = tokens.id_token.clone();
        }
        Ok(set)
    }

    async fn revoke(&self, http: &reqwest::Client, tokens: &TokenSet) -> BlueyResult<()> {
        let token = tokens
            .refresh_token
            .clone()
            .unwrap_or_else(|| tokens.access_token.clone());
        let _ = http
            .post(codex::REVOKE_URL)
            .timeout(Duration::from_secs(10))
            .json(&codex::revoke_body(&token))
            .send()
            .await;
        Ok(())
    }

    async fn probe(
        &self,
        http: &reqwest::Client,
        tokens: &TokenSet,
    ) -> BlueyResult<FingerprintProbe> {
        let identity =
            codex::identity_from_tokens(tokens.id_token.as_deref(), &tokens.access_token);
        let wire = codex::account_identity(&identity);
        let account_id = provider_account_id(tokens, &wire)?;
        let catalog = self.catalog(http, tokens, &wire).await?;
        let model = catalog
            .models
            .iter()
            .find(|m| m.suggested_roles.contains(&ModelRole::Fast))
            .or_else(|| catalog.models.first())
            .map(|m| m.id.clone())
            .ok_or_else(|| codex::catalog_unavailable("no model to probe with"))?;
        let effort = codex::reasoning_effort(
            ReasoningLevel::None,
            LatencyBudget::UltraFast,
            &codex::model_reasoning(&catalog.models, &model),
            None,
        );
        let template = templates().get(&model);
        let session_id = uuid::Uuid::new_v4().to_string();
        let request_id = uuid::Uuid::new_v4().to_string();
        let mut request = ProviderHttpRequest::new(
            "POST",
            &codex::responses_url(),
            codex::probe_body(&model, &session_id, &effort, template.as_deref()),
        );
        shaper(identity.fedramp)
            .shape(
                &mut request,
                &ShapeContext {
                    account_id: CHATGPT_PROVIDER_ID,
                    provider_account_id: Some(&account_id),
                    device_id: "",
                    session_id: &session_id,
                    request_id: &request_id,
                    model: &model,
                    access_token: Some(&tokens.access_token),
                },
            )
            .map_err(|e| BlueyError::internal(e.to_string()))?;
        let (status, headers, body) =
            send_shaped(http, &request, "the ChatGPT probe", PROBE_TIMEOUT).await?;
        let outcome = codex::probe_outcome(status, &headers, &body);
        tracing::info!(status, ok = outcome.ok, billed_to = ?outcome.billed_to, "chatgpt fingerprint probe");
        Ok(FingerprintProbe {
            account_id: CHATGPT_PROVIDER_ID.to_string(),
            ok: outcome.ok,
            billed_to: outcome.billed_to,
            fingerprint_version: Some(fingerprints::codex::VERSION.to_string()),
            message: Some(format!("{model}: {}", outcome.message)),
            checked_at: now_iso(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sw_vers_output_is_a_dotted_version_or_nothing() {
        assert_eq!(parse_sw_vers("26.0.1\n"), Some("26.0.1".into()));
        assert_eq!(parse_sw_vers("15.7"), Some("15.7".into()));
        assert_eq!(parse_sw_vers(""), None);
        assert_eq!(parse_sw_vers("ProductVersion: 26"), None);
    }

    #[test]
    fn the_template_cache_replaces_wholesale() {
        let cache = TemplateCache::default();
        assert!(cache.is_empty());
        cache.replace(vec![("gpt-6-astra".into(), "T1".into())]);
        assert_eq!(cache.get("gpt-6-astra"), Some("T1".into()));
        assert_eq!(cache.get("other"), None);
        cache.replace(vec![("gpt-5.6-luna".into(), "T2".into())]);
        assert_eq!(cache.get("gpt-6-astra"), None);
        assert!(!cache.is_empty());
    }

    #[test]
    fn the_profile_reports_the_documented_fingerprint() {
        let profile = ChatgptProfile;
        assert_eq!(profile.provider_id(), "chatgpt");
        assert_eq!(profile.kind(), AiProviderKind::ChatgptCodex);
        assert_eq!(profile.fingerprint(), Some(fingerprints::codex::INFO));
        let env = client_environment();
        assert!(!env.os_version.is_empty());
        assert_eq!(env.terminal, "Bluey");
    }
}
