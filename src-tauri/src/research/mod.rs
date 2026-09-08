//! Research clients that run in Rust: Exa search and Firecrawl scrape (keys from
//! the Keychain, bodies never logged) plus the hand-off to the deep-research
//! agent sidecar. The Research Router in TypeScript decides *which* path runs.

use std::sync::Arc;
use std::time::Duration;

use bluey_core::error::RecoveryAction;
use bluey_core::types::{DeepResearchRequest, ScrapeResult, SearchResult};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::{exa, firecrawl};
use serde::Serialize;

use crate::agent::AgentManager;
use crate::ai::providers::{map_http_status, map_transport_error};
use crate::secrets::{SecretsStore, EXA_KEY, FIRECRAWL_KEY};

const EXA_TIMEOUT: Duration = Duration::from_secs(20);
const FIRECRAWL_TIMEOUT: Duration = Duration::from_secs(45);
const DEFAULT_NUM_RESULTS: u32 = 8;
const MAX_NUM_RESULTS: u32 = 10;

/// Mirrors the `research_available` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchAvailability {
    pub search: bool,
    pub scrape: bool,
    pub deep_agent: bool,
}

pub struct ResearchManager {
    http: reqwest::Client,
    secrets: Arc<SecretsStore>,
    agent: Arc<AgentManager>,
}

impl ResearchManager {
    pub fn new(
        http: reqwest::Client,
        secrets: Arc<SecretsStore>,
        agent: Arc<AgentManager>,
    ) -> Self {
        Self {
            http,
            secrets,
            agent,
        }
    }

    fn missing_key(tool: &str, key_name: &str) -> BlueyError {
        BlueyError::configuration(
            "missing_key",
            format!("{tool} is not configured — add your {key_name} in Settings → AI"),
        )
        .recoverable(RecoveryAction::OpenSettings { tab: "ai".into() })
    }

    /// Exa `POST /search` (public queries only — the caller scrubbed them).
    pub async fn search(
        &self,
        query: &str,
        num_results: Option<u32>,
    ) -> BlueyResult<Vec<SearchResult>> {
        let key = self
            .secrets
            .get(EXA_KEY)
            .await?
            .ok_or_else(|| Self::missing_key("Web search", "Exa API key"))?;
        let n = num_results
            .unwrap_or(DEFAULT_NUM_RESULTS)
            .clamp(1, MAX_NUM_RESULTS);
        let response = self
            .http
            .post(exa::SEARCH_URL)
            .header("x-api-key", key)
            .timeout(EXA_TIMEOUT)
            .json(&exa::build_search_body(query, n))
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "Exa"))?;
        let status = response.status().as_u16();
        if status >= 400 {
            return Err(map_http_status(status, "Exa"));
        }
        let parsed: exa::SearchResponse = response
            .json()
            .await
            .map_err(|_| BlueyError::research("search_parse", "unexpected Exa response"))?;
        Ok(exa::to_search_results(parsed))
    }

    /// Firecrawl v2 scrape (markdown, main content only).
    pub async fn scrape(&self, url: &str) -> BlueyResult<ScrapeResult> {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(BlueyError::invalid_params("scrape URLs must be http(s)"));
        }
        let key = self
            .secrets
            .get(FIRECRAWL_KEY)
            .await?
            .ok_or_else(|| Self::missing_key("Page reading", "Firecrawl API key"))?;
        let response = self
            .http
            .post(firecrawl::SCRAPE_URL)
            .bearer_auth(key)
            .timeout(FIRECRAWL_TIMEOUT)
            .json(&firecrawl::build_scrape_body(url))
            .send()
            .await
            .map_err(|e| map_transport_error(&e, "Firecrawl"))?;
        let status = response.status().as_u16();
        if status >= 400 {
            return Err(map_http_status(status, "Firecrawl"));
        }
        let parsed: firecrawl::ScrapeResponse = response
            .json()
            .await
            .map_err(|_| BlueyError::research("scrape_parse", "unexpected Firecrawl response"))?;
        firecrawl::to_scrape_result(url, parsed)
            .map_err(|reason| BlueyError::research("scrape_failed", reason))
    }

    /// Which research paths are usable right now.
    pub async fn availability(&self) -> ResearchAvailability {
        let search = self.secrets.has(EXA_KEY).await.unwrap_or(false);
        let scrape = self.secrets.has(FIRECRAWL_KEY).await.unwrap_or(false);
        let deep_agent = self.agent.available().await;
        ResearchAvailability {
            search,
            scrape,
            deep_agent,
        }
    }

    /// Start a deep-research job in the agent sidecar (events arrive as
    /// `research.event`).
    pub async fn deep_start(&self, request: DeepResearchRequest) -> BlueyResult<()> {
        self.agent.start(request).await
    }

    /// Cancel a running job. Returns whether the job was known.
    pub async fn deep_cancel(&self, job_id: &str) -> bool {
        self.agent.cancel(job_id).await
    }
}
