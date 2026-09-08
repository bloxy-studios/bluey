//! Azure OpenAI / Microsoft Foundry provider: v1 GA surface
//! (`{base}/openai/v1/chat/completions`, `api-key` header, no `api-version`).
//! `api_version = "preview"` opts into v1 preview features; a *dated*
//! `api_version` selects the legacy deployment-scoped form (see
//! `bluey_protocols::azure`). The wire protocol itself is OpenAI-compatible and
//! `model` is always the Foundry *deployment name*.

use std::collections::BTreeMap;

use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::{azure as proto, openai as openai_proto};
use tokio_util::sync::CancellationToken;

use bluey_core::types::ModelRole;

use super::openai::spawn_openai_sse;
use super::EmbedPurpose;
use super::{map_http_status, map_transport_error, AiProvider, ChunkStream, ProviderRequest};

/// Common Foundry model names offered alongside the deployments map in
/// `ai_list_models` (deployments cannot be enumerated with a data-plane key).
/// Deployment names default to the model id, so these work out of the box for
/// standard deployments. Current OpenAI line-up on Foundry (Sep 2026):
/// GPT‑6 Astra (frontier), GPT‑5.6 Sol / Terra / Luna (reasoning / balanced /
/// fast), GPT‑5.5, GPT‑4.1 family, MAI-Transcribe-1.5 (Voice Live STT), and the
/// text-embedding-3 models.
const COMMON_MODELS: &[&str] = &[
    "gpt-6-astra",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-4.1",
    "gpt-4.1-mini",
    "MAI-Transcribe-1.5",
    "text-embedding-3-small",
    "text-embedding-3-large",
];

pub struct AzureProvider {
    http: reqwest::Client,
    base_url: String,
    api_version: Option<String>,
    deployments: Option<BTreeMap<String, String>>,
    api_key: String,
}

impl AzureProvider {
    pub fn new(
        http: reqwest::Client,
        base_url: String,
        api_version: Option<String>,
        deployments: Option<BTreeMap<String, String>>,
        api_key: String,
    ) -> Self {
        Self {
            http,
            base_url,
            api_version,
            deployments,
            api_key,
        }
    }

    fn deployment<'a>(&'a self, model: &'a str) -> &'a str {
        proto::resolve_deployment(model, self.deployments.as_ref())
    }
}

#[async_trait::async_trait]
impl AiProvider for AzureProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        token: CancellationToken,
    ) -> BlueyResult<ChunkStream> {
        let deployment = self.deployment(&request.model);
        let url = proto::chat_url(&self.base_url, self.api_version.as_deref(), deployment);
        let body = openai_proto::build_chat_body(&openai_proto::ChatBodyOptions {
            model: deployment,
            messages: &request.messages,
            stream: true,
            include_usage: true,
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            output_schema: request.output_schema.as_ref(),
        });
        let response = self
            .http
            .post(url)
            .header("api-key", &self.api_key)
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "Azure"))?;
        let status = response.status().as_u16();
        if status >= 400 {
            return Err(map_http_status(status, "Azure"));
        }
        Ok(spawn_openai_sse(response, token))
    }

    async fn embed(
        &self,
        model: &str,
        texts: &[String],
        _purpose: &EmbedPurpose,
    ) -> BlueyResult<Vec<Vec<f32>>> {
        let deployment = self.deployment(model);
        let url = proto::embeddings_url(&self.base_url, self.api_version.as_deref(), deployment);
        let body = openai_proto::build_embeddings_body(deployment, texts);
        let response = self
            .http
            .post(url)
            .header("api-key", &self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "Azure"))?;
        let status = response.status().as_u16();
        if status >= 400 {
            return Err(map_http_status(status, "Azure"));
        }
        let parsed: openai_proto::EmbeddingsResponse = response
            .json()
            .await
            .map_err(|_| BlueyError::ai("embeddings_parse", "unexpected embeddings response"))?;
        Ok(parsed.into_vectors())
    }

    async fn list_models(&self, _role: Option<ModelRole>) -> BlueyResult<Vec<String>> {
        let mut models: Vec<String> = self
            .deployments
            .as_ref()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default();
        for common in COMMON_MODELS {
            if !models.iter().any(|m| m == common) {
                models.push((*common).to_string());
            }
        }
        Ok(models)
    }
}
