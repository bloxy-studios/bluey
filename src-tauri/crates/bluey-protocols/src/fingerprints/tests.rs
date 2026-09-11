use std::path::PathBuf;

use pretty_assertions::assert_eq;
use serde_json::{json, Value};

use super::rules::Seg;
use super::*;

fn documented(provider: Provider, endpoint: &str) -> Capture {
    let rules = provider.rules();
    provider
        .documented()
        .into_iter()
        .find(|c| {
            rules
                .endpoint_for(&c.request.method, &c.request.path())
                .is_some_and(|e| e.name == endpoint)
        })
        .unwrap_or_else(|| panic!("{} has no documented {endpoint}", provider.id()))
}

fn set_header(capture: &mut Capture, name: &str, value: &str) {
    if let Some(h) = capture.request.headers.iter_mut().find(|h| h.name == name) {
        h.value = value.to_string();
    } else {
        capture.request.headers.push(Header::new(name, value));
    }
}

fn remove_header(capture: &mut Capture, name: &str) {
    capture.request.headers.retain(|h| h.name != name);
}

fn body_mut(capture: &mut Capture) -> &mut Value {
    match &mut capture.request.body {
        Body::Json { value } => value,
        other => panic!("not a JSON body: {}", other.kind_name()),
    }
}

fn run(provider: Provider, expected: &Capture, actual: &Capture) -> DiffReport {
    diff(provider.rules(), expected, actual, "documented", "capture")
}

fn locations(report: &DiffReport, severity: Severity) -> Vec<String> {
    report
        .findings
        .iter()
        .filter(|f| f.severity == severity)
        .map(|f| f.location.clone())
        .collect()
}

#[test]
fn provider_ids_hosts_and_versions() {
    assert_eq!(Provider::parse("codex"), Some(Provider::Chatgpt));
    assert_eq!(Provider::parse("Claude-Code"), Some(Provider::Claude));
    assert_eq!(Provider::parse("google"), Some(Provider::Antigravity));
    assert_eq!(Provider::parse("gemini"), None);
    assert_eq!(
        Provider::for_host("api.anthropic.com"),
        Some(Provider::Claude)
    );
    assert_eq!(
        Provider::for_host("chatgpt.com:443"),
        Some(Provider::Chatgpt)
    );
    assert_eq!(
        Provider::for_host("daily-cloudcode-pa.sandbox.googleapis.com"),
        Some(Provider::Antigravity)
    );
    assert_eq!(Provider::for_host("example.com"), None);
    assert_eq!(Provider::Claude.info().version, "claude_code/2.1.258");
    assert_eq!(Provider::Chatgpt.info().version, "codex/0.154.0");
    assert_eq!(Provider::Antigravity.info().version, "antigravity/2.12.2");
    for p in Provider::ALL {
        assert_eq!(p.info().captured_on, "2026-09-11");
        assert!(p.rules().endpoint_for("POST", "/nope").is_none());
    }
    assert_eq!(
        Provider::Claude
            .rules()
            .endpoint_for("post", "/v1/messages")
            .map(|e| e.name),
        Some("messages")
    );
}

#[test]
fn path_patterns_match_exact_depth_and_report_the_wildcard_index() {
    let path = [
        Seg::Key("system".into()),
        Seg::Index(1),
        Seg::Key("text".into()),
    ];
    let m = match_pattern("system[*].text", &path).expect("matches");
    assert_eq!(m.first_any_index, Some(1));
    assert!(match_pattern("system[1].text", &path).unwrap().specificity > m.specificity);
    assert!(
        match_pattern("system", &path).is_none(),
        "a shorter pattern is not a prefix match"
    );
    assert!(match_pattern("system[*].text.more", &path).is_none());
    let tool = [
        Seg::Key("tools".into()),
        Seg::Index(0),
        Seg::Key("description".into()),
    ];
    assert!(match_pattern("tools[*].*", &tool).is_some());
    assert!(match_pattern("tools[*].name", &tool).is_none());
    assert_eq!(render_path(&path), "system[1].text");
    assert_eq!(render_path(&[]), "$");
    let rules = Provider::Claude.rules();
    let third = [
        Seg::Key("system".into()),
        Seg::Index(2),
        Seg::Key("text".into()),
    ];
    assert!(
        rules.scrub_rule(&third).is_some(),
        "the third system block is caller content"
    );
    assert!(
        rules.scrub_rule(&path).is_none(),
        "the identity block is kept"
    );
}

#[test]
fn scrub_replaces_secrets_ids_and_user_content_with_stable_placeholders() {
    let mut capture = documented(Provider::Claude, "messages");
    capture.source = CaptureSource::Proxy;
    set_header(
        &mut capture,
        "authorization",
        "Bearer sk-ant-oat01-SECRETSECRETSECRETSECRET",
    );
    set_header(
        &mut capture,
        "x-claude-code-session-id",
        "6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b",
    );
    set_header(&mut capture, "cookie", "session=abc");
    let body = body_mut(&mut capture);
    body["system"][0]["text"] =
        json!("x-anthropic-billing-header: cc_version=2.1.268.a1f; cc_entrypoint=cli; cch=1b2c3;");
    body["system"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "type": "text", "text": "Third-party system prompt mentioning jordan@example.com" }));
    body["messages"] = json!([
        { "role": "user", "content": "hello from /Users/jordan/project" },
        { "role": "assistant", "content": [ { "type": "text", "text": "Hi Jordan" }, { "type": "tool_use", "id": "toolu_01ABCDEFGHIJ", "name": "Read", "input": { "path": "/Users/jordan/notes.md" } } ] },
        { "role": "user", "content": [ { "type": "tool_result", "tool_use_id": "toolu_01ABCDEFGHIJ", "content": "file body" }, { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB" } } ] }
    ]);
    body["metadata"]["user_id"] = json!(format!(
        r#"{{"device_id":"{}","account_uuid":"9d1c250a-e61b-44d9-88ed-5944d1962f5e","session_id":"6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b"}}"#,
        "a".repeat(64)
    ));
    body["tools"] = json!([{ "name": "Read", "description": "Reads a file from disk", "input_schema": { "type": "object", "properties": { "path": { "type": "string", "description": "Absolute path" } } } }]);
    capture.response = Some(CapturedResponse {
        status: 200,
        headers: vec![
            Header::new("request-id", "req_011CVabcdefghij"),
            Header::new("set-cookie", "x=y"),
        ],
        body: Body::Sse {
            events: vec![
                SseEvent {
                    event: Some("message_start".into()),
                    data: json!({ "type": "message_start", "message": { "id": "msg_01XYZabcdefgh", "content": [] } }),
                },
                SseEvent {
                    event: Some("content_block_delta".into()),
                    data: json!({ "delta": { "type": "text_delta", "text": "Hello Jordan, your email jordan@example.com" } }),
                },
            ],
        },
        duration_ms: Some(12),
        truncated: false,
    });

    scrub_capture(&mut capture, Provider::Claude.rules());

    assert_eq!(
        capture.request.header("authorization"),
        Some("Bearer <ACCESS_TOKEN>")
    );
    assert_eq!(
        capture.request.header("x-claude-code-session-id"),
        Some("<UUID>")
    );
    assert_eq!(capture.request.header("cookie"), Some("<REDACTED>"));
    let body = body_mut(&mut capture).clone();
    assert_eq!(
        body["system"][0]["text"],
        json!("x-anthropic-billing-header: cc_version=2.1.268.a1f; cc_entrypoint=cli; cch=1b2c3;")
    );
    assert_eq!(body["system"][1]["text"], json!(claude_code::IDENTITY));
    assert_eq!(body["system"][2]["text"], json!("<TEXT 55>"));
    assert_eq!(body["messages"][0]["content"], json!("<TEXT 32>"));
    assert_eq!(body["messages"][1]["content"][0]["text"], json!("<TEXT 9>"));
    assert_eq!(body["messages"][1]["content"][1]["id"], json!("<ID>"));
    assert_eq!(
        body["messages"][1]["content"][1]["input"]["path"],
        json!("<TEXT 22>")
    );
    assert_eq!(
        body["messages"][2]["content"][0]["content"],
        json!("<TEXT 9>")
    );
    assert_eq!(
        body["messages"][2]["content"][1]["source"]["data"],
        json!("<BASE64 24>")
    );
    assert_eq!(
        body["metadata"]["user_id"],
        json!(claude_code::USER_ID_SHAPE)
    );
    assert_eq!(body["tools"][0]["name"], json!("Read"));
    assert_eq!(body["tools"][0]["description"], json!("<TEXT 22>"));
    assert_eq!(
        body["tools"][0]["input_schema"]["properties"]["path"]["description"],
        json!("<TEXT 13>")
    );
    let response = capture.response.as_ref().unwrap();
    assert_eq!(response.header("request-id"), Some("<ID>"));
    assert_eq!(response.header("set-cookie"), Some("<REDACTED>"));
    if let Body::Sse { events } = &response.body {
        assert_eq!(events[0].data["message"]["id"], json!("<ID>"));
        assert_eq!(events[1].data["delta"]["text"], json!("<TEXT 43>"));
    } else {
        panic!("response body kind changed");
    }
    for name in [
        "authorization",
        "credential_header",
        "user_text",
        "model_text",
        "image_data",
        "uuid",
        "opaque_id",
    ] {
        assert!(
            capture.scrubbed.iter().any(|s| s == name),
            "{name} should be listed in {:?}",
            capture.scrubbed
        );
    }
    let serialized = serde_json::to_string(&capture).unwrap();
    for secret in [
        "SECRETSECRET",
        "jordan@example.com",
        "/Users/jordan",
        "toolu_01",
        "msg_01",
        "req_011",
        "6f1d2c3b",
    ] {
        assert!(
            !serialized.contains(secret),
            "{secret} leaked: {serialized}"
        );
    }
    let before = capture.clone();
    scrub_capture(&mut capture, Provider::Claude.rules());
    assert_eq!(capture, before, "scrubbing is idempotent");
}

#[test]
fn scrub_string_patterns() {
    let mut fired = scrub::Fired::default();
    assert_eq!(
        scrub_string("ya29.a0AfH6SMBxyz-123", &mut fired),
        "<ACCESS_TOKEN>"
    );
    assert_eq!(
        scrub_string("1//0gabcdefghijklmnop-qrstuv", &mut fired),
        "<REFRESH_TOKEN>"
    );
    assert_eq!(scrub_string("token eyJhbGciOiJSUzI1NiIsImtpZCI6IjEifQ.eyJzdWIiOiIxMjMifQ.c2lnbmF0dXJlLXNpZ25hdHVyZQ", &mut fired), "token <JWT>");
    assert_eq!(
        scrub_string("Bearer abcdefghijklmnopqrstuvwxyz", &mut fired),
        "Bearer <ACCESS_TOKEN>"
    );
    assert_eq!(
        scrub_string("key AIzaSyD-1234567890abcdefghijklmnopqrstu", &mut fired),
        "key <API_KEY>"
    );
    assert_eq!(
        scrub_string("<TEXT 12> and <ACCESS_TOKEN>", &mut fired),
        "<TEXT 12> and <ACCESS_TOKEN>"
    );
    assert_eq!(
        scrub_string("agent-6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b", &mut fired),
        "agent-<UUID>"
    );
    assert_eq!(
        scrub_string("projects/bluey-owner-4f2a/locations", &mut fired),
        "projects/<PROJECT_ID>/locations"
    );
    assert_eq!(
        scrub_string("plain text stays", &mut fired),
        "plain text stays"
    );
}

#[test]
fn capture_bodies_decode_by_content_type() {
    let sse = Body::from_bytes(
        Some("text/event-stream"),
        b"event: message_start\ndata: {\"type\":\"message_start\"}\n\ndata: [DONE]\n\n",
    );
    match &sse {
        Body::Sse { events } => {
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].event.as_deref(), Some("message_start"));
            assert_eq!(events[0].data["type"], json!("message_start"));
            assert_eq!(events[1].data, json!("[DONE]"));
        }
        other => panic!("{}", other.kind_name()),
    }
    assert!(matches!(
        Body::from_bytes(Some("application/json"), br#"{"a":1}"#),
        Body::Json { .. }
    ));
    assert!(matches!(
        Body::from_bytes(None, br#"[1,2]"#),
        Body::Json { .. }
    ));
    assert!(matches!(
        Body::from_bytes(Some("text/html"), b"<html>"),
        Body::Text { .. }
    ));
    assert!(matches!(
        Body::from_bytes(None, &[0xff, 0xfe]),
        Body::Binary { bytes: 2 }
    ));
    assert!(matches!(Body::from_bytes(None, b""), Body::Empty));
    assert_eq!(
        client_from_user_agent("claude-cli/2.1.268 (external, cli)"),
        Some("claude-cli/2.1.268".into())
    );
    assert_eq!(client_from_user_agent("Mozilla"), None);
}

#[test]
fn every_documented_capture_matches_itself_and_its_rules() {
    for provider in Provider::ALL {
        let captures = provider.documented();
        assert!(!captures.is_empty());
        for capture in &captures {
            assert_eq!(capture.provider, provider.id());
            assert_eq!(capture.fingerprint.version, provider.info().version);
            assert!(
                provider
                    .rules()
                    .endpoint_for(&capture.request.method, &capture.request.path())
                    .is_some(),
                "{} documents an endpoint it does not know: {}",
                provider.id(),
                capture.request.url
            );
            let report = run(provider, capture, capture);
            assert!(
                report.findings.is_empty(),
                "{} {} vs itself: {}",
                provider.id(),
                capture.request.url,
                report.render()
            );
            let mut scrubbed = capture.clone();
            scrub_capture(&mut scrubbed, provider.rules());
            let mut expected = capture.clone();
            expected.scrubbed = scrubbed.scrubbed.clone();
            assert_eq!(
                scrubbed, expected,
                "documented captures contain nothing to scrub"
            );
        }
    }
}

#[test]
fn claude_version_bumps_within_the_documented_format_are_version_findings() {
    let expected = documented(Provider::Claude, "messages");
    let mut actual = expected.clone();
    set_header(
        &mut actual,
        "user-agent",
        "claude-cli/2.1.270 (external, cli)",
    );
    set_header(&mut actual, "x-stainless-package-version", "0.113.0");
    body_mut(&mut actual)["system"][0]["text"] =
        json!("x-anthropic-billing-header: cc_version=2.1.270.9ab; cc_entrypoint=cli; cch=1b2c3;");
    let report = run(Provider::Claude, &expected, &actual);
    assert!(!report.has_drift(), "{}", report.render());
    let mut versions = locations(&report, Severity::Version);
    versions.sort();
    assert_eq!(
        versions,
        vec![
            "body system[0].text",
            "header user-agent",
            "header x-stainless-package-version"
        ]
    );
    let rendered = report.render();
    assert!(rendered.contains("VERSION"));
    assert!(rendered.contains("docs/PROVIDER_ACCOUNTS.md › Claude › Headers"));
}

#[test]
fn claude_drift_cases() {
    let expected = documented(Provider::Claude, "messages");

    let mut actual = expected.clone();
    set_header(
        &mut actual,
        "anthropic-beta",
        "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,thinking-token-count-2026-05-13,context-management-2025-06-27,prompt-caching-scope-2026-01-05,effort-2025-11-24,brand-new-beta-2026-10-01",
    );
    set_header(&mut actual, "x-api-key", "<REDACTED>");
    set_header(&mut actual, "content-length", "4242");
    set_header(&mut actual, "x-stainless-retry-count", "2");
    remove_header(&mut actual, "x-app");
    actual.request.url = "https://api.anthropic.com/v1/messages".into();
    let body = body_mut(&mut actual);
    body["system"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "type": "text", "text": "<TEXT 40>" }));
    body["brand_new_field"] = json!(true);
    body["tools"] = json!([
        { "name": "Read", "description": "<TEXT 9>", "input_schema": { "type": "object" } },
        { "name": "mcp__github__list_issues", "description": "<TEXT 9>", "input_schema": {} },
        { "name": "lowercase_tool", "description": "<TEXT 9>", "input_schema": {} }
    ]);
    body["model"] = json!("claude-opus-5");
    let report = run(Provider::Claude, &expected, &actual);
    let rendered = report.render();
    let mut drift = locations(&report, Severity::Drift);
    drift.sort();
    drift.dedup();
    assert_eq!(
        drift,
        vec![
            "body body",
            "body brand_new_field",
            "body system[2]",
            "body tools[2].name",
            "header anthropic-beta",
            "header x-api-key",
            "header x-app",
            "query beta",
        ]
        .into_iter()
        .map(String::from)
        .filter(|l| l != "body body")
        .collect::<Vec<_>>(),
        "{rendered}"
    );
    let beta_notes: Vec<&str> = report
        .findings
        .iter()
        .filter(|f| f.location == "header anthropic-beta")
        .map(|f| f.note.as_str())
        .collect();
    assert!(
        beta_notes
            .iter()
            .any(|n| n.contains("redact-thinking-2026-02-12")),
        "{rendered}"
    );
    assert!(
        beta_notes
            .iter()
            .any(|n| n.contains("not in the documented set")),
        "{rendered}"
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.location == "header anthropic-beta"
                && f.severity == Severity::Info
                && f.note.contains("effort-2025-11-24")),
        "{rendered}"
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.location.contains("content-length") || f.location.contains("retry-count")),
        "volatile headers are ignored: {rendered}"
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.location == "body tools" && f.severity == Severity::Info),
        "tools present is informational: {rendered}"
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.location == "body model" && f.severity == Severity::Info),
        "{rendered}"
    );
    assert!(rendered.find("DRIFT").unwrap() < rendered.find("INFO").unwrap());
    assert!(rendered.contains("› Claude › Betas — always on / conditional"));
    assert!(rendered.contains("query beta: missing"));
}

#[test]
fn claude_models_endpoint_only_requires_the_oauth_beta() {
    let expected = documented(Provider::Claude, "models");
    let mut actual = expected.clone();
    set_header(
        &mut actual,
        "anthropic-beta",
        "oauth-2025-04-20,claude-code-20250219",
    );
    let report = run(Provider::Claude, &expected, &actual);
    assert!(!report.has_drift(), "{}", report.render());
    let mut actual = expected.clone();
    set_header(&mut actual, "anthropic-beta", "claude-code-20250219");
    let report = run(Provider::Claude, &expected, &actual);
    assert!(report.has_drift(), "{}", report.render());
}

#[test]
fn different_endpoints_stop_after_the_path_finding() {
    let messages = documented(Provider::Claude, "messages");
    let models = documented(Provider::Claude, "models");
    let report = run(Provider::Claude, &messages, &models);
    assert_eq!(report.findings.len(), 2, "{}", report.render());
    assert_eq!(locations(&report, Severity::Drift), vec!["method", "path"]);
}

#[test]
fn codex_rules() {
    let expected = documented(Provider::Chatgpt, "responses");
    let mut actual = expected.clone();
    set_header(&mut actual, "openai-beta", "responses=experimental");
    set_header(
        &mut actual,
        "user-agent",
        "codex_cli_rs/0.160.2 (Mac OS 26.1; arm64) iTerm.app",
    );
    set_header(&mut actual, "version", "0.160.2");
    let body = body_mut(&mut actual);
    body["reasoning"]["effort"] = json!("ultra");
    body["max_output_tokens"] = json!(4096);
    body["tools"] = json!([
        { "type": "function", "name": "shell", "description": "<TEXT 3>", "parameters": {}, "strict": false },
        { "type": "web_search" },
        { "type": "teleport" }
    ]);
    body["text"]["verbosity"] = json!("low");
    let report = run(Provider::Chatgpt, &expected, &actual);
    let rendered = report.render();
    let mut drift = locations(&report, Severity::Drift);
    drift.sort();
    assert_eq!(
        drift,
        vec![
            "body max_output_tokens",
            "body reasoning.effort",
            "body tools[2].type",
            "header openai-beta"
        ],
        "{rendered}"
    );
    let mut versions = locations(&report, Severity::Version);
    versions.sort();
    assert_eq!(
        versions,
        vec!["body text.verbosity", "header user-agent", "header version"],
        "{rendered}"
    );

    let models_expected = documented(Provider::Chatgpt, "models");
    let mut models_actual = models_expected.clone();
    models_actual.request.url =
        "https://chatgpt.com/backend-api/codex/models?client_version=0.160.2".into();
    let report = run(Provider::Chatgpt, &models_expected, &models_actual);
    assert_eq!(
        locations(&report, Severity::Version),
        vec!["query client_version"],
        "{}",
        report.render()
    );
    models_actual.request.url =
        "https://chatgpt.com/backend-api/codex/models?client_version=nightly".into();
    let report = run(Provider::Chatgpt, &models_expected, &models_actual);
    assert_eq!(
        locations(&report, Severity::Drift),
        vec!["query client_version"],
        "{}",
        report.render()
    );
}

#[test]
fn antigravity_rules() {
    let expected = documented(Provider::Antigravity, "stream");
    let mut actual = expected.clone();
    actual.request.url =
        "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:streamGenerateContent?alt=sse".into();
    set_header(
        &mut actual,
        "host",
        "daily-cloudcode-pa.sandbox.googleapis.com",
    );
    set_header(&mut actual, "x-goog-api-client", "gl-node/22.21.1");
    set_header(
        &mut actual,
        "user-agent",
        "Antigravity/2.13.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/132.0.6834.160 Electron/39.2.3",
    );
    let body = body_mut(&mut actual);
    body["request"]["safetySettings"] = json!([]);
    body["request"]["systemInstruction"] =
        json!({ "role": "user", "parts": [ { "text": "You are Antigravity" } ] });
    let report = run(Provider::Antigravity, &expected, &actual);
    let rendered = report.render();
    let mut drift = locations(&report, Severity::Drift);
    drift.sort();
    assert_eq!(
        drift,
        vec![
            "body request.safetySettings",
            "header user-agent",
            "header x-goog-api-client"
        ],
        "{rendered}"
    );
    let mut info = locations(&report, Severity::Info);
    info.sort();
    assert_eq!(
        info,
        vec!["body request.systemInstruction", "header host", "host"],
        "{rendered}"
    );
    assert!(rendered.contains("› Google AI › User-Agent"));

    // onboardUser is the one call that carries X-Goog-Api-Client.
    let mut onboard = expected.clone();
    onboard.request.url = "https://daily-cloudcode-pa.googleapis.com/v1internal:onboardUser".into();
    set_header(&mut onboard, "x-goog-api-client", "gl-node/22.21.1");
    let report = run(Provider::Antigravity, &onboard, &onboard);
    assert!(report.findings.is_empty(), "{}", report.render());
}

#[test]
fn report_serialises_for_the_json_flag() {
    let expected = documented(Provider::Claude, "messages");
    let mut actual = expected.clone();
    remove_header(&mut actual, "x-app");
    let report = run(Provider::Claude, &expected, &actual);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["endpoint"], json!("messages"));
    assert_eq!(json["findings"][0]["severity"], json!("drift"));
    assert_eq!(json["findings"][0]["location"], json!("header x-app"));
    assert_eq!(report.counts(), (1, 0, 0));
}

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/fingerprints")
}

/// The documented captures are checked in as golden fixtures so the TypeScript side
/// (mock fixtures, docs tooling) can read them. Regenerate with
/// `UPDATE_FIXTURES=1 cargo test -p bluey-protocols fingerprints`.
#[test]
fn documented_fixtures_are_current() {
    let update = std::env::var_os("UPDATE_FIXTURES").is_some();
    let mut stale = Vec::new();
    for provider in Provider::ALL {
        let rules = provider.rules();
        for capture in provider.documented() {
            let endpoint = rules
                .endpoint_for(&capture.request.method, &capture.request.path())
                .expect("documented endpoint");
            let path = fixtures_root()
                .join(provider.id())
                .join("documented")
                .join(format!("{}.json", endpoint.name));
            let mut rendered = serde_json::to_string_pretty(&capture).unwrap();
            rendered.push('\n');
            if update {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &rendered).unwrap();
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(on_disk) if on_disk == rendered => {}
                Ok(_) => stale.push(format!("{} differs", path.display())),
                Err(e) => stale.push(format!("{}: {e}", path.display())),
            }
        }
    }
    assert!(
        stale.is_empty(),
        "documented fixtures are stale — run `UPDATE_FIXTURES=1 cargo test -p bluey-protocols fingerprints`:\n{}",
        stale.join("\n")
    );
}
