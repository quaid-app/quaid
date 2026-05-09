#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for the basic open/compact lifecycle of `quaid::core::db`.
//!
//! Covers the public open API and post-open invariants: required tables,
//! user_version, WAL mode, foreign keys, parent-dir validation, idempotent
//! reopen, and the public `compact` helper.

use quaid::core::db::{compact, open};
use quaid::core::types::DbError;

#[test]
fn open_creates_all_expected_tables() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let conn = open(db_path.to_str().unwrap()).unwrap();

    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();

    let expected = [
        "assertions",
        "quaid_config",
        "collections",
        "collection_owners",
        "config",
        "contradictions",
        "correction_sessions",
        "embedding_jobs",
        "embedding_models",
        "extraction_queue",
        "file_state",
        "import_manifest",
        "knowledge_gaps",
        "links",
        "namespaces",
        "page_embeddings",
        "page_fts",
        "pages",
        "raw_data",
        "raw_imports",
        "serve_sessions",
        "tags",
        "timeline_entries",
    ];

    for name in &expected {
        assert!(
            tables.contains(&(*name).to_string()),
            "missing table: {name}"
        );
    }
}

#[test]
fn open_sets_user_version_to_9() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let conn = open(db_path.to_str().unwrap()).unwrap();

    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);
}

#[test]
fn open_enables_wal_and_foreign_keys() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let conn = open(db_path.to_str().unwrap()).unwrap();

    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal.to_lowercase(), "wal");

    let fk: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fk, 1);
}

#[test]
fn open_rejects_nonexistent_parent_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing = dir.path().join("missing-parent").join("memory.db");
    let result = open(missing.to_str().unwrap());
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), DbError::PathNotFound { .. }));
}

#[test]
fn open_is_idempotent() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let path_str = db_path.to_str().unwrap();

    let conn1 = open(path_str).unwrap();
    drop(conn1);

    let conn2 = open(path_str).unwrap();
    let version: i64 = conn2
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);
}

#[test]
fn compact_checkpoints_wal() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let conn = open(db_path.to_str().unwrap()).unwrap();
    assert!(compact(&conn).is_ok());
}
