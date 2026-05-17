#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `quaid collection restore` and `quaid collection
//! restore-reset` truth-merge behavior, including startup recovery for
//! orphaned restores and live-serve rebind on online restore.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

#[cfg(unix)]
use quaid::core::vault_sync;
use rusqlite::params;
#[cfg(unix)]
use sha2::Digest;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;
use truth_fixtures::*;

#[cfg(unix)]
#[test]
fn offline_restore_completes_inline_and_releases_cli_lease() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "offline-restore-cli.db");
    let conn = open_test_db(&db_path);
    let source_root = dir.path().join("source");
    let target_root = dir.path().join("restored");
    std::fs::create_dir_all(source_root.join("notes")).expect("create source root");
    let collection_id = insert_collection(&conn, "work", &source_root);
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\nquaid_id: 11111111-1111-7111-8111-111111111111\nslug: notes/a\ntitle: Restored Note\ntype: concept\n---\nhello from restore\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        raw_bytes,
        "notes/a.md",
    );
    std::fs::write(source_root.join("notes").join("a.md"), raw_bytes).expect("seed source note");
    drop(conn);

    let restore_output = run_quaid(
        &db_path,
        &[
            "--json",
            "collection",
            "restore",
            "work",
            target_root.to_str().expect("utf-8 target"),
        ],
    );

    assert!(
        restore_output.status.success(),
        "offline restore should complete the root switch successfully: {restore_output:?}"
    );
    let restore_json = parse_stdout_json(&restore_output);
    assert_eq!(restore_json["status"].as_str(), Some("ok"));
    assert!(restore_json["command_identity"].as_str().is_some());

    type OfflineRestoreRow = (
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );

    let conn = open_test_db(&db_path);
    let row: OfflineRestoreRow = conn
        .query_row(
            "SELECT state, root_path, needs_full_sync, pending_root_path, integrity_failed_at,
                    pending_manifest_incomplete_at, restore_lease_session_id
             FROM collections WHERE id = ?1",
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
                ))
            },
        )
        .expect("load finalized collection");
    assert_eq!(row.0, "active");
    assert_eq!(row.1, target_root.to_str().expect("utf-8 target"));
    assert_eq!(row.2, 0);
    assert!(row.3.is_none());
    assert!(row.4.is_none());
    assert!(row.5.is_none());
    assert!(row.6.is_none());
    assert_cli_lease_released(&conn, collection_id);
    assert_eq!(
        std::fs::read(target_root.join("notes").join("a.md")).expect("read restored file"),
        raw_bytes
    );
}

#[cfg(unix)]
#[test]
fn offline_restore_captures_source_drift_and_added_pages_before_inline_attach() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "offline-restore-drift-cli.db");
    let conn = open_test_db(&db_path);
    let source_root = dir.path().join("source");
    let target_root = dir.path().join("restored");
    std::fs::create_dir_all(source_root.join("notes")).expect("create source root");
    std::fs::create_dir_all(&target_root).expect("create empty target");

    let collection_id = insert_collection(&conn, "work", &source_root);
    let stale_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\nquaid_id: 11111111-1111-7111-8111-111111111111\nslug: notes/a\ntitle: Restored Note\ntype: concept\n---\nstale restore body\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        stale_bytes,
        "notes/a.md",
    );
    drop(conn);

    let refreshed_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\nquaid_id: 11111111-1111-7111-8111-111111111111\nslug: notes/a\ntitle: Restored Note\ntype: concept\n---\nrefreshed restore body captured from the live source root before restore completes\n";
    std::fs::write(source_root.join("notes").join("a.md"), refreshed_bytes)
        .expect("write refreshed source note");
    let added_bytes =
        b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\nquaid_id: 22222222-2222-7222-8222-222222222222\nslug: notes/b\ntitle: Added During Drift Capture\ntype: concept\n---\nthis note only existed on disk when restore began, so phase 1 must ingest it before materialization\n";
    std::fs::write(source_root.join("notes").join("b.md"), added_bytes)
        .expect("write added source note");

    let restore_output = run_quaid(
        &db_path,
        &[
            "--json",
            "collection",
            "restore",
            "work",
            target_root.to_str().expect("utf-8 target"),
        ],
    );

    assert!(
        restore_output.status.success(),
        "offline restore should capture live drift and complete inline: {restore_output:?}"
    );
    let restore_json = parse_stdout_json(&restore_output);
    assert_eq!(restore_json["status"].as_str(), Some("ok"));
    assert_eq!(restore_json["restored"].as_u64(), Some(2));
    assert_eq!(restore_json["byte_exact"].as_u64(), Some(2));

    let conn = open_test_db(&db_path);
    let row: (
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT state,
                    root_path,
                    needs_full_sync,
                    pending_root_path,
                    restore_command_id,
                    restore_lease_session_id
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
                ))
            },
        )
        .expect("load restored collection");
    assert_eq!(row.0, "active");
    assert_eq!(row.1, target_root.to_str().expect("utf-8 target"));
    assert_eq!(row.2, 0);
    assert!(row.3.is_none());
    assert!(row.4.is_none());
    assert!(row.5.is_none());
    assert_cli_lease_released(&conn, collection_id);

    let restored_page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = ?1 AND quarantined_at IS NULL",
            [collection_id],
            |row| row.get(0),
        )
        .expect("count restored pages");
    assert_eq!(restored_page_count, 2);
    assert_eq!(
        std::fs::read(target_root.join("notes").join("a.md")).expect("read restored note a"),
        refreshed_bytes
    );
    assert_eq!(
        std::fs::read(target_root.join("notes").join("b.md")).expect("read restored note b"),
        added_bytes
    );
}

#[cfg(unix)]
#[test]
fn startup_recovery_finalizes_tx_b_restore_orphan() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "startup-recovery.db");
    let conn = open_test_db(&db_path);
    let source_root = dir.path().join("source");
    let pending_root = dir.path().join("restored");
    std::fs::create_dir_all(&source_root).expect("create source root");
    let collection_id = insert_collection(&conn, "work", &source_root);
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\ntitle: Restored Note\ntype: concept\n---\nhello from restore\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        raw_bytes,
        "notes/a.md",
    );
    std::fs::create_dir_all(pending_root.join("notes")).expect("create pending notes dir");
    std::fs::write(pending_root.join("notes").join("a.md"), raw_bytes)
        .expect("write restored note");
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('stale-owner', 1, 'host', datetime('now', '-16 seconds'))",
        [],
    )
    .expect("seed stale owner session");
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'stale-owner')",
        [collection_id],
    )
    .expect("seed stale owner lease");
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
    .expect("seed restore orphan");
    drop(conn);

    let runtime =
        vault_sync::start_serve_runtime(db_path.to_str().expect("utf-8 db path").to_owned())
            .expect("start serve runtime");

    let conn = open_test_db(&db_path);
    let row: (String, String, i64, Option<String>, Option<String>, i64, i64) = conn
        .query_row(
            "SELECT state,
                    root_path,
                    needs_full_sync,
                    pending_root_path,
                    pending_command_heartbeat_at,
                    (SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'stale-owner'),
                    (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2)
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
                ))
            },
        )
        .expect("load recovered collection");
    assert_eq!(row.0, "active");
    assert_eq!(row.1, pending_root.display().to_string());
    assert_eq!(row.2, 0);
    assert!(row.3.is_none());
    assert!(row.4.is_none());
    assert_eq!(row.5, 0);
    assert_eq!(row.6, 1);
    assert_eq!(
        std::fs::read(pending_root.join("notes").join("a.md")).expect("read restored note"),
        raw_bytes
    );

    drop(runtime);
}

#[test]
fn restore_reset_returns_failure_for_retryable_manifest_gap() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "restore-reset-retryable.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    let pending_root = dir.path().join("restored");
    std::fs::create_dir_all(&pending_root).expect("create pending root");
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_manifest_incomplete_at = '2026-04-23T00:05:00Z'
         WHERE id = ?1",
        params![collection_id, pending_root.display().to_string()],
    )
    .expect("seed retryable restore gap");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["collection", "restore-reset", "work", "--confirm"],
    );

    assert!(
        !output.status.success(),
        "restore-reset must fail while manifest retry is still pending: {output:?}"
    );
    let text = combined_output(&output);
    assert!(
        text.contains("RestoreResetBlockedError"),
        "restore-reset must explain the blocked state: {output:?}"
    );
    assert!(
        text.contains("manifest_incomplete_retryable"),
        "restore-reset must name the retryable manifest reason: {output:?}"
    );

    let conn = open_test_db(&db_path);
    let row: (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state, pending_root_path, pending_manifest_incomplete_at
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load collection after blocked reset");
    assert_eq!(row.0, "restoring");
    assert_eq!(
        row.1.as_deref(),
        Some(pending_root.to_str().expect("utf-8 pending root"))
    );
    assert_eq!(row.2.as_deref(), Some("2026-04-23T00:05:00Z"));
}

#[test]
fn restore_reset_succeeds_for_terminal_integrity_failure() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "restore-reset-terminal.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             integrity_failed_at = '2026-04-23T00:00:00Z',
             pending_manifest_incomplete_at = '2026-04-23T00:05:00Z',
             restore_command_id = 'restore-1'
         WHERE id = ?1",
        params![
            collection_id,
            dir.path().join("restored").display().to_string()
        ],
    )
    .expect("seed terminal integrity failure");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["--json", "collection", "restore-reset", "work", "--confirm"],
    );

    assert!(
        output.status.success(),
        "restore-reset should succeed after terminal integrity failure: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["status"].as_str(), Some("ok"));
    assert_eq!(parsed["command"].as_str(), Some("restore-reset"));

    let conn = open_test_db(&db_path);
    let row: (String, Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state, pending_root_path, integrity_failed_at, restore_command_id
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("load collection after reset");
    assert_eq!(row.0, "active");
    assert!(row.1.is_none());
    assert!(row.2.is_none());
    assert!(row.3.is_none());
}

#[cfg(unix)]
#[test]
fn offline_restore_round_trips_exact_raw_import_bytes() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "offline-restore-byte-exact.db");
    let conn = open_test_db(&db_path);
    let source_root = dir.path().join("source");
    let target_root = dir.path().join("restored");
    std::fs::create_dir_all(source_root.join("notes")).expect("create source root");
    let collection_id = insert_collection(&conn, "work", &source_root);
    let raw_bytes = b"---\r\ntitle: Byte Exact  \r\ntype: concept\r\nslug: notes/byte-exact\r\n---\r\nfirst line with trailing spaces   \r\n\r\n- bullet one\r\n- bullet two\r\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/byte-exact",
        "11111111-1111-7111-8111-111111111111",
        raw_bytes,
        "notes/byte-exact.md",
    );
    std::fs::write(source_root.join("notes").join("byte-exact.md"), raw_bytes)
        .expect("seed source note");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "--json",
            "collection",
            "restore",
            "work",
            target_root.to_str().expect("utf-8 target"),
        ],
    );

    assert!(
        output.status.success(),
        "offline restore should succeed for byte-exact proof: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["status"].as_str(), Some("ok"));
    assert_eq!(parsed["byte_exact"].as_u64(), Some(1));

    let conn = open_test_db(&db_path);
    let stored_raw_bytes: Vec<u8> = conn
        .query_row(
            "SELECT ri.raw_bytes
             FROM raw_imports ri
             JOIN pages p ON p.id = ri.page_id
             WHERE p.collection_id = ?1
               AND p.slug = 'notes/byte-exact'
               AND ri.is_active = 1",
            [collection_id],
            |row| row.get(0),
        )
        .expect("load active raw import");
    assert_eq!(stored_raw_bytes, raw_bytes);
    assert_eq!(
        std::fs::read(target_root.join("notes").join("byte-exact.md"))
            .expect("read restored bytes"),
        raw_bytes,
        "restore must materialize the exact active raw_import bytes"
    );
}

#[cfg(unix)]
#[test]
fn online_restore_with_live_serve_rebinds_without_restarting_serve() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    // Resolve the TempDir through any platform symlinks (macOS /var → /private/var)
    // so test-side paths match the canonical paths that production code, FSEvents,
    // and SQLite all use.
    let dir_canonical = std::fs::canonicalize(dir.path()).expect("canonicalize tempdir");
    let db_path = dir_canonical.join("online-restore-live-serve.db");
    let conn = open_test_db(&db_path);
    let source_root = dir_canonical.join("source");
    let target_root = dir_canonical.join("restored");
    std::fs::create_dir_all(source_root.join("notes")).expect("create source root");
    let collection_id = insert_collection(&conn, "work", &source_root);
    conn.execute(
        "CREATE TABLE watcher_release_audit (
             collection_id INTEGER NOT NULL,
             watcher_session_id TEXT NOT NULL,
             reload_generation INTEGER NOT NULL
         )",
        [],
    )
    .expect("create watcher release audit");
    conn.execute(
        "CREATE TRIGGER watcher_release_audit_insert
         AFTER UPDATE ON collections
         WHEN OLD.watcher_released_at IS NULL AND NEW.watcher_released_at IS NOT NULL
         BEGIN
             INSERT INTO watcher_release_audit (collection_id, watcher_session_id, reload_generation)
             VALUES (NEW.id, NEW.watcher_released_session_id, NEW.watcher_released_generation);
         END",
        [],
    )
    .expect("create watcher release audit trigger");
    let raw_bytes =
        b"---\nmemory_id: 33333333-3333-7333-8333-333333333333\ntitle: Online Restore\ntype: concept\n---\nrestored body before live attach\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/online",
        "33333333-3333-7333-8333-333333333333",
        raw_bytes,
        "notes/online.md",
    );
    std::fs::write(source_root.join("notes").join("online.md"), raw_bytes)
        .expect("seed source note");
    drop(conn);

    let runtime =
        vault_sync::start_serve_runtime(db_path.to_str().expect("utf-8 db path").to_owned())
            .expect("start serve runtime");
    thread::sleep(Duration::from_secs(1));

    let output = run_quaid_with_env(
        &db_path,
        &[
            "--json",
            "collection",
            "restore",
            "work",
            target_root.to_str().expect("utf-8 target"),
            "--online",
        ],
        &[("QUAID_HANDSHAKE_TIMEOUT_SECS", "90")],
    );

    assert!(
        output.status.success(),
        "online restore should succeed against the live serve session: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["status"].as_str(), Some("ok"));
    assert!(parsed["command_identity"].as_str().is_some());

    type OnlineRestoreRow = (
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
        i64,
        i64,
    );
    let final_row: OnlineRestoreRow =
        wait_for_db_value(&db_path, Duration::from_secs(15), |verify| {
            verify
                .query_row(
                    "SELECT c.state,
                            c.root_path,
                            c.needs_full_sync,
                            c.pending_root_path,
                            c.restore_command_id,
                            (SELECT COUNT(*) FROM serve_sessions WHERE session_id = ?2 AND session_type IN ('daemon', 'serve_host', 'serve')),
                            (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2)
                     FROM collections c
                     WHERE c.id = ?1",
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
                        ))
                    },
                )
                .ok()
                .and_then(|row: OnlineRestoreRow| {
                    (row.0 == "active"
                        && row.1 == target_root.display().to_string()
                        && row.2 == 0
                        && row.3.is_none()
                        && row.4.is_none()
                        && row.5 == 1
                        && row.6 == 1)
                        .then_some(row)
                })
        })
        .expect("serve runtime never completed the online attach");
    assert_eq!(final_row.0, "active");
    assert_eq!(final_row.1, target_root.display().to_string());
    assert_eq!(final_row.2, 0);
    assert!(final_row.3.is_none());
    assert!(final_row.4.is_none());
    assert_eq!(final_row.5, 1);
    assert_eq!(final_row.6, 1);

    let conn = open_test_db(&db_path);
    let release_audit: (i64, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT COUNT(*), MIN(watcher_session_id), MIN(reload_generation)
             FROM watcher_release_audit
             WHERE collection_id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load watcher release audit");
    assert_eq!(
        release_audit.0, 1,
        "handshake should release the watcher exactly once"
    );
    assert_eq!(
        release_audit.1.as_deref(),
        Some(runtime.session_id.as_str()),
        "online restore must release the live serve watcher, not a new session"
    );
    assert!(release_audit.2.is_some());
    drop(conn);

    thread::sleep(Duration::from_secs(1));

    std::fs::write(
        target_root.join("notes").join("online.md"),
        b"---\nmemory_id: 33333333-3333-7333-8333-333333333333\ntitle: Online Restore\ntype: concept\n---\nupdated after online restore\n",
    )
    .expect("write live edit into restored root");

    let rebind_row = wait_for_db_value(&db_path, Duration::from_secs(15), |verify| {
        verify
            .query_row(
                "SELECT p.compiled_truth,
                        (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2)
                 FROM pages p
                 WHERE p.collection_id = ?1
                   AND p.slug = 'notes/online'",
                params![collection_id, runtime.session_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .ok()
            .filter(|row: &(String, i64)| row.0.contains("updated after online restore"))
    })
    .expect("watcher never reconciled the live edit on the restored target");
    assert!(rebind_row.0.contains("updated after online restore"));
    assert_eq!(rebind_row.1, 1);

    drop(runtime);
}
