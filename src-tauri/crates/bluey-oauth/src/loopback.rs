//! One-shot loopback listener for browser redirects.
//!
//! Bound to `127.0.0.1` only — on an OS-chosen port, or on a *required* fixed
//! port for providers whose client registration allows just one (the Codex
//! CLI's 1455). It accepts a single connection, reads one request head of at
//! most [`LOOPBACK_MAX_HEAD`] bytes within [`LOOPBACK_READ_TIMEOUT`], hands the
//! request target (`/callback?code=…`) to the caller — who validates `state`
//! first — and lets it answer with a small page. Then it is gone.

use std::io;
use std::time::Duration;

use bluey_protocols::oauth::{http_request_target, loopback_http_response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

/// The only address a listener binds.
pub const LOOPBACK_HOST: &str = "127.0.0.1";
/// Longest HTTP request head the listener reads.
pub const LOOPBACK_MAX_HEAD: usize = 8 * 1024;
/// How long the listener waits for the request head once connected.
pub const LOOPBACK_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Which port to bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopbackPort {
    /// Any free port, chosen by the OS (`http://127.0.0.1:<port>/…`).
    Any,
    /// Exactly this port; [`LoopbackError::PortInUse`] when it is taken, so the
    /// caller can fall back (another port, a device-code flow, …).
    Fixed(u16),
}

#[derive(Debug, thiserror::Error)]
pub enum LoopbackError {
    #[error("port {0} is already in use")]
    PortInUse(u16),
    #[error("cannot open the callback port: {0}")]
    Bind(#[source] io::Error),
    #[error("the callback listener was cancelled")]
    Cancelled,
    #[error("accepting the callback connection failed: {0}")]
    Accept(#[source] io::Error),
}

#[derive(Debug)]
pub struct LoopbackListener {
    listener: TcpListener,
    port: u16,
    read_timeout: Duration,
}

impl LoopbackListener {
    /// Bind `127.0.0.1` on the requested port.
    pub async fn bind(port: LoopbackPort) -> Result<Self, LoopbackError> {
        let requested = match port {
            LoopbackPort::Any => 0,
            LoopbackPort::Fixed(port) => port,
        };
        let listener = TcpListener::bind((LOOPBACK_HOST, requested))
            .await
            .map_err(|error| match (port, error.kind()) {
                (LoopbackPort::Fixed(port), io::ErrorKind::AddrInUse) => {
                    LoopbackError::PortInUse(port)
                }
                _ => LoopbackError::Bind(error),
            })?;
        let port = listener.local_addr().map_err(LoopbackError::Bind)?.port();
        Ok(Self {
            listener,
            port,
            read_timeout: LOOPBACK_READ_TIMEOUT,
        })
    }

    /// The bound port (the one to put in the redirect URI).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Override the request-head read timeout (tests).
    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    /// Accept exactly one connection and read its request head. `target` is
    /// `None` when the head was malformed, too large, or did not arrive in
    /// time — the caller still answers, with its failure page. Cancelling
    /// `cancel` before a connection arrives ends the wait.
    pub async fn accept_one(self, cancel: &CancellationToken) -> Result<Accepted, LoopbackError> {
        let accepted = tokio::select! {
            _ = cancel.cancelled() => return Err(LoopbackError::Cancelled),
            accepted = self.listener.accept() => accepted,
        };
        let (mut stream, _) = accepted.map_err(LoopbackError::Accept)?;
        let mut buffer = vec![0u8; LOOPBACK_MAX_HEAD];
        let mut length = 0usize;
        let read = tokio::time::timeout(self.read_timeout, async {
            loop {
                let n = stream.read(&mut buffer[length..]).await?;
                if n == 0 {
                    break;
                }
                length += n;
                if buffer[..length].windows(4).any(|w| w == b"\r\n\r\n") || length == buffer.len() {
                    break;
                }
            }
            Ok::<(), io::Error>(())
        })
        .await;
        let target = match read {
            Ok(Ok(())) => {
                http_request_target(&String::from_utf8_lossy(&buffer[..length])).map(str::to_string)
            }
            _ => None,
        };
        Ok(Accepted {
            target,
            responder: Responder { stream },
        })
    }
}

/// One accepted callback connection.
pub struct Accepted {
    /// The request target (`/callback?code=…&state=…`), if the head was a
    /// well-formed `GET`.
    pub target: Option<String>,
    pub responder: Responder,
}

/// Writes the one response the browser gets, then closes the connection.
pub struct Responder {
    stream: TcpStream,
}

impl Responder {
    /// Answer with an HTML page (see `bluey_protocols::oauth::loopback_html`).
    /// Write errors are ignored: the browser may already be gone.
    pub async fn respond_html(mut self, body: &str) {
        let _ = self
            .stream
            .write_all(loopback_http_response(body).as_bytes())
            .await;
        let _ = self.stream.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tokio::io::AsyncReadExt;

    use super::*;

    async fn connect_and_send(port: u16, head: &[u8]) -> TcpStream {
        let mut client = TcpStream::connect((LOOPBACK_HOST, port)).await.unwrap();
        client.write_all(head).await.unwrap();
        client
    }

    async fn read_all(mut client: TcpStream) -> String {
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }

    #[tokio::test]
    async fn a_callback_request_is_delivered_and_answered() {
        let listener = LoopbackListener::bind(LoopbackPort::Any).await.unwrap();
        let port = listener.port();
        assert_ne!(port, 0);
        let cancel = CancellationToken::new();
        let client = connect_and_send(
            port,
            b"GET /callback?code=abc&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: */*\r\n\r\n",
        )
        .await;
        let accepted = listener.accept_one(&cancel).await.unwrap();
        assert_eq!(
            accepted.target.as_deref(),
            Some("/callback?code=abc&state=xyz")
        );
        accepted.responder.respond_html("<p>done</p>").await;
        let response = read_all(client).await;
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Content-Length: 11\r\n"));
        assert!(response.contains("Connection: close\r\n"));
        assert!(response.ends_with("\r\n\r\n<p>done</p>"));
    }

    #[tokio::test]
    async fn a_fixed_port_that_is_taken_is_reported_as_such() {
        let taken = LoopbackListener::bind(LoopbackPort::Any).await.unwrap();
        let port = taken.port();
        match LoopbackListener::bind(LoopbackPort::Fixed(port)).await {
            Err(LoopbackError::PortInUse(reported)) => assert_eq!(reported, port),
            other => panic!("expected PortInUse, got {other:?}"),
        }
        drop(taken);
        let free = LoopbackListener::bind(LoopbackPort::Fixed(port))
            .await
            .unwrap();
        assert_eq!(free.port(), port);
    }

    #[tokio::test]
    async fn cancellation_ends_the_wait_for_a_connection() {
        let listener = LoopbackListener::bind(LoopbackPort::Any).await.unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            listener.accept_one(&cancel).await,
            Err(LoopbackError::Cancelled)
        ));
    }

    #[tokio::test]
    async fn malformed_heads_still_get_an_answer() {
        let listener = LoopbackListener::bind(LoopbackPort::Any).await.unwrap();
        let port = listener.port();
        let cancel = CancellationToken::new();
        let client = connect_and_send(port, b"POST /callback HTTP/1.1\r\nHost: x\r\n\r\n").await;
        let accepted = listener.accept_one(&cancel).await.unwrap();
        assert_eq!(accepted.target, None);
        accepted.responder.respond_html("<p>nope</p>").await;
        assert!(read_all(client).await.contains("<p>nope</p>"));
    }

    #[tokio::test]
    async fn oversized_heads_are_cut_at_the_cap() {
        let listener = LoopbackListener::bind(LoopbackPort::Any).await.unwrap();
        let port = listener.port();
        let cancel = CancellationToken::new();
        // A request line that never ends: no GET target can be parsed from it.
        let head = vec![b'A'; LOOPBACK_MAX_HEAD + 1024];
        let client = connect_and_send(port, &head).await;
        let accepted = listener.accept_one(&cancel).await.unwrap();
        assert_eq!(accepted.target, None);
        accepted.responder.respond_html("x").await;
        drop(client);
    }

    #[tokio::test]
    async fn a_client_that_never_finishes_its_head_times_out() {
        let listener = LoopbackListener::bind(LoopbackPort::Any)
            .await
            .unwrap()
            .with_read_timeout(Duration::from_millis(50));
        let port = listener.port();
        let cancel = CancellationToken::new();
        let client = connect_and_send(port, b"GET /callback?code=a&state=b HTTP/1.1\r\n").await;
        let accepted = listener.accept_one(&cancel).await.unwrap();
        assert_eq!(accepted.target, None);
        accepted.responder.respond_html("late").await;
        assert!(read_all(client).await.contains("late"));
    }
}
