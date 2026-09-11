//! `RequestShaper` — a provider's request *fingerprint* as data (ADR 0009 §3.6).
//!
//! Every subscription provider validates that requests look like its official
//! client. The shape — headers, system blocks, body normalisation — is a moving
//! target, so each provider keeps it in one module (`fingerprints::codex`,
//! `fingerprints::claude_code`, `fingerprints::antigravity`, added by PR 3a–3c)
//! that implements this trait: it is applied **last**, after the codec built
//! the request, and it knows which provider responses mean the fingerprint has
//! drifted. Adapters never store credentials between requests; the access
//! token travels in [`ShapeContext`] for the duration of one `shape` call.

use bluey_core::types::UnavailableReason;

/// A provider HTTP request before it is sent. Headers are `(name, value)`
/// pairs in send order; names compare case-insensitively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}

impl ProviderHttpRequest {
    pub fn new(method: &str, url: &str, body: serde_json::Value) -> Self {
        Self {
            method: method.to_string(),
            url: url.to_string(),
            headers: Vec::new(),
            body,
        }
    }

    /// First header value with this name (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Replace every header with this name by one value, or append it.
    pub fn set_header(&mut self, name: &str, value: &str) {
        let mut replaced = false;
        self.headers.retain_mut(|(key, existing)| {
            if key.eq_ignore_ascii_case(name) {
                if replaced {
                    return false;
                }
                *existing = value.to_string();
                replaced = true;
            }
            true
        });
        if !replaced {
            self.headers.push((name.to_string(), value.to_string()));
        }
    }

    /// Remove every header with this name.
    pub fn remove_header(&mut self, name: &str) {
        self.headers
            .retain(|(key, _)| !key.eq_ignore_ascii_case(name));
    }
}

/// Per-request facts a shaper may need. Everything is borrowed for the one
/// call; nothing is retained.
#[derive(Debug, Clone, Copy)]
pub struct ShapeContext<'a> {
    pub account_id: &'a str,
    /// Provider-side account / organisation id (Codex `chatgpt_account_id`,
    /// Claude `account_uuid`), when known.
    pub provider_account_id: Option<&'a str>,
    /// Stable per-install device id (Keychain), for `metadata.user_id`-style fields.
    pub device_id: &'a str,
    /// One id per conversation (session / thread headers, prompt-cache keys).
    pub session_id: &'a str,
    /// One id per request (`x-client-request-id`-style headers).
    pub request_id: &'a str,
    pub model: &'a str,
    pub access_token: Option<&'a str>,
}

/// Why a request could not be shaped (a bug in the caller, not a provider drift).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeError {
    /// The context lacks something the fingerprint needs (`access_token`, …).
    Missing(&'static str),
    /// The body is not the shape the shaper expects.
    InvalidBody(String),
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(what) => write!(f, "request shaping needs {what}"),
            Self::InvalidBody(why) => write!(f, "request body cannot be shaped: {why}"),
        }
    }
}

impl std::error::Error for ShapeError {}

/// Which capture a shaper reproduces: the fingerprint module version and the
/// day it was captured from the official client (`docs/PROVIDER_ACCOUNTS.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FingerprintInfo {
    pub version: &'static str,
    pub captured_on: &'static str,
}

pub trait RequestShaper: Send + Sync {
    fn fingerprint(&self) -> FingerprintInfo;

    /// Make `request` look like the official client's. Applied last.
    fn shape(
        &self,
        request: &mut ProviderHttpRequest,
        ctx: &ShapeContext<'_>,
    ) -> Result<(), ShapeError>;

    /// Whether a provider response says the fingerprint no longer passes (or
    /// the account is blocked). `None` = an ordinary response; the adapter's
    /// error mapper handles 401 / 429 / 5xx as usual.
    fn detect_drift(
        &self,
        status: u16,
        body: &str,
        headers: &[(String, String)],
    ) -> Option<UnavailableReason>;
}

/// The shaper of API-key providers and the mock: leaves requests alone and
/// never reports drift.
#[derive(Debug, Clone, Copy, Default)]
pub struct PassthroughShaper;

impl RequestShaper for PassthroughShaper {
    fn fingerprint(&self) -> FingerprintInfo {
        FingerprintInfo {
            version: "none",
            captured_on: "n/a",
        }
    }

    fn shape(
        &self,
        _request: &mut ProviderHttpRequest,
        _ctx: &ShapeContext<'_>,
    ) -> Result<(), ShapeError> {
        Ok(())
    }

    fn detect_drift(
        &self,
        _status: u16,
        _body: &str,
        _headers: &[(String, String)],
    ) -> Option<UnavailableReason> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn request() -> ProviderHttpRequest {
        let mut request = ProviderHttpRequest::new(
            "POST",
            "https://api.example.com/v1/messages",
            serde_json::json!({ "model": "m", "messages": [] }),
        );
        request
            .headers
            .push(("Content-Type".into(), "application/json".into()));
        request.headers.push(("anthropic-beta".into(), "a".into()));
        request
    }

    #[test]
    fn headers_are_case_insensitive_and_replaced_in_place() {
        let mut request = request();
        assert_eq!(request.header("content-type"), Some("application/json"));
        request.set_header("ANTHROPIC-BETA", "a,b");
        assert_eq!(request.header("anthropic-beta"), Some("a,b"));
        assert_eq!(request.headers.len(), 2, "replaced, not appended");
        request.set_header("x-app", "cli");
        assert_eq!(request.headers.len(), 3);
        request.headers.push(("X-App".into(), "duplicate".into()));
        request.set_header("x-app", "cli2");
        assert_eq!(
            request
                .headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("x-app"))
                .count(),
            1,
            "duplicates collapse to one"
        );
        request.remove_header("Content-Type");
        assert_eq!(request.header("content-type"), None);
    }

    #[test]
    fn the_passthrough_shaper_changes_nothing_and_sees_no_drift() {
        let before = request();
        let mut after = before.clone();
        let ctx = ShapeContext {
            account_id: "chatgpt",
            provider_account_id: None,
            device_id: "device",
            session_id: "session",
            request_id: "request",
            model: "m",
            access_token: Some("token"),
        };
        PassthroughShaper.shape(&mut after, &ctx).unwrap();
        assert_eq!(after, before);
        assert_eq!(PassthroughShaper.fingerprint().version, "none");
        assert_eq!(
            PassthroughShaper.detect_drift(
                400,
                "Third-party apps now draw from your extra usage",
                &[]
            ),
            None
        );
        assert_eq!(
            ShapeError::Missing("access_token").to_string(),
            "request shaping needs access_token"
        );
    }
}
