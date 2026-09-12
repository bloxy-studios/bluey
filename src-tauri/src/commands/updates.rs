//! `updates_*` commands: the in-app updater's status and the three user
//! actions — check now, install, relaunch (docs/UPDATES.md). The cycle itself
//! lives in [`crate::updates::UpdatesManager`]; transitions arrive as
//! `update.status` events.

use bluey_core::types::UpdateStatus;
use bluey_core::BlueyResult;
use tauri::State;

use crate::state::AppCore;
use crate::updates::CheckTrigger;

#[tauri::command]
pub fn updates_get_status(core: State<'_, AppCore>) -> BlueyResult<UpdateStatus> {
    Ok(core.updates.status())
}

#[tauri::command]
pub async fn updates_check(core: State<'_, AppCore>) -> BlueyResult<UpdateStatus> {
    core.updates.check(CheckTrigger::Manual).await
}

#[tauri::command]
pub async fn updates_install(core: State<'_, AppCore>) -> BlueyResult<UpdateStatus> {
    core.updates.install().await
}

#[tauri::command]
pub fn updates_relaunch(core: State<'_, AppCore>) -> BlueyResult<()> {
    core.updates.relaunch()
}
