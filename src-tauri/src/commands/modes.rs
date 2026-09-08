//! `modes_*` commands.

use bluey_core::types::{AppStatus, BlueyMode, ModePatch, Settings};
use bluey_core::BlueyResult;
use tauri::State;

use crate::state::AppCore;

#[tauri::command]
pub async fn modes_list(core: State<'_, AppCore>) -> BlueyResult<Vec<BlueyMode>> {
    core.modes.list().await
}

#[tauri::command]
pub async fn modes_get(core: State<'_, AppCore>, id: String) -> BlueyResult<BlueyMode> {
    core.modes.get(id).await
}

#[tauri::command]
pub async fn modes_create(core: State<'_, AppCore>, draft: ModePatch) -> BlueyResult<BlueyMode> {
    core.modes.create(draft).await
}

#[tauri::command]
pub async fn modes_update(
    core: State<'_, AppCore>,
    id: String,
    patch: ModePatch,
) -> BlueyResult<BlueyMode> {
    core.modes.update(id, patch).await
}

#[tauri::command]
pub async fn modes_delete(core: State<'_, AppCore>, id: String) -> BlueyResult<()> {
    core.modes.delete(id).await
}

#[tauri::command]
pub async fn modes_duplicate(core: State<'_, AppCore>, id: String) -> BlueyResult<BlueyMode> {
    core.modes.duplicate(id).await
}

#[tauri::command]
pub async fn modes_set_default(core: State<'_, AppCore>, id: String) -> BlueyResult<Settings> {
    core.modes.get(id.clone()).await?;
    core.settings.set_default_mode(&id).await
}

#[tauri::command]
pub async fn modes_set_active(core: State<'_, AppCore>, id: String) -> BlueyResult<AppStatus> {
    core.modes.set_active(id).await
}

#[tauri::command]
pub async fn modes_reset_built_in(core: State<'_, AppCore>, id: String) -> BlueyResult<BlueyMode> {
    core.modes.reset_built_in(id).await
}
