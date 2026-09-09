//! Browser-based sign-in (ADR 0008).
//!
//! Clerk is the OAuth 2.0 / OpenID Connect provider and Bluey a *public*
//! client (PKCE, no secret): `auth_begin_sign_in` opens the system browser on
//! `{issuer}/oauth/authorize`; the redirect comes back through the
//! `bluey://auth/callback` deep link (installed builds) or a one-shot loopback
//! listener on `127.0.0.1` (development builds, which macOS does not register
//! for deep links); Rust exchanges the code, validates the ID token, fetches
//! `userinfo` and keeps the tokens in the Keychain. The WebView never talks to
//! Clerk and never sees a token — it only receives [`AuthStatus`].
//!
//! Configuration (environment / `.env`): `VITE_CLERK_PUBLISHABLE_KEY` (→
//! issuer), `BLUEY_CLERK_OAUTH_CLIENT_ID` (a *public* OAuth application in the
//! Clerk Dashboard with `bluey://auth/callback` and
//! `http://127.0.0.1/callback` as redirect URIs), optional
//! `BLUEY_CLERK_ACCOUNT_PORTAL_URL` and `BLUEY_AUTH_REDIRECT=deep_link|loopback`.
//!
//! Nothing secret is logged: not the authorization URL (it is harmless but
//! long), not callback URLs (they carry the code), never tokens.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bluey_core::events::BlueyEvent;
use bluey_core::types::{AppEvent, AuthState, AuthStatus, AuthUser, SignInRedirect, SignInStart};
use bluey_core::{now_iso, BlueyError, BlueyErrorKind, BlueyResult};
use bluey_protocols::clerk::{self, CallbackOutcome};
use bluey_storage::{SettingsRepository, UserRepository};
use chrono::SecondsFormat;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::events::EventBus;
use crate::secrets::{SecretsStore, CLERK_OAUTH_TOKENS_KEY, CLERK_TOKEN_KEY};
use crate::state::{AppCore, StateHub};
use crate::storage::Storage;

/// Settings-table key remembering which cached user is signed in.
const CURRENT_USER_KEY: &str = "auth_user_id";
/// A browser sign-in that has not returned within this window is abandoned.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
/// Refresh the access token this long before it expires.
const REFRESH_LEEWAY: Duration = Duration::from_secs(60);
/// Longest HTTP request head the loopback listener reads.
const LOOPBACK_MAX_HEAD: usize = 8 * 1024;

/// What the Keychain holds for the signed-in user (JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredTokens {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    id_token: Option<String>,
}

impl StoredTokens {
    fn from_response(response: clerk::TokenResponse, previous_refresh: Option<String>) -> Self {
        Self {
            access_token: response.access_token,
            refresh_token: response.refresh_token.or(previous_refresh),
            expires_at: response
                .expires_in
                .map(|secs| unix_now().saturating_add(secs)),
            id_token: response.id_token,
        }
    }

    fn is_expiring(&self, leeway: Duration, now: u64) -> bool {
        match self.expires_at {
            Some(at) => at.saturating_sub(leeway.as_secs()) <= now,
            None => false,
        }
    }
}

#[derive(Debug, Clone)]
struct OAuthConfig {
    issuer: String,
    client_id: String,
    account_portal: Option<String>,
    redirect: SignInRedirect,
}

struct PendingSignIn {
    state: String,
    code_verifier: String,
    nonce: String,
    redirect_uri: String,
    started: Instant,
    /// Window that started the flow; brought to the front when it completes.
    origin_window: Option<String>,
    /// Stops the loopback listener and the expiry watchdog.
    cancel: CancellationToken,
}

pub struct AuthManager {
    secrets: Arc<SecretsStore>,
    storage: Arc<Storage>,
    hub: Arc<StateHub>,
    bus: Arc<EventBus>,
    http: reqwest::Client,
    config: Option<OAuthConfig>,
    user: parking_lot::Mutex<Option<AuthUser>>,
    pending: parking_lot::Mutex<Option<PendingSignIn>>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn iso_in(duration: Duration) -> String {
    let delta = chrono::Duration::from_std(duration).unwrap_or_else(|_| chrono::Duration::zero());
    (chrono::Utc::now() + delta).to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// 16 random bytes, base64url (state / nonce).
fn random_token() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    clerk::base64url(&bytes)
}

fn not_configured() -> BlueyError {
    BlueyError::authentication(
        "not_configured",
        "sign-in is not configured — set VITE_CLERK_PUBLISHABLE_KEY and BLUEY_CLERK_OAUTH_CLIENT_ID",
    )
}

impl AuthManager {
    /// Resolve the OAuth configuration from the environment and restore the
    /// cached user (tokens are validated asynchronously by [`Self::restore`]).
    pub fn load(
        secrets: Arc<SecretsStore>,
        storage: Arc<Storage>,
        hub: Arc<StateHub>,
        bus: Arc<EventBus>,
        http: reqwest::Client,
    ) -> BlueyResult<Self> {
        let config = oauth_config_from_env();
        let user = storage
            .run_sync(|db| SettingsRepository::get_json(db, CURRENT_USER_KEY))?
            .and_then(|v| v.as_str().map(String::from))
            .and_then(|id| {
                storage
                    .run_sync(|db| UserRepository::get(db, &id))
                    .ok()
                    .flatten()
            });
        Ok(Self {
            secrets,
            storage,
            hub,
            bus,
            http,
            config,
            user: parking_lot::Mutex::new(user),
            pending: parking_lot::Mutex::new(None),
        })
    }

    /// Whether sign-in is configured at all. Without it the app boots straight
    /// into `ready` (local use without an account).
    pub fn auth_required(&self) -> bool {
        self.config.is_some()
    }

    /// Whether OAuth tokens are stored (bootstrap decides the initial state).
    pub fn has_stored_session(&self) -> bool {
        self.secrets.has_sync(CLERK_OAUTH_TOKENS_KEY)
    }

    pub async fn status(&self) -> BlueyResult<AuthStatus> {
        let has_stored_session = self.secrets.has(CLERK_OAUTH_TOKENS_KEY).await?;
        let user = self.user.lock().clone();
        let state = match (&user, has_stored_session) {
            (Some(_), _) => AuthState::SignedIn,
            (None, true) => AuthState::Unknown,
            (None, false) => AuthState::SignedOut,
        };
        Ok(AuthStatus {
            state,
            user,
            has_stored_session,
            configured: self.config.is_some(),
            sign_in_pending: self.pending.lock().is_some(),
            checked_at: now_iso(),
        })
    }

    async fn publish(&self) {
        if let Ok(status) = self.status().await {
            self.bus.publish(BlueyEvent::AuthChanged(status));
        }
    }

    /// Start a browser sign-in: PKCE + state + nonce, the redirect (deep link
    /// or a fresh loopback listener), open the authorization URL in the
    /// default browser. Any previous pending flow is abandoned.
    pub async fn begin_sign_in(
        &self,
        app: &AppHandle,
        origin_window: Option<String>,
    ) -> BlueyResult<SignInStart> {
        let config = self.config.clone().ok_or_else(not_configured)?;
        self.cancel_pending();

        let mut random = [0u8; 32];
        rand::rng().fill_bytes(&mut random);
        let (code_verifier, code_challenge) = clerk::pkce_pair(&random);
        let state = random_token();
        let nonce = random_token();
        let cancel = CancellationToken::new();

        let (redirect_uri, listener) = match config.redirect {
            SignInRedirect::DeepLink => (clerk::DEEP_LINK_REDIRECT_URI.to_string(), None),
            SignInRedirect::Loopback => {
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                    .await
                    .map_err(|e| {
                        BlueyError::authentication(
                            "loopback_bind",
                            format!("cannot open the sign-in callback port: {e}"),
                        )
                    })?;
                let port = listener
                    .local_addr()
                    .map_err(|e| BlueyError::internal(format!("loopback address: {e}")))?
                    .port();
                (clerk::loopback_redirect_uri(port), Some((listener, port)))
            }
        };
        let url = clerk::authorize_url(&clerk::AuthorizeRequest {
            issuer: &config.issuer,
            client_id: &config.client_id,
            redirect_uri: &redirect_uri,
            state: &state,
            code_challenge: &code_challenge,
            nonce: &nonce,
        })
        .map_err(|e| {
            BlueyError::authentication(
                "bad_issuer",
                format!("the Clerk issuer URL is invalid: {e}"),
            )
        })?;

        *self.pending.lock() = Some(PendingSignIn {
            state,
            code_verifier,
            nonce,
            redirect_uri,
            started: Instant::now(),
            origin_window,
            cancel: cancel.clone(),
        });
        if let Some((listener, port)) = listener {
            spawn_loopback(app.clone(), listener, port, cancel.clone());
        }
        // Expiry watchdog: a flow the browser never completes is forgotten.
        let watchdog_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = tokio::time::sleep(SIGN_IN_TIMEOUT) => {
                    watchdog_handle.state::<AppCore>().auth.expire_pending().await;
                }
            }
        });

        if let Err(e) = app.opener().open_url(&url, None::<&str>) {
            self.cancel_pending();
            return Err(BlueyError::authentication(
                "browser_open_failed",
                format!("could not open the browser: {e}"),
            ));
        }
        tracing::info!(redirect = ?config.redirect, "browser sign-in started");
        self.publish().await;
        Ok(SignInStart {
            url,
            redirect: config.redirect,
            expires_at: iso_in(SIGN_IN_TIMEOUT),
        })
    }

    /// Forget the pending flow (stops its listener / watchdog).
    fn cancel_pending(&self) {
        if let Some(pending) = self.pending.lock().take() {
            pending.cancel.cancel();
        }
    }

    /// `auth_cancel_sign_in`.
    pub async fn cancel_sign_in(&self) -> BlueyResult<AuthStatus> {
        self.cancel_pending();
        tracing::info!("browser sign-in cancelled");
        self.publish().await;
        self.status().await
    }

    /// Watchdog: drop a flow that outlived [`SIGN_IN_TIMEOUT`].
    async fn expire_pending(&self) {
        let expired = {
            let mut pending = self.pending.lock();
            match pending.as_ref() {
                Some(p) if p.started.elapsed() >= SIGN_IN_TIMEOUT => pending.take(),
                _ => None,
            }
        };
        if let Some(pending) = expired {
            pending.cancel.cancel();
            tracing::warn!("browser sign-in timed out; waiting for the browser no more");
            self.publish().await;
        }
    }

    /// A redirect arrived (deep link or loopback). Validates `state`, exchanges
    /// the code with PKCE, checks the ID token, loads `userinfo`, stores
    /// everything and brings the originating window to the front.
    pub async fn handle_callback_url(&self, app: &AppHandle, url: &str) -> BlueyResult<AuthStatus> {
        let Some(outcome) = clerk::parse_callback(url) else {
            return Err(BlueyError::authentication(
                "not_callback",
                "the URL is not a sign-in callback",
            ));
        };
        let config = self.config.clone().ok_or_else(not_configured)?;
        // One shot: whatever happens next, this flow is over.
        let Some(pending) = self.pending.lock().take() else {
            tracing::warn!("sign-in callback arrived with no sign-in in progress");
            return Err(BlueyError::authentication(
                "no_pending_sign_in",
                "no sign-in is in progress — start again from Bluey",
            ));
        };
        pending.cancel.cancel();

        let result = self.complete_sign_in(&config, &pending, outcome).await;
        match &result {
            Ok(()) => {
                tracing::info!("browser sign-in completed");
                if let Some(label) = pending.origin_window.as_deref() {
                    if let Err(e) = crate::platform::open_window(app, label, None) {
                        tracing::debug!(error = %e, "could not bring the window to front");
                    }
                }
            }
            Err(error) => {
                tracing::warn!(code = %error.code, "browser sign-in failed");
                // The command that started the flow returned long ago: surface
                // the failure through the error toast channel.
                self.bus.publish(BlueyEvent::AppError(error.clone()));
            }
        }
        self.publish().await;
        result?;
        self.status().await
    }

    async fn complete_sign_in(
        &self,
        config: &OAuthConfig,
        pending: &PendingSignIn,
        outcome: CallbackOutcome,
    ) -> BlueyResult<()> {
        let (code, state) = match outcome {
            CallbackOutcome::Code { code, state } => (code, state),
            CallbackOutcome::Denied {
                error, description, ..
            } => {
                let message = description
                    .filter(|d| !d.trim().is_empty())
                    .unwrap_or_else(|| format!("the sign-in was not completed ({error})"));
                return Err(BlueyError::authentication("denied", message));
            }
        };
        if state != pending.state {
            return Err(BlueyError::authentication(
                "state_mismatch",
                "the sign-in response did not match the request — start again from Bluey",
            ));
        }
        if pending.started.elapsed() > SIGN_IN_TIMEOUT {
            return Err(BlueyError::authentication(
                "expired",
                "the sign-in took too long — start again from Bluey",
            ));
        }
        let response = self
            .http
            .post(clerk::token_endpoint(&config.issuer))
            .form(&clerk::token_exchange_form(
                &config.client_id,
                &code,
                &pending.redirect_uri,
                &pending.code_verifier,
            ))
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(transport_error)?;
        let tokens = Self::read_tokens(response, None).await?;
        if let Some(id_token) = tokens.id_token.as_deref() {
            let claims = clerk::decode_id_token_claims(id_token).ok_or_else(|| {
                BlueyError::authentication("id_token_invalid", "the ID token could not be decoded")
            })?;
            clerk::validate_id_token(
                &claims,
                &config.issuer,
                &config.client_id,
                &pending.nonce,
                unix_now(),
            )
            .map_err(|reason| {
                BlueyError::authentication(
                    "id_token_invalid",
                    format!("the ID token failed validation: {reason}"),
                )
            })?;
        }
        let user = self.fetch_user(config, &tokens.access_token).await?;
        self.store(tokens, &user).await
    }

    /// Map a `/oauth/token` response: success → tokens; OAuth error bodies →
    /// `auth.<error>` (never logged, they can echo request data); 5xx → network.
    async fn read_tokens(
        response: reqwest::Response,
        previous_refresh: Option<String>,
    ) -> BlueyResult<StoredTokens> {
        let status = response.status();
        let body = response.text().await.map_err(transport_error)?;
        if status.is_success() {
            let parsed = clerk::parse_token_response(&body).map_err(|_| {
                BlueyError::authentication("token_parse", "unexpected token response")
            })?;
            return Ok(StoredTokens::from_response(parsed, previous_refresh));
        }
        if status.is_server_error() {
            return Err(BlueyError::network(
                "http_5xx",
                format!(
                    "the sign-in service is unavailable (HTTP {})",
                    status.as_u16()
                ),
            ));
        }
        let error = clerk::parse_oauth_error(&body);
        let code = error
            .as_ref()
            .map(|e| e.error.clone())
            .unwrap_or_else(|| format!("http_{}", status.as_u16()));
        let message = match code.as_str() {
            "invalid_grant" => "the sign-in code was rejected (already used or expired) — start again from Bluey".to_string(),
            "invalid_client" | "unauthorized_client" => "Clerk does not know this OAuth client — check BLUEY_CLERK_OAUTH_CLIENT_ID and the redirect URIs of the OAuth application".to_string(),
            _ => format!("the sign-in service rejected the request ({code})"),
        };
        Err(BlueyError::authentication(&code, message))
    }

    /// `GET /oauth/userinfo` with the access token.
    async fn fetch_user(&self, config: &OAuthConfig, access_token: &str) -> BlueyResult<AuthUser> {
        let response = self
            .http
            .get(clerk::userinfo_endpoint(&config.issuer))
            .bearer_auth(access_token)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(BlueyError::authentication(
                "session_rejected",
                "the stored sign-in is no longer valid",
            ));
        }
        if !status.is_success() {
            return Err(BlueyError::network(
                "request",
                format!("the sign-in service returned HTTP {}", status.as_u16()),
            ));
        }
        let body = response.text().await.map_err(transport_error)?;
        let info = clerk::parse_userinfo(&body).map_err(|_| {
            BlueyError::authentication("userinfo_parse", "unexpected userinfo response")
        })?;
        Ok(clerk::user_from_userinfo(info))
    }

    async fn refresh(
        &self,
        config: &OAuthConfig,
        tokens: &StoredTokens,
    ) -> BlueyResult<StoredTokens> {
        let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
            BlueyError::authentication("no_refresh_token", "the stored sign-in cannot be renewed")
        })?;
        let response = self
            .http
            .post(clerk::token_endpoint(&config.issuer))
            .form(&clerk::token_refresh_form(
                &config.client_id,
                &refresh_token,
            ))
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(transport_error)?;
        Self::read_tokens(response, Some(refresh_token)).await
    }

    async fn load_tokens(&self) -> Option<StoredTokens> {
        let raw = self
            .secrets
            .get(CLERK_OAUTH_TOKENS_KEY)
            .await
            .ok()
            .flatten()?;
        serde_json::from_str(&raw).ok()
    }

    /// Persist tokens (Keychain) and the user (SQLite, no tokens); the state
    /// machine leaves `auth_required`.
    async fn store(&self, tokens: StoredTokens, user: &AuthUser) -> BlueyResult<()> {
        let raw = serde_json::to_string(&tokens)
            .map_err(|_| BlueyError::internal("cannot serialise the sign-in tokens"))?;
        self.secrets.set(CLERK_OAUTH_TOKENS_KEY, raw).await?;
        let cache = user.clone();
        self.storage
            .run(move |db| {
                UserRepository::upsert(db, &cache)?;
                SettingsRepository::set_json(
                    db,
                    CURRENT_USER_KEY,
                    &serde_json::Value::String(cache.id.clone()),
                )
            })
            .await?;
        *self.user.lock() = Some(user.clone());
        self.hub.transition_soft(AppEvent::Authenticated);
        Ok(())
    }

    /// Boot: drop the legacy client token, then validate the stored sign-in —
    /// refresh when expiring, sign out when Clerk rejects it, keep the cached
    /// user when merely offline.
    pub async fn restore(&self) {
        if self.secrets.has(CLERK_TOKEN_KEY).await.unwrap_or(false)
            && self.secrets.delete(CLERK_TOKEN_KEY).await.is_ok()
        {
            tracing::info!("removed the legacy Clerk client token");
        }
        let Some(config) = self.config.clone() else {
            return;
        };
        let Some(mut tokens) = self.load_tokens().await else {
            return;
        };
        if tokens.is_expiring(REFRESH_LEEWAY, unix_now()) {
            match self.refresh(&config, &tokens).await {
                Ok(fresh) => tokens = fresh,
                Err(error) if error.kind == BlueyErrorKind::Authentication => {
                    tracing::warn!(code = %error.code, "stored sign-in could not be renewed; signing out");
                    let _ = self.clear_session().await;
                    return;
                }
                Err(error) => {
                    tracing::debug!(code = %error.code, "sign-in renewal deferred (offline?)");
                    return;
                }
            }
        }
        match self.fetch_user(&config, &tokens.access_token).await {
            Ok(user) => {
                if let Err(error) = self.store(tokens, &user).await {
                    tracing::warn!(code = %error.code, "could not persist the restored sign-in");
                }
                self.publish().await;
            }
            Err(error) if error.kind == BlueyErrorKind::Authentication => {
                // The access token may just be stale: one refresh attempt.
                match self.refresh(&config, &tokens).await {
                    Ok(fresh) => match self.fetch_user(&config, &fresh.access_token).await {
                        Ok(user) => {
                            let _ = self.store(fresh, &user).await;
                            self.publish().await;
                        }
                        Err(_) => {
                            tracing::warn!("stored sign-in rejected by Clerk; signing out");
                            let _ = self.clear_session().await;
                        }
                    },
                    Err(_) => {
                        tracing::warn!("stored sign-in rejected by Clerk; signing out");
                        let _ = self.clear_session().await;
                    }
                }
            }
            Err(error) => {
                tracing::debug!(code = %error.code, "sign-in check deferred (offline?)");
            }
        }
    }

    /// Sign out: revoke the tokens (best effort), forget them and the cached
    /// user, move the state machine to `auth_required`.
    pub async fn clear_session(&self) -> BlueyResult<AuthStatus> {
        self.cancel_pending();
        if let (Some(config), Some(tokens)) = (self.config.as_ref(), self.load_tokens().await) {
            for token in
                std::iter::once(tokens.access_token.clone()).chain(tokens.refresh_token.clone())
            {
                let _ = self
                    .http
                    .post(clerk::revoke_endpoint(&config.issuer))
                    .form(&clerk::token_revoke_form(&config.client_id, &token))
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await;
            }
        }
        self.secrets.delete(CLERK_OAUTH_TOKENS_KEY).await?;
        self.storage
            .run(|db| {
                UserRepository::clear(db)?;
                SettingsRepository::set_json(db, CURRENT_USER_KEY, &serde_json::Value::Null)
            })
            .await?;
        *self.user.lock() = None;
        if self.auth_required() {
            self.hub.transition_soft(AppEvent::SignedOut);
        }
        tracing::info!("signed out");
        self.publish().await;
        self.status().await
    }

    /// Clerk's hosted Account Portal (profile, security) in the browser.
    pub fn open_account_portal(&self, app: &AppHandle) -> BlueyResult<()> {
        let config = self.config.as_ref().ok_or_else(not_configured)?;
        let url = config.account_portal.as_deref().ok_or_else(|| {
            BlueyError::authentication(
                "no_account_portal",
                "no Account Portal URL is known for this Clerk instance — set BLUEY_CLERK_ACCOUNT_PORTAL_URL",
            )
        })?;
        app.opener().open_url(url, None::<&str>).map_err(|e| {
            BlueyError::authentication(
                "browser_open_failed",
                format!("could not open the browser: {e}"),
            )
        })
    }
}

fn transport_error(error: reqwest::Error) -> BlueyError {
    if error.is_timeout() {
        BlueyError::network("timeout", "the sign-in request timed out")
    } else {
        BlueyError::network("request", "the sign-in request failed")
    }
}

/// One-shot loopback listener: accept a single connection, read the request
/// head, hand the callback URL to the auth manager and answer with a small
/// page. The socket is bound to 127.0.0.1 on an OS-chosen port and only lives
/// for one flow.
fn spawn_loopback(
    app: AppHandle,
    listener: tokio::net::TcpListener,
    port: u16,
    cancel: CancellationToken,
) {
    tauri::async_runtime::spawn(async move {
        let accepted = tokio::select! {
            _ = cancel.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let Ok((mut stream, _)) = accepted else {
            return;
        };
        let mut buffer = vec![0u8; LOOPBACK_MAX_HEAD];
        let mut length = 0usize;
        let read = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let n = stream.read(&mut buffer[length..]).await?;
                if n == 0 {
                    break;
                }
                length += n;
                if buffer[..length].windows(4).any(|w| w == b"\r\n\r\n") || length == buffer.len() {
                    break;
                }
            }
            Ok::<(), std::io::Error>(())
        })
        .await;
        let head = String::from_utf8_lossy(&buffer[..length]).into_owned();
        let target = match read {
            Ok(Ok(())) => clerk::http_request_target(&head).map(str::to_string),
            _ => None,
        };
        let outcome = match target {
            Some(target) => {
                let core = app.state::<AppCore>();
                core.auth
                    .handle_callback_url(&app, &format!("http://127.0.0.1:{port}{target}"))
                    .await
                    .map(|_| ())
            }
            None => Err(BlueyError::authentication(
                "invalid_callback",
                "malformed callback request",
            )),
        };
        let body = clerk::loopback_html(outcome.is_ok());
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    });
}

/// OAuth configuration from the environment: issuer from the publishable key
/// (or `VITE_CLERK_FRONTEND_API_URL`), the public OAuth client id, the
/// Account Portal URL (explicit or derived) and the redirect style.
fn oauth_config_from_env() -> Option<OAuthConfig> {
    let host = fapi_host_from_env()?;
    let client_id = ["BLUEY_CLERK_OAUTH_CLIENT_ID", "CLERK_OAUTH_CLIENT_ID"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let Some(client_id) = client_id else {
        tracing::warn!(
            "a Clerk publishable key is set but BLUEY_CLERK_OAUTH_CLIENT_ID is not; sign-in stays disabled (see docs/DEVELOPMENT.md)"
        );
        return None;
    };
    let account_portal = std::env::var("BLUEY_CLERK_ACCOUNT_PORTAL_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| v.starts_with("https://"))
        .or_else(|| clerk::account_portal_url(&host));
    let redirect = match std::env::var("BLUEY_AUTH_REDIRECT")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("loopback") => SignInRedirect::Loopback,
        Some("deep_link") | Some("deep-link") | Some("deeplink") => SignInRedirect::DeepLink,
        // Development builds are not installed in /Applications, so macOS never
        // routes `bluey://` to them; the loopback works everywhere.
        _ if cfg!(debug_assertions) => SignInRedirect::Loopback,
        _ => SignInRedirect::DeepLink,
    };
    Some(OAuthConfig {
        issuer: clerk::issuer_from_host(&host),
        client_id,
        account_portal,
        redirect,
    })
}

/// Frontend API host from the environment: an explicit
/// `VITE_CLERK_FRONTEND_API_URL` (host or URL) wins over the publishable key.
fn fapi_host_from_env() -> Option<String> {
    if let Ok(explicit) = std::env::var("VITE_CLERK_FRONTEND_API_URL") {
        let trimmed = explicit
            .trim()
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    let key = std::env::var("VITE_CLERK_PUBLISHABLE_KEY")
        .or_else(|_| std::env::var("CLERK_PUBLISHABLE_KEY"))
        .ok()?;
    clerk::fapi_host_from_publishable_key(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_tokens_track_expiry_with_leeway() {
        let tokens = StoredTokens {
            access_token: "a".into(),
            refresh_token: None,
            expires_at: Some(1_000),
            id_token: None,
        };
        assert!(!tokens.is_expiring(Duration::from_secs(60), 900));
        assert!(tokens.is_expiring(Duration::from_secs(60), 940));
        assert!(tokens.is_expiring(Duration::from_secs(60), 2_000));
        let forever = StoredTokens {
            expires_at: None,
            ..tokens.clone()
        };
        assert!(!forever.is_expiring(Duration::from_secs(60), u64::MAX));
    }

    #[test]
    fn a_refresh_response_without_a_new_refresh_token_keeps_the_old_one() {
        let response =
            clerk::parse_token_response(r#"{"access_token":"new","expires_in":60}"#).unwrap();
        let tokens = StoredTokens::from_response(response, Some("old-rt".into()));
        assert_eq!(tokens.access_token, "new");
        assert_eq!(tokens.refresh_token.as_deref(), Some("old-rt"));
        assert!(tokens.expires_at.unwrap() > unix_now());
        let rotated =
            clerk::parse_token_response(r#"{"access_token":"n","refresh_token":"rt2"}"#).unwrap();
        assert_eq!(
            StoredTokens::from_response(rotated, Some("old".into()))
                .refresh_token
                .as_deref(),
            Some("rt2")
        );
    }

    #[test]
    fn random_tokens_are_url_safe_and_unique() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 22);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}
