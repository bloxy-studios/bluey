//! Clerk session persistence (ADR 0003). Clerk itself runs in the WebView; Rust
//! only keeps the client JWT in the Keychain (`auth:clerk:client_token`), caches
//! the signed-in user profile in SQLite (no tokens) and offers the strict
//! `auth_fapi_fetch` proxy limited to the instance's Frontend API host.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bluey_core::types::{AppEvent, AuthState, AuthStatus, AuthUser};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_protocols::clerk;
use bluey_storage::{SettingsRepository, UserRepository};
use serde::Serialize;

use crate::secrets::{SecretsStore, CLERK_TOKEN_KEY};
use crate::state::StateHub;
use crate::storage::Storage;

/// Settings-table key remembering which cached user is signed in.
const CURRENT_USER_KEY: &str = "auth_user_id";
/// Response headers worth forwarding to clerk-js.
const FORWARDED_RESPONSE_HEADERS: &[&str] =
    &["authorization", "content-type", "x-clerk-auth-reason"];

/// Result of `auth_fapi_fetch` (mirrors the TS shape).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FapiResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

pub struct AuthManager {
    secrets: Arc<SecretsStore>,
    storage: Arc<Storage>,
    hub: Arc<StateHub>,
    http: reqwest::Client,
    fapi_host: Option<String>,
    user: parking_lot::Mutex<Option<AuthUser>>,
}

impl AuthManager {
    /// Resolve the Clerk Frontend API host (`VITE_CLERK_FRONTEND_API_URL` or the
    /// publishable key) and restore the cached user.
    pub fn load(
        secrets: Arc<SecretsStore>,
        storage: Arc<Storage>,
        hub: Arc<StateHub>,
        http: reqwest::Client,
    ) -> BlueyResult<Self> {
        let fapi_host = fapi_host_from_env();
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
            http,
            fapi_host,
            user: parking_lot::Mutex::new(user),
        })
    }

    /// Whether Clerk is configured at all. Without a publishable key the app
    /// boots straight into `ready` (local development without an account).
    pub fn auth_required(&self) -> bool {
        self.fapi_host.is_some()
    }

    /// Whether a client token is stored (bootstrap decides the initial state).
    pub fn has_stored_session(&self) -> bool {
        self.secrets.has_sync(CLERK_TOKEN_KEY)
    }

    pub async fn status(&self) -> BlueyResult<AuthStatus> {
        let has_stored_session = self.secrets.has(CLERK_TOKEN_KEY).await?;
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
            checked_at: now_iso(),
        })
    }

    /// Persist a full session (token + user) after Clerk signs in.
    pub async fn store_session(
        &self,
        client_token: String,
        user: AuthUser,
    ) -> BlueyResult<AuthStatus> {
        self.secrets.set(CLERK_TOKEN_KEY, client_token).await?;
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
        *self.user.lock() = Some(user);
        self.hub.transition_soft(AppEvent::Authenticated);
        self.status().await
    }

    /// Persist a rotated client JWT (user unchanged).
    pub async fn store_token(&self, client_token: String) -> BlueyResult<()> {
        self.secrets.set(CLERK_TOKEN_KEY, client_token).await
    }

    /// The stored client JWT, replayed by clerk-js on load.
    pub async fn load_client_token(&self) -> BlueyResult<Option<String>> {
        self.secrets.get(CLERK_TOKEN_KEY).await
    }

    /// Sign out: drop the token, the cached user and move the state machine to
    /// `auth_required`.
    pub async fn clear_session(&self) -> BlueyResult<AuthStatus> {
        self.secrets.delete(CLERK_TOKEN_KEY).await?;
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
        self.status().await
    }

    /// Strict proxy for Clerk Frontend API calls (ADR 0003 fallback): https
    /// only, exactly the configured host, bodies never logged.
    pub async fn fapi_fetch(
        &self,
        url: String,
        method: String,
        headers: HashMap<String, String>,
        body: Option<String>,
    ) -> BlueyResult<FapiResponse> {
        let host = self.fapi_host.as_deref().ok_or_else(|| {
            BlueyError::authentication("not_configured", "Clerk is not configured")
        })?;
        if !clerk::is_allowed_fapi_url(&url, host) {
            return Err(BlueyError::authentication(
                "url_not_allowed",
                "only the Clerk Frontend API may be proxied",
            ));
        }
        let method = reqwest::Method::from_bytes(method.to_ascii_uppercase().as_bytes())
            .map_err(|_| BlueyError::invalid_params("invalid HTTP method"))?;
        let mut request = self
            .http
            .request(method, &url)
            .timeout(Duration::from_secs(20));
        for (name, value) in &headers {
            if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
                continue;
            }
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                BlueyError::network("timeout", "the Clerk request timed out")
            } else {
                BlueyError::network("request", "the Clerk request failed")
            }
        })?;
        let status = response.status().as_u16();
        let mut out_headers = HashMap::new();
        for name in FORWARDED_RESPONSE_HEADERS {
            if let Some(value) = response.headers().get(*name).and_then(|v| v.to_str().ok()) {
                out_headers.insert((*name).to_string(), value.to_string());
            }
        }
        let body = response
            .text()
            .await
            .map_err(|_| BlueyError::network("request", "the Clerk response could not be read"))?;
        Ok(FapiResponse {
            status,
            headers: out_headers,
            body,
        })
    }
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
