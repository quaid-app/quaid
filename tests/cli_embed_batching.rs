#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests panic on setup failure"
)]

use quaid::commands::embed::run_with_batch;
use quaid::core::db;
use rusqlite::Connection;
use uuid::Uuid;

fn open_test_db() -> Connection {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    std::mem::forget(dir);
    conn
}

fn insert_page(conn: &Connection, slug: &str) {
    conn.execute(
        "INSERT INTO pages
             (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, 'concept', ?1, '', ?3, '', '{}', '', '', 1)",
        rusqlite::params![
            slug,
            Uuid::now_v7().to_string(),
            format!("## State\n{slug} has enough content to embed.")
        ],
    )
    .unwrap();
}

#[test]
fn run_with_batch_rejects_zero_batch_size_before_embedding() {
    let conn = open_test_db();
    let error = run_with_batch(&conn, None, true, false, Some(0)).unwrap_err();
    assert!(error.to_string().contains("batch-size"));
}

#[test]
fn run_with_batch_embeds_all_pages_and_rerun_is_idempotent() {
    let conn = open_test_db();
    for index in 0..12 {
        insert_page(&conn, &format!("notes/page-{index:02}"));
    }

    run_with_batch(&conn, None, true, false, Some(5)).unwrap();
    let first_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(first_count, 12);

    run_with_batch(&conn, None, true, false, Some(5)).unwrap();
    let second_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(second_count, first_count);
}
