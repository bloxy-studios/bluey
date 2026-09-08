//! `data_*` commands: usage statistics, targeted deletion and the full reset.
//! Deletion really deletes (rows + files), then the database is vacuumed.

use bluey_core::events::BlueyEvent;
use bluey_core::types::DEFAULT_MODE_ID;
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_storage::{ModeRepository, UsageStats};
use tauri::State;

use crate::secrets::{provider_key, AGENT_ANTHROPIC_KEY, CLERK_TOKEN_KEY, EXA_KEY, FIRECRAWL_KEY};
use crate::state::AppCore;

#[tauri::command]
pub async fn data_usage_stats(core: State<'_, AppCore>) -> BlueyResult<UsageStats> {
    let cache_dir = core.paths.frames_dir.clone();
    core.storage
        .run(move |db| bluey_storage::usage_stats(db, &cache_dir))
        .await
}

#[tauri::command]
pub async fn data_delete_screenshots(core: State<'_, AppCore>) -> BlueyResult<u64> {
    let (deleted, paths) = core.storage.run(bluey_storage::delete_screenshots).await?;
    crate::platform::remove_files(&paths);
    vacuum(&core).await;
    Ok(deleted)
}

#[tauri::command]
pub async fn data_clear_transcripts(core: State<'_, AppCore>) -> BlueyResult<u64> {
    let removed = core.audio.clear(None).await?;
    vacuum(&core).await;
    Ok(removed)
}

#[tauri::command]
pub async fn data_clear_ai_cache(core: State<'_, AppCore>) -> BlueyResult<u64> {
    core.storage.run(bluey_storage::clear_ai_cache).await
}

/// Wipe everything: sessions and their files, documents, responses, settings,
/// shortcuts, provider configs, Keychain entries owned by Bluey and the Clerk
/// session. Built-in modes are re-seeded so the app keeps working.
#[tauri::command]
pub async fn data_reset_all(core: State<'_, AppCore>) -> BlueyResult<()> {
    if core.audio.is_running() {
        let _ = core.audio.stop().await;
    }
    core.ai.cancel_all();
    core.sessions.delete_all().await?;

    // Keychain entries first (while the provider list is still known).
    let providers = core.settings.get().ai.providers;
    for provider in &providers {
        core.secrets.delete(&provider_key(&provider.id)).await?;
    }
    for key in [EXA_KEY, FIRECRAWL_KEY, AGENT_ANTHROPIC_KEY, CLERK_TOKEN_KEY] {
        core.secrets.delete(key).await?;
    }

    let paths = core.storage.run(bluey_storage::reset_all).await?;
    crate::platform::remove_files(&paths);

    // Re-seed built-in modes and restore defaults in memory + on disk.
    core.storage
        .run(|db| {
            let modes = bluey_core::modes::built_in_modes(&now_iso());
            ModeRepository::seed_built_in(db, &modes)
        })
        .await?;
    let (old, new) = core.settings.reset().await?;
    crate::settings::side_effects::apply(&core, &old, &new).await;
    core.modes.set_active(DEFAULT_MODE_ID.to_string()).await?;
    let modes = core.modes.list().await?;
    core.bus.publish(BlueyEvent::ModesChanged(modes));
    core.auth.clear_session().await?;
    vacuum(&core).await;
    Ok(())
}

#[tauri::command]
pub async fn data_export_session(
    core: State<'_, AppCore>,
    session_id: String,
    format: String,
) -> BlueyResult<String> {
    if format != "markdown" && format != "json" {
        return Err(BlueyError::invalid_params(format!(
            "unknown export format `{format}`"
        )));
    }
    core.sessions.export(session_id, &format).await
}

async fn vacuum(core: &AppCore) {
    if let Err(e) = core.storage.run(|db| db.vacuum()).await {
        tracing::warn!(error = %e, "vacuum after deletion failed");
    }
}
