#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests panic on setup failure"
)]

//! `quaid validate` must detect orphaned vec rows (vec0 entries whose backing
//! `page_embeddings` join row is gone) in addition to the pre-existing
//! `stale_vec_rowid` (pe → vec) staleness check — review item #10.

use quaid::commands::embed::run_with_batch;
use quaid::commands::validate::{execute_validate, CheckFlags};
use quaid::core::db;
use rusqlite::Connection;
use uuid::Uuid;

fn open_test_db() -> Connection {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    std::mem::forget(dir);
    conn
}

fn insert_page(conn: &Connection, slug: &str) {
    conn.execute(
        "INSERT INTO pages
             (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, 'concept', ?1, '', ?3, '', '{}', '', '', 1)",
        rusqlite::params![
            slug,
            Uuid::now_v7().to_string(),
            format!("## State\n{slug} has enough content to embed.")
        ],
    )
    .unwrap();
}

fn embeddings_only_flags() -> CheckFlags {
    CheckFlags {
        links: false,
        assertions: false,
        embeddings: true,
    }
}

#[test]
fn validate_detects_orphaned_vec_rows() {
    let conn = open_test_db();
    insert_page(&conn, "notes/orphan-source");
    run_with_batch(&conn, None, true, false, Some(8)).unwrap();

    // A clean store has no orphans.
    let clean = execute_validate(&conn, &embeddings_only_flags()).unwrap();
    assert!(
        !clean
            .violations
            .iter()
            .any(|v| v.violation_type == "vec_orphans"),
        "freshly embedded store must not report vec orphans: {:?}",
        clean.violations
    );

    // Simulate a legacy delete path that dropped the page_embeddings join row
    // but left the vec0 row behind, orphaning the vector.
    conn.execute("DELETE FROM page_embeddings", []).unwrap();

    let report = execute_validate(&conn, &embeddings_only_flags()).unwrap();
    let orphan = report
        .violations
        .iter()
        .find(|v| v.violation_type == "vec_orphans")
        .expect("validate must report vec_orphans for a dangling vec row");
    assert_eq!(orphan.check, "embeddings");
    assert!(
        orphan.details["orphan_count"].as_i64().unwrap() >= 1,
        "orphan_count must be reported: {:?}",
        orphan.details
    );
    assert!(!report.passed, "report with orphans must not pass");
}
