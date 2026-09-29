//! Connection management and migrations.
//!
//! One [`Database`] wraps a single [`rusqlite::Connection`] behind a
//! [`parking_lot::Mutex`]; every repository call locks it for the duration of a
//! query or transaction. This is deliberate — Bluey is a single-process desktop
//! app and WAL mode keeps the occasional concurrent reader happy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bluey_core::error::BlueyError;
use bluey_core::now_iso;
use parking_lot::Mutex;
use rusqlite::Connection;

use crate::error::{db_err, SqlExt};

/// Embedded, ordered SQL migrations (`migrations/NNNN_*.sql`).
pub const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    (
        "0002_fts_sync",
        include_str!("../migrations/0002_fts_sync.sql"),
    ),
    (
        "0003_embedding_model",
        include_str!("../migrations/0003_embedding_model.sql"),
    ),
    (
        "0004_ai_request_trace",
        include_str!("../migrations/0004_ai_request_trace.sql"),
    ),
    (
        "0005_modes_lifecycle",
        include_str!("../migrations/0005_modes_lifecycle.sql"),
    ),
    (
        "0006_shortcut_defaults",
        include_str!("../migrations/0006_shortcut_defaults.sql"),
    ),
];

/// A single SQLite database handle shared by all repositories.
pub struct Database {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Database {
    /// Open (creating if needed) the database at `path`, apply the connection
    /// PRAGMAs (`journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`,
    /// `busy_timeout=5000`, `temp_store=MEMORY`, `recursive_triggers=ON`) and run
    /// any pending migrations. Parent directories are created automatically.
    pub fn open(path: &Path) -> Result<Self, BlueyError> {
        let db = Self::open_unmigrated(path)?;
        db.run_migrations()?;
        db.enable_fts_secure_delete()?;
        Ok(db)
    }

    /// [`Database::open`], but an existing database with migrations pending is
    /// first copied to `<path>.bak-<tag>` (the app passes its version), so a
    /// migration that fails or goes wrong after an update can be rolled back
    /// by hand (CRIT-003). A backup that cannot be written is logged, not fatal.
    pub fn open_with_backup(path: &Path, tag: &str) -> Result<Self, BlueyError> {
        let db = Self::open_unmigrated(path)?;
        if let Err(e) = db.backup_before_migrations(tag) {
            tracing::warn!(error = %e, "could not back up the database before migrating");
        }
        db.run_migrations()?;
        db.enable_fts_secure_delete()?;
        Ok(db)
    }

    fn open_unmigrated(path: &Path) -> Result<Self, BlueyError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    BlueyError::storage("io", format!("cannot create database directory: {e}"))
                })?;
            }
        }
        let conn = Connection::open(path)
            .map_err(|e| BlueyError::storage("io", format!("cannot open database: {e}")))?;
        Self::configure(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: Some(path.to_path_buf()),
        })
    }

    /// Copy the database to `<path>.bak-<tag>` when it already holds data and
    /// has migrations pending. An existing backup with that tag is kept: it is
    /// the older, pre-migration copy. Returns the backup path when there is one.
    pub fn backup_before_migrations(&self, tag: &str) -> Result<Option<PathBuf>, BlueyError> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        let applied: Vec<String> = self.with_conn(|conn| {
            let has_table: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations')",
                    [],
                    |r| r.get(0),
                )
                .sql()?;
            if !has_table {
                return Ok(Vec::new());
            }
            let mut stmt = conn.prepare("SELECT name FROM schema_migrations").sql()?;
            let rows = stmt.query_map([], |r| r.get(0)).sql()?;
            rows.collect::<Result<Vec<String>, _>>().sql()
        })?;
        let pending = MIGRATIONS
            .iter()
            .any(|(name, _)| !applied.iter().any(|done| done == name));
        if applied.is_empty() || !pending {
            return Ok(None);
        }
        let backup = PathBuf::from(format!("{}.bak-{tag}", path.display()));
        if !backup.exists() {
            let target = backup.to_string_lossy().into_owned();
            self.with_conn(|conn| conn.execute("VACUUM INTO ?1", [target]).map(|_| ()).sql())?;
            tracing::info!(backup = %backup.display(), "backed up the database before migrating");
        }
        Ok(Some(backup))
    }

    /// In-memory database for tests: same PRAGMAs, migrations already applied.
    pub fn in_memory() -> Result<Self, BlueyError> {
        let conn = Connection::open_in_memory().map_err(|e| {
            BlueyError::storage("io", format!("cannot open in-memory database: {e}"))
        })?;
        Self::configure(&conn)?;
        let db = Self {
            conn: Mutex::new(conn),
            path: None,
        };
        db.run_migrations()?;
        db.enable_fts_secure_delete()?;
        Ok(db)
    }

    fn configure(conn: &Connection) -> Result<(), BlueyError> {
        conn.busy_timeout(Duration::from_millis(5000)).sql()?;
        // `journal_mode` returns a row, so it cannot go through execute_batch.
        conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })
        .map(|_| ())
        .sql()?;
        conn.execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA recursive_triggers = ON;
             PRAGMA secure_delete = ON;",
        )
        .sql()?;
        Ok(())
    }

    /// FTS5 `secure-delete` (persistent per table): a deleted row's tokens
    /// leave the full-text index at once instead of lingering in old index
    /// segments until a merge. With `PRAGMA secure_delete` and the WAL
    /// checkpoints in `retention`, deleted text is really gone (DATA-010).
    fn enable_fts_secure_delete(&self) -> Result<(), BlueyError> {
        self.with_conn(|conn| {
            for table in ["transcript_fts", "responses_fts", "document_chunks_fts"] {
                conn.execute(
                    &format!("INSERT INTO {table}({table}, rank) VALUES ('secure-delete', 1)"),
                    [],
                )
                .sql()?;
            }
            Ok(())
        })
    }

    /// Apply every migration from [`MIGRATIONS`] that has not been recorded in
    /// `schema_migrations` yet. Each migration runs inside its own transaction
    /// and is recorded atomically with it, so the whole procedure is idempotent
    /// and crash-safe. Returns the number of migrations applied by this call.
    pub fn run_migrations(&self) -> Result<u32, BlueyError> {
        let mut guard = self.conn.lock();
        let conn = &mut *guard;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
               name       TEXT PRIMARY KEY,
               applied_at TEXT NOT NULL
             );",
        )
        .map_err(|e| migration_err("schema_migrations", e))?;

        let mut applied = 0u32;
        for (name, sql) in MIGRATIONS {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE name = ?1)",
                    [name],
                    |r| r.get(0),
                )
                .map_err(|e| migration_err(name, e))?;
            if exists {
                continue;
            }
            let tx = conn.transaction().map_err(|e| migration_err(name, e))?;
            tx.execute_batch(sql).map_err(|e| migration_err(name, e))?;
            tx.execute(
                "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, ?2)",
                rusqlite::params![name, now_iso()],
            )
            .map_err(|e| migration_err(name, e))?;
            tx.commit().map_err(|e| migration_err(name, e))?;
            applied += 1;
            tracing::info!(migration = name, "applied database migration");
        }
        Ok(applied)
    }

    /// Run `f` with the (locked) connection.
    ///
    /// Do **not** call other `Database` methods from inside `f` — the mutex is
    /// not re-entrant and doing so deadlocks.
    pub fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, BlueyError>,
    ) -> Result<T, BlueyError> {
        let guard = self.conn.lock();
        f(&guard)
    }

    /// Run `f` inside a transaction: committed when `f` returns `Ok`, rolled
    /// back when it returns `Err`. The same re-entrancy rule as
    /// [`Database::with_conn`] applies.
    pub fn transaction<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, BlueyError>,
    ) -> Result<T, BlueyError> {
        let mut guard = self.conn.lock();
        let tx = guard.transaction().sql()?;
        let value = f(&tx)?;
        tx.commit().sql()?;
        Ok(value)
    }

    /// Filesystem path of the database, `None` for in-memory databases.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Run `VACUUM` to compact the database file.
    pub fn vacuum(&self) -> Result<(), BlueyError> {
        self.with_conn(|conn| conn.execute_batch("VACUUM").sql())
    }

    /// Checkpoint and truncate the WAL file (`PRAGMA wal_checkpoint(TRUNCATE)`).
    pub fn checkpoint(&self) -> Result<(), BlueyError> {
        self.with_conn(|conn| {
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_row| Ok(()))
                .sql()
        })
    }

    /// Logical database size in bytes (`page_count * page_size`), which also
    /// works for in-memory databases.
    pub fn db_size_bytes(&self) -> Result<u64, BlueyError> {
        self.with_conn(|conn| {
            let pages: i64 = conn
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .sql()?;
            let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0)).sql()?;
            Ok((pages.max(0) as u64) * (page_size.max(0) as u64))
        })
    }

    /// Whether the linked SQLite has the FTS5 module compiled in.
    pub fn fts5_available(&self) -> Result<bool, BlueyError> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT count(*) FROM pragma_module_list WHERE name = 'fts5'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .map_err(db_err)
        })
    }
}

fn migration_err(name: &str, e: rusqlite::Error) -> BlueyError {
    BlueyError::storage("migration", format!("migration {name} failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_open_applies_migrations_idempotently() {
        let db = Database::in_memory().unwrap();
        // A second run applies nothing.
        assert_eq!(db.run_migrations().unwrap(), 0);
        let names: Vec<String> = db
            .with_conn(|c| {
                let mut stmt = c
                    .prepare("SELECT name FROM schema_migrations ORDER BY name")
                    .sql()?;
                let rows = stmt.query_map([], |r| r.get(0)).sql()?;
                rows.collect::<Result<Vec<String>, _>>().sql()
            })
            .unwrap();
        assert_eq!(
            names,
            vec![
                "0001_init".to_string(),
                "0002_fts_sync".to_string(),
                "0003_embedding_model".to_string(),
                "0004_ai_request_trace".to_string(),
                "0005_modes_lifecycle".to_string(),
                "0006_shortcut_defaults".to_string()
            ]
        );
        assert!(db.path().is_none());
    }

    #[test]
    fn a_database_with_pending_migrations_is_backed_up_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bluey.db");
        let backup = dir.path().join("bluey.db.bak-9.9.9");

        // A fresh database has nothing to lose: no backup.
        drop(Database::open_with_backup(&path, "9.9.9").unwrap());
        assert!(!backup.exists());

        // An update brings a migration this database has not applied yet.
        let db = Database::open(&path).unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO settings (key, value, updated_at) VALUES ('probe', '1', 'x');
                 DELETE FROM schema_migrations WHERE name = '0006_shortcut_defaults';",
            )
            .sql()
        })
        .unwrap();
        drop(db);
        let db = Database::open_with_backup(&path, "9.9.9").unwrap();
        assert_eq!(db.run_migrations().unwrap(), 0, "migrated after the backup");
        drop(db);

        let copy = Connection::open(&backup).unwrap();
        let probe: String = copy
            .query_row("SELECT value FROM settings WHERE key = 'probe'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(probe, "1");
        let pre_migration: bool = copy
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM schema_migrations WHERE name = '0006_shortcut_defaults')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(pre_migration, "the copy is the pre-migration state");
    }

    #[test]
    fn fts5_is_available_with_bundled_sqlite() {
        let db = Database::in_memory().unwrap();
        assert!(
            db.fts5_available().unwrap(),
            "bundled SQLite must include FTS5"
        );
        // And an actual virtual table can be created + queried.
        db.with_conn(|c| {
            c.execute_batch(
                "CREATE VIRTUAL TABLE temp.fts_probe USING fts5(x);
                 INSERT INTO temp.fts_probe(x) VALUES ('hello world');",
            )
            .sql()?;
            let n: i64 = c
                .query_row(
                    "SELECT count(*) FROM temp.fts_probe WHERE fts_probe MATCH 'hello'",
                    [],
                    |r| r.get(0),
                )
                .sql()?;
            assert_eq!(n, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn open_creates_parent_dirs_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/bluey.db");
        let db = Database::open(&path).unwrap();
        assert_eq!(db.path(), Some(path.as_path()));
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO settings (key, value, updated_at) VALUES ('k', '1', ?1)",
                [now_iso()],
            )
            .sql()
        })
        .unwrap();
        db.checkpoint().unwrap();
        db.vacuum().unwrap();
        assert!(db.db_size_bytes().unwrap() > 0);
        drop(db);

        // Reopen: data still there, migrations recorded.
        let db = Database::open(&path).unwrap();
        let v: String = db
            .with_conn(|c| {
                c.query_row("SELECT value FROM settings WHERE key = 'k'", [], |r| {
                    r.get(0)
                })
                .sql()
            })
            .unwrap();
        assert_eq!(v, "1");
    }

    #[test]
    fn transaction_rolls_back_on_error() {
        let db = Database::in_memory().unwrap();
        let err = db.transaction(|c| {
            c.execute(
                "INSERT INTO settings (key, value, updated_at) VALUES ('a', '1', ?1)",
                [now_iso()],
            )
            .sql()?;
            Err::<(), _>(BlueyError::storage("query", "boom"))
        });
        assert!(err.is_err());
        let n: i64 = db
            .with_conn(|c| {
                c.query_row("SELECT count(*) FROM settings", [], |r| r.get(0))
                    .sql()
            })
            .unwrap();
        assert_eq!(n, 0);
    }
}
