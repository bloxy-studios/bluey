use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    #[default]
    Unknown,
    SignedOut,
    SignedIn,
}

/// Mirrors `AuthUser` — the signed-in identity (from the OIDC `userinfo`
/// endpoint); never carries tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

/// Mirrors `AuthStatus`. Tokens stay in Rust; the WebView only ever sees this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub state: AuthState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<AuthUser>,
    /// Whether OAuth tokens exist in secure storage.
    pub has_stored_session: bool,
    /// Sign-in is configured (Clerk issuer + OAuth client id). Without it the
    /// app runs without an account.
    #[serde(default)]
    pub configured: bool,
    /// A browser sign-in was started and Bluey is waiting for the redirect.
    #[serde(default)]
    pub sign_in_pending: bool,
    pub checked_at: String,
}

/// How the browser hands the authorization code back to Bluey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignInRedirect {
    /// `bluey://auth/callback` (installed builds; the OS opens Bluey).
    DeepLink,
    /// `http://127.0.0.1:<port>/callback` (development builds, which macOS
    /// does not register for deep links).
    Loopback,
}

/// Mirrors `SignInStart` — what `auth_begin_sign_in` returns after opening the
/// system browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInStart {
    /// The authorization URL that was opened (so the UI can offer "copy link").
    pub url: String,
    pub redirect: SignInRedirect,
    /// When the pending flow expires (ISO 8601).
    pub expires_at: String,
}
