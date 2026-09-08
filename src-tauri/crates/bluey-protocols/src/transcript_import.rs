//! Imported recordings: batch-transcription speaker turns → finalized
//! [`TranscriptSegment`]s (pure; the app's `transcription::batch` does the I/O).
//!
//! Segments are cut where the speaker changes (one turn per speaker), where the
//! audio pauses for more than [`SEGMENT_GAP_MS`], and every
//! [`MAX_SEGMENT_WORDS`] words, so retrieval and the transcript strip see
//! utterance-sized pieces. Turns without word timings are split at sentence
//! ends and given estimated timings ([`ESTIMATED_MS_PER_WORD`]) so ordering
//! stays intact.

use std::collections::BTreeSet;

use bluey_core::types::{AudioSource, TranscriptSegment};

use crate::gemini::{TranscribedWord, TranscriptTurn};

/// A pause longer than this starts a new segment.
pub const SEGMENT_GAP_MS: u64 = 1_500;
/// Longest segment, in words.
pub const MAX_SEGMENT_WORDS: usize = 40;
/// Timing estimate for turns without word offsets.
pub const ESTIMATED_MS_PER_WORD: u64 = 350;

/// Turn speaker turns into finalized segments (see the module docs for the
/// cutting rules). `next_id` mints segment ids.
pub fn segments_from_turns(
    turns: &[TranscriptTurn],
    session_id: Option<&str>,
    language: Option<&str>,
    created_at: &str,
    mut next_id: impl FnMut() -> String,
) -> Vec<TranscriptSegment> {
    let mut segments = Vec::new();
    let mut clock: u64 = 0;
    for turn in turns {
        let speaker = turn.speaker.as_deref();
        let timed = turn
            .words
            .iter()
            .any(|w| w.start_ms.is_some() || w.end_ms.is_some());
        if timed {
            let mut chunk: Vec<&TranscribedWord> = Vec::new();
            let mut chunk_start = clock;
            let mut last_end = clock;
            for word in &turn.words {
                let start = word.start_ms.unwrap_or(last_end);
                let end = word.end_ms.unwrap_or(start).max(start);
                let gap = start.saturating_sub(last_end);
                if !chunk.is_empty() && (gap > SEGMENT_GAP_MS || chunk.len() >= MAX_SEGMENT_WORDS) {
                    segments.push(segment(
                        next_id(),
                        session_id,
                        speaker,
                        language,
                        created_at,
                        &join_words(&chunk),
                        chunk_start,
                        last_end,
                    ));
                    chunk.clear();
                }
                if chunk.is_empty() {
                    chunk_start = start;
                }
                chunk.push(word);
                last_end = end;
            }
            if !chunk.is_empty() {
                segments.push(segment(
                    next_id(),
                    session_id,
                    speaker,
                    language,
                    created_at,
                    &join_words(&chunk),
                    chunk_start,
                    last_end,
                ));
            }
            clock = last_end.max(clock);
        } else {
            for piece in split_sentences(&turn.text, MAX_SEGMENT_WORDS) {
                let words = piece.split_whitespace().count().max(1) as u64;
                let start = clock;
                let end = start + words * ESTIMATED_MS_PER_WORD;
                segments.push(segment(
                    next_id(),
                    session_id,
                    speaker,
                    language,
                    created_at,
                    &piece,
                    start,
                    end,
                ));
                clock = end;
            }
        }
    }
    segments
}

fn join_words(words: &[&TranscribedWord]) -> String {
    words
        .iter()
        .map(|w| w.word.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[allow(clippy::too_many_arguments)]
fn segment(
    id: String,
    session_id: Option<&str>,
    speaker: Option<&str>,
    language: Option<&str>,
    created_at: &str,
    text: &str,
    start_time: u64,
    end_time: u64,
) -> TranscriptSegment {
    TranscriptSegment {
        id,
        session_id: session_id.map(str::to_string),
        speaker: speaker.map(str::to_string),
        speaker_confidence: None,
        source: AudioSource::System,
        text: text.to_string(),
        start_time,
        end_time: end_time.max(start_time),
        confidence: None,
        finalized: true,
        language: language.map(str::to_string),
        created_at: created_at.to_string(),
    }
}

/// Split `text` at sentence ends (`.`, `?`, `!` followed by whitespace) and
/// regroup the sentences into pieces of at most `max_words` words; a single
/// longer sentence is cut every `max_words` words.
pub fn split_sentences(text: &str, max_words: usize) -> Vec<String> {
    let max_words = max_words.max(1);
    let chars: Vec<char> = text.chars().collect();
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    for (i, ch) in chars.iter().enumerate() {
        current.push(*ch);
        let ends_sentence = matches!(ch, '.' | '?' | '!')
            && chars.get(i + 1).map(|c| c.is_whitespace()).unwrap_or(true);
        if ends_sentence {
            let sentence = current.trim();
            if !sentence.is_empty() {
                sentences.push(sentence.to_string());
            }
            current.clear();
        }
    }
    let tail = current.trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }

    let mut pieces: Vec<String> = Vec::new();
    let mut piece: Vec<String> = Vec::new();
    let mut count = 0usize;
    for sentence in sentences {
        let words: Vec<&str> = sentence.split_whitespace().collect();
        if words.len() > max_words {
            if !piece.is_empty() {
                pieces.push(piece.join(" "));
                piece.clear();
                count = 0;
            }
            for chunk in words.chunks(max_words) {
                pieces.push(chunk.join(" "));
            }
            continue;
        }
        if count + words.len() > max_words && !piece.is_empty() {
            pieces.push(piece.join(" "));
            piece.clear();
            count = 0;
        }
        count += words.len();
        piece.push(sentence);
    }
    if !piece.is_empty() {
        pieces.push(piece.join(" "));
    }
    pieces
}

/// Distinct speaker labels among `segments`.
pub fn speaker_count(segments: &[TranscriptSegment]) -> u32 {
    segments
        .iter()
        .filter_map(|s| s.speaker.as_deref())
        .collect::<BTreeSet<_>>()
        .len() as u32
}

/// `4 min 10 s` / `35 s`.
pub fn format_duration(ms: u64) -> String {
    let seconds = ms.div_ceil(1000);
    if seconds >= 60 {
        format!("{} min {} s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds} s")
    }
}

/// Timeline detail for the `recording_imported` session event.
pub fn import_detail(
    file_name: &str,
    segments: usize,
    speakers: u32,
    duration_ms: u64,
    stored: bool,
) -> String {
    let mut detail = format!(
        "{file_name} · {segments} segment{} · {}",
        if segments == 1 { "" } else { "s" },
        format_duration(duration_ms)
    );
    if speakers > 0 {
        detail.push_str(&format!(
            " · {speakers} speaker{}",
            if speakers == 1 { "" } else { "s" }
        ));
    }
    if !stored {
        detail.push_str(" · not stored (transcript storage is off)");
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn timed(speaker: &str, words: &[(&str, u64, u64)]) -> TranscriptTurn {
        TranscriptTurn {
            speaker: Some(speaker.into()),
            text: words.iter().map(|w| w.0).collect::<Vec<_>>().join(" "),
            words: words
                .iter()
                .map(|(word, start, end)| TranscribedWord {
                    word: word.to_string(),
                    start_ms: Some(*start),
                    end_ms: Some(*end),
                })
                .collect(),
        }
    }

    fn ids() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("seg-{n}")
        }
    }

    #[test]
    fn timed_turns_split_on_speaker_and_long_pauses() {
        let turns = vec![
            timed(
                "spk_1",
                &[
                    ("Hello", 100, 400),
                    ("there", 450, 800),
                    ("again", 3_000, 3_400),
                ],
            ),
            timed("spk_2", &[("Hi", 3_600, 3_900)]),
        ];
        let segments = segments_from_turns(&turns, Some("ses-1"), Some("en"), "now", ids());
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "Hello there");
        assert_eq!((segments[0].start_time, segments[0].end_time), (100, 800));
        assert_eq!(segments[0].speaker.as_deref(), Some("spk_1"));
        assert_eq!(segments[0].session_id.as_deref(), Some("ses-1"));
        assert_eq!(segments[0].language.as_deref(), Some("en"));
        assert!(segments[0].finalized);
        assert_eq!(segments[0].source, AudioSource::System);
        assert_eq!(segments[1].text, "again");
        assert_eq!(
            (segments[1].start_time, segments[1].end_time),
            (3_000, 3_400)
        );
        assert_eq!(segments[2].speaker.as_deref(), Some("spk_2"));
        assert_eq!(
            (segments[2].start_time, segments[2].end_time),
            (3_600, 3_900)
        );
        assert_eq!(segments[2].id, "seg-3");
    }

    #[test]
    fn segments_never_exceed_the_word_cap() {
        let words: Vec<(String, u64, u64)> = (0..85u64)
            .map(|i| (format!("w{i}"), i * 300, i * 300 + 250))
            .collect();
        let refs: Vec<(&str, u64, u64)> =
            words.iter().map(|(w, s, e)| (w.as_str(), *s, *e)).collect();
        let segments = segments_from_turns(&[timed("spk_1", &refs)], None, None, "now", ids());
        let counts: Vec<usize> = segments
            .iter()
            .map(|s| s.text.split_whitespace().count())
            .collect();
        assert_eq!(counts, vec![40, 40, 5]);
        assert_eq!(segments[1].start_time, 40 * 300);
        assert!(segments[0].session_id.is_none());
    }

    #[test]
    fn untimed_turns_get_sentences_and_estimated_monotonic_timings() {
        let turns = vec![
            TranscriptTurn {
                speaker: None,
                text: "Thanks for joining. Let's get started! Ready?".into(),
                words: Vec::new(),
            },
            timed("spk_1", &[("Yes", 9_000, 9_300)]),
        ];
        let segments = segments_from_turns(&turns, None, None, "now", ids());
        assert_eq!(segments.len(), 2);
        assert_eq!(
            segments[0].text,
            "Thanks for joining. Let's get started! Ready?"
        );
        assert_eq!(segments[0].start_time, 0);
        assert_eq!(segments[0].end_time, 7 * ESTIMATED_MS_PER_WORD);
        assert!(segments[0].speaker.is_none());
        assert_eq!(
            segments[1].start_time, 9_000,
            "real timings win once available"
        );
    }

    #[test]
    fn words_missing_offsets_inherit_the_running_clock() {
        let turn = TranscriptTurn {
            speaker: Some("spk_1".into()),
            text: "one two".into(),
            words: vec![
                TranscribedWord {
                    word: "one".into(),
                    start_ms: Some(500),
                    end_ms: None,
                },
                TranscribedWord {
                    word: "two".into(),
                    start_ms: None,
                    end_ms: Some(900),
                },
            ],
        };
        let segments = segments_from_turns(&[turn], None, None, "now", ids());
        assert_eq!(segments.len(), 1);
        assert_eq!((segments[0].start_time, segments[0].end_time), (500, 900));
    }

    #[test]
    fn sentence_splitting_groups_up_to_the_cap_and_cuts_run_ons() {
        let pieces = split_sentences("One two three. Four five. Six seven eight nine.", 5);
        assert_eq!(
            pieces,
            vec!["One two three. Four five.", "Six seven eight nine."]
        );
        let separate = split_sentences("One two three. Four five six.", 5);
        assert_eq!(separate, vec!["One two three.", "Four five six."]);
        let grouped = split_sentences("One two. Three four. Five six.", 4);
        assert_eq!(grouped, vec!["One two. Three four.", "Five six."]);
        let run_on = split_sentences("a b c d e f g", 3);
        assert_eq!(run_on, vec!["a b c", "d e f", "g"]);
        assert_eq!(
            split_sentences("Version 3.5 shipped.", 10),
            vec!["Version 3.5 shipped."]
        );
        assert!(split_sentences("   ", 10).is_empty());
    }

    #[test]
    fn speaker_count_and_detail_copy() {
        let turns = vec![
            timed("spk_1", &[("a", 0, 100)]),
            timed("spk_2", &[("b", 200, 300)]),
            timed("spk_1", &[("c", 400, 500)]),
        ];
        let segments = segments_from_turns(&turns, None, None, "now", ids());
        assert_eq!(speaker_count(&segments), 2);
        assert_eq!(
            import_detail("standup.wav", 12, 2, 250_000, true),
            "standup.wav · 12 segments · 4 min 10 s · 2 speakers"
        );
        assert_eq!(
            import_detail("memo.mp3", 1, 0, 35_000, false),
            "memo.mp3 · 1 segment · 35 s · not stored (transcript storage is off)"
        );
        assert_eq!(format_duration(0), "0 s");
        assert_eq!(format_duration(60_000), "1 min 0 s");
    }
}
