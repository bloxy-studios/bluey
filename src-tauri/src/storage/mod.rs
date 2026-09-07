//! Storage handle: app directory resolution and a [`Storage`] wrapper that
//! runs every blocking SQLite call on the tokio blocking pool so the async
//! runtime and the UI never block.

use std::path::PathBuf;
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
    /// `~/Library/Caches/com.codewithabdul.bluey/frames`
    pub frames_dir: PathBuf,
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
        for dir in [&data_dir, &frames_dir, &logs_dir] {
            std::fs::create_dir_all(dir).map_err(|e| {
                BlueyError::storage("io", format!("cannot create {}: {e}", dir.display()))
            })?;
        }
        Ok(Self {
            db_path: data_dir.join("bluey.db"),
            data_dir,
            frames_dir,
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
        let db = Database::open(&paths.db_path)?;
        Ok(Self {
            db: Arc::new(db),
            paths,
        })
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
