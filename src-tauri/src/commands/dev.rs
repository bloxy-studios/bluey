//! `dev_*` commands (developer mode).

use bluey_core::types::{DevSimulation, LatencyMetrics};
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
