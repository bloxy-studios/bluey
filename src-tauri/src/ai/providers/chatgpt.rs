//! ChatGPT (Codex) adapter: the Responses API at `chatgpt.com/backend-api/codex`
//! with the account's OAuth token, shaped like the Codex CLI (ADR 0009 §4).
//!
//! The adapter holds the access token only for the request it was built for
//! (`AiManager::adapter_for` builds one per call from `AccountsManager::credential_for`).
//! The pure parts — body, SSE events, error mapping, the shaper — are
//! `bluey_protocols::codex`; this file is the HTTP around them.
//!
//! `instructions`: Bluey's own prompt is sent as `instructions`
//! (`InstructionsPolicy::Own`) so the model is primed by Bluey, not by the
//! 13–21 KB Codex CLI agent prompt. The backend's acceptance of that shape is
//! the open replay of docs/PROVIDER_ACCOUNTS.md; it is probed in production:
//! a 400 on the own-prompt shape is retried once with the catalog template
//! (Bluey's prompt as a `developer` item), and a stream that finishes without
//! a single text delta switches the next request to the template. Either
//! switch is sticky for the process and logged once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
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

/// Whether the Codex backend has refused Bluey's own prompt as `instructions`
/// in this process (module docs). Shared by every adapter instance, since one
/// is built per request.
static OWN_INSTRUCTIONS_REJECTED: LazyLock<Arc<AtomicBool>> =
    LazyLock::new(|| Arc::new(AtomicBool::new(false)));

/// What one `/responses` POST came back with, before any SSE is read.
enum Sent {
    Stream(ChunkStream),
    /// An HTTP error status; `error` is the mapped `BlueyError`.
    Rejected {
        status: u16,
        error: BlueyError,
    },
}

/// `Own` unless the fallback is in force and a template exists to fall back to.
fn instructions_policy(own: bool, template: Option<&str>) -> codex::InstructionsPolicy<'_> {
    match (own, template) {
        (false, Some(template)) => codex::InstructionsPolicy::Template(template),
        _ => codex::InstructionsPolicy::Own,
    }
}

pub struct ChatgptProvider {
    http: reqwest::Client,
    credential: OAuthCredential,
    shaper: codex::CodexShaper,
    /// `https://chatgpt.com/backend-api/codex`; tests point it at a stub.
    base_url: String,
    /// The process-wide own-instructions verdict (tests get a fresh one).
    own_instructions_rejected: Arc<AtomicBool>,
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
            own_instructions_rejected: Arc::clone(&OWN_INSTRUCTIONS_REJECTED),
        }
    }

    #[cfg(test)]
    fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    /// Tests share a process; each gets its own verdict.
    #[cfg(test)]
    fn with_fresh_instructions_verdict(mut self) -> Self {
        self.own_instructions_rejected = Arc::new(AtomicBool::new(false));
        self
    }

    /// Whether this process has seen the backend refuse Bluey's prompt as `instructions`.
    pub fn own_instructions_rejected(&self) -> bool {
        self.own_instructions_rejected.load(Ordering::Relaxed)
    }

    fn reject_own_instructions(flag: &AtomicBool, reason: &str) {
        if !flag.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                reason,
                "the Codex backend did not accept Bluey's prompt as `instructions`; sending the catalog template with Bluey's prompt as a developer item from now on"
            );
        }
    }

    /// Build, shape and send one `/responses` request.
    async fn send_responses(
        &self,
        request: &ProviderRequest,
        account_id: &str,
        conversation: &str,
        effort: &str,
        instructions: codex::InstructionsPolicy<'_>,
        token: &CancellationToken,
    ) -> BlueyResult<Sent> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let body = codex::build_responses_body(&codex::ResponsesBodyOptions {
            model: &request.model,
            messages: &request.messages,
            instructions,
            effort,
            verbosity: codex::verbosity_for(request.latency),
            prompt_cache_key: conversation,
            output_schema: request.output_schema.as_ref(),
            image_detail: codex::DEFAULT_IMAGE_DETAIL,
        });
        let mut shaped =
            ProviderHttpRequest::new("POST", &format!("{}/responses", self.base_url), body);
        self.shaper
            .shape(
                &mut shaped,
                &self.context(account_id, conversation, &request_id, &request.model),
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
            return Ok(Sent::Rejected {
                status,
                error: codex::map_error(
                    status,
                    &headers,
                    &body,
                    codex::Endpoint::Responses,
                    unix_now(),
                ),
            });
        }
        Ok(Sent::Stream(spawn_codex_sse(response, token.clone())))
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
        let levels = self
            .credential
            .catalog
            .as_ref()
            .map(|catalog| codex::model_reasoning(&catalog.models, &request.model))
            .unwrap_or_default();
        let effort = codex::reasoning_effort(request.reasoning, request.latency, &levels, None);
        let template = self.ensure_template(&request.model).await;
        // Bluey's prompt as `instructions` first (module docs); the catalog
        // template is the fallback, so without one `Own` is the only shape.
        let can_fall_back = template.is_some();
        let own_first = !self.own_instructions_rejected() || !can_fall_back;
        let sent = self
            .send_responses(
                request,
                &account_id,
                &conversation,
                &effort,
                instructions_policy(own_first, template.as_deref()),
                &token,
            )
            .await?;
        match sent {
            Sent::Stream(stream) if own_first && can_fall_back => Ok(watch_for_empty_output(
                stream,
                Arc::clone(&self.own_instructions_rejected),
            )),
            Sent::Stream(stream) => Ok(stream),
            Sent::Rejected { status: 400, .. } if own_first && can_fall_back => {
                Self::reject_own_instructions(&self.own_instructions_rejected, "http 400");
                match self
                    .send_responses(
                        request,
                        &account_id,
                        &conversation,
                        &effort,
                        instructions_policy(false, template.as_deref()),
                        &token,
                    )
                    .await?
                {
                    Sent::Stream(stream) => Ok(stream),
                    Sent::Rejected { error, .. } => Err(error),
                }
            }
            Sent::Rejected { error, .. } => Err(error),
        }
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

/// Flip the shared verdict when a stream on the own-instructions shape
/// finishes without a single text delta — the quiet way the backend can
/// refuse Bluey's prompt as `instructions`. This reply still flows through
/// unchanged (the WebView shows its failure state); the next request carries
/// the template.
fn watch_for_empty_output(stream: ChunkStream, rejected: Arc<AtomicBool>) -> ChunkStream {
    let mut saw_text = false;
    Box::pin(stream.inspect(move |item| match item {
        Ok(StreamItem::Delta(text)) if !text.trim().is_empty() => saw_text = true,
        Ok(StreamItem::Finished(_)) if !saw_text => {
            ChatgptProvider::reject_own_instructions(&rejected, "empty output");
        }
        _ => {}
    }))
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

    /// Read one HTTP request — head plus `content-length` body — off the socket.
    async fn read_request(socket: &mut tokio::net::TcpStream) -> (String, String) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
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
                return (head, String::from_utf8_lossy(&body).to_string());
            }
            if n == 0 {
                return (String::new(), String::new());
            }
        }
    }

    /// A stub backend: one canned response per connection, in order; every raw
    /// request head and body is handed back once all have been served.
    async fn stub_many(
        responses: Vec<String>,
    ) -> (
        String,
        tokio::sync::oneshot::Receiver<Vec<(String, String)>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut seen = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                socket.write_all(response.as_bytes()).await.unwrap();
                let _ = socket.shutdown().await;
                seen.push(request);
            }
            let _ = tx.send(seen);
        });
        (format!("http://{addr}"), rx)
    }

    /// A stub backend: one request, one canned response; the raw request head
    /// and body are handed back for assertions.
    async fn stub(response: String) -> (String, tokio::sync::oneshot::Receiver<(String, String)>) {
        let (base, seen) = stub_many(vec![response]).await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Ok(mut seen) = seen.await {
                if let Some(first) = seen.pop() {
                    let _ = tx.send(first);
                }
            }
        });
        (base, rx)
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
                project_id: None,
            },
        )
        .with_base_url(base)
        .with_fresh_instructions_verdict()
    }

    const HELLO_SSE: &str = "event: response.created\ndata: {\"type\":\"response.created\"}\n\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n\
data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":2}}}\n\n";

    fn ok_response(sse: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{sse}",
            sse.len()
        )
    }

    fn rejected_response() -> String {
        let body = r#"{"error":{"message":"Invalid value for 'instructions'","type":"invalid_request_error"}}"#;
        format!(
            "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
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
    async fn streams_a_codex_response_shaped_like_the_cli_with_bluey_as_instructions() {
        let (base, seen) = stub(ok_response(HELLO_SSE)).await;
        let provider = provider(&base);
        let stream = provider
            .stream(&request(), CancellationToken::new())
            .await
            .unwrap();
        let text = super::super::collect_text(stream).await.unwrap();
        assert_eq!(text, "Hello");
        assert!(
            !provider.own_instructions_rejected(),
            "a stream with text keeps the own-instructions shape"
        );

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
        // Bluey's system prompt primes the model; the Codex agent template does not.
        assert_eq!(body["instructions"], "You are Bluey.");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][0]["text"], "hello");
        assert_eq!(body["input"].as_array().map(Vec::len), Some(1));
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
    async fn a_400_on_bluey_instructions_retries_once_with_the_codex_template() {
        let (base, seen) = stub_many(vec![rejected_response(), ok_response(HELLO_SSE)]).await;
        let provider = provider(&base);
        let stream = provider
            .stream(&request(), CancellationToken::new())
            .await
            .expect("the template retry succeeds");
        assert_eq!(super::super::collect_text(stream).await.unwrap(), "Hello");
        assert!(provider.own_instructions_rejected());

        let seen = seen.await.unwrap();
        assert_eq!(
            seen.len(),
            2,
            "one own-instructions attempt, one template retry"
        );
        let first: serde_json::Value = serde_json::from_str(&seen[0].1).unwrap();
        assert_eq!(first["instructions"], "You are Bluey.");
        assert_eq!(first["input"][0]["role"], "user");
        let second: serde_json::Value = serde_json::from_str(&seen[1].1).unwrap();
        assert_eq!(second["instructions"], "You are Codex.");
        assert_eq!(second["input"][0]["role"], "developer");
        assert_eq!(second["input"][0]["content"][0]["text"], "You are Bluey.");
        assert_eq!(second["input"][1]["content"][0]["text"], "hello");
    }

    #[tokio::test]
    async fn an_empty_stream_on_bluey_instructions_switches_the_next_request_to_the_template() {
        let empty_sse = "event: response.created\ndata: {\"type\":\"response.created\"}\n\n\
data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":0}}}\n\n";
        let (base, seen) = stub_many(vec![ok_response(empty_sse), ok_response(HELLO_SSE)]).await;
        let provider = provider(&base);

        let first = provider
            .stream(&request(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            super::super::collect_text(first).await.unwrap(),
            "",
            "the empty reply itself is passed through — the WebView shows its failure state"
        );
        assert!(provider.own_instructions_rejected());

        let second = provider
            .stream(&request(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(super::super::collect_text(second).await.unwrap(), "Hello");

        let seen = seen.await.unwrap();
        let first: serde_json::Value = serde_json::from_str(&seen[0].1).unwrap();
        assert_eq!(first["instructions"], "You are Bluey.");
        let second: serde_json::Value = serde_json::from_str(&seen[1].1).unwrap();
        assert_eq!(second["instructions"], "You are Codex.");
        assert_eq!(second["input"][0]["role"], "developer");
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
