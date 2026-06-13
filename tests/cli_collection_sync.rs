#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `quaid collection sync` and `audit` (the public
//! `CollectionAction::Sync` / `Audit` surfaces).
//!
//! Covers the finalize-pending external-handoff path, full-hash audit with
//! optional raw-import GC, and the bare `sync` refusal/no-op paths
//! (active-root precondition, restore-in-progress, pending-finalize,
//! integrity-blocked, and reconcile-halted states).

use std::fs;

use quaid::commands::collection::{run, CollectionAction, CollectionAuditArgs, CollectionSyncArgs};
use quaid::core::db;
use uuid::Uuid;

#[path = "common/collection_fixtures.rs"]
mod fixtures;
use fixtures::{insert_collection, insert_page_with_raw_import, open_test_db_file};

#[cfg(unix)]
#[test]
fn sync_finalize_pending_uses_external_finalize_path() {
    let (_db_dir, conn) = open_test_db_file();
    let temp = tempfile::TempDir::new().unwrap();
    let pending_root = temp.path().join("restored");
    fs::create_dir_all(&pending_root).unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_restore_manifest = '{\"entries\":[]}',
             restore_command_id = 'restore-1',
             pending_command_heartbeat_at = datetime('now', '-120 seconds')
         WHERE id = ?1",
        rusqlite::params![collection_id, pending_root.display().to_string()],
    )
    .unwrap();

    run(
        &conn,
        CollectionAction::Sync(CollectionSyncArgs {
            name: "work".to_owned(),
            remap_root: None,
            finalize_pending: true,
            online: false,
            no_embed: false,
        }),
        true,
    )
    .unwrap();

    let row: (String, String, i64, Option<String>) = conn
        .query_row(
            "SELECT state, root_path, needs_full_sync, pending_root_path
              FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row.0, "active");
    assert_eq!(row.1, pending_root.display().to_string());
    assert_eq!(row.2, 0);
    assert!(row.3.is_none());
}

#[cfg(unix)]
#[test]
fn audit_runs_full_hash_reconcile_and_optional_raw_import_gc() {
    let (_db_dir, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    let relative_path = "notes/a.md";
    let file_path = root.path().join(relative_path);
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    let uuid = Uuid::now_v7().to_string();
    let raw_bytes =
        format!("---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nBody.\n");
    fs::write(&file_path, raw_bytes.as_bytes()).unwrap();
    let page_id = insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        &uuid,
        raw_bytes.as_bytes(),
        relative_path,
    );
    conn.execute(
        "UPDATE file_state
         SET last_full_hash_at = datetime('now', '-8 days')
         WHERE collection_id = ?1",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path, created_at)
         VALUES (?1, ?2, 0, ?3, ?4, '2000-01-01T00:00:00Z')",
        rusqlite::params![page_id, Uuid::now_v7().to_string(), b"old", relative_path],
    )
    .unwrap();

    run(
        &conn,
        CollectionAction::Audit(CollectionAuditArgs {
            name: "work".to_owned(),
            raw_imports_gc: true,
        }),
        true,
    )
    .unwrap();

    let row: (Option<String>, i64) = conn
        .query_row(
            "SELECT last_sync_at,
                    (SELECT COUNT(*)
                     FROM raw_imports
                     WHERE page_id = ?2 AND is_active = 0)
             FROM collections
             WHERE id = ?1",
            rusqlite::params![collection_id, page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(row.0.is_some());
    assert_eq!(row.1, 0);
}

#[cfg(unix)]
#[test]
fn sync_without_flags_requires_active_root_collection() {
    let conn = db::open(":memory:").unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "UPDATE collections SET state = 'detached' WHERE id = ?1",
        [collection_id],
    )
    .unwrap();
    let error = run(
        &conn,
        CollectionAction::Sync(CollectionSyncArgs {
            name: "work".to_owned(),
            remap_root: None,
            finalize_pending: false,
            online: false,
            no_embed: false,
        }),
        true,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("PlainSyncActiveRootRequiredError"));
}

#[cfg(unix)]
#[test]
fn sync_without_flags_refuses_restore_in_progress_state() {
    let conn = db::open(":memory:").unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "UPDATE collections SET state = 'restoring' WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let error = run(
        &conn,
        CollectionAction::Sync(CollectionSyncArgs {
            name: "work".to_owned(),
            remap_root: None,
            finalize_pending: false,
            online: false,
            no_embed: false,
        }),
        true,
    )
    .unwrap_err();

    let state: String = conn
        .query_row(
            "SELECT state FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(error.to_string().contains("RestoreInProgressError"));
    assert_eq!(state, "restoring");
}

#[cfg(unix)]
#[test]
fn sync_without_flags_does_not_finalize_pending_restore_state() {
    let conn = db::open(":memory:").unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    let pending_root = temp.path().join("restored");
    fs::create_dir_all(&pending_root).unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_restore_manifest = '{\"entries\":[]}',
             restore_command_id = 'restore-1',
             pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        rusqlite::params![collection_id, pending_root.display().to_string()],
    )
    .unwrap();

    let error = run(
        &conn,
        CollectionAction::Sync(CollectionSyncArgs {
            name: "work".to_owned(),
            remap_root: None,
            finalize_pending: false,
            online: false,
            no_embed: false,
        }),
        true,
    )
    .unwrap_err();

    let row: (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state, pending_root_path, restore_command_id
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(error.to_string().contains("RestorePendingFinalizeError"));
    assert_eq!(row.0, "restoring");
    assert_eq!(row.1.as_deref(), Some(pending_root.to_str().unwrap()));
    assert_eq!(row.2.as_deref(), Some("restore-1"));
}

#[cfg(unix)]
#[test]
fn sync_without_flags_refuses_restore_integrity_blocked_state() {
    let conn = db::open(":memory:").unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             integrity_failed_at = '2026-04-23T00:00:00Z'
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let error = run(
        &conn,
        CollectionAction::Sync(CollectionSyncArgs {
            name: "work".to_owned(),
            remap_root: None,
            finalize_pending: false,
            online: false,
            no_embed: false,
        }),
        true,
    )
    .unwrap_err();

    let row: (String, Option<String>) = conn
        .query_row(
            "SELECT state, integrity_failed_at FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(error.to_string().contains("RestoreIntegrityBlockedError"));
    assert_eq!(row.0, "restoring");
    assert_eq!(row.1.as_deref(), Some("2026-04-23T00:00:00Z"));
}

#[cfg(unix)]
#[test]
fn sync_without_flags_does_not_clear_integrity_or_reconcile_halt_markers() {
    let conn = db::open(":memory:").unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "UPDATE collections
         SET integrity_failed_at = '2026-04-23T00:00:00Z',
             reconcile_halted_at = '2026-04-23T00:05:00Z',
             reconcile_halt_reason = 'duplicate_uuid'
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let error = run(
        &conn,
        CollectionAction::Sync(CollectionSyncArgs {
            name: "work".to_owned(),
            remap_root: None,
            finalize_pending: false,
            online: false,
            no_embed: false,
        }),
        true,
    )
    .unwrap_err();

    let row: (Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT integrity_failed_at, reconcile_halted_at, reconcile_halt_reason
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(error.to_string().contains("ReconcileHaltedError"));
    assert_eq!(row.0.as_deref(), Some("2026-04-23T00:00:00Z"));
    assert_eq!(row.1.as_deref(), Some("2026-04-23T00:05:00Z"));
    assert_eq!(row.2.as_deref(), Some("duplicate_uuid"));
}
