/// Targeted tests for Mom's quarantine revision blockers (Professor + Nibbler rejection)
///
/// Truth repair: quarantine restore is currently backed out of the live CLI surface.
/// Prior blocker 1: Failed export still unlocks discard
/// Restore-specific regression guard: disabled restore must not mutate disk or DB.
use gbrain::core::db;
use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;
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

#[test]
fn restore_surface_is_deferred_for_non_markdown_target() {
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

    let restore_result = std::process::Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("restore")
        .arg("work::notes/quarantined")
        .arg("notes/restored.txt")
        .output()
        .expect("run restore");

    assert!(
        !restore_result.status.success(),
        "restore to non-.md target should fail: {restore_result:?}"
    );
    let output_text = String::from_utf8_lossy(&restore_result.stderr);
    assert!(
        output_text.contains("quarantine restore is deferred in this batch"),
        "restore must surface the deferred-surface error: {output_text}"
    );

    assert!(
        !root.join("notes").join("restored.txt").exists(),
        "no .txt file should be written"
    );
}

#[test]
fn restore_surface_is_deferred_for_live_owned_collection() {
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

    let restore_result = std::process::Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("restore")
        .arg("work::notes/quarantined")
        .arg("notes/restored")
        .output()
        .expect("run restore");

    assert!(
        !restore_result.status.success(),
        "restore when serve owns collection should fail: {restore_result:?}"
    );
    let output_text = String::from_utf8_lossy(&restore_result.stderr);
    assert!(
        output_text.contains("quarantine restore is deferred in this batch"),
        "restore must surface the deferred-surface error: {output_text}"
    );

    assert!(
        !root.join("notes").join("restored.md").exists(),
        "no file should be written when serve owns collection"
    );
}

#[test]
fn restore_surface_is_deferred_before_target_conflict_mutation() {
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

    let restore_result = std::process::Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("restore")
        .arg("work::notes/quarantined")
        .arg("notes/conflict")
        .output()
        .expect("run restore");

    assert!(
        !restore_result.status.success(),
        "restore to existing target should fail: {restore_result:?}"
    );
    let output_text = String::from_utf8_lossy(&restore_result.stderr);
    assert!(
        output_text.contains("quarantine restore is deferred in this batch"),
        "restore must surface the deferred-surface error: {output_text}"
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

#[test]
fn restore_surface_is_deferred_for_read_only_collection() {
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

    let restore_result = std::process::Command::new(env!("CARGO_BIN_EXE_gbrain"))
        .arg("--db")
        .arg(&db_path)
        .arg("collection")
        .arg("quarantine")
        .arg("restore")
        .arg("readonly::notes/quarantined")
        .arg("notes/restored")
        .output()
        .expect("run restore");

    assert!(
        !restore_result.status.success(),
        "restore to read-only collection should fail: {restore_result:?}"
    );
    let output_text = String::from_utf8_lossy(&restore_result.stderr);
    assert!(
        output_text.contains("quarantine restore is deferred in this batch"),
        "restore must surface the deferred-surface error: {output_text}"
    );

    assert!(
        !root.join("notes").join("restored.md").exists(),
        "no file should be written when collection is read-only"
    );
}
