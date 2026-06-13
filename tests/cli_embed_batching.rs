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
    let error = run_with_batch(&conn, None, true, false, Some(0), false).unwrap_err();
    assert!(error.to_string().contains("batch-size"));
}

#[test]
fn run_with_batch_embeds_all_pages_and_rerun_is_idempotent() {
    let conn = open_test_db();
    for index in 0..12 {
        insert_page(&conn, &format!("notes/page-{index:02}"));
    }

    run_with_batch(&conn, None, true, false, Some(5), false).unwrap();
    let first_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(first_count, 12);

    run_with_batch(&conn, None, true, false, Some(5), false).unwrap();
    let second_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(second_count, first_count);
}

/// `--stale` keeps the hash-based skip; `--all` bypasses it. Tampering with
/// stored `chunk_text` while leaving `content_hash` intact is invisible to
/// the stale check, so only a forced `--all` run repairs the row.
#[test]
fn run_with_batch_all_forces_re_embed_while_stale_skips() {
    let conn = open_test_db();
    insert_page(&conn, "notes/force-target");

    run_with_batch(&conn, None, true, false, Some(5), false).unwrap();
    conn.execute("UPDATE page_embeddings SET chunk_text = 'tampered'", [])
        .unwrap();

    // --stale: hashes match, page is skipped, the tampered row survives.
    run_with_batch(&conn, None, false, true, Some(5), false).unwrap();
    let after_stale: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM page_embeddings WHERE chunk_text = 'tampered'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after_stale, 1, "--stale must skip hash-unchanged pages");

    // --all: force re-embed rewrites the chunk row from the page content.
    run_with_batch(&conn, None, true, false, Some(5), false).unwrap();
    let after_all: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM page_embeddings WHERE chunk_text = 'tampered'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after_all, 0,
        "--all must bypass the page_needs_refresh skip"
    );
}
