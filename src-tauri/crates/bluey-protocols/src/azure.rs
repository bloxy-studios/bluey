//! Azure OpenAI / Microsoft Foundry URL building.
//!
//! The v1 surface (`{base}/openai/v1/...`, no `api-version`) is the default;
//! when a provider config carries an `api_version` the legacy dated form
//! (`{base}/openai/deployments/{d}/...?api-version=...`) is used instead.

use std::collections::BTreeMap;

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

/// Chat-completions URL. `api_version: None` → v1 GA surface.
pub fn chat_url(base_url: &str, api_version: Option<&str>, deployment: &str) -> String {
    build(base_url, api_version, deployment, "chat/completions")
}

/// Embeddings URL. `api_version: None` → v1 GA surface.
pub fn embeddings_url(base_url: &str, api_version: Option<&str>, deployment: &str) -> String {
    build(base_url, api_version, deployment, "embeddings")
}

fn build(base_url: &str, api_version: Option<&str>, deployment: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match api_version {
        Some(version) => {
            format!("{base}/openai/deployments/{deployment}/{path}?api-version={version}")
        }
        None => format!("{base}/openai/v1/{path}"),
    }
}

/// Realtime transcription WebSocket URL for an Azure resource:
/// `wss://{resource}/openai/v1/realtime?intent=transcription`.
pub fn realtime_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let wss = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if base.starts_with("wss://") || base.starts_with("ws://") {
        base.to_string()
    } else {
        format!("wss://{base}")
    };
    format!("{wss}/openai/v1/realtime?intent=transcription")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn v1_urls() {
        assert_eq!(
            chat_url("https://res.openai.azure.com", None, "gpt-4o"),
            "https://res.openai.azure.com/openai/v1/chat/completions"
        );
        assert_eq!(
            embeddings_url("https://res.openai.azure.com/", None, "embed"),
            "https://res.openai.azure.com/openai/v1/embeddings"
        );
    }

    #[test]
    fn legacy_urls_when_api_version_set() {
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
    fn deployment_mapping_falls_back_to_model_id() {
        let mut map = BTreeMap::new();
        map.insert("gpt-4o".to_string(), "my-4o-deployment".to_string());
        assert_eq!(resolve_deployment("gpt-4o", Some(&map)), "my-4o-deployment");
        assert_eq!(resolve_deployment("gpt-4o-mini", Some(&map)), "gpt-4o-mini");
        assert_eq!(resolve_deployment("gpt-4o", None), "gpt-4o");
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
}
