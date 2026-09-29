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

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bluey_core::events::BlueyEvent;
use bluey_core::presets;
use bluey_core::types::{
    AiProviderKind, AppEvent, AppState, AudioDevice, AudioLevels, AudioSessionConfig,
    AudioSessionConfigPatch, AudioSessionState, AudioSource, AudioSourcePreference, AudioStatus,
    TranscriptSegment, TranscriptionProviderKind,
};
use bluey_core::{new_id, now_iso, BlueyError, BlueyErrorKind, BlueyResult};
use bluey_protocols::helper::{
    DevicesResult, HelperEvent, TranscriptAssembler, WireSpeechRoute, WireTranscript,
};
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

mod ring;
mod stt_health;

use ring::{RingScope, TranscriptRing};

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

/// Why a cloud provider is not used while Privacy → Cloud AI is off.
pub const CLOUD_AI_OFF_REASON: &str = "Cloud AI is turned off in Settings → Privacy";

/// Decide how a configured provider is served. `cloud_allowed` is the Privacy
/// → Cloud AI switch; `cloud_ready` says whether a provider session can
/// actually be opened (key + model present). When a cloud provider may not or
/// cannot run, the route falls back to Apple with a human-readable reason.
pub fn route_for(
    provider: TranscriptionProviderKind,
    cloud_allowed: bool,
    cloud_ready: bool,
) -> (TranscriptionRoute, Option<&'static str>) {
    match provider {
        TranscriptionProviderKind::Apple => (TranscriptionRoute::Apple, None),
        TranscriptionProviderKind::GeminiLive | TranscriptionProviderKind::CloudRealtime
            if !cloud_allowed =>
        {
            (TranscriptionRoute::Apple, Some(CLOUD_AI_OFF_REASON))
        }
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
    health: stt_health::SttHealth,
    sink: transcription::EventSink,
    pump: tauri::async_runtime::JoinHandle<()>,
}

/// Chunk timing per source, used to stamp cloud transcripts.
#[derive(Debug, Clone, Copy, Default)]
struct ChunkTiming {
    utterance_start_ms: Option<u64>,
    last_start_ms: u64,
    last_end_ms: u64,
    /// The cloud utterance its interims and final belong to (UX-010).
    open_utterance: Option<u64>,
    utterances: u64,
}

impl ChunkTiming {
    /// Stamp a cloud transcript with its span and utterance id. A provider
    /// streams one utterance at a time per source, so the first event opens
    /// an utterance and its final closes it; the id never depends on timing
    /// (an interim can arrive before the speech chunk that starts the span).
    fn stamp_cloud(&mut self, finalized: bool) -> (u64, u64, String) {
        let start = self.utterance_start_ms.unwrap_or(self.last_start_ms);
        let end = self.last_end_ms.max(start);
        let seq = match self.open_utterance {
            Some(seq) => seq,
            None => {
                self.utterances += 1;
                self.open_utterance = Some(self.utterances);
                self.utterances
            }
        };
        if finalized {
            self.utterance_start_ms = None;
            self.open_utterance = None;
        }
        (start, end, format!("cloud-{seq}"))
    }
}

/// The Apple route promises on-device transcription; when the recognizer has
/// no on-device model for the locale the helper falls back to Apple's servers,
/// which the user is told (MAC-007) instead of it happening silently.
fn server_speech_notice(route: &WireSpeechRoute) -> Option<BlueyError> {
    (!route.on_device).then(|| {
        BlueyError::audio(
            "speech_server",
            format!(
                "Apple Speech has no on-device model for {}, so it transcribes on Apple's servers",
                route.locale
            ),
        )
    })
}

/// Claim the start slot under the status lock. Only the claimant goes on to
/// start the helper; a start racing it (native shortcut + HUD, double click)
/// gets the current status back instead.
fn claim_start(status: &parking_lot::Mutex<AudioStatus>) -> Result<(), Box<AudioStatus>> {
    let mut status = status.lock();
    if matches!(
        status.state,
        AudioSessionState::Starting | AudioSessionState::Running | AudioSessionState::Paused
    ) {
        return Err(Box::new(status.clone()));
    }
    status.state = AudioSessionState::Starting;
    status.error = None;
    Ok(())
}

/// What a failed helper `audio.start` needs before it is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartRecovery {
    /// The helper still runs a session Rust lost track of: stop it and start
    /// once more.
    StopAndRetry,
    /// The helper may still finish starting after the timeout and keep the
    /// microphone open: stop it (best effort) and report the error.
    StopHelper,
    Report,
}

fn start_recovery(error: &BlueyError) -> StartRecovery {
    match error.code.as_str() {
        "audio.audio_already_running" => StartRecovery::StopAndRetry,
        "sidecar.timeout" => StartRecovery::StopHelper,
        _ => StartRecovery::Report,
    }
}

/// What the helper exiting means for the listening run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HelperExitPlan {
    /// The run was live: it stops (with an error) — no capture survives the
    /// process.
    stop_run: bool,
    /// Re-issue the run with its config once the replacement helper is up.
    resume: bool,
    /// The helper is gone for good: the run's auto-started session ends.
    end_auto_session: bool,
}

fn helper_exit_plan(state: AudioSessionState, restarting: bool) -> HelperExitPlan {
    let live = matches!(
        state,
        AudioSessionState::Running | AudioSessionState::Paused
    );
    HelperExitPlan {
        stop_run: live,
        resume: live && restarting,
        end_auto_session: !restarting,
    }
}

/// Move a run-relative transcript onto the session's timeline.
fn on_session_timeline(mut wire: WireTranscript, offset_ms: u64) -> WireTranscript {
    wire.start_ms = wire.start_ms.saturating_add(offset_ms);
    wire.end_ms = wire.end_ms.saturating_add(offset_ms);
    wire
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
    ring: parking_lot::Mutex<TranscriptRing>,
    /// The listening run the ring files new finals under (bumped by `start`).
    run_id: AtomicU64,
    partials: parking_lot::Mutex<HashMap<AudioSource, TranscriptSegment>>,
    assembler: parking_lot::Mutex<TranscriptAssembler>,
    /// The session `start` created for this listening run (it ends with the
    /// run, unless the user has switched to another session meanwhile).
    auto_session: parking_lot::Mutex<Option<String>>,
    /// Added to this run's segment times: a run attached to a session that
    /// already has a transcript continues its timeline.
    time_offset_ms: AtomicU64,
    /// The config of a run the helper died under, re-issued once the
    /// supervisor's replacement helper is up.
    resume: parking_lot::Mutex<Option<AudioSessionConfig>>,
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
            ring: parking_lot::Mutex::new(TranscriptRing::new(RING_CAPACITY)),
            run_id: AtomicU64::new(0),
            partials: parking_lot::Mutex::new(HashMap::new()),
            assembler: parking_lot::Mutex::new(TranscriptAssembler::new()),
            auto_session: parking_lot::Mutex::new(None),
            time_offset_ms: AtomicU64::new(0),
            resume: parking_lot::Mutex::new(None),
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
        // The sign-in gate is enforced here, not only in the UI: the shortcut,
        // the tray and the command all start listening through this call.
        // (Without a configured sign-in, boot never enters `AuthRequired`.)
        ensure_signed_in(self.hub.state())?;
        let config = self.resolve_config(patch);
        if !config.microphone.enabled && !config.system_audio.enabled {
            return Err(BlueyError::audio(
                "no_source",
                "enable the microphone or system audio first",
            ));
        }
        // Claimed before the first await, so a duplicate toggle cannot start
        // the helper twice (and then tear down the winner's transcription).
        if let Err(current) = claim_start(&self.status) {
            return Ok(*current);
        }
        self.start_claimed(config).await
    }

    /// The body of [`Self::start`] once the start slot is claimed.
    async fn start_claimed(
        self: &Arc<Self>,
        config: AudioSessionConfig,
    ) -> BlueyResult<AudioStatus> {
        let cloud = self.cloud_provider(&config).await;
        let cloud_allowed = self.settings.get().privacy.cloud_ai_enabled;
        let (route, fallback) = route_for(
            config.transcription.provider,
            cloud_allowed,
            cloud.is_some(),
        );
        let params = helper_start_params(&config, route);
        // Reset per-session state and install the cloud transcription sink
        // *before* the helper starts capturing, so the first PCM chunks are not
        // dropped for lack of a session.
        self.assembler.lock().reset();
        self.partials.lock().clear();
        self.chunk_times.lock().clear();
        // Finals of earlier runs stay in the ring (the transcript view may
        // still list them) but are out of this run's context scope.
        self.run_id.fetch_add(1, Ordering::SeqCst);
        // This run supersedes one waiting for a helper restart.
        self.resume.lock().take();
        // Segment times restart at 0 with every helper `audio.start`.
        let offset = match self.sessions.active_id() {
            Some(id) => self.session_last_end_ms(&id).await,
            None => 0,
        };
        self.time_offset_ms.store(offset, Ordering::SeqCst);
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
                    health: stt_health::SttHealth::default(),
                    sink: tx,
                    pump,
                });
            }
        }
        let value = match self.start_helper_audio(params).await {
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
                Ok(session) => *self.auto_session.lock() = Some(session.id),
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

    /// Move a live cloud session onto on-device Apple Speech (Privacy → Cloud
    /// AI turned off, or the provider rejected its configuration): the helper
    /// restarts capture with transcription on; the session and its timeline
    /// continue. A no-op unless listening on a cloud route.
    pub fn fall_back_to_apple(self: &Arc<Self>, reason: impl Into<String>) {
        let reason = reason.into();
        let this = self.clone();
        // Spawned: this may be called from the transcription pump, which
        // `close_stt` waits for.
        tauri::async_runtime::spawn(async move { this.reroute_to_apple(reason).await });
    }

    async fn reroute_to_apple(self: &Arc<Self>, reason: String) {
        let (config, was_paused) = {
            let mut status = self.status.lock();
            let on_cloud = matches!(
                status.provider,
                Some(
                    TranscriptionProviderKind::GeminiLive
                        | TranscriptionProviderKind::CloudRealtime
                )
            );
            let live = matches!(
                status.state,
                AudioSessionState::Running | AudioSessionState::Paused
            );
            let Some(mut config) = self.config.lock().clone() else {
                return;
            };
            if !live || !on_cloud {
                return;
            }
            let was_paused = status.state == AudioSessionState::Paused;
            // Owning the helper session now: its `audio.stopped{requested}`
            // is not a user stop.
            status.state = AudioSessionState::Starting;
            config.transcription.provider = TranscriptionProviderKind::Apple;
            (config, was_paused)
        };
        tracing::info!("moving live transcription to on-device Apple Speech");
        self.stop_helper_audio().await;
        self.close_stt().await;
        match self.start_claimed(config).await {
            Ok(_) => {
                self.bus.publish(BlueyEvent::AudioError(BlueyError::audio(
                    "stt_fallback",
                    format!("{reason}; using on-device Apple Speech"),
                )));
                if was_paused {
                    let _ = self.pause().await;
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "could not restart listening on Apple Speech");
            }
        }
    }

    /// `audio.start` on the helper, reconciling a helper session Rust lost
    /// track of (an earlier start that timed out here but finished there).
    async fn start_helper_audio(self: &Arc<Self>, params: Value) -> BlueyResult<Value> {
        let error = match self.helper.call("audio.start", params.clone()).await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        match start_recovery(&error) {
            StartRecovery::StopAndRetry => {
                tracing::info!("the helper was still capturing; restarting its audio session");
                self.stop_helper_audio().await;
                self.helper.call("audio.start", params).await
            }
            StartRecovery::StopHelper => {
                self.stop_helper_audio().await;
                Err(error)
            }
            StartRecovery::Report => Err(error),
        }
    }

    /// Best-effort `audio.stop` (the helper may be gone).
    async fn stop_helper_audio(&self) {
        if self.helper.is_running() {
            if let Err(e) = self.helper.request("audio.stop", json!({})).await {
                tracing::debug!(error = %e, "audio.stop failed (helper may be gone)");
            }
        }
    }

    /// Build the cloud transcription provider for the configured kind, or
    /// `None` when it cannot run (no key / model / developer mode, or Privacy
    /// → Cloud AI off: no audio leaves the Mac then). Returns the provider and
    /// the model id to open sessions with.
    async fn cloud_provider(
        &self,
        config: &AudioSessionConfig,
    ) -> Option<(Arc<dyn TranscriptionProvider>, String)> {
        let settings = self.settings.get();
        let assignment = settings.ai.models.transcription.clone();
        match config.transcription.provider {
            TranscriptionProviderKind::Apple => None,
            TranscriptionProviderKind::GeminiLive | TranscriptionProviderKind::CloudRealtime
                if !settings.privacy.cloud_ai_enabled =>
            {
                None
            }
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
    async fn forward_pcm(self: &Arc<Self>, chunk: PcmChunk) {
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
            if stt.opening.contains(&source)
                || !stt.health.may_forward(source, std::time::Instant::now())
            {
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
                        Err(error) => self.on_stt_failed(stt, source, error),
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
    async fn on_stt_event(self: &Arc<Self>, event: TranscriptionEvent) {
        match event {
            TranscriptionEvent::Interim { source, text } => {
                self.stt_healthy(source).await;
                self.on_cloud_text(source, text, None, false).await
            }
            TranscriptionEvent::Final {
                source,
                text,
                language,
            } => {
                self.stt_healthy(source).await;
                self.on_cloud_text(source, text, language, true).await
            }
            TranscriptionEvent::Degraded { source, error } => {
                tracing::warn!(?source, error = %error, "cloud transcription lost its connection; reconnecting");
                let announce = self
                    .stt
                    .lock()
                    .await
                    .as_mut()
                    .is_some_and(|stt| stt.health.degraded(source));
                if announce {
                    self.announce_stt_degraded();
                }
            }
            TranscriptionEvent::Recovered { source } => self.stt_healthy(source).await,
            TranscriptionEvent::Failed { source, error } => {
                if let Some(stt) = self.stt.lock().await.as_mut() {
                    self.on_stt_failed(stt, source, error);
                }
            }
        }
    }

    /// A provider session for `source` ended. Configuration the provider
    /// cannot run with moves listening to Apple Speech (retrying a bad key
    /// would only fail again); anything else is re-opened after a cool-down,
    /// announced once per outage.
    fn on_stt_failed(
        self: &Arc<Self>,
        stt: &mut ActiveStt,
        source: AudioSource,
        error: BlueyError,
    ) {
        tracing::warn!(?source, error = %error, "cloud transcription failed");
        stt.sessions.remove(&source);
        let announce = stt.health.failed(source, std::time::Instant::now());
        if matches!(
            error.kind,
            BlueyErrorKind::Configuration | BlueyErrorKind::NotSupported
        ) {
            let reason = error.message.clone();
            self.status.lock().error = Some(error.clone());
            self.bus.publish(BlueyEvent::AudioError(error));
            self.fall_back_to_apple(reason);
        } else if announce {
            self.announce_stt_degraded();
        }
    }

    fn announce_stt_degraded(&self) {
        let error = stt_health::degraded_error();
        self.status.lock().error = Some(error.clone());
        self.bus.publish(BlueyEvent::AudioError(error));
    }

    /// Transcripts flow for `source` again: clear the outage notice once no
    /// source is degraded.
    async fn stt_healthy(&self, source: AudioSource) {
        let ended = self
            .stt
            .lock()
            .await
            .as_mut()
            .is_some_and(|stt| stt.health.healthy(source));
        if ended {
            let mut status = self.status.lock();
            if status
                .error
                .as_ref()
                .is_some_and(|e| e.code == stt_health::DEGRADED_CODE)
            {
                status.error = None;
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
        let (start_ms, end_ms, utterance_id) = self
            .chunk_times
            .lock()
            .entry(source)
            .or_default()
            .stamp_cloud(finalized);
        let wire = WireTranscript {
            source,
            text,
            start_ms,
            end_ms,
            confidence: None,
            locale: language,
            utterance_id: Some(utterance_id),
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
        self.resume.lock().take();
        self.stop_helper_audio().await;
        self.close_stt().await;
        let status = self.mark_stopped(None);
        self.end_auto_session().await;
        Ok(status)
    }

    /// End the session `start` created for the run that just ended — not a
    /// session the user switched to meanwhile.
    async fn end_auto_session(&self) {
        let Some(id) = self.auto_session.lock().take() else {
            return;
        };
        if self.sessions.active_id().as_deref() != Some(id.as_str()) {
            return;
        }
        if let Err(e) = self.sessions.end().await {
            tracing::debug!(error = %e, "could not end the auto-started session");
        }
    }

    /// Where a session's transcript ends (stored segments, or the ring when
    /// transcripts are not persisted).
    async fn session_last_end_ms(&self, session_id: &str) -> u64 {
        let id = session_id.to_string();
        let stored = self
            .storage
            .run(move |db| TranscriptRepository::last_end_ms(db, &id))
            .await
            .unwrap_or_else(|e| {
                tracing::debug!(error = %e, "could not read the session's transcript end");
                0
            });
        let in_memory = self
            .ring
            .lock()
            .last_end_of_session(session_id)
            .unwrap_or(0);
        stored.max(in_memory)
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
            status.speech_locale = None;
            status.speech_on_device = None;
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

    /// Finals from the last `window_seconds`, oldest first — the context
    /// snapshot's transcript. While listening only the current run counts;
    /// otherwise only the active session's finals (none without a session).
    pub fn recent(&self, window_seconds: u32) -> Vec<TranscriptSegment> {
        let scope = if self.is_running() {
            RingScope::Run(self.run_id.load(Ordering::SeqCst))
        } else {
            match self.sessions.active_id() {
                Some(id) => RingScope::Session(id),
                None => RingScope::Nothing,
            }
        };
        self.ring.lock().recent(&scope, window_seconds)
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
        Ok(self
            .ring
            .lock()
            .list(session_id.as_deref(), since_ms, limit))
    }

    /// Delete stored segments (one session or everything) and clear the ring.
    pub async fn clear(&self, session_id: Option<String>) -> BlueyResult<u64> {
        let target = session_id.clone();
        let removed = self
            .storage
            .run(move |db| {
                let removed = TranscriptRepository::clear(db, target.as_deref())?;
                // Nothing of it may linger in the WAL or a pre-migration backup.
                db.finish_deletion()?;
                Ok(removed)
            })
            .await?;
        {
            let mut ring = self.ring.lock();
            match &session_id {
                Some(id) => ring.forget_session(id),
                None => ring.clear(),
            }
        }
        self.partials.lock().clear();
        self.bus
            .publish(BlueyEvent::TranscriptCleared { session_id });
        Ok(removed)
    }

    /// A session was deleted: its finals leave the in-memory ring too (the
    /// database rows went with the session).
    pub fn forget_session(&self, session_id: &str) {
        {
            // Stopping must not try to end the deleted session (DATA-006).
            let mut auto = self.auto_session.lock();
            if auto.as_deref() == Some(session_id) {
                *auto = None;
            }
        }
        self.ring.lock().forget_session(session_id);
        self.bus.publish(BlueyEvent::TranscriptCleared {
            session_id: Some(session_id.to_string()),
        });
    }

    /// Every session was deleted.
    pub fn forget_all_sessions(&self) {
        *self.auto_session.lock() = None;
        self.ring.lock().forget_all_sessions();
        self.bus
            .publish(BlueyEvent::TranscriptCleared { session_id: None });
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
            .last_end_of_run(self.run_id.load(Ordering::SeqCst))
            .map(|end| end + 800)
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

    /// The helper died. A live run cannot capture any more, so the status
    /// stops claiming it; when a replacement is on its way the run is resumed
    /// with the same config (and session) once it is up.
    async fn on_helper_exited(&self, restarting: bool) {
        let plan = helper_exit_plan(self.status().state, restarting);
        if plan.stop_run {
            let config = self.config.lock().clone();
            self.close_stt().await;
            if plan.resume {
                *self.resume.lock() = config;
            }
            self.mark_stopped(Some(BlueyError::sidecar(
                "helper_exited",
                "the native helper stopped while listening",
            )));
        }
        if plan.end_auto_session {
            self.resume.lock().take();
            self.end_auto_session().await;
        }
    }

    /// The replacement helper is up: resume the run it lost (once).
    fn on_helper_restarted(self: &Arc<Self>) {
        let Some(config) = self.resume.lock().take() else {
            return;
        };
        // The user started listening again meanwhile.
        if claim_start(&self.status).is_err() {
            return;
        }
        tracing::info!("resuming listening after a helper restart");
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = this.start_claimed(config).await {
                tracing::warn!(error = %error, "could not resume listening after a helper restart");
                this.end_auto_session().await;
            }
        });
    }

    async fn handle_helper_event(self: &Arc<Self>, event: HelperEvent) {
        match event {
            HelperEvent::AudioStarted {
                microphone,
                system_audio,
                device,
                speech,
            } => {
                // Also re-sent mid-run when the microphone drops out or comes
                // back (MAC-005); those updates carry no speech route.
                let mut status = self.status.lock();
                // A late event of a start that was already given up on.
                if !matches!(
                    status.state,
                    AudioSessionState::Starting
                        | AudioSessionState::Running
                        | AudioSessionState::Paused
                ) {
                    return;
                }
                status.microphone_active = microphone;
                status.system_audio_active = system_audio;
                if device.is_some() {
                    status.current_input_device = device;
                }
                if let Some(route) = speech {
                    let notice = server_speech_notice(&route);
                    status.speech_locale = Some(route.locale);
                    status.speech_on_device = Some(route.on_device);
                    drop(status);
                    if let Some(notice) = notice {
                        self.bus.publish(BlueyEvent::AudioError(notice));
                    }
                }
            }
            HelperEvent::AudioStopped { reason } => {
                // While `Starting`, a (re)start owns the helper session and
                // its own `audio.stop` is expected.
                if !matches!(
                    self.status().state,
                    AudioSessionState::Running | AudioSessionState::Paused
                ) {
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
                // Capture ended in the helper (device lost, stream error): the
                // listening run is over, and so is the session it started.
                self.end_auto_session().await;
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
            HelperEvent::Exited { restarting } => self.on_helper_exited(restarting).await,
            HelperEvent::Restarted => self.on_helper_restarted(),
            HelperEvent::Ready { .. }
            | HelperEvent::ScreenChanged { .. }
            | HelperEvent::Unknown { .. } => {}
        }
    }

    async fn on_transcript(&self, wire: WireTranscript, finalized: bool) {
        if wire.text.trim().is_empty() {
            return;
        }
        let wire = on_session_timeline(wire, self.time_offset_ms.load(Ordering::SeqCst));
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
        self.ring
            .lock()
            .push(self.run_id.load(Ordering::SeqCst), segment.clone());
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

/// Listening needs a signed-in user wherever sign-in is required.
fn ensure_signed_in(state: AppState) -> BlueyResult<()> {
    if state == AppState::AuthRequired {
        return Err(BlueyError::authentication(
            "sign_in_required",
            "sign in to Bluey before listening",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::types::VadSensitivity;

    #[test]
    fn listening_is_refused_while_signed_out() {
        let error = ensure_signed_in(AppState::AuthRequired).unwrap_err();
        assert_eq!(error.code, "auth.sign_in_required");
        // Signed in, or no sign-in configured (boot goes straight to Ready).
        assert!(ensure_signed_in(AppState::Ready).is_ok());
        assert!(ensure_signed_in(AppState::Listening).is_ok());
    }

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
            route_for(TranscriptionProviderKind::Apple, true, true),
            (TranscriptionRoute::Apple, None)
        );
        assert_eq!(
            route_for(TranscriptionProviderKind::GeminiLive, true, true),
            (TranscriptionRoute::Pcm, None)
        );
        assert_eq!(
            route_for(TranscriptionProviderKind::CloudRealtime, true, true),
            (TranscriptionRoute::Pcm, None)
        );
        let (route, reason) = route_for(TranscriptionProviderKind::GeminiLive, true, false);
        assert_eq!(route, TranscriptionRoute::Apple);
        assert!(reason.unwrap().contains("Google AI Studio key"));
        let (route, reason) = route_for(TranscriptionProviderKind::CloudRealtime, true, false);
        assert_eq!(route, TranscriptionRoute::Apple);
        assert!(reason.is_some());
        assert_eq!(
            route_for(TranscriptionProviderKind::Mock, true, false).0,
            TranscriptionRoute::Apple
        );
    }

    #[test]
    fn cloud_ai_off_keeps_live_audio_on_the_mac() {
        for provider in [
            TranscriptionProviderKind::GeminiLive,
            TranscriptionProviderKind::CloudRealtime,
        ] {
            // Even with a stored key the audio never takes the cloud route.
            assert_eq!(
                route_for(provider, false, true),
                (TranscriptionRoute::Apple, Some(CLOUD_AI_OFF_REASON))
            );
        }
        // Local routes are unaffected by the switch.
        assert_eq!(
            route_for(TranscriptionProviderKind::Apple, false, false),
            (TranscriptionRoute::Apple, None)
        );
        assert_eq!(
            route_for(TranscriptionProviderKind::Mock, false, true),
            (TranscriptionRoute::Pcm, None)
        );
    }

    #[test]
    fn only_one_concurrent_start_claims_the_slot() {
        let status = Arc::new(parking_lot::Mutex::new(AudioStatus::default()));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let claims: Vec<_> = (0..8)
            .map(|_| {
                let status = status.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    claim_start(&status).is_ok()
                })
            })
            .collect();
        let won = claims
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|won| *won)
            .count();
        assert_eq!(won, 1, "exactly one start reaches the helper");
        assert_eq!(status.lock().state, AudioSessionState::Starting);
        // A duplicate toggle while running gets the running status back.
        status.lock().state = AudioSessionState::Running;
        assert_eq!(
            claim_start(&status).unwrap_err().state,
            AudioSessionState::Running
        );
        // A stopped or failed session can be started again.
        for state in [AudioSessionState::Stopped, AudioSessionState::Error] {
            status.lock().state = state;
            assert!(claim_start(&status).is_ok());
        }
    }

    #[test]
    fn a_failed_helper_start_is_reconciled_with_the_helper() {
        let timeout = BlueyError::sidecar("timeout", "helper call `audio.start` timed out");
        assert_eq!(start_recovery(&timeout), StartRecovery::StopHelper);
        let already = jsonl_audio_error("audio_already_running");
        assert_eq!(start_recovery(&already), StartRecovery::StopAndRetry);
        let denied = BlueyError::audio("engine_start_failed", "no");
        assert_eq!(start_recovery(&denied), StartRecovery::Report);
    }

    /// An `audio.*` error as the helper reports it on the wire.
    fn jsonl_audio_error(code: &str) -> BlueyError {
        bluey_protocols::jsonl::WireError {
            code: code.into(),
            message: "m".into(),
            kind: Some("audio".into()),
            details: None,
        }
        .into_bluey()
    }

    /// MAC-007: server recognition behind an on-device promise is surfaced.
    #[test]
    fn apple_speech_on_the_server_is_announced() {
        let route = |on_device| WireSpeechRoute {
            locale: "de-DE".into(),
            on_device,
        };
        assert!(server_speech_notice(&route(true)).is_none());
        let notice = server_speech_notice(&route(false)).expect("announced");
        assert_eq!(notice.code, "audio.speech_server");
        assert!(notice.message.contains("de-DE"));
    }

    /// UX-010: the first interim after a final arrives before the speech
    /// chunk that starts its span; it must still share the final's id.
    #[test]
    fn cloud_interims_and_their_final_share_one_utterance_id() {
        let mut timing = ChunkTiming {
            last_start_ms: 5_000,
            last_end_ms: 5_100,
            ..ChunkTiming::default()
        };
        let (interim_start, _, interim) = timing.stamp_cloud(false);
        timing.utterance_start_ms = Some(5_200);
        let (final_start, _, final_) = timing.stamp_cloud(true);
        assert_ne!(interim_start, final_start, "the span moved…");
        assert_eq!(interim, final_, "…but the utterance did not");

        let (_, _, next) = timing.stamp_cloud(false);
        assert_ne!(next, final_, "a final closes its utterance");
    }

    #[test]
    fn a_second_run_in_a_session_continues_its_timeline() {
        // Run 1 ended at 42 s; run 2's clock restarts at 0.
        let run_two = WireTranscript {
            source: AudioSource::Microphone,
            text: "later".into(),
            start_ms: 1_000,
            end_ms: 2_500,
            confidence: None,
            locale: None,
            utterance_id: None,
        };
        let placed = on_session_timeline(run_two, 42_000);
        assert_eq!((placed.start_ms, placed.end_ms), (43_000, 44_500));
        assert!(placed.start_ms > 42_000, "never interleaves with run 1");
    }

    #[test]
    fn a_helper_exit_never_leaves_the_run_claiming_to_listen() {
        use AudioSessionState::*;
        // A replacement is on its way: the live run stops and resumes on it.
        for state in [Running, Paused] {
            let plan = helper_exit_plan(state, true);
            assert!(plan.stop_run && plan.resume && !plan.end_auto_session);
        }
        // Gone for good (crash loop, restart failed, restarts disabled).
        let plan = helper_exit_plan(Running, false);
        assert!(plan.stop_run && !plan.resume && plan.end_auto_session);
        // Not listening: nothing to stop or resume (a start in flight fails
        // with its pending request).
        for state in [Stopped, Error, Starting] {
            let plan = helper_exit_plan(state, true);
            assert!(!plan.stop_run && !plan.resume);
        }
    }

    #[test]
    fn gemini_live_is_the_default_provider() {
        assert_eq!(
            AudioSessionConfig::default().transcription.provider,
            TranscriptionProviderKind::GeminiLive
        );
    }
}
