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

/// Inserts a page and returns its rowid; the FTS index reflects its
/// `compiled_truth` via the `pages_ai` trigger.
fn insert_fts_page(conn: &Connection, slug: &str, compiled_truth: &str) -> i64 {
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
         VALUES (?1, ?2, ?3, 'concept', 'Title', '', ?4, '', '{}', 'notes', '', 1)",
        params![collection_id, slug, uuid::Uuid::now_v7().to_string(), compiled_truth],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn fts_match_count(conn: &Connection, term: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM page_fts WHERE page_fts MATCH ?1",
        [term],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn metadata_only_update_does_not_retokenize_fts() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let path_str = db_path.to_str().unwrap();
    let conn = open(path_str).unwrap();

    let page_id = insert_fts_page(&conn, "notes/meta-only", "alphaword");
    assert_eq!(fts_match_count(&conn, "alphaword"), 1);

    // Desync the FTS index from the content row: drop the FTS entry while the
    // pages.compiled_truth still says "alphaword". If a metadata-only UPDATE
    // re-fired the trigger it would re-insert the FTS row.
    conn.execute_batch(
        "INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
         SELECT 'delete', id, title, slug, compiled_truth, timeline FROM pages WHERE id =
         (SELECT id FROM pages WHERE slug = 'notes/meta-only')",
    )
    .unwrap();
    assert_eq!(fts_match_count(&conn, "alphaword"), 0);

    // Metadata-only writes: a version bump and a quarantine timestamp re-stamp
    // (NULL stays NULL). Neither touches title/slug/compiled_truth/timeline nor
    // flips the quarantine NULLness, so the guarded trigger must NOT fire and
    // the FTS row stays absent.
    conn.execute(
        "UPDATE pages SET version = version + 1 WHERE id = ?1",
        [page_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE pages SET updated_at = '2026-06-13T01:00:00Z' WHERE id = ?1",
        [page_id],
    )
    .unwrap();
    assert_eq!(
        fts_match_count(&conn, "alphaword"),
        0,
        "metadata-only UPDATE must not re-tokenize the page into FTS"
    );

    // A real content change on a *separate*, FTS-consistent page DOES fire the
    // trigger and re-indexes it (the first page above is intentionally
    // desynced, so re-running the delete branch on it would corrupt FTS).
    let other_id = insert_fts_page(&conn, "notes/content-change", "betaword");
    assert_eq!(fts_match_count(&conn, "betaword"), 1);
    conn.execute(
        "UPDATE pages SET compiled_truth = 'epsilonword' WHERE id = ?1",
        [other_id],
    )
    .unwrap();
    assert_eq!(fts_match_count(&conn, "epsilonword"), 1);
    assert_eq!(fts_match_count(&conn, "betaword"), 0);
}

#[test]
fn quarantine_flip_removes_and_restores_fts_rows() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let path_str = db_path.to_str().unwrap();
    let conn = open(path_str).unwrap();

    let page_id = insert_fts_page(&conn, "notes/quarantine-flip", "gammaword");
    assert_eq!(fts_match_count(&conn, "gammaword"), 1);

    // Quarantining (NULL -> NOT NULL) must fire the trigger and drop the row,
    // even though no FTS-visible column changed: the quarantine NULLness flip
    // is part of the WHEN guard precisely so FTS-side filtering stays correct.
    conn.execute(
        "UPDATE pages SET quarantined_at = '2026-06-13T00:00:00Z' WHERE id = ?1",
        [page_id],
    )
    .unwrap();
    assert_eq!(
        fts_match_count(&conn, "gammaword"),
        0,
        "quarantine flip must remove the page from FTS"
    );

    // Un-quarantining (NOT NULL -> NULL) restores the FTS row.
    conn.execute(
        "UPDATE pages SET quarantined_at = NULL WHERE id = ?1",
        [page_id],
    )
    .unwrap();
    assert_eq!(
        fts_match_count(&conn, "gammaword"),
        1,
        "clearing quarantine must restore the page to FTS"
    );
}

#[test]
fn open_replaces_unguarded_pages_update_trigger_with_when_guarded_one() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let path_str = db_path.to_str().unwrap();

    // Install a quarantine-aware but WHEN-guardless trigger (the prior
    // generation): correct FTS body, but re-fires on every UPDATE.
    let conn = open(path_str).unwrap();
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS pages_au;
         CREATE TRIGGER pages_au AFTER UPDATE ON pages BEGIN
             INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
             SELECT 'delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline
             WHERE old.quarantined_at IS NULL;
             INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
             SELECT new.id, new.title, new.slug, new.compiled_truth, new.timeline
             WHERE new.quarantined_at IS NULL;
         END;",
    )
    .unwrap();
    drop(conn);

    // Re-open: the open-time repair must rewrite the trigger to the WHEN-guarded
    // form because the stored SQL lacks the guard marker.
    let conn = open(path_str).unwrap();
    let trigger_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = 'pages_au'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        trigger_sql.contains("(old.quarantined_at IS NULL) <> (new.quarantined_at IS NULL)"),
        "open must repair the trigger to the WHEN-guarded form, got: {trigger_sql}"
    );

    // And the repaired trigger behaves: metadata-only UPDATE is a no-op for FTS.
    let page_id = insert_fts_page(&conn, "notes/repaired", "deltaword");
    conn.execute_batch(
        "INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
         SELECT 'delete', id, title, slug, compiled_truth, timeline FROM pages
         WHERE slug = 'notes/repaired'",
    )
    .unwrap();
    conn.execute(
        "UPDATE pages SET superseded_by = ?1 WHERE id = ?1",
        [page_id],
    )
    .unwrap();
    assert_eq!(fts_match_count(&conn, "deltaword"), 0);
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

#[test]
fn open_rebuilds_assertions_check_to_allow_extraction_provenance() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let path_str = db_path.to_str().unwrap();

    let conn = open(path_str).unwrap();
    conn.execute(
        "INSERT INTO pages (slug, uuid, type, title)
         VALUES ('notes/assert-test', ?1, 'concept', 'notes/assert-test')",
        [quaid::core::page_uuid::generate_uuid_v7()],
    )
    .unwrap();
    let page_id: i64 = conn
        .query_row(
            "SELECT id FROM pages WHERE slug = 'notes/assert-test'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(conn);

    // Simulate a database created before 'extraction' was an allowed
    // asserted_by value.
    let conn = Connection::open(path_str).unwrap();
    conn.execute_batch(
        "DROP TABLE assertions;
         CREATE TABLE assertions (
             id              INTEGER PRIMARY KEY AUTOINCREMENT,
             page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
             subject         TEXT    NOT NULL,
             predicate       TEXT    NOT NULL,
             object          TEXT    NOT NULL,
             valid_from      TEXT    DEFAULT NULL,
             valid_until     TEXT    DEFAULT NULL,
             supersedes_id   INTEGER DEFAULT NULL REFERENCES assertions(id),
             confidence      REAL    DEFAULT 1.0,
             asserted_by     TEXT    NOT NULL DEFAULT 'agent',
             source_ref      TEXT    NOT NULL DEFAULT '',
             evidence_text   TEXT    NOT NULL DEFAULT '',
             created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
             CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from),
             CHECK (asserted_by IN ('agent', 'manual', 'import', 'enrichment'))
         );
         CREATE INDEX IF NOT EXISTS idx_assertions_subj ON assertions(subject);
         CREATE INDEX IF NOT EXISTS idx_assertions_pred ON assertions(predicate);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assertions (page_id, subject, predicate, object, asserted_by)
         VALUES (?1, 'Alice', 'works_at', 'Acme Corp', 'manual')",
        [page_id],
    )
    .unwrap();
    drop(conn);

    let conn = open(path_str).unwrap();
    // Legacy rows survive the rebuild...
    let (subject, asserted_by): (String, String) = conn
        .query_row(
            "SELECT subject, asserted_by FROM assertions WHERE page_id = ?1",
            [page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(subject, "Alice");
    assert_eq!(asserted_by, "manual");

    // ...and 'extraction' provenance is now accepted.
    conn.execute(
        "INSERT INTO assertions (page_id, subject, predicate, object, asserted_by)
         VALUES (?1, 'timezone', 'fact', 'Matt is based in UTC+2', 'extraction')",
        [page_id],
    )
    .unwrap();
}
