//! OAuth 2.0 client primitives shared by every provider Bluey signs in with
//! through the system browser — Clerk today (ADR 0008), the subscription
//! accounts next (ADR 0009): PKCE, authorization-URL building, redirect and
//! manual-code parsing, token-endpoint bodies and responses, JWT payload
//! decoding for display claims, and the loopback listener's HTTP bits.
//!
//! Everything here is pure and unit-tested on any host. The runtime half —
//! sockets, timers, single-flight refresh — is the `bluey-oauth` crate; the
//! provider-specific shapes (Clerk's redirect styles and `nonce`, a provider's
//! request fingerprint) stay in their own modules and build on these.

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

/// base64url without padding (RFC 4648 §5), as PKCE and JWTs use it.
pub fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE (RFC 7636): 32 random bytes → a 43-character `code_verifier` and its
/// `S256` `code_challenge`.
pub fn pkce_pair(random: &[u8; 32]) -> (String, String) {
    pkce_pair_from(random)
}

/// PKCE for any verifier entropy. RFC 7636 allows 43–128 verifier characters:
/// 32 random bytes give 43, 64 give 86 (the Codex CLI's choice).
pub fn pkce_pair_from(random: &[u8]) -> (String, String) {
    let verifier = base64url(random);
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Everything an authorization-code + PKCE `S256` URL needs. Provider-specific
/// parameters (OIDC `nonce`, Codex's `originator`, …) go in `extra`, appended
/// after the standard ones in the order given.
#[derive(Debug, Clone, Copy)]
pub struct AuthorizeUrl<'a> {
    /// The full authorization endpoint, e.g. `https://issuer/oauth/authorize`.
    pub endpoint: &'a str,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub scope: &'a str,
    pub state: &'a str,
    pub code_challenge: &'a str,
    pub extra: &'a [(&'a str, &'a str)],
}

/// The URL to open in the browser. Only `https` endpoints are accepted.
pub fn authorize_url(request: &AuthorizeUrl<'_>) -> Result<String, String> {
    let mut url = Url::parse(request.endpoint).map_err(|e| e.to_string())?;
    if url.scheme() != "https" {
        return Err("the authorization endpoint must be an https URL".to_string());
    }
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", request.client_id)
            .append_pair("redirect_uri", request.redirect_uri)
            .append_pair("scope", request.scope)
            .append_pair("state", request.state)
            .append_pair("code_challenge", request.code_challenge)
            .append_pair("code_challenge_method", "S256");
        for (key, value) in request.extra {
            query.append_pair(key, value);
        }
    }
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

/// Read `code` / `state` / `error` / `error_description` from a redirect URL
/// the caller has already recognised as its own.
pub fn callback_outcome(redirect: &Url) -> CallbackOutcome {
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = None;
    for (key, value) in redirect.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => description = Some(value.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = error.filter(|e| !e.is_empty()) {
        return CallbackOutcome::Denied {
            error,
            description,
            state,
        };
    }
    match (code, state) {
        (Some(code), Some(state)) if !code.is_empty() && !state.is_empty() => {
            CallbackOutcome::Code { code, state }
        }
        (_, state) => CallbackOutcome::Denied {
            error: "invalid_callback".to_string(),
            description: Some("the redirect carried no authorization code".to_string()),
            state,
        },
    }
}

/// A code the user pasted back by hand (providers whose redirect page shows
/// it instead of calling a loopback listener).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualCode {
    pub code: String,
    pub state: Option<String>,
}

/// Parse what a user pastes after a browser flow: `code#state` (Claude Code's
/// shape), a bare code, or the whole callback URL. `None` for empty input,
/// whitespace inside the code, or a URL that carries an error instead of a
/// code. The caller still checks `state` against the flow it started.
pub fn parse_manual_code(input: &str) -> Option<ManualCode> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(trimmed) {
        if url.host_str().is_some() {
            return match callback_outcome(&url) {
                CallbackOutcome::Code { code, state } => Some(ManualCode {
                    code,
                    state: Some(state),
                }),
                CallbackOutcome::Denied { .. } => None,
            };
        }
    }
    let (code, state) = match trimmed.split_once('#') {
        Some((code, state)) => (code.trim(), Some(state.trim()).filter(|s| !s.is_empty())),
        None => (trimmed, None),
    };
    if code.is_empty() || code.chars().any(char::is_whitespace) {
        return None;
    }
    Some(ManualCode {
        code: code.to_string(),
        state: state.map(str::to_string),
    })
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

/// A successful token-endpoint response.
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

/// `expires_in` (relative seconds) → `expires_at` (Unix seconds), saturating.
pub fn expires_at(expires_in: Option<u64>, now_unix: u64) -> Option<u64> {
    expires_in.map(|secs| now_unix.saturating_add(secs))
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

/// Decode the payload of a JWT (three dot-separated parts) **without**
/// checking its signature. Only for display claims a client may take on
/// trust because the token arrived straight from the token endpoint over TLS
/// (OIDC Core §3.1.3.7); anything security-relevant must be validated by the
/// caller (see `clerk::validate_id_token`).
pub fn decode_jwt_payload(token: &str) -> Option<Value> {
    let mut parts = token.trim().split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
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

/// The one-shot HTTP/1.1 response the loopback listener writes around
/// [`loopback_html`]: uncacheable, connection closed.
pub fn loopback_http_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pkce_accepts_longer_verifier_entropy() {
        let (short, _) = pkce_pair(&[7u8; 32]);
        let (long, challenge) = pkce_pair_from(&[7u8; 64]);
        assert_eq!(short.len(), 43);
        assert_eq!(long.len(), 86);
        assert_eq!(challenge, base64url(&Sha256::digest(long.as_bytes())));
        assert!(long
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn authorize_url_appends_extras_after_the_standard_parameters() {
        let url = authorize_url(&AuthorizeUrl {
            endpoint: "https://auth.example.com/oauth/authorize",
            client_id: "cid",
            redirect_uri: "http://localhost:1455/auth/callback",
            scope: "openid profile",
            state: "st",
            code_challenge: "ch",
            extra: &[("originator", "codex_cli_rs"), ("prompt", "login")],
        })
        .unwrap();
        let parsed = Url::parse(&url).unwrap();
        let keys: Vec<String> = parsed.query_pairs().map(|(k, _)| k.into_owned()).collect();
        assert_eq!(
            keys,
            vec![
                "response_type",
                "client_id",
                "redirect_uri",
                "scope",
                "state",
                "code_challenge",
                "code_challenge_method",
                "originator",
                "prompt"
            ]
        );
        assert!(url.contains("scope=openid+profile") || url.contains("scope=openid%20profile"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
        let http = authorize_url(&AuthorizeUrl {
            endpoint: "http://auth.example.com/oauth/authorize",
            client_id: "c",
            redirect_uri: "r",
            scope: "s",
            state: "s",
            code_challenge: "c",
            extra: &[],
        });
        assert_eq!(
            http,
            Err("the authorization endpoint must be an https URL".to_string())
        );
        assert!(authorize_url(&AuthorizeUrl {
            endpoint: "not a url",
            client_id: "c",
            redirect_uri: "r",
            scope: "s",
            state: "s",
            code_challenge: "c",
            extra: &[],
        })
        .is_err());
    }

    #[test]
    fn callback_outcomes_from_query_strings() {
        let ok = Url::parse("http://localhost:54545/callback?state=xyz&code=abc&other=1").unwrap();
        assert_eq!(
            callback_outcome(&ok),
            CallbackOutcome::Code {
                code: "abc".into(),
                state: "xyz".into()
            }
        );
        let denied = Url::parse(
            "http://localhost:54545/callback?error=access_denied&error_description=User%20cancelled&state=xyz",
        )
        .unwrap();
        assert_eq!(
            callback_outcome(&denied),
            CallbackOutcome::Denied {
                error: "access_denied".into(),
                description: Some("User cancelled".into()),
                state: Some("xyz".into()),
            }
        );
        let empty = Url::parse("http://localhost:54545/callback?code=&state=xyz").unwrap();
        assert!(matches!(
            callback_outcome(&empty),
            CallbackOutcome::Denied { error, state: Some(s), .. } if error == "invalid_callback" && s == "xyz"
        ));
    }

    #[test]
    fn manual_codes_in_every_pasted_shape() {
        assert_eq!(
            parse_manual_code("  abc123#state-1 \n"),
            Some(ManualCode {
                code: "abc123".into(),
                state: Some("state-1".into())
            })
        );
        assert_eq!(
            parse_manual_code("abc123"),
            Some(ManualCode {
                code: "abc123".into(),
                state: None
            })
        );
        assert_eq!(
            parse_manual_code("abc123#"),
            Some(ManualCode {
                code: "abc123".into(),
                state: None
            })
        );
        assert_eq!(
            parse_manual_code(
                "https://platform.claude.com/oauth/code/callback?code=abc123&state=state-1"
            ),
            Some(ManualCode {
                code: "abc123".into(),
                state: Some("state-1".into())
            })
        );
        assert_eq!(
            parse_manual_code(
                "https://platform.claude.com/oauth/code/callback?error=access_denied"
            ),
            None
        );
        assert_eq!(parse_manual_code("   "), None);
        assert_eq!(parse_manual_code("two words#state"), None);
        assert_eq!(parse_manual_code("#state-only"), None);
    }

    #[test]
    fn jwt_payloads_decode_without_signature_checks() {
        let header = base64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = base64url(
            br#"{"sub":"user_1","https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}"#,
        );
        let token = format!("{header}.{payload}.signature");
        let value = decode_jwt_payload(&token).unwrap();
        assert_eq!(value["sub"], "user_1");
        assert_eq!(
            value["https://api.openai.com/auth"]["chatgpt_plan_type"],
            "plus"
        );
        assert!(decode_jwt_payload("only.two").is_none());
        assert!(decode_jwt_payload("a.b.c.d").is_none());
        assert!(decode_jwt_payload(&format!("{header}.!!!.sig")).is_none());
    }

    #[test]
    fn expiry_and_loopback_response() {
        assert_eq!(expires_at(Some(3600), 1_000), Some(4_600));
        assert_eq!(expires_at(None, 1_000), None);
        assert_eq!(expires_at(Some(u64::MAX), 5), Some(u64::MAX));
        let response = loopback_http_response("<p>hi</p>");
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Content-Length: 9\r\n"));
        assert!(response.contains("Cache-Control: no-store\r\n"));
        assert!(response.ends_with("\r\n\r\n<p>hi</p>"));
    }
}
