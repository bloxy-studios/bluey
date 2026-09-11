//! Google AI Pro / Ultra — the real `ProviderProfile` (ADR 0009 §4c, PR 3c).
//!
//! Sign-in: Google OAuth with the Antigravity public client (five scopes, PKCE),
//! loopback port 51121 like every reference, any free port when it is taken;
//! then `loadCodeAssist` for the Cloud Code project and the plan, `onboardUser`
//! when the account has no project yet. Import: the Keychain item the standalone
//! app / `agy` keep, read-only. Requests are shaped by
//! `bluey_protocols::antigravity::AntigravityShaper`; this file only does I/O.
//!
//! The OAuth client secret is public in the shipped app but deliberately not
//! committed: the build supplies it — `BLUEY_ANTIGRAVITY_CLIENT_SECRET` in
//! `.env.local` / `.env`, baked in by `build.rs` like the Clerk identifiers, or
//! set in the environment (see `docs/PROVIDER_ACCOUNTS.md › Google AI`).
//! Without it the card reports the build as unable to sign in, *Connect*
//! explains, and the Gemini API key keeps working.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bluey_core::types::{
    AccountConnectOptions, AccountIdentity, AiMessage, AiProviderKind, AiRole, AiTask,
    CatalogSource, ConnectFlow, ConnectFlowKind, FingerprintProbe, LatencyBudget,
    ProviderModelCatalog, ReasoningLevel, ANTIGRAVITY_PROVIDER_ID,
};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_oauth::{
    random_bytes, random_token, unix_now, LoopbackError, LoopbackListener, LoopbackPort, TokenSet,
};
use bluey_protocols::antigravity as ag;
use bluey_protocols::fingerprints;
use bluey_protocols::gemini;
use bluey_protocols::oauth::{self, CallbackOutcome};
use bluey_protocols::request_shaper::{
    FingerprintInfo, ProviderHttpRequest, RequestShaper, ShapeContext,
};
use tokio_util::sync::CancellationToken;

use super::chatgpt::send_shaped;
use super::process_device_id;
use super::profile::{ConnectStart, Connected, ProviderProfile};

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(10);
const FLOW_TIMEOUT: Duration = Duration::from_secs(ag::BROWSER_FLOW_SECS);
/// The Hub manifest is re-read after this long (CLIProxyAPI: 6 h).
const MANIFEST_TTL: Duration = Duration::from_secs(6 * 60 * 60);
/// `onboardUser` is re-posted this often until the operation is `done`.
const ONBOARD_ATTEMPTS: u32 = 5;
const ONBOARD_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct AntigravityProfile;

/// The OAuth client secret: the process environment at run time (the shell, or
/// `.env.local` / `.env` loaded at boot in development), else the value
/// `build.rs` baked from those files at build time, else one exported in the
/// build shell. `None` = this build cannot sign in to Google.
pub fn client_secret() -> Option<String> {
    std::env::var("BLUEY_ANTIGRAVITY_CLIENT_SECRET")
        .ok()
        .or_else(|| option_env!("BLUEY_BAKED_BLUEY_ANTIGRAVITY_CLIENT_SECRET").map(str::to_string))
        .or_else(|| option_env!("BLUEY_ANTIGRAVITY_CLIENT_SECRET").map(str::to_string))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Why a build without the secret cannot start a Google sign-in — shown on the
/// account card and in the *Connect* error (never a token, never a value).
pub const MISSING_SECRET_DETAIL: &str = "this build of Bluey carries no Antigravity OAuth client secret — add BLUEY_ANTIGRAVITY_CLIENT_SECRET to .env.local and rebuild (docs/PROVIDER_ACCOUNTS.md › Google AI), or keep using a Gemini API key";

fn secret_or_error() -> BlueyResult<String> {
    client_secret().ok_or_else(|| {
        BlueyError::configuration("antigravity_client_secret", MISSING_SECRET_DETAIL)
            .recoverable(bluey_core::error::RecoveryAction::UseApiKey)
    })
}

/// The HTTP/1.1-only client Google traffic goes through: the native client
/// negotiates TLS without ALPN and never uses h2, and one OAuth identity never
/// shares a connection pool with anything else.
pub fn google_http() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .http1_only()
                .pool_idle_timeout(Duration::from_secs(90))
                .tcp_keepalive(Duration::from_secs(30))
                .build()
                .unwrap_or_default()
        })
        .clone()
}

static VERSION_CACHE: OnceLock<parking_lot::Mutex<Option<(Instant, String)>>> = OnceLock::new();

/// The Antigravity Hub version the User-Agent carries: the manifest's (cached
/// 6 h, floored at 2.9.1), else the fingerprint's own.
pub fn cached_client_version() -> String {
    VERSION_CACHE
        .get_or_init(Default::default)
        .lock()
        .as_ref()
        .map(|(_, version)| version.clone())
        .unwrap_or_else(|| fingerprints::antigravity::CLIENT_VERSION.to_string())
}

async fn refresh_client_version(http: &reqwest::Client) -> String {
    if let Some((fetched, version)) = VERSION_CACHE.get_or_init(Default::default).lock().clone() {
        if fetched.elapsed() < MANIFEST_TTL {
            return version;
        }
    }
    let manifest = match http
        .get(ag::MANIFEST_URL)
        .timeout(MANIFEST_TIMEOUT)
        .header("user-agent", ag::MANIFEST_USER_AGENT)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response.text().await.ok(),
        Ok(response) => {
            tracing::debug!(
                status = response.status().as_u16(),
                "antigravity manifest refused"
            );
            None
        }
        Err(error) => {
            tracing::debug!(%error, "antigravity manifest unreachable — using the fingerprint's version");
            None
        }
    };
    let version = ag::effective_version(manifest.as_deref());
    *VERSION_CACHE.get_or_init(Default::default).lock() = Some((Instant::now(), version.clone()));
    tracing::info!(version, "antigravity client version for the user-agent");
    version
}

fn shaper() -> ag::AntigravityShaper {
    ag::AntigravityShaper::new(cached_client_version(), ag::arch_label())
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
    bluey_protocols::codex::iso_from_unix(unix_now().saturating_add(secs))
}

fn shape_context<'a>(
    project: Option<&'a str>,
    device_id: &'a str,
    session_id: &'a str,
    request_id: &'a str,
    model: &'a str,
    access_token: &'a str,
) -> ShapeContext<'a> {
    ShapeContext {
        account_id: ANTIGRAVITY_PROVIDER_ID,
        provider_account_id: project,
        device_id,
        session_id,
        request_id,
        model,
        access_token: Some(access_token),
    }
}

/// A shaped Cloud Code call: the shaper sets the whole header set.
async fn cloud_code(
    http: &reqwest::Client,
    url: &str,
    body: serde_json::Value,
    access_token: &str,
    what: &str,
    timeout: Duration,
) -> BlueyResult<(u16, String)> {
    let device_id = process_device_id();
    let request_id = uuid::Uuid::new_v4().to_string();
    let mut request = ProviderHttpRequest::new("POST", url, body);
    shaper()
        .shape(
            &mut request,
            &shape_context(None, &device_id, "", &request_id, "", access_token),
        )
        .map_err(|e| BlueyError::internal(e.to_string()))?;
    let (status, _headers, body) = send_shaped(http, &request, what, timeout).await?;
    Ok((status, body))
}

/// `GET userinfo` — the e-mail for the card; `None` when Google does not answer
/// (a 401 is the token's problem and surfaces as such).
async fn fetch_userinfo(
    http: &reqwest::Client,
    access_token: &str,
) -> BlueyResult<Option<ag::UserInfo>> {
    let response = http
        .get(ag::USERINFO_URL)
        .timeout(HTTP_TIMEOUT)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| network("the Google account lookup", &e))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status == 401 {
        return Err(ag::needs_reauth());
    }
    if status >= 400 {
        tracing::debug!(
            status,
            "google userinfo refused; the card shows the plan only"
        );
        return Ok(None);
    }
    Ok(ag::parse_userinfo(&body).ok())
}

/// `loadCodeAssist` on the prod host.
async fn load_code_assist(
    http: &reqwest::Client,
    access_token: &str,
    project_hint: Option<&str>,
) -> BlueyResult<ag::CodeAssistInfo> {
    let (status, body) = cloud_code(
        http,
        &ag::load_code_assist_url(),
        ag::load_code_assist_body(project_hint),
        access_token,
        "the Cloud Code eligibility check",
        HTTP_TIMEOUT,
    )
    .await?;
    if status >= 400 {
        return Err(ag::map_error(status, &body, unix_now()));
    }
    ag::parse_load_code_assist(&body).map_err(|_| {
        BlueyError::network(
            "request",
            "Cloud Code returned an unexpected eligibility response",
        )
    })
}

/// `onboardUser` until the operation is done; the managed project it created.
async fn onboard(
    http: &reqwest::Client,
    access_token: &str,
    tier_id: &str,
    version: &str,
) -> BlueyResult<Option<String>> {
    for attempt in 1..=ONBOARD_ATTEMPTS {
        let (status, body) = cloud_code(
            http,
            &ag::onboard_user_url(fingerprints::antigravity::UPSTREAM),
            ag::onboard_user_body(tier_id, version),
            access_token,
            "the Cloud Code onboarding",
            HTTP_TIMEOUT,
        )
        .await?;
        if status >= 400 {
            return Err(ag::map_error(status, &body, unix_now()));
        }
        let state = ag::parse_onboard_response(&body).map_err(|_| {
            BlueyError::network(
                "request",
                "Cloud Code returned an unexpected onboarding response",
            )
        })?;
        if state.project.is_some() {
            return Ok(state.project);
        }
        if state.done {
            break;
        }
        tracing::debug!(attempt, "cloud code onboarding still running");
        tokio::time::sleep(ONBOARD_INTERVAL).await;
    }
    Ok(None)
}

/// Who the token belongs to and which project it works on.
async fn resolve_identity(
    http: &reqwest::Client,
    access_token: &str,
    project_hint: Option<&str>,
) -> BlueyResult<AccountIdentity> {
    let user = fetch_userinfo(http, access_token).await?;
    let info = load_code_assist(http, access_token, project_hint).await?;
    let mut project = project_hint
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or_else(|| info.project.clone());
    if project.is_none() {
        if let Some(link) = info.validation_url() {
            return Err(ag::policy_blocked(format!(
                "Google asks you to verify this account in the browser first: {link} — then reconnect"
            )));
        }
        let version = cached_client_version();
        project = onboard(http, access_token, &info.onboarding_tier(), &version).await?;
    }
    let Some(project) = project else {
        return Err(BlueyError::account(
            "unavailable",
            "this Google account (Workspace or enterprise) needs a Google Cloud project id — pass it with the connect options; the Accounts card field arrives with the next UI PR",
        )
        .recoverable(bluey_core::error::RecoveryAction::UseApiKey));
    };
    Ok(ag::account_identity(user.as_ref(), &info, Some(&project)))
}

async fn exchange(
    http: &reqwest::Client,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
    project_hint: Option<&str>,
) -> BlueyResult<Connected> {
    let secret = secret_or_error()?;
    let response = http
        .post(ag::TOKEN_URL)
        .timeout(HTTP_TIMEOUT)
        .form(&ag::token_exchange_form(
            code,
            redirect_uri,
            verifier,
            &secret,
        ))
        .send()
        .await
        .map_err(|e| network("the Google token exchange", &e))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status >= 400 {
        let detail = oauth::parse_oauth_error(&body)
            .map(|e| format!(", {}", e.error))
            .unwrap_or_default();
        return Err(denied(format!(
            "Google did not complete the sign-in (HTTP {status}{detail})"
        )));
    }
    let parsed = oauth::parse_token_response(&body)
        .map_err(|_| denied("Google returned an unexpected token response"))?;
    let tokens = TokenSet::from_response(parsed, None, unix_now());
    let identity = resolve_identity(http, &tokens.access_token, project_hint).await?;
    Ok(Connected { tokens, identity })
}

fn loopback_flow(
    http: reqwest::Client,
    listener: LoopbackListener,
    cancel: CancellationToken,
    project_hint: Option<String>,
) -> ConnectStart {
    let redirect = ag::redirect_uri(listener.port());
    let state = random_token();
    let (verifier, challenge) = oauth::pkce_pair(&random_bytes::<32>());
    let url = ag::authorize_url(&redirect, &challenge, &state);
    let flow = ConnectFlow {
        kind: ConnectFlowKind::Browser,
        url: Some(url),
        user_code: None,
        verification_url: None,
        expires_at: iso_after(FLOW_TIMEOUT.as_secs()),
    };
    let completion_cancel = cancel.clone();
    let completion = Box::pin(async move {
        let accepted =
            match tokio::time::timeout(FLOW_TIMEOUT, listener.accept_one(&completion_cancel)).await
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
                exchange(&http, &code, &redirect, &verifier, project_hint.as_deref()).await
            }
            Some(CallbackOutcome::Denied {
                error, description, ..
            }) => {
                accepted
                    .responder
                    .respond_html(&oauth::loopback_html(false))
                    .await;
                Err(denied(format!(
                    "Google sign-in was not completed ({error}{})",
                    description.map(|d| format!(": {d}")).unwrap_or_default()
                )))
            }
            _ => {
                accepted
                    .responder
                    .respond_html(&oauth::loopback_html(false))
                    .await;
                Err(denied(
                    "the sign-in callback did not match the flow Bluey started",
                ))
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

/// The standalone app's / `agy`'s Keychain item (generic password `gemini` / `antigravity`).
fn read_keychain_tokens() -> BlueyResult<ag::ImportedTokens> {
    let raw = keyring::Entry::new(ag::KEYCHAIN_SERVICE, ag::KEYCHAIN_ACCOUNT)
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .ok_or_else(|| {
            import_not_found(
                "Antigravity is not signed in on this Mac (no Keychain item `gemini` / `antigravity`) — open Antigravity or run `agy` and sign in first, or connect in the browser",
            )
        })?;
    ag::parse_keychain_payload(&raw).map_err(|error| match error {
        ag::ImportError::NoTokens => {
            import_not_found("Antigravity's Keychain item holds no Google OAuth tokens")
        }
        ag::ImportError::Malformed(detail) => import_not_found(format!(
            "Antigravity's Keychain item could not be read: {detail}"
        )),
    })
}

async fn refresh_tokens(http: &reqwest::Client, refresh_token: &str) -> BlueyResult<TokenSet> {
    let secret = secret_or_error()?;
    let response = http
        .post(ag::TOKEN_URL)
        .timeout(HTTP_TIMEOUT)
        .header("user-agent", ag::REFRESH_USER_AGENT)
        .form(&ag::refresh_form(refresh_token, &secret))
        .send()
        .await
        .map_err(|e| network("the Google token refresh", &e))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status >= 400 {
        let error = oauth::parse_oauth_error(&body).map(|e| e.error);
        let permanent = status == 401
            || error.as_deref().is_some_and(|e| {
                e == "invalid_grant" || e == "unauthorized_client" || e == "invalid_client"
            });
        return Err(if permanent {
            tracing::warn!(status, error = ?error, "google refresh token rejected");
            ag::needs_reauth()
        } else {
            BlueyError::network(
                "request",
                format!("the Google token refresh failed (HTTP {status}) — will retry"),
            )
        });
    }
    let parsed = oauth::parse_token_response(&body).map_err(|_| {
        BlueyError::network("request", "Google returned an unexpected refresh response")
    })?;
    Ok(TokenSet::from_response(
        parsed,
        Some(refresh_token.to_string()),
        unix_now(),
    ))
}

#[async_trait]
impl ProviderProfile for AntigravityProfile {
    fn provider_id(&self) -> &'static str {
        ANTIGRAVITY_PROVIDER_ID
    }

    fn kind(&self) -> AiProviderKind {
        AiProviderKind::AntigravityGoogle
    }

    fn display_name(&self) -> &'static str {
        "Google AI"
    }

    fn fingerprint(&self) -> Option<FingerprintInfo> {
        Some(fingerprints::antigravity::INFO)
    }

    fn unavailable_in_this_build(&self) -> Option<String> {
        client_secret()
            .is_none()
            .then(|| MISSING_SECRET_DETAIL.to_string())
    }

    async fn begin_connect(
        &self,
        _http: &reqwest::Client,
        options: &AccountConnectOptions,
    ) -> BlueyResult<ConnectStart> {
        secret_or_error()?;
        let http = google_http();
        refresh_client_version(&http).await;
        if options.prefer_device_code {
            tracing::debug!("google has no device-code flow; using the browser");
        }
        let cancel = CancellationToken::new();
        let project_hint = options.project_id.clone();
        match LoopbackListener::bind(LoopbackPort::Fixed(ag::LOOPBACK_PORT)).await {
            Ok(listener) => return Ok(loopback_flow(http, listener, cancel, project_hint)),
            Err(LoopbackError::PortInUse(port)) => {
                tracing::info!(port, "antigravity's callback port busy — any free port")
            }
            Err(error) => tracing::warn!(%error, "cannot bind the fixed callback port"),
        }
        match LoopbackListener::bind(LoopbackPort::Any).await {
            Ok(listener) => Ok(loopback_flow(http, listener, cancel, project_hint)),
            Err(error) => Err(BlueyError::network(
                "connect",
                format!("no loopback port for the Google sign-in callback: {error}"),
            )),
        }
    }

    async fn import(&self, _http: &reqwest::Client) -> BlueyResult<Connected> {
        let http = google_http();
        // The Keychain read may prompt the user; keep it off the async executor.
        let imported = tokio::task::spawn_blocking(read_keychain_tokens)
            .await
            .map_err(|_| import_not_found("the Keychain lookup was interrupted"))??;
        let mut tokens = TokenSet {
            access_token: imported.access_token.clone(),
            refresh_token: imported.refresh_token.clone(),
            expires_at: imported.expires_at,
            id_token: None,
        };
        // Google refresh tokens do not rotate, so renewing an expired access token
        // here leaves Antigravity's own sign-in intact.
        let expired = tokens
            .expires_at
            .is_some_and(|at| at <= unix_now().saturating_add(60));
        if expired {
            if let Some(refresh_token) = tokens.refresh_token.clone() {
                tokens = refresh_tokens(&http, &refresh_token).await?;
            }
        }
        let identity = resolve_identity(&http, &tokens.access_token, None).await?;
        tracing::info!("imported the Antigravity sign-in (read-only)");
        Ok(Connected { tokens, identity })
    }

    async fn catalog(
        &self,
        _http: &reqwest::Client,
        tokens: &TokenSet,
        identity: &AccountIdentity,
    ) -> BlueyResult<ProviderModelCatalog> {
        let http = google_http();
        let body = match identity.project_id.as_deref() {
            Some(project) => serde_json::json!({ "project": project }),
            None => serde_json::json!({}),
        };
        let (status, body) = cloud_code(
            &http,
            &ag::models_url(fingerprints::antigravity::UPSTREAM),
            body,
            &tokens.access_token,
            "the Google AI model list",
            HTTP_TIMEOUT,
        )
        .await?;
        if status == 401 || status == 403 {
            return Err(ag::map_error(status, &body, unix_now()));
        }
        let curated = || {
            (
                ag::curated_catalog(),
                CatalogSource::Curated {
                    version: fingerprints::antigravity::CLIENT_VERSION.to_string(),
                },
            )
        };
        let (models, source) = if status < 400 {
            match ag::parse_models_response(&body) {
                Ok(entries) if !entries.is_empty() => {
                    (ag::catalog_models(&entries), CatalogSource::Endpoint)
                }
                _ => {
                    tracing::info!("fetchAvailableModels answered without models — using the curated pool list");
                    curated()
                }
            }
        } else {
            tracing::info!(
                status,
                "fetchAvailableModels refused — using the curated pool list"
            );
            curated()
        };
        Ok(ProviderModelCatalog {
            account_id: ANTIGRAVITY_PROVIDER_ID.to_string(),
            provider_id: ANTIGRAVITY_PROVIDER_ID.to_string(),
            fetched_at: now_iso(),
            source,
            models,
        })
    }

    async fn refresh(&self, _http: &reqwest::Client, tokens: &TokenSet) -> BlueyResult<TokenSet> {
        let refresh_token = tokens.refresh_token.clone().ok_or_else(ag::needs_reauth)?;
        refresh_tokens(&google_http(), &refresh_token).await
    }

    async fn revoke(&self, _http: &reqwest::Client, tokens: &TokenSet) -> BlueyResult<()> {
        // Best effort: revoking the refresh token also ends the access token.
        let token = tokens
            .refresh_token
            .clone()
            .unwrap_or_else(|| tokens.access_token.clone());
        match google_http()
            .post(ag::REVOKE_URL)
            .timeout(HTTP_TIMEOUT)
            .form(&ag::revoke_form(&token))
            .send()
            .await
        {
            Ok(response) => {
                tracing::debug!(status = response.status().as_u16(), "google token revoked")
            }
            Err(error) => tracing::debug!(%error, "google token revocation skipped"),
        }
        Ok(())
    }

    async fn probe(
        &self,
        _http: &reqwest::Client,
        tokens: &TokenSet,
    ) -> BlueyResult<FingerprintProbe> {
        let http = google_http();
        let info = load_code_assist(&http, &tokens.access_token, None).await?;
        let project = info.project.clone().ok_or_else(|| {
            BlueyError::account(
                "unavailable",
                "the Google account has no Cloud Code project to probe with — reconnect it",
            )
        })?;
        let device_id = process_device_id();
        let session_id = uuid::Uuid::new_v4().to_string();
        let messages = [AiMessage::text(
            AiRole::User,
            "Reply with the single word: ok",
        )];
        let build = |identity: bool| {
            let mut body = gemini::build_generate_body(&gemini::GenerateBodyOptions {
                model: ag::PROBE_MODEL,
                messages: &messages,
                max_output_tokens: Some(16),
                temperature: None,
                output_schema: None,
                thinking_level: gemini::thinking_level_for(
                    AiTask::Answer,
                    LatencyBudget::UltraFast,
                    ReasoningLevel::None,
                    ag::PROBE_MODEL,
                ),
            });
            ag::normalise_request(&mut body, ag::PROBE_MODEL, ReasoningLevel::None);
            if identity {
                ag::inject_identity(&mut body);
            }
            body
        };
        let send = |body: serde_json::Value| {
            let http = http.clone();
            let device_id = device_id.clone();
            let session_id = session_id.clone();
            let project = project.clone();
            let access_token = tokens.access_token.clone();
            async move {
                let request_id = uuid::Uuid::new_v4().to_string();
                let mut request = ProviderHttpRequest::new(
                    "POST",
                    &ag::generate_url(fingerprints::antigravity::UPSTREAM),
                    body,
                );
                shaper()
                    .shape(
                        &mut request,
                        &shape_context(
                            Some(&project),
                            &device_id,
                            &session_id,
                            &request_id,
                            ag::PROBE_MODEL,
                            &access_token,
                        ),
                    )
                    .map_err(|e| BlueyError::internal(e.to_string()))?;
                let (status, _headers, body) =
                    send_shaped(&http, &request, "the Google AI probe", PROBE_TIMEOUT).await?;
                Ok::<_, BlueyError>(ag::probe_outcome(status, &body, unix_now()))
            }
        };
        // A/B: the plain request first; the archived references' identity text
        // only when Cloud Code rejects the plain shape (never on a Terms-of-Service 403).
        let plain = send(build(false)).await?;
        let outcome = if plain.ok
            || plain.message.contains("account.policy_blocked")
            || plain.message.contains("account.needs_reauth")
        {
            plain
        } else {
            let with_identity = send(build(true)).await?;
            let mut outcome = with_identity;
            outcome.message = if outcome.ok {
                format!(
                    "{} — only with the Antigravity identity text (plain request: {}); re-capture",
                    outcome.message, plain.message
                )
            } else {
                format!(
                    "plain: {} · with identity text: {}",
                    plain.message, outcome.message
                )
            };
            outcome
        };
        tracing::info!(ok = outcome.ok, billed_to = ?outcome.billed_to, "antigravity fingerprint probe");
        Ok(FingerprintProbe {
            account_id: ANTIGRAVITY_PROVIDER_ID.to_string(),
            ok: outcome.ok,
            billed_to: outcome.billed_to,
            fingerprint_version: Some(fingerprints::antigravity::VERSION.to_string()),
            message: Some(format!("{}: {}", ag::PROBE_MODEL, outcome.message)),
            checked_at: now_iso(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_reports_the_documented_fingerprint() {
        let profile = AntigravityProfile;
        assert_eq!(profile.provider_id(), "antigravity");
        assert_eq!(profile.kind(), AiProviderKind::AntigravityGoogle);
        assert_eq!(profile.fingerprint(), Some(fingerprints::antigravity::INFO));
        assert_eq!(
            ag::redirect_uri(ag::LOOPBACK_PORT),
            "http://localhost:51121/oauth-callback"
        );
        assert!(shaper().user_agent().starts_with("antigravity/hub/"));
    }

    #[test]
    fn a_build_without_the_secret_says_so_before_the_browser_opens() {
        let profile = AntigravityProfile;
        // The availability the card shows and the error *Connect* raises agree,
        // and both name the variable to set — never a value.
        assert_eq!(
            profile.unavailable_in_this_build().is_some(),
            client_secret().is_none()
        );
        let error = secret_or_error().err();
        assert_eq!(error.is_some(), client_secret().is_none());
        if let Some(error) = error {
            assert_eq!(error.code, "config.antigravity_client_secret");
            assert_eq!(error.message, MISSING_SECRET_DETAIL);
            assert_eq!(
                error.recovery,
                Some(bluey_core::error::RecoveryAction::UseApiKey)
            );
        }
        assert!(MISSING_SECRET_DETAIL.contains("BLUEY_ANTIGRAVITY_CLIENT_SECRET"));
        assert!(!MISSING_SECRET_DETAIL.contains("GOCSPX"));
    }
}
