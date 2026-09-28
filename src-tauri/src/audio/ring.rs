//! In-memory ring of recent finals: the context snapshot's transcript and the
//! transcript view's source while transcripts are not persisted.
//!
//! Segment times restart at 0 on every `audio.start`, so the ring cannot be
//! read by time alone. Every entry carries the listening run that heard it
//! (bumped by each start) and the session it was filed under; a read is always
//! scoped to one run or one session, and its recency window is measured from
//! that scope's own newest segment. A later ask in another session therefore
//! never sees an earlier conversation.

use std::collections::VecDeque;

use bluey_core::types::TranscriptSegment;

/// Which finals a context read may see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RingScope {
    /// Listening: the finals of the current run.
    Run(u64),
    /// Not listening: the finals filed under the active session.
    Session(String),
    /// Neither: nothing is recent.
    Nothing,
}

#[derive(Debug, Clone)]
struct Entry {
    run_id: u64,
    segment: TranscriptSegment,
}

#[derive(Debug)]
pub struct TranscriptRing {
    entries: VecDeque<Entry>,
    capacity: usize,
}

impl TranscriptRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Append a final heard during `run_id` (the oldest entry makes room).
    pub fn push(&mut self, run_id: u64, segment: TranscriptSegment) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry { run_id, segment });
    }

    /// Finals of `scope` from the last `window_seconds` of that scope's own
    /// timeline, oldest first.
    pub fn recent(&self, scope: &RingScope, window_seconds: u32) -> Vec<TranscriptSegment> {
        let in_scope = |entry: &&Entry| match scope {
            RingScope::Run(run_id) => entry.run_id == *run_id,
            RingScope::Session(id) => entry.segment.session_id.as_deref() == Some(id.as_str()),
            RingScope::Nothing => false,
        };
        let Some(last_end) = self
            .entries
            .iter()
            .filter(in_scope)
            .map(|e| e.segment.end_time)
            .max()
        else {
            return Vec::new();
        };
        let cutoff = last_end.saturating_sub(u64::from(window_seconds) * 1_000);
        self.entries
            .iter()
            .filter(in_scope)
            .filter(|e| e.segment.end_time >= cutoff)
            .map(|e| e.segment.clone())
            .collect()
    }

    /// Finals of one session (or all of them), optionally from `since_ms`,
    /// keeping the newest `limit`.
    pub fn list(
        &self,
        session_id: Option<&str>,
        since_ms: Option<u64>,
        limit: Option<u32>,
    ) -> Vec<TranscriptSegment> {
        let mut segments: Vec<TranscriptSegment> = self
            .entries
            .iter()
            .map(|e| &e.segment)
            .filter(|s| session_id.is_none_or(|id| s.session_id.as_deref() == Some(id)))
            .filter(|s| since_ms.is_none_or(|since| s.start_time >= since))
            .cloned()
            .collect();
        if let Some(limit) = limit {
            let keep = limit as usize;
            if segments.len() > keep {
                segments.drain(..segments.len() - keep);
            }
        }
        segments
    }

    /// End of the newest final of `run_id` (developer simulation continues
    /// after it).
    pub fn last_end_of_run(&self, run_id: u64) -> Option<u64> {
        self.entries
            .iter()
            .filter(|e| e.run_id == run_id)
            .map(|e| e.segment.end_time)
            .max()
    }

    /// End of the newest final filed under `session_id`.
    pub fn last_end_of_session(&self, session_id: &str) -> Option<u64> {
        self.entries
            .iter()
            .filter(|e| e.segment.session_id.as_deref() == Some(session_id))
            .map(|e| e.segment.end_time)
            .max()
    }

    /// Drop the finals of one session (deleted or cleared).
    pub fn forget_session(&mut self, session_id: &str) {
        self.entries
            .retain(|e| e.segment.session_id.as_deref() != Some(session_id));
    }

    /// Drop every final filed under a session (all sessions deleted).
    pub fn forget_all_sessions(&mut self) {
        self.entries.retain(|e| e.segment.session_id.is_none());
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::types::AudioSource;

    fn seg(session: Option<&str>, text: &str, start: u64, end: u64) -> TranscriptSegment {
        TranscriptSegment {
            id: format!("seg_{text}"),
            session_id: session.map(String::from),
            speaker: None,
            speaker_confidence: None,
            source: AudioSource::System,
            text: text.into(),
            start_time: start,
            end_time: end,
            confidence: None,
            finalized: true,
            language: None,
            created_at: "t".into(),
        }
    }

    fn texts(segments: &[TranscriptSegment]) -> Vec<&str> {
        segments.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn a_new_run_never_sees_the_previous_runs_conversation() {
        let mut ring = TranscriptRing::new(500);
        // Run 1: a 20-minute conversation in session A (large end times).
        ring.push(1, seg(Some("A"), "enterprise tier", 1_190_000, 1_195_000));
        ring.push(1, seg(Some("A"), "500 seats", 1_196_000, 1_200_000));
        // Run 2 restarts the clock in session B.
        ring.push(2, seg(Some("B"), "why leave", 4_000, 9_000));

        let recent = ring.recent(&RingScope::Run(2), 180);
        assert_eq!(texts(&recent), vec!["why leave"]);
        // Run 1's window is measured on run 1's own timeline.
        assert_eq!(
            texts(&ring.recent(&RingScope::Run(1), 180)),
            vec!["enterprise tier", "500 seats"]
        );
    }

    #[test]
    fn the_window_is_measured_from_the_scopes_own_newest_segment() {
        let mut ring = TranscriptRing::new(500);
        ring.push(3, seg(Some("A"), "old", 0, 10_000));
        ring.push(3, seg(Some("A"), "new", 200_000, 205_000));
        // A later run with smaller times must not pull the cutoff down.
        ring.push(4, seg(Some("B"), "other", 0, 1_000));
        assert_eq!(texts(&ring.recent(&RingScope::Run(3), 180)), vec!["new"]);
    }

    #[test]
    fn after_listening_stops_only_the_active_session_is_visible() {
        let mut ring = TranscriptRing::new(500);
        ring.push(1, seg(Some("A"), "from a", 0, 1_000));
        ring.push(2, seg(Some("B"), "from b", 0, 1_000));
        assert_eq!(
            texts(&ring.recent(&RingScope::Session("B".into()), 180)),
            vec!["from b"]
        );
        assert!(ring.recent(&RingScope::Nothing, 180).is_empty());
        assert!(ring
            .recent(&RingScope::Session("gone".into()), 180)
            .is_empty());
    }

    #[test]
    fn deleting_a_session_purges_its_finals() {
        let mut ring = TranscriptRing::new(500);
        ring.push(1, seg(Some("A"), "secret", 0, 1_000));
        ring.push(1, seg(Some("B"), "kept", 1_000, 2_000));
        ring.push(1, seg(None, "loose", 2_000, 3_000));
        ring.forget_session("A");
        assert_eq!(texts(&ring.list(None, None, None)), vec!["kept", "loose"]);
        assert!(ring.recent(&RingScope::Session("A".into()), 180).is_empty());
        ring.forget_all_sessions();
        assert_eq!(texts(&ring.list(None, None, None)), vec!["loose"]);
    }

    #[test]
    fn list_filters_by_session_and_since_and_keeps_the_newest() {
        let mut ring = TranscriptRing::new(3);
        for (i, text) in ["a", "b", "c", "d"].iter().enumerate() {
            let start = i as u64 * 1_000;
            ring.push(1, seg(Some("A"), text, start, start + 500));
        }
        assert_eq!(texts(&ring.list(None, None, None)), vec!["b", "c", "d"]);
        assert_eq!(
            texts(&ring.list(Some("A"), Some(2_000), None)),
            vec!["c", "d"]
        );
        assert_eq!(texts(&ring.list(Some("A"), None, Some(1))), vec!["d"]);
        assert!(ring.list(Some("B"), None, None).is_empty());
        assert_eq!(ring.last_end_of_run(1), Some(3_500));
        assert_eq!(ring.last_end_of_session("A"), Some(3_500));
        assert_eq!(ring.last_end_of_run(2), None);
    }
}
