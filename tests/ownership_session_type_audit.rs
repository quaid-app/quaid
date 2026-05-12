#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure"
)]

//! Audit that every public ownership predicate in
//! `src/core/vault_sync/ownership.rs` accepts `daemon`, `serve_host`,
//! AND legacy `serve` rows as live owners. Spec backing:
//! `openspec/changes/daemon-and-http-transport/specs/vault-sync/spec.md`
//! "Daemon and serve_host are the watcher and supervised-duty owners"
//! and the MODIFIED "Live-serve coordination" requirement.

#[path = "common/vault_sync_fixtures.rs"]
mod fixtures;

use fixtures::{insert_collection, open_test_db};

use rusqlite::params;

use quaid::core::vault_sync::{ensure_no_live_serve_owner, live_collection_owner, VaultSyncError};

fn insert_owner_row(
    conn: &rusqlite::Connection,
    session_id: &str,
    pid: i64,
    host: &str,
    session_type: &str,
    collection_id: i64,
) {
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![session_id, pid, host, session_type],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id)
         VALUES (?1, ?2)",
        params![collection_id, session_id],
    )
    .unwrap();
}

#[test]
fn live_collection_owner_recognizes_daemon() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());

    insert_owner_row(&conn, "d-1", 100, "h1", "daemon", collection_id);

    let owner = live_collection_owner(&conn, collection_id)
        .unwrap()
        .unwrap();
    assert_eq!(owner.session_type, "daemon");
    assert_eq!(owner.session_id, "d-1");
    assert_eq!(owner.pid, 100);
}

#[test]
fn live_collection_owner_recognizes_serve_host() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());

    insert_owner_row(&conn, "sh-1", 200, "h2", "serve_host", collection_id);

    let owner = live_collection_owner(&conn, collection_id)
        .unwrap()
        .unwrap();
    assert_eq!(owner.session_type, "serve_host");
    assert_eq!(owner.pid, 200);
}

#[test]
fn live_collection_owner_recognizes_legacy_serve_for_partial_rollback_safety() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());

    insert_owner_row(&conn, "s-1", 300, "h3", "serve", collection_id);

    let owner = live_collection_owner(&conn, collection_id)
        .unwrap()
        .unwrap();
    assert_eq!(owner.session_type, "serve");
}

#[test]
fn live_collection_owner_ignores_cli_session_as_owner() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());

    insert_owner_row(&conn, "cli-1", 400, "h4", "cli", collection_id);

    let owner = live_collection_owner(&conn, collection_id).unwrap();
    assert!(
        owner.is_none(),
        "cli session_type must not appear as a collection owner"
    );
}

#[test]
fn ensure_no_live_serve_owner_returns_runtime_owns_collection_error_for_daemon() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    insert_owner_row(&conn, "d-x", 1234, "host-d", "daemon", collection_id);

    let err = ensure_no_live_serve_owner(&conn, collection_id).unwrap_err();
    match err {
        VaultSyncError::RuntimeOwnsCollectionError {
            owner_session_type,
            owner_pid,
            owner_host,
            ..
        } => {
            assert_eq!(owner_session_type, "daemon");
            assert_eq!(owner_pid, 1234);
            assert_eq!(owner_host, "host-d");
        }
        other => panic!("expected RuntimeOwnsCollectionError, got: {other:?}"),
    }
}

#[test]
fn ensure_no_live_serve_owner_returns_runtime_owns_collection_error_for_serve_host() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    insert_owner_row(&conn, "sh-x", 5678, "host-sh", "serve_host", collection_id);

    let err = ensure_no_live_serve_owner(&conn, collection_id).unwrap_err();
    match err {
        VaultSyncError::RuntimeOwnsCollectionError {
            owner_session_type, ..
        } => {
            assert_eq!(owner_session_type, "serve_host");
        }
        other => panic!("expected RuntimeOwnsCollectionError, got: {other:?}"),
    }
}
