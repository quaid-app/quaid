/// Targeted tests for quarantine restore's narrow re-enable gate.
use gbrain::core::db;
use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::process::{Command, Output};
use tempfile::TempDir;

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().expect("utf-8 db path")).expect("open test db")
}

fn insert_collection(conn: &Connection, name: &str, root: &Path, writable: bool) -> i64 {
    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target)
         VALUES (?1, ?2, 'active', ?3, 0)",
        params![name, root.display().to_string(), i64::from(writable)],
    )
    .expect("insert collection");
    conn.last_insert_rowid()
}

fn insert_quarantined_page(
    conn: &Connection,
    collection_id: i64,
    slug: &str,
    raw_bytes: &[u8],
) -> i64 {
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version, quarantined_at)
         VALUES (?1, ?2, ?3, 'note', ?2, '', 'truth', '', '{}', 'notes', '', 1, '2026-04-25T00:00:00Z')",
        params![collection_id, slug, uuid::Uuid::now_v7().to_string()],
    )
    .expect("insert page");
    let page_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, raw_bytes, file_path, is_active)
         VALUES (?1, 'initial', ?2, '', 1)",
        params![page_id, raw_bytes],
    )
    .expect("insert raw_import");
    page_id
}

#[cfg(unix)]
fn run_restore(db_path: &Path, slug: &str, relative_path: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("restore")
        .arg(slug)
        .arg(relative_path)
        .output()
        .expect("run restore")
}

#[cfg(unix)]
fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn blocker_1_failed_export_does_not_unlock_discard() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root, true);
    let page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );
    conn.execute(
        "INSERT INTO knowledge_gaps (page_id, query_hash, context) VALUES (?1, 'gap', 'context')",
        [page_id],
    )
    .expect("add db-only state");

    // Create a file at the location where the parent directory would be,
    // making fs::create_dir_all fail
    let blocked_parent = dir.path().join("blocked");
    fs::write(&blocked_parent, b"file blocks dir").expect("create blocker file");
    let export_path = blocked_parent.join("subdir").join("out.json");
    drop(conn);

    let export_result = std::process::Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("export")
        .arg("work::notes/quarantined")
        .arg(&export_path)
        .output()
        .expect("run export");

    assert!(
        !export_result.status.success(),
        "export with blocked parent directory should fail: {export_result:?}"
    );

    let conn = open_test_db(&db_path);
    let export_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM quarantine_exports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .expect("count exports");
    assert_eq!(
        export_exists, 0,
        "failed export must not record quarantine_exports row"
    );

    drop(conn);

    let discard_result = std::process::Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("discard")
        .arg("work::notes/quarantined")
        .output()
        .expect("run discard");

    assert!(
        !discard_result.status.success(),
        "discard without export or --force must fail: {discard_result:?}"
    );
    let output_text = String::from_utf8_lossy(&discard_result.stderr);
    assert!(
        output_text.contains("QuarantineDiscardExportRequiredError"),
        "discard must surface export-required error: {output_text}"
    );

    let conn = open_test_db(&db_path);
    let page_still_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .expect("count pages");
    assert_eq!(
        page_still_exists, 1,
        "page must still exist after failed discard"
    );
}

#[cfg(unix)]
#[test]
fn restore_rejects_non_markdown_target() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root, true);
    let _page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );
    drop(conn);

    let restore_result = run_restore(&db_path, "work::notes/quarantined", "notes/restored.txt");

    assert!(
        !restore_result.status.success(),
        "restore to non-.md target should fail: {restore_result:?}"
    );
    let output_text = String::from_utf8_lossy(&restore_result.stderr);
    assert!(
        output_text.contains("QuarantineRestoreTargetNotMarkdownError"),
        "restore must surface the Markdown-target error: {output_text}"
    );

    assert!(
        !root.join("notes").join("restored.txt").exists(),
        "no .txt file should be written"
    );
}

#[cfg(unix)]
#[test]
fn restore_refuses_live_owned_collection() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root, true);
    let _page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );

    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('live-owner', 12345, 'testhost', datetime('now'))",
        [],
    )
    .expect("insert live session");
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id)
         VALUES (?1, 'live-owner')",
        [collection_id],
    )
    .expect("establish live owner");
    drop(conn);

    let restore_result = run_restore(&db_path, "work::notes/quarantined", "notes/restored");

    assert!(
        !restore_result.status.success(),
        "restore when serve owns collection should fail: {restore_result:?}"
    );
    let output_text = String::from_utf8_lossy(&restore_result.stderr);
    assert!(
        output_text.contains("ServeOwnsCollectionError"),
        "restore must preserve the live-owner refusal: {output_text}"
    );

    assert!(
        !root.join("notes").join("restored.md").exists(),
        "no file should be written when serve owns collection"
    );
}

#[cfg(unix)]
#[test]
fn restore_refuses_existing_target_without_mutation() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root, true);
    let page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );

    fs::create_dir_all(root.join("notes")).expect("create notes dir");
    fs::write(root.join("notes").join("conflict.md"), b"existing")
        .expect("create conflicting file");
    drop(conn);

    let restore_result = run_restore(&db_path, "work::notes/quarantined", "notes/conflict");

    assert!(
        !restore_result.status.success(),
        "restore to existing target should fail: {restore_result:?}"
    );
    let output_text = String::from_utf8_lossy(&restore_result.stderr);
    assert!(
        output_text.contains("QuarantineRestoreTargetOccupiedError"),
        "restore must surface the occupied-target refusal: {output_text}"
    );

    let conn = open_test_db(&db_path);
    let quarantined_at: Option<String> = conn
        .query_row(
            "SELECT quarantined_at FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .expect("load page");
    assert!(
        quarantined_at.is_some(),
        "page must still be quarantined after failed restore"
    );

    let file_state_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .expect("count file_state");
    assert_eq!(
        file_state_exists, 0,
        "file_state must not exist when restore fails"
    );

    let exports_deleted: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM quarantine_exports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .expect("count exports");
    assert_eq!(
        exports_deleted, 0,
        "export records must remain absent when restore fails"
    );

    let temp_files: Vec<_> = fs::read_dir(root.join("notes"))
        .expect("read notes dir")
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".quarantine-restore-")
        })
        .collect();
    assert!(
        temp_files.is_empty(),
        "no temp files should remain on disk after failed restore"
    );
}

#[cfg(unix)]
#[test]
fn restore_refuses_read_only_collection() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(&root).expect("create root");
    // writable=false: collection is read-only (vault-byte writes must be refused)
    let collection_id = insert_collection(&conn, "readonly", &root, false);
    let _page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );
    drop(conn);

    let restore_result = run_restore(&db_path, "readonly::notes/quarantined", "notes/restored");

    assert!(
        !restore_result.status.success(),
        "restore to read-only collection should fail: {restore_result:?}"
    );
    let output_text = String::from_utf8_lossy(&restore_result.stderr);
    assert!(
        output_text.contains("CollectionReadOnlyError"),
        "restore must surface the read-only vault-byte gate: {output_text}"
    );

    assert!(
        !root.join("notes").join("restored.md").exists(),
        "no file should be written when collection is read-only"
    );
}

#[cfg(unix)]
#[test]
fn restore_refuses_when_target_appears_after_the_earlier_absence_check() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(root.join("notes")).expect("create notes dir");
    let collection_id = insert_collection(&conn, "work", &root, true);
    let page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );
    drop(conn);

    let pause_file = dir.path().join("restore.pause");
    let mut child = Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("restore")
        .arg("work::notes/quarantined")
        .arg("notes/restored")
        .env("GBRAIN_TEST_QUARANTINE_RESTORE_PAUSE_FILE", &pause_file)
        .spawn()
        .expect("spawn restore");

    while !pause_file.exists() {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    fs::write(root.join("notes").join("restored.md"), b"competing target")
        .expect("write competing target");
    fs::remove_file(&pause_file).expect("release restore");
    let restore_result = child.wait_with_output().expect("wait for restore");

    assert!(
        !restore_result.status.success(),
        "restore must fail closed when target appears after precheck: {restore_result:?}"
    );
    let output_text = combined_output(&restore_result);
    assert!(
        output_text.contains("QuarantineRestoreTargetOccupiedError"),
        "restore must report the install-time no-replace refusal: {output_text}"
    );
    assert_eq!(
        fs::read(root.join("notes").join("restored.md")).expect("read competing target"),
        b"competing target"
    );

    let conn = open_test_db(&db_path);
    let state: (Option<String>, i64) = conn
        .query_row(
            "SELECT quarantined_at,
                    (SELECT COUNT(*) FROM file_state WHERE page_id = ?1)
             FROM pages
             WHERE id = ?1",
            [page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load quarantined state");
    assert!(state.0.is_some(), "page must remain quarantined");
    assert_eq!(state.1, 0, "file_state must remain inactive");
    let residue: Vec<_> = fs::read_dir(root.join("notes"))
        .expect("read notes dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".quarantine-restore-")
        })
        .collect();
    assert!(residue.is_empty(), "restore must leave no tempfile residue");
}

#[cfg(unix)]
#[test]
fn restore_cleans_up_tempfile_when_write_fails() {
    // Blocker 1: pre-install write_all/sync_all failure must not leave
    // a `.quarantine-restore-*.tmp` residue on disk.
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(root.join("notes")).expect("create notes dir");
    let collection_id = insert_collection(&conn, "work", &root, true);
    let page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );
    drop(conn);

    let restore_result = Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("restore")
        .arg("work::notes/quarantined")
        .arg("notes/restored")
        .env(
            "GBRAIN_TEST_QUARANTINE_RESTORE_FAIL_AFTER_TEMPFILE_CREATE",
            "1",
        )
        .output()
        .expect("run restore");

    assert!(
        !restore_result.status.success(),
        "injected tempfile-create failure must cause restore to fail: {restore_result:?}"
    );
    let output_text = combined_output(&restore_result);
    assert!(
        output_text.contains("QuarantineRestoreHookError"),
        "injected failure must surface hook error: {output_text}"
    );

    assert!(
        !root.join("notes").join("restored.md").exists(),
        "no target file should be written"
    );
    let residue: Vec<_> = fs::read_dir(root.join("notes"))
        .expect("read notes dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".quarantine-restore-")
        })
        .collect();
    assert!(
        residue.is_empty(),
        "pre-install failure must leave no tempfile residue: {residue:?}"
    );

    let conn = open_test_db(&db_path);
    let quarantined: Option<String> = conn
        .query_row(
            "SELECT quarantined_at FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .expect("load page");
    assert!(
        quarantined.is_some(),
        "page must still be quarantined after failed restore"
    );
}

#[cfg(unix)]
#[test]
fn restore_rolls_back_target_when_parse_fails_after_install() {
    // Blocker 3: a parse-time error after the file is installed must roll back the
    // installed target, leaving no orphaned vault bytes with the page still quarantined.
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(root.join("notes")).expect("create notes dir");
    let collection_id = insert_collection(&conn, "work", &root, true);
    let page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );
    drop(conn);

    let restore_result = Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("restore")
        .arg("work::notes/quarantined")
        .arg("notes/restored")
        .env("GBRAIN_TEST_QUARANTINE_RESTORE_FAIL_IN_PARSE", "1")
        .output()
        .expect("run restore");

    assert!(
        !restore_result.status.success(),
        "injected parse failure must cause restore to fail: {restore_result:?}"
    );
    let output_text = combined_output(&restore_result);
    assert!(
        output_text.contains("QuarantineRestoreHookError"),
        "injected parse failure must surface hook error: {output_text}"
    );

    assert!(
        !root.join("notes").join("restored.md").exists(),
        "parse-failure rollback must leave no installed target on disk"
    );
    let residue: Vec<_> = fs::read_dir(root.join("notes"))
        .expect("read notes dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".quarantine-restore-")
        })
        .collect();
    assert!(
        residue.is_empty(),
        "parse-failure rollback must leave no tempfile residue"
    );

    let conn = open_test_db(&db_path);
    let state: (Option<String>, i64) = conn
        .query_row(
            "SELECT quarantined_at,
                    (SELECT COUNT(*) FROM file_state WHERE page_id = ?1)
             FROM pages
             WHERE id = ?1",
            [page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load quarantined state");
    assert!(
        state.0.is_some(),
        "page must remain quarantined after parse-failure rollback"
    );
    assert_eq!(
        state.1, 0,
        "file_state must remain inactive after parse-failure rollback"
    );
}

#[cfg(unix)]
#[test]
fn restore_refuses_absent_parent_directory() {
    // Blocker 4: restore now refuses targets whose parent directory does not exist.
    // Previously walk_to_parent_create_dirs would silently create it without durably
    // fsyncing the new chain. The narrow fix is to require the caller to pre-create
    // the directory, keeping the slice within crash-durable install semantics only.
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(&root).expect("create vault root");
    // Note: root/notes does NOT exist — parent directory is absent.
    let collection_id = insert_collection(&conn, "work", &root, true);
    let _page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );
    drop(conn);

    let restore_result = run_restore(&db_path, "work::notes/quarantined", "notes/restored");

    assert!(
        !restore_result.status.success(),
        "restore with absent parent directory must fail: {restore_result:?}"
    );
    // Parent-absent failure surfaces as an I/O error (NotFound).
    assert!(
        !root.join("notes").exists(),
        "absent parent directory must not be created by restore"
    );
    assert!(
        !root.join("notes").join("restored.md").exists(),
        "no file should be written when parent is absent"
    );
}

#[cfg(unix)]
#[test]
fn restore_rollback_unlinks_residue_and_fsyncs_parent_before_returning() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    fs::create_dir_all(root.join("notes")).expect("create notes dir");
    let collection_id = insert_collection(&conn, "work", &root, true);
    let page_id = insert_quarantined_page(
        &conn,
        collection_id,
        "notes/quarantined",
        b"---\ntitle: Q\ntype: note\n---\nquarantined\n",
    );
    drop(conn);

    let trace_file = dir.path().join("restore.trace");
    let restore_result = Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("restore")
        .arg("work::notes/quarantined")
        .arg("notes/restored")
        .env("GBRAIN_TEST_QUARANTINE_RESTORE_FAIL_AFTER_INSTALL", "1")
        .env("GBRAIN_TEST_QUARANTINE_RESTORE_TRACE_FILE", &trace_file)
        .output()
        .expect("run restore");

    assert!(
        !restore_result.status.success(),
        "restore hook must force a post-install rollback: {restore_result:?}"
    );
    let output_text = combined_output(&restore_result);
    assert!(
        output_text.contains("QuarantineRestoreHookError"),
        "injected restore failure must surface the hook error: {output_text}"
    );
    assert!(
        !root.join("notes").join("restored.md").exists(),
        "rolled-back restore must leave no target bytes behind"
    );
    let residue: Vec<_> = fs::read_dir(root.join("notes"))
        .expect("read notes dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".quarantine-restore-")
        })
        .collect();
    assert!(
        residue.is_empty(),
        "rollback must leave no tempfile residue"
    );

    let trace = fs::read_to_string(&trace_file).expect("read trace");
    let events: Vec<_> = trace.lines().collect();
    assert_eq!(
        events,
        vec![
            "unlink:temp",
            "fsync-after-unlink:temp",
            "unlink:target",
            "fsync-after-unlink:target"
        ],
        "rollback must fsync the parent after every successful unlink: {trace}"
    );

    let conn = open_test_db(&db_path);
    let state: (Option<String>, i64) = conn
        .query_row(
            "SELECT quarantined_at,
                    (SELECT COUNT(*) FROM file_state WHERE page_id = ?1)
             FROM pages
             WHERE id = ?1",
            [page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load quarantined state");
    assert!(
        state.0.is_some(),
        "page must remain quarantined after rollback"
    );
    assert_eq!(state.1, 0, "file_state must remain inactive after rollback");
}
