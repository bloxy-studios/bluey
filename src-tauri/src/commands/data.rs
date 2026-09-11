//! `data_*` commands: usage statistics, targeted deletion and the full reset.
//! Deletion really deletes (rows + files), then the database is vacuumed.

use bluey_core::events::BlueyEvent;
use bluey_core::types::DEFAULT_MODE_ID;
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_storage::{ModeRepository, UsageStats};
use tauri::State;

use crate::secrets::{
    provider_key, AGENT_ANTHROPIC_KEY, CLERK_OAUTH_TOKENS_KEY, CLERK_TOKEN_KEY, EXA_KEY,
    FIRECRAWL_KEY,
};
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
///
/// Every step runs even when an earlier one fails — one stuck Keychain entry
/// must not leave sessions, documents or settings in place — and the failures
/// are reported together at the end (`reset_incomplete`, with the steps in
/// `details`).
#[tauri::command]
pub async fn data_reset_all(core: State<'_, AppCore>) -> BlueyResult<()> {
    if core.audio.is_running() {
        let _ = core.audio.stop().await;
    }
    core.ai.cancel_all();
    let mut failures = ResetFailures::default();

    failures.note("sessions", core.sessions.delete_all().await.map(|_| ()));

    // Keychain entries first (while the provider list is still known).
    let providers = core.settings.get().ai.providers;
    for provider in &providers {
        failures.note(
            &format!("provider key {}", provider.id),
            core.secrets.delete(&provider_key(&provider.id)).await,
        );
    }
    for key in [
        EXA_KEY,
        FIRECRAWL_KEY,
        AGENT_ANTHROPIC_KEY,
        CLERK_TOKEN_KEY,
        CLERK_OAUTH_TOKENS_KEY,
    ] {
        failures.note(key, core.secrets.delete(key).await);
    }

    match core.storage.run(bluey_storage::reset_all).await {
        Ok(paths) => crate::platform::remove_files(&paths),
        Err(error) => failures.push("database", error),
    }

    // Re-seed built-in modes and restore defaults in memory + on disk.
    failures.note(
        "built-in modes",
        core.storage
            .run(|db| {
                let modes = bluey_core::modes::built_in_modes(&now_iso());
                ModeRepository::seed_built_in(db, &modes)
            })
            .await
            .map(|_| ()),
    );
    match core.settings.reset().await {
        Ok((old, new)) => crate::settings::side_effects::apply(&core, &old, &new).await,
        Err(error) => failures.push("settings", error),
    }
    failures.note(
        "active mode",
        core.modes
            .set_active(DEFAULT_MODE_ID.to_string())
            .await
            .map(|_| ()),
    );
    match core.modes.list().await {
        Ok(modes) => core.bus.publish(BlueyEvent::ModesChanged(modes)),
        Err(error) => failures.push("modes", error),
    }
    failures.note("sign-in", core.auth.clear_session().await.map(|_| ()));
    vacuum(&core).await;
    failures.into_result()
}

/// Failures collected while resetting, reported together so the user learns
/// what did not go while the reset still removes everything it can.
#[derive(Default)]
struct ResetFailures(Vec<(String, BlueyError)>);

impl ResetFailures {
    fn note<T>(&mut self, step: &str, result: BlueyResult<T>) {
        if let Err(error) = result {
            self.push(step, error);
        }
    }

    fn push(&mut self, step: &str, error: BlueyError) {
        tracing::warn!(step, code = %error.code, "reset step failed; continuing");
        self.0.push((step.to_string(), error));
    }

    fn into_result(self) -> BlueyResult<()> {
        if self.0.is_empty() {
            return Ok(());
        }
        let steps: Vec<&str> = self.0.iter().map(|(step, _)| step.as_str()).collect();
        let details: Vec<serde_json::Value> = self
            .0
            .iter()
            .map(|(step, error)| serde_json::json!({ "step": step, "code": error.code }))
            .collect();
        Err(BlueyError::storage(
            "reset_incomplete",
            format!(
                "Bluey was reset, but {} step(s) failed: {}",
                self.0.len(),
                steps.join(", ")
            ),
        )
        .with_details(serde_json::Value::Array(details)))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reset_without_failures_is_ok() {
        let mut failures = ResetFailures::default();
        failures.note("sessions", Ok::<u64, BlueyError>(3));
        assert!(failures.into_result().is_ok());
    }

    #[test]
    fn failures_are_collected_and_reported_together() {
        let mut failures = ResetFailures::default();
        failures.note("sessions", Ok::<(), BlueyError>(()));
        failures.note(
            "provider key gemini",
            Err::<(), _>(BlueyError::storage(
                "keychain",
                "failed to delete from the keychain",
            )),
        );
        failures.push("database", BlueyError::internal("locked"));
        let error = failures.into_result().unwrap_err();
        assert_eq!(error.code, "reset_incomplete");
        assert!(error.message.contains("2 step(s) failed"));
        assert!(error.message.contains("provider key gemini, database"));
        let details = error.details.expect("steps are listed in details");
        assert_eq!(details[0]["step"], "provider key gemini");
        assert_eq!(details[0]["code"], "keychain");
        assert_eq!(details[1]["step"], "database");
    }
}
