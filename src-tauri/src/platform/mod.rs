//! macOS platform glue that is not a manager of its own: the menu bar item,
//! launch-at-login, opening/closing the secondary windows with a route, and
//! the data-management helpers behind the `data_*` commands.

use std::path::PathBuf;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{AppState, AppStatus};
use bluey_core::{BlueyError, BlueyResult};
use tauri::menu::{Menu, MenuBuilder, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt as _;

use crate::state::AppCore;

/// Window labels the frontend may open/close.
pub const WINDOW_LABELS: &[&str] = &["main", "settings", "onboarding"];
const TRAY_ID: &str = "bluey-tray";
const MENU_TOGGLE_LISTENING: &str = "toggle_listening";
const MENU_TOGGLE_PANEL: &str = "toggle_panel";
const MENU_PRIVACY: &str = "toggle_privacy";
const MENU_SESSIONS: &str = "view_sessions";
const MENU_PREFERENCES: &str = "preferences";
const MENU_QUIT: &str = "quit";

/// Enable/disable launch at login (best effort, logged).
pub fn set_autostart(app: &AppHandle, enabled: bool) {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(e) = result {
        tracing::warn!(enabled, error = %e, "cannot change launch-at-login");
    }
}

/// Show + focus a window. For `settings`/`onboarding` an optional `route`
/// (settings tab id) is delivered as the `?tab=` query parameter; the HUD is
/// shown through the panel manager.
pub fn open_window(app: &AppHandle, label: &str, route: Option<&str>) -> BlueyResult<()> {
    if !WINDOW_LABELS.contains(&label) {
        return Err(BlueyError::invalid_params(format!(
            "unknown window `{label}`"
        )));
    }
    if label == "main" {
        let core = app.state::<AppCore>();
        let panel = core.panel.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = panel.show().await {
                tracing::warn!(error = %e, "cannot show the HUD");
            }
        });
        return Ok(());
    }
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| BlueyError::internal(format!("window `{label}` does not exist")))?;
    if let Some(route) = route {
        if let Ok(mut url) = window.url() {
            let mut pairs: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(key, _)| key != "tab")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            pairs.push(("tab".to_string(), route.to_string()));
            url.query_pairs_mut().clear().extend_pairs(pairs);
            if let Err(e) = window.navigate(url) {
                tracing::warn!(error = %e, "cannot navigate window to route");
            }
        }
    }
    window
        .show()
        .and_then(|_| window.set_focus())
        .map_err(|e| BlueyError::internal(format!("cannot show window `{label}`: {e}")))
}

/// Hide a window (windows are pre-created and never destroyed).
pub fn close_window(app: &AppHandle, label: &str) -> BlueyResult<()> {
    if !WINDOW_LABELS.contains(&label) {
        return Err(BlueyError::invalid_params(format!(
            "unknown window `{label}`"
        )));
    }
    if label == "main" {
        let core = app.state::<AppCore>();
        let panel = core.panel.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = panel.hide().await {
                tracing::warn!(error = %e, "cannot hide the HUD");
            }
        });
        return Ok(());
    }
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| BlueyError::internal(format!("window `{label}` does not exist")))?;
    window
        .hide()
        .map_err(|e| BlueyError::internal(format!("cannot hide window `{label}`: {e}")))
}

/// Best-effort deletion of files reported by retention calls.
pub fn remove_files(paths: &[PathBuf]) {
    crate::storage::Storage::remove_files(paths);
}

/// Menu-bar label for the listening item.
pub fn listening_label(status: &AppStatus) -> &'static str {
    if status.audio_active {
        "Stop Listening"
    } else {
        "Start Listening"
    }
}

/// One-line state text for the tray tooltip.
pub fn state_tooltip(status: &AppStatus) -> String {
    let state = match status.state {
        AppState::Booting => "Starting",
        AppState::AuthRequired => "Sign in required",
        AppState::Ready => "Ready",
        AppState::Listening => "Listening",
        AppState::Capturing => "Reading screen",
        AppState::Analyzing => "Analyzing",
        AppState::Thinking => "Thinking",
        AppState::ResponseReady => "Response ready",
        AppState::Error => "Needs attention",
        AppState::Paused => "Paused",
    };
    format!("Bluey — {state}")
}

/// Build the menu bar item and keep its labels in sync with `app.state`.
pub fn build_tray(app: &AppHandle) -> BlueyResult<()> {
    let listen = MenuItem::with_id(
        app,
        MENU_TOGGLE_LISTENING,
        "Start Listening",
        true,
        None::<&str>,
    )
    .map_err(tray_err)?;
    let toggle = MenuItem::with_id(
        app,
        MENU_TOGGLE_PANEL,
        "Show / Hide Bluey",
        true,
        None::<&str>,
    )
    .map_err(tray_err)?;
    let privacy = MenuItem::with_id(app, MENU_PRIVACY, "Toggle Privacy Mode", true, None::<&str>)
        .map_err(tray_err)?;
    let sessions = MenuItem::with_id(app, MENU_SESSIONS, "View Sessions", true, None::<&str>)
        .map_err(tray_err)?;
    let preferences = MenuItem::with_id(
        app,
        MENU_PREFERENCES,
        "Preferences…",
        true,
        Some("CmdOrCtrl+,"),
    )
    .map_err(tray_err)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit Bluey", true, Some("CmdOrCtrl+Q"))
        .map_err(tray_err)?;
    let menu: Menu<Wry> = MenuBuilder::new(app)
        .item(&listen)
        .item(&toggle)
        .separator()
        .item(&privacy)
        .item(&sessions)
        .item(&preferences)
        .separator()
        .item(&quit)
        .build()
        .map_err(tray_err)?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Bluey — Starting")
        .on_menu_event(|app, event| on_menu_event(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon).icon_as_template(true);
    }
    let tray = builder.build(app).map_err(tray_err)?;

    // Keep the listening label and tooltip current.
    let core = app.state::<AppCore>();
    let mut rx = core.bus.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(BlueyEvent::AppState(status)) => {
                    let _ = listen.set_text(listening_label(&status));
                    let _ = tray.set_tooltip(Some(state_tooltip(&status)));
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    Ok(())
}

fn on_menu_event(app: &AppHandle, id: &str) {
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
        let core = app.state::<AppCore>();
        let result: BlueyResult<()> = match id.as_str() {
            MENU_TOGGLE_LISTENING => {
                if core.audio.is_running() {
                    core.audio.stop().await.map(|_| ())
                } else {
                    core.audio.start(None).await.map(|_| ())
                }
            }
            MENU_TOGGLE_PANEL => core.panel.toggle().await.map(|_| ()),
            MENU_PRIVACY => {
                let enabled = !core.capture.protection().enabled;
                let mode = if enabled { "privacy" } else { "standard" };
                let patch = serde_json::json!({ "privacy": { "displayMode": mode } });
                match core.settings.update(patch).await {
                    Ok((old, new)) => {
                        crate::settings::side_effects::apply(&core, &old, &new).await;
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            MENU_SESSIONS => open_window(&app, "settings", Some("sessions")),
            MENU_PREFERENCES => open_window(&app, "settings", None),
            MENU_QUIT => {
                crate::app::shutdown(&app).await;
                app.exit(0);
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!(item = %id, error = %e, "menu action failed");
        }
    });
}

fn tray_err(e: tauri::Error) -> BlueyError {
    BlueyError::internal(format!("menu bar setup failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::now_iso;

    fn status(state: AppState, audio_active: bool) -> AppStatus {
        AppStatus {
            state,
            audio_active,
            session_id: None,
            mode_id: "general".into(),
            error: None,
            resume_state: None,
            updated_at: now_iso(),
        }
    }

    #[test]
    fn labels_follow_the_state_machine() {
        assert_eq!(
            listening_label(&status(AppState::Ready, false)),
            "Start Listening"
        );
        assert_eq!(
            listening_label(&status(AppState::Listening, true)),
            "Stop Listening"
        );
        assert_eq!(
            state_tooltip(&status(AppState::Thinking, true)),
            "Bluey — Thinking"
        );
        assert_eq!(
            state_tooltip(&status(AppState::Error, false)),
            "Bluey — Needs attention"
        );
    }
}
