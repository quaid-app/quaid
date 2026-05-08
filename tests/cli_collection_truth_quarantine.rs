#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `quaid collection quarantine` truth-merge behavior,
//! including startup sweeps performed by the serve runtime on open.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use quaid::core::vault_sync;
use serde_json::Value;
use truth_fixtures::*;

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
