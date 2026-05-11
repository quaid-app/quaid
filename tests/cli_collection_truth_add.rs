#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]
#![cfg(unix)]

//! Integration tests for `quaid collection add` truth-merge behavior,
//! including UUID write-back and live-owner refusal scenarios.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use truth_fixtures::*;

#[cfg(unix)]
#[test]
fn collection_add_write_quaid_id_updates_file_and_rotates_raw_imports() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = init_db(&dir);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        root.join("note.md"),
        "---\ntitle: Note\ntype: concept\n---\nhello from add\n",
    )
    .expect("write note");

    let output = run_quaid(
        &db_path,
        &[
            "--json",
            "collection",
            "add",
            "work",
            root.to_str().expect("root path"),
            "--write-quaid-id",
        ],
    );
    assert!(
        output.status.success(),
        "collection add should succeed: {output:?}"
    );
    let json = parse_stdout_json(&output);
    assert_eq!(json["uuid_write_back"]["migrated"].as_u64(), Some(1));
    assert_eq!(
        json["uuid_write_back"]["skipped_readonly"].as_u64(),
        Some(0)
    );
    assert_eq!(
        json["uuid_write_back"]["already_had_uuid"].as_u64(),
        Some(0)
    );

    let rendered = std::fs::read_to_string(root.join("note.md")).expect("read migrated note");
    assert!(rendered.contains("quaid_id: "));
    assert!(!rendered.contains("memory_id: "));

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
    assert_eq!(total_rows, 2);
}

#[cfg(unix)]
#[test]
fn collection_add_write_quaid_id_refuses_same_root_live_owner_before_alias_attach() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = init_db(&dir);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        root.join("note.md"),
        "---\ntitle: Note\ntype: concept\n---\nhello from add\n",
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
    assert!(add.status.success(), "initial add should succeed: {add:?}");

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
         VALUES ('serve-live', 2468, 'alias-host', datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-live')",
        [collection_id],
    )
    .unwrap();
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "collection",
            "add",
            "alias",
            root.to_str().expect("root path"),
            "--write-quaid-id",
        ],
    );
    assert!(
        !output.status.success(),
        "same-root live owner should block alias write-back add: {output:?}"
    );
    let text = combined_output(&output);
    assert!(text.contains("RuntimeOwnsCollectionError"));
    assert!(text.contains("owner_pid=2468"));
    assert!(text.contains("owner_host=alias-host"));
    assert!(
        text.contains("stop the daemon first") || text.contains("stop the running serve first"),
        "expected runtime stop-hint in error text: {text}"
    );

    let conn = open_test_db(&db_path);
    let alias_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM collections WHERE name = 'alias'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(alias_count, 0);
}
