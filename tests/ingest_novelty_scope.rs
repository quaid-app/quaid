//! Novelty-scope regression (#75): the ingest novelty check used to compare
//! the incoming content against a same-slug page from ANY collection, so an
//! ingest into the default collection was silently dropped whenever another
//! collection already held identical content under the same slug.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

use quaid::commands::ingest;
use quaid::core::db;
use rusqlite::Connection;

fn open_test_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    (dir, conn)
}

#[test]
fn ingest_is_not_skipped_by_same_slug_page_in_another_collection() {
    let (dir, conn) = open_test_db();

    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target)
         VALUES ('work', '/work', 'active', 1, 0)",
        [],
    )
    .unwrap();
    let work_id = conn.last_insert_rowid();

    let body = "Alice works at Acme and invests in climate software.";
    conn.execute(
        "INSERT INTO pages (collection_id, namespace, slug, uuid, type, title, summary, \
                            compiled_truth, timeline, frontmatter, wing, room, version) \
         VALUES (?1, '', 'notes/shared', '01969f11-9448-7d79-8d3f-c68f54761234', 'concept', \
                 'Shared', '', ?2, '', '{}', 'notes', '', 1)",
        rusqlite::params![work_id, body],
    )
    .unwrap();

    let file_path = dir.path().join("shared.md");
    std::fs::write(
        &file_path,
        format!("---\nslug: notes/shared\ntitle: Shared\ntype: concept\n---\n{body}\n"),
    )
    .unwrap();

    ingest::run(&conn, file_path.to_str().unwrap(), false).unwrap();

    let default_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = 1 AND slug = 'notes/shared'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        default_count, 1,
        "identical content in another collection must not suppress the ingest (#75)"
    );

    let work_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = ?1 AND slug = 'notes/shared'",
            [work_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(work_count, 1, "the other collection's page is untouched");
}

#[test]
fn ingest_still_skips_near_duplicates_within_its_own_collection() {
    let (dir, conn) = open_test_db();

    let body = "Alice works at Acme and invests in climate software.";
    let first = dir.path().join("first.md");
    std::fs::write(
        &first,
        format!("---\nslug: notes/shared\ntitle: Shared\ntype: concept\n---\n{body}\n"),
    )
    .unwrap();
    ingest::run(&conn, first.to_str().unwrap(), false).unwrap();

    // Different bytes (extra frontmatter key), near-identical content.
    let second = dir.path().join("second.md");
    std::fs::write(
        &second,
        format!(
            "---\nslug: notes/shared\ntitle: Shared\ntype: concept\nwing: notes\n---\n{body}\n"
        ),
    )
    .unwrap();
    ingest::run(&conn, second.to_str().unwrap(), false).unwrap();

    let version: i64 = conn
        .query_row(
            "SELECT version FROM pages WHERE collection_id = 1 AND slug = 'notes/shared'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        version, 1,
        "same-collection duplicate must still be skipped by the novelty check"
    );
}
