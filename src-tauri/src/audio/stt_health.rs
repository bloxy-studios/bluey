//! Health of the live cloud transcription, per source (LIVE-004).
//!
//! A provider that loses its connection keeps reconnecting on its own and
//! reports the outage once ([`TranscriptionEvent::Degraded`]); one that gives
//! up ([`TranscriptionEvent::Failed`]) is re-opened after a cool-down instead
//! of staying dead until the next Start. Either way the user hears about an
//! outage once, and the notice clears when transcripts flow again.
//!
//! [`TranscriptionEvent::Degraded`]: crate::transcription::TranscriptionEvent::Degraded
//! [`TranscriptionEvent::Failed`]: crate::transcription::TranscriptionEvent::Failed

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use bluey_core::types::AudioSource;
use bluey_core::BlueyError;

/// A source whose provider session gave up is re-opened after this long.
pub(super) const REOPEN_COOLDOWN: Duration = Duration::from_secs(10);

/// `AudioStatus.error` / `audio.error` code for an ongoing outage.
pub(super) const DEGRADED_CODE: &str = "audio.stt_degraded";

/// The one notice published per outage.
pub(super) fn degraded_error() -> BlueyError {
    BlueyError::audio(
        "stt_degraded",
        "live transcription lost its connection and is reconnecting",
    )
}

#[derive(Debug, Default)]
pub(super) struct SttHealth {
    /// Sources whose provider session ended, and when.
    failed: HashMap<AudioSource, Instant>,
    /// Sources in an announced outage.
    degraded: HashSet<AudioSource>,
}

impl SttHealth {
    /// The source's connection is down. True when this starts an outage
    /// (announce it); false while one is already announced.
    pub(super) fn degraded(&mut self, source: AudioSource) -> bool {
        self.degraded.insert(source)
    }

    /// The provider session for `source` gave up (a retryable error): it is
    /// re-opened after [`REOPEN_COOLDOWN`]. True when this starts an outage.
    pub(super) fn failed(&mut self, source: AudioSource, now: Instant) -> bool {
        self.failed.insert(source, now);
        self.degraded(source)
    }

    /// Transcripts flow (or the provider reconnected). True when this ended
    /// the last outage, so the notice can be cleared.
    pub(super) fn healthy(&mut self, source: AudioSource) -> bool {
        self.degraded.remove(&source) && self.degraded.is_empty()
    }

    /// Whether audio for `source` may reach the provider now: false while
    /// cooling down after a failure; the first chunk after that re-opens it.
    pub(super) fn may_forward(&mut self, source: AudioSource, now: Instant) -> bool {
        match self.failed.get(&source) {
            Some(at) if now.saturating_duration_since(*at) < REOPEN_COOLDOWN => false,
            Some(_) => {
                self.failed.remove(&source);
                true
            }
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIC: AudioSource = AudioSource::Microphone;
    const SYSTEM: AudioSource = AudioSource::System;

    /// LIVE-004: a failed source is not dead for the rest of the run.
    #[test]
    fn a_failed_source_reopens_after_the_cool_down() {
        let mut health = SttHealth::default();
        let t0 = Instant::now();
        assert!(health.failed(MIC, t0), "the first failure starts an outage");

        assert!(!health.may_forward(MIC, t0 + Duration::from_secs(1)));
        assert!(
            health.may_forward(SYSTEM, t0),
            "other sources are unaffected"
        );
        assert!(health.may_forward(MIC, t0 + REOPEN_COOLDOWN));
        assert!(
            health.may_forward(MIC, t0 + REOPEN_COOLDOWN),
            "re-opened, not cooling down"
        );
    }

    #[test]
    fn an_outage_is_announced_once_and_cleared_by_transcripts() {
        let mut health = SttHealth::default();
        let t0 = Instant::now();
        assert!(health.degraded(MIC));
        assert!(
            !health.failed(MIC, t0),
            "the provider giving up is the same outage"
        );
        assert!(!health.degraded(MIC));

        assert!(health.healthy(MIC), "the last outage ended");
        assert!(!health.healthy(MIC), "nothing left to clear");
        assert!(health.degraded(MIC), "a later outage is announced again");
    }

    #[test]
    fn the_notice_stays_while_another_source_is_down() {
        let mut health = SttHealth::default();
        health.degraded(MIC);
        health.degraded(SYSTEM);
        assert!(!health.healthy(MIC));
        assert!(health.healthy(SYSTEM));
    }
}
