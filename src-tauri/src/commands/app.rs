//! `app_*` commands: state machine snapshot and transitions, developer info,
//! setup checks, quit.

use bluey_core::types::{AppEvent, AppStatus, DevInfo, SetupCheck};
use bluey_core::BlueyResult;
use tauri::{AppHandle, State};

use crate::state::AppCore;

#[tauri::command]
pub fn app_get_status(core: State<'_, AppCore>) -> BlueyResult<AppStatus> {
    Ok(core.hub.status())
}

/// Pause: the pipeline stops and audio capture is suspended (the machine keeps
/// `audioActive` so resuming returns to listening).
#[tauri::command]
pub async fn app_pause(core: State<'_, AppCore>) -> BlueyResult<AppStatus> {
    let status = core.hub.transition(AppEvent::Paused)?;
    if core.audio.is_running() {
        if let Err(e) = core.audio.pause().await {
            tracing::warn!(error = %e, "could not pause audio while pausing the app");
        }
    }
    Ok(status)
}

#[tauri::command]
pub async fn app_resume(core: State<'_, AppCore>) -> BlueyResult<AppStatus> {
    let status = core.hub.transition(AppEvent::Resumed)?;
    if core.audio.is_running() {
        if let Err(e) = core.audio.resume().await {
            tracing::warn!(error = %e, "could not resume audio while resuming the app");
        }
    }
    Ok(status)
}

#[tauri::command]
pub fn app_recover(core: State<'_, AppCore>) -> BlueyResult<AppStatus> {
    core.hub.transition(AppEvent::Recovered)
}

/// Dismiss the response (or cancel a busy pipeline). Never fails: when the
/// machine is already idle the current status is returned unchanged.
#[tauri::command]
pub fn app_dismiss_response(core: State<'_, AppCore>) -> BlueyResult<AppStatus> {
    core.ai.cancel_all();
    Ok(core
        .hub
        .transition_soft(AppEvent::ResponseDismissed)
        .unwrap_or_else(|| core.hub.status()))
}

#[tauri::command]
pub fn app_get_dev_info(core: State<'_, AppCore>, app: AppHandle) -> BlueyResult<DevInfo> {
    Ok(crate::app::checks::dev_info(&core, &app))
}

#[tauri::command]
pub async fn app_run_setup_checks(core: State<'_, AppCore>) -> BlueyResult<Vec<SetupCheck>> {
    Ok(crate::app::checks::setup_checks(&core).await)
}

#[tauri::command]
pub async fn app_quit(app: AppHandle) -> BlueyResult<()> {
    crate::app::shutdown(&app).await;
    app.exit(0);
    Ok(())
}
