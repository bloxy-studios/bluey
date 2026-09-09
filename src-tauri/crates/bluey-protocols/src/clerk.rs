//! Clerk: publishable-key → Frontend API host derivation, the (legacy) Frontend
//! API URL allowlist, and the pure half of the browser sign-in flow (OAuth 2.0
//! authorization code + PKCE with Clerk as the OpenID Connect provider).
//!
//! A publishable key is `pk_test_` or `pk_live_` followed by
//! `base64("<fapi-host>$")`, e.g. `pk_test_Y2xlcmsuZXhhbXBsZS5jb20k` →
//! `clerk.example.com`.

use base64::Engine;

/// Derive the Frontend API host from a publishable key. Returns `None` for
/// malformed keys.
pub fn fapi_host_from_publishable_key(publishable_key: &str) -> Option<String> {
    let key = publishable_key.trim();
    let encoded = key
        .strip_prefix("pk_test_")
        .or_else(|| key.strip_prefix("pk_live_"))?;
    let bytes = decode_base64(encoded)?;
    let decoded = String::from_utf8(bytes).ok()?;
    let host = decoded.strip_suffix('$').unwrap_or(&decoded).to_string();
    if host.is_empty() || !is_plausible_host(&host) {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

fn decode_base64(encoded: &str) -> Option<Vec<u8>> {
    let engine = base64::engine::general_purpose::STANDARD;
    if let Ok(bytes) = engine.decode(encoded) {
        return Some(bytes);
    }
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(encoded.trim_end_matches('='))
        .ok()
}

fn is_plausible_host(host: &str) -> bool {
    !host.is_empty()
        && host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Whether `url` is an HTTPS URL on exactly `allowed_host` (no port, no
/// userinfo tricks). This is the only shape `auth_fapi_fetch` will proxy.
pub fn is_allowed_fapi_url(url: &str, allowed_host: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    // Reject userinfo (`user@host`) and ports (`host:443`) outright.
    if authority.contains('@') || authority.contains(':') {
        return false;
    }
    !authority.is_empty() && authority.eq_ignore_ascii_case(allowed_host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn encode_key(prefix: &str, host: &str) -> String {
        let engine = base64::engine::general_purpose::STANDARD;
        format!("{prefix}{}", engine.encode(format!("{host}$")))
    }

    #[test]
    fn derives_hosts_from_test_and_live_keys() {
        let key = encode_key("pk_test_", "adapted-gator-12.clerk.accounts.dev");
        assert_eq!(
            fapi_host_from_publishable_key(&key).as_deref(),
            Some("adapted-gator-12.clerk.accounts.dev")
        );
        let key = encode_key("pk_live_", "clerk.bluey.app");
        assert_eq!(
            fapi_host_from_publishable_key(&key).as_deref(),
            Some("clerk.bluey.app")
        );
    }

    #[test]
    fn tolerates_missing_padding_and_dollar() {
        let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
        let key = format!("pk_test_{}", engine.encode("clerk.example.com$"));
        assert_eq!(
            fapi_host_from_publishable_key(&key).as_deref(),
            Some("clerk.example.com")
        );
        let key = format!("pk_test_{}", engine.encode("clerk.example.com"));
        assert_eq!(
            fapi_host_from_publishable_key(&key).as_deref(),
            Some("clerk.example.com")
        );
    }

    #[test]
    fn rejects_malformed_keys() {
        assert_eq!(fapi_host_from_publishable_key(""), None);
        assert_eq!(fapi_host_from_publishable_key("sk_test_abc"), None);
        assert_eq!(fapi_host_from_publishable_key("pk_test_!!!"), None);
        // decodes but is not a hostname
        let engine = base64::engine::general_purpose::STANDARD;
        let key = format!("pk_test_{}", engine.encode("not a host$"));
        assert_eq!(fapi_host_from_publishable_key(&key), None);
    }

    #[test]
    fn url_allowlist() {
        let host = "clerk.example.com";
        assert!(is_allowed_fapi_url(
            "https://clerk.example.com/v1/client",
            host
        ));
        assert!(is_allowed_fapi_url("https://CLERK.example.com", host));
        assert!(is_allowed_fapi_url(
            "https://clerk.example.com?_is_native=1",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "http://clerk.example.com/v1/client",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "https://evil.com/clerk.example.com",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "https://clerk.example.com.evil.com/x",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "https://clerk.example.com@evil.com/",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "https://clerk.example.com:8443/",
            host
        ));
        assert!(!is_allowed_fapi_url("clerk.example.com/v1", host));
    }
}

// ── OAuth 2.0 / OpenID Connect client (Clerk as the identity provider) ───────
//
// Bluey signs users in through the system browser (ADR 0008): it is a *public*
// OAuth client of the Clerk instance (PKCE, no client secret). Everything here
// is pure — URLs, PKCE, redirect parsing, token/ID-token/userinfo decoding —
// so it can be unit-tested; the app crate does the I/O.

use bluey_core::types::AuthUser;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

/// Scopes requested at sign-in: an ID token (`openid`), the user card
/// (`profile`, `email`) and a refresh token (`offline_access`) so the session
/// survives relaunches without a browser round-trip.
pub const OAUTH_SCOPES: &str = "openid profile email offline_access";
/// Redirect for installed builds — the OS opens Bluey on this URL.
pub const DEEP_LINK_REDIRECT_URI: &str = "bluey://auth/callback";
/// Path of the loopback redirect (`http://127.0.0.1:<port>/callback`), used by
/// development builds, which macOS does not register for deep links.
pub const LOOPBACK_CALLBACK_PATH: &str = "/callback";

/// `https://<frontend api host>` — Clerk's OAuth issuer.
pub fn issuer_from_host(fapi_host: &str) -> String {
    format!("https://{}", fapi_host.trim().trim_end_matches('/'))
}

fn oauth_endpoint(issuer: &str, path: &str) -> String {
    format!("{}/oauth/{path}", issuer.trim().trim_end_matches('/'))
}

pub fn authorize_endpoint(issuer: &str) -> String {
    oauth_endpoint(issuer, "authorize")
}

pub fn token_endpoint(issuer: &str) -> String {
    oauth_endpoint(issuer, "token")
}

pub fn userinfo_endpoint(issuer: &str) -> String {
    oauth_endpoint(issuer, "userinfo")
}

pub fn revoke_endpoint(issuer: &str) -> String {
    oauth_endpoint(issuer, "revoke")
}

/// base64url without padding (RFC 4648 §5), as PKCE and JWTs use it.
pub fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE (RFC 7636): 32 random bytes → a 43-character `code_verifier` and its
/// `S256` `code_challenge`.
pub fn pkce_pair(random: &[u8; 32]) -> (String, String) {
    let verifier = base64url(random);
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// `http://127.0.0.1:<port>/callback`.
pub fn loopback_redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{LOOPBACK_CALLBACK_PATH}")
}

/// Everything the authorization URL needs.
#[derive(Debug, Clone, Copy)]
pub struct AuthorizeRequest<'a> {
    pub issuer: &'a str,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub code_challenge: &'a str,
    pub nonce: &'a str,
}

/// The `/oauth/authorize` URL to open in the browser (authorization code +
/// PKCE `S256`, OIDC `nonce`).
pub fn authorize_url(request: &AuthorizeRequest<'_>) -> Result<String, String> {
    let mut url = Url::parse(&authorize_endpoint(request.issuer)).map_err(|e| e.to_string())?;
    if url.scheme() != "https" {
        return Err("the issuer must be an https URL".to_string());
    }
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", request.client_id)
        .append_pair("redirect_uri", request.redirect_uri)
        .append_pair("scope", OAUTH_SCOPES)
        .append_pair("state", request.state)
        .append_pair("code_challenge", request.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("nonce", request.nonce);
    Ok(url.to_string())
}

/// What a redirect back into Bluey carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackOutcome {
    Code {
        code: String,
        state: String,
    },
    /// The authorization server refused (`error=access_denied`, …) or the
    /// redirect carried no code.
    Denied {
        error: String,
        description: Option<String>,
        state: Option<String>,
    },
}

/// Parse a redirect URL — the `bluey://auth/callback` deep link or the
/// loopback `http://127.0.0.1:<port>/callback`. `None` for anything else, so
/// unrelated deep links are ignored rather than treated as sign-in attempts.
pub fn parse_callback(url: &str) -> Option<CallbackOutcome> {
    let parsed = Url::parse(url.trim()).ok()?;
    let ours = match parsed.scheme() {
        "bluey" => parsed.host_str() == Some("auth") && parsed.path() == "/callback",
        "http" => parsed.host_str() == Some("127.0.0.1") && parsed.path() == LOOPBACK_CALLBACK_PATH,
        _ => false,
    };
    if !ours {
        return None;
    }
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => description = Some(value.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = error.filter(|e| !e.is_empty()) {
        return Some(CallbackOutcome::Denied {
            error,
            description,
            state,
        });
    }
    match (code, state) {
        (Some(code), Some(state)) if !code.is_empty() && !state.is_empty() => {
            Some(CallbackOutcome::Code { code, state })
        }
        (_, state) => Some(CallbackOutcome::Denied {
            error: "invalid_callback".to_string(),
            description: Some("the redirect carried no authorization code".to_string()),
            state,
        }),
    }
}

/// Form body of the authorization-code exchange (public client: no secret,
/// the PKCE verifier proves we started the flow).
pub fn token_exchange_form(
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Vec<(&'static str, String)> {
    vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("client_id", client_id.to_string()),
        ("code_verifier", code_verifier.to_string()),
    ]
}

/// Form body of a refresh-token grant.
pub fn token_refresh_form(client_id: &str, refresh_token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("client_id", client_id.to_string()),
    ]
}

/// Form body of a token revocation (RFC 7009).
pub fn token_revoke_form(client_id: &str, token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("token", token.to_string()),
        ("client_id", client_id.to_string()),
    ]
}

/// A successful `/oauth/token` response.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    /// Seconds until the access token expires.
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

pub fn parse_token_response(json: &str) -> Result<TokenResponse, serde_json::Error> {
    serde_json::from_str(json)
}

/// An OAuth error body (`{"error": "invalid_grant", "error_description": …}`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OAuthErrorBody {
    pub error: String,
    #[serde(default)]
    pub error_description: Option<String>,
}

pub fn parse_oauth_error(json: &str) -> Option<OAuthErrorBody> {
    serde_json::from_str::<OAuthErrorBody>(json)
        .ok()
        .filter(|body| !body.error.is_empty())
}

/// The claims Bluey reads from an ID token.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IdTokenClaims {
    pub iss: String,
    pub sub: String,
    pub aud: Vec<String>,
    /// Unix seconds.
    pub exp: u64,
    pub nonce: Option<String>,
    pub email: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub name: Option<String>,
    pub picture: Option<String>,
}

/// Decode the payload of an ID token (a JWT). The signature is **not**
/// checked here: the token arrives straight from the token endpoint over TLS
/// in exchange for our PKCE verifier, where OIDC Core §3.1.3.7 lets a client
/// rely on TLS; issuer, audience, nonce and expiry are still validated by
/// [`validate_id_token`].
pub fn decode_id_token_claims(id_token: &str) -> Option<IdTokenClaims> {
    let mut parts = id_token.trim().split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(String::from);
    let aud = match value.get("aud") {
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Array(many)) => many
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    };
    Some(IdTokenClaims {
        iss: text("iss")?,
        sub: text("sub")?,
        aud,
        exp: value.get("exp").and_then(Value::as_u64)?,
        nonce: text("nonce"),
        email: text("email"),
        given_name: text("given_name"),
        family_name: text("family_name"),
        name: text("name"),
        picture: text("picture"),
    })
}

/// Clock skew tolerated when checking `exp`.
pub const ID_TOKEN_SKEW_SECS: u64 = 60;

/// Issuer, audience, nonce and expiry checks (OIDC Core §3.1.3.7).
pub fn validate_id_token(
    claims: &IdTokenClaims,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now_unix: u64,
) -> Result<(), String> {
    if claims.iss.trim_end_matches('/') != issuer.trim().trim_end_matches('/') {
        return Err("issuer mismatch".to_string());
    }
    if !claims.aud.iter().any(|aud| aud == client_id) {
        return Err("audience mismatch".to_string());
    }
    if claims.nonce.as_deref() != Some(nonce) {
        return Err("nonce mismatch".to_string());
    }
    if claims.exp.saturating_add(ID_TOKEN_SKEW_SECS) < now_unix {
        return Err("token expired".to_string());
    }
    Ok(())
}

/// `/oauth/userinfo` response (OIDC standard claims).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UserInfo {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: Option<bool>,
    #[serde(default)]
    pub given_name: Option<String>,
    #[serde(default)]
    pub family_name: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub picture: Option<String>,
}

pub fn parse_userinfo(json: &str) -> Result<UserInfo, serde_json::Error> {
    serde_json::from_str(json)
}

fn split_name(name: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return (None, None);
    };
    match name.split_once(' ') {
        Some((first, rest)) => (Some(first.to_string()), Some(rest.trim().to_string())),
        None => (Some(name.to_string()), None),
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The identity Bluey keeps (no tokens), from `userinfo`.
pub fn user_from_userinfo(info: UserInfo) -> AuthUser {
    let (split_first, split_last) = split_name(info.name.as_deref());
    AuthUser {
        id: info.sub,
        email: non_empty(info.email),
        first_name: non_empty(info.given_name).or(split_first),
        last_name: non_empty(info.family_name).or(split_last),
        image_url: non_empty(info.picture),
    }
}

/// The identity from ID-token claims (fallback when `userinfo` is unavailable).
pub fn user_from_claims(claims: &IdTokenClaims) -> AuthUser {
    let (split_first, split_last) = split_name(claims.name.as_deref());
    AuthUser {
        id: claims.sub.clone(),
        email: non_empty(claims.email.clone()),
        first_name: non_empty(claims.given_name.clone()).or(split_first),
        last_name: non_empty(claims.family_name.clone()).or(split_last),
        image_url: non_empty(claims.picture.clone()),
    }
}

/// Clerk's hosted Account Portal for this instance (`/user` = profile &
/// security), derived from the Frontend API host: `clerk.example.com` →
/// `accounts.example.com`, `slug.clerk.accounts.dev` → `slug.accounts.dev`.
pub fn account_portal_url(fapi_host: &str) -> Option<String> {
    let host = fapi_host.trim().trim_end_matches('/').to_ascii_lowercase();
    if let Some(rest) = host.strip_prefix("clerk.") {
        if is_plausible_host(rest) {
            return Some(format!("https://accounts.{rest}/user"));
        }
    }
    if let Some(slug) = host.strip_suffix(".clerk.accounts.dev") {
        if !slug.is_empty() && !slug.contains('.') {
            return Some(format!("https://{slug}.accounts.dev/user"));
        }
    }
    None
}

/// The request-target of an HTTP/1.x `GET` request head
/// (`GET /callback?code=… HTTP/1.1` → `/callback?code=…`).
pub fn http_request_target(head: &str) -> Option<&str> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    target.starts_with('/').then_some(target)
}

/// The page the loopback listener shows after the redirect (no tokens, no
/// scripts, no external resources).
pub fn loopback_html(success: bool) -> String {
    let (title, body) = if success {
        (
            "Signed in to Bluey",
            "You're signed in. You can close this tab and go back to Bluey.",
        )
    } else {
        (
            "Sign-in didn't complete",
            "Bluey could not finish signing you in. Go back to Bluey and try again.",
        )
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title>\
<style>body{{font:15px -apple-system,system-ui,sans-serif;background:#161616;color:#f2f2f2;\
display:flex;align-items:center;justify-content:center;height:100vh;margin:0}}\
main{{text-align:center;max-width:420px;padding:32px}}h1{{font-size:20px;margin:0 0 8px}}\
p{{color:#9a9a9a;margin:0}}</style></head><body><main><h1>{title}</h1><p>{body}</p></main></body></html>"
    )
}

#[cfg(test)]
mod oauth_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pkce_pair_matches_the_rfc_7636_test_vector() {
        // RFC 7636 Appendix B: verifier "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        // has the S256 challenge "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".
        let random: [u8; 32] = [
            116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212,
            37, 77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
        ];
        let (verifier, challenge) = pkce_pair(&random);
        assert_eq!(verifier, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(verifier.len(), 43);
    }

    #[test]
    fn authorize_url_carries_every_parameter() {
        let url = authorize_url(&AuthorizeRequest {
            issuer: "https://clerk.example.com/",
            client_id: "client_123",
            redirect_uri: DEEP_LINK_REDIRECT_URI,
            state: "st ate",
            code_challenge: "chal",
            nonce: "n0nce",
        })
        .unwrap();
        let parsed = Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("clerk.example.com"));
        assert_eq!(parsed.path(), "/oauth/authorize");
        let q: std::collections::HashMap<String, String> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "client_123");
        assert_eq!(q["redirect_uri"], "bluey://auth/callback");
        assert_eq!(q["scope"], OAUTH_SCOPES);
        assert_eq!(q["state"], "st ate");
        assert_eq!(q["code_challenge"], "chal");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["nonce"], "n0nce");
        assert!(url.contains("redirect_uri=bluey%3A%2F%2Fauth%2Fcallback"));
        assert!(authorize_url(&AuthorizeRequest {
            issuer: "http://clerk.example.com",
            client_id: "c",
            redirect_uri: "r",
            state: "s",
            code_challenge: "c",
            nonce: "n",
        })
        .is_err());
    }

    #[test]
    fn endpoints_and_redirects() {
        assert_eq!(
            issuer_from_host("clerk.example.com/"),
            "https://clerk.example.com"
        );
        assert_eq!(
            token_endpoint("https://clerk.example.com"),
            "https://clerk.example.com/oauth/token"
        );
        assert_eq!(
            userinfo_endpoint("https://clerk.example.com/"),
            "https://clerk.example.com/oauth/userinfo"
        );
        assert_eq!(
            revoke_endpoint("https://clerk.example.com"),
            "https://clerk.example.com/oauth/revoke"
        );
        assert_eq!(
            loopback_redirect_uri(54321),
            "http://127.0.0.1:54321/callback"
        );
    }

    #[test]
    fn callbacks_are_recognised_on_both_redirect_styles_only() {
        assert_eq!(
            parse_callback("bluey://auth/callback?code=abc&state=xyz"),
            Some(CallbackOutcome::Code {
                code: "abc".into(),
                state: "xyz".into()
            })
        );
        assert_eq!(
            parse_callback("http://127.0.0.1:51234/callback?state=xyz&code=abc"),
            Some(CallbackOutcome::Code {
                code: "abc".into(),
                state: "xyz".into()
            })
        );
        assert_eq!(
            parse_callback("bluey://auth/callback?error=access_denied&error_description=User%20cancelled&state=xyz"),
            Some(CallbackOutcome::Denied {
                error: "access_denied".into(),
                description: Some("User cancelled".into()),
                state: Some("xyz".into()),
            })
        );
        assert!(matches!(
            parse_callback("bluey://auth/callback?state=xyz"),
            Some(CallbackOutcome::Denied { error, .. }) if error == "invalid_callback"
        ));
        assert_eq!(parse_callback("bluey://open/session/123"), None);
        assert_eq!(
            parse_callback("http://localhost:51234/callback?code=a&state=b"),
            None
        );
        assert_eq!(parse_callback("http://127.0.0.1:51234/favicon.ico"), None);
        assert_eq!(
            parse_callback("https://evil.example/auth/callback?code=a&state=b"),
            None
        );
        assert_eq!(parse_callback("not a url"), None);
    }

    #[test]
    fn token_forms_and_responses() {
        let form = token_exchange_form("cid", "code1", "bluey://auth/callback", "verifier");
        assert_eq!(form[0], ("grant_type", "authorization_code".to_string()));
        assert!(form
            .iter()
            .any(|(k, v)| *k == "code_verifier" && v == "verifier"));
        assert!(!form.iter().any(|(k, _)| *k == "client_secret"));
        let refresh = token_refresh_form("cid", "rt");
        assert_eq!(refresh[0], ("grant_type", "refresh_token".to_string()));
        let revoke = token_revoke_form("cid", "tok");
        assert_eq!(
            revoke,
            vec![
                ("token", "tok".to_string()),
                ("client_id", "cid".to_string())
            ]
        );

        let tokens = parse_token_response(
            r#"{"access_token":"at","token_type":"bearer","expires_in":7200,"refresh_token":"rt","id_token":"a.b.c","scope":"openid profile"}"#,
        )
        .unwrap();
        assert_eq!(tokens.access_token, "at");
        assert_eq!(tokens.expires_in, Some(7200));
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        assert!(parse_token_response(r#"{"token_type":"bearer"}"#).is_err());
        let error =
            parse_oauth_error(r#"{"error":"invalid_grant","error_description":"code used"}"#)
                .unwrap();
        assert_eq!(error.error, "invalid_grant");
        assert!(parse_oauth_error(r#"{"message":"nope"}"#).is_none());
    }

    fn jwt_with(payload: serde_json::Value) -> String {
        let header = base64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let body = base64url(payload.to_string().as_bytes());
        format!("{header}.{body}.signature")
    }

    #[test]
    fn id_token_claims_decode_and_validate() {
        let token = jwt_with(serde_json::json!({
            "iss": "https://clerk.example.com",
            "sub": "user_1",
            "aud": "cid",
            "exp": 1_700_000_000u64,
            "nonce": "n1",
            "email": "a@example.com",
            "given_name": "Ada",
            "family_name": "Lovelace",
            "picture": "https://img.clerk.com/a.png"
        }));
        let claims = decode_id_token_claims(&token).unwrap();
        assert_eq!(claims.sub, "user_1");
        assert_eq!(claims.aud, vec!["cid".to_string()]);
        assert!(validate_id_token(
            &claims,
            "https://clerk.example.com/",
            "cid",
            "n1",
            1_700_000_030
        )
        .is_ok());
        assert_eq!(
            validate_id_token(
                &claims,
                "https://clerk.example.com",
                "other",
                "n1",
                1_700_000_000
            ),
            Err("audience mismatch".to_string())
        );
        assert_eq!(
            validate_id_token(
                &claims,
                "https://clerk.example.com",
                "cid",
                "n2",
                1_700_000_000
            ),
            Err("nonce mismatch".to_string())
        );
        assert_eq!(
            validate_id_token(
                &claims,
                "https://other.example.com",
                "cid",
                "n1",
                1_700_000_000
            ),
            Err("issuer mismatch".to_string())
        );
        assert_eq!(
            validate_id_token(
                &claims,
                "https://clerk.example.com",
                "cid",
                "n1",
                1_700_000_000 + 61
            ),
            Err("token expired".to_string())
        );
        let user = user_from_claims(&claims);
        assert_eq!(user.first_name.as_deref(), Some("Ada"));
        assert_eq!(
            user.image_url.as_deref(),
            Some("https://img.clerk.com/a.png")
        );

        let array_aud =
            jwt_with(serde_json::json!({"iss": "i", "sub": "s", "aud": ["x", "cid"], "exp": 1}));
        assert_eq!(
            decode_id_token_claims(&array_aud).unwrap().aud,
            vec!["x", "cid"]
        );
        assert!(decode_id_token_claims("not.a.jwt.at.all").is_none());
        assert!(decode_id_token_claims("onlytwo.parts").is_none());
        assert!(decode_id_token_claims(&jwt_with(serde_json::json!({"sub": "s"}))).is_none());
    }

    #[test]
    fn userinfo_becomes_an_auth_user() {
        let info = parse_userinfo(
            r#"{"sub":"user_2","email":"b@example.com","email_verified":true,"name":"Grace Brewster Hopper","picture":"https://img.clerk.com/g.png"}"#,
        )
        .unwrap();
        let user = user_from_userinfo(info);
        assert_eq!(user.id, "user_2");
        assert_eq!(user.first_name.as_deref(), Some("Grace"));
        assert_eq!(user.last_name.as_deref(), Some("Brewster Hopper"));
        assert_eq!(user.email.as_deref(), Some("b@example.com"));
        let bare = user_from_userinfo(parse_userinfo(r#"{"sub":"user_3","email":""}"#).unwrap());
        assert_eq!(bare.email, None);
        assert_eq!(bare.first_name, None);
        assert!(parse_userinfo(r#"{"email":"x"}"#).is_err());
    }

    #[test]
    fn account_portal_is_derived_from_the_frontend_api_host() {
        assert_eq!(
            account_portal_url("clerk.bluey.app").as_deref(),
            Some("https://accounts.bluey.app/user")
        );
        assert_eq!(
            account_portal_url("adapted-gator-12.clerk.accounts.dev").as_deref(),
            Some("https://adapted-gator-12.accounts.dev/user")
        );
        assert_eq!(account_portal_url("api.example.com"), None);
        assert_eq!(account_portal_url(""), None);
    }

    #[test]
    fn loopback_request_parsing_and_page() {
        assert_eq!(
            http_request_target("GET /callback?code=a&state=b HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
            Some("/callback?code=a&state=b")
        );
        assert_eq!(http_request_target("POST /callback HTTP/1.1\r\n"), None);
        assert_eq!(http_request_target("GET callback HTTP/1.1\r\n"), None);
        assert_eq!(http_request_target(""), None);
        let ok = loopback_html(true);
        assert!(ok.contains("Signed in to Bluey") && !ok.contains("<script"));
        assert!(loopback_html(false).contains("didn't complete"));
    }
}
