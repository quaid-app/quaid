#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests panic on setup failure"
)]

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;

fn wait_for_session(db_path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let conn = Connection::open(db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM serve_sessions", [], |row| row.get(0))
            .unwrap();
        if count > 0 {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("serve session was not registered before timeout");
}

fn wait_for_exit(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    panic!("quaid runtime did not exit after SIGTERM");
}

fn init_db(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let db_path = dir.path().join("memory.db");
    let init = Command::new(common::quaid_bin())
        .arg("init")
        .arg(&db_path)
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    db_path
}

fn spawn_quaid(db_path: &std::path::Path, args: &[&str]) -> Child {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .arg("--db")
        .arg(db_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn terminate_child(child: &Child) {
    rustix::process::kill_process(
        rustix::process::Pid::from_child(child),
        rustix::process::Signal::Term,
    )
    .unwrap();
}

fn assert_no_sessions(db_path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last_count = 0;
    while Instant::now() < deadline {
        let conn = Connection::open(db_path).unwrap();
        last_count = conn
            .query_row("SELECT COUNT(*) FROM serve_sessions", [], |row| row.get(0))
            .unwrap();
        if last_count == 0 {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(last_count, 0);
}

#[test]
fn serve_sigterm_unregisters_runtime_session() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let mut child = spawn_quaid(&db_path, &["serve"]);

    wait_for_session(&db_path);
    terminate_child(&child);
    wait_for_exit(&mut child);
    assert_no_sessions(&db_path);
}

#[test]
fn daemon_run_sigterm_unregisters_runtime_session() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let mut child = spawn_quaid(&db_path, &["daemon", "run"]);

    wait_for_session(&db_path);
    terminate_child(&child);
    wait_for_exit(&mut child);
    assert_no_sessions(&db_path);
}
