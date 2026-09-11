//! Fingerprints as data (ADR 0009, `docs/PROVIDER_ACCOUNTS.md`).
//!
//! A subscription provider recognises its official client by a *request
//! fingerprint* — headers, system blocks, wrapper fields — that changes with every
//! client release. This module keeps that fingerprint as data rather than as code
//! spread over adapters:
//!
//! * [`capture`] — the JSON shape of one scrubbed request/response pair, written by the
//!   `bluey-fingerprints` harness (capture proxy, HAR import) and by Bluey's own probe.
//! * [`scrub`] — tokens, ids, e-mails and user content become stable placeholders
//!   before anything touches disk.
//! * [`rules`] — how each header and field is compared (exact, documented pattern,
//!   set, absent, volatile …), each rule naming the doc row it keeps true.
//! * [`diff`] — one capture against another, graded *drift* / *version* / *info*.
//! * [`claude_code`], [`codex`], [`antigravity`] — the three providers: `VERSION` /
//!   `CAPTURED_ON`, the constants PR 3a–3c's shapers reuse, the rules, and the
//!   *documented* capture (the doc's columns as a capture). Golden copies live in
//!   `tests/fixtures/fingerprints/<provider>/documented/`.
//!
//! The runbook that ties them together is `docs/PROVIDER_ACCOUNTS.md › Re-capture runbook`.

pub mod antigravity;
pub mod capture;
pub mod claude_code;
pub mod codex;
pub mod diff;
pub mod rules;
pub mod scrub;

#[cfg(test)]
mod tests;

pub use capture::{
    client_from_user_agent, Body, Capture, CaptureSource, CapturedRequest, CapturedResponse,
    FingerprintStamp, Header, SseEvent, SCHEMA_VERSION,
};
pub use diff::{diff, DiffReport, Finding, Severity};
pub use rules::{
    match_pattern, render_path, Endpoint, FieldRule, HeaderRule, Placeholder, ProviderRules, Rule,
    ScrubRule, Seg,
};
pub use scrub::{scrub_capture, scrub_string};

use crate::request_shaper::FingerprintInfo;

/// The three subscription providers (reserved ids `chatgpt`, `claude`, `antigravity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Chatgpt,
    Claude,
    Antigravity,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Chatgpt, Provider::Claude, Provider::Antigravity];

    pub const fn id(self) -> &'static str {
        match self {
            Provider::Chatgpt => "chatgpt",
            Provider::Claude => "claude",
            Provider::Antigravity => "antigravity",
        }
    }

    /// The heading of the provider's table in `docs/PROVIDER_ACCOUNTS.md`.
    pub const fn display_name(self) -> &'static str {
        match self {
            Provider::Chatgpt => "ChatGPT",
            Provider::Claude => "Claude",
            Provider::Antigravity => "Google AI",
        }
    }

    /// Accepts the reserved id and the names people type (`codex`, `claude-code`, `google`).
    pub fn parse(text: &str) -> Option<Provider> {
        match text.trim().to_ascii_lowercase().as_str() {
            "chatgpt" | "codex" | "openai" | "chatgpt_codex" => Some(Provider::Chatgpt),
            "claude" | "claude-code" | "claude_code" | "anthropic" | "claude_subscription" => {
                Some(Provider::Claude)
            }
            "antigravity" | "google" | "google-ai" | "google_ai" | "antigravity_google" => {
                Some(Provider::Antigravity)
            }
            _ => None,
        }
    }

    pub fn rules(self) -> &'static ProviderRules {
        match self {
            Provider::Chatgpt => &codex::RULES,
            Provider::Claude => &claude_code::RULES,
            Provider::Antigravity => &antigravity::RULES,
        }
    }

    pub fn info(self) -> FingerprintInfo {
        self.rules().info
    }

    /// The doc's tables as captures, one per documented endpoint.
    pub fn documented(self) -> Vec<Capture> {
        match self {
            Provider::Chatgpt => codex::documented(),
            Provider::Claude => claude_code::documented(),
            Provider::Antigravity => antigravity::documented(),
        }
    }

    /// The provider an upstream host belongs to (`api.anthropic.com` → Claude).
    pub fn for_host(host: &str) -> Option<Provider> {
        Provider::ALL
            .into_iter()
            .find(|p| p.rules().is_provider_host(host))
    }
}

/// Build a documented capture (placeholders stand for the values the scrubber replaces).
pub(crate) fn documented_capture(
    rules: &ProviderRules,
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Body,
    response: Option<CapturedResponse>,
    client: &str,
) -> Capture {
    Capture {
        schema: SCHEMA_VERSION,
        provider: rules.provider.id().to_string(),
        source: CaptureSource::Documented,
        captured_at: format!("{}T00:00:00Z", rules.info.captured_on),
        fingerprint: FingerprintStamp {
            version: rules.info.version.to_string(),
            captured_on: rules.info.captured_on.to_string(),
        },
        client: client_from_user_agent(client),
        request: CapturedRequest {
            method: method.to_string(),
            url: url.to_string(),
            headers: headers.iter().map(|(n, v)| Header::new(n, v.clone())).collect(),
            body,
        },
        response,
        scrubbed: Vec::new(),
        notes: vec![
            "built from docs/PROVIDER_ACCOUNTS.md — placeholders stand for the values the scrubber replaces"
                .to_string(),
        ],
    }
}
