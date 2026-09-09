//! Audio manager: drives the helper's `audio.*` methods, consumes its
//! `audio.*` / `transcript.*` events, assembles transcript segments (stable ids
//! across partials, speaker labels from the channel), keeps a ring buffer of
//! recent finals for the context snapshot, persists finals when the privacy
//! settings allow and mirrors everything onto the bus.
//!
//! Two routes: **Apple** (the helper transcribes on device and emits
//! `transcript.*`) and **PCM** (the helper emits `audio.chunk { pcm16 }` and a
//! [`crate::transcription::TranscriptionProvider`] — Gemini Live by default,
//! Foundry Voice Live, or the mock — transcribes it, one session per source).
//! A cloud provider without a usable key falls back to Apple with a non-fatal
//! `audio.error{code: stt_fallback}` so listening never silently fails.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bluey_core::events::BlueyEvent;
use bluey_core::presets;
use bluey_core::types::{
    AiProviderKind, AppEvent, AudioDevice, AudioLevels, AudioSessionConfig,
    AudioSessionConfigPatch, AudioSessionState, AudioSource, AudioSourcePreference, AudioStatus,
    TranscriptSegment, TranscriptionProviderKind,
};
use bluey_core::{new_id, now_iso, BlueyError, BlueyResult};
use bluey_protocols::helper::{DevicesResult, HelperEvent, TranscriptAssembler, WireTranscript};
use bluey_protocols::voice_live::{self, TranscriptionTransport};
use bluey_storage::TranscriptRepository;
use serde::Serialize;
use serde_json::{json, Value};

use crate::events::EventBus;
use crate::modes::ModeManager;
use crate::secrets::{provider_key, SecretsStore};
use crate::sessions::SessionManager;
use crate::settings::SettingsManager;
use crate::sidecar::HelperClient;
use crate::state::StateHub;
use crate::storage::Storage;
use crate::transcription::cloud_realtime::CloudRealtimeProvider;
use crate::transcription::gemini_live::GeminiLiveProvider;
use crate::transcription::mock::MockTranscriptionProvider;
use crate::transcription::{
    self, PcmChunk, SessionOptions, TranscriptionEvent, TranscriptionProvider, TranscriptionSession,
};

/// Helper capture sample rate (mono PCM16).
pub const SAMPLE_RATE_HZ: u32 = 16_000;
/// Chunk length for the on-device path.
const APPLE_CHUNK_MS: u32 = 200;
/// Finals kept in memory for the context snapshot.
const RING_CAPACITY: usize = 500;

/// Result of `audio_test_microphone`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneTest {
    pub peak_level: f32,
    pub ok: bool,
}

/// Which transcription path the current session uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptionRoute {
    /// Apple Speech inside the helper (events arrive as `transcript.*`).
    Apple,
    /// The helper emits PCM; a [`TranscriptionProvider`] transcribes it.
    Pcm,
}

/// Decide how a configured provider is served. `cloud_ready` says whether a
/// provider session can actually be opened (key + model present); when it
/// cannot, the route falls back to Apple with a human-readable reason.
pub fn route_for(
    provider: TranscriptionProviderKind,
    cloud_ready: bool,
) -> (TranscriptionRoute, Option<&'static str>) {
    match provider {
        TranscriptionProviderKind::Apple => (TranscriptionRoute::Apple, None),
        _ if cloud_ready => (TranscriptionRoute::Pcm, None),
        TranscriptionProviderKind::GeminiLive => (
            TranscriptionRoute::Apple,
            Some("no Google AI Studio key is stored for live transcription"),
        ),
        TranscriptionProviderKind::CloudRealtime => (
            TranscriptionRoute::Apple,
            Some("cloud realtime transcription needs a Foundry MAI-Transcribe deployment with a stored key (the OpenAI realtime endpoint expects 24 kHz audio)"),
        ),
        TranscriptionProviderKind::Mock => (
            TranscriptionRoute::Apple,
            Some("mock transcription is only available in developer mode"),
        ),
    }
}

/// Live cloud transcription state for the running session.
struct ActiveStt {
    provider: Arc<dyn TranscriptionProvider>,
    model: String,
    language: Option<String>,
    /// Sources whose provider session is being opened (chunks meanwhile are dropped).
    opening: HashSet<AudioSource>,
    sessions: HashMap<AudioSource, Box<dyn TranscriptionSession>>,
    failed: HashSet<AudioSource>,
    sink: transcription::EventSink,
    pump: tauri::async_runtime::JoinHandle<()>,
}

/// Chunk timing per source, used to stamp cloud transcripts.
#[derive(Debug, Clone, Copy, Default)]
struct ChunkTiming {
    utterance_start_ms: Option<u64>,
    last_start_ms: u64,
    last_end_ms: u64,
}

/// Build the helper `audio.start` params from a session config.
pub fn helper_start_params(config: &AudioSessionConfig, route: TranscriptionRoute) -> Value {
    let mut sources = Vec::new();
    if config.microphone.enabled {
        sources.push("microphone");
    }
    if config.system_audio.enabled {
        sources.push("system");
    }
    let mut transcription = json!({
        "enabled": route == TranscriptionRoute::Apple,
        "onDevice": true,
        "sources": sources,
    });
    if config.transcription.language != "auto" && !config.transcription.language.is_empty() {
        transcription["locale"] = json!(config.transcription.language);
    }
    json!({
        "microphone": { "enabled": config.microphone.enabled, "deviceId": config.microphone.device_id },
        "systemAudio": { "enabled": config.system_audio.enabled },
        "sampleRate": SAMPLE_RATE_HZ,
        "vad": { "enabled": config.vad.enabled, "sensitivity": config.vad.sensitivity },
        "emitPcm": route == TranscriptionRoute::Pcm,
        "chunkMs": APPLE_CHUNK_MS,
        "transcription": transcription,
        "levels": { "enabled": true, "intervalMs": 100 },
    })
}

pub struct AudioManager {
    helper: Arc<HelperClient>,
    bus: Arc<EventBus>,
    hub: Arc<StateHub>,
    settings: Arc<SettingsManager>,
    storage: Arc<Storage>,
    sessions: Arc<SessionManager>,
    modes: Arc<ModeManager>,
    secrets: Arc<SecretsStore>,
    stt: tokio::sync::Mutex<Option<ActiveStt>>,
    chunk_times: parking_lot::Mutex<HashMap<AudioSource, ChunkTiming>>,
    status: parking_lot::Mutex<AudioStatus>,
    config: parking_lot::Mutex<Option<AudioSessionConfig>>,
    ring: parking_lot::Mutex<VecDeque<TranscriptSegment>>,
    partials: parking_lot::Mutex<HashMap<AudioSource, TranscriptSegment>>,
    assembler: parking_lot::Mutex<TranscriptAssembler>,
    /// The session was started by `audio_start` and ends with it.
    auto_session: AtomicBool,
    listener_started: AtomicBool,
}

impl AudioManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        helper: Arc<HelperClient>,
        bus: Arc<EventBus>,
        hub: Arc<StateHub>,
        settings: Arc<SettingsManager>,
        storage: Arc<Storage>,
        sessions: Arc<SessionManager>,
        modes: Arc<ModeManager>,
        secrets: Arc<SecretsStore>,
    ) -> Self {
        Self {
            helper,
            bus,
            hub,
            settings,
            storage,
            sessions,
            modes,
            secrets,
            stt: tokio::sync::Mutex::new(None),
            chunk_times: parking_lot::Mutex::new(HashMap::new()),
            status: parking_lot::Mutex::new(AudioStatus::default()),
            config: parking_lot::Mutex::new(None),
            ring: parking_lot::Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
            partials: parking_lot::Mutex::new(HashMap::new()),
            assembler: parking_lot::Mutex::new(TranscriptAssembler::new()),
            auto_session: AtomicBool::new(false),
            listener_started: AtomicBool::new(false),
        }
    }

    /// Current status snapshot.
    pub fn status(&self) -> AudioStatus {
        self.status.lock().clone()
    }

    /// Whether a session is running or paused.
    pub fn is_running(&self) -> bool {
        matches!(
            self.status().state,
            AudioSessionState::Running | AudioSessionState::Paused | AudioSessionState::Starting
        )
    }

    /// The session config resolved from settings (+ optional patch).
    fn resolve_config(&self, patch: Option<AudioSessionConfigPatch>) -> AudioSessionConfig {
        let settings = self.settings.get();
        let mut config = AudioSessionConfig::default();
        config.microphone.enabled = settings.audio.source != AudioSourcePreference::System;
        config.microphone.device_id = settings.audio.microphone_device_id.clone();
        config.system_audio.enabled = settings.audio.source != AudioSourcePreference::Microphone;
        config.transcription.provider = settings.audio.transcription_provider;
        config.transcription.language = settings.audio.transcription_language.clone();
        config.transcription.speaker_identification = settings.audio.speaker_identification;
        config.vad.sensitivity = settings.audio.vad_sensitivity;
        config.retain_raw_audio = settings.privacy.store_raw_audio;
        match patch {
            Some(patch) => config.apply(patch),
            None => config,
        }
    }

    pub async fn list_devices(&self) -> BlueyResult<Vec<AudioDevice>> {
        let value = self.helper.call("audio.devices", json!({})).await?;
        let result: DevicesResult = serde_json::from_value(value)
            .map_err(|_| BlueyError::internal("malformed audio devices response"))?;
        Ok(result.devices)
    }

    /// Start listening. Starts a session when none is active.
    pub async fn start(
        self: &Arc<Self>,
        patch: Option<AudioSessionConfigPatch>,
    ) -> BlueyResult<AudioStatus> {
        if self.is_running() {
            return Ok(self.status());
        }
        let config = self.resolve_config(patch);
        if !config.microphone.enabled && !config.system_audio.enabled {
            return Err(BlueyError::audio(
                "no_source",
                "enable the microphone or system audio first",
            ));
        }
        let cloud = self.cloud_provider(&config).await;
        let (route, fallback) = route_for(config.transcription.provider, cloud.is_some());
        {
            let mut status = self.status.lock();
            status.state = AudioSessionState::Starting;
            status.error = None;
        }
        let params = helper_start_params(&config, route);
        // Reset per-session state and install the cloud transcription sink
        // *before* the helper starts capturing, so the first PCM chunks are not
        // dropped for lack of a session.
        self.assembler.lock().reset();
        self.partials.lock().clear();
        self.chunk_times.lock().clear();
        *self.config.lock() = Some(config.clone());
        if route == TranscriptionRoute::Pcm {
            if let Some((provider, model)) = cloud {
                let (tx, mut rx) = tokio::sync::mpsc::channel::<TranscriptionEvent>(512);
                let this = self.clone();
                let pump = tauri::async_runtime::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        this.on_stt_event(event).await;
                    }
                });
                let language = Some(config.transcription.language.clone())
                    .filter(|l| !l.is_empty() && l != "auto");
                tracing::info!(provider = ?provider.kind(), model = %model, "cloud transcription active");
                *self.stt.lock().await = Some(ActiveStt {
                    provider,
                    model,
                    language,
                    opening: HashSet::new(),
                    sessions: HashMap::new(),
                    failed: HashSet::new(),
                    sink: tx,
                    pump,
                });
            }
        }
        let value = match self.helper.call("audio.start", params).await {
            Ok(value) => value,
            Err(error) => {
                self.close_stt().await;
                *self.config.lock() = None;
                let mut status = self.status.lock();
                status.state = AudioSessionState::Error;
                status.error = Some(error.clone());
                drop(status);
                self.bus.publish(BlueyEvent::AudioError(error.clone()));
                return Err(error);
            }
        };
        let microphone = value
            .get("microphone")
            .and_then(Value::as_bool)
            .unwrap_or(config.microphone.enabled);
        let system_audio = value
            .get("systemAudio")
            .and_then(Value::as_bool)
            .unwrap_or(config.system_audio.enabled);

        if self.sessions.active().is_none() {
            match self.sessions.start(None, None).await {
                Ok(_) => self.auto_session.store(true, Ordering::SeqCst),
                Err(e) => tracing::warn!(error = %e, "could not start a session for listening"),
            }
        }
        let status = {
            let mut status = self.status.lock();
            status.state = AudioSessionState::Running;
            status.microphone_active = microphone;
            status.system_audio_active = system_audio;
            status.provider = Some(if fallback.is_some() {
                TranscriptionProviderKind::Apple
            } else {
                config.transcription.provider
            });
            status.started_at = Some(now_iso());
            status.levels = Some(AudioLevels::default());
            status.error = None;
            status.clone()
        };
        self.hub.transition_soft(AppEvent::AudioStarted);
        self.bus.publish(BlueyEvent::AudioStarted(status.clone()));
        if let Some(reason) = fallback {
            self.bus.publish(BlueyEvent::AudioError(BlueyError::audio(
                "stt_fallback",
                format!("{reason}; using on-device Apple Speech"),
            )));
        }
        Ok(status)
    }

    /// Build the cloud transcription provider for the configured kind, or
    /// `None` when it cannot run (no key / model / developer mode). Returns the
    /// provider and the model id to open sessions with.
    async fn cloud_provider(
        &self,
        config: &AudioSessionConfig,
    ) -> Option<(Arc<dyn TranscriptionProvider>, String)> {
        let settings = self.settings.get();
        let assignment = settings.ai.models.transcription.clone();
        match config.transcription.provider {
            TranscriptionProviderKind::Apple => None,
            TranscriptionProviderKind::Mock => {
                let allowed = cfg!(feature = "dev-tools")
                    || cfg!(debug_assertions)
                    || settings.general.developer_mode;
                allowed.then(|| {
                    (
                        Arc::new(MockTranscriptionProvider) as Arc<dyn TranscriptionProvider>,
                        "mock".to_string(),
                    )
                })
            }
            TranscriptionProviderKind::GeminiLive => {
                let mut candidates: Vec<_> = settings
                    .ai
                    .providers
                    .iter()
                    .filter(|p| p.kind == AiProviderKind::GoogleGemini && p.enabled)
                    .collect();
                candidates.sort_by_key(|p| p.id != presets::GEMINI_ID);
                let provider = candidates.first()?;
                let key = self
                    .secrets
                    .get(&provider_key(&provider.id))
                    .await
                    .ok()
                    .flatten()?;
                let assigned = assignment
                    .as_ref()
                    .filter(|a| a.provider_id == provider.id)
                    .map(|a| a.model.as_str());
                let model = transcription::gemini_live_model(assigned);
                Some((Arc::new(GeminiLiveProvider::new(key)), model))
            }
            TranscriptionProviderKind::CloudRealtime => {
                let assignment = assignment?;
                let provider = settings
                    .ai
                    .providers
                    .iter()
                    .find(|p| p.id == assignment.provider_id && p.enabled)?;
                if !matches!(
                    provider.kind,
                    AiProviderKind::AzureFoundry | AiProviderKind::OpenaiCompatible
                ) {
                    return None;
                }
                // The OpenAI realtime endpoint expects 24 kHz audio the helper does
                // not produce; deciding here (not at `open`) lets the session fall
                // back to Apple instead of silently transcribing nothing.
                if voice_live::transport_for_model(&assignment.model)
                    == TranscriptionTransport::OpenaiRealtime
                {
                    return None;
                }
                let key = self
                    .secrets
                    .get(&provider_key(&provider.id))
                    .await
                    .ok()
                    .flatten()?;
                let companion = std::env::var("BLUEY_MODEL_VOICE_LIVE").ok();
                Some((
                    Arc::new(CloudRealtimeProvider::new(
                        provider.base_url.clone(),
                        key,
                        companion,
                    )),
                    assignment.model,
                ))
            }
        }
    }

    /// Forward one PCM chunk to the provider session for its source (opening
    /// the session on first use). Audio bytes are never retained.
    async fn forward_pcm(&self, chunk: PcmChunk) {
        {
            let mut times = self.chunk_times.lock();
            let timing = times.entry(chunk.source).or_default();
            if chunk.is_speech && timing.utterance_start_ms.is_none() {
                timing.utterance_start_ms = Some(chunk.start_ms);
            }
            timing.last_start_ms = chunk.start_ms;
            timing.last_end_ms = chunk.end_ms;
        }
        let source = chunk.source;
        // Fast path: a session exists — hand the chunk over (providers never
        // block on `push_audio`, so holding the lock here is fine).
        let (provider, options, sink) = {
            let mut guard = self.stt.lock().await;
            let Some(stt) = guard.as_mut() else { return };
            if stt.failed.contains(&source) || stt.opening.contains(&source) {
                return;
            }
            if let Some(session) = stt.sessions.get(&source) {
                if let Err(error) = session.push_audio(chunk).await {
                    tracing::debug!(error = %error, "dropping a pcm chunk");
                }
                return;
            }
            stt.opening.insert(source);
            (
                stt.provider.clone(),
                SessionOptions {
                    source,
                    model: stt.model.clone(),
                    language: stt.language.clone(),
                    vocabulary: Vec::new(),
                },
                stt.sink.clone(),
            )
        };
        // Slow path: open the provider session *without* holding the lock — a
        // connect can take seconds and this runs on the helper event loop.
        let opened = provider.open(options, sink).await;
        let stale_session = {
            let mut guard = self.stt.lock().await;
            match guard.as_mut() {
                Some(stt) => {
                    stt.opening.remove(&source);
                    match opened {
                        Ok(session) => {
                            if let Err(error) = session.push_audio(chunk).await {
                                tracing::debug!(error = %error, "dropping a pcm chunk");
                            }
                            stt.sessions.insert(source, session);
                        }
                        Err(error) => {
                            stt.failed.insert(source);
                            self.bus.publish(BlueyEvent::AudioError(error));
                        }
                    }
                    None
                }
                // Listening stopped while we were connecting.
                None => opened.ok(),
            }
        };
        if let Some(session) = stale_session {
            session.close().await;
        }
    }

    /// Provider events → transcript segments (same assembler as the Apple path).
    async fn on_stt_event(&self, event: TranscriptionEvent) {
        match event {
            TranscriptionEvent::Interim { source, text } => {
                self.on_cloud_text(source, text, None, false).await
            }
            TranscriptionEvent::Final {
                source,
                text,
                language,
            } => self.on_cloud_text(source, text, language, true).await,
            TranscriptionEvent::Failed { source, error } => {
                tracing::warn!(?source, error = %error, "cloud transcription failed");
                if let Some(stt) = self.stt.lock().await.as_mut() {
                    stt.failed.insert(source);
                    stt.sessions.remove(&source);
                }
                self.status.lock().error = Some(error.clone());
                self.bus.publish(BlueyEvent::AudioError(error));
            }
        }
    }

    async fn on_cloud_text(
        &self,
        source: AudioSource,
        text: String,
        language: Option<String>,
        finalized: bool,
    ) {
        let (start_ms, end_ms) = {
            let mut times = self.chunk_times.lock();
            let timing = times.entry(source).or_default();
            let start = timing.utterance_start_ms.unwrap_or(timing.last_start_ms);
            let end = timing.last_end_ms.max(start);
            if finalized {
                timing.utterance_start_ms = None;
            }
            (start, end)
        };
        let wire = WireTranscript {
            source,
            text,
            start_ms,
            end_ms,
            confidence: None,
            locale: language,
        };
        self.on_transcript(wire, finalized).await;
    }

    /// Close every provider session and stop the event pump.
    async fn close_stt(&self) {
        let active = self.stt.lock().await.take();
        if let Some(stt) = active {
            // Close every source in parallel (each may drain for a few seconds).
            futures::future::join_all(stt.sessions.into_values().map(|session| async move {
                session.close().await;
            }))
            .await;
            drop(stt.sink);
            let mut pump = stt.pump;
            if tokio::time::timeout(Duration::from_secs(3), &mut pump)
                .await
                .is_err()
            {
                // A provider clone of the sink is still alive somewhere: stop the
                // pump so late events cannot land in a later session.
                pump.abort();
            }
        }
        self.chunk_times.lock().clear();
    }

    /// Stop listening (and end the auto-started session).
    pub async fn stop(&self) -> BlueyResult<AudioStatus> {
        if self.helper.is_running() {
            if let Err(e) = self.helper.request("audio.stop", json!({})).await {
                tracing::debug!(error = %e, "audio.stop failed (helper may be gone)");
            }
        }
        self.close_stt().await;
        let status = self.mark_stopped(None);
        if self.auto_session.swap(false, Ordering::SeqCst) {
            if let Err(e) = self.sessions.end().await {
                tracing::debug!(error = %e, "could not end the auto-started session");
            }
        }
        Ok(status)
    }

    fn mark_stopped(&self, error: Option<BlueyError>) -> AudioStatus {
        self.assembler.lock().reset();
        self.partials.lock().clear();
        *self.config.lock() = None;
        let status = {
            let mut status = self.status.lock();
            // `stop()` and the helper's `audio.stopped{requested}` both land here;
            // an already-stopped session is not stopped again (no duplicate events).
            if status.state == AudioSessionState::Stopped && error.is_none() {
                return status.clone();
            }
            status.state = if error.is_some() {
                AudioSessionState::Error
            } else {
                AudioSessionState::Stopped
            };
            status.microphone_active = false;
            status.system_audio_active = false;
            status.levels = None;
            status.started_at = None;
            status.error = error;
            status.clone()
        };
        self.hub.transition_soft(AppEvent::AudioStopped);
        self.bus.publish(BlueyEvent::AudioStopped(status.clone()));
        status
    }

    pub async fn pause(&self) -> BlueyResult<AudioStatus> {
        if self.status().state != AudioSessionState::Running {
            return Ok(self.status());
        }
        self.helper.request("audio.pause", json!({})).await?;
        let status = {
            let mut status = self.status.lock();
            status.state = AudioSessionState::Paused;
            status.clone()
        };
        self.bus.publish(BlueyEvent::AudioPaused(status.clone()));
        Ok(status)
    }

    pub async fn resume(&self) -> BlueyResult<AudioStatus> {
        if self.status().state != AudioSessionState::Paused {
            return Ok(self.status());
        }
        self.helper.request("audio.resume", json!({})).await?;
        let status = {
            let mut status = self.status.lock();
            status.state = AudioSessionState::Running;
            status.clone()
        };
        self.bus.publish(BlueyEvent::AudioResumed(status.clone()));
        Ok(status)
    }

    /// Short level measurement of the microphone (onboarding / settings check).
    pub async fn test_microphone(
        &self,
        device_id: Option<String>,
        duration_ms: Option<u32>,
    ) -> BlueyResult<MicrophoneTest> {
        let mut params = json!({ "durationMs": duration_ms.unwrap_or(1500) });
        if let Some(id) = device_id {
            params["deviceId"] = json!(id);
        }
        let value = self.helper.call("audio.testMicrophone", params).await?;
        Ok(MicrophoneTest {
            peak_level: value
                .get("peakLevel")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32,
            ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    // ── Transcript access ──────────────────────────────────────────────────

    /// Finals from the last `window_seconds` (relative to the newest segment),
    /// oldest first — the context snapshot's transcript.
    pub fn recent(&self, window_seconds: u32) -> Vec<TranscriptSegment> {
        let ring = self.ring.lock();
        let Some(last_end) = ring.back().map(|s| s.end_time) else {
            return Vec::new();
        };
        let cutoff = last_end.saturating_sub(u64::from(window_seconds) * 1_000);
        ring.iter()
            .filter(|s| s.end_time >= cutoff)
            .cloned()
            .collect()
    }

    /// Stored segments (or the in-memory ring when transcripts are not persisted).
    pub async fn list(
        &self,
        session_id: Option<String>,
        since_ms: Option<u64>,
        limit: Option<u32>,
    ) -> BlueyResult<Vec<TranscriptSegment>> {
        if self.settings.get().privacy.store_transcripts && session_id.is_some() {
            return self
                .storage
                .run(move |db| {
                    TranscriptRepository::list(db, session_id.as_deref(), since_ms, limit)
                })
                .await;
        }
        let ring = self.ring.lock();
        let mut segments: Vec<TranscriptSegment> = ring
            .iter()
            .filter(|s| {
                session_id
                    .as_deref()
                    .is_none_or(|id| s.session_id.as_deref() == Some(id))
            })
            .filter(|s| since_ms.is_none_or(|since| s.start_time >= since))
            .cloned()
            .collect();
        if let Some(limit) = limit {
            let keep = limit as usize;
            if segments.len() > keep {
                segments.drain(..segments.len() - keep);
            }
        }
        Ok(segments)
    }

    /// Delete stored segments (one session or everything) and clear the ring.
    pub async fn clear(&self, session_id: Option<String>) -> BlueyResult<u64> {
        let target = session_id.clone();
        let removed = self
            .storage
            .run(move |db| TranscriptRepository::clear(db, target.as_deref()))
            .await?;
        {
            let mut ring = self.ring.lock();
            match &session_id {
                Some(id) => ring.retain(|s| s.session_id.as_deref() != Some(id.as_str())),
                None => ring.clear(),
            }
        }
        self.partials.lock().clear();
        self.bus
            .publish(BlueyEvent::TranscriptCleared { session_id });
        Ok(removed)
    }

    /// Developer mode: inject a finalized segment as if it had been heard.
    pub async fn push_simulated(
        &self,
        text: String,
        speaker: Option<String>,
        source: AudioSource,
    ) -> TranscriptSegment {
        let start = self
            .ring
            .lock()
            .back()
            .map(|s| s.end_time + 800)
            .unwrap_or(0);
        let segment = TranscriptSegment {
            id: new_id("seg"),
            session_id: self.sessions.active_id(),
            speaker_confidence: speaker.as_ref().map(|_| 0.72),
            speaker,
            source,
            text,
            start_time: start,
            end_time: start + 3_200,
            confidence: Some(0.94),
            finalized: true,
            language: Some("en".into()),
            created_at: now_iso(),
        };
        self.commit_final(segment.clone()).await;
        segment
    }

    // ── Helper event consumption ───────────────────────────────────────────

    /// Start consuming helper events (idempotent; call once after construction).
    pub fn start_listener(self: &Arc<Self>) {
        if self.listener_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut rx = this.helper.subscribe();
            loop {
                match rx.recv().await {
                    Ok(event) => this.handle_helper_event(event).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "audio event consumer lagged");
                    }
                    // The helper client lives as long as the app; a closed
                    // channel means shutdown.
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn handle_helper_event(&self, event: HelperEvent) {
        match event {
            HelperEvent::AudioStarted {
                microphone,
                system_audio,
                device,
            } => {
                let mut status = self.status.lock();
                status.microphone_active = microphone;
                status.system_audio_active = system_audio;
                if device.is_some() {
                    status.current_input_device = device;
                }
            }
            HelperEvent::AudioStopped { reason } => {
                if !self.is_running() {
                    return;
                }
                let error = match reason.as_str() {
                    "requested" => None,
                    "device_lost" => Some(BlueyError::audio(
                        "device_lost",
                        "the audio input device was disconnected",
                    )),
                    other => Some(BlueyError::audio(
                        "stopped",
                        format!("audio capture stopped ({other})"),
                    )),
                };
                if let Some(error) = &error {
                    self.bus.publish(BlueyEvent::AudioError(error.clone()));
                }
                self.close_stt().await;
                self.mark_stopped(error);
            }
            HelperEvent::AudioLevel { microphone, system } => {
                if let Some(levels) = self.status.lock().levels.as_mut() {
                    levels.microphone = microphone;
                    levels.system = system;
                }
                self.bus
                    .publish(BlueyEvent::AudioLevel { microphone, system });
            }
            HelperEvent::AudioChunk(chunk) => {
                // PCM (when requested) goes to the cloud transcription provider;
                // the contract event never carries audio bytes.
                if let Some(pcm16) = chunk.pcm16.clone() {
                    self.forward_pcm(PcmChunk {
                        source: chunk.source,
                        base64: pcm16,
                        sample_rate: chunk.sample_rate.unwrap_or(SAMPLE_RATE_HZ),
                        start_ms: chunk.start_ms,
                        end_ms: chunk.end_ms,
                        is_speech: chunk.is_speech,
                    })
                    .await;
                }
                self.bus.publish(BlueyEvent::AudioChunk {
                    source: chunk.source,
                    start_ms: chunk.start_ms,
                    end_ms: chunk.end_ms,
                    is_speech: chunk.is_speech,
                    rms: chunk.rms,
                });
            }
            HelperEvent::AudioDeviceChanged(change) => {
                if let Some(current) = &change.current_input {
                    self.status.lock().current_input_device = Some(current.clone());
                }
                self.bus.publish(BlueyEvent::AudioDeviceChanged {
                    devices: change.devices,
                    current_input: change.current_input,
                });
            }
            HelperEvent::AudioError(wire) => {
                let error = wire.into_bluey();
                self.status.lock().error = Some(error.clone());
                self.bus.publish(BlueyEvent::AudioError(error));
            }
            HelperEvent::TranscriptPartial(wire) => self.on_transcript(wire, false).await,
            HelperEvent::TranscriptFinal(wire) => self.on_transcript(wire, true).await,
            HelperEvent::Ready { .. }
            | HelperEvent::ScreenChanged { .. }
            | HelperEvent::Unknown { .. } => {}
        }
    }

    async fn on_transcript(&self, wire: WireTranscript, finalized: bool) {
        if wire.text.trim().is_empty() {
            return;
        }
        let speaker_identification = self
            .config
            .lock()
            .as_ref()
            .map(|c| c.transcription.speaker_identification)
            .unwrap_or(true);
        let session_id = self.sessions.active_id();
        let mode_id = self.modes.active_id();
        let segment = self.assembler.lock().ingest(
            &wire,
            finalized,
            session_id.as_deref(),
            &mode_id,
            speaker_identification,
            &now_iso(),
            || new_id("seg"),
        );
        if finalized {
            self.partials.lock().remove(&segment.source);
            self.commit_final(segment).await;
        } else {
            self.partials.lock().insert(segment.source, segment.clone());
            self.bus.publish(BlueyEvent::TranscriptPartial(segment));
        }
    }

    async fn commit_final(&self, segment: TranscriptSegment) {
        {
            let mut ring = self.ring.lock();
            if ring.len() >= RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(segment.clone());
        }
        if self.settings.get().privacy.store_transcripts && segment.session_id.is_some() {
            let stored = segment.clone();
            let result = self
                .storage
                .run(move |db| TranscriptRepository::upsert_partial(db, &stored))
                .await;
            if let Err(e) = result {
                tracing::warn!(error = %e, "failed to persist transcript segment");
            }
        }
        self.bus.publish(BlueyEvent::TranscriptFinal(segment));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::types::VadSensitivity;

    #[test]
    fn apple_route_asks_the_helper_to_transcribe_on_device() {
        let mut config = AudioSessionConfig::default();
        config.transcription.language = "auto".into();
        config.vad.sensitivity = VadSensitivity::High;
        let params = helper_start_params(&config, TranscriptionRoute::Apple);
        assert_eq!(params["sampleRate"], 16_000);
        assert_eq!(params["emitPcm"], false);
        assert_eq!(params["chunkMs"], 200);
        assert_eq!(params["transcription"]["enabled"], true);
        assert_eq!(params["transcription"]["onDevice"], true);
        assert!(
            params["transcription"].get("locale").is_none(),
            "auto → no locale"
        );
        assert_eq!(
            params["transcription"]["sources"],
            json!(["microphone", "system"])
        );
        assert_eq!(params["vad"]["sensitivity"], "high");
        assert_eq!(params["levels"]["enabled"], true);
    }

    #[test]
    fn pcm_route_emits_audio_and_skips_helper_transcription() {
        let mut config = AudioSessionConfig::default();
        config.system_audio.enabled = false;
        config.transcription.language = "en-US".into();
        config.microphone.device_id = Some("mic-1".into());
        let params = helper_start_params(&config, TranscriptionRoute::Pcm);
        assert_eq!(params["emitPcm"], true);
        assert_eq!(params["transcription"]["enabled"], false);
        assert_eq!(params["transcription"]["locale"], "en-US");
        assert_eq!(params["transcription"]["sources"], json!(["microphone"]));
        assert_eq!(params["microphone"]["deviceId"], "mic-1");
    }

    #[test]
    fn cloud_providers_route_to_pcm_when_ready_and_fall_back_to_apple_otherwise() {
        assert_eq!(
            route_for(TranscriptionProviderKind::Apple, true),
            (TranscriptionRoute::Apple, None)
        );
        assert_eq!(
            route_for(TranscriptionProviderKind::GeminiLive, true),
            (TranscriptionRoute::Pcm, None)
        );
        assert_eq!(
            route_for(TranscriptionProviderKind::CloudRealtime, true),
            (TranscriptionRoute::Pcm, None)
        );
        let (route, reason) = route_for(TranscriptionProviderKind::GeminiLive, false);
        assert_eq!(route, TranscriptionRoute::Apple);
        assert!(reason.unwrap().contains("Google AI Studio key"));
        let (route, reason) = route_for(TranscriptionProviderKind::CloudRealtime, false);
        assert_eq!(route, TranscriptionRoute::Apple);
        assert!(reason.is_some());
        assert_eq!(
            route_for(TranscriptionProviderKind::Mock, false).0,
            TranscriptionRoute::Apple
        );
    }

    #[test]
    fn gemini_live_is_the_default_provider() {
        assert_eq!(
            AudioSessionConfig::default().transcription.provider,
            TranscriptionProviderKind::GeminiLive
        );
    }
}
