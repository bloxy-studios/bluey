//! Azure OpenAI / Microsoft Foundry provider: v1 GA surface
//! (`{base}/openai/v1/chat/completions`, `api-key` header, no `api-version`).
//! `api_version = "preview"` opts into v1 preview features; a *dated*
//! `api_version` selects the legacy deployment-scoped form (see
//! `bluey_protocols::azure`). The wire protocol itself is OpenAI-compatible and
//! `model` is always the Foundry *deployment name*.

use std::collections::BTreeMap;

use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::{api_error, azure as proto, openai as openai_proto};
use tokio_util::sync::CancellationToken;

use bluey_core::types::ModelRole;

use super::openai::spawn_openai_sse;
use super::EmbedPurpose;
use super::{
    api_error_from, map_http_status, map_transport_error, read_limited, remember_schema_in_prompt,
    schema_in_prompt, send_with_retry, AiProvider, ChunkStream, ProviderRequest,
};

/// Names the API in errors ("Azure Foundry rejected the request: …").
const HINT: &str = "Azure Foundry";

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

    async fn send_chat(
        &self,
        request: &ProviderRequest,
        schema_as_prompt: bool,
        token: &CancellationToken,
    ) -> BlueyResult<reqwest::Response> {
        let deployment = self.deployment(&request.model);
        let url = proto::chat_url(&self.base_url, self.api_version.as_deref(), deployment);
        // The Foundry presets are all reasoning models: the family is read from
        // the model id (the deployment name usually is the model id too).
        let effort = [request.model.as_str(), deployment]
            .into_iter()
            .find_map(|m| {
                openai_proto::reasoning_effort_for(m, request.reasoning, request.latency)
            });
        let body = openai_proto::build_chat_body(&openai_proto::ChatBodyOptions {
            model: deployment,
            messages: &request.messages,
            stream: true,
            include_usage: true,
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            reasoning_effort: effort.as_deref(),
            output_schema: request.output_schema.as_ref(),
            schema_as_prompt_fallback: schema_as_prompt,
        });
        let build = || {
            self.http
                .post(&url)
                .header("api-key", &self.api_key)
                .header("accept", "text/event-stream")
                .json(&body)
        };
        send_with_retry(build, token, HINT).await
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
        let endpoint = format!("{}|{deployment}", self.base_url);
        let mut schema_as_prompt = request.output_schema.is_some() && schema_in_prompt(&endpoint);
        let mut response = self.send_chat(request, schema_as_prompt, &token).await?;
        if response.status().as_u16() == 400 && request.output_schema.is_some() && !schema_as_prompt
        {
            // Read privately to spot a response_format rejection; never logged.
            let body = read_limited(response).await;
            if !openai_proto::is_response_format_rejection(400, &body) {
                let parsed = api_error::parse_error_body(&body);
                return Err(api_error::map_api_error(
                    400,
                    parsed.as_ref(),
                    None,
                    HINT,
                    deployment,
                ));
            }
            tracing::info!("deployment rejected response_format; schema goes in the prompt");
            remember_schema_in_prompt(&endpoint);
            schema_as_prompt = true;
            response = self.send_chat(request, schema_as_prompt, &token).await?;
        }
        if response.status().as_u16() >= 400 {
            return Err(api_error_from(response, HINT, deployment).await);
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

#[cfg(test)]
mod tests {
    use super::super::test_http::{openai_sse, response, stub_many};
    use super::*;
    use bluey_core::error::RecoveryAction;
    use bluey_core::types::{
        AiMessage, AiRole, AiTask, JsonSchemaSpec, LatencyBudget, ReasoningLevel,
    };

    fn provider(base: &str) -> AzureProvider {
        AzureProvider::new(reqwest::Client::new(), base.into(), None, None, "k".into())
    }

    fn request(model: &str) -> ProviderRequest {
        ProviderRequest {
            model: model.into(),
            messages: vec![AiMessage::text(AiRole::User, "hello")],
            max_output_tokens: None,
            temperature: Some(0.2),
            output_schema: None,
            task: AiTask::Answer,
            latency: LatencyBudget::Fast,
            reasoning: ReasoningLevel::None,
            session_id: None,
        }
    }

    async fn stream_error(base: &str, request: &ProviderRequest) -> BlueyError {
        provider(base)
            .stream(request, CancellationToken::new())
            .await
            .err()
            .expect("an error")
    }

    #[tokio::test]
    async fn a_missing_deployment_names_it_and_points_at_the_provider_settings() {
        let body = r#"{"error":{"code":"DeploymentNotFound","message":"The API deployment for this resource does not exist."}}"#;
        let (base, _) = stub_many(vec![response("404 Not Found", &[], body)]).await;
        let error = stream_error(&base, &request("gpt-5.5")).await;
        assert_eq!(error.code, "config.model_not_found");
        assert_eq!(error.recovery, Some(RecoveryAction::ConfigureProvider));
        assert_eq!(error.details.unwrap()["model"], "gpt-5.5");
    }

    #[tokio::test]
    async fn a_rejected_parameter_quotes_the_provider() {
        let body = r#"{"error":{"code":"unsupported_parameter","message":"Unsupported value: 'temperature'."}}"#;
        let (base, _) = stub_many(vec![response("400 Bad Request", &[], body)]).await;
        let error = stream_error(&base, &request("gpt-4.1")).await;
        assert_eq!(error.code, "ai.invalid_request");
        assert!(
            error.message.contains("Unsupported value: 'temperature'."),
            "{}",
            error.message
        );
    }

    #[tokio::test]
    async fn a_long_rate_limit_is_returned_with_its_delay() {
        let (base, _) = stub_many(vec![response(
            "429 Too Many Requests",
            &[("retry-after", "30")],
            "{}",
        )])
        .await;
        let error = stream_error(&base, &request("gpt-4.1")).await;
        assert_eq!(error.code, "network.http_429");
        assert_eq!(error.details.unwrap()["retryAfterMs"], 30_000);
    }

    #[tokio::test]
    async fn an_overloaded_endpoint_is_retried_before_the_first_byte() {
        let busy = || response("503 Service Unavailable", &[("retry-after-ms", "1")], "{}");
        let (base, seen) = stub_many(vec![busy(), busy(), openai_sse("hi")]).await;
        let stream = provider(&base)
            .stream(&request("gpt-4.1"), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(super::super::collect_text(stream).await.unwrap(), "hi");
        assert_eq!(seen.await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_rejected_response_format_moves_the_schema_into_the_prompt_for_good() {
        let rejected = r#"{"error":{"code":"BadRequest","message":"response_format json_schema is not supported"}}"#;
        let (base, seen) = stub_many(vec![
            response("400 Bad Request", &[], rejected),
            openai_sse("{}"),
            openai_sse("{}"),
        ])
        .await;
        let structured = ProviderRequest {
            output_schema: Some(JsonSchemaSpec {
                name: "answer".into(),
                schema: serde_json::json!({ "type": "object" }),
                strict: None,
            }),
            ..request("gpt-4.1")
        };
        for _ in 0..2 {
            let stream = provider(&base)
                .stream(&structured, CancellationToken::new())
                .await
                .unwrap();
            super::super::collect_text(stream).await.unwrap();
        }
        let bodies: Vec<serde_json::Value> = seen
            .await
            .unwrap()
            .into_iter()
            .map(|(_, body)| serde_json::from_str(&body).unwrap())
            .collect();
        assert!(bodies[0].get("response_format").is_some());
        for body in &bodies[1..] {
            assert!(body.get("response_format").is_none(), "{body}");
            assert_eq!(body["messages"][0]["role"], "system");
        }
    }
}
