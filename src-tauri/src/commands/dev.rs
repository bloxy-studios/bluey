//! `dev_*` commands (developer mode).

use bluey_core::types::{BenchOptions, BenchReport, DevSimulation, LatencyMetrics};
use bluey_core::BlueyResult;
use tauri::State;

use crate::state::AppCore;

#[tauri::command]
pub async fn dev_simulate(core: State<'_, AppCore>, simulation: DevSimulation) -> BlueyResult<()> {
    crate::app::dev::simulate(&core, simulation).await
}

#[tauri::command]
pub fn dev_get_metrics(core: State<'_, AppCore>) -> BlueyResult<LatencyMetrics> {
    Ok(core.metrics.snapshot())
}

#[tauri::command]
pub async fn dev_restart_helper(core: State<'_, AppCore>) -> BlueyResult<()> {
    core.helper.restart().await
}

/// The ⌘↵ fast-path bench (ADR 0010 §2): `iterations` runs of capture → snapshot →
/// request against `provider`, as a percentile table. Developer mode / `dev-tools`.
#[tauri::command]
pub async fn dev_bench_fast_path(
    core: State<'_, AppCore>,
    options: BenchOptions,
) -> BlueyResult<BenchReport> {
    crate::app::bench::run(&core, options).await
}
