//! Budget-unit regression: `quaid query` used to spend its token budget in
//! bytes while truncating in chars, so multibyte summaries were overcounted
//! and the budget semantics diverged from `core::progressive` (chars/4).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::process::Command;

#[test]
fn query_token_budget_truncates_multibyte_summaries_by_chars() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    {
        let conn = quaid::core::db::open(db_path.to_str().unwrap()).unwrap();
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES ('notes/jp', 'concept', 'JP', ?1, 'Body.', '', '{}', 'notes', '', 1)",
            [&"あ".repeat(100)],
        )
        .unwrap();
    }

    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    let output = command
        .arg("--db")
        .arg(&db_path)
        .args([
            "query",
            "notes/jp",
            "--depth",
            "none",
            "--token-budget",
            "10",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)
        .expect("query output must stay valid UTF-8 — truncation may not split a character");
    let summary_chars = stdout.chars().filter(|ch| *ch == 'あ').count();
    // 10 tokens = 40 chars; prefix "default::notes/jp: " = 19 chars → 21
    // summary chars. A byte-based budget would have allowed only 7 chars.
    assert_eq!(summary_chars, 21, "stdout: {stdout}");
}
