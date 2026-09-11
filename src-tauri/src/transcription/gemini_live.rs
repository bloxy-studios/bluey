//! Gemini Live API transcription (`gemini-3.5-transcribe-live`).
//!
//! One WebSocket per audio source: `setup` → `setupComplete`, then
//! `realtimeInput.audio` frames (PCM16 16 kHz mono) and `audioStreamEnd`
//! after 500 ms of silence so utterances finalize promptly. The service
//! answers on **binary** WebSocket frames (UTF-8 JSON), so every frame is
//! decoded through [`decode_frame`] whatever its opcode. Sessions are
//! capped at ten minutes by the service, so a replacement socket is opened at
//! 9 min 30 s (or on `goAway`); the old one drains for two seconds and a final
//! that repeats across the hand-over is dropped by [`FinalDedupe`] — armed
//! only around a rotation, so a genuinely repeated short answer ("Yes.") is
//! kept the rest of the time.
//!
//! Server-side errors and closes reconnect with exponential backoff and give
//! up after a few attempts without transcript progress. Audio never
//! back-pressures the capture pipeline: a chunk that does not fit the worker's
//! buffer is dropped.
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
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::{
    session_closed, EventSink, FinalDedupe, PcmChunk, SessionOptions, TranscriptionEvent,
    TranscriptionProvider, TranscriptionSession, DEDUPE_WINDOW,
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
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECT_ATTEMPTS: u32 = 3;
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// Server-caused reconnects (errors, closes) tolerated without any transcript
/// progress in between.
const MAX_RECONNECTS_WITHOUT_PROGRESS: u32 = 5;
const RECONNECT_BACKOFF_BASE: Duration = Duration::from_millis(500);
const RECONNECT_BACKOFF_CAP: Duration = Duration::from_secs(8);
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
            dedupe_until: Arc::new(parking_lot::Mutex::new(None)),
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
        // Never block the capture pipeline: while the worker reconnects and its
        // buffer is full, the chunk is dropped (late real-time audio is useless).
        match self.tx.try_send(Command::Audio(chunk)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(BlueyError::transcription(
                "backpressure",
                "the live transcription session is not keeping up; dropping audio",
            )),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(session_closed()),
        }
    }

    async fn close(&self) {
        let _ = tokio::time::timeout(Duration::from_secs(1), self.tx.send(Command::Close)).await;
        let task = self.task.lock().take();
        if let Some(mut task) = task {
            if tokio::time::timeout(DRAIN_GRACE + Duration::from_secs(2), &mut task)
                .await
                .is_err()
            {
                // A stuck worker must not outlive the session: it holds a sink
                // clone and could deliver finals into whatever session is
                // active later.
                tracing::warn!("gemini live worker did not finish draining; aborting it");
                task.abort();
            }
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
    /// Nothing to do.
    None,
    /// A transcript event arrived: the connection is healthy.
    Progress,
    /// Planned hand-over (soft limit, `goAway`).
    Rotate,
    /// The server errored or closed: reconnect with backoff.
    Reconnect,
    Fatal(BlueyError),
}

struct Worker {
    api_key: String,
    smart_mode: bool,
    options: SessionOptions,
    sink: EventSink,
    dedupe: Arc<parking_lot::Mutex<FinalDedupe>>,
    /// Dedupe is armed until this instant (set by a rotation).
    dedupe_until: Arc<parking_lot::Mutex<Option<Instant>>>,
}

/// Map a Live `error.status` (gRPC status name or a numeric HTTP-style code)
/// onto the contract error.
pub fn map_live_error(status: &str) -> BlueyError {
    match status.trim().to_ascii_uppercase().as_str() {
        "UNAUTHENTICATED" | "PERMISSION_DENIED" | "401" | "403" => BlueyError::new(
            BlueyErrorKind::Configuration,
            "config.api_key_invalid",
            "Google AI Studio rejected the API key for live transcription",
        )
        .recoverable(bluey_core::error::RecoveryAction::ConfigureProvider),
        "RESOURCE_EXHAUSTED" | "429" => BlueyError::network(
            "http_429",
            "the Gemini Live API rate-limited the transcription session",
        ),
        "NOT_FOUND" | "INVALID_ARGUMENT" | "404" | "400" => BlueyError::new(
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

/// Map the HTTP status of a rejected WebSocket upgrade (a bad key is refused
/// at the handshake, before any in-band error frame).
pub fn map_live_http_status(status: u16) -> BlueyError {
    match status {
        401 | 403 | 404 | 429 => map_live_error(&status.to_string()),
        other => BlueyError::network(
            "connect",
            format!("the Gemini Live API refused the connection (HTTP {other})"),
        ),
    }
}

/// Configuration errors end the session; everything else is worth a reconnect.
fn is_fatal(error: &BlueyError) -> bool {
    error.kind == BlueyErrorKind::Configuration
}

fn reconnect_backoff(reconnects: u32) -> Duration {
    RECONNECT_BACKOFF_BASE
        .checked_mul(2u32.saturating_pow(reconnects.saturating_sub(1)))
        .unwrap_or(RECONNECT_BACKOFF_CAP)
        .min(RECONNECT_BACKOFF_CAP)
}

impl Worker {
    fn redacted_url(&self) -> String {
        proto::redact_live_url(&proto::live_url(&self.api_key))
    }

    /// Connect, send `setup`, wait for `setupComplete`. Retries transport
    /// failures with a short backoff; configuration errors (including a key
    /// refused at the handshake) are returned as is.
    async fn connect(&self) -> BlueyResult<Connection> {
        let url = proto::live_url(&self.api_key);
        let mut last_error =
            BlueyError::network("connect", "could not connect to the Gemini Live API");
        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            match tokio::time::timeout(CONNECT_TIMEOUT, connect_async(url.as_str())).await {
                Ok(Ok((socket, _))) => {
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
                Ok(Err(tungstenite::Error::Http(response))) => {
                    let status = response.status().as_u16();
                    let error = map_live_http_status(status);
                    tracing::warn!(
                        attempt,
                        status,
                        url = %self.redacted_url(),
                        "gemini live handshake rejected"
                    );
                    if is_fatal(&error) {
                        return Err(error);
                    }
                    last_error = error;
                }
                Ok(Err(error)) => {
                    tracing::warn!(
                        attempt,
                        url = %self.redacted_url(),
                        error = %error,
                        "gemini live connect failed"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        attempt,
                        url = %self.redacted_url(),
                        "gemini live connect timed out"
                    );
                    last_error = BlueyError::network(
                        "timeout",
                        "connecting to the Gemini Live API timed out",
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
        if text.trim().is_empty() {
            return;
        }
        let now = Instant::now();
        let armed = self
            .dedupe_until
            .lock()
            .map(|until| now <= until)
            .unwrap_or(false);
        let fresh = self.dedupe.lock().accept(&text, now);
        if armed && !fresh {
            tracing::debug!("dropping a final repeated across the session hand-over");
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
                Action::Progress
            }
            LiveEvent::Final { text, language } => {
                self.emit_final(text, language).await;
                Action::Progress
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
                    Action::Reconnect
                }
            }
            LiveEvent::SetupComplete | LiveEvent::ResumptionUpdate { .. } | LiveEvent::Other => {
                Action::None
            }
        }
    }

    /// Open the replacement socket first, then let the old one drain finals.
    /// Dedupe is armed for the hand-over window only.
    async fn rotate(&self, old: Connection) -> BlueyResult<Connection> {
        let fresh = self.connect().await?;
        *self.dedupe_until.lock() = Some(Instant::now() + DRAIN_GRACE + DEDUPE_WINDOW);
        let worker = Self {
            api_key: self.api_key.clone(),
            smart_mode: self.smart_mode,
            options: self.options.clone(),
            sink: self.sink.clone(),
            dedupe: self.dedupe.clone(),
            dedupe_until: self.dedupe_until.clone(),
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
                Ok(Some(Ok(Message::Close(_)))) => break,
                Ok(Some(Ok(message))) => {
                    if let Some(LiveEvent::Final { text, language }) = decode_frame(&message) {
                        self.emit_final(text, language).await;
                    }
                }
                _ => break,
            }
        }
        let _ = conn.socket.close(None).await;
    }

    async fn fail(&self, error: BlueyError) {
        let _ = self
            .sink
            .send(TranscriptionEvent::Failed {
                source: self.options.source,
                error,
            })
            .await;
    }

    async fn run(self, mut rx: mpsc::Receiver<Command>) {
        let mut conn = match self.connect().await {
            Ok(conn) => conn,
            Err(error) => return self.fail(error).await,
        };
        // Consecutive connect failures while rotating.
        let mut failures = 0u32;
        // Server-caused reconnects since the last transcript event.
        let mut reconnects = 0u32;
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
                        Err(_) => Action::Reconnect,
                    },
                },
                message = conn.socket.next() => match message {
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::info!("gemini live socket closed by the server; reconnecting");
                        Action::Reconnect
                    }
                    Some(Ok(message)) => match decode_frame(&message) {
                        Some(event) => self.handle(event).await,
                        None => Action::None,
                    },
                    Some(Err(error)) => {
                        tracing::warn!(error = %error, "gemini live socket error");
                        Action::Reconnect
                    }
                },
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(rotate_at)) => {
                    tracing::info!(
                        source = ?self.options.source,
                        "gemini live session reached the soft limit; rotating"
                    );
                    Action::Rotate
                }
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
            let reconnecting = matches!(action, Action::Reconnect);
            match action {
                Action::None => {}
                Action::Progress => {
                    failures = 0;
                    reconnects = 0;
                }
                Action::Rotate | Action::Reconnect => {
                    if reconnecting {
                        reconnects += 1;
                        if reconnects > MAX_RECONNECTS_WITHOUT_PROGRESS {
                            return self
                                .fail(BlueyError::network(
                                    "stream",
                                    "the Gemini Live session keeps failing; giving up",
                                ))
                                .await;
                        }
                        tokio::time::sleep(reconnect_backoff(reconnects)).await;
                    }
                    match self.rotate(conn).await {
                        Ok(fresh) => {
                            conn = fresh;
                            failures = 0;
                        }
                        Err(error) => {
                            failures += 1;
                            if is_fatal(&error) || failures >= MAX_CONSECUTIVE_FAILURES {
                                return self.fail(error).await;
                            }
                            // Keep trying with a fresh socket; audio in the meantime is lost.
                            match self.connect().await {
                                Ok(fresh) => conn = fresh,
                                Err(error) => return self.fail(error).await,
                            }
                        }
                    }
                }
                Action::Fatal(error) => {
                    self.fail(error).await;
                    let _ = conn.socket.close(None).await;
                    return;
                }
            }
        }
        // Closing: flush the current utterance and drain what is left.
        self.drain(conn, DRAIN_GRACE).await;
    }
}

/// One Live message from a WebSocket frame, whatever its opcode: the service
/// sends its JSON on binary frames, tooling and proxies may re-frame it as
/// text. Pings, pongs and close frames are not messages.
fn decode_frame(message: &Message) -> Option<LiveEvent> {
    match message {
        Message::Text(text) => Some(proto::parse_live_message(text)),
        Message::Binary(bytes) => Some(proto::parse_live_frame(bytes)),
        _ => None,
    }
}

/// Read frames until `setupComplete` (errors before it are setup failures).
async fn wait_for_setup(conn: &mut Connection) -> BlueyResult<()> {
    loop {
        match conn.socket.next().await {
            Some(Ok(Message::Close(_))) | None => {
                return Err(BlueyError::network(
                    "stream",
                    "the Gemini Live socket closed during setup",
                ))
            }
            Some(Ok(message)) => match decode_frame(&message) {
                Some(LiveEvent::SetupComplete) => return Ok(()),
                Some(LiveEvent::Error(status)) => return Err(map_live_error(&status)),
                Some(other) => {
                    tracing::debug!(kind = other.kind(), "frame before setupComplete ignored")
                }
                None => {}
            },
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
    fn numeric_codes_and_handshake_statuses_map_too() {
        assert_eq!(map_live_error("401").code, "config.api_key_invalid");
        assert_eq!(map_live_error(" 403 ").code, "config.api_key_invalid");
        assert_eq!(map_live_error("404").code, "config.model_not_found");
        assert_eq!(map_live_error("429").code, "network.http_429");
        assert!(is_fatal(&map_live_http_status(403)));
        assert!(is_fatal(&map_live_http_status(404)));
        assert!(!is_fatal(&map_live_http_status(429)));
        assert_eq!(map_live_http_status(502).code, "network.connect");
        assert!(!is_fatal(&map_live_http_status(502)));
    }

    #[test]
    fn binary_and_text_frames_decode_alike_and_control_frames_do_not() {
        let json = r#"{"setupComplete":{}}"#;
        assert_eq!(
            decode_frame(&Message::Text(json.into())),
            Some(LiveEvent::SetupComplete)
        );
        assert_eq!(
            decode_frame(&Message::Binary(json.as_bytes().to_vec().into())),
            Some(LiveEvent::SetupComplete),
            "the service answers on binary frames"
        );
        assert_eq!(decode_frame(&Message::Ping(Vec::new().into())), None);
        assert_eq!(decode_frame(&Message::Pong(Vec::new().into())), None);
    }

    #[test]
    fn rotation_happens_before_the_service_cap() {
        assert!(SOFT_SESSION_LIMIT < Duration::from_secs(10 * 60));
        assert!(SILENCE_STREAM_END < Duration::from_secs(1));
    }

    #[test]
    fn reconnect_backoff_grows_and_caps() {
        assert_eq!(reconnect_backoff(1), Duration::from_millis(500));
        assert_eq!(reconnect_backoff(2), Duration::from_secs(1));
        assert_eq!(reconnect_backoff(4), Duration::from_secs(4));
        assert_eq!(reconnect_backoff(10), RECONNECT_BACKOFF_CAP);
    }
}
