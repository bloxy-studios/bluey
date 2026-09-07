//! Screen + accessibility snapshots. Raw images live on disk (the app's cache
//! dir); this table only stores the path (and only when the privacy setting
//! allows), OCR output and app/window context.

use std::path::PathBuf;

use bluey_core::error::BlueyError;
use bluey_core::new_id;
use bluey_core::types::context::{
    AccessibilityContext, ApplicationContext, OcrContext, ScreenFrame, WindowContext,
};
use rusqlite::params;

use super::{to_enum_str, to_json_string};
use crate::db::Database;
use crate::error::SqlExt;

/// Persistence for `screen_snapshots` and `accessibility_snapshots`.
pub struct SnapshotRepository;

impl SnapshotRepository {
    /// Store a screen snapshot row for `frame`. The image path is only recorded
    /// when `store_image` is true (privacy: `storeScreenshots`). Returns the
    /// snapshot id.
    pub fn save_screen(
        db: &Database,
        session_id: Option<&str>,
        frame: &ScreenFrame,
        ocr: Option<&OcrContext>,
        active_app: Option<&ApplicationContext>,
        active_window: Option<&WindowContext>,
        store_image: bool,
    ) -> Result<String, BlueyError> {
        let id = new_id("snap");
        let mime = to_enum_str(&frame.mime_type)?;
        let image_path = if store_image {
            frame.path.clone()
        } else {
            None
        };
        let ocr_text = ocr.map(|o| o.text.clone());
        let ocr_json = ocr.map(to_json_string).transpose()?;
        let app_json = active_app.map(to_json_string).transpose()?;
        let window_json = active_window.map(to_json_string).transpose()?;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO screen_snapshots (id, session_id, display_id, width, height,
                    mime_type, image_path, hash, ocr_text, ocr_json, active_app, active_window, captured_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    id,
                    session_id,
                    frame.display_id,
                    frame.width,
                    frame.height,
                    mime,
                    image_path,
                    frame.hash,
                    ocr_text,
                    ocr_json,
                    app_json,
                    window_json,
                    frame.captured_at
                ],
            )
            .sql()?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Every stored screenshot image path (for cache accounting / cleanup).
    pub fn list_screen_paths(db: &Database) -> Result<Vec<PathBuf>, BlueyError> {
        db.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT image_path FROM screen_snapshots WHERE image_path IS NOT NULL")
                .sql()?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).sql()?;
            Ok(rows
                .collect::<Result<Vec<_>, _>>()
                .sql()?
                .into_iter()
                .map(PathBuf::from)
                .collect())
        })
    }

    /// Delete every screen snapshot row. Returns `(deleted_rows, image_paths)`
    /// — the caller is responsible for removing the files from disk.
    pub fn delete_all_screens(db: &Database) -> Result<(u64, Vec<PathBuf>), BlueyError> {
        db.transaction(|conn| {
            let mut stmt = conn
                .prepare("SELECT image_path FROM screen_snapshots WHERE image_path IS NOT NULL")
                .sql()?;
            let paths = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .sql()?
                .collect::<Result<Vec<String>, _>>()
                .sql()?;
            let deleted = conn.execute("DELETE FROM screen_snapshots", []).sql()?;
            Ok((
                deleted as u64,
                paths.into_iter().map(PathBuf::from).collect(),
            ))
        })
    }

    /// Null out stored image paths without deleting the snapshot rows (used when
    /// the user disables screenshot storage but OCR context should survive).
    /// Returns the paths that were removed so the files can be deleted.
    pub fn strip_image_paths(db: &Database) -> Result<Vec<PathBuf>, BlueyError> {
        db.transaction(|conn| {
            let mut stmt = conn
                .prepare("SELECT image_path FROM screen_snapshots WHERE image_path IS NOT NULL")
                .sql()?;
            let paths = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .sql()?
                .collect::<Result<Vec<String>, _>>()
                .sql()?;
            conn.execute(
                "UPDATE screen_snapshots SET image_path = NULL WHERE image_path IS NOT NULL",
                [],
            )
            .sql()?;
            Ok(paths.into_iter().map(PathBuf::from).collect())
        })
    }

    /// Store an accessibility snapshot (bounded JSON). Returns the snapshot id.
    pub fn save_accessibility(
        db: &Database,
        session_id: Option<&str>,
        context: &AccessibilityContext,
    ) -> Result<String, BlueyError> {
        let id = new_id("axs");
        let json = to_json_string(context)?;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accessibility_snapshots (id, session_id, snapshot_json, captured_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, session_id, json, context.captured_at],
            )
            .sql()?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Delete screen snapshots captured before `cutoff_iso` (RFC 3339).
    /// Returns `(deleted_rows, image_paths)`.
    pub fn prune_screens_before(
        db: &Database,
        cutoff_iso: &str,
    ) -> Result<(u64, Vec<PathBuf>), BlueyError> {
        db.transaction(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT image_path FROM screen_snapshots
                      WHERE captured_at < ?1 AND image_path IS NOT NULL",
                )
                .sql()?;
            let paths = stmt
                .query_map([cutoff_iso], |r| r.get::<_, String>(0))
                .sql()?
                .collect::<Result<Vec<String>, _>>()
                .sql()?;
            let deleted = conn
                .execute(
                    "DELETE FROM screen_snapshots WHERE captured_at < ?1",
                    [cutoff_iso],
                )
                .sql()?;
            Ok((
                deleted as u64,
                paths.into_iter().map(PathBuf::from).collect(),
            ))
        })
    }

    /// Delete accessibility snapshots captured before `cutoff_iso`. Returns rows removed.
    pub fn prune_accessibility_before(db: &Database, cutoff_iso: &str) -> Result<u64, BlueyError> {
        db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM accessibility_snapshots WHERE captured_at < ?1",
                [cutoff_iso],
            )
            .map(|n| n as u64)
            .sql()
        })
    }

    /// Number of stored screen snapshots.
    pub fn count_screens(db: &Database) -> Result<u64, BlueyError> {
        db.with_conn(|conn| {
            conn.query_row("SELECT count(*) FROM screen_snapshots", [], |r| {
                r.get::<_, i64>(0)
            })
            .map(|n| n.max(0) as u64)
            .sql()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::SessionRepository;
    use crate::testutil;
    use bluey_core::now_iso;
    use bluey_core::types::context::{CaptureTarget, ImageMimeType};
    use pretty_assertions::assert_eq;

    fn frame(path: Option<&str>) -> ScreenFrame {
        ScreenFrame {
            id: bluey_core::new_id("frame"),
            image: None,
            mime_type: ImageMimeType::Jpeg,
            path: path.map(str::to_string),
            width: 1600,
            height: 1000,
            display_id: Some("display-1".into()),
            scale_factor: 2.0,
            captured_at: now_iso(),
            hash: Some("abc123".into()),
            changed: true,
            target: CaptureTarget::default(),
            duration_ms: Some(42),
        }
    }

    fn ax_context() -> AccessibilityContext {
        AccessibilityContext {
            application: ApplicationContext {
                name: "Zoom".into(),
                bundle_id: None,
                pid: Some(1),
            },
            window: None,
            focused_element: None,
            elements: vec![],
            selected_text: None,
            visible_text: "meeting notes".into(),
            truncated: false,
            captured_at: now_iso(),
        }
    }

    #[test]
    fn save_screen_respects_store_image_flag() {
        let db = testutil::db();
        let s = SessionRepository::create(&db, "general", None).unwrap();
        let ocr = OcrContext {
            blocks: vec![],
            text: "Terminal output".into(),
            level: bluey_core::types::context::OcrLevel::Fast,
            languages: vec!["en-US".into()],
            duration_ms: 12,
            frame_id: None,
        };
        SnapshotRepository::save_screen(
            &db,
            Some(&s.id),
            &frame(Some("/tmp/a.jpg")),
            Some(&ocr),
            None,
            None,
            true,
        )
        .unwrap();
        SnapshotRepository::save_screen(
            &db,
            Some(&s.id),
            &frame(Some("/tmp/b.jpg")),
            None,
            None,
            None,
            false,
        )
        .unwrap();

        assert_eq!(SnapshotRepository::count_screens(&db).unwrap(), 2);
        assert_eq!(
            SnapshotRepository::list_screen_paths(&db).unwrap(),
            vec![PathBuf::from("/tmp/a.jpg")]
        );
        let stored_ocr: Option<String> = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT ocr_text FROM screen_snapshots WHERE image_path = '/tmp/a.jpg'",
                    [],
                    |r| r.get(0),
                )
                .sql()
            })
            .unwrap();
        assert_eq!(stored_ocr.as_deref(), Some("Terminal output"));
    }

    #[test]
    fn delete_strip_and_prune() {
        let db = testutil::db();
        let s = SessionRepository::create(&db, "general", None).unwrap();
        SnapshotRepository::save_screen(
            &db,
            Some(&s.id),
            &frame(Some("/tmp/a.jpg")),
            None,
            None,
            None,
            true,
        )
        .unwrap();
        SnapshotRepository::save_screen(
            &db,
            None,
            &frame(Some("/tmp/b.jpg")),
            None,
            None,
            None,
            true,
        )
        .unwrap();

        let stripped = SnapshotRepository::strip_image_paths(&db).unwrap();
        assert_eq!(stripped.len(), 2);
        assert!(SnapshotRepository::list_screen_paths(&db)
            .unwrap()
            .is_empty());
        assert_eq!(SnapshotRepository::count_screens(&db).unwrap(), 2);

        SnapshotRepository::save_screen(
            &db,
            None,
            &frame(Some("/tmp/c.jpg")),
            None,
            None,
            None,
            true,
        )
        .unwrap();
        let (deleted, paths) = SnapshotRepository::delete_all_screens(&db).unwrap();
        assert_eq!(deleted, 3);
        assert_eq!(paths, vec![PathBuf::from("/tmp/c.jpg")]);
        assert_eq!(SnapshotRepository::count_screens(&db).unwrap(), 0);

        // Prune by captured_at.
        SnapshotRepository::save_screen(
            &db,
            None,
            &frame(Some("/tmp/old.jpg")),
            None,
            None,
            None,
            true,
        )
        .unwrap();
        let (deleted, paths) =
            SnapshotRepository::prune_screens_before(&db, "2999-01-01T00:00:00Z").unwrap();
        assert_eq!((deleted, paths.len()), (1, 1));

        let ax = ax_context();
        SnapshotRepository::save_accessibility(&db, None, &ax).unwrap();
        assert_eq!(
            SnapshotRepository::prune_accessibility_before(&db, "2999-01-01T00:00:00Z").unwrap(),
            1
        );
        assert_eq!(testutil::count(&db, "accessibility_snapshots"), 0);
    }
}
