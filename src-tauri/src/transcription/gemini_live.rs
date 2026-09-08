//! Gemini Live API transcription (`gemini-3.5-transcribe-live`).
//!
//! One WebSocket per audio source: `setup` → `setupComplete`, then
//! `realtimeInput.audio` frames (PCM16 16 kHz mono) and `audioStreamEnd`
//! after 500 ms of silence so utterances finalize promptly. Sessions are
//! capped at ten minutes by the service, so a replacement socket is opened at
//! 9 min 30 s (or on `goAway`); the old one drains for two seconds and finals
//! that repeat across the hand-over are dropped by [`FinalDedupe`].
//!
//! The API key travels only in the WebSocket URL query (the Live API's
//! requirement); every log line uses [`proto::redact_live_url`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bluey_core::types::TranscriptionProviderKind;
use bluey_core::{BlueyError, BlueyErrorKind, BlueyResult};
use bluey_protocols::gemini::{self as proto, LiveEvent};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::{
    session_closed, EventSink, FinalDedupe, PcmChunk, SessionOptions, TranscriptionEvent,
    TranscriptionProvider, TranscriptionSession,
};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Open the replacement session this long after a session started.
pub const SOFT_SESSION_LIMIT: Duration = Duration::from_secs(9 * 60 + 30);
/// How long a replaced session keeps draining finals.
pub const DRAIN_GRACE: Duration = Duration::from_secs(2);
/// Silence after speech that triggers `audioStreamEnd`.
pub const SILENCE_STREAM_END: Duration = Duration::from_millis(500);
/// Server VAD end-of-speech silence requested in `setup`.
pub const SERVER_SILENCE_MS: u32 = 600;
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECT_ATTEMPTS: u32 = 3;
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const COMMAND_BUFFER: usize = 256;

pub struct GeminiLiveProvider {
    api_key: String,
    smart_mode: bool,
}

impl GeminiLiveProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            smart_mode: true,
        }
    }
}

#[async_trait]
impl TranscriptionProvider for GeminiLiveProvider {
    fn kind(&self) -> TranscriptionProviderKind {
        TranscriptionProviderKind::GeminiLive
    }

    async fn open(
        &self,
        options: SessionOptions,
        sink: EventSink,
    ) -> BlueyResult<Box<dyn TranscriptionSession>> {
        let (tx, rx) = mpsc::channel::<Command>(COMMAND_BUFFER);
        let worker = Worker {
            api_key: self.api_key.clone(),
            smart_mode: self.smart_mode,
            options,
            sink,
            dedupe: Arc::new(parking_lot::Mutex::new(FinalDedupe::default())),
        };
        let task = tauri::async_runtime::spawn(async move { worker.run(rx).await });
        Ok(Box::new(LiveSession {
            tx,
            task: parking_lot::Mutex::new(Some(task)),
        }))
    }
}

enum Command {
    Audio(PcmChunk),
    Close,
}

struct LiveSession {
    tx: mpsc::Sender<Command>,
    task: parking_lot::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

#[async_trait]
impl TranscriptionSession for LiveSession {
    async fn push_audio(&self, chunk: PcmChunk) -> BlueyResult<()> {
        self.tx
            .send(Command::Audio(chunk))
            .await
            .map_err(|_| session_closed())
    }

    async fn close(&self) {
        let _ = self.tx.send(Command::Close).await;
        let task = self.task.lock().take();
        if let Some(task) = task {
            let _ = tokio::time::timeout(DRAIN_GRACE + Duration::from_secs(2), task).await;
        }
    }
}

struct Connection {
    socket: Socket,
    opened_at: Instant,
    /// `audioStreamEnd` was sent since the last speech chunk.
    stream_end_sent: bool,
    last_speech: Option<Instant>,
}

impl Connection {
    async fn send(&mut self, value: serde_json::Value) -> BlueyResult<()> {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .await
            .map_err(|_| BlueyError::network("stream", "the Gemini Live socket closed"))
    }
}

enum Action {
    None,
    Rotate,
    Fatal(BlueyError),
}

struct Worker {
    api_key: String,
    smart_mode: bool,
    options: SessionOptions,
    sink: EventSink,
    dedupe: Arc<parking_lot::Mutex<FinalDedupe>>,
}

/// Map a Live `error.status` onto the contract error.
pub fn map_live_error(status: &str) -> BlueyError {
    match status.to_ascii_uppercase().as_str() {
        "UNAUTHENTICATED" | "PERMISSION_DENIED" => BlueyError::new(
            BlueyErrorKind::Configuration,
            "config.api_key_invalid",
            "Google AI Studio rejected the API key for live transcription",
        )
        .recoverable(bluey_core::error::RecoveryAction::ConfigureProvider),
        "RESOURCE_EXHAUSTED" => BlueyError::network(
            "http_429",
            "the Gemini Live API rate-limited the transcription session",
        ),
        "NOT_FOUND" | "INVALID_ARGUMENT" => BlueyError::new(
            BlueyErrorKind::Configuration,
            "config.model_not_found",
            "the Live transcription model is not available for this key",
        )
        .recoverable(bluey_core::error::RecoveryAction::ConfigureProvider),
        other => BlueyError::transcription(
            "live_error",
            format!("the Gemini Live session reported {other}"),
        ),
    }
}

/// Configuration errors end the session; everything else is worth a reconnect.
fn is_fatal(error: &BlueyError) -> bool {
    error.kind == BlueyErrorKind::Configuration
}

impl Worker {
    fn redacted_url(&self) -> String {
        proto::redact_live_url(&proto::live_url(&self.api_key))
    }

    /// Connect, send `setup`, wait for `setupComplete`. Retries transport
    /// failures with a short backoff; configuration errors are returned as is.
    async fn connect(&self) -> BlueyResult<Connection> {
        let url = proto::live_url(&self.api_key);
        let mut last_error =
            BlueyError::network("connect", "could not connect to the Gemini Live API");
        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            match connect_async(url.as_str()).await {
                Ok((socket, _)) => {
                    let mut conn = Connection {
                        socket,
                        opened_at: Instant::now(),
                        stream_end_sent: true,
                        last_speech: None,
                    };
                    let setup = proto::live_setup_message(&proto::LiveSetupOptions {
                        model: &self.options.model,
                        language: self.options.language.as_deref(),
                        custom_vocabulary: &self.options.vocabulary,
                        smart_mode: self.smart_mode,
                        silence_ms: SERVER_SILENCE_MS,
                    });
                    conn.send(setup).await?;
                    match tokio::time::timeout(SETUP_TIMEOUT, wait_for_setup(&mut conn)).await {
                        Ok(Ok(())) => {
                            tracing::info!(
                                source = ?self.options.source,
                                model = %self.options.model,
                                url = %self.redacted_url(),
                                "gemini live transcription session ready"
                            );
                            return Ok(conn);
                        }
                        Ok(Err(error)) => {
                            if is_fatal(&error) {
                                return Err(error);
                            }
                            last_error = error;
                        }
                        Err(_) => {
                            last_error = BlueyError::network(
                                "timeout",
                                "the Gemini Live session did not complete setup in time",
                            );
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        attempt,
                        url = %self.redacted_url(),
                        error = %error,
                        "gemini live connect failed"
                    );
                }
            }
            if attempt < MAX_CONNECT_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(400 * u64::from(attempt))).await;
            }
        }
        Err(last_error)
    }

    async fn forward(&self, conn: &mut Connection, chunk: &PcmChunk) -> BlueyResult<()> {
        conn.send(proto::live_audio_message(&chunk.base64, chunk.sample_rate))
            .await?;
        if chunk.is_speech {
            conn.last_speech = Some(Instant::now());
            conn.stream_end_sent = false;
        }
        Ok(())
    }

    async fn emit_final(&self, text: String, language: Option<String>) {
        if !self.dedupe.lock().accept(&text, Instant::now()) {
            return;
        }
        let _ = self
            .sink
            .send(TranscriptionEvent::Final {
                source: self.options.source,
                text,
                language,
            })
            .await;
    }

    async fn handle(&self, event: LiveEvent) -> Action {
        match event {
            LiveEvent::Interim(text) => {
                if !text.trim().is_empty() {
                    let _ = self
                        .sink
                        .send(TranscriptionEvent::Interim {
                            source: self.options.source,
                            text,
                        })
                        .await;
                }
                Action::None
            }
            LiveEvent::Final { text, language } => {
                self.emit_final(text, language).await;
                Action::None
            }
            LiveEvent::GoAway { time_left_ms } => {
                tracing::info!(?time_left_ms, "gemini live session is going away; rotating");
                Action::Rotate
            }
            LiveEvent::Error(status) => {
                let error = map_live_error(&status);
                if is_fatal(&error) {
                    Action::Fatal(error)
                } else {
                    tracing::warn!(status = %status, "gemini live session error; reconnecting");
                    Action::Rotate
                }
            }
            LiveEvent::SetupComplete | LiveEvent::ResumptionUpdate { .. } | LiveEvent::Other => {
                Action::None
            }
        }
    }

    /// Open the replacement socket first, then let the old one drain finals.
    async fn rotate(&self, old: Connection) -> BlueyResult<Connection> {
        let fresh = self.connect().await?;
        let worker = Self {
            api_key: self.api_key.clone(),
            smart_mode: self.smart_mode,
            options: self.options.clone(),
            sink: self.sink.clone(),
            dedupe: self.dedupe.clone(),
        };
        tauri::async_runtime::spawn(async move { worker.drain(old, DRAIN_GRACE).await });
        Ok(fresh)
    }

    /// Ask for the last utterance and forward finals until `grace` elapses.
    async fn drain(&self, mut conn: Connection, grace: Duration) {
        let _ = conn.send(proto::live_audio_stream_end()).await;
        let deadline = Instant::now() + grace;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, conn.socket.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    if let LiveEvent::Final { text, language } = proto::parse_live_message(&text) {
                        self.emit_final(text, language).await;
                    }
                }
                Ok(Some(Ok(_))) => {}
                _ => break,
            }
        }
        let _ = conn.socket.close(None).await;
    }

    async fn run(self, mut rx: mpsc::Receiver<Command>) {
        let mut conn = match self.connect().await {
            Ok(conn) => conn,
            Err(error) => {
                let _ = self
                    .sink
                    .send(TranscriptionEvent::Failed {
                        source: self.options.source,
                        error,
                    })
                    .await;
                return;
            }
        };
        let mut failures = 0u32;
        loop {
            let rotate_at = conn.opened_at + SOFT_SESSION_LIMIT;
            let silence_deadline = match (conn.stream_end_sent, conn.last_speech) {
                (false, Some(at)) => Some(at + SILENCE_STREAM_END),
                _ => None,
            };
            let action = tokio::select! {
                command = rx.recv() => match command {
                    None | Some(Command::Close) => break,
                    Some(Command::Audio(chunk)) => match self.forward(&mut conn, &chunk).await {
                        Ok(()) => Action::None,
                        Err(_) => Action::Rotate,
                    },
                },
                message = conn.socket.next() => match message {
                    Some(Ok(Message::Text(text))) => self.handle(proto::parse_live_message(&text)).await,
                    Some(Ok(Message::Close(_))) | None => Action::Rotate,
                    Some(Ok(_)) => Action::None,
                    Some(Err(error)) => {
                        tracing::warn!(error = %error, "gemini live socket error");
                        Action::Rotate
                    }
                },
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(rotate_at)) => Action::Rotate,
                _ = async {
                    match silence_deadline {
                        Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    if conn.send(proto::live_audio_stream_end()).await.is_ok() {
                        conn.stream_end_sent = true;
                    }
                    Action::None
                }
            };
            match action {
                Action::None => {
                    failures = 0;
                }
                Action::Rotate => match self.rotate(conn).await {
                    Ok(fresh) => {
                        conn = fresh;
                        failures = 0;
                    }
                    Err(error) => {
                        failures += 1;
                        if is_fatal(&error) || failures >= MAX_CONSECUTIVE_FAILURES {
                            let _ = self
                                .sink
                                .send(TranscriptionEvent::Failed {
                                    source: self.options.source,
                                    error,
                                })
                                .await;
                            return;
                        }
                        // Keep trying with a fresh socket; audio in the meantime is lost.
                        match self.connect().await {
                            Ok(fresh) => conn = fresh,
                            Err(error) => {
                                let _ = self
                                    .sink
                                    .send(TranscriptionEvent::Failed {
                                        source: self.options.source,
                                        error,
                                    })
                                    .await;
                                return;
                            }
                        }
                    }
                },
                Action::Fatal(error) => {
                    let _ = self
                        .sink
                        .send(TranscriptionEvent::Failed {
                            source: self.options.source,
                            error,
                        })
                        .await;
                    let _ = conn.socket.close(None).await;
                    return;
                }
            }
        }
        // Closing: flush the current utterance and drain what is left.
        self.drain(conn, DRAIN_GRACE).await;
    }
}

/// Read frames until `setupComplete` (errors before it are setup failures).
async fn wait_for_setup(conn: &mut Connection) -> BlueyResult<()> {
    loop {
        match conn.socket.next().await {
            Some(Ok(Message::Text(text))) => match proto::parse_live_message(&text) {
                LiveEvent::SetupComplete => return Ok(()),
                LiveEvent::Error(status) => return Err(map_live_error(&status)),
                _ => {}
            },
            Some(Ok(Message::Close(_))) | None => {
                return Err(BlueyError::network(
                    "stream",
                    "the Gemini Live socket closed during setup",
                ))
            }
            Some(Ok(_)) => {}
            Some(Err(_)) => {
                return Err(BlueyError::network(
                    "stream",
                    "the Gemini Live socket failed during setup",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_errors_map_to_contract_codes() {
        assert_eq!(
            map_live_error("UNAUTHENTICATED").code,
            "config.api_key_invalid"
        );
        assert_eq!(
            map_live_error("resource_exhausted").code,
            "network.http_429"
        );
        assert_eq!(map_live_error("NOT_FOUND").code, "config.model_not_found");
        assert_eq!(map_live_error("INTERNAL").code, "transcription.live_error");
        assert!(is_fatal(&map_live_error("PERMISSION_DENIED")));
        assert!(!is_fatal(&map_live_error("UNAVAILABLE")));
    }

    #[test]
    fn rotation_happens_before_the_service_cap() {
        assert!(SOFT_SESSION_LIMIT < Duration::from_secs(10 * 60));
        assert!(SILENCE_STREAM_END < Duration::from_secs(1));
    }
}
