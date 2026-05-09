#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! `start_serve_runtime` startup, restart, and orchestration tests.
//!
//! Migrated verbatim from `src/core/vault_sync.rs::tests` (the pre-extraction
//! inline `mod tests` block). Test bodies are unchanged; only `use` paths were
//! rewritten to the public crate path. White-box tests that touch private
//! items remain inline in `src/core/vault_sync.rs`.

#[path = "common/vault_sync_fixtures.rs"]
mod fixtures;

use fixtures::*;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use quaid::core::collections::{Collection, CollectionState};
use quaid::core::db;
use quaid::core::fs_safety;
use quaid::core::markdown;
use quaid::core::raw_imports;
#[cfg(unix)]
use quaid::core::file_state;
use quaid::core::vault_sync::*;

#[cfg(unix)]
#[test]
fn start_serve_runtime_watcher_reconciles_external_edit_after_debounce() {
    let (_dir, db_path, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    write_restore_file(
        root.path(),
        "notes/a.md",
        b"---\ntitle: A\ntype: note\n---\nOriginal body.\n",
    );
    sync_collection(&conn, "work").unwrap();
    drop(conn);

    let runtime = start_serve_runtime(db_path.clone()).unwrap();

    write_restore_file(
        root.path(),
        "notes/a.md",
        b"---\ntitle: A\ntype: note\n---\nUpdated by watcher.\n",
    );

    let compiled_truth = wait_for_collection_update(
        &db_path,
        collection_id,
        Duration::from_secs(8),
        |verify, collection_id| {
            verify
                .query_row(
                    "SELECT compiled_truth
                     FROM pages
                     WHERE collection_id = ?1 AND slug = 'notes/a'",
                    [collection_id],
                    |row| row.get::<_, String>(0),
                )
                .ok()
                .and_then(|compiled_truth| {
                    compiled_truth
                        .contains("Updated by watcher.")
                        .then_some(compiled_truth)
                })
        },
    );
    assert!(compiled_truth.contains("Updated by watcher."));

    drop(runtime);
}

#[test]
fn start_serve_runtime_logs_scheduled_maintenance_failures() {
    let source = production_vault_sync_source();
    let start = source.find("pub fn start_serve_runtime(").unwrap();
    let end = source[start..]
        .find("pub fn begin_restore(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("WARN: scheduled_full_hash_audit_failed")
            && snippet.contains("if let Err(error) = run_full_hash_audit_pass(&conn, &session_id_for_thread)")
            && snippet.contains("WARN: janitor_sweep_failed")
            && snippet.contains("if let Err(error) = janitor::run_tick(&conn)")
            && snippet.contains("WARN: raw_import_ttl_sweep_failed")
            && snippet.contains("if let Err(error) = sweep_raw_import_ttl(&conn)"),
        "serve loop must log janitor, scheduled audit, and TTL sweep failures instead of discarding them silently"
    );
}

#[test]
fn start_serve_runtime_spawns_extraction_worker() {
    let source = production_vault_sync_source();
    let start = source.find("pub fn start_serve_runtime(").unwrap();
    let end = source[start..]
        .find("pub fn begin_restore(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("run_extraction_worker"),
        "serve runtime must spawn the extraction worker so queued jobs are drained: {snippet}"
    );
    assert!(
        snippet.contains("extractor_handle: Some("),
        "serve runtime must store the extraction worker handle on ServeRuntime for shutdown: {snippet}"
    );
}

#[test]
fn run_extraction_worker_logs_failures_and_honors_stop_signal() {
    let source = production_vault_sync_source();
    let start = source
        .find("fn run_extraction_worker(")
        .expect("run_extraction_worker function must exist for the worker thread");
    let end = source[start..]
        .find("\nfn ")
        .or_else(|| source[start..].find("\npub fn "))
        .map(|offset| start + offset)
        .unwrap_or(source.len());
    let snippet = &source[start..end];

    assert!(
        snippet.contains("Worker::new(&conn, LazySlmRunner::new(), ResolvingFactWriter)"),
        "worker thread must construct the production extractor::Worker: {snippet}"
    );
    assert!(
        snippet.contains("worker.run_once()"),
        "worker thread must drive the queue via run_once so the stop signal is honored between polls: {snippet}"
    );
    assert!(
        snippet.contains("stop.load(Ordering::SeqCst)"),
        "worker thread loop must honor the ServeRuntime stop signal: {snippet}"
    );
    assert!(
        snippet.contains("WARN: extraction_worker_run_failed"),
        "worker thread must log run failures instead of swallowing them: {snippet}"
    );
    assert!(
        snippet.contains("WARN: extraction_worker_init_failed"),
        "worker thread must log init failures instead of crashing the daemon: {snippet}"
    );
}

#[test]
fn serve_runtime_drop_joins_extraction_worker_handle() {
    let source = production_vault_sync_source();
    let start = source.find("impl Drop for ServeRuntime").unwrap();
    let end = source[start..]
        .find("\n}\n")
        .map(|offset| start + offset + 3)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("self.extractor_handle.take()"),
        "ServeRuntime drop must join the extraction worker handle: {snippet}"
    );
}

#[cfg(unix)]
#[test]
fn start_serve_runtime_leaves_restoring_needs_full_sync_for_overflow_worker() {
    let (_dir, db_path, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             needs_full_sync = 1,
             integrity_failed_at = '2026-04-28T00:00:00Z'
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();
    drop(conn);

    let runtime = start_serve_runtime(db_path.clone()).unwrap();
    thread::sleep(Duration::from_millis(1200));

    let verify = Connection::open(&db_path).unwrap();
    let collection = load_collection_by_id(&verify, collection_id).unwrap();

    assert!(collection.needs_full_sync);
    assert_eq!(collection.state, CollectionState::Restoring);
    drop(runtime);
}

#[cfg(unix)]
#[test]
fn start_serve_runtime_recovers_tx_b_orphan_exactly_once_before_supervisor_ack() {
    let (_dir, db_path, conn) = open_test_db_file();
    let source_root = tempfile::TempDir::new().unwrap();
    let pending_parent = tempfile::TempDir::new().unwrap();
    let pending_root = pending_parent.path().join("restored");
    let collection_id = insert_collection(&conn, "work", source_root.path());
    write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
    let manifest_json = manifest_json_for_directory(&pending_root);
    conn.execute(
        "CREATE TABLE startup_finalize_audit (
             collection_id INTEGER NOT NULL,
             cleared_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
         )",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TRIGGER startup_finalize_exactly_once
         AFTER UPDATE ON collections
         WHEN OLD.pending_root_path IS NOT NULL AND NEW.pending_root_path IS NULL
         BEGIN
             INSERT INTO startup_finalize_audit (collection_id) VALUES (NEW.id);
         END",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('stale-owner', 1, 'host', datetime('now', '-16 seconds'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host)
         VALUES ('foreign-live', 2, 'host')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id)
         VALUES (?1, 'stale-owner')",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_restore_manifest = ?3,
             restore_command_id = 'restore-1',
             pending_command_heartbeat_at = datetime('now', '-16 seconds')
         WHERE id = ?1",
        params![
            collection_id,
            pending_root.display().to_string(),
            manifest_json
        ],
    )
    .unwrap();
    drop(conn);

    let runtime = start_serve_runtime(db_path.clone()).unwrap();

    type StartupRecoveryStealsLeaseRow = (
        String,
        String,
        i64,
        Option<String>,
        i64,
        i64,
        i64,
        i64,
        Option<String>,
    );

    let row: StartupRecoveryStealsLeaseRow = wait_for_collection_update(
        &db_path,
        collection_id,
        Duration::from_secs(5),
        |verify, collection_id| {
            verify
                    .query_row(
                        "SELECT state,
                                root_path,
                                needs_full_sync,
                                pending_root_path,
                                (SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'stale-owner'),
                                (SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'foreign-live'),
                                (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2),
                                (SELECT COUNT(*) FROM startup_finalize_audit WHERE collection_id = ?1),
                                watcher_released_session_id
                         FROM collections
                         WHERE id = ?1",
                        params![collection_id, runtime.session_id.as_str()],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                                row.get(5)?,
                                row.get(6)?,
                                row.get(7)?,
                                row.get(8)?,
                            ))
                        },
                    )
                    .ok()
                    .and_then(|row| if row.0 == "active" { Some(row) } else { None })
        },
    );
    assert_eq!(row.0, "active");
    assert_eq!(row.1, pending_root.display().to_string());
    assert_eq!(row.2, 0);
    assert!(row.3.is_none());
    assert_eq!(row.4, 0);
    assert_eq!(row.5, 1);
    assert_eq!(row.6, 1);
    assert_eq!(row.7, 1);
    assert!(row.8.is_none());

    drop(runtime);
}

#[cfg(unix)]
#[test]
fn start_serve_runtime_defers_fresh_restore_heartbeat_and_leaves_collection_blocked() {
    let (_dir, db_path, conn) = open_test_db_file();
    let source_root = tempfile::TempDir::new().unwrap();
    let source_root_canonical = fs::canonicalize(source_root.path()).unwrap();
    let pending_parent = tempfile::TempDir::new().unwrap();
    let pending_root = fs::canonicalize(pending_parent.path())
        .unwrap()
        .join("restored");
    let collection_id = insert_collection(&conn, "work", &source_root_canonical);
    write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
    let manifest_json = manifest_json_for_directory(&pending_root);
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('stale-owner', 1, 'host', datetime('now', '-16 seconds'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id)
         VALUES (?1, 'stale-owner')",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_restore_manifest = ?3,
             restore_command_id = 'restore-1',
             pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![
            collection_id,
            pending_root.display().to_string(),
            manifest_json
        ],
    )
    .unwrap();
    drop(conn);

    let runtime = start_serve_runtime(db_path.clone()).unwrap();
    thread::sleep(Duration::from_millis(500));

    type RecoveryRow = (
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
        i64,
        Option<String>,
        Option<i64>,
        Option<String>,
    );

    let verify = Connection::open(&db_path).unwrap();
    let row: RecoveryRow = verify
        .query_row(
            "SELECT state,
                    root_path,
                    needs_full_sync,
                    pending_root_path,
                    restore_command_id,
                    (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2),
                    watcher_released_session_id,
                    watcher_released_generation,
                    watcher_released_at
             FROM collections
             WHERE id = ?1",
            params![collection_id, runtime.session_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, "restoring");
    assert_eq!(row.1, source_root_canonical.display().to_string());
    assert_eq!(row.2, 0);
    assert_eq!(row.3.as_deref(), Some(pending_root.to_str().unwrap()));
    assert_eq!(row.4.as_deref(), Some("restore-1"));
    assert_eq!(row.5, 1);
    assert!(
        row.6.is_none() && row.7.is_none() && row.8.is_none(),
        "fresh serve must not impersonate the originator by writing the watcher ack triple"
    );

    drop(runtime);
}

#[cfg(unix)]
#[test]
fn start_serve_runtime_bootstraps_recovery_directories_for_existing_collections() {
    #[cfg(target_os = "linux")]
    let _env_lock = env_mutation_lock().lock().unwrap();
    #[cfg(target_os = "linux")]
    let _runtime_root = secure_runtime_root();
    #[cfg(target_os = "linux")]
    let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", _runtime_root.path().to_str().unwrap());
    let (dir, db_path, conn) = open_test_db_file();
    let root_a = tempfile::TempDir::new().unwrap();
    let root_b = tempfile::TempDir::new().unwrap();
    let collection_a = insert_collection(&conn, "work", root_a.path());
    let collection_b = insert_collection(&conn, "notes", root_b.path());
    drop(conn);

    let runtime = start_serve_runtime(db_path).unwrap();

    let recovery_root = dir.path().join("recovery");
    assert!(collection_recovery_dir(&recovery_root, collection_a).is_dir());
    assert!(collection_recovery_dir(&recovery_root, collection_b).is_dir());

    drop(runtime);
}

#[test]
fn start_serve_runtime_refreshes_session_heartbeat_on_five_second_interval() {
    let (_dir, db_path, conn) = open_test_db_file();
    drop(conn);

    let runtime = start_serve_runtime(db_path.clone()).unwrap();
    let conn = Connection::open(&db_path).unwrap();
    let first_heartbeat: String = conn
        .query_row(
            "SELECT heartbeat_at FROM serve_sessions WHERE session_id = ?1",
            [runtime.session_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    drop(conn);

    thread::sleep(Duration::from_millis(6200));

    let conn = Connection::open(&db_path).unwrap();
    let second_heartbeat: String = conn
        .query_row(
            "SELECT heartbeat_at FROM serve_sessions WHERE session_id = ?1",
            [runtime.session_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(first_heartbeat, second_heartbeat);

    drop(runtime);
}
