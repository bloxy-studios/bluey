//! Scrubbing: secrets, identifiers, e-mails and user content become stable
//! placeholders *before* a capture touches disk. The placeholders are the same
//! ones the documented captures use, so a scrubbed real capture and the doc's
//! columns compare directly (`authorization: Bearer <ACCESS_TOKEN>`).
//!
//! Two layers: provider path rules ([`ProviderRules::scrub`] — which fields hold user
//! text, images or ids) and generic patterns applied to every string everywhere
//! (tokens, JWTs, e-mails, UUIDs, long hex ids, home directories, opaque ids).

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use super::capture::{Body, Capture, Header};
use super::rules::{Placeholder, ProviderRules, Seg};

struct Pattern {
    name: &'static str,
    regex: Regex,
    replacement: &'static str,
}

/// Ordered: provider tokens before the generic bearer/API-key shapes so the placeholder
/// names stay meaningful; identifiers last.
const PATTERNS: &[(&str, &str, &str)] = &[
    (
        "anthropic_access_token",
        r"sk-ant-oat01-[A-Za-z0-9_\-]+",
        "<ACCESS_TOKEN>",
    ),
    (
        "anthropic_refresh_token",
        r"sk-ant-ort01-[A-Za-z0-9_\-]+",
        "<REFRESH_TOKEN>",
    ),
    ("api_key", r"sk-[A-Za-z0-9_\-]{20,}", "<API_KEY>"),
    (
        "google_access_token",
        r"ya29\.[A-Za-z0-9._\-]+",
        "<ACCESS_TOKEN>",
    ),
    (
        "google_refresh_token",
        r"1//[A-Za-z0-9._\-]+",
        "<REFRESH_TOKEN>",
    ),
    ("google_api_key", r"AIza[0-9A-Za-z_\-]{35}", "<API_KEY>"),
    (
        "jwt",
        r"eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
        "<JWT>",
    ),
    (
        "bearer",
        r"(?i)bearer [A-Za-z0-9._~+/=\-]{16,}",
        "Bearer <ACCESS_TOKEN>",
    ),
    (
        "email",
        r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}",
        "<EMAIL>",
    ),
    (
        "uuid",
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        "<UUID>",
    ),
    ("hex64", r"\b[0-9a-f]{64}\b", "<HEX64>"),
    ("hex32", r"\b[0-9a-f]{32}\b", "<HEX32>"),
    ("home_dir", r"/(Users|home)/[^/\s\x22'`]+", "/$1/<USER>"),
    (
        "gcp_project_path",
        r"projects/[a-z][a-z0-9\-]{4,}",
        "projects/<PROJECT_ID>",
    ),
    (
        "opaque_id",
        r"\b(msg|resp|req|rs|fc|ws|item|chatcmpl|thread|conv|toolu|srvtoolu|rsp)_[A-Za-z0-9]{8,}\b",
        "<ID>",
    ),
];

fn patterns() -> &'static [Pattern] {
    static CELL: OnceLock<Vec<Pattern>> = OnceLock::new();
    CELL.get_or_init(|| {
        PATTERNS
            .iter()
            .map(|(name, re, replacement)| Pattern {
                name,
                regex: Regex::new(re).expect("scrub pattern compiles"),
                replacement,
            })
            .collect()
    })
}

/// Response keys whose string values are model or user content.
const RESPONSE_CONTENT_KEYS: &[&str] = &[
    "text",
    "thinking",
    "partial_json",
    "arguments",
    "output_text",
    "transcript",
    "delta",
    "summary",
    "content",
    "refusal",
];
const OPAQUE_KEYS: &[&str] = &["signature", "encrypted_content"];
const PROJECT_KEYS: &[&str] = &[
    "project",
    "cloudaicompanionProject",
    "projectId",
    "project_id",
];

/// Which rules fired, for `Capture::scrubbed`.
#[derive(Default)]
pub struct Fired(BTreeSet<&'static str>);

impl Fired {
    fn hit(&mut self, name: &'static str) {
        self.0.insert(name);
    }

    pub fn names(&self) -> Vec<String> {
        self.0.iter().map(|s| s.to_string()).collect()
    }
}

/// Apply the generic patterns to one string.
pub fn scrub_string(input: &str, fired: &mut Fired) -> String {
    let mut text = input.to_string();
    for p in patterns() {
        if p.regex.is_match(&text) {
            fired.hit(p.name);
            text = p.regex.replace_all(&text, p.replacement).into_owned();
        }
    }
    text
}

/// `<TEXT 12>`, `<ACCESS_TOKEN>`, `<BASE64 40>` — already scrubbed, left alone so
/// scrubbing is idempotent and documented captures survive a scrub unchanged.
fn is_placeholder(s: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^<[A-Z0-9_]+( \d+)?>$").expect("placeholder regex"))
        .is_match(s)
}

fn text_placeholder(s: &str) -> Value {
    if is_placeholder(s) {
        return Value::String(s.to_string());
    }
    Value::String(format!("<TEXT {}>", s.chars().count()))
}

fn base64_placeholder(s: &str) -> Value {
    if is_placeholder(s) {
        return Value::String(s.to_string());
    }
    Value::String(format!("<BASE64 {}>", s.len() * 3 / 4))
}

fn data_url_placeholder(s: &str) -> Option<Value> {
    let rest = s.strip_prefix("data:")?;
    let (mime, payload) = rest.split_once(";base64,")?;
    if is_placeholder(payload) {
        return None;
    }
    Some(Value::String(format!(
        "data:{mime};base64,<BASE64 {}>",
        payload.len() * 3 / 4
    )))
}

fn deep_text(value: &mut Value) {
    match value {
        Value::String(s) => *value = text_placeholder(s),
        Value::Array(items) => items.iter_mut().for_each(deep_text),
        Value::Object(map) => map.values_mut().for_each(deep_text),
        _ => {}
    }
}

/// Returns whether the placeholder applied; string-only placeholders leave containers
/// untouched so the caller keeps descending (a `content` that is an array of blocks).
fn apply_placeholder(value: &mut Value, placeholder: Placeholder, fired: &mut Fired) -> bool {
    match placeholder {
        Placeholder::Text => {
            if let Value::String(s) = value {
                fired.hit("user_text");
                *value = text_placeholder(s);
                return true;
            }
            false
        }
        Placeholder::TextDeep => {
            fired.hit("user_text");
            deep_text(value);
            true
        }
        Placeholder::Base64 => {
            if let Value::String(s) = value {
                fired.hit("image_data");
                *value = base64_placeholder(s);
                return true;
            }
            false
        }
        Placeholder::Opaque(name) => {
            if let Value::String(s) = value {
                if !is_placeholder(s) {
                    fired.hit("opaque");
                    *value = Value::String(format!("<{name} {}>", s.len()));
                }
                return true;
            }
            false
        }
        Placeholder::Fixed(text) => {
            if let Value::String(_) = value {
                fired.hit("identifier");
                *value = Value::String(text.to_string());
                return true;
            }
            false
        }
    }
}

fn scrub_value(
    value: &mut Value,
    path: &mut Vec<Seg>,
    rules: &ProviderRules,
    response: bool,
    fired: &mut Fired,
) {
    if !response {
        if let Some(rule) = rules.scrub_rule(path) {
            if apply_placeholder(value, rule.placeholder, fired) {
                return;
            }
        }
    }
    if let Some(Seg::Key(key)) = path.last() {
        if let Value::String(s) = &*value {
            if is_placeholder(s) {
                return;
            }
            if PROJECT_KEYS.contains(&key.as_str()) {
                fired.hit("identifier");
                *value = Value::String("<PROJECT_ID>".into());
                return;
            }
            if OPAQUE_KEYS.contains(&key.as_str()) {
                fired.hit("opaque");
                *value = Value::String(format!("<OPAQUE {}>", s.len()));
                return;
            }
            if response && RESPONSE_CONTENT_KEYS.contains(&key.as_str()) {
                fired.hit("model_text");
                *value = text_placeholder(s);
                return;
            }
            if response && key == "data" && s.len() > 256 {
                fired.hit("image_data");
                *value = base64_placeholder(s);
                return;
            }
        }
    }
    match value {
        Value::String(s) => {
            if let Some(replacement) = data_url_placeholder(s) {
                fired.hit("image_data");
                *value = replacement;
            } else {
                let scrubbed = scrub_string(s, fired);
                if scrubbed != *s {
                    *value = Value::String(scrubbed);
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                path.push(Seg::Index(i));
                scrub_value(item, path, rules, response, fired);
                path.pop();
            }
        }
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                path.push(Seg::Key(k.clone()));
                scrub_value(v, path, rules, response, fired);
                path.pop();
            }
        }
        _ => {}
    }
}

fn scrub_header(header: &mut Header, fired: &mut Fired) {
    match header.name.as_str() {
        "authorization" | "proxy-authorization" => {
            fired.hit("authorization");
            let scheme = header.value.split_whitespace().next().unwrap_or("");
            header.value = if scheme.eq_ignore_ascii_case("bearer") {
                "Bearer <ACCESS_TOKEN>".to_string()
            } else if scheme.eq_ignore_ascii_case("basic") {
                "Basic <REDACTED>".to_string()
            } else {
                "<REDACTED>".to_string()
            };
            return;
        }
        "x-api-key" | "x-goog-api-key" | "api-key" | "cookie" | "set-cookie" => {
            fired.hit("credential_header");
            header.value = "<REDACTED>".to_string();
            return;
        }
        "chatgpt-account-id" => {
            fired.hit("identifier");
            header.value = "<ACCOUNT_UUID>".to_string();
            return;
        }
        "x-goog-user-project" => {
            fired.hit("identifier");
            header.value = "<PROJECT_ID>".to_string();
            return;
        }
        _ => {}
    }
    header.value = scrub_string(&header.value, fired);
}

fn scrub_url(url: &str, fired: &mut Fired) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return scrub_string(url, fired);
    };
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), scrub_string(&v, fired)))
        .collect();
    if pairs.is_empty() {
        return url.to_string();
    }
    parsed.query_pairs_mut().clear().extend_pairs(pairs);
    parsed.to_string()
}

fn scrub_body(body: &mut Body, rules: &ProviderRules, response: bool, fired: &mut Fired) {
    match body {
        Body::Json { value } => scrub_value(value, &mut Vec::new(), rules, response, fired),
        Body::Sse { events } => {
            for event in events {
                scrub_value(&mut event.data, &mut Vec::new(), rules, true, fired);
            }
        }
        Body::Text { text } => *text = scrub_string(text, fired),
        Body::Empty | Body::Binary { .. } => {}
    }
}

/// Scrub a capture in place and record which rules fired. Idempotent.
pub fn scrub_capture(capture: &mut Capture, rules: &ProviderRules) {
    let mut fired = Fired::default();
    for header in &mut capture.request.headers {
        scrub_header(header, &mut fired);
    }
    capture.request.url = scrub_url(&capture.request.url, &mut fired);
    scrub_body(&mut capture.request.body, rules, false, &mut fired);
    if let Some(response) = &mut capture.response {
        for header in &mut response.headers {
            scrub_header(header, &mut fired);
        }
        scrub_body(&mut response.body, rules, true, &mut fired);
    }
    if let Some(client) = &capture.client {
        capture.client = Some(scrub_string(client, &mut fired));
    }
    let mut names = fired.names();
    names.extend(capture.scrubbed.iter().cloned());
    names.sort();
    names.dedup();
    capture.scrubbed = names;
}
