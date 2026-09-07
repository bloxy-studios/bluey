//! Text utilities shared by budgeting, context trimming and detection:
//! token estimation, char-boundary-safe truncation, line dedupe and small
//! linguistic helpers. Pure functions, no state.

/// Which part of the text to keep when truncating to a token budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keep {
    /// Keep the beginning, cut the end (good for OCR / documents).
    Head,
    /// Keep the end, cut the beginning (good for transcripts — most recent last).
    Tail,
    /// Keep both ends, cut the middle.
    HeadAndTail,
}

/// Rough token estimate without a tokenizer.
///
/// Heuristic: ~4 characters per token for Latin-ish text, ~1.5 characters per
/// token for CJK-heavy text (>20% CJK characters). Word count × 1.3 is used as
/// a lower bound so short-word text is not underestimated. Never returns 0 for
/// non-empty input.
pub fn estimate_tokens(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let mut chars: u64 = 0;
    let mut cjk: u64 = 0;
    for c in text.chars() {
        chars += 1;
        if is_cjk(c) {
            cjk += 1;
        }
    }
    let words = text.split_whitespace().count() as f64;
    let char_estimate = if cjk * 5 >= chars {
        chars as f64 / 1.5
    } else {
        chars as f64 / 4.0
    };
    let estimate = (words * 1.3).max(char_estimate).round();
    (estimate as u32).max(1)
}

fn is_cjk(c: char) -> bool {
    matches!(u32::from(c),
        0x3000..=0x303F   // CJK punctuation
        | 0x3040..=0x30FF // Hiragana + Katakana
        | 0x3400..=0x4DBF // CJK extension A
        | 0x4E00..=0x9FFF // CJK unified ideographs
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
    )
}

/// Truncate to at most `max_chars` characters (char-boundary safe), counting
/// the `ellipsis` toward the limit. Returns the input unchanged when it fits.
pub fn truncate_chars(text: &str, max_chars: usize, ellipsis: &str) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    let ellipsis_chars = ellipsis.chars().count();
    if max_chars <= ellipsis_chars {
        return text.chars().take(max_chars).collect();
    }
    let keep = max_chars - ellipsis_chars;
    let mut out: String = text.chars().take(keep).collect();
    out.push_str(ellipsis);
    out
}

/// Truncate `text` so that [`estimate_tokens`] of the result is ≤ `max_tokens`.
///
/// `keep` selects which part survives; an `…` marker is inserted at each cut.
/// Returns the input unchanged when it already fits, and an empty string for a
/// zero budget.
pub fn truncate_to_tokens(text: &str, max_tokens: u32, keep: Keep) -> String {
    if max_tokens == 0 {
        return String::new();
    }
    let estimate = estimate_tokens(text);
    if estimate <= max_tokens {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // Proportional first guess, then shrink until the estimate fits.
    let mut budget = ((chars.len() as u64 * u64::from(max_tokens)) / u64::from(estimate)) as usize;
    budget = budget.min(chars.len());
    while budget > 0 {
        let candidate = cut(&chars, budget, keep);
        if estimate_tokens(&candidate) <= max_tokens {
            return candidate;
        }
        budget = budget.saturating_sub((budget / 10).max(1));
    }
    String::new()
}

fn cut(chars: &[char], budget: usize, keep: Keep) -> String {
    match keep {
        Keep::Head => {
            let head: String = chars[..budget].iter().collect();
            format!("{head}…")
        }
        Keep::Tail => {
            let tail: String = chars[chars.len() - budget..].iter().collect();
            format!("…{tail}")
        }
        Keep::HeadAndTail => {
            let head_len = budget / 2;
            let tail_len = budget - head_len;
            let head: String = chars[..head_len].iter().collect();
            let tail: String = chars[chars.len() - tail_len..].iter().collect();
            format!("{head}\n…\n{tail}")
        }
    }
}

/// Trim every line, drop empty lines and exact duplicates (first occurrence
/// wins), preserving order. Joined with `\n`.
pub fn dedupe_lines(text: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed) {
            out.push(trimmed);
        }
    }
    out.join("\n")
}

/// Collapse all whitespace runs (including newlines) into single spaces and trim.
pub fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Leading words that make an utterance a question even without a `?`.
const INTERROGATIVES: &[&str] = &[
    "what", "why", "how", "when", "where", "who", "whom", "whose", "which", "can", "could",
    "would", "should", "will", "shall", "do", "does", "did", "is", "are", "am", "was", "were",
    "have", "has", "had", "may", "might",
];

/// Multi-word question-like openers common in spoken interviews.
const QUESTION_PREFIXES: &[&str] = &["tell me", "walk me"];

/// Heuristic question detection: ends with `?`/`？` or starts with an
/// interrogative / question-like opener ("tell me…", "walk me…").
pub fn is_question(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.ends_with('?') || trimmed.ends_with('？') {
        return true;
    }
    let lower = trimmed.to_lowercase();
    if QUESTION_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    let Some(first) = lower.split_whitespace().next() else {
        return false;
    };
    // "what's" → "what"; "whatever" stays "whatever" and does not match.
    let word: String = first.chars().take_while(|c| c.is_alphabetic()).collect();
    INTERROGATIVES.contains(&word.as_str())
}

/// Split text into sentences on `.`, `!`, `?` (when followed by whitespace or
/// end of input, so "3.14" survives), on full-width `。！？`, and on newlines.
/// Sentences are trimmed; empties are dropped.
pub fn sentence_split(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut iter = text.chars().peekable();
    while let Some(c) = iter.next() {
        if c == '\n' {
            push_sentence(&mut out, &mut current);
            continue;
        }
        current.push(c);
        let ascii_terminator = matches!(c, '.' | '!' | '?');
        let wide_terminator = matches!(c, '。' | '！' | '？');
        let at_boundary = wide_terminator
            || (ascii_terminator && iter.peek().is_none_or(|next| next.is_whitespace()));
        if at_boundary {
            push_sentence(&mut out, &mut current);
        }
    }
    push_sentence(&mut out, &mut current);
    out
}

fn push_sentence(out: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn estimates_latin_text() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 = 2.75 → 3
                                                       // Word bound dominates for short words: 8 words × 1.3 = 10.4 vs 25/4.
        assert_eq!(estimate_tokens("a b c d e f g h"), 10);
        assert!(estimate_tokens("x") >= 1);
    }

    #[test]
    fn estimates_cjk_text() {
        let cjk = "こんにちは世界これはテストです"; // 15 chars, no spaces
        assert_eq!(estimate_tokens(cjk), 10); // 15 / 1.5
        let latin = "a".repeat(15);
        assert_eq!(estimate_tokens(&latin), 4); // 15 / 4 ≈ 3.75 → 4
    }

    #[test]
    fn truncate_chars_is_boundary_safe() {
        assert_eq!(truncate_chars("hello", 10, "…"), "hello");
        assert_eq!(truncate_chars("hello world", 6, "…"), "hello…");
        // Multibyte input must not split a char.
        let s = "éééééééééé";
        let t = truncate_chars(s, 5, "…");
        assert_eq!(t.chars().count(), 5);
        assert!(t.ends_with('…'));
        // Limit smaller than the ellipsis: plain char cut, no marker.
        assert_eq!(truncate_chars("hello", 1, "…"), "h");
    }

    #[test]
    fn truncate_to_tokens_head_tail_and_both() {
        let text = format!("START {} END", "middle words ".repeat(200));
        let head = truncate_to_tokens(&text, 40, Keep::Head);
        assert!(head.starts_with("START"));
        assert!(head.ends_with('…'));
        assert!(estimate_tokens(&head) <= 40);

        let tail = truncate_to_tokens(&text, 40, Keep::Tail);
        assert!(tail.starts_with('…'));
        assert!(tail.ends_with("END"));
        assert!(estimate_tokens(&tail) <= 40);

        let both = truncate_to_tokens(&text, 40, Keep::HeadAndTail);
        assert!(both.starts_with("START"));
        assert!(both.ends_with("END"));
        assert!(both.contains('…'));
        assert!(estimate_tokens(&both) <= 40);
    }

    #[test]
    fn truncate_to_tokens_noop_and_zero() {
        assert_eq!(truncate_to_tokens("short", 100, Keep::Head), "short");
        assert_eq!(truncate_to_tokens("anything at all", 0, Keep::Tail), "");
    }

    #[test]
    fn dedupes_lines_preserving_order() {
        let input = "b\n a \n\nb\nc\na";
        assert_eq!(dedupe_lines(input), "b\na\nc");
    }

    #[test]
    fn normalizes_whitespace() {
        assert_eq!(normalize_whitespace("  a\t\tb\n\nc  "), "a b c");
        assert_eq!(normalize_whitespace(""), "");
    }

    #[test]
    fn detects_questions() {
        assert!(is_question("How does this work?"));
        assert!(is_question("what is the time complexity"));
        assert!(is_question("Would you take the offer"));
        assert!(is_question("Tell me about a time you failed"));
        assert!(is_question("What's the catch"));
        assert!(is_question("это вопрос?"));
        assert!(!is_question("Whatever happens, stay calm."));
        assert!(!is_question("The answer is 42."));
        assert!(!is_question(""));
    }

    #[test]
    fn splits_sentences() {
        assert_eq!(
            sentence_split("First one. Second one! Third?"),
            vec!["First one.", "Second one!", "Third?"]
        );
        assert_eq!(
            sentence_split("Pi is 3.14 exactly. Yes."),
            vec!["Pi is 3.14 exactly.", "Yes."]
        );
        assert_eq!(
            sentence_split("line one\nline two"),
            vec!["line one", "line two"]
        );
        assert_eq!(
            sentence_split("こんにちは。元気？"),
            vec!["こんにちは。", "元気？"]
        );
        assert_eq!(sentence_split("  "), Vec::<String>::new());
        assert_eq!(sentence_split("no terminator"), vec!["no terminator"]);
    }
}
