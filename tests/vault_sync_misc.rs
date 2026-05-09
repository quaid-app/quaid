#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! List-collections views, ensure_unix_platform, embedding queue, ensure_collection_write_allowed.
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

#[test]
fn drain_embedding_queue_marks_failed_jobs_and_retries_after_backoff() {
    let conn = open_test_db();
    let page_id = insert_page_with_raw_import(
        &conn,
        1,
        "notes/retry",
        &Uuid::now_v7().to_string(),
        "Retry candidate truth.",
        b"---\ntitle: Retry\ntype: note\n---\nRetry candidate truth.\n",
        "notes/retry.md",
    );
    quaid::core::raw_imports::enqueue_embedding_job(&conn, page_id).unwrap();
    conn.execute(
        "UPDATE embedding_models SET vec_table = 'bad-table' WHERE active = 1",
        [],
    )
    .unwrap();

    drain_embedding_queue(&conn).unwrap();

    let failed_row: (String, i64, Option<String>) = conn
        .query_row(
            "SELECT job_state, attempt_count, last_error
             FROM embedding_jobs
             WHERE page_id = ?1",
            [page_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(failed_row.0, "failed");
    assert_eq!(failed_row.1, 1);
    assert!(failed_row
        .2
        .as_deref()
        .is_some_and(|message| message.contains("unsafe vec table name")));

    drain_embedding_queue(&conn).unwrap();
    let attempt_count_after_immediate_retry: i64 = conn
        .query_row(
            "SELECT attempt_count FROM embedding_jobs WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(attempt_count_after_immediate_retry, 1);

    conn.execute(
        "UPDATE embedding_jobs
         SET started_at = datetime('now', '-2 seconds')
         WHERE page_id = ?1",
        [page_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE embedding_models
         SET vec_table = 'page_embeddings_vec_384'
         WHERE active = 1",
        [],
    )
    .unwrap();

    drain_embedding_queue(&conn).unwrap();

    let remaining_jobs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embedding_jobs WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_jobs, 0);
}

#[test]
fn drain_embedding_queue_leaves_five_attempt_jobs_failed_without_reclaiming() {
    let conn = open_test_db();
    let page_id = insert_page_with_raw_import(
        &conn,
        1,
        "notes/permanent-failure",
        &Uuid::now_v7().to_string(),
        "Permanent failure truth.",
        b"---\ntitle: Permanent Failure\ntype: note\n---\nPermanent failure truth.\n",
        "notes/permanent-failure.md",
    );
    conn.execute(
        "INSERT INTO embedding_jobs (page_id, job_state, attempt_count, last_error, started_at)
         VALUES (?1, 'failed', 5, 'still broken', '2026-04-28T00:00:00Z')",
        [page_id],
    )
    .unwrap();

    drain_embedding_queue(&conn).unwrap();

    let row: (String, i64, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT job_state, attempt_count, last_error, started_at
             FROM embedding_jobs
             WHERE page_id = ?1",
            [page_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row.0, "failed");
    assert_eq!(row.1, 5);
    assert_eq!(row.2.as_deref(), Some("still broken"));
    assert_eq!(row.3.as_deref(), Some("2026-04-28T00:00:00Z"));
}

#[test]
fn list_memory_collections_counts_pending_and_running_separately_from_failed_jobs() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    let pending_page_id = insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/pending",
        &Uuid::now_v7().to_string(),
        "Pending truth.",
        b"---\ntitle: Pending\ntype: note\n---\nPending truth.\n",
        "notes/pending.md",
    );
    let running_page_id = insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/running",
        &Uuid::now_v7().to_string(),
        "Running truth.",
        b"---\ntitle: Running\ntype: note\n---\nRunning truth.\n",
        "notes/running.md",
    );
    let failed_page_id = insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/failed",
        &Uuid::now_v7().to_string(),
        "Failed truth.",
        b"---\ntitle: Failed\ntype: note\n---\nFailed truth.\n",
        "notes/failed.md",
    );
    conn.execute(
        "INSERT INTO embedding_jobs (page_id, job_state) VALUES (?1, 'pending')",
        [pending_page_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO embedding_jobs (page_id, job_state, started_at)
         VALUES (?1, 'running', '2026-04-28T00:00:00Z')",
        [running_page_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO embedding_jobs (page_id, job_state, attempt_count, last_error)
         VALUES (?1, 'failed', 5, 'boom')",
        [failed_page_id],
    )
    .unwrap();

    let view = list_memory_collections(&conn)
        .unwrap()
        .into_iter()
        .find(|view| view.name == "work")
        .unwrap();
    assert_eq!(view.embedding_queue_depth, 2);
    assert_eq!(view.failing_jobs, 1);
}

#[test]
fn list_memory_collections_only_marks_restore_in_progress_after_release_ack() {
    let conn = open_test_db();
    let active_id = insert_collection(&conn, "active", Path::new("vault-active"));
    let restoring_pending_id = insert_collection(
        &conn,
        "restoring-pending",
        Path::new("vault-restoring-pending"),
    );
    let restoring_live_id =
        insert_collection(&conn, "restoring-live", Path::new("vault-restoring-live"));
    conn.execute(
        "UPDATE collections
         SET state = 'active',
             restore_command_id = 'restore-active',
             watcher_released_at = '2026-04-25T00:00:00Z'
         WHERE id = ?1",
        [active_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             restore_command_id = 'restore-pending',
             watcher_released_at = NULL
         WHERE id = ?1",
        [restoring_pending_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             restore_command_id = 'restore-live',
             watcher_released_at = '2026-04-25T00:00:00Z'
         WHERE id = ?1",
        [restoring_live_id],
    )
    .unwrap();

    let views = list_memory_collections(&conn).unwrap();

    let active = views.iter().find(|view| view.name == "active").unwrap();
    assert_eq!(active.state, "active");
    assert!(!active.restore_in_progress);

    let restoring_pending = views
        .iter()
        .find(|view| view.name == "restoring-pending")
        .unwrap();
    assert_eq!(restoring_pending.state, "restoring");
    assert!(!restoring_pending.restore_in_progress);

    let restoring_live = views
        .iter()
        .find(|view| view.name == "restoring-live")
        .unwrap();
    assert_eq!(restoring_live.state, "restoring");
    assert!(restoring_live.restore_in_progress);
}

#[test]
fn load_collection_by_id_round_trips_optional_restore_metadata() {
    let conn = open_test_db();
    let collection_id = insert_collection(&conn, "work", Path::new("vault"));
    conn.execute(
        "UPDATE collections
         SET active_lease_session_id = 'serve-1',
             restore_command_id = 'restore-1',
             restore_lease_session_id = 'cli-lease',
             reload_generation = 9,
             watcher_released_session_id = 'serve-1',
             watcher_released_generation = 8,
             watcher_released_at = '2026-04-28T00:00:00Z',
             pending_command_heartbeat_at = '2026-04-28T00:01:00Z',
             pending_root_path = 'D:\\restored',
             pending_restore_manifest = '{\"entries\":[]}',
             restore_command_pid = 42,
             restore_command_host = 'host-a',
             integrity_failed_at = '2026-04-28T00:02:00Z',
             pending_manifest_incomplete_at = '2026-04-28T00:03:00Z',
             reconcile_halted_at = '2026-04-28T00:04:00Z',
             reconcile_halt_reason = 'duplicate_uuid'
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();

    assert_eq!(
        collection.active_lease_session_id.as_deref(),
        Some("serve-1")
    );
    assert_eq!(collection.restore_command_id.as_deref(), Some("restore-1"));
    assert_eq!(
        collection.restore_lease_session_id.as_deref(),
        Some("cli-lease")
    );
    assert_eq!(collection.reload_generation, 9);
    assert_eq!(
        collection.watcher_released_session_id.as_deref(),
        Some("serve-1")
    );
    assert_eq!(collection.watcher_released_generation, Some(8));
    assert_eq!(
        collection.pending_root_path.as_deref(),
        Some("D:\\restored")
    );
    assert_eq!(
        collection.pending_restore_manifest.as_deref(),
        Some("{\"entries\":[]}")
    );
    assert_eq!(collection.restore_command_pid, Some(42));
    assert_eq!(collection.restore_command_host.as_deref(), Some("host-a"));
    assert_eq!(
        collection.reconcile_halt_reason.as_deref(),
        Some("duplicate_uuid")
    );
}

#[test]
fn load_collection_by_id_rejects_invalid_collection_state() {
    let conn = open_test_db();
    let collection_id = insert_collection(&conn, "work", Path::new("vault"));
    conn.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    conn.execute(
        "UPDATE collections SET state = 'bogus' WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let error = load_collection_by_id(&conn, collection_id)
        .unwrap_err()
        .to_string();

    assert!(error.contains("invalid collection state"));
    assert!(error.contains("bogus"));
}

#[cfg(not(unix))]
#[test]
fn ensure_unix_platform_fails_closed_on_windows() {
    let error = ensure_unix_platform("quaid collection sync")
        .unwrap_err()
        .to_string();
    assert!(error.contains("UnsupportedPlatformError"));
    assert!(error.contains("quaid collection sync"));
}

#[test]
fn list_memory_collections_reports_integrity_blocked_variants() {
    let conn = open_test_db();
    let manifest_tamper = insert_collection(&conn, "manifest-tamper", Path::new("vault-a"));
    let manifest_retry = insert_collection(&conn, "manifest-retry", Path::new("vault-b"));
    let duplicate_uuid = insert_collection(&conn, "duplicate-uuid", Path::new("vault-c"));
    let trivial = insert_collection(&conn, "trivial", Path::new("vault-d"));
    let unknown = insert_collection(&conn, "unknown", Path::new("vault-e"));
    conn.execute(
        "UPDATE collections
         SET integrity_failed_at = '2026-04-28T00:00:00Z'
         WHERE id = ?1",
        [manifest_tamper],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET pending_manifest_incomplete_at = datetime('now', '-7200 seconds')
         WHERE id = ?1",
        [manifest_retry],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET reconcile_halted_at = '2026-04-28T00:00:00Z',
             reconcile_halt_reason = 'duplicate_uuid'
         WHERE id = ?1",
        [duplicate_uuid],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET reconcile_halted_at = '2026-04-28T00:00:00Z',
             reconcile_halt_reason = 'unresolvable_trivial_content'
         WHERE id = ?1",
        [trivial],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections
         SET reconcile_halted_at = '2026-04-28T00:00:00Z',
             reconcile_halt_reason = 'mystery'
         WHERE id = ?1",
        [unknown],
    )
    .unwrap();

    let views = list_memory_collections(&conn).unwrap();

    assert_eq!(
        views
            .iter()
            .find(|view| view.name == "manifest-tamper")
            .unwrap()
            .integrity_blocked
            .as_deref(),
        Some("manifest_tampering")
    );
    assert_eq!(
        views
            .iter()
            .find(|view| view.name == "manifest-retry")
            .unwrap()
            .integrity_blocked
            .as_deref(),
        Some("manifest_incomplete_escalated")
    );
    assert_eq!(
        views
            .iter()
            .find(|view| view.name == "duplicate-uuid")
            .unwrap()
            .integrity_blocked
            .as_deref(),
        Some("duplicate_uuid")
    );
    assert_eq!(
        views
            .iter()
            .find(|view| view.name == "trivial")
            .unwrap()
            .integrity_blocked
            .as_deref(),
        Some("unresolvable_trivial_content")
    );
    assert!(views
        .iter()
        .find(|view| view.name == "unknown")
        .unwrap()
        .integrity_blocked
        .is_none());
}

#[test]
fn ensure_collection_write_allowed_refuses_on_restoring_or_needs_full_sync() {
    let conn = open_test_db();
    let collection_id = insert_collection(&conn, "work", Path::new("vault"));
    conn.execute(
        "UPDATE collections SET state = 'restoring', needs_full_sync = 1 WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let error = ensure_collection_write_allowed(&conn, collection_id).unwrap_err();
    assert!(error.to_string().contains("CollectionRestoringError"));
    assert!(error.to_string().contains("needs_full_sync=true"));
}

#[test]
fn ensure_collection_write_allowed_refuses_when_only_needs_full_sync_is_set() {
    let conn = open_test_db();
    let collection_id = insert_collection(&conn, "work", Path::new("vault"));
    conn.execute(
        "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let error = ensure_collection_write_allowed(&conn, collection_id).unwrap_err();
    assert!(error.to_string().contains("CollectionRestoringError"));
    assert!(error.to_string().contains("needs_full_sync=true"));
}
