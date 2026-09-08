//! `context_build_snapshot` — the ⌘↵ fast path.

use bluey_core::types::{ContextSnapshot, SnapshotOptions};
use bluey_core::BlueyResult;
use tauri::State;

use crate::state::AppCore;

#[tauri::command]
pub async fn context_build_snapshot(
    core: State<'_, AppCore>,
    options: SnapshotOptions,
) -> BlueyResult<ContextSnapshot> {
    crate::context::build_snapshot(&core, options).await
}
