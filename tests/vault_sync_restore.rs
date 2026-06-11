#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    unused_imports,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites; unused_imports because the broad import header is shared across all vault_sync_*.rs files but each only consumes a subset"
)]

//! `begin_restore` / `finalize_pending_restore` / `restore_reset` pipeline tests.
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
fn build_restore_manifest_for_directory_sorts_paths_and_hashes_contents() {
    let root = tempfile::TempDir::new().unwrap();
    write_restore_file(root.path(), "notes/z.md", b"zeta");
    write_restore_file(root.path(), "notes/a.md", b"alpha");

    let manifest = build_restore_manifest_for_directory(root.path()).unwrap();

    assert_eq!(
        manifest
            .entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect::<Vec<_>>(),
        vec!["notes/a.md", "notes/z.md"]
    );
    assert_eq!(manifest.entries[0].sha256, sha256_hex(b"alpha"));
    assert_eq!(manifest.entries[0].size_bytes, 5);
    assert_eq!(manifest.entries[1].sha256, sha256_hex(b"zeta"));
    assert_eq!(manifest.entries[1].size_bytes, 4);
}

#[test]
fn restore_reset_reports_each_block_reason_before_terminal_reset() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());

    conn.execute(
        "UPDATE collections
         SET pending_manifest_incomplete_at = '2026-04-28T00:00:00Z'
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();
    let manifest_error = restore_reset(&conn, "work").unwrap_err().to_string();
    assert!(manifest_error.contains("RestoreResetBlockedError"));
    assert!(manifest_error.contains("manifest_incomplete_retryable"));

    conn.execute(
        "UPDATE collections
         SET pending_manifest_incomplete_at = NULL,
             state = 'restoring',
             pending_root_path = 'D:\\restored'
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();
    let pending_error = restore_reset(&conn, "work").unwrap_err().to_string();
    assert!(pending_error.contains("RestoreResetBlockedError"));
    assert!(pending_error.contains("pending_finalize"));

    conn.execute(
        "UPDATE collections
         SET pending_root_path = NULL,
             state = 'restoring',
             needs_full_sync = 1
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();
    let progress_error = restore_reset(&conn, "work").unwrap_err().to_string();
    assert!(progress_error.contains("RestoreResetBlockedError"));
    assert!(progress_error.contains("restore_in_progress"));

    conn.execute(
        "UPDATE collections
         SET state = 'active',
             needs_full_sync = 0
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();
    let clean_error = restore_reset(&conn, "work").unwrap_err().to_string();
    assert!(clean_error.contains("RestoreResetBlockedError"));
    assert!(clean_error.contains("no_integrity_failure"));
}

#[test]
fn reconcile_reset_reports_missing_collection() {
    assert!(matches!(
        reconcile_reset(&open_test_db(), "missing"),
        Err(VaultSyncError::CollectionNotFound { name }) if name == "missing"
    ));
}

#[test]
fn restore_reset_can_return_collection_to_detached_and_finalize_covers_remaining_outcomes() {
    let conn = open_test_db();
    let placeholder_id = insert_collection(&conn, "placeholder", Path::new("vault"));
    conn.execute(
        "UPDATE collections
         SET root_path = '',
             state = 'restoring',
             integrity_failed_at = '2026-04-28T00:00:00Z'
         WHERE id = ?1",
        [placeholder_id],
    )
    .unwrap();
    restore_reset(&conn, "placeholder").unwrap();
    let placeholder = load_collection_by_id(&conn, placeholder_id).unwrap();
    assert_eq!(placeholder.state, CollectionState::Detached);
    assert_eq!(placeholder.root_path, "");

    let no_pending_id = insert_collection(&conn, "no-pending", Path::new("vault-no-pending"));
    let no_pending = finalize_pending_restore(
        &conn,
        no_pending_id,
        FinalizeCaller::ExternalFinalize {
            session_id: "serve-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(no_pending, FinalizeOutcome::NoPendingWork);

    let orphan_id = insert_collection(&conn, "orphan", Path::new("vault-orphan"));
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             restore_command_id = 'restore-1',
             pending_root_path = NULL
         WHERE id = ?1",
        [orphan_id],
    )
    .unwrap();
    let orphan = finalize_pending_restore(
        &conn,
        orphan_id,
        FinalizeCaller::ExternalFinalize {
            session_id: "serve-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(orphan, FinalizeOutcome::OrphanRecovered);

    let aborted_id = insert_collection(&conn, "aborted", Path::new("vault-aborted"));
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = 'D:\\missing-restore-root',
             pending_restore_manifest = '{\"entries\":[]}',
             restore_command_id = 'restore-1',
             pending_command_heartbeat_at = datetime('now', '-120 seconds')
         WHERE id = ?1",
        [aborted_id],
    )
    .unwrap();
    let aborted = finalize_pending_restore(
        &conn,
        aborted_id,
        FinalizeCaller::ExternalFinalize {
            session_id: "serve-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(aborted, FinalizeOutcome::Aborted);
}

#[test]
fn finalize_pending_restore_returns_no_pending_work_when_pending_root_path_is_null() {
    let conn = open_test_db();
    let collection_id = insert_collection(&conn, "no-pending-null", Path::new("vault-np"));
    // insert_collection sets state='active' and pending_root_path defaults to NULL
    let outcome = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::ExternalFinalize {
            session_id: "serve-null-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(outcome, FinalizeOutcome::NoPendingWork);
}

#[test]
fn finalize_pending_restore_returns_orphan_recovered_when_restoring_with_null_pending_root() {
    let conn = open_test_db();
    let collection_id = insert_collection(&conn, "orphan-null", Path::new("vault-orphan-null"));
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             restore_command_id = 'restore-orphan-null',
             pending_root_path = NULL
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();
    let outcome = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::ExternalFinalize {
            session_id: "serve-null-2".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(outcome, FinalizeOutcome::OrphanRecovered);
    let reverted = load_collection_by_id(&conn, collection_id).unwrap();
    assert_ne!(reverted.state, CollectionState::Restoring);
}

#[test]
fn begin_restore_rejects_non_empty_target() {
    let conn = open_test_db();
    let source_root = tempfile::TempDir::new().unwrap();
    let target_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", source_root.path());
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
        "notes/a.md",
    );
    fs::write(target_root.path().join("occupied.txt"), b"x").unwrap();

    let error = begin_restore(&conn, "work", target_root.path(), false).unwrap_err();
    assert!(error.to_string().contains("RestoreNonEmptyTargetError"));
}

#[cfg(not(unix))]
#[test]
fn begin_restore_on_windows_releases_cli_lease_after_inline_attach_failure() {
    let (_db_dir, _db_path, conn) = open_test_db_file();
    let source_root = tempfile::TempDir::new().unwrap();
    let target_parent = tempfile::TempDir::new().unwrap();
    let target_root = target_parent.path().join("restored");
    let collection_id = insert_collection(&conn, "work", source_root.path());
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        raw_bytes,
        "notes/a.md",
    );

    let error = begin_restore(&conn, "work", &target_root, false).unwrap_err();

    assert!(error
        .to_string()
        .contains("Vault sync commands require Unix"));
    type RestoreWindowsFailureRow = (
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        i64,
    );

    let row: RestoreWindowsFailureRow = conn
        .query_row(
            "SELECT state,
                    root_path,
                    needs_full_sync,
                    pending_root_path,
                    restore_command_id,
                    restore_lease_session_id,
                    (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1),
                    (SELECT COUNT(*) FROM serve_sessions WHERE session_type = 'cli')
             FROM collections
             WHERE id = ?1",
            [collection_id],
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
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, "restoring");
    assert_eq!(row.1, target_root.display().to_string());
    assert_eq!(row.2, 1);
    assert!(row.3.is_none());
    assert!(row.4.is_none());
    assert!(row.5.is_none());
    assert_eq!(row.6, 0);
    assert_eq!(row.7, 0);
    assert_eq!(
        fs::read(target_root.join("notes").join("a.md")).unwrap(),
        raw_bytes
    );
}

#[test]
fn finalize_pending_restore_requires_exact_originator_or_stale_heartbeat() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = 'C:/restored',
             pending_restore_manifest = '{\"entries\":[]}',
             restore_command_id = 'restore-1',
             pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let outcome = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::StartupRecovery {
            session_id: "serve-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(outcome, FinalizeOutcome::Deferred);
}

#[test]
fn finalize_pending_restore_production_callers_pass_explicit_finalize_caller_variants() {
    let source = production_vault_sync_source();
    let callsites = source
        .match_indices("finalize_pending_restore(")
        .map(|(index, _)| index)
        .filter(|index| !source[..*index].ends_with("fn "))
        .collect::<Vec<_>>();

    assert_eq!(
        callsites.len(),
        3,
        "expected exactly three production finalize_pending_restore call sites \
         (begin_restore unified its online/offline calls into register_manifest)"
    );

    for callsite in callsites {
        let snippet_end = std::cmp::min(callsite + 240, source.len());
        let snippet = &source[callsite..snippet_end];
        assert!(
            snippet.contains("FinalizeCaller::ExternalFinalize")
                || snippet.contains("FinalizeCaller::StartupRecovery")
                || snippet.contains("FinalizeCaller::RestoreOriginator"),
            "production finalize call site must pass an explicit FinalizeCaller variant: {snippet}"
        );
    }
}

#[test]
fn finalize_pending_restore_startup_recovery_uses_shared_15_second_heartbeat_threshold() {
    let conn = open_test_db();
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
             pending_command_heartbeat_at = datetime('now', '-14 seconds')
         WHERE id = ?1",
        params![collection_id, pending_root.display().to_string()],
    )
    .unwrap();

    let fresh = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::StartupRecovery {
            session_id: "serve-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(fresh, FinalizeOutcome::Deferred);

    conn.execute(
        "UPDATE collections
         SET pending_command_heartbeat_at = datetime('now', '-16 seconds')
         WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let stale = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::StartupRecovery {
            session_id: "serve-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(stale, FinalizeOutcome::Finalized);
}

#[test]
fn finalize_pending_restore_allows_exact_originator_with_fresh_heartbeat() {
    let conn = open_test_db();
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
        params![collection_id, pending_root.display().to_string()],
    )
    .unwrap();

    let outcome = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::RestoreOriginator {
            command_id: "restore-1".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(outcome, FinalizeOutcome::Finalized);
    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    assert_eq!(collection.root_path, pending_root.display().to_string());
    assert!(collection.pending_root_path.is_none());
    assert!(collection.needs_full_sync);
}

#[test]
fn finalize_pending_restore_rejects_foreign_originator_with_fresh_heartbeat() {
    let conn = open_test_db();
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
        params![collection_id, pending_root.display().to_string()],
    )
    .unwrap();

    let outcome = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::RestoreOriginator {
            command_id: "restore-2".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(outcome, FinalizeOutcome::Deferred);
    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    assert_eq!(
        collection.pending_root_path.as_deref(),
        Some(pending_root.to_str().unwrap())
    );
    assert_eq!(collection.restore_command_id.as_deref(), Some("restore-1"));
}

#[test]
fn finalize_pending_restore_external_finalize_runs_tx_b_canonical_state() {
    let conn = open_test_db();
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
             restore_command_pid = 99,
             restore_command_host = 'host',
             pending_command_heartbeat_at = datetime('now', '-120 seconds'),
             watcher_released_session_id = 'serve-1',
             watcher_released_generation = 2,
             watcher_released_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![collection_id, pending_root.display().to_string()],
    )
    .unwrap();

    let outcome = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::ExternalFinalize {
            session_id: "serve-1".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(outcome, FinalizeOutcome::Finalized);
    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    assert_eq!(collection.root_path, pending_root.display().to_string());
    assert_eq!(collection.state, CollectionState::Restoring);
    assert!(collection.needs_full_sync);
    assert!(collection.pending_root_path.is_none());
    assert!(collection.pending_restore_manifest.is_none());
    assert!(collection.restore_command_id.is_none());
    assert!(collection.pending_command_heartbeat_at.is_none());
    assert!(collection.watcher_released_session_id.is_none());
    assert!(collection.watcher_released_generation.is_none());
    assert!(collection.watcher_released_at.is_none());
}

#[test]
fn run_tx_b_is_idempotent_and_arms_write_gate() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_restore_manifest = '{\"entries\":[]}'
         WHERE id = ?1",
        params![collection_id, temp.path().display().to_string()],
    )
    .unwrap();

    assert!(run_tx_b(&conn, collection_id).unwrap());
    assert!(!run_tx_b(&conn, collection_id).unwrap());

    let row: (String, i64) = conn
        .query_row(
            "SELECT state, needs_full_sync FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row.0, "restoring");
    assert_eq!(row.1, 1);
}

#[test]
fn complete_attach_source_is_reentry_guarded_by_needs_full_sync() {
    let source = production_vault_sync_source();
    let start = source.find("fn complete_attach(").unwrap();
    let end = source[start..]
        .find("pub fn run_rcrt_pass(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("if !collection.needs_full_sync") && snippet.contains("return Ok(false);"),
        "complete_attach must fail closed to a no-op when the write gate is already cleared"
    );
    assert!(
        snippet.contains("reload_generation = reload_generation + 1"),
        "attach completion must advance generation exactly on the guarded transition"
    );
    assert!(
        snippet.contains("AND needs_full_sync = 1"),
        "the attach-completion UPDATE must be gated on needs_full_sync so re-entry cannot bump generation again"
    );
}

#[test]
fn begin_restore_preserves_tx_b_residue_and_plain_sync_cannot_consume_it() {
    let (_db_dir, _db_path, conn) = open_test_db_file();
    let source_root = tempfile::TempDir::new().unwrap();
    let target_parent = tempfile::TempDir::new().unwrap();
    let target_root = target_parent.path().join("restored");
    fs::create_dir_all(source_root.path().join("notes")).unwrap();
    let collection_id = insert_collection(&conn, "work", source_root.path());
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        raw_bytes,
        "notes/a.md",
    );
    fs::write(source_root.path().join("notes").join("a.md"), raw_bytes).unwrap();
    conn.execute(
        "CREATE TRIGGER tx_b_fail
         BEFORE UPDATE ON collections
         WHEN OLD.pending_root_path IS NOT NULL AND NEW.pending_root_path IS NULL
         BEGIN
             SELECT RAISE(FAIL, 'tx-b fail');
         END",
        [],
    )
    .unwrap();

    let error = begin_restore(&conn, "work", &target_root, false).unwrap_err();

    assert!(error.to_string().contains("tx-b fail"));
    let row: (String, Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state, pending_root_path, restore_command_id, pending_restore_manifest
             FROM collections
             WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row.0, "restoring");
    assert_eq!(row.1.as_deref(), Some(target_root.to_str().unwrap()));
    assert!(row.2.is_some());
    assert!(row.3.is_some());
    assert!(target_root.exists());

    let sync_error = sync_collection(&conn, "work").unwrap_err();

    assert!(sync_error
        .to_string()
        .contains("RestorePendingFinalizeError"));
    let retained_pending_root: Option<String> = conn
        .query_row(
            "SELECT pending_root_path FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        retained_pending_root.as_deref(),
        Some(target_root.to_str().unwrap())
    );
}

#[test]
fn finalize_pending_restore_retries_manifest_incomplete_until_success() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let pending_root = temp.path().join("restored");
    write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
    let manifest_json = manifest_json_for_directory(&pending_root);
    fs::remove_file(pending_root.join("notes").join("a.md")).unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
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

    let first = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::RestoreOriginator {
            command_id: "restore-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(first, FinalizeOutcome::ManifestIncomplete);
    let first_incomplete_at: String = conn
        .query_row(
            "SELECT pending_manifest_incomplete_at FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap();

    let second = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::RestoreOriginator {
            command_id: "restore-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(second, FinalizeOutcome::ManifestIncomplete);
    let second_incomplete_at: String = conn
        .query_row(
            "SELECT pending_manifest_incomplete_at FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(first_incomplete_at, second_incomplete_at);

    write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
    let final_outcome = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::RestoreOriginator {
            command_id: "restore-1".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(final_outcome, FinalizeOutcome::Finalized);
    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    assert!(collection.pending_root_path.is_none());
    assert!(collection.pending_manifest_incomplete_at.is_none());
    assert!(collection.integrity_failed_at.is_none());
    assert_eq!(collection.root_path, pending_root.display().to_string());
}

#[test]
fn finalize_pending_restore_escalates_manifest_incomplete_after_ttl() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let pending_root = temp.path().join("restored");
    write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
    let manifest_json = manifest_json_for_directory(&pending_root);
    fs::remove_file(pending_root.join("notes").join("a.md")).unwrap();
    let collection_id = insert_collection(&conn, "work", temp.path());
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_restore_manifest = ?3,
             restore_command_id = 'restore-1',
             pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             pending_manifest_incomplete_at = datetime('now', '-31 minutes')
         WHERE id = ?1",
        params![
            collection_id,
            pending_root.display().to_string(),
            manifest_json
        ],
    )
    .unwrap();

    let outcome = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::RestoreOriginator {
            command_id: "restore-1".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(outcome, FinalizeOutcome::IntegrityFailed);
    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    assert_eq!(
        collection.pending_root_path.as_deref(),
        Some(pending_root.to_str().unwrap())
    );
    assert!(collection.pending_manifest_incomplete_at.is_some());
    assert!(collection.integrity_failed_at.is_some());
}

#[test]
fn finalize_pending_restore_detects_manifest_tamper_and_restore_reset_clears_it() {
    let conn = open_test_db();
    let temp = tempfile::TempDir::new().unwrap();
    let pending_root = temp.path().join("restored");
    write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
    let manifest_json = manifest_json_for_directory(&pending_root);
    write_restore_file(&pending_root, "notes/a.md", b"tampered bytes");
    let collection_id = insert_collection(&conn, "work", temp.path());
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

    let outcome = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::RestoreOriginator {
            command_id: "restore-1".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(outcome, FinalizeOutcome::IntegrityFailed);
    let blocked = load_collection_by_id(&conn, collection_id).unwrap();
    assert_eq!(
        blocked.pending_root_path.as_deref(),
        Some(pending_root.to_str().unwrap())
    );
    assert!(blocked.integrity_failed_at.is_some());

    write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
    let still_blocked = finalize_pending_restore(
        &conn,
        collection_id,
        FinalizeCaller::RestoreOriginator {
            command_id: "restore-1".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(still_blocked, FinalizeOutcome::IntegrityFailed);

    restore_reset(&conn, "work").unwrap();

    let reset = load_collection_by_id(&conn, collection_id).unwrap();
    assert_eq!(reset.state, CollectionState::Active);
    assert!(reset.pending_root_path.is_none());
    assert!(reset.pending_restore_manifest.is_none());
    assert!(reset.restore_command_id.is_none());
    assert!(reset.integrity_failed_at.is_none());
}

#[cfg(unix)]
#[test]
fn offline_restore_runs_attach_inline_and_reopens_writes() {
    let (_db_dir, _db_path, conn) = open_test_db_file();
    let source_root = tempfile::TempDir::new().unwrap();
    let target_parent = tempfile::TempDir::new().unwrap();
    let target_root = target_parent.path().join("restored");
    fs::create_dir_all(source_root.path().join("notes")).unwrap();
    let collection_id = insert_collection(&conn, "work", source_root.path());
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        raw_bytes,
        "notes/a.md",
    );
    fs::write(source_root.path().join("notes").join("a.md"), raw_bytes).unwrap();

    begin_restore(&conn, "work", &target_root, false).unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    assert_eq!(collection.state, CollectionState::Active);
    assert!(!collection.needs_full_sync);
    assert!(collection.active_lease_session_id.is_none());
    assert!(collection.restore_lease_session_id.is_none());
    assert!(owner_session_id(&conn, collection_id).unwrap().is_none());
    ensure_collection_write_allowed(&conn, collection_id).unwrap();
}

#[test]
fn restore_source_runs_safety_pipeline_before_materialization() {
    // After §6 begin_restore was decomposed into validate_target /
    // stage_pending / register_manifest, both branches of the
    // online/offline split share a single stage_pending body. The
    // safety-pipeline-before-materialization invariant is now a
    // single ordering inside stage_pending; the orchestrator call
    // order (validate_target → stage_pending → register_manifest)
    // means it also holds across the begin_restore call.
    let source = production_vault_sync_source();
    let stage_start = source.find("fn stage_pending(").unwrap();
    let stage_end = source[stage_start..]
        .find("fn register_manifest(")
        .map(|offset| stage_start + offset)
        .unwrap();
    let stage_source = &source[stage_start..stage_end];
    let safety_idx = stage_source
        .find("run_restore_remap_safety_pipeline_without_mount_check")
        .unwrap();
    let materialize_idx = stage_source
        .find("materialize_collection_to_path(conn, &prep.collection, &staging_path)?;")
        .unwrap();
    assert!(
        safety_idx < materialize_idx,
        "stage_pending must capture old-root drift before materializing raw_imports into the target"
    );
}

#[test]
fn restore_online_source_waits_for_exact_ack_before_safety_pipeline() {
    // After §6 the online-restore handshake (mark + ack +
    // initial UPDATE) lives in validate_target while the safety
    // pipeline lives in stage_pending. The orchestrator runs
    // validate_target before stage_pending, so the
    // "ack-before-safety-pipeline" invariant becomes:
    // wait_for_exact_ack appears in validate_target, and
    // run_restore_remap_safety_pipeline_without_mount_check
    // appears in stage_pending.
    let source = production_vault_sync_source();
    let validate_start = source.find("fn validate_target(").unwrap();
    let validate_end = source[validate_start..]
        .find("fn stage_pending(")
        .map(|offset| validate_start + offset)
        .unwrap();
    let validate_source = &source[validate_start..validate_end];
    assert!(
        validate_source
            .contains("wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?"),
        "validate_target must wait for the acknowledged owner lease (online branch) before stage_pending runs"
    );
    let stage_start = validate_end;
    let stage_end = source[stage_start..]
        .find("fn register_manifest(")
        .map(|offset| stage_start + offset)
        .unwrap();
    let stage_source = &source[stage_start..stage_end];
    assert!(
        stage_source.contains("run_restore_remap_safety_pipeline_without_mount_check"),
        "stage_pending owns the safety pipeline call (after validate_target's ack)"
    );
}

#[test]
fn restore_offline_source_uses_short_lived_lease_and_inline_attach() {
    let source = production_vault_sync_source();
    let restore_start = source.find("pub fn begin_restore(").unwrap();
    let restore_end = source[restore_start..]
        .find("pub(super) fn ensure_restore_not_blocked(")
        .map(|offset| restore_start + offset)
        .unwrap();
    let restore_source = &source[restore_start..restore_end];
    let offline_start = restore_source
        .find("let lease = start_short_lived_owner_lease(conn, collection.id)?;")
        .unwrap();
    let offline_source = &restore_source[offline_start..];

    assert!(
        offline_source.contains("complete_attach(")
            && offline_source.contains("AttachReason::RestorePostFinalize"),
        "offline restore must run the attach/full-hash path inline while the CLI lease is live"
    );
    assert!(
        !offline_source.contains("unregister_session("),
        "offline restore must not drop its lease before the inline attach finishes"
    );
}

// ── Security hardening: restore path-traversal rejection ──────────

#[test]
fn validated_restore_relative_path_accepts_normal_nested_paths() {
    let path =
        validated_restore_relative_path("work", "notes/a", PathBuf::from("notes/a.md")).unwrap();
    assert_eq!(path, PathBuf::from("notes/a.md"));
}

#[test]
fn validated_restore_relative_path_rejects_parent_traversal() {
    let error =
        validated_restore_relative_path("work", "notes/a", PathBuf::from("../evil.md")).unwrap_err();
    assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
    let message = error.to_string();
    assert!(message.contains("refusing restore path"), "message={message}");
}

#[test]
fn validated_restore_relative_path_rejects_embedded_traversal() {
    let error =
        validated_restore_relative_path("work", "notes/a", PathBuf::from("notes/../../evil.md"))
            .unwrap_err();
    assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
}

#[test]
fn validated_restore_relative_path_rejects_absolute_paths() {
    let error = validated_restore_relative_path("work", "notes/a", PathBuf::from("/etc/passwd"))
        .unwrap_err();
    assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
}

// NOTE on end-to-end coverage: the materialize-time guard
// (`validated_restore_relative_path`, exercised by the unit tests above)
// is a defense-in-depth layer. A full `begin_restore` traversal test is
// not meaningful for it because the offline restore runs a full-hash
// reconcile (the safety pipeline) before materialization, which
// normalizes a tampered `file_state.relative_path` back to the value
// implied by the on-disk source tree — so the malicious path never
// reaches materialize in the happy offline path. The guard's value is
// catching a corrupt/tampered row that survives or bypasses reconcile,
// which the direct unit tests above cover.
