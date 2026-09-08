//! Anthropic Messages API provider (`POST {base}/v1/messages`, `x-api-key`,
//! `anthropic-version: 2023-06-01`). Structured output goes through
//! `output_config`; a 400 rejection falls back to instructing the schema in
//! the prompt.
//!
//! Works unchanged against Claude in Microsoft Foundry: set the provider's
//! base URL to `https://{resource}.services.ai.azure.com/anthropic` and use
//! the Foundry resource key (Foundry accepts `x-api-key`); `model` is then the
//! Foundry deployment name (defaults to the model id, e.g. `claude-opus-5`).

use bluey_core::types::FinishReason;
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::anthropic as proto;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::{
    channel_stream, map_http_status, map_transport_error, AiProvider, ChunkStream, ProviderRequest,
    StreamItem,
};

pub struct AnthropicProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl AnthropicProvider {
    pub fn new(http: reqwest::Client, base_url: String, api_key: String) -> Self {
        let base_url = if base_url.trim().is_empty() {
            "https://api.anthropic.com".to_string()
        } else {
            base_url
        };
        Self {
            http,
            base_url,
            api_key,
        }
    }

    async fn send_messages(
        &self,
        request: &ProviderRequest,
        schema_as_prompt: bool,
    ) -> Result<reqwest::Response, BlueyError> {
        let body = proto::build_messages_body(&proto::MessagesBodyOptions {
            model: &request.model,
            messages: &request.messages,
            stream: true,
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            output_schema: request.output_schema.as_ref(),
            schema_as_prompt_fallback: schema_as_prompt,
        });
        self.http
            .post(proto::messages_url(&self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", proto::ANTHROPIC_VERSION)
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "Anthropic"))
    }
}

#[async_trait::async_trait]
impl AiProvider for AnthropicProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        token: CancellationToken,
    ) -> BlueyResult<ChunkStream> {
        let mut response = self.send_messages(request, false).await?;
        let mut status = response.status().as_u16();
        if status == 400 && request.output_schema.is_some() {
            // Read the body privately to detect an output_config rejection;
            // never logged (may quote the request).
            let body = response.text().await.unwrap_or_default();
            if proto::is_output_config_rejection(status, &body) {
                tracing::info!("anthropic rejected output_config; retrying with schema-in-prompt");
                response = self.send_messages(request, true).await?;
                status = response.status().as_u16();
            } else {
                return Err(map_http_status(status, "Anthropic"));
            }
        }
        if status >= 400 {
            return Err(map_http_status(status, "Anthropic"));
        }
        Ok(spawn_anthropic_sse(response, token))
    }

    async fn embed(&self, _model: &str, _texts: &[String]) -> BlueyResult<Vec<Vec<f32>>> {
        Err(BlueyError::not_supported(
            "embeddings",
            "the Anthropic API does not serve embeddings",
        ))
    }

    async fn list_models(&self) -> BlueyResult<Vec<String>> {
        let response = self
            .http
            .get(proto::models_url(&self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", proto::ANTHROPIC_VERSION)
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "Anthropic"))?;
        let status = response.status().as_u16();
        if status >= 400 {
            return Err(map_http_status(status, "Anthropic"));
        }
        #[derive(serde::Deserialize)]
        struct Models {
            #[serde(default)]
            data: Vec<ModelRow>,
        }
        #[derive(serde::Deserialize)]
        struct ModelRow {
            id: String,
        }
        let parsed: Models = response
            .json()
            .await
            .map_err(|_| BlueyError::ai("models_parse", "unexpected models response"))?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }
}

fn spawn_anthropic_sse(response: reqwest::Response, token: CancellationToken) -> ChunkStream {
    let (tx, stream) = channel_stream();
    tauri::async_runtime::spawn(async move {
        let mut events = response.bytes_stream().eventsource();
        let mut json_acc = proto::JsonAccumulator::new();
        let mut finish = FinishReason::Stop;
        let mut input_tokens: Option<u32> = None;
        let mut finished_sent = false;
        loop {
            let frame = tokio::select! {
                _ = token.cancelled() => break,
                frame = events.next() => frame,
            };
            let Some(frame) = frame else { break };
            let frame = match frame {
                Ok(frame) => frame,
                Err(_) => {
                    let _ = tx
                        .send(Err(BlueyError::network(
                            "stream",
                            "the response stream ended unexpectedly",
                        )))
                        .await;
                    return;
                }
            };
            let event_name = frame.event.as_deref().unwrap_or("message");
            let event = match proto::parse_event(event_name, &frame.data) {
                Ok(event) => event,
                Err(_) => continue,
            };
            match event {
                proto::StreamEvent::MessageStart {
                    input_tokens: tokens,
                } => input_tokens = tokens,
                proto::StreamEvent::TextDelta { text } => {
                    if !text.is_empty() && tx.send(Ok(StreamItem::Delta(text))).await.is_err() {
                        return;
                    }
                }
                proto::StreamEvent::InputJsonDelta { partial_json } => {
                    // Structured tool-style output: accumulate, emit at the end.
                    json_acc.push(&partial_json);
                }
                proto::StreamEvent::MessageDelta {
                    stop_reason,
                    output_tokens,
                } => {
                    if let Some(reason) = stop_reason.as_deref() {
                        finish = proto::map_stop_reason(reason);
                    }
                    if output_tokens.is_some() || input_tokens.is_some() {
                        let _ = tx
                            .send(Ok(StreamItem::Usage {
                                input: input_tokens,
                                output: output_tokens,
                            }))
                            .await;
                    }
                }
                proto::StreamEvent::MessageStop => {
                    if !json_acc.is_empty() {
                        // Deliver accumulated structured output as one delta.
                        let text = json_acc.text().to_string();
                        let _ = tx.send(Ok(StreamItem::Delta(text))).await;
                    }
                    finished_sent = true;
                    let _ = tx.send(Ok(StreamItem::Finished(finish))).await;
                    break;
                }
                proto::StreamEvent::Error {
                    error_type,
                    message,
                } => {
                    let error = if error_type == "overloaded_error" {
                        BlueyError::network("overloaded", message)
                    } else {
                        BlueyError::ai(&error_type, message)
                    };
                    let _ = tx.send(Err(error)).await;
                    return;
                }
                _ => {}
            }
        }
        if !finished_sent && !token.is_cancelled() {
            let _ = tx.send(Ok(StreamItem::Finished(finish))).await;
        }
    });
    stream
}
