//! A display-only popup, bound to the invoking HUD webview (no caller-chosen window).

use bluey_core::BlueyResult;
use bluey_protocols::hud_menu::HudMenuRequest;
use tauri::WebviewWindow;

#[tauri::command]
pub async fn hud_menu_popup(
    window: WebviewWindow,
    request: HudMenuRequest,
) -> BlueyResult<Option<String>> {
    crate::platform::hud_menu::popup(window, request).await
}
