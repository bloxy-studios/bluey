//! ChatGPT — the Codex CLI's ChatGPT-auth backend, as data.
//!
//! Every constant and rule below is a row of `docs/PROVIDER_ACCOUNTS.md › ChatGPT`
//! (verified 2026-09-11 against `openai/codex` rust-v0.154.0). PR 3a's `CodexShaper`
//! reads these constants; `fingerprints:diff` compares a capture of the real CLI
//! against [`documented`].

use serde_json::json;

use super::capture::{Body, Capture, CapturedResponse, Header, SseEvent};
use super::rules::{Endpoint, FieldRule, HeaderRule, Placeholder, ProviderRules, Rule, ScrubRule};
use super::{documented_capture, Provider};
use crate::request_shaper::FingerprintInfo;

pub const VERSION: &str = "codex/0.154.0";
pub const CAPTURED_ON: &str = "2026-09-11";
pub const CLIENT_VERSION: &str = "0.154.0";
pub const INFO: FingerprintInfo = FingerprintInfo {
    version: VERSION,
    captured_on: CAPTURED_ON,
};

pub const UPSTREAM: &str = "https://chatgpt.com";
pub const HOSTS: &[&str] = &["chatgpt.com", "chat.openai.com", "chatgpt-staging.com"];
pub const BASE_PATH: &str = "/backend-api/codex";
pub const ORIGINATOR: &str = "codex_cli_rs";
/// `<TERMINAL>` stands for the terminal name the CLI appends (scrubbed captures keep it).
pub const USER_AGENT_EXAMPLE: &str = "codex_cli_rs/0.154.0 (Mac OS 26.0; arm64) <TERMINAL>";
pub const USER_AGENT_PATTERN: &str =
    r"^codex_cli_rs/\d+\.\d+\.\d+ \(Mac OS [^;)]+; (arm64|x86_64)\)( .+)?$";
pub const VERSION_PATTERN: &str = r"^\d+\.\d+\.\d+$";
pub const REASONING_EFFORT_PATTERN: &str = r"^(none|low|medium|high|xhigh)$";
pub const REASONING_SUMMARY_PATTERN: &str = r"^(auto|concise|detailed|none)$";
pub const VERBOSITY_PATTERN: &str = r"^(low|medium|high)$";
pub const TOOL_TYPE_PATTERN: &str =
    r"^(function|local_shell|web_search|custom|freeform|apply_patch)$";
pub const INCLUDE: &[&str] = &["reasoning.encrypted_content"];

const MODELS_HEADERS: &[HeaderRule] = &[
    HeaderRule {
        name: "accept",
        rule: Rule::Info,
        doc: "Catalog",
    },
    HeaderRule {
        name: "content-type",
        rule: Rule::Optional,
        doc: "Catalog",
    },
];

pub static RULES: ProviderRules = ProviderRules {
    provider: Provider::Chatgpt,
    info: INFO,
    client_version: CLIENT_VERSION,
    upstream: UPSTREAM,
    hosts: HOSTS,
    endpoints: &[
        Endpoint {
            name: "responses",
            method: "POST",
            path: "/backend-api/codex/responses",
            headers: &[],
            body: &[],
        },
        Endpoint {
            name: "models",
            method: "GET",
            path: "/backend-api/codex/models",
            headers: MODELS_HEADERS,
            body: &[],
        },
    ],
    request_headers: &[
        HeaderRule {
            name: "authorization",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "chatgpt-account-id",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "originator",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "user-agent",
            rule: Rule::Pattern(USER_AGENT_PATTERN),
            doc: "Headers",
        },
        HeaderRule {
            name: "version",
            rule: Rule::Pattern(VERSION_PATTERN),
            doc: "Headers",
        },
        HeaderRule {
            name: "accept",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "content-type",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "session-id",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "thread-id",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-client-request-id",
            rule: Rule::Exact,
            doc: "Headers",
        },
        HeaderRule {
            name: "openai-beta",
            rule: Rule::Absent,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-openai-fedramp",
            rule: Rule::Optional,
            doc: "Headers",
        },
        HeaderRule {
            name: "x-codex-turn-state",
            rule: Rule::Optional,
            doc: "Headers",
        },
        HeaderRule {
            name: "session_id",
            rule: Rule::Optional,
            doc: "Headers",
        },
        HeaderRule {
            name: "conversation_id",
            rule: Rule::Optional,
            doc: "Headers",
        },
        HeaderRule {
            name: "host",
            rule: Rule::Exact,
            doc: "Backend",
        },
        HeaderRule {
            name: "accept-encoding",
            rule: Rule::Volatile,
            doc: "",
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
            name: "te",
            rule: Rule::Volatile,
            doc: "",
        },
    ],
    query: &[HeaderRule {
        name: "client_version",
        rule: Rule::Pattern(VERSION_PATTERN),
        doc: "Catalog",
    }],
    body: &[
        FieldRule {
            path: "model",
            rule: Rule::Info,
            doc: "Catalog",
        },
        FieldRule {
            path: "instructions",
            rule: Rule::Info,
            doc: "`instructions`",
        },
        FieldRule {
            path: "input",
            rule: Rule::Shape,
            doc: "Body",
        },
        FieldRule {
            path: "tools",
            rule: Rule::Children,
            doc: "Body",
        },
        FieldRule {
            path: "tools[*].type",
            rule: Rule::Pattern(TOOL_TYPE_PATTERN),
            doc: "Body",
        },
        FieldRule {
            path: "tools[*].*",
            rule: Rule::Volatile,
            doc: "Body",
        },
        FieldRule {
            path: "tool_choice",
            rule: Rule::Exact,
            doc: "Body",
        },
        FieldRule {
            path: "parallel_tool_calls",
            rule: Rule::Info,
            doc: "Body",
        },
        FieldRule {
            path: "reasoning.effort",
            rule: Rule::Pattern(REASONING_EFFORT_PATTERN),
            doc: "Body",
        },
        FieldRule {
            path: "reasoning.summary",
            rule: Rule::Pattern(REASONING_SUMMARY_PATTERN),
            doc: "Body",
        },
        FieldRule {
            path: "store",
            rule: Rule::Exact,
            doc: "Body",
        },
        FieldRule {
            path: "stream",
            rule: Rule::Exact,
            doc: "Body",
        },
        FieldRule {
            path: "include",
            rule: Rule::Exact,
            doc: "Body",
        },
        FieldRule {
            path: "prompt_cache_key",
            rule: Rule::Exact,
            doc: "Body",
        },
        FieldRule {
            path: "text.verbosity",
            rule: Rule::Pattern(VERBOSITY_PATTERN),
            doc: "Body",
        },
        FieldRule {
            path: "text.format",
            rule: Rule::Optional,
            doc: "Body",
        },
        FieldRule {
            path: "previous_response_id",
            rule: Rule::Absent,
            doc: "Body",
        },
        FieldRule {
            path: "max_output_tokens",
            rule: Rule::Absent,
            doc: "Body",
        },
        FieldRule {
            path: "temperature",
            rule: Rule::Absent,
            doc: "Body",
        },
        FieldRule {
            path: "top_p",
            rule: Rule::Absent,
            doc: "Body",
        },
        FieldRule {
            path: "service_tier",
            rule: Rule::Optional,
            doc: "Body",
        },
        FieldRule {
            path: "metadata",
            rule: Rule::Optional,
            doc: "Body",
        },
        FieldRule {
            path: "safety_identifier",
            rule: Rule::Optional,
            doc: "Body",
        },
        FieldRule {
            path: "truncation",
            rule: Rule::Optional,
            doc: "Body",
        },
    ],
    response_headers: &[
        HeaderRule {
            name: "x-codex-*",
            rule: Rule::Optional,
            doc: "Rate limits",
        },
        HeaderRule {
            name: "content-type",
            rule: Rule::Info,
            doc: "Backend",
        },
    ],
    response_body: &[],
    scrub: &[
        ScrubRule {
            path: "input[*].content",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "input[*].content[*].text",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "input[*].output",
            placeholder: Placeholder::TextDeep,
            from_index: 0,
        },
        ScrubRule {
            path: "input[*].arguments",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "input[*].summary",
            placeholder: Placeholder::TextDeep,
            from_index: 0,
        },
        ScrubRule {
            path: "input[*].encrypted_content",
            placeholder: Placeholder::Opaque("ENCRYPTED"),
            from_index: 0,
        },
        ScrubRule {
            path: "tools[*].description",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "tools[*].parameters",
            placeholder: Placeholder::TextDeep,
            from_index: 0,
        },
    ],
};

fn common_headers() -> Vec<(&'static str, String)> {
    vec![
        ("authorization", "Bearer <ACCESS_TOKEN>".into()),
        ("chatgpt-account-id", "<ACCOUNT_UUID>".into()),
        ("host", "chatgpt.com".into()),
        ("originator", ORIGINATOR.into()),
        ("session-id", "<UUID>".into()),
        ("thread-id", "<UUID>".into()),
        ("user-agent", USER_AGENT_EXAMPLE.into()),
        ("version", CLIENT_VERSION.into()),
        ("x-client-request-id", "<UUID>".into()),
    ]
}

/// The documented `POST /backend-api/codex/responses` and `GET /backend-api/codex/models`.
pub fn documented() -> Vec<Capture> {
    let mut responses_headers = common_headers();
    responses_headers.push(("accept", "text/event-stream".into()));
    responses_headers.push(("content-type", "application/json".into()));
    let responses = documented_capture(
        &RULES,
        "POST",
        "https://chatgpt.com/backend-api/codex/responses",
        &responses_headers,
        Body::Json {
            value: json!({
                "model": "gpt-6-astra",
                "instructions": "<INSTRUCTIONS_TEMPLATE>",
                "input": [
                    { "type": "message", "role": "user", "content": [ { "type": "input_text", "text": "<TEXT 5>" } ] }
                ],
                "tools": [],
                "tool_choice": "auto",
                "parallel_tool_calls": true,
                "reasoning": { "effort": "medium", "summary": "auto" },
                "store": false,
                "stream": true,
                "include": INCLUDE,
                "prompt_cache_key": "<UUID>",
                "text": { "verbosity": "medium" }
            }),
        },
        Some(CapturedResponse {
            status: 200,
            headers: vec![
                Header::new("content-type", "text/event-stream; charset=utf-8"),
                Header::new("x-codex-primary-used-percent", "12"),
                Header::new("x-codex-primary-window-minutes", "300"),
                Header::new("x-codex-primary-reset-at", "<UNIX_SECONDS>"),
            ],
            body: Body::Sse {
                events: vec![SseEvent {
                    event: None,
                    data: json!({
                        "type": "response.created",
                        "response": { "id": "<ID>", "object": "response", "status": "in_progress", "model": "gpt-6-astra" }
                    }),
                }],
            },
            duration_ms: None,
            truncated: false,
        }),
        USER_AGENT_EXAMPLE,
    );

    let mut models_headers = common_headers();
    models_headers.push(("accept", "application/json".into()));
    let models = documented_capture(
        &RULES,
        "GET",
        &format!("https://chatgpt.com/backend-api/codex/models?client_version={CLIENT_VERSION}"),
        &models_headers,
        Body::Empty,
        Some(CapturedResponse {
            status: 200,
            headers: vec![Header::new("content-type", "application/json")],
            body: Body::Json {
                value: json!({
                    "models": [
                        {
                            "slug": "gpt-6-astra",
                            "display_name": "GPT-6 Astra",
                            "description": "<TEXT 20>",
                            "default_reasoning_level": "medium",
                            "supported_reasoning_levels": [ { "effort": "medium", "description": "<TEXT 20>" } ],
                            "visibility": "list",
                            "supported_in_api": true,
                            "priority": 1,
                            "minimal_client_version": "0.150.0",
                            "context_window": 400000,
                            "input_modalities": ["text", "image"],
                            "model_messages": { "instructions_template": "<INSTRUCTIONS_TEMPLATE>" },
                            "available_in_plans": ["plus", "pro"]
                        }
                    ]
                }),
            },
            duration_ms: None,
            truncated: false,
        }),
        USER_AGENT_EXAMPLE,
    );
    vec![responses, models]
}
