//! Integration tests for the SQLite session store (phase-08): temp-dir
//! databases, roundtrip fidelity, listing order/paging, deletion,
//! corrupt-row tolerance and migration reopen behavior.

use difflens_core::model::{
    AuditEntry, Change, ChangeKind, DiffResult, DiffStats, DocFormat, Severity,
};
use difflens_core::store::{NewSession, Store};

fn hostile_result() -> DiffResult {
    DiffResult {
        old_path: "/data/contracts/old <v1>.docx".into(),
        new_path: "/data/contracts/new <v2>.docx".into(),
        format: DocFormat::Docx,
        changes: vec![Change {
            id: "c0".into(),
            kind: ChangeKind::Modified,
            old_block_ids: vec!["a0".into()],
            new_block_ids: vec!["b0".into()],
            old_page: Some(3),
            new_page: Some(3),
            spans: vec![],
            old_text: None,
            new_text: None,
            similarity: Some(0.87),
            move_delta: None,
            severity: Some(Severity::High),
            flags: vec![],
        }],
        stats: DiffStats {
            added: 1,
            removed: 2,
            modified: 3,
            moved: 0,
            equal: 40,
        },
        embeddings_used: true,
        ocr_used: false,
        audit: vec![AuditEntry {
            change_id: "c0".into(),
            severity: Severity::High,
            rules: vec![],
        }],
    }
}

fn session_with(result: &DiffResult, id: &str, created_at: i64) -> NewSession {
    NewSession {
        id: id.into(),
        created_at,
        old_path: result.old_path.clone(),
        new_path: result.new_path.clone(),
        format: result.format,
        stats: result.stats.clone(),
        embeddings_used: result.embeddings_used,
        ocr_used: result.ocr_used,
        result_json: serde_json::to_string(result).expect("serialize"),
    }
}

#[test]
fn tempdir_roundtrip_preserves_full_result() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("hist.sqlite3")).expect("open");
    store
        .insert_session(&session_with(&hostile_result(), "job-1", 1_700_000_000_000))
        .expect("insert");

    let got = store.get_session("job-1").expect("get").expect("present");
    assert_eq!(got.created_at, 1_700_000_000_000);
    assert!(got.embeddings_used);
    let result = got.result.as_ref().expect("full result");
    assert_eq!(result.old_path, "/data/contracts/old <v1>.docx");
    assert_eq!(result.changes.len(), 1);
    assert_eq!(result.changes[0].severity, Some(Severity::High));
    assert_eq!(result.stats.modified, 3);
    // Summary view matches the list query.
    let listed = store.list_sessions(10, 0).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "job-1");
    assert_eq!(listed[0].stats.added, 1);
}

#[test]
fn tempdir_list_ordering_newest_first_with_paging() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("hist.sqlite3")).expect("open");
    for i in 0..5_i64 {
        store
            .insert_session(&session_with(
                &hostile_result(),
                &format!("job-{i}"),
                1_000 + i * 10,
            ))
            .expect("insert");
    }
    let newest_first: Vec<String> = store
        .list_sessions(50, 0)
        .expect("list")
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(newest_first, ["job-4", "job-3", "job-2", "job-1", "job-0"]);

    let page = store.list_sessions(2, 2).expect("page");
    assert_eq!(
        page.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["job-2", "job-1"]
    );
}

#[test]
fn tempdir_delete_and_clear() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("hist.sqlite3")).expect("open");
    store
        .insert_session(&session_with(&hostile_result(), "job-a", 1))
        .expect("insert a");
    store
        .insert_session(&session_with(&hostile_result(), "job-b", 2))
        .expect("insert b");

    assert!(store.delete_session("job-a").expect("delete"));
    assert_eq!(store.list_sessions(10, 0).expect("list").len(), 1);
    assert_eq!(store.clear_sessions().expect("clear"), 1);
    assert!(store.list_sessions(10, 0).expect("list").is_empty());
}

#[test]
fn corrupt_result_row_is_tolerated_and_deletable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("hist.sqlite3")).expect("open");
    let mut s = session_with(&hostile_result(), "job-bad", 9);
    // Simulate a truncated write / schema drift.
    s.result_json.truncate(s.result_json.len() / 2);
    store.insert_session(&s).expect("insert");

    let got = store.get_session("job-bad").expect("get").expect("present");
    assert!(
        got.result.is_none(),
        "corrupt JSON must yield result = None"
    );
    // The row still lists (history works) and can be deleted.
    assert_eq!(store.list_sessions(10, 0).unwrap().len(), 1);
    assert!(store.delete_session("job-bad").unwrap());
}

#[test]
fn reopen_applies_migrations_without_duplicating_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("hist.sqlite3");
    {
        let store = Store::open(&path).expect("open v1");
        store
            .insert_session(&session_with(&hostile_result(), "job-keep", 42))
            .expect("insert");
    }
    let reopened = Store::open(&path).expect("reopen");
    assert_eq!(reopened.list_sessions(10, 0).unwrap().len(), 1);
    let got = reopened.get_session("job-keep").unwrap().unwrap();
    assert_eq!(got.created_at, 42);
}

#[test]
fn report_of_stored_session_matches_report_of_live_result() {
    // Reports must be a pure function of the stored session: rendering from
    // the deserialized result equals rendering from the live object.
    let result = hostile_result();
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("hist.sqlite3")).expect("open");
    store
        .insert_session(&session_with(&result, "job-r", 1_234))
        .expect("insert");
    let stored = store.get_session("job-r").unwrap().unwrap();
    let stored_result = stored.result.as_ref().unwrap();

    let live = difflens_core::report::html::render_html(&difflens_core::report::ReportInput {
        id: "job-r",
        result: &result,
        created_at: 1_234,
        lang: difflens_core::report::ReportLang::En,
        version: "1.0.0",
    });
    let from_store =
        difflens_core::report::html::render_html(&difflens_core::report::ReportInput {
            id: "job-r",
            result: stored_result,
            created_at: 1_234,
            lang: difflens_core::report::ReportLang::En,
            version: "1.0.0",
        });
    assert_eq!(live, from_store);
}
