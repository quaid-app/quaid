#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Public has_db_only_state branches: links, assertions, raw_data, contradictions, knowledge_gaps.

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::reconciler::*;

#[test]
fn has_db_only_state_returns_true_for_programmatic_links_branch() {
    let conn = open_test_db();
    conn.execute(
        "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
        [],
    )
    .unwrap();

    let page_a = insert_page(&conn, 1, "notes/a");
    let page_b = insert_page(&conn, 1, "notes/b");

    insert_programmatic_link(&conn, page_a, page_b);
    assert!(has_db_only_state(&conn, page_a).unwrap());
}

#[test]
fn has_db_only_state_returns_true_for_non_import_assertions_branch() {
    let conn = open_test_db();
    conn.execute(
        "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
        [],
    )
    .unwrap();
    let page_a = insert_page(&conn, 1, "notes/a");

    conn.execute(
        "INSERT INTO assertions (page_id, subject, predicate, object, asserted_by)
         VALUES (?1, 'A', 'knows', 'B', 'manual')",
        rusqlite::params![page_a],
    )
    .unwrap();
    assert!(has_db_only_state(&conn, page_a).unwrap());
}

#[test]
fn has_db_only_state_returns_true_for_raw_data_branch() {
    let conn = open_test_db();
    conn.execute(
        "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
        [],
    )
    .unwrap();
    let page_a = insert_page(&conn, 1, "notes/a");

    conn.execute(
        "INSERT INTO raw_data (page_id, source, data) VALUES (?1, 'api', '{}')",
        rusqlite::params![page_a],
    )
    .unwrap();
    assert!(has_db_only_state(&conn, page_a).unwrap());
}

#[test]
fn has_db_only_state_returns_true_for_contradictions_branch() {
    let conn = open_test_db();
    conn.execute(
        "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
        [],
    )
    .unwrap();
    let page_a = insert_page(&conn, 1, "notes/a");
    let page_b = insert_page(&conn, 1, "notes/b");

    conn.execute(
        "INSERT INTO contradictions (page_id, other_page_id, type, description)
         VALUES (?1, ?2, 'assertion_conflict', 'conflict')",
        rusqlite::params![page_a, page_b],
    )
    .unwrap();
    assert!(has_db_only_state(&conn, page_a).unwrap());
}

#[test]
fn has_db_only_state_returns_true_for_knowledge_gaps_branch() {
    let conn = open_test_db();
    conn.execute(
        "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
        [],
    )
    .unwrap();
    let page_a = insert_page(&conn, 1, "notes/a");

    conn.execute(
        "INSERT INTO knowledge_gaps (page_id, query_hash, context)
          VALUES (?1, 'gap-hash', 'context')",
        rusqlite::params![page_a],
    )
    .unwrap();
    assert!(has_db_only_state(&conn, page_a).unwrap());
}
