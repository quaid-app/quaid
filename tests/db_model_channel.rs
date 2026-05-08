//! Integration tests for the online-model channel of `quaid::core::db`.
//!
//! These tests only build when the `online-model` feature is enabled; in
//! the default airgapped build the file compiles to no tests.

#![cfg(feature = "online-model")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

use quaid::core::db::{init, open_with_model, read_quaid_config};
use quaid::core::inference::resolve_model;
use quaid::core::types::DbError;

#[test]
fn init_with_small_then_open_with_large_errors() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");

    init(db_path.to_str().unwrap(), &resolve_model("small")).unwrap();
    let err = open_with_model(db_path.to_str().unwrap(), &resolve_model("large")).unwrap_err();

    assert!(matches!(err, DbError::ModelMismatch { .. }));
}

#[test]
fn init_with_large_then_open_with_large_succeeds() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");

    let init_opened = open_with_model(db_path.to_str().unwrap(), &resolve_model("large")).unwrap();
    drop(init_opened);

    let reopened = open_with_model(db_path.to_str().unwrap(), &resolve_model("large")).unwrap();
    let stored = read_quaid_config(&reopened.conn).unwrap().unwrap();
    assert_eq!(stored.model_alias, "large");
    assert_eq!(stored.embedding_dim, 1024);
}
