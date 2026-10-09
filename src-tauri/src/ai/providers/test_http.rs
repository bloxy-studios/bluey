//! A scripted HTTP backend for adapter tests: one canned response per
//! connection, in order; every raw request (head, body) comes back afterwards.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// `HTTP/1.1 {status}` with the given extra headers and body.
pub fn response(status: &str, headers: &[(&str, &str)], body: &str) -> String {
    let extra: String = headers
        .iter()
        .map(|(k, v)| format!("{k}: {v}\r\n"))
        .collect();
    format!(
        "HTTP/1.1 {status}\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// A 200 SSE answer of OpenAI-style chat chunks saying `text`.
pub fn openai_sse(text: &str) -> String {
    let body = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n\
         data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    );
    response("200 OK", &[("content-type", "text/event-stream")], &body)
}

async fn read_request(socket: &mut TcpStream) -> (String, String) {
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

/// Serve `responses` in order; the receiver yields every request once all
/// have been served.
pub async fn stub_many(
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
