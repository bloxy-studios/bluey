//! Google AI Pro / Ultra — the Antigravity client on Cloud Code `v1internal`, as data.
//!
//! Every constant and rule below is a row of `docs/PROVIDER_ACCOUNTS.md › Google AI`
//! (verified 2026-09-11 against CLIProxyAPI; the User-Agent family and the generation
//! host are *disputed* between the references — a real capture settles them, and
//! this module encodes CLIProxyAPI's answer until it does). PR 3c's
//! `AntigravityShaper` reads these constants.

use serde_json::json;

use super::capture::{Body, Capture, CapturedResponse, Header, SseEvent};
use super::rules::{Endpoint, FieldRule, HeaderRule, Placeholder, ProviderRules, Rule, ScrubRule};
use super::{documented_capture, Provider};
use crate::request_shaper::FingerprintInfo;

pub const VERSION: &str = "antigravity/2.12.2";
pub const CAPTURED_ON: &str = "2026-09-11";
pub const CLIENT_VERSION: &str = "2.12.2";
pub const INFO: FingerprintInfo = FingerprintInfo {
    version: VERSION,
    captured_on: CAPTURED_ON,
};

/// Generation and `onboardUser` (CLIProxyAPI); `loadCodeAssist` goes to [`PROD_UPSTREAM`].
pub const UPSTREAM: &str = "https://daily-cloudcode-pa.googleapis.com";
pub const PROD_UPSTREAM: &str = "https://cloudcode-pa.googleapis.com";
pub const HOSTS: &[&str] = &[
    "daily-cloudcode-pa.googleapis.com",
    "cloudcode-pa.googleapis.com",
    "daily-cloudcode-pa.sandbox.googleapis.com",
    "autopush-cloudcode-pa.sandbox.googleapis.com",
];
pub const USER_AGENT_EXAMPLE: &str = "antigravity/hub/2.12.2 darwin/arm64";
pub const USER_AGENT_PATTERN: &str = r"^antigravity/hub/\d+\.\d+\.\d+ darwin/(arm64|x86_64)$";
pub const IDE_TYPE: &str = "ANTIGRAVITY";
/// Wrapper fields around the Gemini request.
pub const WRAPPER_USER_AGENT: &str = "antigravity";
pub const REQUEST_TYPE: &str = "agent";
pub const REQUEST_ID_SHAPE: &str = "agent-<UUID>";

const LOAD_CODE_ASSIST_HEADERS: &[HeaderRule] = &[HeaderRule {
    name: "accept",
    rule: Rule::Exact,
    doc: "`loadCodeAssist`",
}];

const ONBOARD_USER_HEADERS: &[HeaderRule] = &[HeaderRule {
    name: "x-goog-api-client",
    rule: Rule::Pattern(r"^gl-node/\d+\.\d+\.\d+$"),
    doc: "`onboardUser`",
}];

pub static RULES: ProviderRules = ProviderRules {
    provider: Provider::Antigravity,
    info: INFO,
    client_version: CLIENT_VERSION,
    upstream: UPSTREAM,
    hosts: HOSTS,
    endpoints: &[
        Endpoint {
            name: "stream",
            method: "POST",
            path: "/v1internal:streamGenerateContent",
            headers: &[],
            body: &[],
        },
        Endpoint {
            name: "generate",
            method: "POST",
            path: "/v1internal:generateContent",
            headers: &[],
            body: &[],
        },
        Endpoint {
            name: "count_tokens",
            method: "POST",
            path: "/v1internal:countTokens",
            headers: &[],
            body: &[],
        },
        Endpoint {
            name: "load_code_assist",
            method: "POST",
            path: "/v1internal:loadCodeAssist",
            headers: LOAD_CODE_ASSIST_HEADERS,
            body: &[],
        },
        Endpoint {
            name: "onboard_user",
            method: "POST",
            path: "/v1internal:onboardUser",
            headers: ONBOARD_USER_HEADERS,
            body: &[],
        },
        Endpoint {
            name: "models",
            method: "POST",
            path: "/v1internal:fetchAvailableModels",
            headers: &[],
            body: &[],
        },
        Endpoint {
            name: "quota",
            method: "POST",
            path: "/v1internal:retrieveUserQuota",
            headers: &[],
            body: &[],
        },
        Endpoint {
            name: "quota_summary",
            method: "POST",
            path: "/v1internal:retrieveUserQuotaSummary",
            headers: &[],
            body: &[],
        },
    ],
    request_headers: &[
        HeaderRule {
            name: "authorization",
            rule: Rule::Exact,
            doc: "Headers on generation",
        },
        HeaderRule {
            name: "content-type",
            rule: Rule::Exact,
            doc: "Headers on generation",
        },
        HeaderRule {
            name: "user-agent",
            rule: Rule::Pattern(USER_AGENT_PATTERN),
            doc: "User-Agent",
        },
        HeaderRule {
            name: "accept",
            rule: Rule::Optional,
            doc: "Headers on generation",
        },
        HeaderRule {
            name: "x-goog-api-client",
            rule: Rule::Absent,
            doc: "Headers on generation",
        },
        HeaderRule {
            name: "client-metadata",
            rule: Rule::Absent,
            doc: "Headers on generation",
        },
        HeaderRule {
            name: "x-goog-quotauser",
            rule: Rule::Absent,
            doc: "Headers on generation",
        },
        HeaderRule {
            name: "x-client-device-id",
            rule: Rule::Absent,
            doc: "Headers on generation",
        },
        HeaderRule {
            name: "x-goog-user-project",
            rule: Rule::Absent,
            doc: "Headers on generation",
        },
        HeaderRule {
            name: "host",
            rule: Rule::Info,
            doc: "Host",
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
    ],
    query: &[HeaderRule {
        name: "alt",
        rule: Rule::Exact,
        doc: "Generation",
    }],
    body: &[
        FieldRule {
            path: "model",
            rule: Rule::Info,
            doc: "Current pool",
        },
        FieldRule {
            path: "project",
            rule: Rule::Exact,
            doc: "Generation",
        },
        FieldRule {
            path: "request.contents",
            rule: Rule::Shape,
            doc: "Generation",
        },
        FieldRule {
            path: "request.systemInstruction",
            rule: Rule::Optional,
            doc: "System instruction",
        },
        FieldRule {
            path: "request.generationConfig",
            rule: Rule::Optional,
            doc: "Generation",
        },
        FieldRule {
            path: "request.tools",
            rule: Rule::Optional,
            doc: "Generation",
        },
        FieldRule {
            path: "request.toolConfig",
            rule: Rule::Optional,
            doc: "Generation",
        },
        FieldRule {
            path: "request.safetySettings",
            rule: Rule::Absent,
            doc: "Generation",
        },
        FieldRule {
            path: "request.sessionId",
            rule: Rule::Exact,
            doc: "Generation",
        },
        FieldRule {
            path: "request.cachedContent",
            rule: Rule::Optional,
            doc: "Generation",
        },
        FieldRule {
            path: "request.labels",
            rule: Rule::Optional,
            doc: "Generation",
        },
        FieldRule {
            path: "userAgent",
            rule: Rule::Exact,
            doc: "Generation",
        },
        FieldRule {
            path: "requestType",
            rule: Rule::Exact,
            doc: "Generation",
        },
        FieldRule {
            path: "requestId",
            rule: Rule::Exact,
            doc: "Generation",
        },
        FieldRule {
            path: "metadata.ide_version",
            rule: Rule::Info,
            doc: "`onboardUser`",
        },
        FieldRule {
            path: "tier_id",
            rule: Rule::Info,
            doc: "`onboardUser`",
        },
    ],
    response_headers: &[HeaderRule {
        name: "content-type",
        rule: Rule::Info,
        doc: "Generation",
    }],
    response_body: &[],
    scrub: &[
        ScrubRule {
            path: "request.contents[*].parts[*].text",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "request.contents[*].parts[*].inlineData.data",
            placeholder: Placeholder::Base64,
            from_index: 0,
        },
        ScrubRule {
            path: "request.contents[*].parts[*].functionCall.args",
            placeholder: Placeholder::TextDeep,
            from_index: 0,
        },
        ScrubRule {
            path: "request.contents[*].parts[*].functionResponse.response",
            placeholder: Placeholder::TextDeep,
            from_index: 0,
        },
        ScrubRule {
            path: "request.tools[*].functionDeclarations[*].description",
            placeholder: Placeholder::Text,
            from_index: 0,
        },
        ScrubRule {
            path: "request.tools[*].functionDeclarations[*].parameters",
            placeholder: Placeholder::TextDeep,
            from_index: 0,
        },
        ScrubRule {
            path: "project",
            placeholder: Placeholder::Fixed("<PROJECT_ID>"),
            from_index: 0,
        },
        ScrubRule {
            path: "request.sessionId",
            placeholder: Placeholder::Fixed("<SESSION_ID>"),
            from_index: 0,
        },
    ],
};

fn generation_headers(host: &str) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", "Bearer <ACCESS_TOKEN>".into()),
        ("content-type", "application/json".into()),
        ("host", host.into()),
        ("user-agent", USER_AGENT_EXAMPLE.into()),
    ]
}

/// The documented `streamGenerateContent`, `loadCodeAssist` and `fetchAvailableModels`.
pub fn documented() -> Vec<Capture> {
    let stream = documented_capture(
        &RULES,
        "POST",
        "https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse",
        &generation_headers("daily-cloudcode-pa.googleapis.com"),
        Body::Json {
            value: json!({
                "model": "gemini-3.8-flash-high",
                "project": "<PROJECT_ID>",
                "request": {
                    "contents": [ { "role": "user", "parts": [ { "text": "<TEXT 5>" } ] } ],
                    "generationConfig": { "thinkingConfig": { "includeThoughts": true } },
                    "sessionId": "<SESSION_ID>"
                },
                "userAgent": WRAPPER_USER_AGENT,
                "requestType": REQUEST_TYPE,
                "requestId": REQUEST_ID_SHAPE
            }),
        },
        Some(CapturedResponse {
            status: 200,
            headers: vec![Header::new("content-type", "text/event-stream")],
            body: Body::Sse {
                events: vec![SseEvent {
                    event: None,
                    data: json!({
                        "response": {
                            "candidates": [ { "content": { "role": "model", "parts": [ { "text": "<TEXT 5>" } ] } } ],
                            "modelVersion": "gemini-3.8-flash-high"
                        },
                        "traceId": "<HEX32>"
                    }),
                }],
            },
            duration_ms: None,
            truncated: false,
        }),
        USER_AGENT_EXAMPLE,
    );

    let mut load_headers = generation_headers("cloudcode-pa.googleapis.com");
    load_headers.push(("accept", "*/*".into()));
    let load = documented_capture(
        &RULES,
        "POST",
        "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist",
        &load_headers,
        Body::Json {
            value: json!({ "metadata": { "ideType": IDE_TYPE } }),
        },
        Some(CapturedResponse {
            status: 200,
            headers: vec![Header::new(
                "content-type",
                "application/json; charset=UTF-8",
            )],
            body: Body::Json {
                value: json!({
                    "cloudaicompanionProject": "<PROJECT_ID>",
                    "currentTier": { "id": "<TIER_ID>", "name": "Google AI Pro" },
                    "allowedTiers": [ { "id": "<TIER_ID>", "isDefault": true } ]
                }),
            },
            duration_ms: None,
            truncated: false,
        }),
        USER_AGENT_EXAMPLE,
    );

    let models = documented_capture(
        &RULES,
        "POST",
        "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
        &generation_headers("daily-cloudcode-pa.googleapis.com"),
        Body::Json {
            value: json!({ "project": "<PROJECT_ID>" }),
        },
        Some(CapturedResponse {
            status: 200,
            headers: vec![Header::new(
                "content-type",
                "application/json; charset=UTF-8",
            )],
            body: Body::Json {
                value: json!({
                    "models": {
                        "gemini-3.8-flash-high": {
                            "displayName": "Gemini 3.8 Flash",
                            "maxTokens": 1048576,
                            "maxOutputTokens": 65536,
                            "quotaInfo": { "remainingFraction": 0.9, "resetTime": "<DATE>" }
                        }
                    },
                    "webSearchModelIds": []
                }),
            },
            duration_ms: None,
            truncated: false,
        }),
        USER_AGENT_EXAMPLE,
    );
    vec![stream, load, models]
}
