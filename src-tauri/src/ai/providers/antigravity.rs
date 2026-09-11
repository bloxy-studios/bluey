//! Google AI Pro / Ultra adapter — Cloud Code `v1internal` through the Antigravity
//! OAuth client (ADR 0009 §4c, PR 3c).
//!
//! The body is the Gemini `generateContent` body the API-key adapter builds,
//! normalised for the pool's model line and wrapped by
//! `bluey_protocols::antigravity::AntigravityShaper` into
//! `{model, project, request{…, sessionId}, userAgent, requestType, requestId}`
//! with the thin header set the native client sends. Responses are Gemini chunks
//! inside `{response, traceId}` frames; the Gemini SSE loop unwraps them. Errors
//! go through the Antigravity mapper: a Terms-of-Service 403 stops the account,
//! `QUOTA_EXHAUSTED` is a plan window, capacity gets one short retry. Bearer
//! only, HTTP/1.1 only, the token travels for this one request.

use std::time::Duration;

use bluey_core::types::ModelRole;
use bluey_core::{BlueyError, BlueyResult};
use bluey_oauth::unix_now;
use bluey_protocols::antigravity as ag;
use bluey_protocols::gemini as proto;
use bluey_protocols::request_shaper::{ProviderHttpRequest, RequestShaper, ShapeContext};
use tokio_util::sync::CancellationToken;

use super::gemini::{spawn_gemini_sse, SseMode};
use super::{
    conversation_id, map_transport_error, read_limited, AiProvider, ChunkStream, EmbedPurpose,
    OAuthCredential, ProviderRequest,
};

const HINT: &str = "Google AI";
/// One short retry on capacity / 5xx before the first byte, never past this delay.
const RETRY_CAP: Duration = Duration::from_secs(8);
const MAX_ATTEMPTS: u32 = 2;

pub struct AntigravityProvider {
    http: reqwest::Client,
    credential: OAuthCredential,
    host: String,
}

impl AntigravityProvider {
    /// A Google AI account: the token travels for this one request only.
    pub fn new(credential: OAuthCredential) -> Self {
        Self {
            http: crate::accounts::antigravity::google_http(),
            credential,
            host: bluey_protocols::fingerprints::antigravity::UPSTREAM.to_string(),
        }
    }

    #[cfg(test)]
    fn with_host(mut self, host: &str) -> Self {
        self.host = host.trim_end_matches('/').to_string();
        self
    }

    fn shaped(&self, request: &ProviderRequest) -> BlueyResult<ProviderHttpRequest> {
        let project = self.credential.project_id.clone().ok_or_else(|| {
            BlueyError::account(
                "unavailable",
                "the Google account carries no Cloud Code project — reconnect it",
            )
        })?;
        let thinking = proto::thinking_level_for(
            request.task,
            request.latency,
            request.reasoning,
            &request.model,
        );
        let mut body = proto::build_generate_body(&proto::GenerateBodyOptions {
            model: &request.model,
            messages: &request.messages,
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            output_schema: request.output_schema.as_ref(),
            thinking_level: thinking,
        });
        ag::normalise_request(&mut body, &request.model, request.reasoning);
        let session = conversation_id(request.session_id.as_deref());
        let request_id = uuid::Uuid::new_v4().to_string();
        let mut shaped = ProviderHttpRequest::new("POST", &ag::stream_url(&self.host), body);
        ag::AntigravityShaper::new(
            crate::accounts::antigravity::cached_client_version(),
            ag::arch_label(),
        )
        .shape(
            &mut shaped,
            &ShapeContext {
                account_id: bluey_core::types::ANTIGRAVITY_PROVIDER_ID,
                provider_account_id: Some(&project),
                device_id: &self.credential.device_id,
                session_id: &session,
                request_id: &request_id,
                model: &request.model,
                access_token: Some(&self.credential.access_token),
            },
        )
        .map_err(|e| BlueyError::internal(e.to_string()))?;
        Ok(shaped)
    }

    /// Send the shaped request; one short retry on capacity / 5xx before any
    /// body is consumed. Error bodies are read for the mapper and never logged.
    async fn send(
        &self,
        shaped: &ProviderHttpRequest,
        token: &CancellationToken,
    ) -> BlueyResult<reqwest::Response> {
        let bytes = serde_json::to_vec(&shaped.body)
            .map_err(|_| BlueyError::internal("cannot serialise the Google AI request"))?;
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            if token.is_cancelled() {
                return Err(BlueyError::cancelled());
            }
            let mut builder = self.http.post(&shaped.url);
            for (name, value) in &shaped.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            let sent = tokio::select! {
                _ = token.cancelled() => return Err(BlueyError::cancelled()),
                sent = builder.body(bytes.clone()).send() => sent,
            };
            let response = sent.map_err(|e| map_transport_error(&e, HINT))?;
            let status = response.status().as_u16();
            if status < 400 {
                return Ok(response);
            }
            let body = read_limited(response).await;
            let error = ag::map_error(status, &body, unix_now());
            let retry_after = error
                .details
                .as_ref()
                .and_then(|d| d.get("retryAfterMs"))
                .and_then(|v| v.as_u64())
                .map(Duration::from_millis);
            let retryable = (error.code == "network.http_429"
                && retry_after.is_none_or(|d| d <= RETRY_CAP))
                || error.code == "network.http_5xx";
            if attempt >= MAX_ATTEMPTS || !retryable {
                return Err(error);
            }
            let delay = retry_after.unwrap_or(Duration::from_millis(800));
            tracing::info!(
                status,
                attempt,
                delay_ms = delay.as_millis() as u64,
                "google ai request failed; retrying once"
            );
            tokio::select! {
                _ = token.cancelled() => return Err(BlueyError::cancelled()),
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }
}

#[async_trait::async_trait]
impl AiProvider for AntigravityProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        token: CancellationToken,
    ) -> BlueyResult<ChunkStream> {
        let shaped = self.shaped(request)?;
        let response = self.send(&shaped, &token).await?;
        Ok(spawn_gemini_sse(response, token, SseMode::CloudCode))
    }

    async fn embed(
        &self,
        _model: &str,
        _texts: &[String],
        _purpose: &EmbedPurpose,
    ) -> BlueyResult<Vec<Vec<f32>>> {
        Err(BlueyError::not_supported(
            "embed",
            "a Google AI subscription cannot embed text; assign the embedding role to Google Gemini",
        ))
    }

    async fn list_models(&self, role: Option<ModelRole>) -> BlueyResult<Vec<String>> {
        let catalog = self.credential.catalog.as_ref().ok_or_else(|| {
            BlueyError::account(
                "not_connected",
                "connect the Google AI account and refresh its models first",
            )
        })?;
        if matches!(
            role,
            Some(ModelRole::Embedding) | Some(ModelRole::Transcription)
        ) {
            return Ok(Vec::new());
        }
        Ok(catalog.models.iter().map(|m| m.id.clone()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::super::collect_text;
    use super::*;
    use bluey_core::types::{AiMessage, AiRole, AiTask, LatencyBudget, ReasoningLevel};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A one-shot HTTP/1.1 stub: records the request, answers with `response`.
    async fn stub(response: &'static str) -> (String, tokio::sync::oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 65536];
            let mut collected = Vec::new();
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                collected.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&collected).into_owned();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let length = text[..head_end]
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if collected.len() >= head_end + 4 + length {
                        let _ = tx.send(text);
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.ok();
        });
        (format!("http://127.0.0.1:{port}"), rx)
    }

    fn credential() -> OAuthCredential {
        OAuthCredential {
            access_token: "ya29.SECRET".into(),
            account_id: Some("1029".into()),
            catalog: None,
            device_id: "d".repeat(64),
            project_id: Some("proj-123".into()),
        }
    }

    fn request() -> ProviderRequest {
        ProviderRequest {
            model: "gemini-3.8-flash-high".into(),
            messages: vec![
                AiMessage::text(AiRole::System, "You are Bluey."),
                AiMessage::text(AiRole::User, "hi"),
            ],
            max_output_tokens: Some(400),
            temperature: None,
            output_schema: None,
            task: AiTask::Answer,
            latency: LatencyBudget::Fast,
            reasoning: ReasoningLevel::None,
            session_id: Some("session-1".into()),
        }
    }

    #[tokio::test]
    async fn the_shaped_request_streams_from_the_cloud_code_envelope() {
        let (base, seen) = stub(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n\
             data: {\"response\":{\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hel\"}]}}]},\"traceId\":\"t1\"}\r\n\r\n\
             data: {\"response\":{\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"lo\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":1}},\"traceId\":\"t1\"}\r\n\r\n",
        )
        .await;
        let provider = AntigravityProvider::new(credential()).with_host(&base);
        let stream = provider
            .stream(&request(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(collect_text(stream).await.unwrap(), "Hello");
        let raw = seen.await.unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        let head = head.to_ascii_lowercase();
        assert!(
            head.starts_with("post /v1internal:streamgeneratecontent?alt=sse http/1.1"),
            "{head}"
        );
        assert!(head.contains("authorization: bearer ya29.secret"));
        assert!(head.contains("user-agent: antigravity/hub/"));
        assert!(!head.contains("x-goog-api-client"), "{head}");
        assert!(!head.contains("x-api-key"));
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["model"], "gemini-3.8-flash-high");
        assert_eq!(json["project"], "proj-123");
        assert_eq!(json["userAgent"], "antigravity");
        assert_eq!(json["requestType"], "agent");
        assert!(json["requestId"].as_str().unwrap().starts_with("agent-"));
        assert!(json["request"]["sessionId"]
            .as_str()
            .unwrap()
            .starts_with('-'));
        assert_eq!(json["request"]["systemInstruction"]["role"], "user");
        assert_eq!(json["request"]["contents"][0]["parts"][0]["text"], "hi");
        assert_eq!(json["request"]["generationConfig"]["maxOutputTokens"], 400);
        assert_eq!(
            json["request"]["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "low"
        );
    }

    #[tokio::test]
    async fn a_terms_of_service_403_stops_the_account() {
        let body = r#"{"error":{"code":403,"message":"This service has been disabled in this account for violation of Terms of Service. If you believe this is an error, contact gemini-code-assist-user-feedback@google.com.","status":"PERMISSION_DENIED"}}"#;
        let (base, _seen) = stub(Box::leak(
            format!(
                "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        ))
        .await;
        let provider = AntigravityProvider::new(credential()).with_host(&base);
        let error = provider
            .stream(&request(), CancellationToken::new())
            .await
            .err()
            .expect("the 403 is an error");
        assert_eq!(error.code, "account.policy_blocked");
        assert!(error
            .message
            .contains("gemini-code-assist-user-feedback@google.com"));
        assert_eq!(
            bluey_core::accounts::status_after_error(&error).map(|s| s.state_name()),
            Some("unavailable")
        );
    }
}
