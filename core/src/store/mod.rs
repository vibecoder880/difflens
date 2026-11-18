//! Local session store: finished comparisons persisted in a bundled SQLite
//! database (rusqlite, no system dependency).
//!
//! The store owns no paths — callers hand it an explicit database file
//! path, which keeps it OS-agnostic and headless-testable (the Tauri shell
//! passes `app_data_dir/difflens.sqlite3`; tests pass `tempfile` paths or
//! use [`Store::open_in_memory`]). All queries are parameterized; the
//! schema is created/updated via the `user_version` pragma migrations in
//! [`schema`]. Results are stored as full serialized `DiffResult` JSON so a
//! restored session renders byte-identically to the live one.
//!
//! No network, no process spawning — pure local file I/O.

mod schema;

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::CoreResult;
use crate::model::{DiffResult, DiffStats, DocFormat};

/// Skip persisting absurdly large results (plan risk note): history blobs
/// beyond 64 MB would bloat the database without being renderable anyway.
pub const MAX_RESULT_JSON_BYTES: usize = 64 * 1024 * 1024;

/// A finished comparison to persist. `created_at` is an explicit epoch-ms
/// timestamp — the store never reads the wall clock (determinism /
/// headless-testability; the shell injects "now").
#[derive(Debug, Clone, PartialEq)]
pub struct NewSession {
    pub id: String,
    pub created_at: i64,
    pub old_path: String,
    pub new_path: String,
    pub format: DocFormat,
    pub stats: DiffStats,
    pub embeddings_used: bool,
    pub ocr_used: bool,
    /// Full serialized `DiffResult`.
    pub result_json: String,
}

/// Row summary returned by [`Store::list_sessions`] — everything the
/// history list needs, without loading the result blob.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub created_at: i64,
    pub old_path: String,
    pub new_path: String,
    pub format: DocFormat,
    pub stats: DiffStats,
    pub embeddings_used: bool,
    pub ocr_used: bool,
}

/// A full stored session: [`SessionSummary`] plus the deserialized
/// [`DiffResult`]. `result` is `None` when the stored JSON is corrupt —
/// history browsing keeps working (delete/re-run still possible) instead of
/// erroring on one bad row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredSession {
    pub id: String,
    pub created_at: i64,
    pub old_path: String,
    pub new_path: String,
    pub format: DocFormat,
    pub stats: DiffStats,
    pub embeddings_used: bool,
    pub ocr_used: bool,
    pub result: Option<DiffResult>,
}

impl StoredSession {
    /// View this row as a summary (drops the result blob).
    pub fn summary(&self) -> SessionSummary {
        SessionSummary {
            id: self.id.clone(),
            created_at: self.created_at,
            old_path: self.old_path.clone(),
            new_path: self.new_path.clone(),
            format: self.format,
            stats: self.stats.clone(),
            embeddings_used: self.embeddings_used,
            ocr_used: self.ocr_used,
        }
    }
}

/// Open SQLite connection with pending migrations applied.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating or migrating as needed) the database at `path`.
    ///
    /// Enables WAL journalling and a 5 s busy timeout so concurrent UI
    /// reads/writes never fail with "database is locked".
    pub fn open(path: &Path) -> CoreResult<Store> {
        let conn = Connection::open(path)
            .map_err(|e| crate::error::CoreError::Store(format!("open {path:?}: {e}")))?;
        Self::configure(conn)
    }

    /// In-memory database (tests; the data dies with the connection).
    pub fn open_in_memory() -> CoreResult<Store> {
        let conn = Connection::open_in_memory()
            .map_err(|e| crate::error::CoreError::Store(format!("open in-memory: {e}")))?;
        Self::configure(conn)
    }

    fn configure(conn: Connection) -> CoreResult<Store> {
        // WAL: readers don't block the writer. On :memory: databases the
        // pragma is a harmless no-op (reports "memory").
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "busy_timeout", 5_000)
            .map_err(|e| crate::error::CoreError::Store(format!("busy_timeout: {e}")))?;
        schema::migrate(&conn)?;
        Ok(Store { conn })
    }

    /// Insert (or replace) a session row. Replacing an existing id keeps
    /// re-running the same job id idempotent. Results larger than
    /// [`MAX_RESULT_JSON_BYTES`] are skipped with a warning, not an error.
    pub fn insert_session(&self, session: &NewSession) -> CoreResult<()> {
        if session.result_json.len() > MAX_RESULT_JSON_BYTES {
            tracing::warn!(
                id = %session.id,
                bytes = session.result_json.len(),
                "result too large to persist; skipping (> {} bytes)",
                MAX_RESULT_JSON_BYTES
            );
            return Ok(());
        }
        self.conn
            .execute(
                "INSERT INTO sessions (id, created_at, old_path, new_path, format, stats_json, \
                 embeddings_used, ocr_used, result_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                   created_at = excluded.created_at,
                   old_path = excluded.old_path,
                   new_path = excluded.new_path,
                   format = excluded.format,
                   stats_json = excluded.stats_json,
                   embeddings_used = excluded.embeddings_used,
                   ocr_used = excluded.ocr_used,
                   result_json = excluded.result_json",
                params![
                    session.id,
                    session.created_at,
                    session.old_path,
                    session.new_path,
                    format_serde(&session.format)?,
                    stats_json(&session.stats)?,
                    session.embeddings_used as i64,
                    session.ocr_used as i64,
                    session.result_json,
                ],
            )
            .map_err(|e| crate::error::CoreError::Store(format!("insert session: {e}")))?;
        Ok(())
    }

    /// Paged row summaries, newest first. Corrupt `stats_json` degrades to
    /// zeroed stats instead of failing the whole list.
    pub fn list_sessions(&self, limit: u32, offset: u32) -> CoreResult<Vec<SessionSummary>> {
        let limit = limit.clamp(1, 500);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, created_at, old_path, new_path, format, stats_json, embeddings_used, \
                 ocr_used
                 FROM sessions
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| crate::error::CoreError::Store(format!("list sessions: {e}")))?;
        let rows = stmt
            .query_map(params![limit, offset], |row| {
                Ok(RawSummary {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    old_path: row.get(2)?,
                    new_path: row.get(3)?,
                    format: row.get(4)?,
                    stats_json: row.get(5)?,
                    embeddings_used: row.get::<_, i64>(6)? != 0,
                    ocr_used: row.get::<_, i64>(7)? != 0,
                })
            })
            .map_err(|e| crate::error::CoreError::Store(format!("list sessions: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let raw = row.map_err(|e| crate::error::CoreError::Store(format!("list row: {e}")))?;
            out.push(raw.into_summary());
        }
        Ok(out)
    }

    /// Full session (summary + deserialized result); `None` for unknown ids.
    pub fn get_session(&self, id: &str) -> CoreResult<Option<StoredSession>> {
        let raw = self
            .conn
            .query_row(
                "SELECT id, created_at, old_path, new_path, format, stats_json, embeddings_used, \
                 ocr_used, result_json
                 FROM sessions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(RawSession {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        old_path: row.get(2)?,
                        new_path: row.get(3)?,
                        format: row.get(4)?,
                        stats_json: row.get(5)?,
                        embeddings_used: row.get::<_, i64>(6)? != 0,
                        ocr_used: row.get::<_, i64>(7)? != 0,
                        result_json: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(|e| crate::error::CoreError::Store(format!("get session: {e}")))?;
        Ok(raw.map(RawSession::into_stored))
    }

    /// Delete a session row; `true` when a row was removed.
    pub fn delete_session(&self, id: &str) -> CoreResult<bool> {
        let n = self
            .conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])
            .map_err(|e| crate::error::CoreError::Store(format!("delete session: {e}")))?;
        Ok(n > 0)
    }

    /// Delete every session (settings "clear history").
    pub fn clear_sessions(&self) -> CoreResult<u64> {
        let n = self
            .conn
            .execute("DELETE FROM sessions", [])
            .map_err(|e| crate::error::CoreError::Store(format!("clear sessions: {e}")))?;
        Ok(n as u64)
    }

    /// Read an app preference (generic kv table).
    pub fn get_pref(&self, key: &str) -> CoreResult<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM app_prefs WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| crate::error::CoreError::Store(format!("get pref: {e}")))
    }

    /// Write an app preference.
    pub fn set_pref(&self, key: &str, value: &str) -> CoreResult<()> {
        self.conn
            .execute(
                "INSERT INTO app_prefs (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(|e| crate::error::CoreError::Store(format!("set pref: {e}")))?;
        Ok(())
    }
}

/// Raw column decode helper — `format` and `stats_json` need post-processing.
struct RawSummary {
    id: String,
    created_at: i64,
    old_path: String,
    new_path: String,
    format: String,
    stats_json: String,
    embeddings_used: bool,
    ocr_used: bool,
}

impl RawSummary {
    fn into_summary(self) -> SessionSummary {
        SessionSummary {
            id: self.id,
            created_at: self.created_at,
            old_path: self.old_path,
            new_path: self.new_path,
            format: parse_format(&self.format),
            stats: parse_stats(&self.stats_json),
            embeddings_used: self.embeddings_used,
            ocr_used: self.ocr_used,
        }
    }
}

/// Raw row including the result blob.
struct RawSession {
    id: String,
    created_at: i64,
    old_path: String,
    new_path: String,
    format: String,
    stats_json: String,
    embeddings_used: bool,
    ocr_used: bool,
    result_json: String,
}

impl RawSession {
    fn into_stored(self) -> StoredSession {
        let result = match serde_json::from_str::<DiffResult>(&self.result_json) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!(id = %self.id, error = %e, "stored result JSON corrupt; \
                                returning session without result");
                None
            }
        };
        StoredSession {
            id: self.id,
            created_at: self.created_at,
            old_path: self.old_path,
            new_path: self.new_path,
            format: parse_format(&self.format),
            stats: parse_stats(&self.stats_json),
            embeddings_used: self.embeddings_used,
            ocr_used: self.ocr_used,
            result,
        }
    }
}

fn format_serde(format: &DocFormat) -> CoreResult<String> {
    serde_json::to_string(format)
        .map_err(|e| crate::error::CoreError::Store(format!("serialize format: {e}")))
}

fn stats_json(stats: &DiffStats) -> CoreResult<String> {
    serde_json::to_string(stats)
        .map_err(|e| crate::error::CoreError::Store(format!("serialize stats: {e}")))
}

/// Tolerant decode: unknown format strings fall back to Txt.
fn parse_format(s: &str) -> DocFormat {
    serde_json::from_str(s).unwrap_or(DocFormat::Txt)
}

/// Tolerant decode: corrupt stats degrade to zeros.
fn parse_stats(s: &str) -> DiffStats {
    serde_json::from_str(s).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, created_at: i64) -> NewSession {
        let result = DiffResult {
            old_path: "/tmp/report-old.docx".into(),
            new_path: "/tmp/report-new.docx".into(),
            format: DocFormat::Docx,
            changes: vec![],
            stats: DiffStats {
                added: 2,
                removed: 1,
                modified: 3,
                moved: 0,
                equal: 7,
            },
            embeddings_used: true,
            ocr_used: false,
            audit: vec![],
        };
        NewSession {
            id: id.into(),
            created_at,
            old_path: result.old_path.clone(),
            new_path: result.new_path.clone(),
            format: result.format,
            stats: result.stats.clone(),
            embeddings_used: result.embeddings_used,
            ocr_used: result.ocr_used,
            result_json: serde_json::to_string(&result).expect("serialize"),
        }
    }

    #[test]
    fn insert_get_roundtrip_preserves_result() {
        let store = Store::open_in_memory().expect("store");
        store
            .insert_session(&sample("job-1", 1_000))
            .expect("insert");
        let got = store.get_session("job-1").expect("get").expect("present");
        assert_eq!(got.id, "job-1");
        assert_eq!(got.created_at, 1_000);
        assert_eq!(got.format, DocFormat::Docx);
        assert_eq!(got.stats.added, 2);
        assert_eq!(got.stats.modified, 3);
        assert!(got.embeddings_used);
        assert!(!got.ocr_used);
        let result = got.result.clone().expect("result parses");
        assert_eq!(result.format, DocFormat::Docx);
        assert_eq!(result.old_path, "/tmp/report-old.docx");
        assert_eq!(got.summary(), store.list_sessions(10, 0).unwrap()[0]);
    }

    #[test]
    fn get_unknown_id_is_none() {
        let store = Store::open_in_memory().expect("store");
        assert!(store.get_session("nope").expect("get").is_none());
    }

    #[test]
    fn list_orders_newest_first_and_pages() {
        let store = Store::open_in_memory().expect("store");
        for (i, ts) in [300_i64, 100, 200].into_iter().enumerate() {
            store
                .insert_session(&sample(&format!("job-{i}"), ts))
                .expect("insert");
        }
        let all = store.list_sessions(10, 0).expect("list");
        let stamps: Vec<i64> = all.iter().map(|s| s.created_at).collect();
        assert_eq!(stamps, vec![300, 200, 100]);

        let page0 = store.list_sessions(2, 0).expect("page0");
        assert_eq!(page0.len(), 2);
        assert_eq!(page0[0].created_at, 300);
        let page1 = store.list_sessions(2, 2).expect("page1");
        assert_eq!(page1.len(), 1);
        assert_eq!(page1[0].created_at, 100);
    }

    #[test]
    fn delete_removes_exactly_one_row() {
        let store = Store::open_in_memory().expect("store");
        store.insert_session(&sample("job-a", 1)).expect("insert");
        store.insert_session(&sample("job-b", 2)).expect("insert");
        assert!(store.delete_session("job-a").expect("delete"));
        assert!(!store.delete_session("job-a").expect("delete again"));
        let left = store.list_sessions(10, 0).expect("list");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "job-b");
    }

    #[test]
    fn corrupt_result_json_is_tolerated() {
        let store = Store::open_in_memory().expect("store");
        let mut bad = sample("job-bad", 5);
        bad.result_json = "this is not json {{{".into();
        store.insert_session(&bad).expect("insert raw");
        let got = store.get_session("job-bad").expect("get").expect("present");
        assert!(got.result.is_none(), "corrupt result must yield None");
        // Summary still lists the row for deletion/browsing.
        assert_eq!(store.list_sessions(10, 0).unwrap().len(), 1);
    }

    #[test]
    fn corrupt_stats_json_degrades_to_zeroed_stats() {
        let store = Store::open_in_memory().expect("store");
        store.insert_session(&sample("job-s", 6)).expect("insert");
        // Corrupt the stats column directly (same effect as a bad write).
        store
            .conn
            .execute(
                "UPDATE sessions SET stats_json = '[]' WHERE id = 'job-s'",
                [],
            )
            .expect("corrupt stats");
        let all = store.list_sessions(10, 0).expect("list");
        assert_eq!(all[0].stats, DiffStats::default());
    }

    #[test]
    fn same_id_replaces_existing_row() {
        let store = Store::open_in_memory().expect("store");
        store.insert_session(&sample("job-1", 100)).expect("insert");
        store.insert_session(&sample("job-1", 200)).expect("upsert");
        let all = store.list_sessions(10, 0).expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].created_at, 200);
    }

    #[test]
    fn oversized_results_are_skipped_with_warning() {
        let store = Store::open_in_memory().expect("store");
        let mut huge = sample("job-huge", 1);
        huge.result_json = "x".repeat(MAX_RESULT_JSON_BYTES + 1);
        store.insert_session(&huge).expect("skip is not an error");
        assert!(store.get_session("job-huge").expect("get").is_none());
    }

    #[test]
    fn prefs_roundtrip() {
        let store = Store::open_in_memory().expect("store");
        assert!(store.get_pref("lang").expect("get").is_none());
        store.set_pref("lang", "vi").expect("set");
        store.set_pref("lang", "en").expect("overwrite");
        assert_eq!(store.get_pref("lang").expect("get").as_deref(), Some("en"));
    }

    #[test]
    fn file_backed_db_survives_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("difflens.sqlite3");
        {
            let store = Store::open(&path).expect("open");
            store.insert_session(&sample("job-9", 42)).expect("insert");
        }
        let reopened = Store::open(&path).expect("reopen");
        let got = reopened
            .get_session("job-9")
            .expect("get")
            .expect("present");
        assert_eq!(got.created_at, 42);
        assert!(got.result.is_some());
    }
}
