//! Google Gemini provider (Google AI Studio API key).
//!
//! * `POST {base}/models/{model}:streamGenerateContent?alt=sse` for answers —
//!   bodies come from `bluey_protocols::gemini` (Gemini 3.x rules: roles
//!   `user`/`model`, `systemInstruction`, `thinkingLevel` only, JSON schema
//!   output via `responseJsonSchema`);
//! * `POST …/models/{model}:batchEmbedContents` for embeddings
//!   (`gemini-embedding-2`, MRL-truncated to the configured dimensions, with
//!   the documented `title:`/`task:` prompt prefixes);
//! * `GET …/models` (paged) for model listing, filtered per role.
//!
//! The key travels only in the `x-goog-api-key` header. Failed calls are
//! retried up to three times on 429/5xx — honouring the server's `retryDelay`
//! — and never on 400/403/404. Streams are retried only before the first byte
//! (i.e. on the initial HTTP status). Error bodies are read to extract the
//! machine-readable reason but are never logged (they can echo prompts).

use std::time::Duration;

use bluey_core::types::{FinishReason, ModelRole};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::gemini as proto;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::{
    channel_stream, map_transport_error, AiProvider, ChunkStream, EmbedPurpose, ProviderRequest,
    StreamItem,
};

const HINT: &str = "Gemini";
const MAX_ATTEMPTS: u32 = 3;
const BACKOFF_BASE: Duration = Duration::from_millis(600);
const BACKOFF_CAP: Duration = Duration::from_secs(8);
/// Safety cap on `GET /models` paging.
const MAX_MODEL_PAGES: usize = 10;

pub struct GeminiProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    embedding_dimensions: u32,
}

fn backoff(attempt: u32) -> Duration {
    BACKOFF_BASE
        .checked_mul(2u32.saturating_pow(attempt.saturating_sub(1)))
        .unwrap_or(BACKOFF_CAP)
        .min(BACKOFF_CAP)
}

fn cancelled() -> BlueyError {
    BlueyError::cancelled()
}

impl GeminiProvider {
    pub fn new(
        http: reqwest::Client,
        base_url: String,
        api_key: String,
        embedding_dimensions: u32,
    ) -> Self {
        // An empty base URL means the public endpoint; the codec fills it in.
        Self {
            http,
            base_url,
            api_key,
            embedding_dimensions,
        }
    }

    fn post(&self, url: &str, body: &serde_json::Value) -> reqwest::RequestBuilder {
        self.http
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .json(body)
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.http.get(url).header("x-goog-api-key", &self.api_key)
    }

    /// Send a request, retrying on 429/5xx (before any body is consumed).
    /// Returns the first successful response.
    async fn send_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
        token: Option<&CancellationToken>,
    ) -> BlueyResult<reqwest::Response> {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            if token.map(|t| t.is_cancelled()).unwrap_or(false) {
                return Err(cancelled());
            }
            let response = build()
                .send()
                .await
                .map_err(|e| map_transport_error(&e, HINT))?;
            let status = response.status().as_u16();
            if status < 400 {
                return Ok(response);
            }
            // Private read: only the structured reason / retry hints are kept.
            let body = response.text().await.unwrap_or_default();
            let parsed = proto::parse_error_body(&body);
            let error = proto::map_gemini_error(status, parsed.as_ref());
            if attempt >= MAX_ATTEMPTS || !proto::is_retryable_status(status) {
                return Err(error);
            }
            let delay = parsed
                .as_ref()
                .and_then(|e| e.retry_after)
                .unwrap_or_else(|| backoff(attempt))
                .min(BACKOFF_CAP);
            tracing::info!(
                status,
                attempt,
                delay_ms = delay.as_millis() as u64,
                "gemini request failed; retrying"
            );
            match token {
                Some(token) => {
                    tokio::select! {
                        _ = token.cancelled() => return Err(cancelled()),
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
                None => tokio::time::sleep(delay).await,
            }
        }
    }
}

#[async_trait::async_trait]
impl AiProvider for GeminiProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        token: CancellationToken,
    ) -> BlueyResult<ChunkStream> {
        let thinking = proto::thinking_level_for(
            request.task,
            request.latency,
            request.reasoning,
            &request.model,
        );
        let body = proto::build_generate_body(&proto::GenerateBodyOptions {
            model: &request.model,
            messages: &request.messages,
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            output_schema: request.output_schema.as_ref(),
            thinking_level: thinking,
        });
        let url = proto::generate_url(&self.base_url, &request.model, true);
        let response = self
            .send_with_retry(
                || self.post(&url, &body).header("accept", "text/event-stream"),
                Some(&token),
            )
            .await?;
        Ok(spawn_gemini_sse(response, token))
    }

    async fn embed(
        &self,
        model: &str,
        texts: &[String],
        purpose: &EmbedPurpose,
    ) -> BlueyResult<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        let url = proto::batch_embed_url(&self.base_url, model);
        for batch in texts.chunks(proto::EMBED_BATCH_SIZE) {
            let prefixed: Vec<String> = batch
                .iter()
                .map(|text| proto::embedding_text(purpose, text, model))
                .collect();
            let body =
                proto::build_batch_embed_body(model, &prefixed, Some(self.embedding_dimensions));
            let response = self
                .send_with_retry(|| self.post(&url, &body), None)
                .await?;
            let text = response
                .text()
                .await
                .map_err(|e| map_transport_error(&e, HINT))?;
            let parsed = proto::parse_batch_embeddings(&text).map_err(|_| {
                BlueyError::ai("embeddings_parse", "unexpected embeddings response")
            })?;
            if parsed.len() != batch.len() {
                return Err(BlueyError::ai(
                    "embeddings_count",
                    "the embeddings response did not match the request",
                ));
            }
            vectors.extend(parsed);
        }
        Ok(vectors)
    }

    async fn list_models(&self, role: Option<ModelRole>) -> BlueyResult<Vec<String>> {
        let mut models: Vec<proto::ModelInfo> = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_MODEL_PAGES {
            let url = proto::models_url(&self.base_url, page_token.as_deref());
            let response = self.send_with_retry(|| self.get(&url), None).await?;
            let text = response
                .text()
                .await
                .map_err(|e| map_transport_error(&e, HINT))?;
            let (page, next) = proto::parse_models_page(&text)
                .map_err(|_| BlueyError::ai("models_parse", "unexpected models response"))?;
            models.extend(page);
            match next {
                Some(token) => page_token = Some(token),
                None => break,
            }
        }
        Ok(models
            .into_iter()
            .filter(|info| role.map(|r| proto::role_filter(r, info)).unwrap_or(true))
            .map(|info| info.id)
            .collect())
    }
}

fn spawn_gemini_sse(response: reqwest::Response, token: CancellationToken) -> ChunkStream {
    let (tx, stream) = channel_stream();
    tauri::async_runtime::spawn(async move {
        let mut events = response.bytes_stream().eventsource();
        let mut finish = FinishReason::Stop;
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
            if frame.data.trim().is_empty() {
                continue;
            }
            let chunk = match proto::parse_response(&frame.data) {
                Ok(chunk) => chunk,
                Err(_) => continue,
            };
            if let Some(reason) = chunk.block_reason.as_deref() {
                let _ = tx.send(Err(proto::blocked_error(reason))).await;
                return;
            }
            if !chunk.text.is_empty() && tx.send(Ok(StreamItem::Delta(chunk.text))).await.is_err() {
                return;
            }
            if let Some(usage) = chunk.usage {
                let _ = tx
                    .send(Ok(StreamItem::Usage {
                        input: usage.prompt,
                        output: usage.output_tokens(),
                    }))
                    .await;
            }
            if let Some(reason) = chunk.finish.as_deref() {
                if proto::is_error_finish(reason) {
                    let _ = tx.send(Err(proto::blocked_error(reason))).await;
                    return;
                }
                finish = proto::map_finish_reason(reason);
                finished_sent = true;
                let _ = tx.send(Ok(StreamItem::Finished(finish))).await;
                break;
            }
        }
        if !finished_sent && !token.is_cancelled() {
            let _ = tx.send(Ok(StreamItem::Finished(finish))).await;
        }
    });
    stream
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff(1), Duration::from_millis(600));
        assert_eq!(backoff(2), Duration::from_millis(1200));
        assert_eq!(backoff(3), Duration::from_millis(2400));
        assert_eq!(backoff(30), BACKOFF_CAP);
    }
}
