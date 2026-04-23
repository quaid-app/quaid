use gbrain::core::db;
#[cfg(unix)]
use gbrain::core::vault_sync;
use rusqlite::{params, Connection};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().expect("utf-8 db path")).expect("open test db")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_gbrain")
}

fn run_gbrain(db_path: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(bin_path());
    command.arg("--db").arg(db_path).args(args);
    command.output().expect("run gbrain")
}

fn run_gbrain_with_stdin(db_path: &Path, args: &[&str], stdin: &str) -> std::process::Output {
    let mut command = Command::new(bin_path());
    command
        .arg("--db")
        .arg(db_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn gbrain");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for gbrain")
}

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON")
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn test_db_path(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

fn insert_collection(conn: &Connection, name: &str, root_path: &Path) -> i64 {
    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target)
         VALUES (?1, ?2, 'active', 1, 0)",
        params![name, root_path.display().to_string()],
    )
    .expect("insert collection");
    conn.last_insert_rowid()
}

fn insert_page(conn: &Connection, collection_id: i64, slug: &str) {
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, 'note', ?2, '', 'compiled', '', '{}', 'notes', '', 1)",
        params![collection_id, slug],
    )
    .expect("insert page");
}

#[cfg(unix)]
fn insert_page_with_raw_import(
    conn: &Connection,
    collection_id: i64,
    slug: &str,
    uuid: &str,
    raw_bytes: &[u8],
    relative_path: &str,
) {
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, 'concept', ?2, '', ?2, '', '{}', 'notes', '', 1)",
        params![collection_id, slug, uuid],
    )
    .expect("insert page with uuid");
    let page_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
         VALUES (?1, ?2, 1, ?3, ?4)",
        params![
            page_id,
            uuid::Uuid::now_v7().to_string(),
            raw_bytes,
            relative_path
        ],
    )
    .expect("insert raw import");
    let sha256 = format!("{:x}", sha2::Sha256::digest(raw_bytes));
    conn.execute(
        "INSERT INTO file_state (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256)
         VALUES (?1, ?2, ?3, 1, 1, ?4, 1, ?5)",
        params![
            collection_id,
            relative_path,
            page_id,
            raw_bytes.len() as i64,
            sha256
        ],
    )
    .expect("insert file state");
}

#[test]
fn sync_finalize_pending_returns_failure_for_no_pending_work() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "finalize-no-pending.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    insert_collection(&conn, "work", &root);
    drop(conn);

    let output = run_gbrain(
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

    let output = run_gbrain(
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

    let output = run_gbrain(&db_path, &["collection", "sync", "work"]);

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

    let output = run_gbrain(&db_path, &["--json", "collection", "sync", "work"]);

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

#[test]
fn collection_info_json_reports_restore_integrity_blockers() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-integrity.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             integrity_failed_at = '2026-04-23T00:00:00Z',
             pending_manifest_incomplete_at = '2026-04-23T00:05:00Z'
         WHERE id = ?1",
        params![
            collection_id,
            dir.path().join("restored").display().to_string()
        ],
    )
    .expect("seed integrity blockers");
    drop(conn);

    let output = run_gbrain(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["state"].as_str(), Some("restoring"));
    assert_eq!(
        parsed["blocked_state"].as_str(),
        Some("restore_integrity_blocked")
    );
    assert_eq!(
        parsed["integrity_blocked"].as_str(),
        Some("manifest_tampering")
    );
    assert_eq!(
        parsed["integrity_failed_at"].as_str(),
        Some("2026-04-23T00:00:00Z")
    );
    assert_eq!(
        parsed["pending_manifest_incomplete_at"].as_str(),
        Some("2026-04-23T00:05:00Z")
    );
    assert!(parsed["pending_root_path"].as_str().is_some());
    assert_eq!(
        parsed["suggested_command"].as_str(),
        Some("gbrain collection restore-reset work --confirm")
    );
}

#[cfg(unix)]
#[test]
fn offline_restore_can_complete_via_explicit_cli_finalize_path() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "offline-restore-cli.db");
    let conn = open_test_db(&db_path);
    let source_root = dir.path().join("source");
    let target_root = dir.path().join("restored");
    std::fs::create_dir_all(&source_root).expect("create source root");
    let collection_id = insert_collection(&conn, "work", &source_root);
    let raw_bytes =
        b"---\ngbrain_id: 11111111-1111-7111-8111-111111111111\ntitle: Restored Note\ntype: concept\n---\nhello from restore\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        raw_bytes,
        "notes/a.md",
    );
    drop(conn);

    let restore_output = run_gbrain(
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

    let info_output = run_gbrain(&db_path, &["--json", "collection", "info", "work"]);
    assert!(
        info_output.status.success(),
        "collection info should succeed: {info_output:?}"
    );
    let info_json = parse_stdout_json(&info_output);
    assert_eq!(info_json["blocked_state"].as_str(), Some("pending_attach"));
    assert_eq!(
        info_json["suggested_command"].as_str(),
        Some("gbrain collection sync work --finalize-pending")
    );

    let finalize_output = run_gbrain(
        &db_path,
        &["--json", "collection", "sync", "work", "--finalize-pending"],
    );

    assert!(
        finalize_output.status.success(),
        "explicit finalize path should reopen the restored collection: {finalize_output:?}"
    );
    let finalize_json = parse_stdout_json(&finalize_output);
    assert_eq!(finalize_json["finalize_pending"].as_str(), Some("Attached"));

    let conn = open_test_db(&db_path);
    let row: (String, String, i64, Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state, root_path, needs_full_sync, pending_root_path, integrity_failed_at, pending_manifest_incomplete_at
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
    assert_eq!(
        std::fs::read(target_root.join("notes").join("a.md")).expect("read restored file"),
        raw_bytes
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
        b"---\ngbrain_id: 11111111-1111-7111-8111-111111111111\ntitle: Restored Note\ntype: concept\n---\nhello from restore\n";
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
fn collection_info_json_points_retryable_manifest_gap_to_finalize_pending() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-manifest-gap.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_manifest_incomplete_at = '2026-04-23T00:05:00Z'
         WHERE id = ?1",
        params![
            collection_id,
            dir.path().join("restored").display().to_string()
        ],
    )
    .expect("seed retryable manifest gap");
    drop(conn);

    let output = run_gbrain(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["blocked_state"].as_str(), Some("pending_finalize"));
    assert_eq!(
        parsed["integrity_blocked"].as_str(),
        Some("manifest_incomplete_pending")
    );
    assert_eq!(
        parsed["suggested_command"].as_str(),
        Some("gbrain collection sync work --finalize-pending")
    );
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

    let output = run_gbrain(
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

    let output = run_gbrain(
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

#[test]
fn collection_info_json_reports_reconcile_halt_cause() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-halt.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET reconcile_halted_at = '2026-04-23T00:10:00Z',
             reconcile_halt_reason = 'duplicate_uuid'
         WHERE id = ?1",
        [collection_id],
    )
    .expect("seed reconcile halt");
    drop(conn);

    let output = run_gbrain(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["blocked_state"].as_str(), Some("reconcile_halted"));
    assert_eq!(parsed["integrity_blocked"].as_str(), Some("duplicate_uuid"));
    assert_eq!(
        parsed["reconcile_halted_at"].as_str(),
        Some("2026-04-23T00:10:00Z")
    );
    assert_eq!(
        parsed["reconcile_halt_reason"].as_str(),
        Some("duplicate_uuid")
    );
    assert_eq!(
        parsed["suggested_command"].as_str(),
        Some("gbrain collection reconcile-reset work --confirm")
    );
}

#[test]
fn collection_info_json_reports_read_only_truthfully() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-read-only.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET writable = 0,
             last_sync_at = '2026-04-23T00:15:00Z'
         WHERE id = ?1",
        [collection_id],
    )
    .expect("seed read-only collection");
    drop(conn);

    let output = run_gbrain(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["name"].as_str(), Some("work"));
    assert_eq!(parsed["state"].as_str(), Some("active"));
    assert_eq!(parsed["writable"].as_bool(), Some(false));
    assert_eq!(
        parsed["last_sync_at"].as_str(),
        Some("2026-04-23T00:15:00Z")
    );
}

#[test]
fn collection_list_json_reports_k1_columns_truthfully() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-list.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    insert_page(&conn, collection_id, "notes/a");
    conn.execute(
        "UPDATE collections
         SET writable = 0,
             last_sync_at = '2026-04-23T00:20:00Z'
         WHERE id = ?1",
        [collection_id],
    )
    .expect("seed collection list row");
    drop(conn);

    let output = run_gbrain(&db_path, &["--json", "collection", "list"]);

    assert!(
        output.status.success(),
        "collection list should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    let rows = parsed.as_array().expect("collection list rows");
    let row = rows
        .iter()
        .find(|row| row["name"].as_str() == Some("work"))
        .expect("work row");
    assert_eq!(row["state"].as_str(), Some("active"));
    assert_eq!(row["writable"].as_str(), Some("read-only"));
    assert_eq!(row["write_target"].as_bool(), Some(false));
    assert_eq!(
        row["root_path"].as_str(),
        Some(root.to_str().expect("utf-8 root"))
    );
    assert_eq!(row["page_count"].as_i64(), Some(1));
    assert_eq!(row["last_sync_at"].as_str(), Some("2026-04-23T00:20:00Z"));
    assert_eq!(row["queue_depth"].as_i64(), Some(0));
}

#[test]
fn put_cli_refuses_when_collection_is_persisted_read_only() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "put-read-only.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections SET writable = 0 WHERE id = ?1",
        [collection_id],
    )
    .expect("mark collection read-only");
    drop(conn);

    let output = run_gbrain_with_stdin(
        &db_path,
        &["put", "work::notes/read-only"],
        "---\ntitle: Read Only\ntype: note\n---\nhello\n",
    );

    assert!(
        !output.status.success(),
        "put should fail for read-only collection: {output:?}"
    );
    #[cfg(unix)]
    assert!(
        combined_output(&output).contains("CollectionReadOnlyError"),
        "put must surface CollectionReadOnlyError: {output:?}"
    );
    #[cfg(not(unix))]
    assert!(
        combined_output(&output).contains("UnsupportedPlatformError"),
        "Windows put must fail closed with UnsupportedPlatformError: {output:?}"
    );
}
