//! Azure OpenAI / Microsoft Foundry URL building.
//!
//! The v1 GA surface (`{base}/openai/v1/...`) is the default and needs no
//! `api-version`. A provider's `api_version` is interpreted as:
//!
//! * `None` / empty / `"v1"` → v1 GA (`{base}/openai/v1/{path}`);
//! * `"preview"` → v1 with preview features opted in
//!   (`{base}/openai/v1/{path}?api-version=preview`);
//! * anything else (a dated value such as `2024-10-21`) → the legacy
//!   deployment-scoped form (`{base}/openai/deployments/{d}/{path}?api-version=…`).
//!
//! OpenAI-style realtime STT uses `{base}/openai/v1/realtime`. Microsoft
//! MAI-Transcribe live STT uses the Voice Live host
//! `{resource}.services.ai.azure.com/voice-live/realtime` (see [`voice_live_url`]).

use std::collections::BTreeMap;

/// How a configured `api_version` maps onto the Azure OpenAI URL surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiSurface {
    /// `{base}/openai/v1/{path}` — GA, no `api-version`.
    V1,
    /// `{base}/openai/v1/{path}?api-version=preview` — GA path, preview features.
    V1Preview,
    /// `{base}/openai/deployments/{d}/{path}?api-version={dated}`.
    LegacyDated,
}

/// Classify a provider's `api_version` setting.
pub fn api_surface(api_version: Option<&str>) -> ApiSurface {
    match api_version
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("") | Some("v1") => ApiSurface::V1,
        Some("preview") => ApiSurface::V1Preview,
        Some(_) => ApiSurface::LegacyDated,
    }
}

/// Map a model id onto its Azure deployment name via the provider's
/// `deployments` map; falls back to the model id itself.
pub fn resolve_deployment<'a>(
    model: &'a str,
    deployments: Option<&'a BTreeMap<String, String>>,
) -> &'a str {
    deployments
        .and_then(|map| map.get(model))
        .map(String::as_str)
        .unwrap_or(model)
}

/// Chat-completions URL (see the module docs for how `api_version` is read).
pub fn chat_url(base_url: &str, api_version: Option<&str>, deployment: &str) -> String {
    build(base_url, api_version, deployment, "chat/completions")
}

/// Embeddings URL (see the module docs for how `api_version` is read).
pub fn embeddings_url(base_url: &str, api_version: Option<&str>, deployment: &str) -> String {
    build(base_url, api_version, deployment, "embeddings")
}

fn build(base_url: &str, api_version: Option<&str>, deployment: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match api_surface(api_version) {
        ApiSurface::V1 => format!("{base}/openai/v1/{path}"),
        ApiSurface::V1Preview => format!("{base}/openai/v1/{path}?api-version=preview"),
        ApiSurface::LegacyDated => {
            let version = api_version.unwrap_or_default().trim();
            format!("{base}/openai/deployments/{deployment}/{path}?api-version={version}")
        }
    }
}

/// Realtime transcription WebSocket URL for an Azure OpenAI resource:
/// `wss://{resource}/openai/v1/realtime?intent=transcription`.
///
/// This surface serves OpenAI STT deployments (`gpt-4o-mini-transcribe`, …),
/// not MAI-Transcribe — use [`voice_live_url`] for that.
pub fn realtime_url(base_url: &str) -> String {
    format!(
        "{}/openai/v1/realtime?intent=transcription",
        to_wss(base_url)
    )
}

/// Current Voice Live API version. `2026-04-10` is the GA how-to default;
/// `2026-06-01-preview` is the newest reference that documents `mai-transcribe`.
pub const VOICE_LIVE_API_VERSION: &str = "2026-06-01-preview";

/// Voice Live WebSocket for live MAI-Transcribe (and azure-speech) STT:
/// `wss://{resource}.services.ai.azure.com/voice-live/realtime?api-version=…&model={companion}`.
///
/// `companion_model` is a Voice Live *chat* model (fully managed — it does not
/// have to be deployed in the resource). It is **not** the STT model; STT is
/// set in `session.update` as `input_audio_transcription.model`. Pick the
/// cheapest listed companion (`gpt-4.1-mini` / `gpt-5-nano`) and set
/// `create_response: false` so it never generates replies.
///
/// An `*.openai.azure.com` endpoint is rewritten to `*.services.ai.azure.com`
/// (same resource name). Speech-only `*.cognitiveservices.azure.com` hosts are
/// left alone.
pub fn voice_live_url(base_url: &str, companion_model: &str) -> String {
    voice_live_url_with_version(base_url, companion_model, VOICE_LIVE_API_VERSION)
}

/// [`voice_live_url`] with an explicit `api-version`.
pub fn voice_live_url_with_version(
    base_url: &str,
    companion_model: &str,
    api_version: &str,
) -> String {
    let wss = to_wss(&voice_live_host(base_url));
    let model = urlencoding_unreserved(companion_model.trim());
    format!("{wss}/voice-live/realtime?api-version={api_version}&model={model}")
}

/// Custom-subdomain resource name (`my-res`) from a Foundry / Azure OpenAI /
/// Cognitive Services endpoint. `None` when the host is not one of those.
pub fn foundry_resource_name(endpoint: &str) -> Option<&str> {
    let host = host_of(endpoint)?;
    for suffix in [
        ".openai.azure.com",
        ".services.ai.azure.com",
        ".cognitiveservices.azure.com",
    ] {
        if let Some(name) = host.strip_suffix(suffix) {
            if !name.is_empty() && !name.contains('.') {
                return Some(name);
            }
        }
    }
    None
}

fn voice_live_host(base_url: &str) -> String {
    let host = host_of(base_url).unwrap_or_else(|| base_url.trim_end_matches('/'));
    if let Some(name) = host.strip_suffix(".openai.azure.com") {
        return format!("{name}.services.ai.azure.com");
    }
    host.to_string()
}

fn host_of(endpoint: &str) -> Option<&str> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("wss://"))
        .or_else(|| trimmed.strip_prefix("ws://"))
        .unwrap_or(trimmed);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .filter(|h| !h.is_empty())
}

fn to_wss(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if base.starts_with("wss://") || base.starts_with("ws://") {
        base.to_string()
    } else {
        format!("wss://{base}")
    }
}

/// Model ids are `[A-Za-z0-9._-]` — pass through; anything else is percent-encoded.
fn urlencoding_unreserved(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn v1_urls() {
        assert_eq!(
            chat_url("https://res.openai.azure.com", None, "gpt-6-astra"),
            "https://res.openai.azure.com/openai/v1/chat/completions"
        );
        assert_eq!(
            embeddings_url("https://res.openai.azure.com/", None, "embed"),
            "https://res.openai.azure.com/openai/v1/embeddings"
        );
        // "v1" and blank are the GA surface too, not the legacy dated form.
        assert_eq!(
            chat_url("https://res.openai.azure.com", Some("v1"), "gpt-6-astra"),
            "https://res.openai.azure.com/openai/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://res.openai.azure.com", Some("  "), "gpt-6-astra"),
            "https://res.openai.azure.com/openai/v1/chat/completions"
        );
    }

    #[test]
    fn preview_opts_into_v1_preview_features() {
        assert_eq!(
            chat_url(
                "https://res.openai.azure.com",
                Some("preview"),
                "gpt-6-astra"
            ),
            "https://res.openai.azure.com/openai/v1/chat/completions?api-version=preview"
        );
        assert_eq!(
            embeddings_url("https://res.openai.azure.com", Some("Preview"), "embed"),
            "https://res.openai.azure.com/openai/v1/embeddings?api-version=preview"
        );
    }

    #[test]
    fn legacy_urls_when_dated_api_version_set() {
        assert_eq!(
            chat_url("https://res.openai.azure.com", Some("2024-10-21"), "gpt-4o-dep"),
            "https://res.openai.azure.com/openai/deployments/gpt-4o-dep/chat/completions?api-version=2024-10-21"
        );
        assert_eq!(
            embeddings_url("https://res.openai.azure.com", Some("2024-10-21"), "embed-dep"),
            "https://res.openai.azure.com/openai/deployments/embed-dep/embeddings?api-version=2024-10-21"
        );
    }

    #[test]
    fn api_surface_classification() {
        assert_eq!(api_surface(None), ApiSurface::V1);
        assert_eq!(api_surface(Some("")), ApiSurface::V1);
        assert_eq!(api_surface(Some("V1")), ApiSurface::V1);
        assert_eq!(api_surface(Some("preview")), ApiSurface::V1Preview);
        assert_eq!(api_surface(Some("2024-10-21")), ApiSurface::LegacyDated);
        assert_eq!(
            api_surface(Some("2025-04-01-preview")),
            ApiSurface::LegacyDated
        );
    }

    #[test]
    fn deployment_mapping_falls_back_to_model_id() {
        let mut map = BTreeMap::new();
        map.insert("gpt-6-astra".to_string(), "astra-prod".to_string());
        assert_eq!(resolve_deployment("gpt-6-astra", Some(&map)), "astra-prod");
        assert_eq!(
            resolve_deployment("gpt-5.6-luna", Some(&map)),
            "gpt-5.6-luna"
        );
        assert_eq!(resolve_deployment("gpt-6-astra", None), "gpt-6-astra");
    }

    #[test]
    fn realtime_url_swaps_scheme() {
        assert_eq!(
            realtime_url("https://res.openai.azure.com"),
            "wss://res.openai.azure.com/openai/v1/realtime?intent=transcription"
        );
        assert_eq!(
            realtime_url("res.openai.azure.com/"),
            "wss://res.openai.azure.com/openai/v1/realtime?intent=transcription"
        );
    }

    #[test]
    fn foundry_resource_name_from_hosts() {
        assert_eq!(
            foundry_resource_name("https://claude-code-builds.openai.azure.com"),
            Some("claude-code-builds")
        );
        assert_eq!(
            foundry_resource_name("https://claude-code-builds.openai.azure.com/"),
            Some("claude-code-builds")
        );
        assert_eq!(
            foundry_resource_name("https://claude-code-builds.services.ai.azure.com/anthropic"),
            Some("claude-code-builds")
        );
        assert_eq!(
            foundry_resource_name("https://res.cognitiveservices.azure.com"),
            Some("res")
        );
        assert_eq!(foundry_resource_name("https://api.openai.com"), None);
    }

    #[test]
    fn voice_live_url_rewrites_openai_azure_host() {
        let expected = format!(
            "wss://claude-code-builds.services.ai.azure.com/voice-live/realtime?api-version={VOICE_LIVE_API_VERSION}&model=gpt-4.1-mini"
        );
        assert_eq!(
            voice_live_url(
                "https://claude-code-builds.openai.azure.com",
                "gpt-4.1-mini"
            ),
            expected
        );
        assert_eq!(
            voice_live_url(
                "https://claude-code-builds.services.ai.azure.com",
                "gpt-4.1-mini"
            ),
            expected
        );
        assert_eq!(
            voice_live_url("https://res.cognitiveservices.azure.com", "gpt-4.1-mini"),
            format!(
                "wss://res.cognitiveservices.azure.com/voice-live/realtime?api-version={VOICE_LIVE_API_VERSION}&model=gpt-4.1-mini"
            )
        );
    }
}
