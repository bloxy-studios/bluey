//! Google Gemini provider (Google AI Studio API key).
//!
//! * `POST {base}/models/{model}:streamGenerateContent?alt=sse` for answers —
//!   bodies come from `bluey_protocols::gemini` (Gemini 3.x rules: roles
//!   `user`/`model`, `systemInstruction`, `thinkingLevel` only, JSON schema
//!   output via `responseJsonSchema`);
//! * `POST …/models/{model}:batchEmbedContents` for embeddings
//!   (`gemini-embedding-2`, MRL-truncated to the configured dimensions, with
//!   the documented `title:`/`task:` prompt prefixes);
//! * `GET …/models` (paged) for model listing, filtered per role;
//! * `POST …/models/gemini-3.5-transcribe:generateContent` for whole
//!   recordings (`audioTranscriptionConfig`), inline up to 14 MB and through
//!   the Files API resumable upload above that (the upload is deleted again
//!   right after the transcription — raw audio is never kept around).
//!
//! The key travels only in the `x-goog-api-key` header. Failed calls are
//! retried up to three times on 429/5xx — honouring the server's `retryDelay`
//! — and never on 400/403/404. Streams are retried only before the first byte
//! (i.e. on the initial HTTP status). Error bodies are read to extract the
//! machine-readable reason but are never logged (they can echo prompts).

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use bluey_core::types::{FinishReason, ModelRole};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::gemini as proto;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::{
    channel_stream, map_transport_error, AiProvider, AudioFile, ChunkStream, EmbedPurpose,
    ProviderRequest, StreamItem, TranscribeFileOptions, Transcription,
};

const HINT: &str = "Gemini";
const MAX_ATTEMPTS: u32 = 3;
const BACKOFF_BASE: Duration = Duration::from_millis(600);
const BACKOFF_CAP: Duration = Duration::from_secs(8);
/// Safety cap on `GET /models` paging.
const MAX_MODEL_PAGES: usize = 10;
/// Files API: how often / how long to wait for an upload to leave `PROCESSING`.
const UPLOAD_POLL_INTERVAL: Duration = Duration::from_secs(2);
const UPLOAD_POLL_ATTEMPTS: u32 = 30;
/// Whole-request deadline for unary calls (embeddings, model list, Files API).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// A batch transcription of up to an hour of audio can take a while.
const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Uploading a recording of up to 2 GB.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Longest silence between two SSE frames before the stream counts as stalled.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

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

/// Up to 250 ms of jitter so parallel requests do not retry in lockstep.
fn jitter() -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    Duration::from_millis(u64::from(nanos % 250))
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
            let sent = match token {
                Some(token) => tokio::select! {
                    _ = token.cancelled() => return Err(cancelled()),
                    sent = build().send() => sent,
                },
                None => build().send().await,
            };
            let response = sent.map_err(|e| map_transport_error(&e, HINT))?;
            let status = response.status().as_u16();
            if status < 400 {
                return Ok(response);
            }
            // Private read: only the structured reason / retry hints are kept.
            let body = response.text().await.unwrap_or_default();
            let parsed = proto::parse_error_body(&body);
            let error = proto::map_gemini_error(status, parsed.as_ref());
            let daily_quota = parsed
                .as_ref()
                .map(proto::GeminiError::is_daily_quota)
                .unwrap_or(false);
            let server_delay = parsed.as_ref().and_then(|e| e.retry_after);
            // A daily quota cannot clear within a retry, and a server delay longer
            // than the cap would only be honoured by waiting — the caller can
            // decide that with the `retryAfterMs` detail instead.
            if attempt >= MAX_ATTEMPTS
                || !proto::is_retryable_status(status)
                || daily_quota
                || server_delay.map(|d| d > BACKOFF_CAP).unwrap_or(false)
            {
                return Err(error);
            }
            let delay = server_delay.unwrap_or_else(|| backoff(attempt)) + jitter();
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

    /// Map a non-2xx response onto the contract error (the body is read only
    /// for its structured reason and never logged).
    async fn check_status(response: reqwest::Response) -> BlueyResult<reqwest::Response> {
        let status = response.status().as_u16();
        if status < 400 {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        let parsed = proto::parse_error_body(&body);
        Err(proto::map_gemini_error(status, parsed.as_ref()))
    }

    /// Resumable upload to the Files API for recordings above the inline cap:
    /// `start` → upload URL → one `upload, finalize` chunk → poll until `ACTIVE`.
    async fn upload_file(
        &self,
        bytes: Vec<u8>,
        mime_type: &str,
        display_name: &str,
    ) -> BlueyResult<proto::UploadedFile> {
        let len = bytes.len();
        let start_url = proto::files_upload_url(&self.base_url);
        let start_body = proto::upload_start_body(display_name);
        let started = self
            .send_with_retry(
                || {
                    self.post(&start_url, &start_body)
                        .timeout(REQUEST_TIMEOUT)
                        .header("X-Goog-Upload-Protocol", "resumable")
                        .header("X-Goog-Upload-Command", "start")
                        .header("X-Goog-Upload-Header-Content-Length", len.to_string())
                        .header("X-Goog-Upload-Header-Content-Type", mime_type)
                },
                None,
            )
            .await?;
        let upload_url = started
            .headers()
            .get(proto::UPLOAD_URL_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| {
                BlueyError::ai(
                    "upload_url_missing",
                    "the Files API did not return an upload URL",
                )
            })?;
        let response = self
            .http
            .post(&upload_url)
            .header("x-goog-api-key", &self.api_key)
            .header("X-Goog-Upload-Offset", "0")
            .header("X-Goog-Upload-Command", "upload, finalize")
            .timeout(UPLOAD_TIMEOUT)
            .body(bytes)
            .send()
            .await
            .map_err(|e| map_transport_error(&e, HINT))?;
        let response = Self::check_status(response).await?;
        let text = response
            .text()
            .await
            .map_err(|e| map_transport_error(&e, HINT))?;
        let file = Self::parse_file(&text)?;
        // From here on the recording exists at Google: whatever happens next, a
        // failure must not leave it there for the 48-hour retention window.
        let name = file.name.clone();
        match self.await_active(file).await {
            Ok(file) => Ok(file),
            Err(error) => {
                if !name.is_empty() {
                    self.delete_file(&name).await;
                }
                Err(error)
            }
        }
    }

    fn parse_file(text: &str) -> BlueyResult<proto::UploadedFile> {
        proto::parse_uploaded_file(text)
            .map_err(|_| BlueyError::ai("upload_parse", "unexpected Files API response"))
    }

    /// Poll `GET …/files/{id}` until the upload leaves `PROCESSING`.
    async fn await_active(
        &self,
        mut file: proto::UploadedFile,
    ) -> BlueyResult<proto::UploadedFile> {
        let mut polls = 0u32;
        while file.is_processing() {
            polls += 1;
            if polls > UPLOAD_POLL_ATTEMPTS {
                return Err(BlueyError::network(
                    "timeout",
                    "the uploaded recording was still processing after 60 s",
                ));
            }
            tokio::time::sleep(UPLOAD_POLL_INTERVAL).await;
            let url = proto::file_url(&self.base_url, &file.name);
            let response = self
                .send_with_retry(|| self.get(&url).timeout(REQUEST_TIMEOUT), None)
                .await?;
            let text = response
                .text()
                .await
                .map_err(|e| map_transport_error(&e, HINT))?;
            file = Self::parse_file(&text)?;
        }
        if file.is_failed() || file.uri.is_empty() {
            return Err(BlueyError::ai(
                "upload_failed",
                "the Files API could not process the recording",
            ));
        }
        Ok(file)
    }

    /// Best-effort delete of an uploaded recording (Google would otherwise
    /// keep it for 48 hours). Never fails the transcription.
    async fn delete_file(&self, name: &str) {
        let url = proto::file_url(&self.base_url, name);
        match self
            .http
            .delete(&url)
            .header("x-goog-api-key", &self.api_key)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                tracing::debug!("deleted the uploaded recording")
            }
            Ok(response) => tracing::warn!(
                status = response.status().as_u16(),
                "could not delete the uploaded recording"
            ),
            Err(e) => tracing::warn!(error = %e, "could not delete the uploaded recording"),
        }
    }

    async fn transcribe_request(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> BlueyResult<Transcription> {
        let response = self
            .send_with_retry(|| self.post(url, body).timeout(TRANSCRIBE_TIMEOUT), None)
            .await?;
        let text = response
            .text()
            .await
            .map_err(|e| map_transport_error(&e, HINT))?;
        let parsed = proto::parse_transcription(&text).map_err(|_| {
            BlueyError::ai("transcription_parse", "unexpected transcription response")
        })?;
        if let Some(reason) = parsed.block_reason.as_deref() {
            return Err(proto::blocked_error(reason));
        }
        if let Some(finish) = parsed.finish.as_deref() {
            if proto::is_error_finish(finish) {
                return Err(proto::blocked_error(finish));
            }
        }
        Ok(parsed)
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
            // 3.x counts thinking tokens against the output budget; without the
            // allowance a structured answer can end in MAX_TOKENS before the JSON
            // envelope is complete (docs/AI_ARCHITECTURE.md › Output budgets).
            max_output_tokens: proto::max_output_tokens_with_thinking(
                request.max_output_tokens,
                thinking,
            ),
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
        Ok(spawn_gemini_sse(response, token, SseMode::Direct))
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
                .send_with_retry(|| self.post(&url, &body).timeout(REQUEST_TIMEOUT), None)
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
            if parsed
                .iter()
                .any(|vector| vector.len() as u32 != self.embedding_dimensions)
            {
                return Err(BlueyError::ai(
                    "embeddings_dimensions",
                    "the embeddings response did not use the configured dimensions",
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
            let response = self
                .send_with_retry(|| self.get(&url).timeout(REQUEST_TIMEOUT), None)
                .await?;
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

    async fn transcribe_audio(
        &self,
        model: &str,
        audio: AudioFile,
        options: &TranscribeFileOptions,
    ) -> BlueyResult<Transcription> {
        let AudioFile {
            bytes,
            mime_type,
            display_name,
        } = audio;
        let opts = proto::TranscribeOptions {
            language: options.language.as_deref(),
            diarization: options.diarization,
            word_timestamps: options.word_timestamps,
        };
        let url = proto::generate_url(&self.base_url, model, false);
        let inline = (bytes.len() as u64) <= proto::INLINE_AUDIO_MAX_BYTES;
        tracing::info!(
            model,
            bytes = bytes.len(),
            inline,
            diarization = options.diarization,
            word_timestamps = options.word_timestamps,
            "transcribing recording"
        );
        if inline {
            let encoded = BASE64.encode(&bytes);
            let body = proto::build_transcribe_body(
                &proto::AudioInput::Inline {
                    mime_type,
                    base64: &encoded,
                },
                &opts,
            );
            return self.transcribe_request(&url, &body).await;
        }
        let file = self.upload_file(bytes, mime_type, &display_name).await?;
        let body = proto::build_transcribe_body(
            &proto::AudioInput::File {
                uri: &file.uri,
                mime_type,
            },
            &opts,
        );
        let result = self.transcribe_request(&url, &body).await;
        self.delete_file(&file.name).await;
        result
    }
}

/// How the `data:` frames are framed: the Gemini API sends `GenerateContentResponse`
/// chunks directly; Cloud Code (`v1internal`, the Antigravity adapter) wraps each
/// one in `{response, traceId}` and reports errors through its own envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SseMode {
    Direct,
    /// Built with `subscription-accounts`; the variant is unused otherwise.
    #[cfg_attr(not(feature = "subscription-accounts"), allow(dead_code))]
    CloudCode,
}

pub(super) fn spawn_gemini_sse(
    response: reqwest::Response,
    token: CancellationToken,
    mode: SseMode,
) -> ChunkStream {
    let (tx, stream) = channel_stream();
    tauri::async_runtime::spawn(async move {
        let mut events = response.bytes_stream().eventsource();
        let mut finish = FinishReason::Stop;
        let mut finished_sent = false;
        loop {
            let frame = tokio::select! {
                _ = token.cancelled() => break,
                frame = tokio::time::timeout(STREAM_IDLE_TIMEOUT, events.next()) => match frame {
                    Ok(frame) => frame,
                    Err(_) => {
                        let _ = tx
                            .send(Err(BlueyError::network(
                                "timeout",
                                "the response stream stalled",
                            )))
                            .await;
                        return;
                    }
                },
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
            let data = match mode {
                SseMode::Direct => frame.data,
                SseMode::CloudCode => {
                    // The Antigravity mapper knows the Cloud Code stop signals.
                    if let Some(error) = bluey_protocols::antigravity::map_stream_error(
                        &frame.data,
                        bluey_oauth::unix_now(),
                    ) {
                        let _ = tx.send(Err(error)).await;
                        return;
                    }
                    match bluey_protocols::antigravity::envelope_response(&frame.data) {
                        Ok(Some(inner)) => inner.to_string(),
                        _ => {
                            tracing::debug!(
                                bytes = frame.data.len(),
                                "skipping a cloud code frame without a response"
                            );
                            continue;
                        }
                    }
                }
            };
            // A mid-stream `{"error": …}` payload parses as an empty response;
            // surface it as the error it is (body stays private).
            if mode == SseMode::Direct {
                if let Some(error) = proto::parse_error_body(&data) {
                    let status = error.http_code.unwrap_or(500);
                    let _ = tx
                        .send(Err(proto::map_gemini_error(status, Some(&error))))
                        .await;
                    return;
                }
            }
            let chunk = match proto::parse_response(&data) {
                Ok(chunk) => chunk,
                Err(_) => {
                    tracing::debug!(bytes = data.len(), "skipping an unparseable sse frame");
                    continue;
                }
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
        // Gemini always ends a stream with a `finishReason`; reaching EOF without
        // one means the answer was cut off (proxy, server error) — say so rather
        // than reporting a truncated answer as complete.
        if !finished_sent && !token.is_cancelled() {
            let _ = tx
                .send(Err(BlueyError::network(
                    "stream",
                    "the response stream ended before the model finished",
                )))
                .await;
        }
        let _ = finish;
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
