//! Claude Pro / Max — the Claude Code wire format, as data.
//!
//! Every constant and rule below is a row of `docs/PROVIDER_ACCOUNTS.md › Claude`
//! (verified 2026-09-11 against CLIProxyAPI's alignment to Claude Code 2.1.258).
//! PR 3b's `ClaudeCodeShaper` reads these constants; `fingerprints:diff` compares a
//! capture of the real CLI against [`documented`].

use serde_json::json;

use super::capture::{Body, Capture, CapturedResponse, Header, SseEvent};
use super::rules::{Endpoint, FieldRule, HeaderRule, Placeholder, ProviderRules, Rule, ScrubRule};
use super::{documented_capture, Provider};
use crate::request_shaper::FingerprintInfo;

pub const VERSION: &str = "claude_code/2.1.258";
pub const CAPTURED_ON: &str = "2026-09-11";
pub const CLIENT_VERSION: &str = "2.1.258";
pub const INFO: FingerprintInfo = FingerprintInfo {
    version: VERSION,
    captured_on: CAPTURED_ON,
};

pub const UPSTREAM: &str = "https://api.anthropic.com";
pub const HOSTS: &[&str] = &["api.anthropic.com"];

/// `system[1]` — the entrypoint identity; the billing header is `system[0]`.
pub const IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";
pub const USER_AGENT: &str = "claude-cli/2.1.258 (external, cli)";
pub const USER_AGENT_PATTERN: &str = r"^claude-cli/\d+\.\d+\.\d+ \(external, cli\)$";
pub const BILLING_HEADER_EXAMPLE: &str =
    "x-anthropic-billing-header: cc_version=2.1.258.000; cc_entrypoint=cli; cch=00000;";
pub const BILLING_HEADER_PATTERN: &str = r"^x-anthropic-billing-header: cc_version=\d+\.\d+\.\d+\.[0-9a-f]{3}; cc_entrypoint=cli; cch=[0-9a-f]{5};$";
/// `metadata.user_id` after scrubbing — key order is part of the fingerprint.
pub const USER_ID_SHAPE: &str =
    r#"{"device_id":"<HEX64>","account_uuid":"<UUID>","session_id":"<UUID>"}"#;
pub const TOOL_NAME_PATTERN: &str = r"^(mcp__[A-Za-z0-9_\-]+__[A-Za-z0-9_\-]+|[A-Z][A-Za-z0-9]*)$";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const STAINLESS_PACKAGE_VERSION: &str = "0.112.1";
pub const STAINLESS_RUNTIME_VERSION: &str = "v26.3.0";

/// Wire order, CLI 2.1.258.
pub const BETAS_ALWAYS: &[&str] = &[
    "claude-code-20250219",
    "oauth-2025-04-20",
    "interleaved-thinking-2025-05-14",
    "redact-thinking-2026-02-12",
    "thinking-token-count-2026-05-13",
    "context-management-2025-06-27",
    "prompt-caching-scope-2026-01-05",
];
pub const BETAS_CONDITIONAL: &[&str] = &[
    "mid-conversation-system-2026-04-07",
    "effort-2025-11-24",
    "fallback-credit-2026-06-01",
    "extended-cache-ttl-2025-04-11",
    "structured-outputs-2025-12-15",
    "context-1m-2025-08-07",
];
/// On the catalog and OAuth-API endpoints only `oauth-2025-04-20` is required.
const BETAS_OPTIONAL_ELSEWHERE: &[&str] = &[
    "claude-code-20250219",
    "interleaved-thinking-2025-05-14",
    "redact-thinking-2026-02-12",
    "thinking-token-count-2026-05-13",
    "context-management-2025-06-27",
    "prompt-caching-scope-2026-01-05",
    "mid-conversation-system-2026-04-07",
    "effort-2025-11-24",
    "fallback-credit-2026-06-01",
    "extended-cache-ttl-2025-04-11",
    "structured-outputs-2025-12-15",
    "context-1m-2025-08-07",
];

const OAUTH_BETA_ONLY: HeaderRule = HeaderRule {
    name: "anthropic-beta",
    rule: Rule::Set {
        required: &["oauth-2025-04-20"],
        optional: BETAS_OPTIONAL_ELSEWHERE,
    },
    doc: "Identity / plan",
};

const MODELS_HEADERS: &[HeaderRule] = &[
    HeaderRule {
        name: "anthropic-beta",
        rule: Rule::Set {
            required: &["oauth-2025-04-20"],
            optional: BETAS_OPTIONAL_ELSEWHERE,
        },
        doc: "Catalog",
    },
    HeaderRule {
        name: "content-type",
        rule: Rule::Optional,
        doc: "Catalog",
    },
];

/// `/api/oauth/*` is not the Stainless SDK path; only the OAuth beta is documented.
const OAUTH_API_HEADERS: &[HeaderRule] = &[
    OAUTH_BETA_ONLY,
    HeaderRule {
        name: "user-agent",
        rule: Rule::Info,
        doc: "Identity / plan",
    },
    HeaderRule {
        name: "x-stainless-*",
        rule: Rule::Optional,
        doc: "Identity / plan",
    },
    HeaderRule {
        name: "accept",
        rule: Rule::Optional,
        doc: "Identity / plan",
    },
    HeaderRule {
        name: "anthropic-version",
        rule: Rule::Optional,
        doc: "Identity / plan",
    },
    HeaderRule {
        name: "x-app",
        rule: Rule::Optional,
        doc: "Identity / plan",
    },
    HeaderRule {
        name: "anthropic-dangerous-direct-browser-access",
        rule: Rule::Optional,
        doc: "Identity / plan",
    },
    HeaderRule {
        name: "content-type",
        rule: Rule::Optional,
        doc: "Identity / plan",
    },
];

pub static RULES: ProviderRules = ProviderRules {
    provider: Provider::Claude,
    info: INFO,
    client_version: CLIENT_VERSION,
    upstream: UPSTREAM,
    hosts: HOSTS,
    endpoints: &[
        Endpoint {
            name: "messages",
            method: "POST",
            path: "/v1/messages",
            headers: &[],
            body: &[],
        },
        Endpoint {
            name: "count_tokens",
            method: "POST",
            path: "/v1/messages/count_tokens",
            headers: &[],
            body: &[],
        },
        Endpoint {
            name: "models",
            method: "GET",
            path: "/v1/models",
            headers: MODELS_HEADERS,
            body: &[],
        },
        Endpoint {
            name: "profile",
            method: "GET",
            path: "/api/oauth/profile",
            headers: OAUTH_API_HEADERS,
            body: &[],
        },
        Endpoint {
            name: "usage",
            method: "GET",
            path: "/api/oauth/usage",
            headers: OAUTH_API_HEADERS,
            body: &[],
        },
        Endpoint {
            name: "roles",
            method: "GET",
            path: "/api/oauth/claude_cli/roles",
            headers: OAUTH_API_HEADERS,
            body: &[],
        },
    ],
    request_headers: &[
        HeaderRule {
            name: "user-agent",
            rule: Rule::Pattern(USER_AGENT_PATTERN),
            doc: "Headers",
        },
        HeaderRule {
            name: "x-app",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "anthropic-dangerous-direct-browser-access",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "anthropic-version",
            rule: Rule::Exact,
            doc: "URL / version",
        },
        HeaderRule {
            name: "anthropic-beta",
            rule: Rule::Set {
                required: BETAS_ALWAYS,
                optional: BETAS_CONDITIONAL,
            },
            doc: "Betas — always on / conditional",
        },
        HeaderRule {
            name: "authorization",
            rule: Rule::Exact,
            doc: "Transport",
        },
        HeaderRule {
            name: "x-api-key",
            rule: Rule::Absent,
            doc: "Transport",
        },
        HeaderRule {
            name: "x-claude-code-session-id",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-client-request-id",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-stainless-lang",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-stainless-package-version",
            rule: Rule::Pattern(r"^\d+\.\d+\.\d+$"),
            doc: "Headers",
        },
        HeaderRule {
            name: "x-stainless-os",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-stainless-arch",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-stainless-runtime",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-stainless-runtime-version",
            rule: Rule::Pattern(r"^v\d+\.\d+\.\d+$"),
            doc: "Headers",
        },
        HeaderRule {
            name: "x-stainless-retry-count",
            rule: Rule::Volatile,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-stainless-timeout",
            rule: Rule::Info,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-stainless-*",
            rule: Rule::Info,
            doc: "Headers",
        },
        HeaderRule {
            name: "accept",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "accept-encoding",
            rule: Rule::Info,
            doc: "Headers",
        },
        HeaderRule {
            name: "content-type",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "host",
            rule: Rule::Exact,
            doc: "URL / version",
        },
        HeaderRule {
            name: "content-length",
            rule: Rule::Volatile,
            doc: "",
        },
        HeaderRule {
            name: "connection",
            rule: Rule::Volatile,
            doc: "",
        },
        HeaderRule {
            name: "sec-fetch-mode",
            rule: Rule::Volatile,
            doc: "",
        },
        HeaderRule {
            name: "accept-language",
            rule: Rule::Volatile,
            doc: "",
        },
    ],
    query: &[
        HeaderRule {
            name: "beta",
            rule: Rule::Exact,
            doc: "URL / version",
        },
        HeaderRule {
            name: "limit",
            rule: Rule::Info,
            doc: "Catalog",
        },
    ],
    body: &[
        FieldRule {
            path: "model",
            rule: Rule::Info,
            doc: "Catalog",
        },
        FieldRule {
            path: "max_tokens",
            rule: Rule::Info,
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "stream",
            rule: Rule::Exact,
            doc: "URL / version",
        },
        FieldRule {
            path: "system[0].text",
            rule: Rule::Pattern(BILLING_HEADER_PATTERN),
            doc: "`system[]` — exactly two blocks",
        },
        FieldRule {
            path: "system[1].text",
            rule: Rule::Exact,
            doc: "`system[]` — exactly two blocks",
        },
        FieldRule {
            path: "messages",
            rule: Rule::Shape,
            doc: "`system[]` — exactly two blocks",
        },
        FieldRule {
            path: "metadata.user_id",
            rule: Rule::Exact,
            doc: "`metadata.user_id`",
        },
        FieldRule {
            path: "tools",
            rule: Rule::Children,
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "tools[*].name",
            rule: Rule::Pattern(TOOL_NAME_PATTERN),
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "tools[*].*",
            rule: Rule::Volatile,
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "tool_choice",
            rule: Rule::Optional,
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "thinking",
            rule: Rule::Optional,
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "output_config",
            rule: Rule::Optional,
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "context_management",
            rule: Rule::Optional,
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "fallbacks",
            rule: Rule::Optional,
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "temperature",
            rule: Rule::Optional,
            doc: "Tools / thinking",
        },
        FieldRule {
            path: "top_p",
            rule: Rule::Optional,
            doc: "",
        },
        FieldRule {
            path: "top_k",
            rule: Rule::Optional,
            doc: "",
        },
        FieldRule {
            path: "stop_sequences",
            rule: Rule::Optional,
            doc: "",
        },
    ],
    response_headers: &[
        HeaderRule {
            name: "request-id",
            rule: Rule::Present,
            doc: "Other errors",
        },
        HeaderRule {
            name: "anthropic-ratelimit-unified-status",
            rule: Rule::Present,
            doc: "Rate-limit headers (subscription)",
        },
        HeaderRule {
            name: "anthropic-ratelimit-unified-5h-status",
            rule: Rule::Present,
            doc: "Rate-limit headers (subscription)",
        },
        HeaderRule {
            name: "anthropic-ratelimit-unified-7d-status",
            rule: Rule::Present,
            doc: "Rate-limit headers (subscription)",
        },
        HeaderRule {
            name: "anthropic-ratelimit-unified-5h-reset",
            rule: Rule::Present,
            doc: "Rate-limit headers (subscription)",
        },
        HeaderRule {
            name: "anthropic-ratelimit-unified-7d-reset",
            rule: Rule::Present,
            doc: "Rate-limit headers (subscription)",
        },
        HeaderRule {
            name: "anthropic-ratelimit-unified-5h-utilization",
            rule: Rule::Present,
            doc: "Rate-limit headers (subscription)",
        },
        HeaderRule {
            name: "anthropic-ratelimit-unified-7d-utilization",
            rule: Rule::Present,
            doc: "Rate-limit headers (subscription)",
        },
        HeaderRule {
            name: "anthropic-ratelimit-unified-representative-claim",
            rule: Rule::Present,
            doc: "Rate-limit headers (subscription)",
        },
        HeaderRule {
            name: "anthropic-ratelimit-unified-*",
            rule: Rule::Optional,
            doc: "Rate-limit headers (subscription)",
        },
        HeaderRule {
            name: "content-type",
            rule: Rule::Info,
            doc: "URL / version",
        },
    ],
    response_body: &[],
    scrub: &[
        ScrubRule {
            path: "messages[*].content",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "messages[*].content[*].text",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "messages[*].content[*].thinking",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "messages[*].content[*].source.data",
            placeholder: Placeholder::Base64,
            from_index: 0,
        },
        ScrubRule {
            path: "messages[*].content[*].content",
            placeholder: Placeholder::TextDeep,
            from_index: 0,
        },
        ScrubRule {
            path: "messages[*].content[*].input",
            placeholder: Placeholder::TextDeep,
            from_index: 0,
        },
        ScrubRule {
            path: "system[*].text",
            placeholder: Placeholder::Text,
            from_index: 2,
        },
        ScrubRule {
            path: "tools[*].description",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "tools[*].input_schema",
            placeholder: Placeholder::TextDeep,
            from_index: 0,
        },
    ],
};

fn common_headers() -> Vec<(&'static str, String)> {
    vec![
        ("accept", "application/json".into()),
        ("accept-encoding", "gzip, deflate, br, zstd".into()),
        ("anthropic-dangerous-direct-browser-access", "true".into()),
        ("anthropic-version", ANTHROPIC_VERSION.into()),
        ("authorization", "Bearer <ACCESS_TOKEN>".into()),
        ("host", "api.anthropic.com".into()),
        ("user-agent", USER_AGENT.into()),
        ("x-app", "cli".into()),
        ("x-claude-code-session-id", "<UUID>".into()),
        ("x-client-request-id", "<UUID>".into()),
        ("x-stainless-arch", "arm64".into()),
        ("x-stainless-lang", "js".into()),
        ("x-stainless-os", "MacOS".into()),
        (
            "x-stainless-package-version",
            STAINLESS_PACKAGE_VERSION.into(),
        ),
        ("x-stainless-retry-count", "0".into()),
        ("x-stainless-runtime", "node".into()),
        (
            "x-stainless-runtime-version",
            STAINLESS_RUNTIME_VERSION.into(),
        ),
        ("x-stainless-timeout", "600".into()),
    ]
}

fn rate_limit_headers() -> Vec<Header> {
    [
        ("content-type", "text/event-stream; charset=utf-8"),
        ("request-id", "<ID>"),
        ("anthropic-ratelimit-unified-status", "allowed"),
        ("anthropic-ratelimit-unified-5h-status", "allowed"),
        ("anthropic-ratelimit-unified-7d-status", "allowed"),
        ("anthropic-ratelimit-unified-5h-reset", "<UNIX_SECONDS>"),
        ("anthropic-ratelimit-unified-7d-reset", "<UNIX_SECONDS>"),
        ("anthropic-ratelimit-unified-5h-utilization", "0.12"),
        ("anthropic-ratelimit-unified-7d-utilization", "0.34"),
        (
            "anthropic-ratelimit-unified-representative-claim",
            "five_hour",
        ),
    ]
    .into_iter()
    .map(|(n, v)| Header::new(n, v))
    .collect()
}

/// The documented `POST /v1/messages?beta=true` and `GET /v1/models?limit=100`.
pub fn documented() -> Vec<Capture> {
    let mut messages_headers = common_headers();
    messages_headers.push(("anthropic-beta", BETAS_ALWAYS.join(",")));
    messages_headers.push(("content-type", "application/json".into()));
    let messages = documented_capture(
        &RULES,
        "POST",
        "https://api.anthropic.com/v1/messages?beta=true",
        &messages_headers,
        Body::Json {
            value: json!({
                "model": "claude-sonnet-5",
                "max_tokens": 128000,
                "stream": true,
                "system": [
                    { "type": "text", "text": BILLING_HEADER_EXAMPLE },
                    { "type": "text", "text": IDENTITY, "cache_control": { "type": "ephemeral", "ttl": "1h" } }
                ],
                "messages": [
                    { "role": "user", "content": [ { "type": "text", "text": "<TEXT 5>" } ] }
                ],
                "metadata": { "user_id": USER_ID_SHAPE },
                "thinking": { "type": "adaptive", "display": "summarized" },
                "output_config": { "effort": "medium" },
                "context_management": { "edits": [ { "type": "clear_thinking_20251015", "keep": "all" } ] }
            }),
        },
        Some(CapturedResponse {
            status: 200,
            headers: rate_limit_headers(),
            body: Body::Sse {
                events: vec![SseEvent {
                    event: Some("message_start".into()),
                    data: json!({
                        "type": "message_start",
                        "message": {
                            "id": "<ID>", "type": "message", "role": "assistant", "model": "claude-sonnet-5",
                            "content": [], "stop_reason": null, "stop_sequence": null,
                            "usage": { "input_tokens": 0, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0, "output_tokens": 1 }
                        }
                    }),
                }],
            },
            duration_ms: None,
            truncated: false,
        }),
        USER_AGENT,
    );

    let mut models_headers = common_headers();
    models_headers.push(("anthropic-beta", "oauth-2025-04-20".into()));
    let models = documented_capture(
        &RULES,
        "GET",
        "https://api.anthropic.com/v1/models?limit=100",
        &models_headers,
        Body::Empty,
        Some(CapturedResponse {
            status: 200,
            headers: vec![
                Header::new("content-type", "application/json"),
                Header::new("request-id", "<ID>"),
            ],
            body: Body::Json {
                value: json!({
                    "data": [
                        { "type": "model", "id": "claude-sonnet-5", "display_name": "Claude Sonnet 5", "created_at": "<DATE>" }
                    ],
                    "has_more": false,
                    "first_id": "claude-sonnet-5",
                    "last_id": "claude-sonnet-5"
                }),
            },
            duration_ms: None,
            truncated: false,
        }),
        USER_AGENT,
    );
    vec![messages, models]
}
