#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests panic on setup failure"
)]

//! Supervisor-death detection in the daemon foreground task.
//!
//! `quaid daemon run` must not keep running as a zombie when the
//! supervisor thread dies: watchers, heartbeats, and extraction have
//! silently stopped, yet launchd/systemd would still report the unit
//! healthy. The foreground task polls `JoinHandle::is_finished()` and
//! exits with code 1 so `KeepAlive` / `Restart=on-failure` restarts
//! the unit. The supervisor's death is injected via the
//! `QUAID_TEST_SUPERVISOR_EXIT_AFTER_MS` seam, which makes the
//! supervisor thread return abruptly (no heartbeat join, no session
//! cleanup — the shape of a crash).

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

#[test]
fn daemon_run_exits_nonzero_when_supervisor_thread_dies() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);

    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    let mut child = command
        .arg("--db")
        .arg(&db_path)
        .args(["daemon", "run"])
        .env("QUAID_TEST_SUPERVISOR_EXIT_AFTER_MS", "300")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Without the death watch the foreground awaits signals forever, so
    // bound the wait and fail loudly instead of hanging the suite.
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon did not exit after its supervisor thread died");
        }
        thread::sleep(Duration::from_millis(100));
    }

    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "supervisor death must exit 1 for Restart=on-failure/KeepAlive; stderr: {stderr}"
    );
    assert!(
        stderr.contains("supervisor_test_exit"),
        "test seam must have fired; stderr: {stderr}"
    );
    assert!(
        stderr.contains("daemon_supervisor_exited_unexpectedly"),
        "foreground must log the supervisor death; stderr: {stderr}"
    );
}
