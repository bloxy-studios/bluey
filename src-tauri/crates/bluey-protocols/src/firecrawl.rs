//! Firecrawl v2 scrape API (`POST https://api.firecrawl.dev/v2/scrape`,
//! Bearer auth) request and response models mapped onto
//! `bluey_core::types::ScrapeResult`.

use bluey_core::types::{ScrapeResult, SearchSource};
use serde::Deserialize;
use serde_json::{json, Value};

/// Scrape endpoint.
pub const SCRAPE_URL: &str = "https://api.firecrawl.dev/v2/scrape";

/// Build the scrape body: markdown format, main content only.
pub fn build_scrape_body(url: &str) -> Value {
    json!({
        "url": url,
        "formats": ["markdown"],
        "onlyMainContent": true,
    })
}

/// Scrape response envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct ScrapeResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub data: Option<ScrapeData>,
    /// Error message on failure responses.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeData {
    #[serde(default)]
    pub markdown: Option<String>,
    #[serde(default)]
    pub metadata: Option<ScrapeMetadata>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeMetadata {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "sourceURL")]
    pub source_url: Option<String>,
}

/// Map a successful scrape response onto a [`ScrapeResult`]. Returns the
/// server-side error message on `success: false` or missing markdown.
pub fn to_scrape_result(
    requested_url: &str,
    response: ScrapeResponse,
) -> Result<ScrapeResult, String> {
    if !response.success {
        return Err(response
            .error
            .unwrap_or_else(|| "scrape was not successful".to_string()));
    }
    let data = response.data.unwrap_or_default();
    let markdown = data
        .markdown
        .ok_or_else(|| "scrape response contained no markdown".to_string())?;
    let metadata = data.metadata.unwrap_or_default();
    Ok(ScrapeResult {
        url: metadata
            .source_url
            .unwrap_or_else(|| requested_url.to_string()),
        title: metadata.title.filter(|t| !t.trim().is_empty()),
        markdown,
        source: SearchSource::Firecrawl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn body_shape() {
        let body = build_scrape_body("https://tauri.app");
        assert_eq!(body["url"], "https://tauri.app");
        assert_eq!(body["formats"][0], "markdown");
        assert_eq!(body["onlyMainContent"], true);
    }

    #[test]
    fn maps_success() {
        let raw = r##"{
          "success": true,
          "data": {
            "markdown": "# Title\n\nBody.",
            "metadata": { "title": "Title", "sourceURL": "https://tauri.app/", "statusCode": 200 }
          }
        }"##;
        let resp: ScrapeResponse = serde_json::from_str(raw).unwrap();
        let result = to_scrape_result("https://tauri.app", resp).unwrap();
        assert_eq!(result.url, "https://tauri.app/");
        assert_eq!(result.title.as_deref(), Some("Title"));
        assert_eq!(result.markdown, "# Title\n\nBody.");
        assert_eq!(result.source, SearchSource::Firecrawl);
    }

    #[test]
    fn failure_and_missing_markdown() {
        let resp: ScrapeResponse =
            serde_json::from_str(r#"{"success": false, "error": "denied"}"#).unwrap();
        assert_eq!(to_scrape_result("https://x", resp).unwrap_err(), "denied");

        let resp: ScrapeResponse =
            serde_json::from_str(r#"{"success": true, "data": {}}"#).unwrap();
        assert!(to_scrape_result("https://x", resp)
            .unwrap_err()
            .contains("no markdown"));
    }
}
