//! Deterministic text chunking for retrieval: split on headings/paragraphs
//! first, then sentences, with a configurable token target and overlap.
//! Token counts are estimated locally as ≈ `chars / 4` (no tokenizer dependency).

use unicode_segmentation::UnicodeSegmentation;

/// A chunk produced by [`chunk_text`] (ids are assigned at insert time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkDraft {
    /// Chunk text (paragraphs joined with blank lines; overlap prepended).
    pub content: String,
    /// Estimated token count of `content`.
    pub tokens: u32,
    /// The heading in effect where this chunk starts, if any.
    pub heading: Option<String>,
}

/// Estimate tokens as `ceil(chars / 4)` — good enough for budgeting.
pub fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    chars.div_ceil(4) as u32
}

/// Split `text` into chunks of roughly `target_tokens` tokens with
/// `overlap_tokens` of trailing-sentence overlap between consecutive chunks.
///
/// Structure-aware and deterministic:
/// 1. paragraphs (blank-line separated) are the primary unit;
/// 2. heading lines (Markdown `#`, ALL-CAPS lines, short `…:` label lines,
///    short Title Case lines) update the `heading` carried by later chunks;
/// 3. oversized paragraphs are split on sentence boundaries (and words as a
///    last resort).
pub fn chunk_text(text: &str, target_tokens: u32, overlap_tokens: u32) -> Vec<ChunkDraft> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let target = target_tokens.max(16);
    let paragraphs = split_paragraphs(text);

    let mut chunks: Vec<ChunkDraft> = Vec::new();
    let mut buf = String::new();
    let mut buf_heading: Option<String> = None;
    let mut current_heading: Option<String> = None;

    let flush = |buf: &mut String, heading: &mut Option<String>, chunks: &mut Vec<ChunkDraft>| {
        let content = buf.trim().to_string();
        if !content.is_empty() {
            chunks.push(ChunkDraft {
                tokens: estimate_tokens(&content),
                content,
                heading: heading.clone(),
            });
        }
        buf.clear();
    };

    for paragraph in paragraphs {
        if let Some(heading) = detect_heading(&paragraph) {
            // Headings start a new section: flush the previous chunk (no
            // overlap across section boundaries) and begin with the heading line.
            if !buf.is_empty() {
                flush(&mut buf, &mut buf_heading, &mut chunks);
            }
            current_heading = Some(heading);
            buf_heading = current_heading.clone();
            buf.push_str(&paragraph);
            continue;
        }
        let pieces = if estimate_tokens(&paragraph) > target {
            split_oversized(&paragraph, target)
        } else {
            vec![paragraph]
        };
        for piece in pieces {
            let piece_tokens = estimate_tokens(&piece);
            if !buf.is_empty() && estimate_tokens(&buf) + piece_tokens > target {
                let overlap = tail_sentences(&buf, overlap_tokens);
                flush(&mut buf, &mut buf_heading, &mut chunks);
                buf_heading = current_heading.clone();
                if !overlap.is_empty() {
                    buf.push_str(&overlap);
                }
            }
            if buf.is_empty() {
                buf_heading = current_heading.clone();
            } else {
                buf.push_str("\n\n");
            }
            buf.push_str(&piece);
        }
    }
    flush(&mut buf, &mut buf_heading, &mut chunks);
    chunks
}

/// Blank-line separated paragraphs; a heading on the first line of a paragraph
/// is split into its own paragraph so it can start a fresh section.
fn split_paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split("\n\n") {
        let para = raw.trim();
        if para.is_empty() {
            continue;
        }
        if let Some((first, rest)) = para.split_once('\n') {
            if detect_heading(first).is_some() {
                out.push(first.trim().to_string());
                let rest = rest.trim();
                if !rest.is_empty() {
                    out.push(rest.to_string());
                }
                continue;
            }
        }
        out.push(para.to_string());
    }
    out
}

/// Heading detection on a single line. Returns the cleaned heading text.
fn detect_heading(candidate: &str) -> Option<String> {
    if candidate.contains('\n') {
        return None;
    }
    let line = candidate.trim();
    if line.is_empty() || line.chars().count() > 60 {
        return None;
    }
    // Markdown heading: # … ######
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = line[hashes..].trim();
        if !rest.is_empty() {
            return Some(rest.trim_end_matches(':').trim().to_string());
        }
    }
    let has_letters = line.chars().any(|c| c.is_alphabetic());
    if !has_letters {
        return None;
    }
    // ALL-CAPS line (e.g. "EXPERIENCE", "TECHNICAL SKILLS").
    if !line.chars().any(|c| c.is_lowercase()) {
        return Some(line.trim_end_matches(':').trim().to_string());
    }
    // Short label line ending with a colon (e.g. "Skills:").
    if line.ends_with(':') && !line.contains(". ") {
        return Some(line.trim_end_matches(':').trim().to_string());
    }
    // Short Title Case line without terminal punctuation, at most 5 words.
    let words: Vec<&str> = line.split_whitespace().collect();
    let ends_with_punct = line.ends_with(['.', '!', '?', ',', ';']);
    if !ends_with_punct
        && line.chars().count() <= 40
        && (1..=5).contains(&words.len())
        && words
            .iter()
            .all(|w| w.chars().next().is_some_and(|c| c.is_uppercase()))
    {
        return Some(line.to_string());
    }
    None
}

/// Split an oversized paragraph on sentence boundaries; sentences beyond the
/// target are hard-split on word boundaries.
fn split_oversized(paragraph: &str, target: u32) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut buf = String::new();
    for sentence in paragraph.unicode_sentences() {
        let sentence_tokens = estimate_tokens(sentence);
        if sentence_tokens > target {
            if !buf.trim().is_empty() {
                pieces.push(buf.trim().to_string());
                buf.clear();
            }
            pieces.extend(split_words(sentence, target));
            continue;
        }
        if !buf.is_empty() && estimate_tokens(&buf) + sentence_tokens > target {
            pieces.push(buf.trim().to_string());
            buf.clear();
        }
        buf.push_str(sentence);
    }
    if !buf.trim().is_empty() {
        pieces.push(buf.trim().to_string());
    }
    pieces
}

/// Last-resort split of a single huge sentence on word boundaries.
fn split_words(sentence: &str, target: u32) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut buf = String::new();
    for word in sentence.split_word_bounds() {
        if !buf.is_empty() && estimate_tokens(&buf) + estimate_tokens(word) > target {
            pieces.push(buf.trim().to_string());
            buf.clear();
        }
        buf.push_str(word);
    }
    if !buf.trim().is_empty() {
        pieces.push(buf.trim().to_string());
    }
    pieces.retain(|p| !p.is_empty());
    pieces
}

/// Trailing sentences of `content` adding up to roughly `overlap_tokens`,
/// used as the head of the next chunk. Empty when `overlap_tokens == 0`.
fn tail_sentences(content: &str, overlap_tokens: u32) -> String {
    if overlap_tokens == 0 {
        return String::new();
    }
    let sentences: Vec<&str> = content.unicode_sentences().collect();
    let mut taken: Vec<&str> = Vec::new();
    let mut tokens = 0u32;
    for sentence in sentences.iter().rev() {
        let t = estimate_tokens(sentence);
        if !taken.is_empty() && tokens + t > overlap_tokens {
            break;
        }
        taken.push(sentence);
        tokens += t;
        if tokens >= overlap_tokens {
            break;
        }
    }
    taken.reverse();
    let tail = taken.concat().trim().to_string();
    // A single overlong sentence would defeat the target size — cap it.
    if estimate_tokens(&tail) > overlap_tokens.saturating_mul(2) {
        return String::new();
    }
    tail
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const SAMPLE_RESUME: &str = "Jane Doe\n\nSenior software engineer with ten years of experience building distributed systems. Passionate about reliability and developer tooling.\n\nEXPERIENCE\n\nAcme Corp — Staff Engineer (2019 to 2025). Led the migration of the billing platform to Rust. Reduced p99 latency by 40 percent. Mentored six engineers across two teams.\n\nGlobex — Senior Engineer (2015 to 2019). Built the real-time analytics pipeline processing two million events per second. Owned the on-call rotation and cut incident volume in half.\n\n## Skills\n\nRust, TypeScript, SQLite, Kafka, Kubernetes, incident response, technical writing.\n\nEducation:\n\nBSc Computer Science, State University, 2014.";

    #[test]
    fn chunking_is_deterministic_and_within_budget() {
        let a = chunk_text(SAMPLE_RESUME, 60, 12);
        let b = chunk_text(SAMPLE_RESUME, 60, 12);
        assert_eq!(a, b, "chunking must be deterministic");
        assert!(
            a.len() >= 2,
            "resume should produce several chunks, got {}",
            a.len()
        );
        for chunk in &a {
            assert!(!chunk.content.trim().is_empty());
            assert_eq!(chunk.tokens, estimate_tokens(&chunk.content));
            // target + overlap headroom; single sentences can overshoot slightly.
            assert!(
                chunk.tokens <= 60 + 12 + 20,
                "chunk too large: {} tokens",
                chunk.tokens
            );
        }
        // All the source facts survive somewhere.
        let joined = a
            .iter()
            .map(|c| c.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for needle in ["Acme Corp", "Globex", "Kafka", "State University"] {
            assert!(joined.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn headings_are_tracked() {
        let chunks = chunk_text(SAMPLE_RESUME, 60, 0);
        let with_experience: Vec<_> = chunks
            .iter()
            .filter(|c| c.heading.as_deref() == Some("EXPERIENCE"))
            .collect();
        assert!(
            !with_experience.is_empty(),
            "EXPERIENCE heading should be carried: {chunks:#?}"
        );
        assert!(
            with_experience
                .iter()
                .any(|c| c.content.contains("Acme Corp")),
            "Acme chunk should sit under EXPERIENCE"
        );
        let skills = chunks.iter().find(|c| c.content.contains("Kafka")).unwrap();
        assert_eq!(skills.heading.as_deref(), Some("Skills"));
        let education = chunks
            .iter()
            .find(|c| c.content.contains("State University"))
            .unwrap();
        assert_eq!(education.heading.as_deref(), Some("Education"));
    }

    #[test]
    fn detects_heading_styles() {
        assert_eq!(detect_heading("## Skills"), Some("Skills".into()));
        assert_eq!(detect_heading("EXPERIENCE"), Some("EXPERIENCE".into()));
        assert_eq!(detect_heading("Education:"), Some("Education".into()));
        assert_eq!(detect_heading("Work History"), Some("Work History".into()));
        assert_eq!(detect_heading("This is a normal sentence."), None);
        assert_eq!(detect_heading("lowercase start line"), None);
        assert_eq!(detect_heading(""), None);
        assert_eq!(detect_heading("1234"), None);
    }

    #[test]
    fn overlap_repeats_trailing_sentences() {
        let text = "First sentence about apples. Second sentence about pears. Third sentence about plums. Fourth sentence about grapes. Fifth sentence about kiwis. Sixth sentence about mangos.";
        let chunks = chunk_text(text, 20, 10);
        assert!(chunks.len() >= 2);
        for pair in chunks.windows(2) {
            let prev_last = pair[0]
                .content
                .unicode_sentences()
                .last()
                .unwrap()
                .trim()
                .to_string();
            assert!(
                pair[1].content.starts_with(&prev_last),
                "next chunk should start with the previous tail: {:?} vs {:?}",
                prev_last,
                pair[1].content
            );
        }
    }

    #[test]
    fn empty_and_tiny_inputs() {
        assert!(chunk_text("", 350, 40).is_empty());
        assert!(chunk_text("   \n\n  ", 350, 40).is_empty());
        let one = chunk_text("Short note.", 350, 40);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].content, "Short note.");
        assert_eq!(one[0].heading, None);
    }

    #[test]
    fn giant_unbroken_sentence_is_word_split() {
        let long = "word ".repeat(600); // ~750 tokens
        let chunks = chunk_text(&long, 100, 0);
        assert!(chunks.len() >= 6);
        assert!(chunks.iter().all(|c| c.tokens <= 120));
    }

    #[test]
    fn token_estimate_is_chars_over_four() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("héllo wörld!"), 3);
    }
}
