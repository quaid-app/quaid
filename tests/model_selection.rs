#![cfg(feature = "online-model")]

use quaid::core::{db, inference};

#[test]
fn init_small_open_large_returns_model_mismatch() {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let db_path = dir.path().join("memory.db");
    let path = db_path.to_str().expect("utf8 path");

    db::init(path, &inference::resolve_model("small")).expect("init small db");
    let err = db::open_with_model(path, &inference::resolve_model("large"))
        .expect_err("opening with a different model should fail");

    let message = err.to_string();
    assert!(message.contains("Error: Model mismatch"));
    assert!(message.contains("BAAI/bge-small-en-v1.5"));
    assert!(message.contains("BAAI/bge-large-en-v1.5"));
}

#[test]
fn init_large_open_large_succeeds() {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let db_path = dir.path().join("memory.db");
    let path = db_path.to_str().expect("utf8 path");

    db::init(path, &inference::resolve_model("large")).expect("init large db");
    let opened =
        db::open_with_model(path, &inference::resolve_model("large")).expect("reopen large db");

    assert_eq!(opened.effective_model.model_id, "BAAI/bge-large-en-v1.5");
    assert_eq!(opened.effective_model.embedding_dim, 1024);
}
