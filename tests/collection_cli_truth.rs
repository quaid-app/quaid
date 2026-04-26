use quaid::core::db;
use quaid::core::vault_sync;
use rusqlite::{params, Connection};
use serde_json::Value;
use sha2::Digest;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().expect("utf-8 db path")).expect("open test db")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_quaid")
}

fn run_quaid(db_path: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(bin_path());
    command.arg("--db").arg(db_path).args(args);
    command.output().expect("run quaid")
}

fn run_quaid_with_stdin(db_path: &Path, args: &[&str], stdin: &str) -> std::process::Output {
    let mut command = Command::new(bin_path());
    command
        .arg("--db")
        .arg(db_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn quaid");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for quaid")
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

fn assert_ambiguous_slug_failure(output: &std::process::Output, slug: &str, candidates: &[&str]) {
    assert!(
        !output.status.success(),
        "ambiguous bare slug should fail: {output:?}"
    );
    let text = combined_output(output);
    assert!(
        text.contains(&format!("ambiguous slug: {slug}")),
        "ambiguous bare slug must surface the routing failure: {text}"
    );
    for candidate in candidates {
        assert!(
            text.contains(candidate),
            "ambiguous bare slug must include candidate {candidate}: {text}"
        );
    }
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
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, 'note', ?2, '', 'compiled', '', '{}', 'notes', '', 1)",
        params![collection_id, slug, uuid::Uuid::now_v7().to_string()],
    )
    .expect("insert page");
}

fn insert_page_with_truth(conn: &Connection, collection_id: i64, slug: &str, truth: &str) {
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, 'note', ?2, '', ?4, '', '{}', 'notes', '', 1)",
        params![
            collection_id,
            slug,
            uuid::Uuid::now_v7().to_string(),
            truth
        ],
    )
    .expect("insert page with truth");
}

fn page_id(conn: &Connection, collection_id: i64, slug: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
        params![collection_id, slug],
        |row| row.get(0),
    )
    .expect("load page id")
}

fn insert_timeline_entry(conn: &Connection, page_id: i64, date: &str, summary: &str) {
    let summary_hash = format!("{:x}", sha2::Sha256::digest(summary.as_bytes()));
    conn.execute(
        "INSERT INTO timeline_entries (page_id, date, source, summary, summary_hash, detail)
         VALUES (?1, ?2, '', ?3, ?4, '')",
        params![page_id, date, summary, summary_hash],
    )
    .expect("insert timeline entry");
}

fn quarantine_page(conn: &Connection, page_id: i64, quarantined_at: &str) {
    conn.execute(
        "UPDATE pages SET quarantined_at = ?2 WHERE id = ?1",
        params![page_id, quarantined_at],
    )
    .expect("quarantine page");
}

fn insert_programmatic_link(conn: &Connection, from_page_id: i64, to_page_id: i64) {
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, context, source_kind)
         VALUES (?1, ?2, 'related', '', 'programmatic')",
        params![from_page_id, to_page_id],
    )
    .expect("insert programmatic link");
}

fn insert_knowledge_gap(conn: &Connection, page_id: i64, hash: &str) {
    conn.execute(
        "INSERT INTO knowledge_gaps (page_id, query_hash, context)
         VALUES (?1, ?2, 'gap context')",
        params![page_id, hash],
    )
    .expect("insert knowledge gap");
}

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

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

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
        Some("quaid collection restore-reset work --confirm")
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
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\ntitle: Restored Note\ntype: concept\n---\nhello from restore\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        raw_bytes,
        "notes/a.md",
    );
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

    let info_output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);
    assert!(
        info_output.status.success(),
        "collection info should succeed: {info_output:?}"
    );
    let info_json = parse_stdout_json(&info_output);
    assert_eq!(info_json["blocked_state"].as_str(), Some("pending_attach"));
    assert_eq!(
        info_json["suggested_command"].as_str(),
        Some("quaid collection sync work --finalize-pending")
    );

    let finalize_output = run_quaid(
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

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

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
        Some("quaid collection sync work --finalize-pending")
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

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

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
        Some("quaid collection reconcile-reset work --confirm")
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

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

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
fn collection_info_json_reports_quarantine_backlog_count() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-quarantine.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    insert_page(&conn, collection_id, "notes/active");
    insert_page(&conn, collection_id, "notes/quarantined");
    quarantine_page(
        &conn,
        page_id(&conn, collection_id, "notes/quarantined"),
        "2026-04-25T00:00:00Z",
    );
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(
        parsed["quarantined_pages_awaiting_action"].as_i64(),
        Some(1)
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

    let output = run_quaid(&db_path, &["--json", "collection", "list"]);

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
fn quarantine_export_then_discard_without_force_succeeds_after_export() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "quarantine-export-discard.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    insert_page_with_truth(&conn, collection_id, "notes/quarantined", "truth");
    insert_page_with_truth(&conn, collection_id, "notes/target", "target");
    let quarantined_page_id = page_id(&conn, collection_id, "notes/quarantined");
    let target_page_id = page_id(&conn, collection_id, "notes/target");
    quarantine_page(&conn, quarantined_page_id, "2026-04-25T00:00:00Z");
    insert_programmatic_link(&conn, quarantined_page_id, target_page_id);
    insert_knowledge_gap(&conn, quarantined_page_id, "gap-quarantine-export");
    drop(conn);

    let discard_before_export = run_quaid(
        &db_path,
        &[
            "collection",
            "quarantine",
            "discard",
            "work::notes/quarantined",
        ],
    );
    assert!(
        !discard_before_export.status.success(),
        "discard without force/export must fail: {discard_before_export:?}"
    );
    assert!(
        combined_output(&discard_before_export).contains("QuarantineDiscardExportRequiredError"),
        "discard failure must explain export requirement: {discard_before_export:?}"
    );

    let export_path = dir.path().join("quarantine-export.json");
    let export_output = run_quaid(
        &db_path,
        &[
            "--json",
            "collection",
            "quarantine",
            "export",
            "work::notes/quarantined",
            export_path.to_str().expect("utf-8 export path"),
        ],
    );
    assert!(
        export_output.status.success(),
        "quarantine export should succeed: {export_output:?}"
    );
    let export_json = parse_stdout_json(&export_output);
    assert_eq!(export_json["command"].as_str(), Some("quarantine-export"));
    let exported_payload: Value = serde_json::from_slice(
        &std::fs::read(&export_path).expect("read exported quarantine json"),
    )
    .expect("export payload json");
    assert_eq!(
        export_json["exported_at"].as_str(),
        exported_payload["exported_at"].as_str()
    );
    assert_eq!(
        exported_payload["knowledge_gaps"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        exported_payload["programmatic_links"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let verify_export = open_test_db(&db_path);
    let stored_exported_at: String = verify_export
        .query_row(
            "SELECT exported_at
             FROM quarantine_exports
             WHERE page_id = ?1 AND quarantined_at = '2026-04-25T00:00:00Z'",
            [quarantined_page_id],
            |row| row.get(0),
        )
        .expect("load stored export timestamp");
    assert_eq!(
        export_json["exported_at"].as_str(),
        Some(stored_exported_at.as_str())
    );
    drop(verify_export);

    let discard_output = run_quaid(
        &db_path,
        &[
            "--json",
            "collection",
            "quarantine",
            "discard",
            "work::notes/quarantined",
        ],
    );
    assert!(
        discard_output.status.success(),
        "discard should succeed after export: {discard_output:?}"
    );
    let discard_json = parse_stdout_json(&discard_output);
    assert_eq!(discard_json["command"].as_str(), Some("quarantine-discard"));
    assert_eq!(
        discard_json["exported_before_discard"].as_bool(),
        Some(true)
    );

    let verify = open_test_db(&db_path);
    let remaining: i64 = verify
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = ?1 AND slug = 'notes/quarantined'",
            [collection_id],
            |row| row.get(0),
        )
        .expect("count remaining quarantined page");
    assert_eq!(remaining, 0);
}

#[test]
fn quarantine_list_missing_collection_reports_collection_specific_error() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "quarantine-list-missing-collection.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    insert_collection(&conn, "work", &root);
    drop(conn);

    let output = run_quaid(&db_path, &["collection", "quarantine", "list", "missing"]);

    assert!(
        !output.status.success(),
        "missing collection list must fail: {output:?}"
    );
    let text = combined_output(&output);
    assert!(
        text.contains("quarantine collection not found: missing"),
        "missing collection list must surface the collection-specific error: {text}"
    );
    assert!(
        !text.contains("quarantined page not found"),
        "missing collection list must not report a page-not-found error: {text}"
    );
}

#[test]
fn quarantine_restore_reingests_page_and_reactivates_file_state() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "quarantine-restore.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(root.join("notes")).expect("create notes dir");
    let collection_id = insert_collection(&conn, "work", &root);
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/quarantined",
        "11111111-1111-7111-8111-111111111111",
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\ntitle: Restored\ntype: concept\n---\nrestored body\n",
        "notes/original.md",
    );
    let quarantined_page_id = page_id(&conn, collection_id, "notes/quarantined");
    quarantine_page(&conn, quarantined_page_id, "2026-04-25T00:00:00Z");
    conn.execute(
        "DELETE FROM file_state WHERE page_id = ?1",
        [quarantined_page_id],
    )
    .expect("remove file_state");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "--json",
            "collection",
            "quarantine",
            "restore",
            "work::notes/quarantined",
            "notes/restored",
        ],
    );

    #[cfg(not(unix))]
    assert!(
        !output.status.success(),
        "quarantine restore is Unix-only today: {output:?}"
    );
    #[cfg(not(unix))]
    let stderr = String::from_utf8_lossy(&output.stderr);
    #[cfg(not(unix))]
    assert!(
        stderr.contains("UnsupportedPlatformError"),
        "Windows must fail closed on the quarantine restore write surface: {stderr}"
    );
    #[cfg(not(unix))]
    {
        let verify = open_test_db(&db_path);
        let row: (Option<String>, Option<String>) = verify
            .query_row(
                "SELECT quarantined_at,
                        (SELECT relative_path FROM file_state WHERE page_id = ?1 LIMIT 1)
                 FROM pages
                 WHERE id = ?1",
                [quarantined_page_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("load restored page");
        assert!(row.0.is_some());
        assert_eq!(row.1, None);
        assert!(
            !root.join("notes").join("restored.md").exists(),
            "unsupported restore must not write vault bytes"
        );
    }

    #[cfg(unix)]
    {
        let verify = open_test_db(&db_path);
        let row: (Option<String>, Option<String>, String) = verify
            .query_row(
                "SELECT quarantined_at,
                        (SELECT relative_path FROM file_state WHERE page_id = ?1 LIMIT 1)
                        ,slug
                 FROM pages
                 WHERE id = ?1",
                [quarantined_page_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("load restored page");
        assert_eq!(row.0, None, "restored page must no longer be quarantined");
        assert_eq!(row.1.as_deref(), Some("notes/restored.md"));
        assert_eq!(row.2, "notes/restored");
        let payload = parse_stdout_json(&output);
        assert_eq!(payload["command"], "quarantine-restore");
        assert_eq!(payload["restored_slug"], "notes/restored");
        assert_eq!(payload["restored_relative_path"], "notes/restored.md");
        assert!(
            output.status.success(),
            "quarantine restore should succeed on Unix: {output:?}"
        );
        assert_eq!(
            std::fs::read(root.join("notes").join("restored.md")).expect("read restored bytes"),
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\ntitle: Restored\ntype: concept\n---\nrestored body\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn quarantine_restore_reingests_page_and_reactivates_file_state_at_target_markdown_path() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "quarantine-restore-happy-path.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(root.join("notes")).expect("create notes dir");
    let collection_id = insert_collection(&conn, "work", &root);
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\ntitle: Restored\ntype: concept\n---\nrestored body\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/quarantined",
        "11111111-1111-7111-8111-111111111111",
        raw_bytes,
        "notes/original.md",
    );
    let quarantined_page_id = page_id(&conn, collection_id, "notes/quarantined");
    quarantine_page(&conn, quarantined_page_id, "2026-04-25T00:00:00Z");
    conn.execute(
        "DELETE FROM file_state WHERE page_id = ?1",
        [quarantined_page_id],
    )
    .expect("remove file_state");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "collection",
            "quarantine",
            "restore",
            "work::notes/quarantined",
            "notes/restored",
        ],
    );

    assert!(
        output.status.success(),
        "quarantine restore happy path must succeed: {output:?}"
    );
    let verify = open_test_db(&db_path);
    let row: (Option<String>, Option<String>, i64) = verify
        .query_row(
            "SELECT quarantined_at,
                    (SELECT relative_path FROM file_state WHERE page_id = ?1 LIMIT 1),
                    (SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1 AND is_active = 1)
             FROM pages
             WHERE id = ?1",
            [quarantined_page_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load restored page");
    assert_eq!(row.0, None, "page must leave quarantine after restore");
    assert_eq!(
        row.1.as_deref(),
        Some("notes/restored.md"),
        "file_state must reactivate at the restored markdown path"
    );
    assert_eq!(
        row.2, 1,
        "restore must leave exactly one active raw_import row"
    );
    assert_eq!(
        std::fs::read(root.join("notes").join("restored.md")).expect("read restored bytes"),
        raw_bytes,
        "restored vault bytes must match the active raw import content"
    );
}

#[test]
fn start_serve_runtime_sweeps_expired_clean_quarantines_but_keeps_db_only_state_pages() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "quarantine-startup-sweep.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    insert_page(&conn, collection_id, "notes/clean");
    insert_page(&conn, collection_id, "notes/kept");
    let clean_page_id = page_id(&conn, collection_id, "notes/clean");
    let kept_page_id = page_id(&conn, collection_id, "notes/kept");
    quarantine_page(&conn, clean_page_id, "2026-01-01T00:00:00Z");
    quarantine_page(&conn, kept_page_id, "2026-01-01T00:00:00Z");
    insert_knowledge_gap(&conn, kept_page_id, "gap-startup-sweep");
    drop(conn);

    let runtime =
        vault_sync::start_serve_runtime(db_path.to_str().expect("utf-8 db path").to_owned())
            .expect("start serve runtime");
    drop(runtime);

    let verify = open_test_db(&db_path);
    let clean_exists: i64 = verify
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE id = ?1",
            [clean_page_id],
            |row| row.get(0),
        )
        .expect("count clean page");
    let kept_quarantined_at: Option<String> = verify
        .query_row(
            "SELECT quarantined_at FROM pages WHERE id = ?1",
            [kept_page_id],
            |row| row.get(0),
        )
        .expect("load kept page");
    assert_eq!(clean_exists, 0);
    assert_eq!(kept_quarantined_at.as_deref(), Some("2026-01-01T00:00:00Z"));
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

    let output = run_quaid_with_stdin(
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

#[test]
fn cli_get_accepts_explicit_collection_slug_and_rejects_ambiguous_bare_slug() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-get-parity.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/meeting");
    insert_page(&conn, memory_id, "notes/meeting");
    drop(conn);

    let ambiguous = run_quaid(&db_path, &["get", "notes/meeting"]);
    assert_ambiguous_slug_failure(
        &ambiguous,
        "notes/meeting",
        &["work::notes/meeting", "memory::notes/meeting"],
    );

    let explicit = run_quaid(&db_path, &["--json", "get", "work::notes/meeting"]);
    assert!(
        explicit.status.success(),
        "explicit collection slug should succeed: {explicit:?}"
    );
    let parsed = parse_stdout_json(&explicit);
    assert_eq!(parsed["slug"].as_str(), Some("work::notes/meeting"));
    assert_eq!(
        parsed["frontmatter"]["slug"].as_str(),
        Some("work::notes/meeting")
    );
}

#[test]
fn cli_query_rejects_ambiguous_exact_slug_input() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-query-ambiguous.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page_with_truth(&conn, work_id, "notes/meeting", "work note");
    insert_page_with_truth(&conn, memory_id, "notes/meeting", "memory note");
    drop(conn);

    let bare = run_quaid(&db_path, &["query", "notes/meeting"]);
    assert_ambiguous_slug_failure(
        &bare,
        "notes/meeting",
        &["work::notes/meeting", "memory::notes/meeting"],
    );

    let bracketed = run_quaid(&db_path, &["query", "[[notes/meeting]]"]);
    assert_ambiguous_slug_failure(
        &bracketed,
        "notes/meeting",
        &["work::notes/meeting", "memory::notes/meeting"],
    );

    let explicit = run_quaid(&db_path, &["--json", "query", "work::notes/meeting"]);
    assert!(
        explicit.status.success(),
        "explicit collection slug should route query successfully: {explicit:?}"
    );
    let parsed = parse_stdout_json(&explicit);
    assert_eq!(parsed[0]["slug"].as_str(), Some("work::notes/meeting"));
}

#[test]
fn cli_read_slug_commands_reject_ambiguous_bare_slugs() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-read-ambiguous.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/shared");
    insert_page(&conn, memory_id, "notes/shared");
    drop(conn);

    let candidates = ["work::notes/shared", "memory::notes/shared"];

    let graph = run_quaid(&db_path, &["graph", "notes/shared", "--depth", "1"]);
    assert_ambiguous_slug_failure(&graph, "notes/shared", &candidates);

    let timeline = run_quaid(&db_path, &["timeline", "notes/shared"]);
    assert_ambiguous_slug_failure(&timeline, "notes/shared", &candidates);

    let links = run_quaid(&db_path, &["links", "notes/shared"]);
    assert_ambiguous_slug_failure(&links, "notes/shared", &candidates);

    let backlinks = run_quaid(&db_path, &["backlinks", "notes/shared"]);
    assert_ambiguous_slug_failure(&backlinks, "notes/shared", &candidates);
}

#[test]
fn cli_write_slug_commands_reject_ambiguous_bare_slugs() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-write-ambiguous.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page_with_truth(
        &conn,
        work_id,
        "notes/shared",
        "## Assertions\nAlice works at Acme.\n",
    );
    insert_page_with_truth(
        &conn,
        memory_id,
        "notes/shared",
        "## Assertions\nAlice works at Beta.\n",
    );
    insert_page(&conn, work_id, "notes/target");
    drop(conn);

    let candidates = ["work::notes/shared", "memory::notes/shared"];

    let check = run_quaid(&db_path, &["check", "notes/shared"]);
    assert_ambiguous_slug_failure(&check, "notes/shared", &candidates);

    let link = run_quaid(
        &db_path,
        &[
            "link",
            "notes/shared",
            "work::notes/target",
            "--relationship",
            "relates",
        ],
    );
    assert_ambiguous_slug_failure(&link, "notes/shared", &candidates);

    let unlink = run_quaid(&db_path, &["unlink", "notes/shared", "work::notes/target"]);
    assert_ambiguous_slug_failure(&unlink, "notes/shared", &candidates);
}

#[test]
fn cli_unlink_no_match_reports_canonical_resolved_addresses() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-unlink-canonical.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/a");
    insert_page(&conn, memory_id, "notes/b");
    drop(conn);

    let output = run_quaid(&db_path, &["unlink", "notes/a", "notes/b"]);
    assert!(
        !output.status.success(),
        "unlink should fail when no matching link exists: {output:?}"
    );
    let text = combined_output(&output);
    assert!(
        text.contains("no matching link found between work::notes/a and memory::notes/b"),
        "unlink should report canonical resolved addresses on the no-match path: {text}"
    );
}

#[test]
fn cli_unlink_accepts_explicit_collection_slugs() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-unlink-explicit.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/a");
    insert_page(&conn, memory_id, "notes/a");
    insert_page(&conn, work_id, "notes/b");
    insert_page(&conn, memory_id, "notes/b");
    drop(conn);

    let link = run_quaid(
        &db_path,
        &[
            "link",
            "work::notes/a",
            "memory::notes/b",
            "--relationship",
            "relates",
        ],
    );
    assert!(link.status.success(), "setup link should succeed: {link:?}");

    let unlink = run_quaid(
        &db_path,
        &[
            "unlink",
            "work::notes/a",
            "memory::notes/b",
            "--relationship",
            "relates",
        ],
    );
    assert!(
        unlink.status.success(),
        "explicit collection slug should route unlink successfully: {unlink:?}"
    );
    let text = String::from_utf8_lossy(&unlink.stdout);
    assert!(
        text.contains("Removed 1 link(s) work::notes/a → memory::notes/b"),
        "unlink should report canonical explicit addresses: {text}"
    );

    let conn = open_test_db(&db_path);
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM links l
             JOIN pages fp ON fp.id = l.from_page_id
             JOIN pages tp ON tp.id = l.to_page_id
             JOIN collections fc ON fc.id = fp.collection_id
             JOIN collections tc ON tc.id = tp.collection_id
             WHERE fc.name = 'work'
               AND fp.slug = 'notes/a'
               AND tc.name = 'memory'
               AND tp.slug = 'notes/b'
               AND l.relationship = 'relates'",
            [],
            |row| row.get(0),
        )
        .expect("count remaining explicit link");
    assert_eq!(remaining, 0);
}

#[test]
fn cli_link_views_and_graph_emit_canonical_page_addresses() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-link-graph-parity.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/a");
    insert_page(&conn, memory_id, "notes/b");
    drop(conn);

    let link_output = run_quaid(
        &db_path,
        &[
            "link",
            "work::notes/a",
            "memory::notes/b",
            "--relationship",
            "relates",
        ],
    );
    assert!(
        link_output.status.success(),
        "link should succeed: {link_output:?}"
    );
    let link_text = String::from_utf8_lossy(&link_output.stdout);
    assert!(link_text.contains("work::notes/a"));
    assert!(link_text.contains("memory::notes/b"));

    let outbound = run_quaid(&db_path, &["--json", "links", "work::notes/a"]);
    assert!(
        outbound.status.success(),
        "links should succeed: {outbound:?}"
    );
    let outbound_json = parse_stdout_json(&outbound);
    assert_eq!(
        outbound_json[0]["to_slug"].as_str(),
        Some("memory::notes/b")
    );

    let inbound = run_quaid(&db_path, &["--json", "backlinks", "memory::notes/b"]);
    assert!(
        inbound.status.success(),
        "backlinks should succeed: {inbound:?}"
    );
    let inbound_json = parse_stdout_json(&inbound);
    assert_eq!(inbound_json[0]["from_slug"].as_str(), Some("work::notes/a"));

    let graph = run_quaid(
        &db_path,
        &["--json", "graph", "work::notes/a", "--depth", "1"],
    );
    assert!(graph.status.success(), "graph should succeed: {graph:?}");
    let graph_json = parse_stdout_json(&graph);
    let node_slugs: Vec<_> = graph_json["nodes"]
        .as_array()
        .expect("graph nodes")
        .iter()
        .map(|node| node["slug"].as_str().expect("node slug"))
        .collect();
    assert!(node_slugs.contains(&"work::notes/a"));
    assert!(node_slugs.contains(&"memory::notes/b"));
    assert_eq!(
        graph_json["edges"][0]["from"].as_str(),
        Some("work::notes/a")
    );
    assert_eq!(
        graph_json["edges"][0]["to"].as_str(),
        Some("memory::notes/b")
    );
}

#[test]
fn cli_timeline_and_check_emit_canonical_slugs_for_explicit_routes() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-timeline-check-parity.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    std::fs::create_dir_all(&work_root).expect("create work root");
    let work_id = insert_collection(&conn, "work", &work_root);
    insert_page_with_truth(
        &conn,
        work_id,
        "people/alice",
        "## Assertions\nAlice works at Acme Corp.\n",
    );
    insert_page_with_truth(
        &conn,
        work_id,
        "sources/alice-profile",
        "## Assertions\nAlice works at Beta Corp.\n",
    );
    insert_timeline_entry(
        &conn,
        page_id(&conn, work_id, "people/alice"),
        "2026-04-24",
        "joined",
    );
    drop(conn);

    let timeline = run_quaid(&db_path, &["--json", "timeline", "work::people/alice"]);
    assert!(
        timeline.status.success(),
        "timeline should succeed for explicit slug: {timeline:?}"
    );
    let timeline_json = parse_stdout_json(&timeline);
    assert_eq!(timeline_json["slug"].as_str(), Some("work::people/alice"));

    let warmup = run_quaid(&db_path, &["check", "--all"]);
    assert!(
        warmup.status.success(),
        "all-mode check should seed contradiction rows: {warmup:?}"
    );

    let check = run_quaid(&db_path, &["--json", "check", "work::people/alice"]);
    assert!(
        check.status.success(),
        "check should succeed for explicit slug: {check:?}"
    );
    let check_json = parse_stdout_json(&check);
    assert_eq!(
        check_json[0]["page_slug"].as_str(),
        Some("work::people/alice")
    );
    assert_eq!(
        check_json[0]["other_page_slug"].as_str(),
        Some("work::sources/alice-profile")
    );
}

#[test]
fn cli_list_search_and_query_emit_canonical_slugs() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-search-query-list-parity.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page_with_truth(
        &conn,
        work_id,
        "people/alice",
        "Alice is the founder of Acme.\n",
    );
    insert_page_with_truth(
        &conn,
        memory_id,
        "people/bob",
        "Bob works on distributed systems.\n",
    );
    drop(conn);

    let list = run_quaid(&db_path, &["--json", "list"]);
    assert!(list.status.success(), "list should succeed: {list:?}");
    let list_json = parse_stdout_json(&list);
    let list_slugs: Vec<_> = list_json
        .as_array()
        .expect("list rows")
        .iter()
        .map(|row| row["slug"].as_str().expect("list slug"))
        .collect();
    assert!(list_slugs.contains(&"work::people/alice"));
    assert!(list_slugs.contains(&"memory::people/bob"));

    let search = run_quaid(&db_path, &["--json", "search", "founder"]);
    assert!(search.status.success(), "search should succeed: {search:?}");
    let search_json = parse_stdout_json(&search);
    assert_eq!(search_json[0]["slug"].as_str(), Some("work::people/alice"));

    let query = run_quaid(&db_path, &["--json", "query", "people/alice"]);
    assert!(query.status.success(), "query should succeed: {query:?}");
    let query_json = parse_stdout_json(&query);
    assert_eq!(query_json[0]["slug"].as_str(), Some("work::people/alice"));
}
