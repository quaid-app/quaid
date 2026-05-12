#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    unused_imports,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites; unused_imports because the broad import header is shared across all vault_sync_*.rs files but each only consumes a subset"
)]

//! Owner-lease and session ack/handshake tests.
//!
//! Migrated verbatim from `src/core/vault_sync.rs::tests` (the pre-extraction
//! inline `mod tests` block). Test bodies are unchanged; only `use` paths were
//! rewritten to the public crate path. White-box tests that touch private
//! items remain inline in `src/core/vault_sync.rs`.

#[path = "common/vault_sync_fixtures.rs"]
mod fixtures;

use fixtures::*;

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use quaid::core::collections::{Collection, CollectionState};
use quaid::core::db;
#[cfg(unix)]
use quaid::core::file_state;
use quaid::core::fs_safety;
use quaid::core::markdown;
use quaid::core::raw_imports;
use quaid::core::vault_sync::*;

#[test]
fn mark_collection_restoring_uses_collection_owners_and_clears_ack_residue() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    // Use a high explicit collection id so this test cannot collide with
    // parallel in-memory tests that share the process-global supervisor registry.
    let collection_id = insert_collection_with_id(&conn, 50_001, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-owner', 1, 'host')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-spoof', 2, 'host')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-owner')",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET active_lease_session_id = 'serve-spoof',
             reload_generation = 4,
             watcher_released_session_id = 'serve-spoof',
             watcher_released_generation = 3,
             watcher_released_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let (collection, expected_session_id, generation) =
        mark_collection_restoring_for_handshake(&conn, collection_id).unwrap();

    assert_eq!(expected_session_id, "serve-owner");
    assert_eq!(generation, 5);
    assert_eq!(collection.state, CollectionState::Restoring);
    assert!(collection.watcher_released_session_id.is_none());
    assert!(collection.watcher_released_generation.is_none());
    assert!(collection.watcher_released_at.is_none());
}

// design.md §404-408: mark_collection_restoring_for_handshake must use
// live_collection_owner (session_type='serve') so a live CLI lease in
// collection_owners is never treated as the expected serve supervisor.

#[test]
fn mark_collection_restoring_rejects_cli_session_as_handshake_owner() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    // Insert a CLI-type session directly into serve_sessions.
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type)
         VALUES ('cli-lease', 1, 'host', 'cli')",
        [],
    )
    .unwrap();
    // Force the CLI session as the collection owner (bypasses acquire_owner_lease
    // type gate to exercise the production handshake path directly).
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'cli-lease')",
        [collection_id],
    )
    .unwrap();

    let err = mark_collection_restoring_for_handshake(&conn, collection_id).unwrap_err();

    // live_collection_owner finds no serve-type owner → RuntimeOwnsCollectionError,
    // NOT a timeout waiting for an ack only a serve supervisor can emit.
    assert!(
        err.to_string().contains("RuntimeOwnsCollectionError"),
        "expected RuntimeOwnsCollectionError but got: {err}"
    );
}

// Source-seam invariant: the production handshake paths must use
// live_collection_owner (typed) rather than owner_session_id + session_is_live
// (untyped).  This guards against regressions that re-open the CLI-as-owner hole.

#[test]
fn wait_for_exact_ack_short_circuits_when_live_owner_disappears() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-1', 1, 'host')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-1')",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections SET state = 'restoring', reload_generation = 2 WHERE id = ?1",
        [collection_id],
    )
    .unwrap();
    unregister_session(&conn, "serve-1").unwrap();

    let started = Instant::now();
    let error = wait_for_exact_ack(&conn, collection_id, "serve-1", 2).unwrap_err();

    assert!(matches!(
        error,
        VaultSyncError::Restore(RestoreError::ServeDiedDuringHandshake { .. })
    ));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "owner-loss path must short-circuit instead of waiting for the full handshake timeout"
    );
}

#[test]
fn acquire_owner_lease_refuses_live_foreign_owner_and_preserves_existing_claim() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-owner', 1, 'host')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('cli-owner', 2, 'host')",
        [],
    )
    .unwrap();
    acquire_owner_lease(&conn, collection_id, "serve-owner").unwrap();

    let error = acquire_owner_lease(&conn, collection_id, "cli-owner").unwrap_err();

    assert!(error.to_string().contains("RuntimeOwnsCollectionError"));
    assert_eq!(
        owner_session_id(&conn, collection_id).unwrap().as_deref(),
        Some("serve-owner")
    );
}

#[test]
fn acquire_owner_lease_allows_same_session_reentrant_claim_and_keeps_single_row() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('cli-owner', 2, 'host')",
        [],
    )
    .unwrap();

    acquire_owner_lease(&conn, collection_id, "cli-owner").unwrap();
    acquire_owner_lease(&conn, collection_id, "cli-owner").unwrap();

    let row: (Option<String>, i64, Option<String>) = conn
        .query_row(
            "SELECT active_lease_session_id,
                    (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1),
                    (SELECT session_id FROM collection_owners WHERE collection_id = ?1)
             FROM collections
             WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0.as_deref(), Some("cli-owner"));
    assert_eq!(row.1, 1);
    assert_eq!(row.2.as_deref(), Some("cli-owner"));
    assert_eq!(
        owner_session_id(&conn, collection_id).unwrap().as_deref(),
        Some("cli-owner")
    );
}

#[test]
fn acquire_owner_lease_reclaims_stale_owner_residue_and_updates_mirror_column() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('stale-owner', 1, 'host', datetime('now', '-120 seconds'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('cli-owner', 2, 'host')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'stale-owner')",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections SET active_lease_session_id = 'stale-owner' WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    acquire_owner_lease(&conn, collection_id, "cli-owner").unwrap();

    let row: (Option<String>, i64) = conn
        .query_row(
            "SELECT active_lease_session_id,
                    (SELECT COUNT(*) FROM collection_owners WHERE session_id = 'stale-owner')
             FROM collections
             WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row.0.as_deref(), Some("cli-owner"));
    assert_eq!(row.1, 0);
    assert_eq!(
        owner_session_id(&conn, collection_id).unwrap().as_deref(),
        Some("cli-owner")
    );
}

#[test]
fn unregister_session_clears_ownership_mirror_columns() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-1', 1, 'host')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-1')",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET active_lease_session_id = 'serve-1',
             restore_lease_session_id = 'serve-1'
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    unregister_session(&conn, "serve-1").unwrap();

    let row: (Option<String>, Option<String>, i64, i64) = conn
        .query_row(
            "SELECT active_lease_session_id,
                    restore_lease_session_id,
                    (SELECT COUNT(*) FROM collection_owners WHERE session_id = 'serve-1'),
                    (SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'serve-1')
             FROM collections
             WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert!(row.0.is_none());
    assert!(row.1.is_none());
    assert_eq!(row.2, 0);
    assert_eq!(row.3, 0);
}

#[test]
fn write_supervisor_ack_rejects_foreign_stale_and_replayed_acknowledgements() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-1', 1, 'host')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-1')",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-2', 2, 'host')",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections SET state = 'restoring', reload_generation = 2 WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    assert!(!write_supervisor_ack_if_needed(&conn, collection_id, "serve-2", 2).unwrap());
    assert!(!write_supervisor_ack_if_needed(&conn, collection_id, "serve-1", 1).unwrap());
    assert!(write_supervisor_ack_if_needed(&conn, collection_id, "serve-1", 2).unwrap());
    assert!(!write_supervisor_ack_if_needed(&conn, collection_id, "serve-1", 2).unwrap());

    let ack: (Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT watcher_released_session_id, watcher_released_generation
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(ack.0.as_deref(), Some("serve-1"));
    assert_eq!(ack.1, Some(2));
}

#[test]
fn wait_for_exact_ack_reports_when_serve_ownership_changes_mid_handshake() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('serve-1', 1, 'host', datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('serve-2', 2, 'host', datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-2')",
        [collection_id],
    )
    .unwrap();

    match wait_for_exact_ack(&conn, collection_id, "serve-1", 2) {
        Err(VaultSyncError::Restore(RestoreError::ServeOwnershipChanged {
            collection_name,
            expected_session_id,
            actual_session_id,
        })) => {
            assert_eq!(collection_name, "work");
            assert_eq!(expected_session_id, "serve-1");
            assert_eq!(actual_session_id, "serve-2");
        }
        other => panic!("expected ServeOwnershipChangedError, got {other:?}"),
    }
}

#[test]
fn live_collection_owner_ignores_stale_heartbeat_rows_older_than_fifteen_seconds() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('serve-stale', 77, 'host', datetime('now', '-16 seconds'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-stale')",
        [collection_id],
    )
    .unwrap();

    assert!(live_collection_owner(&conn, collection_id)
        .unwrap()
        .is_none());
}

#[test]
fn ensure_no_live_serve_owner_for_root_path_reports_same_root_alias_owner() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let temp_canonical = fs::canonicalize(temp.path()).unwrap();
    let collection_id = insert_collection(&conn, "work", &temp_canonical);
    insert_collection(&conn, "alias", &temp_canonical);
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('serve-live', 77, 'batch3-host', datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-live')",
        [collection_id],
    )
    .unwrap();

    let error =
        ensure_no_live_serve_owner_for_root_path(&conn, &temp_canonical.display().to_string())
            .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("RuntimeOwnsCollectionError"));
    assert!(text.contains("collection=work"));
    assert!(text.contains("owner_pid=77"));
    assert!(text.contains("owner_host=batch3-host"));
}

#[test]
fn ensure_no_live_serve_owner_for_root_path_allows_stale_same_root_owner_residue() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    insert_collection(&conn, "work", temp.path());
    let alias_id = insert_collection(&conn, "alias", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('serve-stale', 77, 'batch3-host', datetime('now', '-120 seconds'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-stale')",
        [alias_id],
    )
    .unwrap();

    ensure_no_live_serve_owner_for_root_path(&conn, &temp.path().display().to_string()).unwrap();
}

#[test]
fn ensure_no_live_serve_owner_for_root_path_ignores_cli_session() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at, session_type)
         VALUES ('cli-lease', 99, 'cli-host', datetime('now'), 'cli')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'cli-lease')",
        [collection_id],
    )
    .unwrap();

    // A live CLI-type lease must not trigger a RuntimeOwnsCollectionError.
    ensure_no_live_serve_owner_for_root_path(&conn, &temp.path().display().to_string()).unwrap();
}
