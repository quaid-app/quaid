#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `collection restore-reset` and `collection
//! reconcile-reset` (the public `CollectionAction::RestoreReset` /
//! `ReconcileReset` surfaces).
//!
//! Covers the `--confirm` precondition refusals and the successful clear of
//! restore/integrity/reconcile-halt markers when confirmation is provided.

use quaid::commands::collection::{run, CollectionAction};
use quaid::core::db;

#[path = "common/collection_fixtures.rs"]
mod fixtures;
use fixtures::{insert_collection, open_test_db};

#[test]
fn restore_reset_requires_confirm() {
    let conn = db::open(":memory:").unwrap();
    let error = run(
        &conn,
        CollectionAction::RestoreReset {
            name: "work".to_owned(),
            confirm: false,
        },
        true,
    )
    .unwrap_err();
    assert!(error.to_string().contains("--confirm"));
}

#[test]
fn reconcile_reset_requires_confirm() {
    let conn = db::open(":memory:").unwrap();
    let error = run(
        &conn,
        CollectionAction::ReconcileReset {
            name: "work".to_owned(),
            confirm: false,
        },
        true,
    )
    .unwrap_err();
    assert!(error.to_string().contains("--confirm"));
}

#[test]
fn restore_reset_and_reconcile_reset_succeed_when_confirmed() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = 'D:\\vault\\restored',
             integrity_failed_at = '2026-04-23T00:00:00Z',
             reconcile_halted_at = '2026-04-23T00:05:00Z',
             reconcile_halt_reason = 'duplicate_uuid'
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    run(
        &conn,
        CollectionAction::RestoreReset {
            name: "work".to_owned(),
            confirm: true,
        },
        true,
    )
    .unwrap();
    run(
        &conn,
        CollectionAction::ReconcileReset {
            name: "work".to_owned(),
            confirm: true,
        },
        true,
    )
    .unwrap();

    let row: (String, Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state, pending_root_path, integrity_failed_at, reconcile_halted_at
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row.0, "active");
    assert!(row.1.is_none());
    assert!(row.2.is_none());
    assert!(row.3.is_none());
}
