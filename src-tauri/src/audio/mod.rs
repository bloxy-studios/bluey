//! Audio manager: drives the helper's `audio.*` methods, consumes its
//! `audio.*` / `transcript.*` events, assembles transcript segments (stable ids
//! across partials, speaker labels from the channel), keeps a ring buffer of
//! recent finals for the context snapshot, persists finals when the privacy
//! settings allow and mirrors everything onto the bus.
//!
//! This first version serves the on-device Apple Speech path. The cloud
//! transcription providers (`gemini_live`, `cloud_realtime`) plug into
//! [`TranscriptionRoute`] in a later change; until then they fall back to Apple
//! with a non-fatal `audio.error{code: stt_fallback}`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{
    AppEvent, AudioDevice, AudioLevels, AudioSessionConfig, AudioSessionConfigPatch,
    AudioSessionState, AudioSource, AudioSourcePreference, AudioStatus, TranscriptSegment,
    TranscriptionProviderKind,
};
use bluey_core::{new_id, now_iso, BlueyError, BlueyResult};
use bluey_protocols::helper::{DevicesResult, HelperEvent, TranscriptAssembler, WireTranscript};
use bluey_storage::TranscriptRepository;
use serde::Serialize;
use serde_json::{json, Value};

use crate::events::EventBus;
use crate::modes::ModeManager;
use crate::sessions::SessionManager;
use crate::settings::SettingsManager;
use crate::sidecar::HelperClient;
use crate::state::StateHub;
use crate::storage::Storage;

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
    /// The helper emits PCM; a Rust provider transcribes it (not wired yet).
    Pcm,
}

/// Decide how a configured provider is served today. Cloud providers are not
/// wired yet → Apple with a fallback notice.
pub fn route_for(provider: TranscriptionProviderKind) -> (TranscriptionRoute, bool) {
    match provider {
        TranscriptionProviderKind::Apple => (TranscriptionRoute::Apple, false),
        TranscriptionProviderKind::Mock => (TranscriptionRoute::Apple, false),
        TranscriptionProviderKind::CloudRealtime => (TranscriptionRoute::Apple, true),
    }
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
    ) -> Self {
        Self {
            helper,
            bus,
            hub,
            settings,
            storage,
            sessions,
            modes,
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
    pub async fn start(&self, patch: Option<AudioSessionConfigPatch>) -> BlueyResult<AudioStatus> {
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
        let (route, fallback) = route_for(config.transcription.provider);
        {
            let mut status = self.status.lock();
            status.state = AudioSessionState::Starting;
            status.error = None;
        }
        let params = helper_start_params(&config, route);
        let value = match self.helper.call("audio.start", params).await {
            Ok(value) => value,
            Err(error) => {
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
        self.assembler.lock().reset();
        self.partials.lock().clear();
        *self.config.lock() = Some(config.clone());
        let status = {
            let mut status = self.status.lock();
            status.state = AudioSessionState::Running;
            status.microphone_active = microphone;
            status.system_audio_active = system_audio;
            status.provider = Some(if fallback {
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
        if fallback {
            self.bus.publish(BlueyEvent::AudioError(BlueyError::audio(
                "stt_fallback",
                "cloud transcription is not available yet; using on-device Apple Speech",
            )));
        }
        Ok(status)
    }

    /// Stop listening (and end the auto-started session).
    pub async fn stop(&self) -> BlueyResult<AudioStatus> {
        if self.helper.is_running() {
            if let Err(e) = self.helper.request("audio.stop", json!({})).await {
                tracing::debug!(error = %e, "audio.stop failed (helper may be gone)");
            }
        }
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
                // PCM (when requested) is consumed by the cloud transcription
                // providers; the contract event never carries audio bytes.
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
    fn cloud_providers_fall_back_to_apple_for_now() {
        assert_eq!(
            route_for(TranscriptionProviderKind::Apple),
            (TranscriptionRoute::Apple, false)
        );
        assert_eq!(
            route_for(TranscriptionProviderKind::CloudRealtime),
            (TranscriptionRoute::Apple, true)
        );
    }
}
