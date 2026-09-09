//! Cloud transcription providers (`TranscriptionProvider` trait).
//!
//! The helper captures 16 kHz mono PCM16 per source (`microphone`, `system`)
//! and, on the cloud route, emits it as `audio.chunk { pcm16 }`. The audio
//! manager opens one provider session **per source** so speaker labels keep
//! coming from the channel, forwards the chunks, and turns the events back into
//! transcript segments through the same assembler the on-device path uses.
//!
//! * [`gemini_live`] — Gemini Live API (`gemini-3.5-transcribe-live`), the
//!   default; rotates sessions before the 10-minute cap and dedupes finals
//!   across the hand-over.
//! * [`cloud_realtime`] — Foundry Voice Live (MAI-Transcribe) over the shared
//!   realtime event codec.
//! * [`mock`] — deterministic finals for developer mode and tests.
//! * [`batch`] — whole recordings through `gemini-3.5-transcribe`
//!   (`ai_transcribe_file`): speaker turns → segments + a session event.
//!
//! Raw audio is never written anywhere: chunks are forwarded and dropped.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use bluey_core::types::{AudioSource, TranscriptionProviderKind};
use bluey_core::{BlueyError, BlueyResult};
use tokio::sync::mpsc;

pub mod batch;
pub mod cloud_realtime;
pub mod gemini_live;
pub mod mock;

/// Default Gemini Live transcription model (Live API only).
pub const GEMINI_LIVE_MODEL: &str = "gemini-3.5-transcribe-live";
/// A final repeating the previous final within this window is a replay from a
/// rotated session and is dropped.
pub const DEDUPE_WINDOW: Duration = Duration::from_secs(2);

/// One captured chunk (base64 PCM16 LE mono).
#[derive(Debug, Clone)]
pub struct PcmChunk {
    pub source: AudioSource,
    pub base64: String,
    pub sample_rate: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub is_speech: bool,
}

/// Per-source session options.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub source: AudioSource,
    pub model: String,
    /// BCP-47 tag or `None` for auto-detect.
    pub language: Option<String>,
    pub vocabulary: Vec<String>,
}

/// What a provider session reports back.
#[derive(Debug, Clone)]
pub enum TranscriptionEvent {
    /// Speculative text replacing the current utterance.
    Interim { source: AudioSource, text: String },
    /// Committed utterance text.
    Final {
        source: AudioSource,
        text: String,
        language: Option<String>,
    },
    /// The provider gave up on this source after its own retries.
    Failed {
        source: AudioSource,
        error: BlueyError,
    },
}

pub type EventSink = mpsc::Sender<TranscriptionEvent>;

/// A streaming speech-to-text backend.
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    fn kind(&self) -> TranscriptionProviderKind;

    /// Open a streaming session for one audio source; events arrive on `sink`.
    async fn open(
        &self,
        options: SessionOptions,
        sink: EventSink,
    ) -> BlueyResult<Box<dyn TranscriptionSession>>;
}

/// One open streaming session.
#[async_trait]
pub trait TranscriptionSession: Send + Sync {
    async fn push_audio(&self, chunk: PcmChunk) -> BlueyResult<()>;

    /// Flush and close; finals still in flight are delivered before the sink
    /// stops receiving.
    async fn close(&self);
}

/// Final-transcript dedupe across session rotation (and provider replays).
#[derive(Debug, Default)]
pub struct FinalDedupe {
    last: Option<(String, Instant)>,
}

impl FinalDedupe {
    /// Whether `text` should be emitted (non-empty and not a repeat of the
    /// previous final within [`DEDUPE_WINDOW`]).
    pub fn accept(&mut self, text: &str, now: Instant) -> bool {
        let normalized = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if normalized.is_empty() {
            return false;
        }
        if let Some((previous, at)) = &self.last {
            if *previous == normalized && now.duration_since(*at) <= DEDUPE_WINDOW {
                return false;
            }
        }
        self.last = Some((normalized, now));
        true
    }
}

/// The Gemini model to open for live transcription: the assigned
/// transcription-role model when it is a *transcribe* Live model, else the
/// default. Conversational Live models (`gemini-3.1-flash-live-preview`, …)
/// are not transcription models and are never opened here.
pub fn gemini_live_model(assigned: Option<&str>) -> String {
    match assigned.map(str::trim) {
        Some(model) if !model.is_empty() => {
            let lower = model.to_ascii_lowercase();
            if lower.contains("transcribe") && lower.contains("live") {
                model.to_string()
            } else {
                GEMINI_LIVE_MODEL.to_string()
            }
        }
        _ => GEMINI_LIVE_MODEL.to_string(),
    }
}

/// Error for a session that was closed underneath the caller.
pub(crate) fn session_closed() -> BlueyError {
    BlueyError::transcription("session_closed", "the transcription session is closed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_drops_only_immediate_repeats() {
        let mut dedupe = FinalDedupe::default();
        let t0 = Instant::now();
        assert!(dedupe.accept("Hello there", t0));
        assert!(!dedupe.accept("hello  there", t0 + Duration::from_millis(500)));
        assert!(dedupe.accept("Hello there", t0 + Duration::from_secs(3)));
        assert!(dedupe.accept("Something else", t0 + Duration::from_secs(3)));
        assert!(!dedupe.accept("   ", t0));
    }

    #[test]
    fn live_model_prefers_a_live_assignment() {
        assert_eq!(gemini_live_model(None), GEMINI_LIVE_MODEL);
        assert_eq!(
            gemini_live_model(Some("gemini-3.5-transcribe")),
            GEMINI_LIVE_MODEL
        );
        assert_eq!(
            gemini_live_model(Some("gemini-4-transcribe-live")),
            "gemini-4-transcribe-live"
        );
    }
}
