#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `quaid::core::db::open` migrating an existing
//! database forward.
//!
//! Locks in DDL repair (replacing a buggy `pages_au` trigger) and data
//! backfill (`raw_imports.content_hash` for legacy rows).

use quaid::core::db::open;
use rusqlite::{params, Connection};

#[test]
fn open_replaces_buggy_pages_update_trigger_for_quarantined_rows() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let path_str = db_path.to_str().unwrap();

    let conn = open(path_str).unwrap();
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS pages_au;
         CREATE TRIGGER pages_au AFTER UPDATE ON pages BEGIN
             INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
             VALUES ('delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline);
             INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
             SELECT new.id, new.title, new.slug, new.compiled_truth, new.timeline
             WHERE new.quarantined_at IS NULL;
         END;",
    )
    .unwrap();
    drop(conn);

    let conn = open(path_str).unwrap();
    let collection_id: i64 = conn
        .query_row(
            "SELECT id FROM collections ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, 'notes/quarantined', ?2, 'concept', 'Quarantined', '', 'before restore', '', '{}', 'notes', '', 1)",
        params![collection_id, uuid::Uuid::now_v7().to_string()],
    )
    .unwrap();
    let page_id = conn.last_insert_rowid();

    conn.execute(
        "UPDATE pages
         SET quarantined_at = '2026-04-25T00:00:00Z'
         WHERE id = ?1",
        [page_id],
    )
    .unwrap();

    conn.execute(
        "UPDATE pages
         SET slug = 'notes/restored',
             title = 'Restored',
             compiled_truth = 'after restore',
             quarantined_at = NULL
         WHERE id = ?1",
        [page_id],
    )
    .unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM page_fts
             WHERE page_fts MATCH 'after'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn open_backfills_raw_import_content_hash_for_existing_rows() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let path_str = db_path.to_str().unwrap();

    let conn = open(path_str).unwrap();
    conn.execute(
        "INSERT INTO pages (slug, uuid, type, title)
         VALUES ('notes/hash-test', ?1, 'concept', 'notes/hash-test')",
        [quaid::core::page_uuid::generate_uuid_v7()],
    )
    .unwrap();
    let page_id: i64 = conn
        .query_row(
            "SELECT id FROM pages WHERE slug = 'notes/hash-test'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, content_hash, raw_bytes, file_path)
         VALUES (?1, 'import-1', 1, ?2, ?3, 'notes/hash-test.md')",
        params![
            page_id,
            quaid::core::raw_imports::content_hash_hex(b"hello"),
            b"hello"
        ],
    )
    .unwrap();
    drop(conn);

    let conn = Connection::open(path_str).unwrap();
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_raw_imports_content_hash;
         ALTER TABLE raw_imports RENAME TO raw_imports_old;
         CREATE TABLE raw_imports (
             id         INTEGER PRIMARY KEY AUTOINCREMENT,
             page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
             import_id  TEXT    NOT NULL,
             is_active  INTEGER NOT NULL DEFAULT 1,
             raw_bytes  BLOB    NOT NULL,
             file_path  TEXT    NOT NULL,
             created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
             UNIQUE(page_id, import_id)
         );
         INSERT INTO raw_imports (id, page_id, import_id, is_active, raw_bytes, file_path, created_at)
         SELECT id, page_id, import_id, is_active, raw_bytes, file_path, created_at
         FROM raw_imports_old;
         DROP TABLE raw_imports_old;
         CREATE INDEX idx_raw_imports_page ON raw_imports(page_id);
         CREATE INDEX idx_raw_imports_active ON raw_imports(page_id, is_active)
             WHERE is_active = 1;",
    )
    .unwrap();
    drop(conn);

    let conn = open(path_str).unwrap();
    let content_hash: String = conn
        .query_row(
            "SELECT content_hash FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        content_hash,
        quaid::core::raw_imports::content_hash_hex(b"hello")
    );
}
