//! `panel_*` and `window_*` commands.

use bluey_core::types::PanelState;
use bluey_core::BlueyResult;
use bluey_protocols::panel::MoveDirection;
use tauri::{AppHandle, State};

use crate::state::AppCore;

#[tauri::command]
pub async fn panel_show(core: State<'_, AppCore>) -> BlueyResult<PanelState> {
    core.panel.show().await
}

#[tauri::command]
pub async fn panel_hide(core: State<'_, AppCore>) -> BlueyResult<PanelState> {
    core.panel.hide().await
}

#[tauri::command]
pub async fn panel_toggle(core: State<'_, AppCore>) -> BlueyResult<PanelState> {
    core.panel.toggle().await
}

#[tauri::command]
pub async fn panel_move(
    core: State<'_, AppCore>,
    direction: MoveDirection,
    step_px: Option<f64>,
) -> BlueyResult<PanelState> {
    core.panel.move_step(direction, step_px).await
}

#[tauri::command]
pub async fn panel_set_position(
    core: State<'_, AppCore>,
    x: f64,
    y: f64,
) -> BlueyResult<PanelState> {
    core.panel.set_position(x, y).await
}

#[tauri::command]
pub async fn panel_resize(
    core: State<'_, AppCore>,
    width: f64,
    height: f64,
) -> BlueyResult<PanelState> {
    core.panel.resize(width, height).await
}

#[tauri::command]
pub async fn panel_set_expanded(
    core: State<'_, AppCore>,
    expanded: bool,
    height: Option<f64>,
) -> BlueyResult<PanelState> {
    core.panel.set_expanded(expanded, height).await
}

#[tauri::command]
pub async fn panel_set_opacity(core: State<'_, AppCore>, opacity: f32) -> BlueyResult<PanelState> {
    core.panel.set_opacity(opacity).await
}

#[tauri::command]
pub async fn panel_set_pinned(core: State<'_, AppCore>, pinned: bool) -> BlueyResult<PanelState> {
    core.panel.set_pinned(pinned).await
}

#[tauri::command]
pub fn panel_get_state(core: State<'_, AppCore>) -> BlueyResult<PanelState> {
    Ok(core.panel.state())
}

#[tauri::command]
pub fn panel_start_drag(core: State<'_, AppCore>) -> BlueyResult<()> {
    core.panel.start_drag()
}

#[tauri::command]
pub fn window_open(app: AppHandle, label: String, route: Option<String>) -> BlueyResult<()> {
    crate::platform::open_window(&app, &label, route.as_deref())
}

#[tauri::command]
pub fn window_close(app: AppHandle, label: String) -> BlueyResult<()> {
    crate::platform::close_window(&app, &label)
}
