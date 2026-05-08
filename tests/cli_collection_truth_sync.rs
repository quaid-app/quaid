#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `quaid collection sync` truth-merge behavior.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use rusqlite::params;
use sha2::Digest;
use truth_fixtures::*;

#[test]
fn sync_finalize_pending_returns_failure_for_no_pending_work() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "finalize-no-pending.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    insert_collection(&conn, "work", &root);
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["collection", "sync", "work", "--finalize-pending"],
    );

    assert!(
        !output.status.success(),
        "finalize-pending on active collection with no pending restore must return non-success: {output:?}"
    );
    let text = combined_output(&output);
    #[cfg(unix)]
    assert!(
        text.contains("FinalizePendingBlockedError"),
        "must emit FinalizePendingBlockedError for NoPendingWork outcome: {output:?}"
    );
    #[cfg(not(unix))]
    assert!(
        text.contains("UnsupportedPlatformError"),
        "Windows finalize-pending must fail closed with UnsupportedPlatformError: {output:?}"
    );
    #[cfg(unix)]
    assert!(
        text.contains("NoPendingWork"),
        "must name the NoPendingWork outcome in the error: {output:?}"
    );
    #[cfg(unix)]
    assert!(
        text.contains("remains blocked") || text.contains("not finalized"),
        "must explicitly say collection remains blocked / was not finalized: {output:?}"
    );
}

#[test]
fn sync_finalize_pending_returns_failure_for_deferred() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "finalize-deferred.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    let pending_root = dir.path().join("restored");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&pending_root).expect("create pending root");
    let collection_id = insert_collection(&conn, "work", &root);
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
    .expect("seed restoring collection with fresh heartbeat");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["collection", "sync", "work", "--finalize-pending"],
    );

    assert!(
        !output.status.success(),
        "finalize-pending on deferred restore must return non-success: {output:?}"
    );
    let text = combined_output(&output);
    #[cfg(unix)]
    assert!(
        text.contains("FinalizePendingBlockedError"),
        "must emit FinalizePendingBlockedError for Deferred outcome: {output:?}"
    );
    #[cfg(not(unix))]
    assert!(
        text.contains("UnsupportedPlatformError"),
        "Windows finalize-pending must fail closed with UnsupportedPlatformError: {output:?}"
    );
    #[cfg(unix)]
    assert!(
        text.contains("Deferred"),
        "must name the Deferred outcome in the error: {output:?}"
    );
    #[cfg(unix)]
    assert!(
        text.contains("remains blocked") || text.contains("not finalized"),
        "must explicitly say collection remains blocked / was not finalized: {output:?}"
    );
}

#[test]
fn collection_sync_without_flags_returns_failure_and_preserves_pending_finalize_state() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-sync.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    let pending_root = dir.path().join("restored");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&pending_root).expect("create pending root");
    let collection_id = insert_collection(&conn, "work", &root);
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
    .expect("seed pending restore");
    drop(conn);

    let output = run_quaid(&db_path, &["collection", "sync", "work"]);

    assert!(
        !output.status.success(),
        "plain sync should stay non-success while deferred: {output:?}"
    );
    #[cfg(unix)]
    assert!(
        combined_output(&output).contains("RestorePendingFinalizeError"),
        "plain sync should fail closed on pending-finalize collections: {output:?}"
    );
    #[cfg(not(unix))]
    assert!(
        combined_output(&output).contains("UnsupportedPlatformError"),
        "Windows plain sync must fail closed with UnsupportedPlatformError: {output:?}"
    );

    let conn = open_test_db(&db_path);
    let row: (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state, pending_root_path, restore_command_id
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load collection after failed sync");
    assert_eq!(row.0, "restoring");
    assert_eq!(
        row.1.as_deref(),
        Some(pending_root.to_str().expect("utf-8 path"))
    );
    assert_eq!(row.2.as_deref(), Some("restore-1"));
}

#[cfg(unix)]
#[test]
fn collection_sync_active_root_reports_active_root_reconciled_success() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-sync-active.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections SET needs_full_sync = 1 WHERE id = ?1",
        [collection_id],
    )
    .expect("seed active reconcile state");
    std::fs::write(
        root.join("note.md"),
        "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nA body long enough to reconcile through the active-root path.\n",
    )
    .expect("write note");
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "collection", "sync", "work"]);

    assert!(
        output.status.success(),
        "plain sync should succeed on the active root: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["active_root_reconciled"].as_bool(), Some(true));
    assert_eq!(
        parsed["status_message"].as_str(),
        Some("active root reconciled")
    );

    let conn = open_test_db(&db_path);
    let row: (String, i64) = conn
        .query_row(
            "SELECT state, needs_full_sync FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load collection after sync");
    assert_eq!(row.0, "active");
    assert_eq!(row.1, 0);
}

#[cfg(unix)]
#[test]
fn collection_sync_finalize_pending_attaches_pending_root_and_releases_cli_lease() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-sync-finalize-success.db");
    let conn = open_test_db(&db_path);
    let source_root = dir.path().join("source");
    let pending_root = dir.path().join("restored");
    std::fs::create_dir_all(source_root.join("notes")).expect("create source root");
    std::fs::create_dir_all(pending_root.join("notes")).expect("create pending root");
    let collection_id = insert_collection(&conn, "work", &source_root);
    let raw_bytes =
        b"---\nmemory_id: 33333333-3333-7333-8333-333333333333\ntitle: Finalized Note\ntype: concept\n---\nfinalize pending should attach this root inline\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "33333333-3333-7333-8333-333333333333",
        raw_bytes,
        "notes/a.md",
    );
    std::fs::write(pending_root.join("notes").join("a.md"), raw_bytes).expect("write pending note");
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_restore_manifest = ?3,
             restore_command_id = 'restore-1',
             pending_command_heartbeat_at = datetime('now', '-120 seconds')
         WHERE id = ?1",
        params![
            collection_id,
            pending_root.display().to_string(),
            serde_json::json!({
                "entries": [{
                    "relative_path": "notes/a.md",
                    "sha256": format!("{:x}", sha2::Sha256::digest(raw_bytes)),
                    "size_bytes": raw_bytes.len()
                }]
            })
            .to_string()
        ],
    )
    .expect("seed pending restore");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["--json", "collection", "sync", "work", "--finalize-pending"],
    );

    assert!(
        output.status.success(),
        "finalize-pending should attach the pending root: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["status"].as_str(), Some("ok"));
    assert_eq!(parsed["command"].as_str(), Some("sync"));
    assert_eq!(parsed["collection"].as_str(), Some("work"));
    assert_eq!(parsed["finalize_pending"].as_str(), Some("Attached"));

    type FinalizePendingAttachedRow = (
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    );

    let conn = open_test_db(&db_path);
    let row: FinalizePendingAttachedRow = conn
        .query_row(
            "SELECT state,
                    root_path,
                    needs_full_sync,
                    pending_root_path,
                    restore_command_id,
                    restore_lease_session_id,
                    pending_command_heartbeat_at,
                    (SELECT relative_path FROM file_state WHERE page_id = pages.id LIMIT 1)
             FROM collections
             JOIN pages ON pages.collection_id = collections.id AND pages.slug = 'notes/a'
             WHERE collections.id = ?1",
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
        .expect("load finalized collection");
    assert_eq!(row.0, "active");
    assert_eq!(row.1, pending_root.display().to_string());
    assert_eq!(row.2, 0);
    assert!(row.3.is_none());
    assert!(row.4.is_none());
    assert!(row.5.is_none());
    assert!(row.6.is_none());
    assert_eq!(row.7, "notes/a.md");
    assert_cli_lease_released(&conn, collection_id);
}
