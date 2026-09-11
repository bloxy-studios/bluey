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
//!
//! The provider-agnostic machinery — PKCE and `state`/`nonce` generation, the
//! one-shot loopback listener, the token set kept in the Keychain — is the
//! `bluey-oauth` crate (runtime) and `bluey_protocols::oauth` (pure); this
//! module owns what is Clerk's: configuration, redirect styles, the OIDC
//! checks, `userinfo`, the Account Portal, and the state machine.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bluey_core::events::BlueyEvent;
use bluey_core::types::{AppEvent, AuthState, AuthStatus, AuthUser, SignInRedirect, SignInStart};
use bluey_core::{now_iso, BlueyError, BlueyErrorKind, BlueyResult};
use bluey_oauth::{
    unix_now, LoopbackError, LoopbackListener, LoopbackPort, TokenSet, DEFAULT_REFRESH_LEEWAY,
};
use bluey_protocols::clerk::{self, CallbackOutcome};
use bluey_storage::{SettingsRepository, UserRepository};
use chrono::SecondsFormat;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
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

fn iso_in(duration: Duration) -> String {
    let delta = chrono::Duration::from_std(duration).unwrap_or_else(|_| chrono::Duration::zero());
    (chrono::Utc::now() + delta).to_rfc3339_opts(SecondsFormat::Millis, true)
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

        let (code_verifier, code_challenge) = bluey_oauth::pkce();
        let state = bluey_oauth::random_token();
        let nonce = bluey_oauth::random_token();
        let cancel = CancellationToken::new();

        let (redirect_uri, listener) = match config.redirect {
            SignInRedirect::DeepLink => (clerk::DEEP_LINK_REDIRECT_URI.to_string(), None),
            SignInRedirect::Loopback => {
                let listener = LoopbackListener::bind(LoopbackPort::Any)
                    .await
                    .map_err(|e| {
                        let detail = match &e {
                            LoopbackError::Bind(io) => io.to_string(),
                            other => other.to_string(),
                        };
                        BlueyError::authentication(
                            "loopback_bind",
                            format!("cannot open the sign-in callback port: {detail}"),
                        )
                    })?;
                let port = listener.port();
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
    ) -> BlueyResult<TokenSet> {
        let status = response.status();
        let body = response.text().await.map_err(transport_error)?;
        if status.is_success() {
            let parsed = clerk::parse_token_response(&body).map_err(|_| {
                BlueyError::authentication("token_parse", "unexpected token response")
            })?;
            return Ok(TokenSet::from_response(
                parsed,
                previous_refresh,
                unix_now(),
            ));
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

    async fn refresh(&self, config: &OAuthConfig, tokens: &TokenSet) -> BlueyResult<TokenSet> {
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

    async fn load_tokens(&self) -> Option<TokenSet> {
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
    async fn store(&self, tokens: TokenSet, user: &AuthUser) -> BlueyResult<()> {
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
        if tokens.is_expiring(DEFAULT_REFRESH_LEEWAY, unix_now()) {
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
/// head (`bluey_oauth::LoopbackListener` — 127.0.0.1, ≤ 8 KB, 5 s), hand the
/// callback URL to the auth manager and answer with a small page. The socket
/// only lives for one flow; cancelling the flow ends the wait.
fn spawn_loopback(
    app: AppHandle,
    listener: LoopbackListener,
    port: u16,
    cancel: CancellationToken,
) {
    tauri::async_runtime::spawn(async move {
        let Ok(accepted) = listener.accept_one(&cancel).await else {
            return;
        };
        let outcome = match accepted.target {
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
        accepted
            .responder
            .respond_html(&clerk::loopback_html(outcome.is_ok()))
            .await;
    });
}

/// A public Clerk setting. The process environment — including the
/// `.env.local` / `.env` files loaded at boot — wins over the value `build.rs`
/// compiled in from the same files, which is what configures installed builds
/// (no file next to the binary). Empty values count as unset.
fn clerk_setting(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            baked_setting(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
}

/// Compile-time defaults emitted by `build.rs` — public identifiers only, the
/// allowlist lives there.
fn baked_setting(name: &str) -> Option<&'static str> {
    match name {
        "VITE_CLERK_PUBLISHABLE_KEY" => option_env!("BLUEY_BAKED_VITE_CLERK_PUBLISHABLE_KEY"),
        "VITE_CLERK_FRONTEND_API_URL" => option_env!("BLUEY_BAKED_VITE_CLERK_FRONTEND_API_URL"),
        "BLUEY_CLERK_OAUTH_CLIENT_ID" => option_env!("BLUEY_BAKED_BLUEY_CLERK_OAUTH_CLIENT_ID"),
        "BLUEY_CLERK_ACCOUNT_PORTAL_URL" => {
            option_env!("BLUEY_BAKED_BLUEY_CLERK_ACCOUNT_PORTAL_URL")
        }
        _ => None,
    }
}

/// OAuth configuration: issuer from the publishable key (or
/// `VITE_CLERK_FRONTEND_API_URL`), the public OAuth client id, the Account
/// Portal URL (explicit or derived) and the redirect style.
fn oauth_config_from_env() -> Option<OAuthConfig> {
    resolve_oauth_config(&clerk_setting, cfg!(debug_assertions))
}

/// The pure half of [`oauth_config_from_env`], over any settings lookup.
fn resolve_oauth_config(
    setting: &dyn Fn(&str) -> Option<String>,
    debug_build: bool,
) -> Option<OAuthConfig> {
    let Some(host) = fapi_host(setting) else {
        tracing::info!(
            "sign-in is not configured: no Clerk publishable key in the environment, .env/.env.local or the build"
        );
        return None;
    };
    let client_id = ["BLUEY_CLERK_OAUTH_CLIENT_ID", "CLERK_OAUTH_CLIENT_ID"]
        .iter()
        .find_map(|name| setting(name))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let Some(client_id) = client_id else {
        tracing::warn!(
            "a Clerk publishable key is set but BLUEY_CLERK_OAUTH_CLIENT_ID is not; sign-in stays disabled (see docs/DEVELOPMENT.md)"
        );
        return None;
    };
    let account_portal = setting("BLUEY_CLERK_ACCOUNT_PORTAL_URL")
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| v.starts_with("https://"))
        .or_else(|| clerk::account_portal_url(&host));
    let redirect = match setting("BLUEY_AUTH_REDIRECT")
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("loopback") => SignInRedirect::Loopback,
        Some("deep_link") | Some("deep-link") | Some("deeplink") => SignInRedirect::DeepLink,
        // Development builds are not installed in /Applications, so macOS never
        // routes `bluey://` to them; the loopback works everywhere.
        _ if debug_build => SignInRedirect::Loopback,
        _ => SignInRedirect::DeepLink,
    };
    Some(OAuthConfig {
        issuer: clerk::issuer_from_host(&host),
        client_id,
        account_portal,
        redirect,
    })
}

/// Frontend API host: an explicit `VITE_CLERK_FRONTEND_API_URL` (host or URL)
/// wins over the publishable key.
fn fapi_host(setting: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(explicit) = setting("VITE_CLERK_FRONTEND_API_URL") {
        let trimmed = explicit
            .trim()
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    let key = setting("VITE_CLERK_PUBLISHABLE_KEY").or_else(|| setting("CLERK_PUBLISHABLE_KEY"))?;
    clerk::fapi_host_from_publishable_key(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pk_test_` + base64("clerk.example.com$").
    const PUBLISHABLE_KEY: &str = "pk_test_Y2xlcmsuZXhhbXBsZS5jb20k";

    fn settings(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    #[test]
    fn oauth_config_needs_a_publishable_key_and_a_client_id() {
        assert!(resolve_oauth_config(&settings(&[]), true).is_none());
        assert!(resolve_oauth_config(
            &settings(&[("VITE_CLERK_PUBLISHABLE_KEY", PUBLISHABLE_KEY)]),
            true
        )
        .is_none());
        assert!(resolve_oauth_config(
            &settings(&[("BLUEY_CLERK_OAUTH_CLIENT_ID", "client_1")]),
            true
        )
        .is_none());
        // An empty assignment is not a client id.
        assert!(resolve_oauth_config(
            &settings(&[
                ("VITE_CLERK_PUBLISHABLE_KEY", PUBLISHABLE_KEY),
                ("BLUEY_CLERK_OAUTH_CLIENT_ID", "  "),
            ]),
            true
        )
        .is_none());
    }

    #[test]
    fn oauth_config_derives_issuer_portal_and_redirect_defaults() {
        let lookup = settings(&[
            ("VITE_CLERK_PUBLISHABLE_KEY", PUBLISHABLE_KEY),
            ("BLUEY_CLERK_OAUTH_CLIENT_ID", " client_1 "),
        ]);
        let debug = resolve_oauth_config(&lookup, true).unwrap();
        assert_eq!(debug.issuer, "https://clerk.example.com");
        assert_eq!(debug.client_id, "client_1");
        assert_eq!(
            debug.account_portal.as_deref(),
            Some("https://accounts.example.com/user")
        );
        assert!(matches!(debug.redirect, SignInRedirect::Loopback));
        let release = resolve_oauth_config(&lookup, false).unwrap();
        assert!(matches!(release.redirect, SignInRedirect::DeepLink));
    }

    #[test]
    fn oauth_config_honours_explicit_overrides() {
        let config = resolve_oauth_config(
            &settings(&[
                ("VITE_CLERK_FRONTEND_API_URL", "https://clerk.bluey.app/"),
                ("CLERK_OAUTH_CLIENT_ID", "client_2"),
                (
                    "BLUEY_CLERK_ACCOUNT_PORTAL_URL",
                    "https://accounts.bluey.app/user/",
                ),
                ("BLUEY_AUTH_REDIRECT", "deep_link"),
            ]),
            true,
        )
        .unwrap();
        assert_eq!(config.issuer, "https://clerk.bluey.app");
        assert_eq!(config.client_id, "client_2");
        assert_eq!(
            config.account_portal.as_deref(),
            Some("https://accounts.bluey.app/user")
        );
        assert!(matches!(config.redirect, SignInRedirect::DeepLink));

        // A non-https portal URL is ignored in favour of the derived one; the
        // redirect override works in release builds too.
        let config = resolve_oauth_config(
            &settings(&[
                ("VITE_CLERK_PUBLISHABLE_KEY", PUBLISHABLE_KEY),
                ("BLUEY_CLERK_OAUTH_CLIENT_ID", "client_1"),
                (
                    "BLUEY_CLERK_ACCOUNT_PORTAL_URL",
                    "http://insecure.example.com",
                ),
                ("BLUEY_AUTH_REDIRECT", "loopback"),
            ]),
            false,
        )
        .unwrap();
        assert_eq!(
            config.account_portal.as_deref(),
            Some("https://accounts.example.com/user")
        );
        assert!(matches!(config.redirect, SignInRedirect::Loopback));
    }

    #[test]
    fn stored_tokens_keep_the_keychain_json_shape() {
        // The Keychain entry written by earlier builds must still load.
        let raw = r#"{"access_token":"at","refresh_token":"rt","expires_at":1700000000,"id_token":"a.b.c"}"#;
        let tokens: TokenSet = serde_json::from_str(raw).unwrap();
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        assert!(tokens.is_expiring(DEFAULT_REFRESH_LEEWAY, 1_700_000_000));
        assert_eq!(serde_json::to_string(&tokens).unwrap(), raw);
    }
}
