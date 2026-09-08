//! `capture_*`, `ocr_*` and `accessibility_*` commands.

use bluey_core::types::{
    AccessibilityContext, CaptureOptions, CaptureProtection, DisplayInfo, OcrContext, OcrLevel,
    ScreenFrame,
};
use bluey_core::BlueyResult;
use bluey_protocols::helper::{CapturableWindow, FrontmostApp};
use tauri::State;

use crate::state::AppCore;

#[tauri::command]
pub async fn capture_list_displays(core: State<'_, AppCore>) -> BlueyResult<Vec<DisplayInfo>> {
    core.capture.list_displays().await
}

#[tauri::command]
pub async fn capture_list_windows(core: State<'_, AppCore>) -> BlueyResult<Vec<CapturableWindow>> {
    core.capture.list_windows().await
}

#[tauri::command]
pub async fn capture_screen(
    core: State<'_, AppCore>,
    options: Option<CaptureOptions>,
) -> BlueyResult<ScreenFrame> {
    core.capture.capture(options).await
}

#[tauri::command]
pub async fn capture_read_frame(core: State<'_, AppCore>, frame_id: String) -> BlueyResult<String> {
    core.capture.read_frame(&frame_id).await
}

#[tauri::command]
pub async fn capture_discard_frame(core: State<'_, AppCore>, frame_id: String) -> BlueyResult<()> {
    core.capture.discard_frame(&frame_id).await
}

#[tauri::command]
pub async fn capture_observe_start(
    core: State<'_, AppCore>,
    interval_ms: Option<u32>,
    display_id: Option<String>,
) -> BlueyResult<()> {
    core.capture.observe_start(interval_ms, display_id).await
}

#[tauri::command]
pub async fn capture_observe_stop(core: State<'_, AppCore>) -> BlueyResult<()> {
    core.capture.observe_stop().await
}

#[tauri::command]
pub fn capture_get_protection(core: State<'_, AppCore>) -> BlueyResult<CaptureProtection> {
    Ok(core.capture.protection())
}

#[tauri::command]
pub fn capture_set_protection(
    core: State<'_, AppCore>,
    enabled: bool,
) -> BlueyResult<CaptureProtection> {
    core.capture.set_protection(enabled)
}

#[tauri::command]
pub async fn ocr_recognize(
    core: State<'_, AppCore>,
    frame_id: String,
    level: Option<OcrLevel>,
    languages: Option<Vec<String>>,
) -> BlueyResult<OcrContext> {
    core.capture
        .ocr(&frame_id, level, languages, None, true)
        .await
}

#[tauri::command]
pub async fn accessibility_snapshot(
    core: State<'_, AppCore>,
    max_depth: Option<u32>,
    max_elements: Option<u32>,
) -> BlueyResult<AccessibilityContext> {
    core.ax.snapshot(max_depth, max_elements).await
}

#[tauri::command]
pub async fn accessibility_frontmost_app(core: State<'_, AppCore>) -> BlueyResult<FrontmostApp> {
    core.ax.frontmost().await
}
