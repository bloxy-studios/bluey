//! The capture proxy: `127.0.0.1:<port>` in, the real upstream out.
//!
//! One connection = one request (`Connection: close`; the official CLIs reconnect).
//! The request is read in full, forwarded with reqwest (hop-by-hop headers dropped,
//! `Accept-Encoding` stripped so the recorded body is readable), the upstream
//! response is streamed back chunk by chunk, and the exchange — request as
//! received, response as far as the record cap — is scrubbed and written to
//! `captures/` before anyone else sees it.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use bluey_protocols::fingerprints::{
    client_from_user_agent, scrub_capture, Body, Capture, CaptureSource, CapturedRequest,
    CapturedResponse, FingerprintStamp, Header, Provider, SCHEMA_VERSION,
};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{http1, store};

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub provider: Provider,
    /// Forward target (`https://api.anthropic.com`); the request path is appended as-is.
    pub upstream: String,
    pub bind: SocketAddr,
    /// `tests/fixtures/fingerprints` (captures go to `<provider>/captures/`).
    pub out_dir: PathBuf,
    pub max_request_body: usize,
    /// Response bytes kept for the record; the client always receives everything.
    pub record_response_limit: usize,
    pub read_timeout: Duration,
}

impl ProxyConfig {
    pub fn new(provider: Provider, out_dir: PathBuf) -> Self {
        Self {
            provider,
            upstream: provider.rules().upstream.to_string(),
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            out_dir,
            max_request_body: 32 << 20,
            record_response_limit: 256 << 10,
            read_timeout: Duration::from_secs(60),
        }
    }
}

/// One recorded exchange.
#[derive(Debug)]
pub struct Recorded {
    pub path: PathBuf,
    pub capture: Capture,
    pub upstream_url: String,
}

pub struct ProxyHandle {
    pub addr: SocketAddr,
    pub events: mpsc::UnboundedReceiver<Recorded>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl ProxyHandle {
    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

/// Bind and start accepting; the handle's `events` yield every recorded exchange.
pub async fn start(config: ProxyConfig) -> anyhow::Result<ProxyHandle> {
    crate::ensure_crypto_provider();
    let listener = TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("binding {}", config.bind))?;
    let addr = listener.local_addr()?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .build()
        .context("building the upstream client")?;
    let (tx, events) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let config = Arc::new(config);
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    accepted = listener.accept() => match accepted {
                        Ok((stream, _)) => {
                            let (cfg, client, tx) = (config.clone(), client.clone(), tx.clone());
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, cfg, client, tx).await {
                                    eprintln!("  ! {e:#}");
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("  ! accept failed: {e}");
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        }
    });
    Ok(ProxyHandle {
        addr,
        events,
        cancel,
        task,
    })
}

fn host_of(base: &str) -> String {
    reqwest::Url::parse(base)
        .ok()
        .and_then(|u| {
            u.host_str().map(|h| match u.port() {
                Some(p) => format!("{h}:{p}"),
                None => h.to_string(),
            })
        })
        .unwrap_or_default()
}

async fn handle_connection(
    mut stream: TcpStream,
    cfg: Arc<ProxyConfig>,
    client: reqwest::Client,
    tx: mpsc::UnboundedSender<Recorded>,
) -> anyhow::Result<()> {
    let (head, leftover) =
        tokio::time::timeout(cfg.read_timeout, http1::read_head(&mut stream, 64 << 10))
            .await
            .context("timed out waiting for the request head")??;
    if head.expects_continue() {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
    }
    let body = tokio::time::timeout(
        cfg.read_timeout,
        http1::read_body(&mut stream, &head, leftover, cfg.max_request_body),
    )
    .await
    .context("timed out reading the request body")??;

    let started = Instant::now();
    let upstream_base = cfg.upstream.trim_end_matches('/');
    let upstream_url = format!("{upstream_base}{}", head.target);
    let upstream_host = host_of(upstream_base);
    let mut notes = Vec::new();

    let method = reqwest::Method::from_bytes(head.method.as_bytes()).context("invalid method")?;
    let mut request = client.request(method, &upstream_url);
    for (name, value) in &head.headers {
        if http1::is_hop_by_hop(name) || name == "host" || name == "content-length" {
            continue;
        }
        if name == "accept-encoding" {
            notes.push(
                "accept-encoding stripped before forwarding so the recorded response body is readable"
                    .to_string(),
            );
            continue;
        }
        match (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            (Ok(n), Ok(v)) => request = request.header(n, v),
            _ => notes.push(format!("header {name} could not be forwarded")),
        }
    }
    if !body.is_empty() {
        request = request.body(body.clone());
    }

    let recorded_request = CapturedRequest {
        method: head.method.clone(),
        url: upstream_url.clone(),
        headers: head
            .headers
            .iter()
            .map(|(n, v)| {
                if n == "host" {
                    Header::new("host", upstream_host.clone())
                } else {
                    Header::new(n, v.clone())
                }
            })
            .collect(),
        body: Body::from_bytes(head.header("content-type"), &body),
    };

    let response = match request.send().await {
        Ok(response) => Some(
            relay_response(
                &mut stream,
                response,
                cfg.record_response_limit,
                started,
                &mut notes,
            )
            .await,
        ),
        Err(error) => {
            notes.push(format!("upstream request failed: {error}"));
            let payload = serde_json::json!({
                "error": { "type": "bluey_capture_proxy", "message": format!("upstream request failed: {error}") }
            })
            .to_string();
            let head = format!(
                "HTTP/1.1 502 Bad Gateway\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                payload.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(payload.as_bytes()).await;
            None
        }
    };
    let _ = stream.shutdown().await;

    let rules = cfg.provider.rules();
    let mut capture = Capture {
        schema: SCHEMA_VERSION,
        provider: cfg.provider.id().to_string(),
        source: CaptureSource::Proxy,
        captured_at: crate::now_rfc3339(),
        fingerprint: FingerprintStamp {
            version: rules.info.version.to_string(),
            captured_on: rules.info.captured_on.to_string(),
        },
        client: head.header("user-agent").and_then(client_from_user_agent),
        request: recorded_request,
        response,
        scrubbed: Vec::new(),
        notes,
    };
    scrub_capture(&mut capture, rules);
    let path = store::write_capture(&cfg.out_dir, &capture)?;
    let _ = tx.send(Recorded {
        path,
        capture,
        upstream_url,
    });
    Ok(())
}

async fn relay_response(
    stream: &mut TcpStream,
    mut response: reqwest::Response,
    limit: usize,
    started: Instant,
    notes: &mut Vec<String>,
) -> CapturedResponse {
    let status = response.status();
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    let mut headers = Vec::new();
    for (name, value) in response.headers() {
        let name = name.as_str();
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        headers.push(Header::new(name, value.clone()));
        if http1::is_hop_by_hop(name) || matches!(name, "content-length" | "content-encoding") {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("transfer-encoding: chunked\r\nconnection: close\r\n\r\n");
    let mut client_alive = stream.write_all(head.as_bytes()).await.is_ok();

    let content_type = headers
        .iter()
        .find(|h| h.name == "content-type")
        .map(|h| h.value.clone());
    let mut recorded: Vec<u8> = Vec::new();
    let mut truncated = false;
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if client_alive && stream.write_all(&http1::chunk_frame(&chunk)).await.is_err() {
                    client_alive = false;
                    notes.push("client disconnected while the response was streaming".to_string());
                }
                let room = limit.saturating_sub(recorded.len());
                if chunk.len() > room {
                    recorded.extend_from_slice(&chunk[..room]);
                    truncated = true;
                } else {
                    recorded.extend_from_slice(&chunk);
                }
            }
            Ok(None) => break,
            Err(e) => {
                notes.push(format!("upstream body ended with an error: {e}"));
                break;
            }
        }
    }
    if client_alive {
        let _ = stream.write_all(http1::LAST_CHUNK).await;
    }
    if truncated {
        notes.push(format!("response body recorded up to {limit} bytes"));
    }
    CapturedResponse {
        status: status.as_u16(),
        headers,
        body: Body::from_bytes(content_type.as_deref(), &recorded),
        duration_ms: Some(started.elapsed().as_millis() as u64),
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// Accepts one connection, reads the whole request, writes `respond(head, body)`
    /// part by part with small pauses (so streaming is observable).
    async fn fake_upstream(
        respond: impl Fn(http1::RequestHead, Vec<u8>) -> Vec<Vec<u8>> + Send + 'static,
    ) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (head, body) = http1::read_request(&mut stream, 64 << 10, 1 << 20)
                .await
                .unwrap();
            for part in respond(head, body) {
                stream.write_all(&part).await.unwrap();
                stream.flush().await.unwrap();
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let _ = stream.shutdown().await;
        });
        addr
    }

    async fn send_raw(addr: SocketAddr, request: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request).await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let end = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("response head");
        let head = String::from_utf8_lossy(&raw[..end]).to_string();
        let mut lines = head.split("\r\n");
        let status: u16 = lines
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .filter_map(|l| {
                l.split_once(':')
                    .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
            })
            .collect();
        let rest = &raw[end + 4..];
        let body = match http1::decode_chunked(rest) {
            Ok(http1::Chunked::Complete(b, _)) => b,
            _ => rest.to_vec(),
        };
        (status, headers, body)
    }

    fn json_response(body: &str, extra_headers: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n{extra_headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    async fn start_for(
        provider: Provider,
        upstream: SocketAddr,
        dir: &std::path::Path,
    ) -> ProxyHandle {
        let mut config = ProxyConfig::new(provider, dir.to_path_buf());
        config.upstream = format!("http://{upstream}");
        start(config).await.unwrap()
    }

    #[tokio::test]
    async fn forwards_json_and_writes_a_scrubbed_capture() {
        let upstream = fake_upstream(|head, body| {
            assert_eq!(head.target, "/v1/messages?beta=true");
            assert_eq!(
                head.header("authorization"),
                Some("Bearer sk-ant-oat01-SECRETSECRETSECRETSECRET"),
                "the upstream sees the real token"
            );
            assert!(head.header("accept-encoding").is_none(), "stripped before forwarding");
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["messages"][0]["content"], "hello jordan@example.com");
            vec![json_response(
                r#"{"id":"msg_01ABCDEFGHIJ","type":"message","content":[{"type":"text","text":"Hi Jordan"}]}"#,
                "request-id: req_011CVabcdefghij\r\nanthropic-ratelimit-unified-status: allowed\r\n",
            )]
        })
        .await;
        let tmp = tempfile::tempdir().unwrap();
        let mut proxy = start_for(Provider::Claude, upstream, tmp.path()).await;

        let body = r#"{"model":"claude-sonnet-5","messages":[{"role":"user","content":"hello jordan@example.com"}]}"#;
        let request = format!(
            "POST /v1/messages?beta=true HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer sk-ant-oat01-SECRETSECRETSECRETSECRET\r\nUser-Agent: claude-cli/2.1.268 (external, cli)\r\nAccept-Encoding: gzip\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let (status, headers, response_body) = send_raw(proxy.addr, request.as_bytes()).await;
        assert_eq!(status, 200);
        assert!(
            headers
                .iter()
                .any(|(n, v)| n == "request-id" && v == "req_011CVabcdefghij"),
            "the client sees the real response headers: {headers:?}"
        );
        let parsed: serde_json::Value = serde_json::from_slice(&response_body).unwrap();
        assert_eq!(parsed["content"][0]["text"], "Hi Jordan");

        let recorded = proxy.events.recv().await.expect("a capture event");
        assert_eq!(
            recorded.upstream_url,
            format!("http://{upstream}/v1/messages?beta=true")
        );
        assert!(recorded
            .path
            .starts_with(tmp.path().join("claude/captures")));
        let c = &recorded.capture;
        assert_eq!(c.source, CaptureSource::Proxy);
        assert_eq!(c.client.as_deref(), Some("claude-cli/2.1.268"));
        assert_eq!(
            c.request.header("authorization"),
            Some("Bearer <ACCESS_TOKEN>")
        );
        assert_eq!(
            c.request.header("host"),
            Some(upstream.to_string().as_str())
        );
        let Body::Json { value } = &c.request.body else {
            panic!("json request body")
        };
        assert_eq!(value["messages"][0]["content"], "<TEXT 24>");
        let response = c.response.as_ref().expect("recorded response");
        assert_eq!(response.status, 200);
        assert_eq!(response.header("request-id"), Some("<ID>"));
        assert!(response.duration_ms.is_some());
        let Body::Json { value } = &response.body else {
            panic!("json response body")
        };
        assert_eq!(value["content"][0]["text"], "<TEXT 9>");
        assert_eq!(value["id"], "<ID>");
        assert!(c.notes.iter().any(|n| n.contains("accept-encoding")));
        let on_disk = std::fs::read_to_string(&recorded.path).unwrap();
        for secret in [
            "SECRETSECRET",
            "jordan@example.com",
            "Hi Jordan",
            "msg_01",
            "req_011",
        ] {
            assert!(
                !on_disk.contains(secret),
                "{secret} leaked into {}",
                recorded.path.display()
            );
        }
        proxy.shutdown().await;
    }

    #[tokio::test]
    async fn streams_sse_and_records_the_events() {
        let upstream = fake_upstream(|_, _| {
            vec![
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n".to_vec(),
                http1::chunk_frame(b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n"),
                http1::chunk_frame(b"event: content_block_delta\ndata: {\"delta\":{\"text\":\"Hello\"}}\n\n"),
                http1::LAST_CHUNK.to_vec(),
            ]
        })
        .await;
        let tmp = tempfile::tempdir().unwrap();
        let mut proxy = start_for(Provider::Claude, upstream, tmp.path()).await;
        let (status, headers, body) = send_raw(
            proxy.addr,
            b"POST /v1/messages HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
        )
        .await;
        assert_eq!(status, 200);
        assert!(headers
            .iter()
            .any(|(n, v)| n == "content-type" && v == "text/event-stream"));
        let text = String::from_utf8(body).unwrap();
        assert!(
            text.contains("message_start") && text.contains("Hello"),
            "{text}"
        );

        let recorded = proxy.events.recv().await.unwrap();
        let response = recorded.capture.response.as_ref().unwrap();
        let Body::Sse { events } = &response.body else {
            panic!("sse body, got {}", response.body.kind_name())
        };
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.as_deref(), Some("message_start"));
        assert_eq!(events[1].data["delta"]["text"], "<TEXT 5>");
        assert!(!response.truncated);
        proxy.shutdown().await;
    }

    #[tokio::test]
    async fn an_unreachable_upstream_yields_502_and_a_request_only_capture() {
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let tmp = tempfile::tempdir().unwrap();
        let mut proxy = start_for(Provider::Chatgpt, closed, tmp.path()).await;
        let (status, _, body) = send_raw(
            proxy.addr,
            b"GET /backend-api/codex/models?client_version=0.154.0 HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer eyJhbGciOiJSUzI1NiIsImtpZCI6IjEifQ.eyJzdWIiOiIxMjMifQ.c2lnbmF0dXJlLXNpZ25hdHVyZQ\r\n\r\n",
        )
        .await;
        assert_eq!(status, 502);
        assert!(String::from_utf8_lossy(&body).contains("upstream request failed"));
        let recorded = proxy.events.recv().await.unwrap();
        assert!(recorded.capture.response.is_none());
        assert_eq!(
            recorded.capture.request.header("authorization"),
            Some("Bearer <ACCESS_TOKEN>")
        );
        assert!(recorded.path.to_string_lossy().ends_with("-models.json"));
        assert!(recorded
            .capture
            .notes
            .iter()
            .any(|n| n.contains("upstream request failed")));
        proxy.shutdown().await;
    }
}
