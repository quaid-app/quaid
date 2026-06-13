#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test fixtures panic with useful command diagnostics"
)]

//! Asserts the onboarding `setup` skill ships embedded in the binary.
//!
//! Runs `quaid skills list --json` from a temp working directory with an
//! isolated `$HOME` so neither the repo's `./skills/` nor `~/.quaid/skills/`
//! override layers can mask the embedded copy.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::fs;
use std::process::Command;

use serde_json::Value;

fn run(command: &mut Command) -> std::process::Output {
    common_subprocess::configure_test_command(command);
    command.output().expect("run quaid")
}

#[test]
fn setup_skill_is_embedded() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    let workdir = dir.path().join("work");
    let db_path = dir.path().join("memory.db");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&workdir).unwrap();

    // `skills list` opens the database, so initialize one first.
    let mut init = Command::new(common::quaid_bin());
    init.env("HOME", &home)
        .env("USERPROFILE", &home)
        .args(["init", db_path.to_str().unwrap()]);
    let init_out = run(&mut init);
    assert!(
        init_out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );

    let mut command = Command::new(common::quaid_bin());
    command
        .current_dir(&workdir)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .args([
            "--db",
            db_path.to_str().unwrap(),
            "skills",
            "list",
            "--json",
        ]);
    let output = run(&mut command);

    assert!(
        output.status.success(),
        "skills list failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let skills: Value = serde_json::from_slice(&output.stdout).expect("parse skills json");
    let array = skills.as_array().expect("skills list is a JSON array");

    let setup = array
        .iter()
        .find(|s| s["name"] == "setup")
        .expect("setup skill should be present in `quaid skills list`");

    assert_eq!(
        setup["source"], "embedded://skills/setup/SKILL.md",
        "setup skill should resolve to the embedded copy"
    );
    assert_eq!(
        setup["shadowed"], false,
        "embedded setup skill should not be shadowed in a clean working dir"
    );
}
