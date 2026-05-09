#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `quaid collection list` truth-merge behavior.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use truth_fixtures::*;

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
fn collection_list_plain_text_skips_placeholder_rows_and_reports_queue_depth() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-list-plain.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    insert_page(&conn, collection_id, "notes/a");
    let work_page_id = page_id(&conn, collection_id, "notes/a");
    conn.execute(
        "INSERT INTO embedding_jobs (page_id) VALUES (?1)",
        [work_page_id],
    )
    .expect("insert embedding job");
    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target)
         VALUES ('placeholder', '', 'detached', 0, 0)",
        [],
    )
    .expect("insert placeholder collection");
    drop(conn);

    let output = run_quaid(&db_path, &["collection", "list"]);

    assert!(
        output.status.success(),
        "collection list should succeed: {output:?}"
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("name | state | writable | write_target | root_path | page_count | last_sync_at | queue_depth"));
    assert!(text.contains("work | active | writable | false |"));
    assert!(text.contains(" | 1 | - | 1"));
    assert!(
        !text.contains("placeholder"),
        "placeholder rows with empty roots must stay hidden: {text}"
    );
}
