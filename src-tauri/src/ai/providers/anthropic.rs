//! Anthropic Messages API provider (`POST {base}/v1/messages`, `x-api-key`,
//! `anthropic-version: 2023-06-01`). Structured output goes through
//! `output_config`; a 400 rejection falls back to instructing the schema in
//! the prompt.
//!
//! Works unchanged against Claude in Microsoft Foundry: set the provider's
//! base URL to `https://{resource}.services.ai.azure.com/anthropic` and use
//! the Foundry resource key (Foundry accepts `x-api-key`); `model` is then the
//! Foundry deployment name (defaults to the model id, e.g. `claude-opus-5`).
//!
//! **OAuth mode** (ADR 0009 §4b, a Claude Pro/Max account): the same body,
//! `Authorization: Bearer` instead of `x-api-key`, `POST /v1/messages?beta=true`,
//! and `bluey_protocols::claude_code::ClaudeCodeShaper` applied last — the two
//! fingerprint `system` blocks, Bluey's prompt as a `<system-reminder>`,
//! `metadata.user_id`, the betas. Errors go through the Claude mapper, whose
//! extra-usage guard turns a "billed outside the plan" answer into a stop.

use bluey_core::types::{FinishReason, ModelRole, CLAUDE_PROVIDER_ID};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::anthropic as proto;
use bluey_protocols::claude_code;
use bluey_protocols::request_shaper::{ProviderHttpRequest, RequestShaper, ShapeContext};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::EmbedPurpose;
use super::{
    channel_stream, conversation_id, header_pairs, map_http_status, map_transport_error,
    read_limited, AiProvider, ChunkStream, OAuthCredential, ProviderRequest, StreamItem,
};

enum Auth {
    ApiKey(String),
    Oauth(OAuthCredential),
}

pub struct AnthropicProvider {
    http: reqwest::Client,
    base_url: String,
    auth: Auth,
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
            auth: Auth::ApiKey(api_key),
        }
    }

    /// A Claude Pro/Max account: the token travels for this one request only.
    pub fn oauth(http: reqwest::Client, credential: OAuthCredential) -> Self {
        Self {
            http,
            base_url: bluey_protocols::fingerprints::claude_code::UPSTREAM.to_string(),
            auth: Auth::Oauth(credential),
        }
    }

    #[cfg(test)]
    fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    fn is_oauth(&self) -> bool {
        matches!(self.auth, Auth::Oauth(_))
    }

    async fn send_messages(
        &self,
        request: &ProviderRequest,
        schema_as_prompt: bool,
    ) -> Result<reqwest::Response, BlueyError> {
        let mut body = proto::build_messages_body(&proto::MessagesBodyOptions {
            model: &request.model,
            messages: &request.messages,
            stream: true,
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            output_schema: request.output_schema.as_ref(),
            schema_as_prompt_fallback: schema_as_prompt,
        });
        match &self.auth {
            Auth::ApiKey(api_key) => self
                .http
                .post(proto::messages_url(&self.base_url))
                .header("x-api-key", api_key)
                .header("anthropic-version", proto::ANTHROPIC_VERSION)
                .header("accept", "text/event-stream")
                .json(&body)
                .send()
                .await
                .map_err(|e| map_transport_error(&e, "Anthropic")),
            Auth::Oauth(credential) => {
                claude_code::apply_thinking(
                    &mut body,
                    &request.model,
                    request.reasoning,
                    request.latency,
                );
                claude_code::normalise_max_tokens(
                    &mut body,
                    &request.model,
                    request.max_output_tokens,
                );
                let account = credential.account_id.clone().ok_or_else(|| {
                    BlueyError::account(
                        "not_connected",
                        "the Claude sign-in carries no account id — reconnect the account",
                    )
                })?;
                let session = conversation_id(request.session_id.as_deref());
                let request_id = uuid::Uuid::new_v4().to_string();
                let mut shaped = ProviderHttpRequest::new(
                    "POST",
                    &claude_code::messages_url(&self.base_url),
                    body,
                );
                claude_code::ClaudeCodeShaper
                    .shape(
                        &mut shaped,
                        &ShapeContext {
                            account_id: CLAUDE_PROVIDER_ID,
                            provider_account_id: Some(&account),
                            device_id: &credential.device_id,
                            session_id: &session,
                            request_id: &request_id,
                            model: &request.model,
                            access_token: Some(&credential.access_token),
                        },
                    )
                    .map_err(|e| BlueyError::internal(e.to_string()))?;
                let mut builder = self.http.post(&shaped.url);
                for (name, value) in &shaped.headers {
                    builder = builder.header(name.as_str(), value.as_str());
                }
                let bytes = serde_json::to_vec(&shaped.body)
                    .map_err(|_| BlueyError::internal("cannot serialise the Claude request"))?;
                builder
                    .body(bytes)
                    .send()
                    .await
                    .map_err(|e| map_transport_error(&e, "Claude"))
            }
        }
    }

    /// Map a non-2xx response; the body is read only in OAuth mode (the Claude
    /// mapper needs the provider's words) and never logged.
    async fn error_from(&self, response: reqwest::Response) -> BlueyError {
        let status = response.status().as_u16();
        match &self.auth {
            Auth::ApiKey(_) => map_http_status(status, "Anthropic"),
            Auth::Oauth(_) => {
                let headers = header_pairs(response.headers());
                let body = read_limited(response).await;
                claude_code::map_error(status, &headers, &body)
            }
        }
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
            let headers = header_pairs(response.headers());
            let body = response.text().await.unwrap_or_default();
            if self.is_oauth() && claude_code::drift_reason(status, &body, &headers).is_some() {
                // The extra-usage guard outranks the schema retry.
                return Err(claude_code::map_error(status, &headers, &body));
            }
            if proto::is_output_config_rejection(status, &body) {
                tracing::info!("anthropic rejected output_config; retrying with schema-in-prompt");
                response = self.send_messages(request, true).await?;
                status = response.status().as_u16();
            } else if self.is_oauth() {
                return Err(claude_code::map_error(status, &headers, &body));
            } else {
                return Err(map_http_status(status, "Anthropic"));
            }
        }
        if status >= 400 {
            return Err(self.error_from(response).await);
        }
        Ok(spawn_anthropic_sse(response, token, self.is_oauth()))
    }

    async fn embed(
        &self,
        _model: &str,
        _texts: &[String],
        _purpose: &EmbedPurpose,
    ) -> BlueyResult<Vec<Vec<f32>>> {
        Err(BlueyError::not_supported(
            "embeddings",
            "the Anthropic API does not serve embeddings",
        ))
    }

    async fn list_models(&self, role: Option<ModelRole>) -> BlueyResult<Vec<String>> {
        if let Auth::Oauth(credential) = &self.auth {
            let catalog = credential.catalog.as_ref().ok_or_else(|| {
                BlueyError::account(
                    "not_connected",
                    "connect the Claude account and refresh its models first",
                )
            })?;
            if matches!(
                role,
                Some(ModelRole::Embedding) | Some(ModelRole::Transcription)
            ) {
                return Ok(Vec::new());
            }
            return Ok(catalog.models.iter().map(|m| m.id.clone()).collect());
        }
        let Auth::ApiKey(api_key) = &self.auth else {
            unreachable!("handled above");
        };
        let response = self
            .http
            .get(proto::models_url(&self.base_url))
            .header("x-api-key", api_key)
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

fn spawn_anthropic_sse(
    response: reqwest::Response,
    token: CancellationToken,
    oauth: bool,
) -> ChunkStream {
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
            let event_name = if frame.event.is_empty() {
                "message"
            } else {
                frame.event.as_str()
            };
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
                    let error = if oauth {
                        claude_code::map_stream_error(&error_type, &message)
                    } else if error_type == "overloaded_error" {
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

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::types::{AiMessage, AiRole, AiTask, LatencyBudget, ReasoningLevel};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A stub backend: one request, one canned response; the raw head and body come back.
    async fn stub(response: String) -> (String, tokio::sync::oneshot::Receiver<(String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            let (head, body) = loop {
                let n = socket.read(&mut chunk).await.unwrap();
                buf.extend_from_slice(&chunk[..n]);
                if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..end]).to_string();
                    let length: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    let mut body = buf[end + 4..].to_vec();
                    while body.len() < length {
                        let n = socket.read(&mut chunk).await.unwrap();
                        body.extend_from_slice(&chunk[..n]);
                    }
                    break (head, String::from_utf8_lossy(&body).to_string());
                }
                if n == 0 {
                    break (String::new(), String::new());
                }
            };
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = socket.shutdown().await;
            let _ = tx.send((head, body));
        });
        (format!("http://{addr}"), rx)
    }

    fn provider(base: &str) -> AnthropicProvider {
        AnthropicProvider::oauth(
            reqwest::Client::new(),
            OAuthCredential {
                access_token: "sk-ant-oat01-SECRETSECRETSECRET".into(),
                account_id: Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e".into()),
                catalog: None,
                device_id: "a".repeat(64),
            },
        )
        .with_base_url(base)
    }

    fn request() -> ProviderRequest {
        ProviderRequest {
            model: "claude-sonnet-5".into(),
            messages: vec![
                AiMessage::text(AiRole::System, "You are Bluey."),
                AiMessage::text(AiRole::User, "hello"),
            ],
            max_output_tokens: None,
            temperature: Some(0.2),
            output_schema: None,
            task: AiTask::Answer,
            latency: LatencyBudget::Fast,
            reasoning: ReasoningLevel::None,
            session_id: Some("session-1".into()),
        }
    }

    #[tokio::test]
    async fn streams_a_claude_code_shaped_request_over_oauth() {
        let sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n\
event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nanthropic-ratelimit-unified-status: allowed\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{sse}",
            sse.len()
        );
        let (base, seen) = stub(response).await;
        let stream = provider(&base)
            .stream(&request(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(super::super::collect_text(stream).await.unwrap(), "Hello");

        let (head, body) = seen.await.unwrap();
        let lower = head.to_ascii_lowercase();
        assert!(
            lower.contains("post /v1/messages?beta=true http/1.1"),
            "{head}"
        );
        assert!(lower.contains("authorization: bearer sk-ant-oat01-secretsecretsecret"));
        assert!(!lower.contains("x-api-key"), "OAuth is Bearer-only");
        assert!(lower.contains("user-agent: claude-cli/2.1.258 (external, cli)"));
        assert!(lower.contains("x-app: cli"));
        assert!(lower.contains("anthropic-beta: claude-code-20250219,oauth-2025-04-20"));
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            body["system"][1]["text"],
            "You are Claude Code, Anthropic's official CLI for Claude."
        );
        assert!(body["system"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("x-anthropic-billing-header: cc_version=2.1.258."));
        assert!(body["messages"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("<system-reminder>\nYou are Bluey.\n</system-reminder>\n\nhello"));
        assert_eq!(body["max_tokens"], 128_000);
        assert!(body.get("thinking").is_none(), "no reasoning requested");
        assert!(body["metadata"]["user_id"]
            .as_str()
            .unwrap()
            .starts_with("{\"device_id\":\"aaaa"));
    }

    #[tokio::test]
    async fn the_extra_usage_answer_stops_the_account() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Third-party apps now draw from your extra usage, not your plan limits. Add more at claude.ai/settings/usage and keep going."}}"#;
        let response = format!(
            "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let (base, _seen) = stub(response).await;
        let error = provider(&base)
            .stream(&request(), CancellationToken::new())
            .await
            .err()
            .expect("the extra-usage 400 is an error");
        assert_eq!(error.code, "account.extra_usage_blocked");
        assert_eq!(
            error.recovery,
            Some(bluey_core::error::RecoveryAction::UseApiKey)
        );
    }
}
