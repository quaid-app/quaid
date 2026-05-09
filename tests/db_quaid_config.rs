#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests covering `QuaidConfig` reads/writes and the
//! mismatch behavior visible through the public open API.

use quaid::core::db::{open, open_with_model, read_quaid_config, QuaidConfig};
use quaid::core::inference::default_model;
use quaid::core::types::DbError;
use rusqlite::params;

#[test]
fn open_seeds_default_embedding_model() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let opened = open_with_model(db_path.to_str().unwrap(), &default_model()).unwrap();

    let (name, dims, active): (String, i64, i64) = opened
        .conn
        .query_row(
            "SELECT name, dimensions, active FROM embedding_models WHERE active = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();

    assert_eq!(name, "BAAI/bge-small-en-v1.5");
    assert_eq!(dims, 384);
    assert_eq!(active, 1);
}

#[test]
fn quaid_config_roundtrip_preserves_values() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let opened = open_with_model(db_path.to_str().unwrap(), &default_model()).unwrap();

    let config = read_quaid_config(&opened.conn).unwrap().unwrap();
    assert_eq!(
        config,
        QuaidConfig {
            model_id: "BAAI/bge-small-en-v1.5".to_owned(),
            model_alias: "small".to_owned(),
            embedding_dim: 384,
            schema_version: 9,
        }
    );
}

#[test]
fn empty_quaid_config_reads_as_missing() {
    let conn = open(":memory:").unwrap();
    conn.execute("DELETE FROM quaid_config", []).unwrap();

    let config = read_quaid_config(&conn).unwrap();
    assert!(config.is_none());
}

#[test]
fn incomplete_quaid_config_returns_schema_error() {
    let conn = open(":memory:").unwrap();
    conn.execute("DELETE FROM quaid_config", []).unwrap();
    conn.execute(
        "INSERT INTO quaid_config (key, value) VALUES ('model_id', 'BAAI/bge-small-en-v1.5')",
        [],
    )
    .unwrap();

    let err = read_quaid_config(&conn).unwrap_err();
    assert!(matches!(err, DbError::Schema { .. }));
}

#[test]
fn missing_quaid_config_requires_reinit_once_pages_exist() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let opened = open_with_model(db_path.to_str().unwrap(), &default_model()).unwrap();
    let collection_id: i64 = opened
        .conn
        .query_row(
            "SELECT id FROM collections WHERE name = 'default'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    opened
        .conn
        .execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES (?1, 'notes/live', ?2, 'concept', 'Live', '', 'truth', '', '{}', 'notes', '', 1)",
            params![collection_id, uuid::Uuid::now_v7().to_string()],
        )
        .unwrap();
    opened.conn.execute("DELETE FROM quaid_config", []).unwrap();
    drop(opened);

    let reopened = open_with_model(db_path.to_str().unwrap(), &default_model());
    assert!(matches!(reopened, Err(DbError::Schema { .. })));
}
