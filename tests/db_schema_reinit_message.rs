#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would add noise"
)]

//! Accuracy tests for the schema-version-mismatch refusal message.
//!
//! The previous message claimed every mismatched database was "created
//! before the Quaid rename" and pointed users at a `quaid init` invocation
//! that re-fails on the same preflight — a remediation dead-loop. These
//! tests pin the rewritten message: older-than-current names the v9
//! release range, newer-than-current says to upgrade the binary, and both
//! downgrade paths instruct the user to back up / move the old file first.

use quaid::core::db::{open_with_model, QuaidConfig};
use quaid::core::inference::default_model;
use rusqlite::Connection;
use std::path::Path;

/// Seed a database carrying only config tables with the given schema
/// version — the same fixture shape `quaid` v0.20.x-v0.21.x (v9) or a
/// future release would leave on disk.
fn seed_versioned_db(path: &Path, schema_version: i64) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE quaid_config (
             key   TEXT PRIMARY KEY NOT NULL,
             value TEXT NOT NULL
         ) STRICT;
         CREATE TABLE config (
             key   TEXT PRIMARY KEY NOT NULL,
             value TEXT NOT NULL
         ) STRICT;",
    )
    .unwrap();
    let model = default_model();
    quaid::core::db::write_quaid_config(
        &conn,
        &QuaidConfig {
            model_id: model.model_id.clone(),
            model_alias: model.alias.clone(),
            embedding_dim: model.embedding_dim,
            schema_version,
        },
    )
    .unwrap();
    conn.execute(
        "INSERT INTO config (key, value) VALUES ('version', ?1)",
        [schema_version.to_string()],
    )
    .unwrap();
}

#[test]
fn v9_database_message_names_release_range_and_backup_step() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("legacy-v9.db");
    seed_versioned_db(&db_path, 9);

    let error = open_with_model(db_path.to_str().unwrap(), &default_model())
        .expect_err("v9 database must be refused");
    let message = error.to_string();

    assert!(
        message.contains("Found version 9, expected 10"),
        "got: {message}"
    );
    // The rename claim was factually wrong for v9 databases — it must be gone.
    assert!(!message.contains("rename"), "got: {message}");
    assert!(!message.contains("pre-rename"), "got: {message}");
    // Older-than-current names the likely release range for v9.
    assert!(message.contains("v0.20.x-v0.21.x"), "got: {message}");
    assert!(message.contains("older quaid release"), "got: {message}");
    // Remediation must start with backing up / moving the old file, since
    // `quaid init` re-fails on the same preflight at the old path.
    assert!(message.contains("BACK UP"), "got: {message}");
    assert!(
        message.contains(&format!(
            "mv {} {}.bak",
            db_path.display(),
            db_path.display()
        )),
        "got: {message}"
    );
}

#[test]
fn newer_schema_database_message_says_upgrade_binary() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("future.db");
    seed_versioned_db(&db_path, 11);

    let error = open_with_model(db_path.to_str().unwrap(), &default_model())
        .expect_err("future-schema database must be refused");
    let message = error.to_string();

    assert!(
        message.contains("Found version 11, expected 10"),
        "got: {message}"
    );
    assert!(message.contains("NEWER quaid release"), "got: {message}");
    assert!(
        message.contains("Upgrade the quaid binary"),
        "got: {message}"
    );
    // The newer-DB path must NOT suggest re-initialising over good data.
    assert!(
        message.contains("Do NOT run `quaid init` against this database"),
        "got: {message}"
    );
    assert!(!message.contains("rename"), "got: {message}");
}

#[test]
fn versionless_legacy_database_message_describes_legacy_era() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("legacy-versionless.db");
    // A database holding page content but no readable schema version in
    // either config table: the true legacy (pre-versioned-config) case.
    // Built from the current DDL, then stripped of every version marker.
    let conn = quaid::core::db::open(db_path.to_str().unwrap()).unwrap();
    conn.execute(
        "INSERT INTO pages (slug, uuid, type, title)
         VALUES ('notes/legacy', ?1, 'concept', 'Legacy')",
        [uuid::Uuid::now_v7().to_string()],
    )
    .unwrap();
    conn.execute("DELETE FROM quaid_config", []).unwrap();
    conn.execute("DELETE FROM config WHERE key = 'version'", [])
        .unwrap();
    drop(conn);

    let error = open_with_model(db_path.to_str().unwrap(), &default_model())
        .expect_err("versionless database with content must be refused");
    let message = error.to_string();

    assert!(
        message.contains("Found version 0, expected 10"),
        "got: {message}"
    );
    assert!(
        message.contains("no readable schema version"),
        "got: {message}"
    );
    assert!(message.contains("legacy release"), "got: {message}");
    assert!(message.contains("BACK UP"), "got: {message}");
}
