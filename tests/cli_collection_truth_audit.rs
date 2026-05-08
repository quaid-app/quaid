#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]
#![cfg(unix)]

//! Integration tests for `quaid collection audit` truth-merge behavior.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use rusqlite::params;
use truth_fixtures::*;

#[cfg(unix)]
#[test]
fn collection_audit_reports_reconcile_stats_and_raw_import_gc_cleanup() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-audit-cli.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(root.join("notes")).expect("create notes dir");
    let collection_id = insert_collection(&conn, "work", &root);
    let raw_bytes =
        b"---\nmemory_id: 44444444-4444-7444-8444-444444444444\ntitle: Audit Note\ntype: concept\n---\naudit should keep this row active while pruning expired inactive history\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "44444444-4444-7444-8444-444444444444",
        raw_bytes,
        "notes/a.md",
    );
    let page_id = page_id(&conn, collection_id, "notes/a");
    std::fs::write(root.join("notes").join("a.md"), raw_bytes).expect("write vault note");
    conn.execute(
        "UPDATE file_state
         SET last_full_hash_at = datetime('now', '-8 days')
         WHERE collection_id = ?1",
        [collection_id],
    )
    .expect("age file_state for audit");
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path, created_at)
         VALUES (?1, ?2, 0, ?3, ?4, '2000-01-01T00:00:00Z')",
        params![
            page_id,
            uuid::Uuid::now_v7().to_string(),
            b"old audit bytes".as_slice(),
            "notes/a.md"
        ],
    )
    .expect("seed expired inactive raw import");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["--json", "collection", "audit", "work", "--raw-imports-gc"],
    );

    assert!(
        output.status.success(),
        "collection audit should succeed through the CLI: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["status"].as_str(), Some("ok"));
    assert_eq!(parsed["command"].as_str(), Some("audit"));
    assert_eq!(parsed["collection"].as_str(), Some("work"));
    assert_eq!(parsed["walked"].as_u64(), Some(1));
    assert_eq!(parsed["unchanged"].as_u64(), Some(1));
    assert_eq!(parsed["modified"].as_u64(), Some(0));
    assert_eq!(parsed["new"].as_u64(), Some(0));
    assert_eq!(parsed["missing"].as_u64(), Some(0));
    assert_eq!(parsed["uuid_renamed"].as_u64(), Some(0));
    assert_eq!(parsed["hash_renamed"].as_u64(), Some(0));
    assert_eq!(parsed["raw_imports_deleted"].as_u64(), Some(1));

    let conn = open_test_db(&db_path);
    let (active_rows, total_rows) = raw_import_counts(&conn, page_id);
    assert_eq!(
        active_rows, 1,
        "audit must preserve exactly one active raw_import"
    );
    assert_eq!(
        total_rows, 1,
        "audit GC must prune the expired inactive raw_import"
    );
    let row: (String, i64, Option<String>) = conn
        .query_row(
            "SELECT state, needs_full_sync, last_sync_at
             FROM collections
             WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load audited collection");
    assert_eq!(row.0, "active");
    assert_eq!(row.1, 0);
    assert!(row.2.is_some(), "audit should stamp last_sync_at");
}
