//! Storage handle: app directory resolution and a [`Storage`] wrapper that
//! runs every blocking SQLite call on the tokio blocking pool so the async
//! runtime and the UI never block.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bluey_core::{BlueyError, BlueyResult};
use bluey_storage::Database;

/// Bundle identifier — keychain service, data and cache directory names.
pub const BUNDLE_ID: &str = "com.codewithabdul.bluey";

/// Resolved application directories (created on resolve).
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// `~/Library/Application Support/com.codewithabdul.bluey`
    pub data_dir: PathBuf,
    /// `<data_dir>/bluey.db`
    pub db_path: PathBuf,
    /// `~/Library/Caches/com.codewithabdul.bluey/frames` — the helper's temp
    /// frames, deleted once used and swept by the helper.
    pub frames_dir: PathBuf,
    /// `<data_dir>/screenshots` — copies the user chose to keep
    /// (`privacy.storeScreenshots`), referenced by `screen_snapshots.image_path`.
    pub screenshots_dir: PathBuf,
    /// `~/Library/Logs/Bluey`
    pub logs_dir: PathBuf,
}

impl AppPaths {
    /// Resolve and create the app directories. `BLUEY_DATA_DIR` overrides the
    /// data directory root (used by tests / dev sandboxes).
    pub fn resolve() -> BlueyResult<Self> {
        let data_dir = match std::env::var_os("BLUEY_DATA_DIR") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => dirs::data_dir()
                .ok_or_else(|| BlueyError::storage("io", "cannot resolve the data directory"))?
                .join(BUNDLE_ID),
        };
        let frames_dir = dirs::cache_dir()
            .ok_or_else(|| BlueyError::storage("io", "cannot resolve the cache directory"))?
            .join(BUNDLE_ID)
            .join("frames");
        let logs_dir = if cfg!(target_os = "macos") {
            dirs::home_dir()
                .ok_or_else(|| BlueyError::storage("io", "cannot resolve the home directory"))?
                .join("Library/Logs/Bluey")
        } else {
            data_dir.join("logs")
        };
        let screenshots_dir = data_dir.join("screenshots");
        for dir in [&data_dir, &frames_dir, &screenshots_dir, &logs_dir] {
            std::fs::create_dir_all(dir).map_err(|e| {
                BlueyError::storage("io", format!("cannot create {}: {e}", dir.display()))
            })?;
        }
        Ok(Self {
            db_path: data_dir.join("bluey.db"),
            data_dir,
            frames_dir,
            screenshots_dir,
            logs_dir,
        })
    }
}

/// Shared database handle. All repository calls are blocking → run them via
/// [`Storage::run`] from async contexts.
pub struct Storage {
    db: Arc<Database>,
    pub paths: Arc<AppPaths>,
}

impl Storage {
    /// Open the database at the resolved path (migrations run automatically).
    pub fn open(paths: Arc<AppPaths>) -> BlueyResult<Self> {
        // A failed or bad migration after an update can be rolled back by hand (CRIT-003).
        let db = Database::open_with_backup(&paths.db_path, env!("CARGO_PKG_VERSION"))?;
        Ok(Self {
            db: Arc::new(db),
            paths,
        })
    }

    /// A migrated in-memory database (unit tests of the managers above it).
    #[cfg(test)]
    pub fn in_memory() -> Self {
        let dir = std::env::temp_dir().join("bluey-test-storage");
        Self {
            db: Arc::new(Database::in_memory().expect("in-memory database")),
            paths: Arc::new(AppPaths {
                db_path: dir.join("bluey.db"),
                frames_dir: dir.join("frames"),
                screenshots_dir: dir.join("screenshots"),
                logs_dir: dir.join("logs"),
                data_dir: dir,
            }),
        }
    }

    /// Run a blocking storage closure on the blocking pool.
    pub async fn run<T, F>(&self, f: F) -> BlueyResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Database) -> BlueyResult<T> + Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|e| BlueyError::internal(format!("storage task panicked: {e}")))?
    }

    /// Run a blocking storage closure inline (bootstrap / shutdown paths that
    /// are already off the async runtime).
    pub fn run_sync<T>(&self, f: impl FnOnce(&Database) -> BlueyResult<T>) -> BlueyResult<T> {
        f(&self.db)
    }

    /// Best-effort deletion of every file directly inside `dir` (the helper's
    /// temp frames, orphaned screenshot copies). Returns how many went.
    pub fn clear_dir(dir: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        Self::remove_files(&files);
        files.iter().filter(|path| !path.exists()).count()
    }

    /// Delete the `<db>.bak-<version>` copies taken before migrations
    /// (CRIT-003): they hold everything the database held, so Reset must not
    /// leave them behind. Returns how many were removed.
    pub fn remove_db_backups(db_path: &Path) -> usize {
        let (Some(dir), Some(name)) = (db_path.parent(), db_path.file_name()) else {
            return 0;
        };
        let prefix = format!("{}.bak-", name.to_string_lossy());
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let backups: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
            })
            .collect();
        Self::remove_files(&backups);
        backups.iter().filter(|path| !path.exists()).count()
    }

    /// Best-effort file deletion for image paths returned by retention calls.
    pub fn remove_files(paths: &[PathBuf]) {
        for path in paths {
            if let Err(e) = std::fs::remove_file(path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = %path.display(), error = %e, "could not delete file");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_dir_deletes_the_files_and_keeps_subdirectories() {
        let dir = std::env::temp_dir().join(format!("bluey-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        for name in ["f-1.jpg", "f-2.png"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        assert_eq!(Storage::clear_dir(&dir), 2);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        assert_eq!(Storage::clear_dir(&dir.join("missing")), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_removes_the_pre_migration_database_backups_only() {
        let dir = std::env::temp_dir().join(format!("bluey-bak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "bluey.db",
            "bluey.db.bak-0.1.1",
            "bluey.db.bak-0.1.2",
            "other.db.bak-1",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        assert_eq!(Storage::remove_db_backups(&dir.join("bluey.db")), 2);
        assert!(dir.join("bluey.db").exists());
        assert!(dir.join("other.db.bak-1").exists());
        assert!(!dir.join("bluey.db.bak-0.1.1").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
