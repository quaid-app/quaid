#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would add noise"
)]

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use quaid::core::db;
use quaid::core::vault_sync::{register_session, SessionType};
use serde_json::Value;

fn open_test_db_file(dir: &tempfile::TempDir) -> (rusqlite::Connection, PathBuf) {
    let db_path = fs::canonicalize(dir.path()).unwrap().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    (conn, db_path)
}

fn run_quaid_status(db_path: &Path, home_dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .env("HOME", home_dir)
        .env("USERPROFILE", home_dir)
        .arg("--db")
        .arg(db_path)
        .args(args)
        .output()
        .expect("run quaid status")
}

#[test]
fn status_json_reports_runtime_host_and_recent_activity() {
    let dir = tempfile::TempDir::new().unwrap();
    let home_dir = dir.path().join("home");
    fs::create_dir_all(&home_dir).unwrap();
    let (conn, db_path) = open_test_db_file(&dir);

    // Register a daemon session; leave heartbeat_at at the DEFAULT (now) so the
    // 15-second SESSION_LIVENESS_SECS window in find_active_runtime_host keeps it live.
    let session_id = register_session(&conn, SessionType::Daemon).unwrap();
    conn.execute(
        "INSERT INTO extraction_queue (
             session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, status
         ) VALUES (
             'session-a', '/tmp/conversation.jsonl', 'manual',
             '2026-05-12T10:01:00Z', '2026-05-12T10:02:00Z', 'done'
         )",
        [],
    )
    .unwrap();
    drop(conn);

    let output = run_quaid_status(&db_path, &home_dir, &["status", "--json"]);

    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["daemon"]["installed"], false);
    assert_eq!(payload["runtime_host"]["session_type"], "daemon");
    assert_eq!(payload["runtime_host"]["session_id"], session_id);
    assert_eq!(
        payload["activity"]["last_extraction_at"],
        "2026-05-12T10:02:00Z"
    );
    // last_heartbeat_at reflects the daemon session's fresh heartbeat; exact value is
    // runtime-dependent so we only assert presence, not the specific timestamp.
    assert!(
        payload["activity"]["last_heartbeat_at"].is_string(),
        "expected last_heartbeat_at to be a string, got: {:?}",
        payload["activity"]["last_heartbeat_at"]
    );
}

#[test]
fn status_human_output_reports_no_runtime_host_for_empty_database() {
    let dir = tempfile::TempDir::new().unwrap();
    let home_dir = dir.path().join("home");
    fs::create_dir_all(&home_dir).unwrap();
    let (_conn, db_path) = open_test_db_file(&dir);

    let output = run_quaid_status(&db_path, &home_dir, &["status"]);

    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("daemon:"));
    assert!(stdout.contains("runtime_host:"));
    assert!(stdout.contains("none (no live daemon or serve_host)"));
    assert!(stdout.contains("transports:"));
}
