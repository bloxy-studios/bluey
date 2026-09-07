//! Small TTL cache for AI results (`ai_cache`). Keys are hashes of
//! `(task, model, prompt)` computed by the caller — no raw prompts here.

use bluey_core::error::BlueyError;
use bluey_core::now_iso;
use chrono::{Duration, SecondsFormat, Utc};
use rusqlite::{params, OptionalExtension};

use crate::db::Database;
use crate::error::SqlExt;

/// TTL cache over the `ai_cache` table.
pub struct AiCacheRepository;

impl AiCacheRepository {
    /// Fetch a cached value; entries past `expires_at` are treated as missing
    /// (and lazily removed).
    pub fn get(db: &Database, key: &str) -> Result<Option<String>, BlueyError> {
        let now = now_iso();
        let row: Option<(String, Option<String>)> = db.with_conn(|conn| {
            conn.query_row(
                "SELECT value, expires_at FROM ai_cache WHERE key = ?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .sql()
        })?;
        match row {
            None => Ok(None),
            Some((value, expires_at)) => {
                if expires_at.as_deref().is_some_and(|exp| exp <= now.as_str()) {
                    db.with_conn(|conn| {
                        conn.execute("DELETE FROM ai_cache WHERE key = ?1", [key])
                            .sql()?;
                        Ok(())
                    })?;
                    Ok(None)
                } else {
                    Ok(Some(value))
                }
            }
        }
    }

    /// Store a value with an optional TTL (`None` = never expires).
    pub fn set(
        db: &Database,
        key: &str,
        value: &str,
        ttl_secs: Option<u64>,
    ) -> Result<(), BlueyError> {
        let now = Utc::now();
        let expires_at = ttl_secs.map(|secs| {
            (now + Duration::seconds(secs.min(i64::MAX as u64) as i64))
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        });
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO ai_cache (key, value, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(key) DO UPDATE SET
                   value = excluded.value,
                   created_at = excluded.created_at,
                   expires_at = excluded.expires_at",
                params![
                    key,
                    value,
                    now.to_rfc3339_opts(SecondsFormat::Millis, true),
                    expires_at
                ],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Delete every cache entry, returning the number removed.
    pub fn clear(db: &Database) -> Result<u64, BlueyError> {
        db.with_conn(|conn| {
            conn.execute("DELETE FROM ai_cache", [])
                .map(|n| n as u64)
                .sql()
        })
    }

    /// Delete only expired entries, returning the number removed.
    pub fn prune_expired(db: &Database) -> Result<u64, BlueyError> {
        db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM ai_cache WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                [now_iso()],
            )
            .map(|n| n as u64)
            .sql()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;
    use pretty_assertions::assert_eq;

    #[test]
    fn set_get_expire_and_clear() {
        let db = testutil::db();
        AiCacheRepository::set(&db, "k1", "v1", None).unwrap();
        AiCacheRepository::set(&db, "k2", "v2", Some(3600)).unwrap();
        assert_eq!(
            AiCacheRepository::get(&db, "k1").unwrap().as_deref(),
            Some("v1")
        );
        assert_eq!(
            AiCacheRepository::get(&db, "k2").unwrap().as_deref(),
            Some("v2")
        );
        assert_eq!(AiCacheRepository::get(&db, "nope").unwrap(), None);

        // Overwrite value + TTL.
        AiCacheRepository::set(&db, "k1", "v1b", Some(3600)).unwrap();
        assert_eq!(
            AiCacheRepository::get(&db, "k1").unwrap().as_deref(),
            Some("v1b")
        );

        // Force k2 to be expired.
        db.with_conn(|c| {
            c.execute(
                "UPDATE ai_cache SET expires_at = '2000-01-01T00:00:00.000Z' WHERE key = 'k2'",
                [],
            )
            .sql()
        })
        .unwrap();
        assert_eq!(AiCacheRepository::get(&db, "k2").unwrap(), None);
        assert_eq!(
            testutil::count(&db, "ai_cache"),
            1,
            "expired entry was lazily removed"
        );

        db.with_conn(|c| {
            c.execute(
                "INSERT INTO ai_cache (key, value, created_at, expires_at)
                 VALUES ('old', 'x', '2000-01-01T00:00:00.000Z', '2000-01-02T00:00:00.000Z')",
                [],
            )
            .sql()
        })
        .unwrap();
        assert_eq!(AiCacheRepository::prune_expired(&db).unwrap(), 1);
        assert_eq!(AiCacheRepository::clear(&db).unwrap(), 1);
        assert_eq!(testutil::count(&db, "ai_cache"), 0);
    }
}
