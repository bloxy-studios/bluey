//! Private rusqlite → [`BlueyError`] mapping.
//!
//! The orphan rule prevents `impl From<rusqlite::Error> for BlueyError` (both types are
//! foreign to this crate), so the conversion lives in [`db_err`] plus the [`SqlExt`]
//! extension trait used throughout the crate: `conn.execute(..).sql()?`.
//!
//! Error codes (all under the `storage.` prefix):
//! * `busy`       — the database is locked/busy (recoverable, retry).
//! * `constraint` — a uniqueness / foreign key / check constraint failed.
//! * `query`      — any other query or binding failure.
//! * `migration`  — a schema migration failed (set explicitly by `run_migrations`).
//! * `io`         — the database file could not be opened/read/written.

use bluey_core::error::{BlueyError, RecoveryAction};
use rusqlite::ErrorCode;

/// Convert a rusqlite error into the crate-wide [`BlueyError`] storage shape.
pub(crate) fn db_err(err: rusqlite::Error) -> BlueyError {
    match &err {
        rusqlite::Error::SqliteFailure(f, msg) => {
            let detail = msg.clone().unwrap_or_else(|| f.to_string());
            match f.code {
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => {
                    BlueyError::storage("busy", format!("database is busy: {detail}"))
                        .recoverable(RecoveryAction::Retry)
                }
                ErrorCode::ConstraintViolation => {
                    BlueyError::storage("constraint", format!("constraint violation: {detail}"))
                }
                ErrorCode::CannotOpen
                | ErrorCode::DiskFull
                | ErrorCode::ReadOnly
                | ErrorCode::SystemIoFailure
                | ErrorCode::DatabaseCorrupt
                | ErrorCode::NotADatabase => {
                    BlueyError::storage("io", format!("database I/O failure: {detail}"))
                }
                _ => BlueyError::storage("query", detail),
            }
        }
        _ => BlueyError::storage("query", err.to_string()),
    }
}

/// `?`-friendly conversion for `Result<T, rusqlite::Error>`.
pub(crate) trait SqlExt<T> {
    /// Map the rusqlite error into a `BlueyError::storage(..)`.
    fn sql(self) -> Result<T, BlueyError>;
}

impl<T> SqlExt<T> for Result<T, rusqlite::Error> {
    fn sql(self) -> Result<T, BlueyError> {
        self.map_err(db_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_constraint_and_busy_codes() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id TEXT PRIMARY KEY)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES ('a')", []).unwrap();
        let err = conn.execute("INSERT INTO t VALUES ('a')", []).unwrap_err();
        let mapped = db_err(err);
        assert_eq!(mapped.code, "storage.constraint");

        let err = conn.prepare("SELECT nope FROM t").unwrap_err();
        let mapped = db_err(err);
        assert_eq!(mapped.code, "storage.query");
    }
}
