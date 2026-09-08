//! `shortcuts_*` commands.

use bluey_core::types::{ShortcutBinding, ShortcutConflict, ShortcutId};
use bluey_core::BlueyResult;
use tauri::State;

use crate::state::AppCore;

#[tauri::command]
pub fn shortcuts_list(core: State<'_, AppCore>) -> BlueyResult<Vec<ShortcutBinding>> {
    Ok(core.shortcuts.list())
}

#[tauri::command]
pub async fn shortcuts_update(
    core: State<'_, AppCore>,
    id: ShortcutId,
    accelerator: String,
    enabled: Option<bool>,
) -> BlueyResult<Vec<ShortcutBinding>> {
    core.shortcuts.update(id, accelerator, enabled).await
}

#[tauri::command]
pub async fn shortcuts_reset(core: State<'_, AppCore>) -> BlueyResult<Vec<ShortcutBinding>> {
    core.shortcuts.reset().await
}

#[tauri::command]
pub fn shortcuts_check_conflict(
    core: State<'_, AppCore>,
    accelerator: String,
    ignore_id: Option<ShortcutId>,
) -> BlueyResult<Option<ShortcutConflict>> {
    Ok(core.shortcuts.check_conflict(&accelerator, ignore_id))
}
