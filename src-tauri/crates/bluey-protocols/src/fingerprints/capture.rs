//! The capture format: one HTTP exchange of an official client (or of Bluey's own
//! probe), with secrets and user content already scrubbed.
//!
//! Written by the `bluey-fingerprints` harness (`bun run fingerprints:capture`,
//! `fingerprints:import-har`), produced from the documented tables by the provider
//! modules, and compared by [`super::diff`]. Files live under
//! `tests/fixtures/fingerprints/<provider>/` — raw captures in `captures/`
//! (git-ignored), the blessed one in `golden/`, the documented one in `documented/`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sse::SseParser;

/// Bump when the JSON shape changes incompatibly; readers refuse newer schemas.
pub const SCHEMA_VERSION: u32 = 1;

/// Where a capture came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    /// The local capture proxy the official CLI was pointed at.
    Proxy,
    /// Imported from a HAR export of a MITM proxy (Antigravity's Electron app).
    Har,
    /// Built from the tables in `docs/PROVIDER_ACCOUNTS.md` — the doc's columns as data.
    Documented,
    /// Bluey's own `accounts_probe_fingerprint` request.
    Probe,
}

/// The fingerprint a capture documents or was taken against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintStamp {
    /// `claude_code/2.1.258` — the `VERSION` constant of the provider module.
    pub version: String,
    /// `CAPTURED_ON` of that version, `YYYY-MM-DD`.
    pub captured_on: String,
}

/// One scrubbed request/response pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub schema: u32,
    /// Reserved provider id: `chatgpt`, `claude`, `antigravity`.
    pub provider: String,
    pub source: CaptureSource,
    /// RFC 3339, UTC.
    pub captured_at: String,
    pub fingerprint: FingerprintStamp,
    /// `name/version` parsed from the User-Agent (`claude-cli/2.1.268`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    pub request: CapturedRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<CapturedResponse>,
    /// Names of the scrub rules that fired — never the values they replaced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scrubbed: Vec<String>,
    /// Harness notes (`accept-encoding stripped before forwarding`, `response truncated`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedRequest {
    pub method: String,
    /// Absolute upstream URL including the query (`https://api.anthropic.com/v1/messages?beta=true`).
    pub url: String,
    /// Lower-cased names, in wire order.
    pub headers: Vec<Header>,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedResponse {
    pub status: u16,
    pub headers: Vec<Header>,
    pub body: Body,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// The harness stopped recording the body at its cap (the client still got all of it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// A request or response body, decoded far enough to diff field by field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Empty,
    Json {
        value: Value,
    },
    Text {
        text: String,
    },
    /// `text/event-stream`, one entry per dispatched frame; `data` is parsed as JSON when it is JSON.
    Sse {
        events: Vec<SseEvent>,
    },
    /// Not text; only the size is kept.
    Binary {
        bytes: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SseEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    pub data: Value,
}

impl Header {
    /// Lower-cases the name (header names are case-insensitive on the wire).
    pub fn new(name: impl AsRef<str>, value: impl Into<String>) -> Self {
        Self {
            name: name.as_ref().to_ascii_lowercase(),
            value: value.into(),
        }
    }
}

impl CapturedRequest {
    fn parsed(&self) -> Option<url::Url> {
        url::Url::parse(&self.url).ok()
    }

    /// The path without the query (`/v1/messages`).
    pub fn path(&self) -> String {
        self.parsed()
            .map(|u| u.path().to_string())
            .unwrap_or_else(|| self.url.split('?').next().unwrap_or_default().to_string())
    }

    /// `host[:port]` of the upstream.
    pub fn host(&self) -> Option<String> {
        self.parsed().and_then(|u| {
            u.host_str().map(|h| match u.port() {
                Some(p) => format!("{h}:{p}"),
                None => h.to_string(),
            })
        })
    }

    /// Decoded query pairs in URL order.
    pub fn query_pairs(&self) -> Vec<(String, String)> {
        self.parsed()
            .map(|u| {
                u.query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// First value of a header (name compared case-insensitively).
    pub fn header(&self, name: &str) -> Option<&str> {
        header_value(&self.headers, name)
    }
}

impl CapturedResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        header_value(&self.headers, name)
    }
}

fn header_value<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

impl Body {
    /// Decode a body by its `Content-Type`: SSE frames, JSON, UTF-8 text, or size only.
    pub fn from_bytes(content_type: Option<&str>, bytes: &[u8]) -> Body {
        if bytes.is_empty() {
            return Body::Empty;
        }
        let ct = content_type.unwrap_or("").to_ascii_lowercase();
        if ct.contains("text/event-stream") {
            let text = String::from_utf8_lossy(bytes);
            let mut parser = SseParser::new();
            let mut frames = parser.push(&text);
            // A stream cut at the cap may end without the blank line; flush by pushing one.
            frames.extend(parser.push("\n\n"));
            let events = frames
                .into_iter()
                .filter(|f| !f.data.is_empty() || f.event.is_some())
                .map(|f| SseEvent {
                    event: f.event,
                    data: serde_json::from_str(&f.data).unwrap_or(Value::String(f.data)),
                })
                .collect();
            return Body::Sse { events };
        }
        let looks_json = ct.contains("json") || matches!(bytes.first(), Some(b'{') | Some(b'['));
        if looks_json {
            if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
                return Body::Json { value };
            }
        }
        match std::str::from_utf8(bytes) {
            Ok(text) => Body::Text {
                text: text.to_string(),
            },
            Err(_) => Body::Binary { bytes: bytes.len() },
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Body::Empty => "empty",
            Body::Json { .. } => "json",
            Body::Text { .. } => "text",
            Body::Sse { .. } => "sse",
            Body::Binary { .. } => "binary",
        }
    }

    /// The body as one JSON value for field-by-field comparison
    /// (SSE becomes `[{ "event", "data" }, …]`); `None` for text/binary/empty.
    pub fn as_json(&self) -> Option<Value> {
        match self {
            Body::Json { value } => Some(value.clone()),
            Body::Sse { events } => Some(Value::Array(
                events
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "event": e.event,
                            "data": e.data,
                        })
                    })
                    .collect(),
            )),
            _ => None,
        }
    }
}

/// `claude-cli/2.1.268` from `claude-cli/2.1.268 (external, cli)`; `None` when the
/// first token carries no version.
pub fn client_from_user_agent(user_agent: &str) -> Option<String> {
    let first = user_agent.split_whitespace().next()?;
    first.contains('/').then(|| first.to_string())
}
