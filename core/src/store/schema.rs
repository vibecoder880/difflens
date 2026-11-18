//! Schema migrations for the session store.
//!
//! Migrations are keyed on the SQLite `user_version` pragma: opening a
//! database applies every migration with a version greater than the stored
//! value, in order, inside one transaction. Fresh databases start at v0 and
//! run all of them; existing databases only run the delta. `CREATE TABLE IF
//! NOT EXISTS` keeps the v1 DDL idempotent for extra safety.

use rusqlite::Connection;

use crate::error::{CoreError, CoreResult};

/// Current schema version. Bump and append a new `MIGRATIONS` entry when
/// the layout changes.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// v1: `sessions` (one row per finished comparison; `result_json` holds the
/// full serialized `DiffResult`, the scalar columns power the history list
/// without touching the blob) + `app_prefs` (generic key/value store for
/// small UI preferences).
const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id              TEXT PRIMARY KEY,
    created_at      INTEGER NOT NULL,
    old_path        TEXT NOT NULL,
    new_path        TEXT NOT NULL,
    format          TEXT NOT NULL,
    stats_json      TEXT NOT NULL,
    embeddings_used INTEGER NOT NULL DEFAULT 0,
    ocr_used        INTEGER NOT NULL DEFAULT 0,
    result_json     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_created_at ON sessions (created_at DESC);
CREATE TABLE IF NOT EXISTS app_prefs (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

pub(crate) const MIGRATIONS: &[&str] = &[MIGRATION_V1];

/// Apply pending migrations (auto-migrate on open).
pub(crate) fn migrate(conn: &Connection) -> CoreResult<()> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| CoreError::Store(format!("read user_version: {e}")))?;
    if current > SCHEMA_VERSION {
        return Err(CoreError::Store(format!(
            "database schema version {current} is newer than supported ({SCHEMA_VERSION})"
        )));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = i as i64 + 1;
        if version <= current {
            continue;
        }
        conn.execute_batch(sql)
            .map_err(|e| CoreError::Store(format!("migration v{version} failed: {e}")))?;
        conn.pragma_update(None, "user_version", version)
            .map_err(|e| CoreError::Store(format!("migration v{version} stamp failed: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_db_migrates_to_current_version() {
        let conn = Connection::open_in_memory().expect("open");
        migrate(&conn).expect("migrate");
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let n: i64 = conn
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = Connection::open_in_memory().expect("open");
        migrate(&conn).expect("first");
        migrate(&conn).expect("second");
    }
}
