//! Claude Pro / Max — the real `ProviderProfile` (ADR 0009 §4b, PR 3b).
//!
//! Sign-in: PKCE in the system browser with Claude Code's loopback port 54545,
//! any free port when that one is taken, and the pasted `code#state` from
//! `platform.claude.com` as the last resort (or on request). Import: Claude
//! Code's Keychain item / credentials file, read-only and without refreshing
//! (token rotation would sign Claude Code out). Requests are shaped by
//! `bluey_protocols::claude_code::ClaudeCodeShaper`; this file only does I/O.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use bluey_core::types::{
    AccountConnectOptions, AccountIdentity, AiMessage, AiProviderKind, AiRole, CatalogSource,
    ConnectFlow, ConnectFlowKind, FingerprintProbe, LatencyBudget, ProviderModelCatalog,
    ReasoningLevel, CLAUDE_PROVIDER_ID,
};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_oauth::{
    random_bytes, random_token, unix_now, LoopbackError, LoopbackListener, LoopbackPort, TokenSet,
};
use bluey_protocols::anthropic;
use bluey_protocols::claude_code;
use bluey_protocols::fingerprints;
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
const FLOW_TIMEOUT: Duration = Duration::from_secs(claude_code::BROWSER_FLOW_SECS);
/// The cheapest model to probe billing with.
const PROBE_MODEL: &str = "claude-haiku-4-5-20251001";

#[derive(Default)]
pub struct ClaudeProfile;

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
    account_uuid: Option<&'a str>,
    device_id: &'a str,
    session_id: &'a str,
    request_id: &'a str,
    model: &'a str,
    access_token: &'a str,
) -> ShapeContext<'a> {
    ShapeContext {
        account_id: CLAUDE_PROVIDER_ID,
        provider_account_id: account_uuid,
        device_id,
        session_id,
        request_id,
        model,
        access_token: Some(access_token),
    }
}

/// `GET /api/oauth/profile` — who the token belongs to and which plan.
async fn fetch_profile(
    http: &reqwest::Client,
    access_token: &str,
) -> BlueyResult<claude_code::Profile> {
    let response = http
        .get(claude_code::PROFILE_URL)
        .timeout(HTTP_TIMEOUT)
        .bearer_auth(access_token)
        .header("anthropic-beta", claude_code::OAUTH_BETA)
        .header("accept", "application/json")
        .header("user-agent", fingerprints::claude_code::USER_AGENT)
        .send()
        .await
        .map_err(|e| network("the Claude profile lookup", &e))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status == 401 {
        return Err(claude_code::needs_reauth());
    }
    if status >= 400 {
        return Err(BlueyError::network(
            "request",
            format!("the Claude profile lookup failed (HTTP {status})"),
        ));
    }
    claude_code::parse_profile(&body).map_err(|_| {
        BlueyError::network("request", "Claude returned an unexpected profile response")
    })
}

async fn exchange(
    http: &reqwest::Client,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
    state: &str,
) -> BlueyResult<Connected> {
    let mut builder = http.post(claude_code::TOKEN_URL).timeout(HTTP_TIMEOUT);
    for (name, value) in claude_code::TOKEN_HEADERS {
        builder = builder.header(name, value);
    }
    let response = builder
        .json(&claude_code::token_exchange_body(
            code,
            redirect_uri,
            verifier,
            state,
        ))
        .send()
        .await
        .map_err(|e| network("the Claude token exchange", &e))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status >= 400 {
        let (error_type, _) = claude_code::error_fields(&body);
        return Err(denied(format!(
            "Claude did not complete the sign-in (HTTP {status}{})",
            error_type.map(|t| format!(", {t}")).unwrap_or_default()
        )));
    }
    let parsed = oauth::parse_token_response(&body)
        .map_err(|_| denied("Claude returned an unexpected token response"))?;
    let extras = claude_code::parse_token_extras(&body);
    let tokens = TokenSet::from_response(parsed, None, unix_now());
    let identity = match fetch_profile(http, &tokens.access_token).await {
        Ok(profile) => claude_code::account_identity(&profile),
        Err(error) => {
            tracing::debug!(code = %error.code, "claude profile lookup deferred; using the token response");
            AccountIdentity {
                email: extras.email,
                display_name: extras.organization_name,
                plan_tier: None,
                plan_label: Some("Claude".into()),
                account_id: extras.account_uuid,
                project_id: None,
            }
        }
    };
    Ok(Connected { tokens, identity })
}

fn loopback_flow(
    http: reqwest::Client,
    listener: LoopbackListener,
    cancel: CancellationToken,
) -> ConnectStart {
    let redirect = claude_code::redirect_uri(listener.port());
    let state = random_token();
    let (verifier, challenge) = oauth::pkce_pair(&random_bytes::<32>());
    let url = claude_code::authorize_url(&redirect, &challenge, &state);
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
                exchange(&http, &code, &redirect, &verifier, &state).await
            }
            Some(CallbackOutcome::Denied {
                error, description, ..
            }) => {
                accepted
                    .responder
                    .respond_html(&oauth::loopback_html(false))
                    .await;
                Err(denied(format!(
                    "Claude sign-in was not completed ({error}{})",
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

/// The redirect page shows `code#state`; the user pastes it into the account card.
fn manual_flow(http: reqwest::Client, cancel: CancellationToken) -> ConnectStart {
    let state = random_token();
    let (verifier, challenge) = oauth::pkce_pair(&random_bytes::<32>());
    let url = claude_code::authorize_url(claude_code::MANUAL_REDIRECT_URI, &challenge, &state);
    let flow = ConnectFlow {
        kind: ConnectFlowKind::ManualCode,
        url: Some(url),
        user_code: None,
        verification_url: None,
        expires_at: iso_after(FLOW_TIMEOUT.as_secs()),
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let completion_cancel = cancel.clone();
    let completion = Box::pin(async move {
        let pasted = tokio::select! {
            _ = completion_cancel.cancelled() => return Err(BlueyError::cancelled()),
            pasted = tokio::time::timeout(FLOW_TIMEOUT, rx) => match pasted {
                Ok(Ok(text)) => text,
                Ok(Err(_)) => return Err(BlueyError::cancelled()),
                Err(_) => return Err(denied("no sign-in code was pasted in time")),
            },
        };
        let parsed = oauth::parse_manual_code(&pasted).ok_or_else(|| {
            denied("that does not look like a Claude sign-in code (expected code#state)")
        })?;
        if parsed.state.as_deref().is_some_and(|s| s != state) {
            return Err(denied(
                "the pasted code belongs to a different sign-in — start again",
            ));
        }
        exchange(
            &http,
            &parsed.code,
            claude_code::MANUAL_REDIRECT_URI,
            &verifier,
            &state,
        )
        .await
    });
    ConnectStart {
        flow,
        completion,
        cancel,
        manual_code: Some(tx),
    }
}

/// Claude Code's stored sign-in: the Keychain item first, then the credentials file.
fn read_local_credentials() -> BlueyResult<(
    claude_code::ImportedCredentials,
    claude_code::OauthAccountInfo,
)> {
    let username = std::env::var("USER").unwrap_or_default();
    let mut raw: Option<String> = None;
    for account in claude_code::keychain_accounts(&username) {
        if let Ok(entry) = keyring::Entry::new(claude_code::KEYCHAIN_SERVICE, &account) {
            if let Ok(password) = entry.get_password() {
                raw = Some(password);
                break;
            }
        }
    }
    let home =
        dirs::home_dir().ok_or_else(|| import_not_found("cannot find your home directory"))?;
    if raw.is_none() {
        let path: PathBuf = claude_code::credentials_file(
            &home,
            std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref(),
        );
        raw = std::fs::read_to_string(&path).ok();
    }
    let Some(raw) = raw else {
        return Err(import_not_found(
            "Claude Code is not signed in on this Mac (no Keychain item or ~/.claude/.credentials.json) — run `claude` and sign in first, or connect in the browser",
        ));
    };
    let credentials = claude_code::parse_credentials(&raw).map_err(|error| match error {
        claude_code::ImportError::NoTokens => import_not_found(
            "Claude Code's store holds no claude.ai OAuth tokens (API-key sign-in?)",
        ),
        claude_code::ImportError::Malformed(detail) => {
            import_not_found(format!("Claude Code's store could not be read: {detail}"))
        }
    })?;
    let info = std::fs::read_to_string(claude_code::claude_json_path(&home))
        .map(|text| claude_code::parse_claude_json(&text))
        .unwrap_or_default();
    Ok((credentials, info))
}

#[async_trait]
impl ProviderProfile for ClaudeProfile {
    fn provider_id(&self) -> &'static str {
        CLAUDE_PROVIDER_ID
    }

    fn kind(&self) -> AiProviderKind {
        AiProviderKind::ClaudeSubscription
    }

    fn display_name(&self) -> &'static str {
        "Claude"
    }

    fn fingerprint(&self) -> Option<FingerprintInfo> {
        Some(fingerprints::claude_code::INFO)
    }

    async fn begin_connect(
        &self,
        http: &reqwest::Client,
        options: &AccountConnectOptions,
    ) -> BlueyResult<ConnectStart> {
        let cancel = CancellationToken::new();
        // Claude has no device-code flow; "prefer device code" means "skip the listener".
        if options.prefer_device_code {
            return Ok(manual_flow(http.clone(), cancel));
        }
        match LoopbackListener::bind(LoopbackPort::Fixed(claude_code::LOOPBACK_PORT)).await {
            Ok(listener) => return Ok(loopback_flow(http.clone(), listener, cancel)),
            Err(LoopbackError::PortInUse(port)) => {
                tracing::info!(port, "claude code's callback port busy — any free port")
            }
            Err(error) => tracing::warn!(%error, "cannot bind the fixed callback port"),
        }
        match LoopbackListener::bind(LoopbackPort::Any).await {
            Ok(listener) => Ok(loopback_flow(http.clone(), listener, cancel)),
            Err(error) => {
                tracing::warn!(%error, "no loopback listener — falling back to the pasted code");
                Ok(manual_flow(http.clone(), cancel))
            }
        }
    }

    async fn import(&self, http: &reqwest::Client) -> BlueyResult<Connected> {
        // The Keychain read may prompt the user; keep it off the async executor.
        let (credentials, info) = tokio::task::spawn_blocking(read_local_credentials)
            .await
            .map_err(|_| import_not_found("the Keychain lookup was interrupted"))??;
        let tokens = TokenSet {
            access_token: credentials.access_token.clone(),
            refresh_token: credentials.refresh_token.clone(),
            expires_at: credentials.expires_at,
            id_token: None,
        };
        // Copy only — refreshing here would rotate Claude Code's refresh token.
        let identity = match fetch_profile(http, &tokens.access_token).await {
            Ok(profile) => claude_code::account_identity(&profile),
            Err(error) => {
                tracing::debug!(code = %error.code, "claude profile lookup deferred; using the local store");
                claude_code::identity_from_import(&credentials, &info)
            }
        };
        tracing::info!("imported the Claude Code sign-in (read-only)");
        Ok(Connected { tokens, identity })
    }

    async fn catalog(
        &self,
        http: &reqwest::Client,
        tokens: &TokenSet,
        identity: &AccountIdentity,
    ) -> BlueyResult<ProviderModelCatalog> {
        let device_id = process_device_id();
        let session_id = uuid::Uuid::new_v4().to_string();
        let request_id = uuid::Uuid::new_v4().to_string();
        let request = claude_code::models_request(
            &claude_code::ClaudeCodeShaper,
            fingerprints::claude_code::UPSTREAM,
            &shape_context(
                identity.account_id.as_deref(),
                &device_id,
                &session_id,
                &request_id,
                "",
                &tokens.access_token,
            ),
        )
        .map_err(|e| BlueyError::internal(e.to_string()))?;
        let (status, headers, body) =
            send_shaped(http, &request, "the Claude model catalog", HTTP_TIMEOUT).await?;
        if status == 401 {
            return Err(claude_code::needs_reauth());
        }
        let (models, source) = if status < 400 {
            match claude_code::catalog_from_models_response(&body) {
                Ok(models) if !models.is_empty() => (models, CatalogSource::Endpoint),
                _ => {
                    tracing::info!(
                        "claude /v1/models answered without models — using the curated list"
                    );
                    (
                        claude_code::curated_catalog(),
                        CatalogSource::Curated {
                            version: fingerprints::claude_code::CLIENT_VERSION.to_string(),
                        },
                    )
                }
            }
        } else {
            let _ = headers;
            tracing::info!(
                status,
                "claude /v1/models refused the OAuth token — using the curated list"
            );
            (
                claude_code::curated_catalog(),
                CatalogSource::Curated {
                    version: fingerprints::claude_code::CLIENT_VERSION.to_string(),
                },
            )
        };
        Ok(ProviderModelCatalog {
            account_id: CLAUDE_PROVIDER_ID.to_string(),
            provider_id: CLAUDE_PROVIDER_ID.to_string(),
            fetched_at: now_iso(),
            source,
            models,
        })
    }

    async fn refresh(&self, http: &reqwest::Client, tokens: &TokenSet) -> BlueyResult<TokenSet> {
        let refresh_token = tokens
            .refresh_token
            .clone()
            .ok_or_else(claude_code::needs_reauth)?;
        let mut builder = http.post(claude_code::TOKEN_URL).timeout(HTTP_TIMEOUT);
        for (name, value) in claude_code::TOKEN_HEADERS {
            builder = builder.header(name, value);
        }
        let response = builder
            .json(&claude_code::refresh_body(&refresh_token))
            .send()
            .await
            .map_err(|e| network("the Claude token refresh", &e))?;
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        if status >= 400 {
            let (error_type, _) = claude_code::error_fields(&body);
            let permanent = status == 401
                || status == 403
                || (status == 400
                    && error_type.as_deref().is_some_and(|t| {
                        t.contains("invalid_grant") || t.contains("invalid_request")
                    }));
            return Err(if permanent {
                tracing::warn!(status, "claude refresh token rejected");
                claude_code::needs_reauth()
            } else {
                BlueyError::network(
                    "request",
                    format!("the Claude token refresh failed (HTTP {status}) — will retry"),
                )
            });
        }
        let parsed = oauth::parse_token_response(&body).map_err(|_| {
            BlueyError::network("request", "Claude returned an unexpected refresh response")
        })?;
        Ok(TokenSet::from_response(
            parsed,
            Some(refresh_token),
            unix_now(),
        ))
    }

    async fn revoke(&self, _http: &reqwest::Client, _tokens: &TokenSet) -> BlueyResult<()> {
        // No revocation endpoint is documented for claude.ai OAuth; forgetting the tokens is the sign-out.
        Ok(())
    }

    async fn probe(
        &self,
        http: &reqwest::Client,
        tokens: &TokenSet,
    ) -> BlueyResult<FingerprintProbe> {
        let profile = fetch_profile(http, &tokens.access_token).await?;
        let account_uuid = profile.account_uuid.clone().ok_or_else(|| {
            BlueyError::account("not_connected", "the Claude profile carries no account id")
        })?;
        let device_id = process_device_id();
        let session_id = uuid::Uuid::new_v4().to_string();
        let request_id = uuid::Uuid::new_v4().to_string();
        let messages = [AiMessage::text(
            AiRole::User,
            "Reply with the single word: ok",
        )];
        let mut body = anthropic::build_messages_body(&anthropic::MessagesBodyOptions {
            model: PROBE_MODEL,
            messages: &messages,
            stream: true,
            max_output_tokens: Some(16),
            temperature: None,
            output_schema: None,
            schema_as_prompt_fallback: false,
        });
        claude_code::apply_thinking(
            &mut body,
            PROBE_MODEL,
            ReasoningLevel::None,
            LatencyBudget::UltraFast,
        );
        let mut request = ProviderHttpRequest::new(
            "POST",
            &claude_code::messages_url(fingerprints::claude_code::UPSTREAM),
            body,
        );
        claude_code::ClaudeCodeShaper
            .shape(
                &mut request,
                &shape_context(
                    Some(&account_uuid),
                    &device_id,
                    &session_id,
                    &request_id,
                    PROBE_MODEL,
                    &tokens.access_token,
                ),
            )
            .map_err(|e| BlueyError::internal(e.to_string()))?;
        let (status, headers, body) =
            send_shaped(http, &request, "the Claude probe", PROBE_TIMEOUT).await?;
        let outcome = claude_code::probe_outcome(status, &headers, &body);
        tracing::info!(status, ok = outcome.ok, billed_to = ?outcome.billed_to, "claude fingerprint probe");
        Ok(FingerprintProbe {
            account_id: CLAUDE_PROVIDER_ID.to_string(),
            ok: outcome.ok,
            billed_to: outcome.billed_to,
            fingerprint_version: Some(fingerprints::claude_code::VERSION.to_string()),
            message: Some(format!("{PROBE_MODEL}: {}", outcome.message)),
            checked_at: now_iso(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_reports_the_documented_fingerprint() {
        let profile = ClaudeProfile;
        assert_eq!(profile.provider_id(), "claude");
        assert_eq!(profile.kind(), AiProviderKind::ClaudeSubscription);
        assert_eq!(profile.fingerprint(), Some(fingerprints::claude_code::INFO));
        assert_eq!(
            claude_code::redirect_uri(claude_code::LOOPBACK_PORT),
            "http://localhost:54545/callback"
        );
    }
}
