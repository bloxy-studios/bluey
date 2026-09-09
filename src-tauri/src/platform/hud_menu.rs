//! Rust-owned HUD popups. No JS menu resources/channels, menu event subscriptions,
//! responder-chain actions, application activation, or modifications to Tauri/muda.

use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::hud_menu::HudMenuRequest;
use tauri::WebviewWindow;

#[cfg(target_os = "macos")]
#[path = "hud_menu_macos.rs"]
mod macos;

pub async fn popup(window: WebviewWindow, request: HudMenuRequest) -> BlueyResult<Option<String>> {
    request.validate(window.label())?;

    #[cfg(target_os = "macos")]
    {
        use bluey_protocols::hud_menu::PopupGate;
        static GATE: PopupGate = PopupGate::new();
        // Suppress, don't queue: a queued second popup would use a stale anchor/action snapshot.
        let Some(permit) = GATE.try_acquire() else {
            return Ok(None);
        };
        let (send, receive) = tokio::sync::oneshot::channel();
        let owner = window.clone();
        window
            .run_on_main_thread(move || {
                // Every AppKit object is created, used and dropped in this scope. Retain the
                // invoking window until tracking ends, even if its JS/IPC receiver disappears.
                let result = objc2::rc::autoreleasepool(|_| macos::popup(&owner, &request));
                drop(permit);
                let _ = send.send(result);
            })
            .map_err(|_| BlueyError::internal("Cannot schedule the native HUD menu"))?;
        receive
            .await
            .map_err(|_| BlueyError::internal("Native HUD menu task ended without a result"))?
    }
    #[cfg(not(target_os = "macos"))]
    Err(BlueyError::new(
        bluey_core::BlueyErrorKind::NotSupported,
        "hud_menu.platform",
        "Native HUD menus require macOS",
    ))
}
