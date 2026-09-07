//! Private FTS5 / LIKE query helpers shared by [`crate::search`] and
//! [`crate::documents::retrieve`].

/// Small English stop-word list dropped from FTS match expressions.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "did", "do", "does", "for",
    "from", "had", "has", "have", "how", "if", "in", "into", "is", "it", "its", "of", "on", "or",
    "our", "so", "than", "that", "the", "their", "them", "then", "there", "these", "they", "this",
    "to", "was", "we", "were", "what", "when", "where", "which", "who", "why", "will", "with",
    "would", "you", "your",
];

/// Build a sanitized FTS5 MATCH expression from free text.
///
/// Terms are reduced to alphanumeric runs (which removes quotes and every FTS5
/// operator character), lower-cased, de-duplicated, filtered against a stop-word
/// list, wrapped in double quotes and OR-ed together. Returns `None` when nothing
/// searchable remains.
pub(crate) fn fts_match_query(text: &str) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        let term = raw.trim().to_lowercase();
        if term.is_empty() || STOP_WORDS.contains(&term.as_str()) {
            continue;
        }
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

/// Escape `%`, `_` and the escape character itself for a `LIKE ?1 ESCAPE '\'` pattern,
/// returning `%…%` for a contains-match.
pub(crate) fn like_contains_pattern(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_query_sanitizes_and_ors_terms() {
        let q = fts_match_query("Tell me about the \"Rust\" NEAR( experience!").unwrap();
        assert_eq!(
            q,
            "\"tell\" OR \"me\" OR \"about\" OR \"rust\" OR \"near\" OR \"experience\""
        );
    }

    #[test]
    fn match_query_drops_stop_words_and_dupes() {
        let q = fts_match_query("the of and rust rust").unwrap();
        assert_eq!(q, "\"rust\"");
        assert!(fts_match_query("the of and").is_none());
        assert!(fts_match_query("  ").is_none());
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_contains_pattern("50%_a\\b"), "%50\\%\\_a\\\\b%");
    }
}
