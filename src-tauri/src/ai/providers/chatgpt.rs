//! ChatGPT (Codex) adapter: the Responses API at `chatgpt.com/backend-api/codex`
//! with the account's OAuth token, shaped like the Codex CLI (ADR 0009 §4).
//!
//! The adapter holds the access token only for the request it was built for
//! (`AiManager::adapter_for` builds one per call from `AccountsManager::credential_for`).
//! The pure parts — body, SSE events, error mapping, the shaper — are
//! `bluey_protocols::codex`; this file is the HTTP around them.

use std::time::Duration;

use bluey_core::types::{ModelRole, CHATGPT_PROVIDER_ID};
use bluey_core::{BlueyError, BlueyResult};
use bluey_oauth::unix_now;
use bluey_protocols::codex;
use bluey_protocols::request_shaper::{ProviderHttpRequest, RequestShaper, ShapeContext};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::{
    channel_stream, conversation_id, header_pairs, map_transport_error, read_limited, AiProvider,
    ChunkStream, EmbedPurpose, OAuthCredential, ProviderRequest, StreamItem,
};
use crate::accounts::chatgpt::{client_environment, send_shaped, templates};

/// A stream that goes quiet for this long is reported as stalled.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const CATALOG_TIMEOUT: Duration = Duration::from_secs(20);

pub struct ChatgptProvider {
    http: reqwest::Client,
    credential: OAuthCredential,
    shaper: codex::CodexShaper,
    /// `https://chatgpt.com/backend-api/codex`; tests point it at a stub.
    base_url: String,
}

impl ChatgptProvider {
    pub fn new(http: reqwest::Client, credential: OAuthCredential) -> Self {
        let fedramp = codex::identity_from_tokens(None, &credential.access_token).fedramp;
        Self {
            http,
            shaper: codex::CodexShaper {
                env: client_environment(),
                fedramp,
            },
            credential,
            base_url: codex::backend_base(),
        }
    }

    #[cfg(test)]
    fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    fn account_id(&self) -> BlueyResult<String> {
        self.credential
            .account_id
            .clone()
            .or_else(|| codex::identity_from_tokens(None, &self.credential.access_token).account_id)
            .ok_or_else(|| {
                BlueyError::account(
                    "not_connected",
                    "the ChatGPT sign-in carries no account id — reconnect the account",
                )
            })
    }

    fn context<'a>(
        &'a self,
        account_id: &'a str,
        session_id: &'a str,
        request_id: &'a str,
        model: &'a str,
    ) -> ShapeContext<'a> {
        ShapeContext {
            account_id: CHATGPT_PROVIDER_ID,
            provider_account_id: Some(account_id),
            device_id: "",
            session_id,
            request_id,
            model,
            access_token: Some(&self.credential.access_token),
        }
    }

    /// The model's `instructions_template` from the catalog. The profile fills
    /// the cache on every catalog fetch; the first request after a restart
    /// fetches the catalog once itself.
    async fn ensure_template(&self, model: &str) -> Option<String> {
        if let Some(template) = templates().get(model) {
            return Some(template);
        }
        if !templates().is_empty() {
            return None; // catalog known, this model ships no template
        }
        let account_id = self.account_id().ok()?;
        let session_id = conversation_id(None);
        let request_id = uuid::Uuid::new_v4().to_string();
        let mut request = codex::models_request(
            &self.shaper,
            &self.context(&account_id, &session_id, &request_id, model),
        )
        .ok()?;
        request.url = request.url.replace(&codex::backend_base(), &self.base_url);
        let (status, _, body) = send_shaped(
            &self.http,
            &request,
            "the ChatGPT model catalog",
            CATALOG_TIMEOUT,
        )
        .await
        .ok()?;
        if status >= 400 {
            return None;
        }
        let response = codex::parse_models(&body).ok()?;
        templates().replace(codex::instructions_templates(&response));
        templates().get(model)
    }
}

#[async_trait::async_trait]
impl AiProvider for ChatgptProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        token: CancellationToken,
    ) -> BlueyResult<ChunkStream> {
        let account_id = self.account_id()?;
        let conversation = conversation_id(request.session_id.as_deref());
        let request_id = uuid::Uuid::new_v4().to_string();
        let levels = self
            .credential
            .catalog
            .as_ref()
            .map(|catalog| codex::model_reasoning(&catalog.models, &request.model))
            .unwrap_or_default();
        let effort = codex::reasoning_effort(request.reasoning, request.latency, &levels, None);
        let template = self.ensure_template(&request.model).await;
        let body = codex::build_responses_body(&codex::ResponsesBodyOptions {
            model: &request.model,
            messages: &request.messages,
            instructions: match template.as_deref() {
                Some(template) => codex::InstructionsPolicy::Template(template),
                None => codex::InstructionsPolicy::Own,
            },
            effort: &effort,
            verbosity: codex::verbosity_for(request.latency),
            prompt_cache_key: &conversation,
            output_schema: request.output_schema.as_ref(),
            image_detail: codex::DEFAULT_IMAGE_DETAIL,
        });
        let mut shaped =
            ProviderHttpRequest::new("POST", &format!("{}/responses", self.base_url), body);
        self.shaper
            .shape(
                &mut shaped,
                &self.context(&account_id, &conversation, &request_id, &request.model),
            )
            .map_err(|e| BlueyError::internal(e.to_string()))?;
        let mut builder = self.http.post(&shaped.url);
        for (name, value) in &shaped.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let bytes = serde_json::to_vec(&shaped.body)
            .map_err(|_| BlueyError::internal("cannot serialise the ChatGPT request"))?;
        let response = builder
            .body(bytes)
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "ChatGPT"))?;
        let status = response.status().as_u16();
        if status >= 400 {
            let headers = header_pairs(response.headers());
            let body = read_limited(response).await;
            return Err(codex::map_error(
                status,
                &headers,
                &body,
                codex::Endpoint::Responses,
                unix_now(),
            ));
        }
        Ok(spawn_codex_sse(response, token))
    }

    async fn embed(
        &self,
        _model: &str,
        _texts: &[String],
        _purpose: &EmbedPurpose,
    ) -> BlueyResult<Vec<Vec<f32>>> {
        Err(BlueyError::not_supported(
            "embed",
            "a ChatGPT subscription cannot embed documents; assign the embedding role to Google Gemini",
        ))
    }

    async fn list_models(&self, role: Option<ModelRole>) -> BlueyResult<Vec<String>> {
        let catalog = self.credential.catalog.as_ref().ok_or_else(|| {
            BlueyError::account(
                "not_connected",
                "connect the ChatGPT account and refresh its models first",
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

/// Consume the `/responses` SSE stream on a task, forwarding items.
fn spawn_codex_sse(response: reqwest::Response, token: CancellationToken) -> ChunkStream {
    let (tx, stream) = channel_stream();
    tauri::async_runtime::spawn(async move {
        let mut events = response.bytes_stream().eventsource();
        let mut state = codex::StreamState::default();
        loop {
            let frame = tokio::select! {
                _ = token.cancelled() => return,
                frame = tokio::time::timeout(STREAM_IDLE_TIMEOUT, events.next()) => match frame {
                    Ok(frame) => frame,
                    Err(_) => {
                        let _ = tx
                            .send(Err(BlueyError::network("timeout", "the ChatGPT response stalled")))
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
                            "the ChatGPT response stream ended unexpectedly",
                        )))
                        .await;
                    return;
                }
            };
            if frame.data.trim().is_empty() || bluey_protocols::sse::is_done(&frame.data) {
                continue;
            }
            let Ok(event) = codex::parse_event(&frame.data) else {
                continue; // tolerate unknown frames
            };
            for item in state.on_event(event) {
                let sent = match item {
                    codex::StreamItem::Delta(text) => tx.send(Ok(StreamItem::Delta(text))).await,
                    codex::StreamItem::Usage(usage) => {
                        tx.send(Ok(StreamItem::Usage {
                            input: usage.input_tokens,
                            output: usage.output_tokens,
                        }))
                        .await
                    }
                    codex::StreamItem::Finished(reason) => {
                        tx.send(Ok(StreamItem::Finished(reason))).await
                    }
                    codex::StreamItem::Failed(error) => tx.send(Err(error)).await,
                };
                if sent.is_err() {
                    return;
                }
            }
            if state.is_finished() {
                return;
            }
        }
        if let Some(codex::StreamItem::Failed(error)) = state.on_end() {
            let _ = tx.send(Err(error)).await;
        }
    });
    stream
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::types::{AiMessage, AiRole, AiTask, LatencyBudget, ReasoningLevel};
    use bluey_protocols::oauth::base64url;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn access_token() -> String {
        let header = base64url(br#"{"alg":"RS256"}"#);
        let payload = base64url(
            br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"9d1c250a-e61b-44d9-88ed-5944d1962f5e","chatgpt_plan_type":"plus"}}"#,
        );
        format!("{header}.{payload}.sig")
    }

    /// A stub backend: one request, one canned response; the raw request head
    /// and body are handed back for assertions.
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

    fn provider(base: &str) -> ChatgptProvider {
        templates().replace(vec![("gpt-6-astra".into(), "You are Codex.".into())]);
        ChatgptProvider::new(
            reqwest::Client::new(),
            OAuthCredential {
                access_token: access_token(),
                account_id: Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e".into()),
                catalog: None,
                device_id: String::new(),
            },
        )
        .with_base_url(base)
    }

    fn request() -> ProviderRequest {
        ProviderRequest {
            model: "gpt-6-astra".into(),
            messages: vec![
                AiMessage::text(AiRole::System, "You are Bluey."),
                AiMessage::text(AiRole::User, "hello"),
            ],
            max_output_tokens: Some(64),
            temperature: Some(0.0),
            output_schema: None,
            task: AiTask::Answer,
            latency: LatencyBudget::Fast,
            reasoning: ReasoningLevel::None,
            session_id: Some("session-1".into()),
        }
    }

    #[tokio::test]
    async fn streams_a_codex_response_shaped_like_the_cli() {
        let sse = "event: response.created\ndata: {\"type\":\"response.created\"}\n\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n\
data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":2}}}\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{sse}",
            sse.len()
        );
        let (base, seen) = stub(response).await;
        let provider = provider(&base);
        let stream = provider
            .stream(&request(), CancellationToken::new())
            .await
            .unwrap();
        let text = super::super::collect_text(stream).await.unwrap();
        assert_eq!(text, "Hello");

        let (head, body) = seen.await.unwrap();
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains("post /responses http/1.1"), "{head}");
        assert!(lower.contains(&format!(
            "authorization: bearer {}",
            access_token().to_ascii_lowercase()
        )));
        assert!(lower.contains("chatgpt-account-id: 9d1c250a-e61b-44d9-88ed-5944d1962f5e"));
        assert!(lower.contains("originator: codex_cli_rs"));
        assert!(lower.contains("accept: text/event-stream"));
        assert!(
            !lower.contains("openai-beta"),
            "the CLI sends no OpenAI-Beta header"
        );
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["model"], "gpt-6-astra");
        assert_eq!(body["instructions"], "You are Codex.");
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["input"][1]["content"][0]["text"], "hello");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("temperature").is_none());
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(
            body["prompt_cache_key"],
            conversation_id(Some("session-1")),
            "the cache key is stable per session"
        );
    }

    #[tokio::test]
    async fn a_plan_limit_maps_to_the_rate_limited_account_error() {
        let body =
            r#"{"error":{"type":"usage_limit_reached","plan_type":"plus","resets_at":1700007200}}"#;
        let response = format!(
            "HTTP/1.1 429 Too Many Requests\r\ncontent-type: application/json\r\nx-codex-primary-window-minutes: 300\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let (base, _seen) = stub(response).await;
        let provider = provider(&base);
        let error = provider
            .stream(&request(), CancellationToken::new())
            .await
            .err()
            .expect("a 429 is an error");
        assert_eq!(error.code, "account.rate_limited");
        let details = error.details.unwrap();
        assert_eq!(details["until"], "2023-11-15T00:13:20Z");
        assert_eq!(details["window"], "5h");
    }

    #[test]
    fn conversation_ids_are_stable_per_session_and_distinct_across_sessions() {
        let a = conversation_id(Some("s-a"));
        assert_eq!(conversation_id(Some("s-a")), a);
        assert_ne!(conversation_id(Some("s-b")), a);
        assert_eq!(conversation_id(None), conversation_id(None));
        assert_eq!(
            uuid::Uuid::parse_str(&a).map(|u| u.get_version_num()),
            Ok(4)
        );
    }
}
