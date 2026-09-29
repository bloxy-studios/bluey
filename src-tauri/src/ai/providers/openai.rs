//! Generic OpenAI-compatible provider (`{base}/v1/chat/completions`, Bearer).

use bluey_core::types::FinishReason;
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::api_error;
use bluey_protocols::openai as proto;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use bluey_core::types::ModelRole;

use super::EmbedPurpose;
use super::{
    api_error_from, channel_stream, map_http_status, map_transport_error, read_limited,
    remember_schema_in_prompt, schema_in_prompt, send_with_retry, AiProvider, ChunkStream,
    ProviderRequest, StreamItem,
};

/// Names the API in errors ("provider rejected the request: …").
const HINT: &str = "provider";

pub struct OpenAiProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl OpenAiProvider {
    pub fn new(http: reqwest::Client, base_url: String, api_key: String) -> Self {
        Self {
            http,
            base_url,
            api_key,
        }
    }

    async fn send_chat(
        &self,
        request: &ProviderRequest,
        schema_as_prompt: bool,
        token: &CancellationToken,
    ) -> BlueyResult<reqwest::Response> {
        let effort =
            proto::reasoning_effort_for(&request.model, request.reasoning, request.latency);
        let body = proto::build_chat_body(&proto::ChatBodyOptions {
            model: &request.model,
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
                .post(proto::chat_url(&self.base_url))
                .bearer_auth(&self.api_key)
                .header("accept", "text/event-stream")
                .json(&body)
        };
        send_with_retry(build, token, HINT).await
    }
}

#[async_trait::async_trait]
impl AiProvider for OpenAiProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        token: CancellationToken,
    ) -> BlueyResult<ChunkStream> {
        let endpoint = format!("{}|{}", self.base_url, request.model);
        let mut schema_as_prompt = request.output_schema.is_some() && schema_in_prompt(&endpoint);
        let mut response = self.send_chat(request, schema_as_prompt, &token).await?;
        if response.status().as_u16() == 400 && request.output_schema.is_some() && !schema_as_prompt
        {
            // Read privately to spot a response_format rejection; never logged.
            let body = read_limited(response).await;
            if !proto::is_response_format_rejection(400, &body) {
                let parsed = api_error::parse_error_body(&body);
                return Err(api_error::map_api_error(
                    400,
                    parsed.as_ref(),
                    None,
                    HINT,
                    &request.model,
                ));
            }
            tracing::info!("endpoint rejected response_format; schema goes in the prompt");
            remember_schema_in_prompt(&endpoint);
            schema_as_prompt = true;
            response = self.send_chat(request, schema_as_prompt, &token).await?;
        }
        if response.status().as_u16() >= 400 {
            return Err(api_error_from(response, HINT, &request.model).await);
        }
        Ok(spawn_openai_sse(response, token))
    }

    async fn embed(
        &self,
        model: &str,
        texts: &[String],
        _purpose: &EmbedPurpose,
    ) -> BlueyResult<Vec<Vec<f32>>> {
        let body = proto::build_embeddings_body(model, texts);
        let response = self
            .http
            .post(proto::embeddings_url(&self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "provider"))?;
        let status = response.status().as_u16();
        if status >= 400 {
            return Err(map_http_status(status, "provider"));
        }
        let parsed: proto::EmbeddingsResponse = response
            .json()
            .await
            .map_err(|_| BlueyError::ai("embeddings_parse", "unexpected embeddings response"))?;
        Ok(parsed.into_vectors())
    }

    async fn list_models(&self, _role: Option<ModelRole>) -> BlueyResult<Vec<String>> {
        let response = self
            .http
            .get(proto::models_url(&self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "provider"))?;
        let status = response.status().as_u16();
        if status >= 400 {
            return Err(map_http_status(status, "provider"));
        }
        let parsed: proto::ModelsResponse = response
            .json()
            .await
            .map_err(|_| BlueyError::ai("models_parse", "unexpected models response"))?;
        Ok(parsed.into_ids())
    }
}

/// Consume an OpenAI-compatible SSE response on a task, forwarding chunk items.
/// Shared by the OpenAI-compatible and Azure adapters.
pub(super) fn spawn_openai_sse(
    response: reqwest::Response,
    token: CancellationToken,
) -> ChunkStream {
    let (tx, stream) = channel_stream();
    tauri::async_runtime::spawn(async move {
        let mut events = response.bytes_stream().eventsource();
        let mut finished = false;
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
            if bluey_protocols::sse::is_done(&frame.data) {
                break;
            }
            let chunk = match proto::parse_chunk(&frame.data) {
                Ok(chunk) => chunk,
                Err(_) => continue, // tolerate unknown frames
            };
            if let Some(usage) = chunk.usage {
                if tx
                    .send(Ok(StreamItem::Usage {
                        input: usage.prompt_tokens,
                        output: usage.completion_tokens,
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            for choice in chunk.choices {
                if let Some(content) = choice.delta.content {
                    if !content.is_empty() && tx.send(Ok(StreamItem::Delta(content))).await.is_err()
                    {
                        return;
                    }
                }
                if let Some(reason) = choice.finish_reason {
                    finished = true;
                    let _ = tx
                        .send(Ok(StreamItem::Finished(proto::map_finish_reason(&reason))))
                        .await;
                }
            }
        }
        if !finished && !token.is_cancelled() {
            let _ = tx.send(Ok(StreamItem::Finished(FinishReason::Stop))).await;
        }
    });
    stream
}

#[cfg(test)]
mod tests {
    use super::super::test_http::{response, stub_many};
    use super::*;
    use bluey_core::types::{AiMessage, AiRole, AiTask, LatencyBudget, ReasoningLevel};

    #[tokio::test]
    async fn an_unknown_model_is_a_configuration_error_not_a_temporary_one() {
        let body =
            r#"{"error":{"code":"model_not_found","message":"The model `gpt-9` does not exist."}}"#;
        let (base, _) = stub_many(vec![response("404 Not Found", &[], body)]).await;
        let request = ProviderRequest {
            model: "gpt-9".into(),
            messages: vec![AiMessage::text(AiRole::User, "hello")],
            max_output_tokens: None,
            temperature: None,
            output_schema: None,
            task: AiTask::Answer,
            latency: LatencyBudget::Fast,
            reasoning: ReasoningLevel::None,
            session_id: None,
        };
        let error = OpenAiProvider::new(reqwest::Client::new(), base, "k".into())
            .stream(&request, CancellationToken::new())
            .await
            .err()
            .expect("404");
        assert_eq!(error.code, "config.model_not_found");
        assert_eq!(error.details.unwrap()["model"], "gpt-9");
    }
}
