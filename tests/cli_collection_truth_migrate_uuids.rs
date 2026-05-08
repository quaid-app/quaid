#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]
#![cfg(unix)]

//! Integration tests for `quaid collection migrate-uuids` truth-merge behavior.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use truth_fixtures::*;

#[cfg(unix)]
#[test]
fn collection_migrate_uuids_dry_run_reports_without_mutation() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = init_db(&dir);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let original = "---\ntitle: Note\ntype: concept\n---\nhello from dry run\n";
    std::fs::write(root.join("note.md"), original).expect("write note");

    let add = run_quaid(
        &db_path,
        &[
            "collection",
            "add",
            "work",
            root.to_str().expect("root path"),
        ],
    );
    assert!(
        add.status.success(),
        "collection add should succeed: {add:?}"
    );

    let dry_run = run_quaid(
        &db_path,
        &["--json", "collection", "migrate-uuids", "work", "--dry-run"],
    );
    assert!(
        dry_run.status.success(),
        "dry-run should succeed: {dry_run:?}"
    );
    let json = parse_stdout_json(&dry_run);
    assert_eq!(json["migrated"].as_u64(), Some(1));
    assert_eq!(json["skipped_readonly"].as_u64(), Some(0));
    assert_eq!(json["already_had_uuid"].as_u64(), Some(0));
    assert_eq!(
        std::fs::read_to_string(root.join("note.md")).expect("read note"),
        original
    );

    let conn = open_test_db(&db_path);
    let collection_id: i64 = conn
        .query_row(
            "SELECT id FROM collections WHERE name = 'work'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let page_id = page_id(&conn, collection_id, "note");
    let (active_rows, total_rows) = raw_import_counts(&conn, page_id);
    assert_eq!(active_rows, 1);
    assert_eq!(total_rows, 1);
}

#[cfg(unix)]
#[test]
fn collection_migrate_uuids_refuses_live_owner_with_pid_and_host() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = init_db(&dir);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        root.join("note.md"),
        "---\ntitle: Note\ntype: concept\n---\nhello from migrate\n",
    )
    .expect("write note");

    let add = run_quaid(
        &db_path,
        &[
            "collection",
            "add",
            "work",
            root.to_str().expect("root path"),
        ],
    );
    assert!(
        add.status.success(),
        "collection add should succeed: {add:?}"
    );

    let conn = open_test_db(&db_path);
    let collection_id: i64 = conn
        .query_row(
            "SELECT id FROM collections WHERE name = 'work'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('serve-live', 9876, 'truth-host', datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-live')",
        [collection_id],
    )
    .unwrap();
    drop(conn);

    let output = run_quaid(&db_path, &["collection", "migrate-uuids", "work"]);
    assert!(
        !output.status.success(),
        "migrate-uuids should refuse: {output:?}"
    );
    let text = combined_output(&output);
    assert!(text.contains("ServeOwnsCollectionError"));
    assert!(text.contains("owner_pid=9876"));
    assert!(text.contains("owner_host=truth-host"));
    assert!(text.contains("stop serve first"));
}
