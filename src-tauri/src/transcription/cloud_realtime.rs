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
use bluey_core::types::TranscriptionProviderKind;
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::azure;
use bluey_protocols::realtime::{self, RealtimeEvent};
use bluey_protocols::voice_live::{self, TranscriptionTransport};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::{
    session_closed, EventSink, PcmChunk, SessionOptions, TranscriptionEvent, TranscriptionProvider,
    TranscriptionSession,
};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const MAX_CONNECT_ATTEMPTS: u32 = 3;
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

enum Command {
    Audio(PcmChunk),
    Close,
}

struct RealtimeSession {
    tx: mpsc::Sender<Command>,
    task: parking_lot::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

#[async_trait]
impl TranscriptionSession for RealtimeSession {
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
            match connect_async(request).await {
                Ok((mut socket, _)) => {
                    let update = voice_live::session_update_transcription_only(
                        &self.options.model,
                        self.options.language.as_deref(),
                    );
                    socket
                        .send(Message::Text(update.to_string().into()))
                        .await
                        .map_err(|_| {
                            BlueyError::network("stream", "the Voice Live socket closed")
                        })?;
                    tracing::info!(source = ?self.options.source, model = %self.options.model, "voice live transcription session ready");
                    return Ok(socket);
                }
                Err(error) => {
                    tracing::warn!(attempt, error = %error, "voice live connect failed");
                    if attempt < MAX_CONNECT_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(400 * u64::from(attempt))).await;
                    }
                }
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
        let mut socket = match self.connect().await {
            Ok(socket) => socket,
            Err(error) => return self.fail(error).await,
        };
        let mut reconnects = 0u32;
        loop {
            tokio::select! {
                command = rx.recv() => match command {
                    None | Some(Command::Close) => break,
                    Some(Command::Audio(chunk)) => {
                        let frame = realtime::append_audio(&chunk.base64);
                        if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                            reconnects += 1;
                            if reconnects > MAX_CONNECT_ATTEMPTS {
                                return self.fail(BlueyError::network("stream", "the Voice Live socket keeps closing")).await;
                            }
                            match self.connect().await {
                                Ok(fresh) => socket = fresh,
                                Err(error) => return self.fail(error).await,
                            }
                        }
                    }
                },
                message = socket.next() => match message {
                    Some(Ok(Message::Text(text))) => match realtime::parse_event(&text) {
                        RealtimeEvent::Delta { text, .. } => {
                            if !text.trim().is_empty() {
                                let _ = self.sink.send(TranscriptionEvent::Interim { source: self.options.source, text }).await;
                            }
                        }
                        RealtimeEvent::Completed { transcript, .. } => {
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
                        reconnects += 1;
                        if reconnects > MAX_CONNECT_ATTEMPTS {
                            return self.fail(BlueyError::network("stream", "the Voice Live socket keeps closing")).await;
                        }
                        match self.connect().await {
                            Ok(fresh) => socket = fresh,
                            Err(error) => return self.fail(error).await,
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::warn!(error = %error, "voice live socket error");
                        match self.connect().await {
                            Ok(fresh) => socket = fresh,
                            Err(error) => return self.fail(error).await,
                        }
                    }
                },
            }
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
