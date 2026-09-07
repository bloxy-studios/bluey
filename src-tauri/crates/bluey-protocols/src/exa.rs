//! Exa search API (`POST https://api.exa.ai/search`, `x-api-key`) request and
//! response models mapped onto `bluey_core::types::SearchResult`.

use bluey_core::types::{SearchResult, SearchSource};
use serde::Deserialize;
use serde_json::{json, Value};

/// Search endpoint.
pub const SEARCH_URL: &str = "https://api.exa.ai/search";

/// Build the search body: `type: "auto"` with highlights + summary contents.
pub fn build_search_body(query: &str, num_results: u32) -> Value {
    json!({
        "query": query,
        "type": "auto",
        "numResults": num_results,
        "contents": {
            "highlights": true,
            "summary": true,
        }
    })
}

/// Search response envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    #[serde(default)]
    pub results: Vec<ExaResult>,
}

/// One Exa result row.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExaResult {
    #[serde(default)]
    pub id: Option<String>,
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub published_date: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub highlights: Vec<String>,
    #[serde(default)]
    pub text: Option<String>,
}

/// Max characters of a snippet built from raw page text.
const SNIPPET_CHARS: usize = 280;

/// Map an Exa response onto Bluey [`SearchResult`]s. Snippet preference:
/// summary → first highlight → text prefix.
pub fn to_search_results(response: SearchResponse) -> Vec<SearchResult> {
    response
        .results
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let snippet = r
                .summary
                .filter(|s| !s.trim().is_empty())
                .or_else(|| r.highlights.into_iter().find(|h| !h.trim().is_empty()))
                .or_else(|| {
                    r.text.map(|t| {
                        let mut s: String = t.chars().take(SNIPPET_CHARS).collect();
                        if t.chars().count() > SNIPPET_CHARS {
                            s.push('…');
                        }
                        s
                    })
                });
            SearchResult {
                id: r.id.unwrap_or_else(|| format!("exa_{i}")),
                title: r.title.unwrap_or_else(|| r.url.clone()),
                url: r.url,
                snippet,
                published_at: r.published_date,
                source: SearchSource::Exa,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn body_shape() {
        let body = build_search_body("rust tauri", 5);
        assert_eq!(body["query"], "rust tauri");
        assert_eq!(body["type"], "auto");
        assert_eq!(body["numResults"], 5);
        assert_eq!(body["contents"]["highlights"], true);
        assert_eq!(body["contents"]["summary"], true);
    }

    #[test]
    fn maps_results_with_snippet_preference() {
        let raw = r#"{
          "requestId": "req",
          "resolvedSearchType": "neural",
          "results": [
            { "id": "a1", "url": "https://one.dev", "title": "One",
              "publishedDate": "2026-01-02T00:00:00.000Z",
              "summary": "The summary.", "highlights": ["hl"] },
            { "url": "https://two.dev", "title": "Two", "highlights": ["second highlight"] },
            { "url": "https://three.dev", "text": "plain text body" }
          ]
        }"#;
        let resp: SearchResponse = serde_json::from_str(raw).unwrap();
        let results = to_search_results(resp);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].id, "a1");
        assert_eq!(results[0].snippet.as_deref(), Some("The summary."));
        assert_eq!(
            results[0].published_at.as_deref(),
            Some("2026-01-02T00:00:00.000Z")
        );
        assert_eq!(results[1].id, "exa_1");
        assert_eq!(results[1].snippet.as_deref(), Some("second highlight"));
        assert_eq!(results[2].title, "https://three.dev");
        assert_eq!(results[2].snippet.as_deref(), Some("plain text body"));
        assert!(results.iter().all(|r| r.source == SearchSource::Exa));
    }

    #[test]
    fn long_text_snippet_is_truncated() {
        let resp = SearchResponse {
            results: vec![ExaResult {
                id: None,
                url: "https://x.dev".into(),
                title: None,
                published_date: None,
                summary: None,
                highlights: vec![],
                text: Some("y".repeat(500)),
            }],
        };
        let results = to_search_results(resp);
        let snippet = results[0].snippet.as_deref().unwrap();
        assert_eq!(snippet.chars().count(), 281);
        assert!(snippet.ends_with('…'));
    }
}
