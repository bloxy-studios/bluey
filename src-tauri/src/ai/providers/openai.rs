//! Generic OpenAI-compatible provider (`{base}/v1/chat/completions`, Bearer).

use bluey_core::types::FinishReason;
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::openai as proto;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use bluey_core::types::ModelRole;

use super::EmbedPurpose;
use super::{
    channel_stream, map_http_status, map_transport_error, AiProvider, ChunkStream, ProviderRequest,
    StreamItem,
};

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
}

#[async_trait::async_trait]
impl AiProvider for OpenAiProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        token: CancellationToken,
    ) -> BlueyResult<ChunkStream> {
        let body = proto::build_chat_body(&proto::ChatBodyOptions {
            model: &request.model,
            messages: &request.messages,
            stream: true,
            include_usage: true,
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            output_schema: request.output_schema.as_ref(),
        });
        let response = self
            .http
            .post(proto::chat_url(&self.base_url))
            .bearer_auth(&self.api_key)
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "provider"))?;
        let status = response.status().as_u16();
        if status >= 400 {
            return Err(map_http_status(status, "provider"));
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
