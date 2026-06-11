#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and inspect subprocess output"
)]

//! Subprocess tests asserting the global `--json` flag is honoured by
//! dispatch arms that historically ignored it (`put`, `tags`) and that
//! `quaid --json status` matches `quaid status --json` (the global and
//! local flags are OR-ed).

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use quaid::core::db;
use serde_json::Value;

struct Fixture {
    _dir: tempfile::TempDir,
    home_dir: PathBuf,
    db_path: PathBuf,
}

fn fixture_with_writable_vault() -> Fixture {
    let dir = tempfile::TempDir::new().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let home_dir = dir.path().join("home");
    fs::create_dir_all(&home_dir).unwrap();
    let db_path = fs::canonicalize(dir.path()).unwrap().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    let vault_root = dir.path().join("vault");
    fs::create_dir_all(&vault_root).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             writable = 1,
             is_write_target = 1,
             state = 'active',
             needs_full_sync = 0
         WHERE id = 1",
        [fs::canonicalize(&vault_root).unwrap().display().to_string()],
    )
    .unwrap();
    drop(conn);
    Fixture {
        _dir: dir,
        home_dir,
        db_path,
    }
}

fn quaid_command(fixture: &Fixture) -> Command {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .env("HOME", &fixture.home_dir)
        .env("USERPROFILE", &fixture.home_dir)
        .arg("--db")
        .arg(&fixture.db_path);
    command
}

fn run_quaid(fixture: &Fixture, args: &[&str]) -> Output {
    quaid_command(fixture)
        .args(args)
        .output()
        .expect("run quaid")
}

fn run_quaid_with_stdin(fixture: &Fixture, args: &[&str], stdin: &str) -> Output {
    let mut child = quaid_command(fixture)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn quaid");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for quaid")
}

fn stdout_json(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "stdout must be valid JSON, got error {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn seed_page(fixture: &Fixture, slug: &str) {
    let output = run_quaid_with_stdin(
        fixture,
        &["put", slug],
        "---\ntitle: Json Flag\ntype: concept\n---\nContent.\n",
    );
    assert!(
        output.status.success(),
        "seed put failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn global_json_flag_makes_put_emit_json() {
    let fixture = fixture_with_writable_vault();

    let output = run_quaid_with_stdin(
        &fixture,
        &["--json", "put", "notes/json-put"],
        "---\ntitle: Json Put\ntype: concept\n---\nBody.\n",
    );

    assert!(
        output.status.success(),
        "put failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = stdout_json(&output);
    let result = payload["result"].as_str().expect("result string");
    assert!(result.contains("notes/json-put"), "got: {result}");
    assert!(result.contains("version 1"), "got: {result}");
}

#[cfg(unix)]
#[test]
fn global_json_flag_makes_tags_emit_json_array() {
    let fixture = fixture_with_writable_vault();
    seed_page(&fixture, "notes/json-tags");

    let mutate = run_quaid(
        &fixture,
        &["--json", "tags", "notes/json-tags", "--add", "alpha"],
    );
    assert!(
        mutate.status.success(),
        "tags --add failed: {}",
        String::from_utf8_lossy(&mutate.stderr)
    );
    let mutated = stdout_json(&mutate);
    assert_eq!(mutated, serde_json::json!(["alpha"]));

    let list = run_quaid(&fixture, &["--json", "tags", "notes/json-tags"]);
    assert!(list.status.success());
    assert_eq!(stdout_json(&list), serde_json::json!(["alpha"]));
}

#[test]
fn global_json_flag_matches_local_status_json_flag() {
    let fixture = fixture_with_writable_vault();

    let global = run_quaid(&fixture, &["--json", "status"]);
    let local = run_quaid(&fixture, &["status", "--json"]);

    assert_eq!(global.status.code(), local.status.code());
    let global_payload = stdout_json(&global);
    let local_payload = stdout_json(&local);
    assert!(global_payload["daemon"].is_object());
    assert_eq!(
        global_payload["daemon"]["installed"],
        local_payload["daemon"]["installed"]
    );
    assert_eq!(global_payload["database"], local_payload["database"]);
}
