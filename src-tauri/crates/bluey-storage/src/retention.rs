//! Deletion & retention policies. Deletion must **really** delete: every
//! function here removes rows (and reports file paths for the caller to unlink)
//! rather than soft-deleting.

use std::path::{Path, PathBuf};

use bluey_core::error::BlueyError;
use bluey_core::types::settings::PrivacySettings;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::SqlExt;
use crate::repositories::{
    AiCacheRepository, SessionRepository, SnapshotRepository, TranscriptRepository,
};

/// Mirrors `DataUsageStats` in `src/lib/tauri/commands.ts` (camelCase on the wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    pub sessions: u64,
    pub responses: u64,
    pub transcript_segments: u64,
    pub screenshots: u64,
    pub documents: u64,
    pub db_size_bytes: u64,
    pub screenshot_cache_bytes: u64,
}

/// What [`apply_retention`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RetentionReport {
    pub screenshots_deleted: u64,
    pub transcripts_deleted: u64,
    pub sessions_deleted: u64,
    /// Screenshot files the caller must remove from disk.
    pub image_paths: Vec<PathBuf>,
}

/// Row counts, database size and on-disk screenshot cache size.
pub fn usage_stats(db: &Database, cache_dir: &Path) -> Result<UsageStats, BlueyError> {
    let (sessions, responses, transcript_segments, screenshots, documents) =
        db.with_conn(|conn| {
            let count = |table: &str| -> Result<u64, BlueyError> {
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .map(|n| n.max(0) as u64)
                .sql()
            };
            Ok((
                count("sessions")?,
                count("ai_responses")?,
                count("transcript_segments")?,
                count("screen_snapshots")?,
                count("documents")?,
            ))
        })?;
    Ok(UsageStats {
        sessions,
        responses,
        transcript_segments,
        screenshots,
        documents,
        db_size_bytes: db.db_size_bytes()?,
        screenshot_cache_bytes: dir_size_bytes(cache_dir),
    })
}

/// Recursive best-effort directory size (missing dir or IO errors count as 0).
fn dir_size_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

/// Delete one session (cascade). Returns screenshot image paths to unlink.
pub fn delete_session(db: &Database, session_id: &str) -> Result<Vec<PathBuf>, BlueyError> {
    SessionRepository::delete(db, session_id)
}

/// Delete every session. Returns `(deleted_sessions, image_paths)`.
pub fn delete_all_sessions(db: &Database) -> Result<(u64, Vec<PathBuf>), BlueyError> {
    SessionRepository::delete_all(db)
}

/// Delete every screen snapshot row. Returns `(deleted_rows, image_paths)` —
/// the rows are gone; the caller unlinks the files.
pub fn delete_screenshots(db: &Database) -> Result<(u64, Vec<PathBuf>), BlueyError> {
    SnapshotRepository::delete_all_screens(db)
}

/// Delete every transcript segment. Returns rows removed.
pub fn clear_transcripts(db: &Database) -> Result<u64, BlueyError> {
    TranscriptRepository::clear(db, None)
}

/// Delete every AI cache entry. Returns rows removed.
pub fn clear_ai_cache(db: &Database) -> Result<u64, BlueyError> {
    AiCacheRepository::clear(db)
}

/// Wipe **every** row in every table except `schema_migrations`. Nothing is
/// re-seeded here — the app re-seeds built-in modes on next start. Returns the
/// screenshot image paths that were referenced so the caller can unlink them.
pub fn reset_all(db: &Database) -> Result<Vec<PathBuf>, BlueyError> {
    db.transaction(|conn| {
        let mut stmt = conn
            .prepare("SELECT image_path FROM screen_snapshots WHERE image_path IS NOT NULL")
            .sql()?;
        let paths = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .sql()?
            .collect::<Result<Vec<String>, _>>()
            .sql()?;
        // Order matters for foreign keys: session children go with sessions,
        // sessions must go before modes (mode delete re-points sessions at
        // 'general'), documents before modes is not required but harmless.
        for table in [
            "sessions",
            "transcript_segments",
            "screen_snapshots",
            "accessibility_snapshots",
            "speakers",
            "ai_requests",
            "response_feedback",
            "ai_responses",
            "session_events",
            "session_notes",
            "session_summaries",
            "documents",
            "document_chunks",
            "mode_documents",
            "modes",
            "users",
            "settings",
            "shortcuts",
            "model_configs",
            "ai_cache",
            // FTS tables are already emptied by triggers; belt and braces:
            "transcript_fts",
            "responses_fts",
            "document_chunks_fts",
        ] {
            conn.execute(&format!("DELETE FROM {table}"), []).sql()?;
        }
        Ok(paths.into_iter().map(PathBuf::from).collect())
    })
}

/// Enforce the privacy settings on already-stored data:
/// * `store_screenshots = false` → delete all screen snapshot rows (and report
///   their image paths for unlinking);
/// * `store_transcripts = false` → delete all transcript segments;
/// * `store_session_history = false` → delete completed sessions (active /
///   paused sessions survive).
pub fn apply_retention(
    db: &Database,
    settings: &PrivacySettings,
) -> Result<RetentionReport, BlueyError> {
    let mut report = RetentionReport::default();
    if !settings.store_session_history {
        let (deleted, mut paths) = prune_completed_sessions(db)?;
        report.sessions_deleted = deleted;
        report.image_paths.append(&mut paths);
    }
    if !settings.store_screenshots {
        let (deleted, mut paths) = SnapshotRepository::delete_all_screens(db)?;
        report.screenshots_deleted = deleted;
        report.image_paths.append(&mut paths);
    }
    if !settings.store_transcripts {
        report.transcripts_deleted = TranscriptRepository::clear(db, None)?;
    }
    report.image_paths.sort();
    report.image_paths.dedup();
    Ok(report)
}

/// When session history is disabled, completed sessions (and their cascaded
/// children) are deleted as soon as they finish; active/paused sessions stay.
/// Returns the number of sessions deleted.
pub fn prune_finished_sessions_without_history(
    db: &Database,
    store_session_history: bool,
) -> Result<u64, BlueyError> {
    if store_session_history {
        return Ok(0);
    }
    Ok(prune_completed_sessions(db)?.0)
}

fn prune_completed_sessions(db: &Database) -> Result<(u64, Vec<PathBuf>), BlueyError> {
    db.transaction(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT sn.image_path FROM screen_snapshots sn
                   JOIN sessions s ON s.id = sn.session_id
                  WHERE s.status = 'completed' AND sn.image_path IS NOT NULL",
            )
            .sql()?;
        let paths = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .sql()?
            .collect::<Result<Vec<String>, _>>()
            .sql()?;
        let deleted = conn
            .execute("DELETE FROM sessions WHERE status = 'completed'", [])
            .sql()?;
        Ok((
            deleted as u64,
            paths.into_iter().map(PathBuf::from).collect(),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::{
        ModeRepository, ResponseRepository, SessionRepository, SettingsRepository,
        TranscriptRepository,
    };
    use crate::testutil;
    use bluey_core::types::documents::{DocumentKind, DocumentScope};
    use bluey_core::types::session::SessionStatus;
    use pretty_assertions::assert_eq;

    fn seed_everything(db: &Database) -> String {
        let s = SessionRepository::create(db, "general", Some("S".into())).unwrap();
        TranscriptRepository::insert(db, &testutil::segment(Some(&s.id), "hello there", 0, true))
            .unwrap();
        ResponseRepository::save(db, &testutil::response(Some(&s.id), "an answer")).unwrap();
        testutil::insert_screen_snapshot(db, Some(&s.id), Some("/tmp/x.jpg"));
        crate::documents::index::add_document(
            db,
            &testutil::doc_input(
                DocumentKind::Resume,
                DocumentScope::Global,
                None,
                "resume text",
            ),
            |_| unreachable!(),
        )
        .unwrap();
        crate::repositories::AiCacheRepository::set(db, "k", "v", None).unwrap();
        SettingsRepository::set_active_mode_id(db, "general").unwrap();
        s.id
    }

    #[test]
    fn usage_stats_counts_and_cache_dir() {
        let db = testutil::db();
        seed_everything(&db);
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a.jpg"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.path().join("sub/b.jpg"), vec![0u8; 50]).unwrap();

        let stats = usage_stats(&db, dir.path()).unwrap();
        assert_eq!(stats.sessions, 1);
        assert_eq!(stats.responses, 1);
        assert_eq!(stats.transcript_segments, 1);
        assert_eq!(stats.screenshots, 1);
        assert_eq!(stats.documents, 1);
        assert!(stats.db_size_bytes > 0);
        assert_eq!(stats.screenshot_cache_bytes, 150);

        // Missing dir is 0, and the wire shape matches commands.ts (camelCase).
        assert_eq!(
            usage_stats(&db, Path::new("/definitely/missing"))
                .unwrap()
                .screenshot_cache_bytes,
            0
        );
        let json = serde_json::to_value(&stats).unwrap();
        for key in [
            "sessions",
            "responses",
            "transcriptSegments",
            "screenshots",
            "documents",
            "dbSizeBytes",
            "screenshotCacheBytes",
        ] {
            assert!(json.get(key).is_some(), "missing key {key}");
        }
    }

    #[test]
    fn targeted_deletions_really_delete() {
        let db = testutil::db();
        let sid = seed_everything(&db);

        let (rows, paths) = delete_screenshots(&db).unwrap();
        assert_eq!((rows, paths.len()), (1, 1));
        assert_eq!(testutil::count(&db, "screen_snapshots"), 0);

        assert_eq!(clear_transcripts(&db).unwrap(), 1);
        assert_eq!(testutil::count(&db, "transcript_segments"), 0);
        assert_eq!(testutil::count(&db, "transcript_fts"), 0);

        assert_eq!(clear_ai_cache(&db).unwrap(), 1);
        assert_eq!(testutil::count(&db, "ai_cache"), 0);

        let paths = delete_session(&db, &sid).unwrap();
        assert!(paths.is_empty(), "screenshots were already gone");
        assert_eq!(testutil::count(&db, "sessions"), 0);
        assert_eq!(testutil::count(&db, "ai_responses"), 0);
        assert_eq!(testutil::count(&db, "responses_fts"), 0);
    }

    #[test]
    fn reset_all_empties_every_table_and_keeps_migrations() {
        let db = testutil::db();
        seed_everything(&db);
        let paths = reset_all(&db).unwrap();
        assert_eq!(paths, vec![PathBuf::from("/tmp/x.jpg")]);
        for table in [
            "sessions",
            "modes",
            "documents",
            "document_chunks",
            "document_chunks_fts",
            "transcript_segments",
            "transcript_fts",
            "ai_responses",
            "responses_fts",
            "screen_snapshots",
            "settings",
            "ai_cache",
        ] {
            assert_eq!(testutil::count(&db, table), 0, "{table} must be empty");
        }
        assert!(
            testutil::count(&db, "schema_migrations") >= 2,
            "migrations are kept"
        );
        // The database still works afterwards (app re-seeds modes).
        ModeRepository::seed_built_in(&db, &[testutil::mode("general", "General")]).unwrap();
        SessionRepository::create(&db, "general", None).unwrap();
    }

    #[test]
    fn apply_retention_enforces_privacy_settings() {
        let db = testutil::db();
        let sid = seed_everything(&db);
        SessionRepository::set_status(
            &db,
            &sid,
            SessionStatus::Completed,
            Some(bluey_core::now_iso()),
        )
        .unwrap();
        let active = SessionRepository::create(&db, "general", Some("live".into())).unwrap();

        let settings = PrivacySettings {
            store_session_history: false,
            store_screenshots: false,
            store_transcripts: false,
            ..PrivacySettings::default()
        };
        let report = apply_retention(&db, &settings).unwrap();
        assert_eq!(report.sessions_deleted, 1, "completed session pruned");
        assert_eq!(
            report.transcripts_deleted, 0,
            "transcripts cascaded with the session already"
        );
        assert_eq!(report.image_paths, vec![PathBuf::from("/tmp/x.jpg")]);
        assert_eq!(testutil::count(&db, "screen_snapshots"), 0);
        assert_eq!(testutil::count(&db, "transcript_segments"), 0);
        // The active session survives.
        assert_eq!(
            SessionRepository::get(&db, &active.id).unwrap().id,
            active.id
        );
        // Documents are user context, not session data — they stay.
        assert_eq!(testutil::count(&db, "documents"), 1);

        // Defaults (store everything) are a no-op.
        let report = apply_retention(&db, &PrivacySettings::default()).unwrap();
        assert_eq!(report, RetentionReport::default());

        assert_eq!(
            prune_finished_sessions_without_history(&db, true).unwrap(),
            0
        );
        SessionRepository::set_status(&db, &active.id, SessionStatus::Completed, None).unwrap();
        assert_eq!(
            prune_finished_sessions_without_history(&db, false).unwrap(),
            1
        );
        assert_eq!(testutil::count(&db, "sessions"), 0);
    }
}
