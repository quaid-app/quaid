#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    dead_code,
    unreachable_pub,
    reason = "test fixtures legitimately panic on setup failure; pub helpers are shared across `tests/cli_collection_*.rs` files but unreachable from non-test crates; `dead_code` because individual test files only use a subset of the helpers"
)]

//! Shared test fixtures for `tests/cli_collection_*.rs` integration tests.
//!
//! Mirrors the inline helpers that previously lived inside
//! `src/commands/collection.rs::tests`. Test bodies depend on these helpers
//! by name and signature, so they are kept verbatim here aside from
//! visibility (`pub`) and the public-API import paths (`quaid::core::db`,
//! etc.).

use std::fs;
use std::path::Path;

use quaid::commands::collection::{CollectionAction, CollectionAddArgs};
use quaid::core::{db, markdown};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn open_test_db() -> Connection {
    db::open(":memory:").unwrap()
}

pub fn open_test_db_file_any() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    (dir, conn)
}

#[cfg(unix)]
pub fn open_test_db_file() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    (dir, conn)
}

pub fn insert_collection(conn: &Connection, name: &str, root_path: &Path) -> i64 {
    // Production paths reach the `collections.root_path` column via add() which
    // canonicalizes via fs::canonicalize. Tests insert directly here, so canonicalize
    // here too; otherwise macOS's /var ↔ /private/var symlink causes mismatches with
    // production-side lookups (live-owner checks, etc.).
    let root_path = fs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target)
         VALUES (?1, ?2, 'active', 1, 0)",
        rusqlite::params![name, root_path.display().to_string()],
    )
    .unwrap();
    conn.last_insert_rowid()
}

pub fn insert_page_with_raw_import(
    conn: &Connection,
    collection_id: i64,
    slug: &str,
    uuid: &str,
    raw_bytes: &[u8],
    relative_path: &str,
) -> i64 {
    let frontmatter_json = std::str::from_utf8(raw_bytes)
        .ok()
        .map(|s| {
            let (fm, _) = markdown::parse_frontmatter(s);
            serde_json::to_string(&fm).unwrap_or_else(|_| "{}".to_owned())
        })
        .unwrap_or_else(|| "{}".to_owned());
    let compiled_truth = std::str::from_utf8(raw_bytes)
        .ok()
        .map(|s| markdown::parse_frontmatter(s).1)
        .unwrap_or_default();
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, 'concept', ?2, '', ?4, '', ?5, '', '', 1)",
        rusqlite::params![collection_id, slug, uuid, compiled_truth, frontmatter_json],
    )
    .unwrap();
    let page_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
         VALUES (?1, ?2, 1, ?3, ?4)",
        rusqlite::params![
            page_id,
            Uuid::now_v7().to_string(),
            raw_bytes,
            relative_path
        ],
    )
    .unwrap();
    let hash = Sha256::digest(raw_bytes);
    let sha256 = hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    conn.execute(
        "INSERT INTO file_state (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256)
         VALUES (?1, ?2, ?3, 1, 1, ?4, 1, ?5)",
        rusqlite::params![collection_id, relative_path, page_id, raw_bytes.len() as i64, sha256],
    )
    .unwrap();
    page_id
}

pub fn page_id(conn: &Connection, collection_id: i64, slug: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
        rusqlite::params![collection_id, slug],
        |row| row.get(0),
    )
    .unwrap()
}

pub fn quarantine_page(conn: &Connection, page_id: i64, quarantined_at: &str) {
    conn.execute(
        "UPDATE pages SET quarantined_at = ?2 WHERE id = ?1",
        rusqlite::params![page_id, quarantined_at],
    )
    .unwrap();
}

pub fn insert_knowledge_gap(conn: &Connection, page_id: i64, query_hash: &str) {
    conn.execute(
        "INSERT INTO knowledge_gaps (page_id, query_hash, context) VALUES (?1, ?2, 'context')",
        rusqlite::params![page_id, query_hash],
    )
    .unwrap();
}

pub fn insert_embedding_job(conn: &Connection, page_id: i64, job_state: &str, attempt_count: i64) {
    conn.execute(
        "INSERT INTO embedding_jobs (page_id, job_state, attempt_count, last_error, started_at)
         VALUES (?1, ?2, ?3, CASE WHEN ?2 = 'failed' THEN 'boom' ELSE NULL END, CASE WHEN ?2 = 'running' THEN '2026-04-28T00:00:00Z' ELSE NULL END)",
        rusqlite::params![page_id, job_state, attempt_count],
    )
    .unwrap();
}

#[cfg(unix)]
pub fn collection_page_count(conn: &Connection, name: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*)
           FROM pages p
           JOIN collections c ON c.id = p.collection_id
          WHERE c.name = ?1 AND p.quarantined_at IS NULL",
        [name],
        |row| row.get(0),
    )
    .unwrap()
}

#[cfg(unix)]
pub fn fetch_ignore_mirror(conn: &Connection, name: &str) -> Option<String> {
    conn.query_row(
        "SELECT ignore_patterns FROM collections WHERE name = ?1",
        [name],
        |row| row.get(0),
    )
    .unwrap()
}

#[cfg(unix)]
pub fn attach_collection(conn: &Connection, name: &str, root_path: &Path) {
    quaid::commands::collection::run(
        conn,
        CollectionAction::Add(CollectionAddArgs {
            name: name.to_owned(),
            path: root_path.to_path_buf(),
            read_only: false,
            writable: false,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    )
    .unwrap();
}
