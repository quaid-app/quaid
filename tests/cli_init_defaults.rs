#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test fixtures panic with useful command diagnostics"
)]

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rusqlite::Connection;

fn run_quaid_with_home(home: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(args)
        .output()
        .expect("run quaid")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn default_collection_row(db_path: &Path) -> (String, String, i64, i64) {
    let conn = Connection::open(db_path).unwrap();
    conn.query_row(
        "SELECT root_path, state, writable, is_write_target
         FROM collections
         WHERE id = 1 AND name = 'default'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .unwrap()
}

fn default_vault(home: &Path) -> PathBuf {
    home.join(".quaid").join("vault")
}

#[test]
fn init_creates_default_writable_collection_root_under_home() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let db_path = dir.path().join("memory.db");

    let output = run_quaid_with_home(&home, &["init", db_path.to_str().unwrap()]);

    assert_success(&output);
    let expected_root = fs::canonicalize(default_vault(&home)).unwrap();
    let (root_path, state, writable, is_write_target) = default_collection_row(&db_path);
    assert_eq!(PathBuf::from(root_path), expected_root);
    assert_eq!(state, "active");
    assert_eq!(writable, 1);
    assert_eq!(is_write_target, 1);
}

#[test]
fn init_preserves_existing_configured_write_target_root() {
    let dir = tempfile::TempDir::new().unwrap();
    let first_home = dir.path().join("first-home");
    let second_home = dir.path().join("second-home");
    let custom_root = dir.path().join("custom-vault");
    fs::create_dir_all(&first_home).unwrap();
    fs::create_dir_all(&second_home).unwrap();
    fs::create_dir_all(&custom_root).unwrap();
    let custom_root = fs::canonicalize(custom_root).unwrap();
    let db_path = dir.path().join("memory.db");

    assert_success(&run_quaid_with_home(
        &first_home,
        &["init", db_path.to_str().unwrap()],
    ));
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active',
             writable = 1,
             is_write_target = 1
         WHERE id = 1",
        [custom_root.display().to_string()],
    )
    .unwrap();
    drop(conn);

    assert_success(&run_quaid_with_home(
        &second_home,
        &["init", db_path.to_str().unwrap()],
    ));

    let (root_path, state, writable, is_write_target) = default_collection_row(&db_path);
    assert_eq!(PathBuf::from(root_path), custom_root);
    assert_eq!(state, "active");
    assert_eq!(writable, 1);
    assert_eq!(is_write_target, 1);
    assert!(!default_vault(&second_home).exists());
}

#[test]
fn init_repairs_legacy_unconfigured_default_write_target() {
    let dir = tempfile::TempDir::new().unwrap();
    let first_home = dir.path().join("first-home");
    let repaired_home = dir.path().join("repaired-home");
    fs::create_dir_all(&first_home).unwrap();
    fs::create_dir_all(&repaired_home).unwrap();
    let db_path = dir.path().join("memory.db");

    assert_success(&run_quaid_with_home(
        &first_home,
        &["init", db_path.to_str().unwrap()],
    ));
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = '',
             state = 'detached',
             writable = 1,
             is_write_target = 1
         WHERE id = 1",
        [],
    )
    .unwrap();
    drop(conn);

    assert_success(&run_quaid_with_home(
        &repaired_home,
        &["init", db_path.to_str().unwrap()],
    ));

    let expected_root = fs::canonicalize(default_vault(&repaired_home)).unwrap();
    let (root_path, state, writable, is_write_target) = default_collection_row(&db_path);
    assert_eq!(PathBuf::from(root_path), expected_root);
    assert_eq!(state, "active");
    assert_eq!(writable, 1);
    assert_eq!(is_write_target, 1);
}

#[test]
fn init_surfaces_clear_error_when_default_root_cannot_be_created() {
    let dir = tempfile::TempDir::new().unwrap();
    let home_file = dir.path().join("not-a-directory");
    fs::write(&home_file, "not a directory").unwrap();
    let db_path = dir.path().join("memory.db");

    let output = run_quaid_with_home(&home_file, &["init", db_path.to_str().unwrap()]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to create default collection root"));
}

#[test]
fn memory_add_turn_writes_to_default_root_after_fresh_init() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let db_path = dir.path().join("memory.db");

    assert_success(&run_quaid_with_home(
        &home,
        &["init", db_path.to_str().unwrap()],
    ));
    let params = serde_json::json!({
        "session_id": "fresh-session",
        "role": "user",
        "content": "hello from a fresh init",
        "timestamp": "2026-05-03T09:14:22Z"
    })
    .to_string();

    let output = run_quaid_with_home(
        &home,
        &[
            "--db",
            db_path.to_str().unwrap(),
            "call",
            "memory_add_turn",
            &params,
        ],
    );

    assert_success(&output);
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["turn_id"], "fresh-session:1");
    assert_eq!(
        payload["conversation_path"],
        "conversations/2026-05-03/fresh-session.md"
    );
    let conversation_path = default_vault(&home)
        .join("conversations")
        .join("2026-05-03")
        .join("fresh-session.md");
    let conversation = fs::read_to_string(conversation_path).unwrap();
    assert!(conversation.contains("hello from a fresh init"));
}
