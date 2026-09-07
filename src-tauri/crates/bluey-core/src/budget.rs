//! Context token budgeting (spec §77): score every [`ContextItem`], keep the
//! most valuable ones inside the budget, compress what almost fits and drop
//! the rest — while never letting transcripts lose their most recent lines.

use std::collections::HashMap;

use crate::text::{estimate_tokens, truncate_to_tokens, Keep};
use crate::types::{ContextItem, ContextSource};

/// Priorities and per-source caps used by [`allocate`].
///
/// `score = priority(source) × relevance` decides inclusion order. Each source
/// may consume at most `max_share(source) × budget` tokens so one noisy source
/// (e.g. OCR of a dense page) cannot starve the others. Items whose score is
/// at least `compress_threshold` are compressed instead of dropped when they
/// do not fit, as long as at least `min_compress_tokens` of space remain.
#[derive(Debug, Clone)]
pub struct BudgetPolicy {
    /// Priority per source (0.0–1.0). Missing sources default to 0.5.
    pub priorities: HashMap<ContextSource, f32>,
    /// Max share of the total budget per source (0.0–1.0). Missing → 1.0.
    pub max_share: HashMap<ContextSource, f32>,
    /// Minimum score for an oversized item to be compressed instead of dropped.
    pub compress_threshold: f32,
    /// Minimum room (tokens) worth compressing into.
    pub min_compress_tokens: u32,
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        let priorities = HashMap::from([
            (ContextSource::UserInstruction, 1.0),
            (ContextSource::Transcript, 0.9),
            (ContextSource::Ocr, 0.85),
            (ContextSource::Accessibility, 0.85),
            (ContextSource::Screen, 0.8),
            (ContextSource::PersonalInstructions, 0.8),
            (ContextSource::Resume, 0.7),
            (ContextSource::JobDescription, 0.7),
            (ContextSource::SessionMemory, 0.55),
            (ContextSource::Document, 0.5),
            (ContextSource::TranscriptOld, 0.3),
        ]);
        let max_share = HashMap::from([
            (ContextSource::UserInstruction, 1.0),
            (ContextSource::Transcript, 0.5),
            (ContextSource::Ocr, 0.4),
            (ContextSource::Accessibility, 0.3),
            (ContextSource::Screen, 0.25),
            (ContextSource::PersonalInstructions, 0.15),
            (ContextSource::Resume, 0.35),
            (ContextSource::JobDescription, 0.35),
            (ContextSource::SessionMemory, 0.25),
            (ContextSource::Document, 0.35),
            (ContextSource::TranscriptOld, 0.2),
        ]);
        Self {
            priorities,
            max_share,
            compress_threshold: 0.45,
            min_compress_tokens: 48,
        }
    }
}

impl BudgetPolicy {
    /// Priority of a source (default 0.5 when unlisted).
    pub fn priority(&self, source: ContextSource) -> f32 {
        self.priorities.get(&source).copied().unwrap_or(0.5)
    }

    /// Max share of the budget a source may use (default 1.0 when unlisted).
    pub fn share(&self, source: ContextSource) -> f32 {
        self.max_share.get(&source).copied().unwrap_or(1.0)
    }

    /// Inclusion score: `priority × relevance` (relevance clamped to 0–1).
    pub fn score(&self, item: &ContextItem) -> f32 {
        self.priority(item.source) * item.relevance.clamp(0.0, 1.0)
    }
}

/// Result of [`allocate`].
#[derive(Debug, Clone, Default)]
pub struct BudgetResult {
    /// Items that made it into the prompt, in their original relative order.
    pub included: Vec<ContextItem>,
    /// Items dropped entirely, in their original relative order.
    pub dropped: Vec<ContextItem>,
    /// `(source, tokens_before, tokens_after)` for every compressed item.
    pub compressed: Vec<(ContextSource, u32, u32)>,
    /// Sum of tokens over `included`.
    pub total_tokens: u32,
}

/// Fit `items` into `budget_tokens`.
///
/// Algorithm: user instructions are always included whole (the single case
/// allowed to exceed the budget — a caller's instruction is never dropped).
/// The rest is sorted by score and greedily added; an item that does not fit
/// but scores ≥ `compress_threshold` is compressed with
/// [`truncate_to_tokens`] — transcripts keep their tail (most recent speech),
/// OCR keeps its head, everything else keeps head and tail. Output order is
/// the original item order, so transcript items stay chronological.
pub fn allocate(
    items: Vec<ContextItem>,
    budget_tokens: u32,
    policy: &BudgetPolicy,
) -> BudgetResult {
    let mut included: Vec<(usize, ContextItem)> = Vec::new();
    let mut dropped: Vec<(usize, ContextItem)> = Vec::new();
    let mut compressed: Vec<(ContextSource, u32, u32)> = Vec::new();
    let mut total: u32 = 0;
    let mut used_by_source: HashMap<ContextSource, u32> = HashMap::new();

    let mut rest: Vec<(usize, ContextItem)> = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        if item.source == ContextSource::UserInstruction {
            total = total.saturating_add(item.tokens);
            *used_by_source.entry(item.source).or_insert(0) += item.tokens;
            included.push((index, item));
        } else {
            rest.push((index, item));
        }
    }

    rest.sort_by(|(ia, a), (ib, b)| {
        policy
            .score(b)
            .partial_cmp(&policy.score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(ia.cmp(ib))
    });

    for (index, mut item) in rest {
        let remaining = budget_tokens.saturating_sub(total);
        let source_cap = (f64::from(budget_tokens) * f64::from(policy.share(item.source))) as u32;
        let source_used = used_by_source.get(&item.source).copied().unwrap_or(0);
        let available = remaining.min(source_cap.saturating_sub(source_used));

        if item.tokens <= available {
            total += item.tokens;
            *used_by_source.entry(item.source).or_insert(0) += item.tokens;
            included.push((index, item));
            continue;
        }

        let worth_compressing = policy.score(&item) >= policy.compress_threshold
            && available >= policy.min_compress_tokens;
        if worth_compressing {
            let content = truncate_to_tokens(&item.content, available, keep_for(item.source));
            let new_tokens = estimate_tokens(&content);
            if new_tokens > 0 && new_tokens <= available {
                compressed.push((item.source, item.tokens, new_tokens));
                item.content = content;
                item.tokens = new_tokens;
                total += new_tokens;
                *used_by_source.entry(item.source).or_insert(0) += new_tokens;
                included.push((index, item));
                continue;
            }
        }
        dropped.push((index, item));
    }

    included.sort_by_key(|(index, _)| *index);
    dropped.sort_by_key(|(index, _)| *index);
    BudgetResult {
        included: included.into_iter().map(|(_, item)| item).collect(),
        dropped: dropped.into_iter().map(|(_, item)| item).collect(),
        compressed,
        total_tokens: total,
    }
}

/// Which part of an oversized item survives compression.
fn keep_for(source: ContextSource) -> Keep {
    match source {
        // The most recent speech is at the end.
        ContextSource::Transcript | ContextSource::TranscriptOld => Keep::Tail,
        // OCR reads top-to-bottom; the top of the screen matters most.
        ContextSource::Ocr => Keep::Head,
        _ => Keep::HeadAndTail,
    }
}

/// Human label for a context source, used in prompts and notes.
pub fn source_label(source: ContextSource) -> &'static str {
    match source {
        ContextSource::UserInstruction => "user instruction",
        ContextSource::Screen => "screen image",
        ContextSource::Ocr => "screen text (OCR)",
        ContextSource::Accessibility => "accessibility text",
        ContextSource::Transcript => "recent transcript",
        ContextSource::TranscriptOld => "older transcript",
        ContextSource::Resume => "resume",
        ContextSource::JobDescription => "job description",
        ContextSource::Document => "attached documents",
        ContextSource::SessionMemory => "session memory",
        ContextSource::PersonalInstructions => "personal instructions",
    }
}

/// Short note listing what was dropped (unique sources, first-seen order), so
/// prompts can say e.g. "older transcript omitted". `None` when nothing was
/// dropped. Never includes the dropped content itself.
pub fn summarize_dropped(dropped: &[ContextItem]) -> Option<String> {
    if dropped.is_empty() {
        return None;
    }
    let mut seen = Vec::new();
    for item in dropped {
        let label = source_label(item.source);
        if !seen.contains(&label) {
            seen.push(label);
        }
    }
    Some(format!(
        "Omitted to fit the context budget: {}.",
        seen.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn item(source: ContextSource, content: &str, relevance: f32) -> ContextItem {
        ContextItem {
            source,
            content: content.to_string(),
            relevance,
            tokens: estimate_tokens(content),
            r#ref: None,
        }
    }

    fn words(n: usize, word: &str) -> String {
        vec![word; n].join(" ")
    }

    #[test]
    fn respects_the_budget() {
        let items = vec![
            item(ContextSource::Ocr, &words(400, "ocr"), 0.9),
            item(ContextSource::Transcript, &words(400, "talk"), 0.9),
            item(ContextSource::Document, &words(400, "doc"), 0.9),
            item(ContextSource::Resume, &words(400, "cv"), 0.9),
        ];
        let budget = 300;
        let result = allocate(items, budget, &BudgetPolicy::default());
        assert!(
            result.total_tokens <= budget,
            "total {} > budget",
            result.total_tokens
        );
        let sum: u32 = result.included.iter().map(|i| i.tokens).sum();
        assert_eq!(sum, result.total_tokens);
    }

    #[test]
    fn user_instruction_is_always_kept_whole() {
        let instruction = words(100, "please");
        let items = vec![
            item(ContextSource::Document, &words(500, "doc"), 1.0),
            item(ContextSource::UserInstruction, &instruction, 0.0), // relevance ignored
        ];
        let result = allocate(items, 50, &BudgetPolicy::default());
        let kept: Vec<_> = result
            .included
            .iter()
            .filter(|i| i.source == ContextSource::UserInstruction)
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].content, instruction,
            "instruction is never truncated"
        );
    }

    #[test]
    fn low_relevance_is_dropped_first() {
        let items = vec![
            item(ContextSource::Document, &words(120, "boring"), 0.05),
            item(ContextSource::Document, &words(120, "vital"), 0.95),
        ];
        // Room for only one document (document share 0.35 × 600 = 210).
        let result = allocate(items, 600, &BudgetPolicy::default());
        assert!(result.included.iter().any(|i| i.content.contains("vital")));
        assert!(result.dropped.iter().any(|i| i.content.contains("boring")));
    }

    #[test]
    fn transcript_compression_keeps_the_tail() {
        let content = format!("OLDEST_LINE {} NEWEST_LINE", words(2_000, "speech"));
        let items = vec![item(ContextSource::Transcript, &content, 0.9)];
        let result = allocate(items, 400, &BudgetPolicy::default());
        assert_eq!(result.included.len(), 1);
        let kept = &result.included[0];
        assert!(
            kept.content.ends_with("NEWEST_LINE"),
            "most recent speech survives"
        );
        assert!(
            !kept.content.contains("OLDEST_LINE"),
            "oldest speech is cut"
        );
        assert_eq!(result.compressed.len(), 1);
        let (source, from, to) = result.compressed[0];
        assert_eq!(source, ContextSource::Transcript);
        assert!(from > to);
        assert!(result.total_tokens <= 400);
    }

    #[test]
    fn ocr_compression_keeps_the_head() {
        let content = format!("TOP_OF_SCREEN {} BOTTOM_OF_SCREEN", words(2_000, "text"));
        let items = vec![item(ContextSource::Ocr, &content, 0.9)];
        let result = allocate(items, 400, &BudgetPolicy::default());
        assert_eq!(result.included.len(), 1);
        assert!(result.included[0].content.starts_with("TOP_OF_SCREEN"));
        assert!(!result.included[0].content.contains("BOTTOM_OF_SCREEN"));
    }

    #[test]
    fn output_preserves_original_order_within_sources() {
        let items = vec![
            item(
                ContextSource::Transcript,
                "first chronological chunk of speech here",
                0.5,
            ),
            item(
                ContextSource::Transcript,
                "second chronological chunk of speech here",
                0.9,
            ),
        ];
        let result = allocate(items, 10_000, &BudgetPolicy::default());
        assert_eq!(result.included.len(), 2);
        assert!(
            result.included[0].content.starts_with("first"),
            "chronology preserved"
        );
        assert!(result.included[1].content.starts_with("second"));
    }

    #[test]
    fn summarizes_dropped_sources() {
        assert_eq!(summarize_dropped(&[]), None);
        let dropped = vec![
            item(ContextSource::TranscriptOld, "old talk", 0.1),
            item(ContextSource::Document, "doc", 0.1),
            item(ContextSource::TranscriptOld, "older talk", 0.1),
        ];
        let note = summarize_dropped(&dropped).expect("note");
        assert_eq!(
            note,
            "Omitted to fit the context budget: older transcript, attached documents."
        );
        assert!(!note.contains("old talk"), "note never leaks content");
    }
}
