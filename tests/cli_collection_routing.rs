#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for cross-cutting `CollectionAction` routing through the
//! public `run` entry point and adjacent public surfaces.
//!
//! Covers `put` refusing writes against a read-only collection and the
//! `run`-level `List` / `Info` JSON helpers seeded with a real page,
//! quarantine, embedding job, and ignore-parse error state.

use quaid::commands::collection::{run, CollectionAction};
use quaid::commands::put;
use uuid::Uuid;

#[path = "common/collection_fixtures.rs"]
mod fixtures;
use fixtures::{
    insert_collection, insert_embedding_job, insert_page_with_raw_import, open_test_db,
    quarantine_page,
};

#[test]
fn put_refuses_read_only_collection() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    conn.execute(
        "UPDATE collections SET writable = 0 WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let error = put::put_from_string(
        &conn,
        "work::notes/read-only",
        "---\ntitle: Read Only\ntype: note\n---\nhello\n",
        None,
    )
    .unwrap_err();

    assert!(error.to_string().contains("CollectionReadOnlyError"));
}

#[test]
fn run_routes_list_and_info_json_helpers_for_seeded_collection() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    let page_id = insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/info",
        &Uuid::now_v7().to_string(),
        b"---\ntitle: Info\ntype: note\n---\nbody\n",
        "notes/info.md",
    );
    quarantine_page(&conn, page_id, "2026-04-28T00:00:00Z");
    insert_embedding_job(&conn, page_id, "failed", 2);
    conn.execute(
        "UPDATE collections
         SET integrity_failed_at = '2026-04-28T00:00:00Z',
             ignore_parse_errors = 'line 1 raw=\"[broken\" error=Invalid glob pattern'
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    run(&conn, CollectionAction::List, true).unwrap();
    run(
        &conn,
        CollectionAction::Info {
            name: "work".to_owned(),
        },
        true,
    )
    .unwrap();
}
