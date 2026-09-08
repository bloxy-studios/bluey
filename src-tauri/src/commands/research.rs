//! `research_*` commands: Exa search, Firecrawl scrape and the deep-research agent.

use bluey_core::types::{DeepResearchRequest, ScrapeResult, SearchResult};
use bluey_core::BlueyResult;
use tauri::State;

use crate::research::ResearchAvailability;
use crate::state::AppCore;

#[tauri::command]
pub async fn research_search(
    core: State<'_, AppCore>,
    query: String,
    num_results: Option<u32>,
) -> BlueyResult<Vec<SearchResult>> {
    core.research.search(&query, num_results).await
}

#[tauri::command]
pub async fn research_scrape(core: State<'_, AppCore>, url: String) -> BlueyResult<ScrapeResult> {
    core.research.scrape(&url).await
}

#[tauri::command]
pub async fn research_deep_start(
    core: State<'_, AppCore>,
    request: DeepResearchRequest,
) -> BlueyResult<()> {
    core.research.deep_start(request).await
}

#[tauri::command]
pub async fn research_deep_cancel(core: State<'_, AppCore>, job_id: String) -> BlueyResult<bool> {
    Ok(core.research.deep_cancel(&job_id).await)
}

#[tauri::command]
pub async fn research_available(core: State<'_, AppCore>) -> BlueyResult<ResearchAvailability> {
    Ok(core.research.availability().await)
}
