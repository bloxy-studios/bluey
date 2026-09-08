//! `permissions_*` commands.

use bluey_core::types::{PermissionKind, PermissionState};
use bluey_core::BlueyResult;
use tauri::State;

use crate::state::AppCore;

#[tauri::command]
pub async fn permissions_get(core: State<'_, AppCore>) -> BlueyResult<PermissionState> {
    core.permissions.refresh().await
}

#[tauri::command]
pub async fn permissions_request(
    core: State<'_, AppCore>,
    kind: PermissionKind,
) -> BlueyResult<PermissionState> {
    core.permissions.request(kind).await
}

#[tauri::command]
pub fn permissions_open_settings(
    core: State<'_, AppCore>,
    kind: PermissionKind,
) -> BlueyResult<()> {
    core.permissions.open_settings(kind)
}
