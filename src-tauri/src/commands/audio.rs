//! `audio_*` and `transcript_*` commands.

use bluey_core::types::{AudioDevice, AudioSessionConfigPatch, AudioStatus, TranscriptSegment};
use bluey_core::BlueyResult;
use tauri::State;

use crate::audio::MicrophoneTest;
use crate::state::AppCore;

#[tauri::command]
pub async fn audio_list_devices(core: State<'_, AppCore>) -> BlueyResult<Vec<AudioDevice>> {
    core.audio.list_devices().await
}

#[tauri::command]
pub async fn audio_start(
    core: State<'_, AppCore>,
    config: Option<AudioSessionConfigPatch>,
) -> BlueyResult<AudioStatus> {
    core.audio.start(config).await
}

#[tauri::command]
pub async fn audio_stop(core: State<'_, AppCore>) -> BlueyResult<AudioStatus> {
    core.audio.stop().await
}

#[tauri::command]
pub async fn audio_pause(core: State<'_, AppCore>) -> BlueyResult<AudioStatus> {
    core.audio.pause().await
}

#[tauri::command]
pub async fn audio_resume(core: State<'_, AppCore>) -> BlueyResult<AudioStatus> {
    core.audio.resume().await
}

#[tauri::command]
pub fn audio_get_status(core: State<'_, AppCore>) -> BlueyResult<AudioStatus> {
    Ok(core.audio.status())
}

#[tauri::command]
pub async fn audio_test_microphone(
    core: State<'_, AppCore>,
    device_id: Option<String>,
    duration_ms: Option<u32>,
) -> BlueyResult<MicrophoneTest> {
    core.audio.test_microphone(device_id, duration_ms).await
}

#[tauri::command]
pub async fn transcript_list(
    core: State<'_, AppCore>,
    session_id: Option<String>,
    since_ms: Option<u64>,
    limit: Option<u32>,
) -> BlueyResult<Vec<TranscriptSegment>> {
    core.audio.list(session_id, since_ms, limit).await
}

#[tauri::command]
pub fn transcript_recent(
    core: State<'_, AppCore>,
    window_seconds: u32,
) -> BlueyResult<Vec<TranscriptSegment>> {
    Ok(core.audio.recent(window_seconds))
}

#[tauri::command]
pub async fn transcript_clear(
    core: State<'_, AppCore>,
    session_id: Option<String>,
) -> BlueyResult<()> {
    core.audio.clear(session_id).await.map(|_| ())
}
