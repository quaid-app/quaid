#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would generate noise"
)]

//! Subprocess tests for the custom-model download pin policy.
//!
//! `quaid model pull <custom-id>` must refuse before any network call
//! unless the operator supplies both `--allow-unverified-model` and
//! `--model-revision <sha>`. These use a fake model id so the refusal
//! itself is what is observed — there is never a real download attempt.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::process::{Command, Output};

fn run_quaid(args: &[&str]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    // Point HuggingFace at an unroutable base URL: if the refusal ever
    // regressed into an actual fetch, the test would fail loudly rather
    // than reaching the network.
    command.env("QUAID_HF_BASE_URL", "http://127.0.0.1:1");
    command.args(args);
    command.output().expect("run quaid")
}

#[test]
fn model_pull_refuses_unpinned_custom_model_without_flags() {
    let output = run_quaid(&["model", "pull", "fake-org/fake-unpinned-model"]);
    assert!(
        !output.status.success(),
        "unpinned custom pull must fail; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--model-revision"),
        "expected a pin-required error, got stderr={stderr}"
    );
}

#[test]
fn model_pull_refuses_custom_model_with_revision_but_no_allow_flag() {
    let output = run_quaid(&[
        "model",
        "pull",
        "fake-org/fake-unpinned-model",
        "--model-revision",
        "0123456789abcdef0123456789abcdef01234567",
    ]);
    assert!(
        !output.status.success(),
        "must require --allow-unverified-model"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--allow-unverified-model"),
        "expected an unverified-opt-in error, got stderr={stderr}"
    );
}

#[test]
fn model_pull_refuses_revision_override_for_curated_alias() {
    let output = run_quaid(&[
        "model",
        "pull",
        "phi-3.5-mini",
        "--allow-unverified-model",
        "--model-revision",
        "0123456789abcdef0123456789abcdef01234567",
    ]);
    assert!(
        !output.status.success(),
        "curated alias must refuse a revision override"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("curated alias"),
        "expected a curated-alias error, got stderr={stderr}"
    );
}
