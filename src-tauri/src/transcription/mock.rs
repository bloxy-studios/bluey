//! Deterministic transcription for developer mode and tests: every fifth
//! speech chunk of a source becomes a final segment.

use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use bluey_core::types::TranscriptionProviderKind;
use bluey_core::BlueyResult;

use super::{
    EventSink, PcmChunk, SessionOptions, TranscriptionEvent, TranscriptionProvider,
    TranscriptionSession,
};

const CHUNKS_PER_FINAL: u32 = 5;
const LINES: &[&str] = &[
    "Thanks for joining, let's get started.",
    "Could you walk me through your recent project?",
    "What trade-offs did you consider?",
    "How would you scale that design?",
];

#[derive(Default)]
pub struct MockTranscriptionProvider;

#[async_trait]
impl TranscriptionProvider for MockTranscriptionProvider {
    fn kind(&self) -> TranscriptionProviderKind {
        TranscriptionProviderKind::Mock
    }

    async fn open(
        &self,
        options: SessionOptions,
        sink: EventSink,
    ) -> BlueyResult<Box<dyn TranscriptionSession>> {
        Ok(Box::new(MockSession {
            options,
            sink,
            speech_chunks: AtomicU32::new(0),
            finals: AtomicU32::new(0),
        }))
    }
}

struct MockSession {
    options: SessionOptions,
    sink: EventSink,
    speech_chunks: AtomicU32,
    finals: AtomicU32,
}

#[async_trait]
impl TranscriptionSession for MockSession {
    async fn push_audio(&self, chunk: PcmChunk) -> BlueyResult<()> {
        if !chunk.is_speech {
            return Ok(());
        }
        let count = self.speech_chunks.fetch_add(1, Ordering::SeqCst) + 1;
        let index = self.finals.load(Ordering::SeqCst) as usize % LINES.len();
        let text = LINES[index];
        if count % CHUNKS_PER_FINAL == 0 {
            self.finals.fetch_add(1, Ordering::SeqCst);
            let _ = self
                .sink
                .send(TranscriptionEvent::Final {
                    source: self.options.source,
                    text: text.to_string(),
                    language: Some("en".into()),
                })
                .await;
        } else {
            let words: Vec<&str> = text.split_whitespace().collect();
            let shown =
                words.len() * (count % CHUNKS_PER_FINAL) as usize / CHUNKS_PER_FINAL as usize;
            let _ = self
                .sink
                .send(TranscriptionEvent::Interim {
                    source: self.options.source,
                    text: words[..shown.max(1).min(words.len())].join(" "),
                })
                .await;
        }
        Ok(())
    }

    async fn close(&self) {}
}
