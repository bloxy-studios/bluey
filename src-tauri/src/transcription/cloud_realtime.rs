//! Cloud realtime transcription over the shared OpenAI-realtime event codec.
//!
//! Foundry **Voice Live** (MAI-Transcribe) is the supported transport: the
//! helper's 16 kHz PCM16 goes straight in, the Foundry resource key travels in
//! the `api-key` header and the companion chat model on the URL never replies
//! (`create_response: false`). The OpenAI realtime endpoint expects 24 kHz
//! audio, which the helper does not produce; selecting it yields a
//! `not_supported` error so the audio manager falls back to on-device speech.

use std::time::Duration;

use async_trait::async_trait;
use bluey_core::error::RecoveryAction;
use bluey_core::types::TranscriptionProviderKind;
use bluey_core::{BlueyError, BlueyErrorKind, BlueyResult};
use bluey_protocols::azure;
use bluey_protocols::realtime::{self, RealtimeEvent};
use bluey_protocols::voice_live::{self, TranscriptionTransport};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::reconnect::{is_fatal, reopen, Command, Outage, Reopened, Watchdog, SEND_TIMEOUT};
use super::{
    session_closed, EventSink, PcmChunk, SessionOptions, TranscriptionEvent, TranscriptionProvider,
    TranscriptionSession,
};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// A send that stalls (a half-open socket) fails like a closed one.
async fn send(socket: &mut Socket, value: serde_json::Value) -> BlueyResult<()> {
    let sent = socket.send(Message::Text(value.to_string().into()));
    match tokio::time::timeout(SEND_TIMEOUT, sent).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(BlueyError::network(
            "stream",
            "the Voice Live socket closed",
        )),
        Err(_) => Err(BlueyError::network(
            "timeout",
            "the Voice Live socket stalled",
        )),
    }
}

/// Map the HTTP status of a rejected WebSocket upgrade. A refused key or a
/// wrong resource/deployment is configuration (never retried in a loop).
pub fn map_http_status(status: u16) -> BlueyError {
    match status {
        401 | 403 => BlueyError::new(
            BlueyErrorKind::Configuration,
            "config.api_key_invalid",
            "Foundry rejected the API key for live transcription",
        )
        .recoverable(RecoveryAction::ConfigureProvider),
        404 => BlueyError::new(
            BlueyErrorKind::Configuration,
            "config.model_not_found",
            "the Voice Live endpoint or model was not found for this resource",
        )
        .recoverable(RecoveryAction::ConfigureProvider),
        other => BlueyError::network(
            "connect",
            format!("Foundry Voice Live refused the connection (HTTP {other})"),
        ),
    }
}

const MAX_CONNECT_ATTEMPTS: u32 = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Server-caused reconnects (errors, closes) tolerated without any transcript
/// progress in between; the audio manager re-opens the source after that.
const MAX_RECONNECTS_WITHOUT_PROGRESS: u32 = 5;
const COMMAND_BUFFER: usize = 256;
const DRAIN_GRACE: Duration = Duration::from_secs(2);

pub struct CloudRealtimeProvider {
    base_url: String,
    api_key: String,
    companion_model: String,
}

impl CloudRealtimeProvider {
    /// `companion_model` is the Voice Live chat model on the URL query
    /// (`BLUEY_MODEL_VOICE_LIVE`, default `gpt-4.1-mini`) — not the STT model.
    pub fn new(base_url: String, api_key: String, companion_model: Option<String>) -> Self {
        Self {
            base_url,
            api_key,
            companion_model: companion_model
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| voice_live::DEFAULT_COMPANION_MODEL.to_string()),
        }
    }
}

#[async_trait]
impl TranscriptionProvider for CloudRealtimeProvider {
    fn kind(&self) -> TranscriptionProviderKind {
        TranscriptionProviderKind::CloudRealtime
    }

    async fn open(
        &self,
        options: SessionOptions,
        sink: EventSink,
    ) -> BlueyResult<Box<dyn TranscriptionSession>> {
        if voice_live::transport_for_model(&options.model) == TranscriptionTransport::OpenaiRealtime
        {
            return Err(BlueyError::not_supported(
                "openai_realtime_sample_rate",
                "the OpenAI realtime transcription endpoint expects 24 kHz audio; pick MAI-Transcribe (Voice Live) or Gemini Live",
            ));
        }
        let (tx, rx) = mpsc::channel::<Command>(COMMAND_BUFFER);
        let worker = Worker {
            url: azure::voice_live_url(&self.base_url, &self.companion_model),
            api_key: self.api_key.clone(),
            options,
            sink,
        };
        let task = tauri::async_runtime::spawn(async move { worker.run(rx).await });
        Ok(Box::new(RealtimeSession {
            tx,
            task: parking_lot::Mutex::new(Some(task)),
        }))
    }
}

struct RealtimeSession {
    tx: mpsc::Sender<Command>,
    task: parking_lot::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

#[async_trait]
impl TranscriptionSession for RealtimeSession {
    async fn push_audio(&self, chunk: PcmChunk) -> BlueyResult<()> {
        // Never block the capture pipeline: a chunk that does not fit while the
        // worker reconnects is dropped.
        match self.tx.try_send(Command::Audio(chunk)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(BlueyError::transcription(
                "backpressure",
                "the Voice Live session is not keeping up; dropping audio",
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
                tracing::warn!("voice live worker did not finish draining; aborting it");
                task.abort();
            }
        }
    }
}

struct Worker {
    url: String,
    api_key: String,
    options: SessionOptions,
    sink: EventSink,
}

impl Worker {
    async fn connect(&self) -> BlueyResult<Socket> {
        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            let mut request = self.url.as_str().into_client_request().map_err(|_| {
                BlueyError::configuration("invalid_endpoint", "invalid Voice Live endpoint")
            })?;
            let key = self.api_key.parse().map_err(|_| {
                BlueyError::configuration("invalid_key", "the API key contains invalid characters")
            })?;
            request.headers_mut().insert("api-key", key);
            match tokio::time::timeout(CONNECT_TIMEOUT, connect_async(request)).await {
                Ok(Ok((mut socket, _))) => {
                    let update = voice_live::session_update_transcription_only(
                        &self.options.model,
                        self.options.language.as_deref(),
                    );
                    send(&mut socket, update).await?;
                    tracing::info!(source = ?self.options.source, model = %self.options.model, "voice live transcription session ready");
                    return Ok(socket);
                }
                Ok(Err(tungstenite::Error::Http(response))) => {
                    let error = map_http_status(response.status().as_u16());
                    tracing::warn!(attempt, error = %error, "voice live handshake rejected");
                    if is_fatal(&error) {
                        return Err(error);
                    }
                }
                Ok(Err(error)) => {
                    tracing::warn!(attempt, error = %error, "voice live connect failed");
                }
                Err(_) => tracing::warn!(attempt, "voice live connect timed out"),
            }
            if attempt < MAX_CONNECT_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(400 * u64::from(attempt))).await;
            }
        }
        Err(BlueyError::network(
            "connect",
            "could not connect to Foundry Voice Live",
        ))
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
        let mut outage = Outage::new(self.options.source, self.sink.clone());
        let mut socket = match reopen(&mut rx, &mut outage, 0, || self.connect()).await {
            Reopened::Open(socket) => socket,
            Reopened::Closed => return,
            Reopened::Fatal(error) => return self.fail(error).await,
        };
        let mut watchdog = Watchdog::default();
        // Reconnects since the last transcript event (a healthy session resets it).
        let mut reconnects = 0u32;
        // Voice Live deltas are increments; the interim shown to the user is the
        // accumulated utterance, keyed by item id.
        let mut partial: Option<(Option<String>, String)> = None;
        loop {
            // `Some(counted)`: reconnect; a stall is not counted (see gemini_live).
            let mut reconnect = None;
            tokio::select! {
                command = rx.recv() => match command {
                    None | Some(Command::Close) => break,
                    Some(Command::Audio(chunk)) => {
                        let frame = realtime::append_audio(&chunk.base64);
                        if send(&mut socket, frame).await.is_err() {
                            reconnect = Some(true);
                        } else if chunk.is_speech {
                            watchdog.sent_speech();
                        }
                    }
                },
                message = socket.next() => {
                    if matches!(message, Some(Ok(_))) {
                        watchdog.heard();
                    }
                    match message {
                    Some(Ok(Message::Text(text))) => match realtime::parse_event(&text) {
                        RealtimeEvent::Delta { item_id, text } => {
                            reconnects = 0;
                            let accumulated = match &mut partial {
                                Some((id, acc)) if *id == item_id => {
                                    acc.push_str(&text);
                                    acc.clone()
                                }
                                _ => {
                                    partial = Some((item_id, text.clone()));
                                    text
                                }
                            };
                            if !accumulated.trim().is_empty() {
                                let _ = self.sink.send(TranscriptionEvent::Interim { source: self.options.source, text: accumulated }).await;
                            }
                        }
                        RealtimeEvent::Completed { transcript, .. } => {
                            reconnects = 0;
                            partial = None;
                            if !transcript.trim().is_empty() {
                                let _ = self.sink.send(TranscriptionEvent::Final { source: self.options.source, text: transcript, language: None }).await;
                            }
                        }
                        RealtimeEvent::Failed { .. } => {
                            tracing::debug!("voice live dropped one utterance");
                        }
                        RealtimeEvent::Error { .. } => {
                            return self.fail(BlueyError::transcription("realtime_error", "the Voice Live session reported an error")).await;
                        }
                        RealtimeEvent::Other => {}
                    },
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::info!("voice live socket closed by the server; reconnecting");
                        reconnect = Some(true);
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::warn!(error = %error, "voice live socket error");
                        reconnect = Some(true);
                    }
                    }
                },
                () = watchdog.expired() => {
                    tracing::warn!(source = ?self.options.source, "voice live stopped answering; reconnecting");
                    reconnect = Some(false);
                }
            }
            let Some(counted) = reconnect else { continue };
            let mut attempt = 0;
            if counted {
                reconnects += 1;
                if reconnects > MAX_RECONNECTS_WITHOUT_PROGRESS {
                    // The manager re-opens the source after a cool-down.
                    return self
                        .fail(BlueyError::network(
                            "stream",
                            "the Voice Live socket keeps closing",
                        ))
                        .await;
                }
                attempt = reconnects;
            }
            partial = None;
            watchdog = Watchdog::default();
            // Until the service is back (or the session closes); audio in the
            // meantime is lost.
            socket = match reopen(&mut rx, &mut outage, attempt, || self.connect()).await {
                Reopened::Open(fresh) => fresh,
                Reopened::Closed => return,
                Reopened::Fatal(error) => return self.fail(error).await,
            };
        }
        // Closing: give the service a moment to flush the last utterance.
        let deadline = tokio::time::Instant::now() + DRAIN_GRACE;
        loop {
            match tokio::time::timeout_at(deadline, socket.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    if let RealtimeEvent::Completed { transcript, .. } =
                        realtime::parse_event(&text)
                    {
                        if !transcript.trim().is_empty() {
                            let _ = self
                                .sink
                                .send(TranscriptionEvent::Final {
                                    source: self.options.source,
                                    text: transcript,
                                    language: None,
                                })
                                .await;
                        }
                    }
                }
                Ok(Some(Ok(_))) => {}
                _ => break,
            }
        }
        let _ = socket.close(None).await;
    }
}
