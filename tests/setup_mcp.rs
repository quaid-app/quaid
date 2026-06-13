#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test fixtures panic with useful command diagnostics"
)]

//! Subprocess coverage for `quaid setup --register-mcp`.
//!
//! Each test runs the real binary with an isolated `$HOME` so the writes land
//! in a temp dir, never the developer's real `~/.claude` / `~/.cursor`.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn run_setup(home: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        // Clear any inherited DB override so the default `~/.quaid/memory.db`
        // path resolves against our temp HOME.
        .env_remove("QUAID_DB")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(args)
        .output()
        .expect("run quaid setup")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn claude_config(home: &Path) -> PathBuf {
    home.join(".claude").join("mcp.json")
}

fn cursor_config(home: &Path) -> PathBuf {
    home.join(".cursor").join("mcp.json")
}

fn read_json(path: &Path) -> Value {
    let contents =
        fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&contents).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

#[test]
fn register_creates_configs_from_absent() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();

    assert_success(&run_setup(&home, &["setup", "--register-mcp"]));

    for path in [claude_config(&home), cursor_config(&home)] {
        assert!(path.exists(), "expected {} to be created", path.display());
        let json = read_json(&path);
        let server = &json["mcpServers"]["quaid"];
        assert_eq!(server["command"], "quaid");
        assert_eq!(server["args"], serde_json::json!(["serve"]));
        let db = server["env"]["QUAID_DB"].as_str().unwrap();
        assert!(
            db.ends_with(".quaid/memory.db") || db.ends_with(".quaid\\memory.db"),
            "QUAID_DB should point at the resolved default DB path, got {db}"
        );
        // The home was expanded — no literal tilde should remain.
        assert!(!db.contains('~'), "QUAID_DB should not contain a literal ~");
    }
}

#[test]
fn register_preserves_other_servers_and_keys() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    let claude = claude_config(&home);
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::write(
        &claude,
        r#"{
  "mcpServers": {
    "other": { "command": "other-bin", "args": ["run"] }
  },
  "topLevel": 42
}"#,
    )
    .unwrap();

    assert_success(&run_setup(&home, &["setup", "--register-mcp"]));

    let json = read_json(&claude);
    // Existing server untouched.
    assert_eq!(json["mcpServers"]["other"]["command"], "other-bin");
    assert_eq!(
        json["mcpServers"]["other"]["args"],
        serde_json::json!(["run"])
    );
    // Unrelated top-level key untouched.
    assert_eq!(json["topLevel"], 42);
    // Quaid server merged in.
    assert_eq!(json["mcpServers"]["quaid"]["command"], "quaid");
}

#[test]
fn register_backs_up_pre_existing_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    let claude = claude_config(&home);
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    let original = r#"{ "mcpServers": { "other": { "command": "x" } } }"#;
    fs::write(&claude, original).unwrap();

    assert_success(&run_setup(&home, &["setup", "--register-mcp"]));

    let backup = {
        let mut name = claude.into_os_string();
        name.push(".bak");
        PathBuf::from(name)
    };
    assert!(backup.exists(), "expected a .bak of the pre-existing file");
    assert_eq!(
        fs::read_to_string(&backup).unwrap(),
        original,
        ".bak should hold the byte-exact original"
    );
}

#[test]
fn register_is_idempotent() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();

    assert_success(&run_setup(&home, &["setup", "--register-mcp"]));
    let after_first = fs::read_to_string(claude_config(&home)).unwrap();

    let second = run_setup(&home, &["setup", "--register-mcp"]);
    assert_success(&second);
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("already up to date"),
        "second run should report no changes, got:\n{stdout}"
    );

    let after_second = fs::read_to_string(claude_config(&home)).unwrap();
    assert_eq!(after_first, after_second, "idempotent run changed the file");

    // No .bak should be created when nothing changes.
    let backup = {
        let mut name = claude_config(&home).into_os_string();
        name.push(".bak");
        PathBuf::from(name)
    };
    assert!(!backup.exists(), "no .bak expected on a no-op run");
}

#[test]
fn dry_run_writes_nothing() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();

    let output = run_setup(&home, &["setup", "--register-mcp", "--dry-run"]);
    assert_success(&output);

    assert!(
        !claude_config(&home).exists(),
        "dry-run must not create the Claude config"
    );
    assert!(
        !cursor_config(&home).exists(),
        "dry-run must not create the Cursor config"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Dry run"), "dry-run should announce itself");
    assert!(
        stdout.contains("would write"),
        "dry-run should print the planned diff"
    );
}
