#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Doc-lint gate: no doc may claim skills are "extracted to ~/.quaid/skills"
//! (or extracted "on first run") unless the `quaid skills extract` command is
//! mentioned nearby. Skills are embedded in the binary and materialized only
//! on demand via `quaid skills extract`; the old "extracted on first run"
//! wording was false and walked users through editing files that did not
//! exist. This test fails CI if that wording is reintroduced without the
//! command that makes it true.

use std::fs;
use std::path::{Path, PathBuf};

/// Phrases that assert silent/automatic extraction of skills to disk.
const FORBIDDEN_PHRASES: &[&str] = &[
    "extracted to `~/.quaid/skills",
    "extracted to ~/.quaid/skills",
    "extracts them to `~/.quaid/skills",
    "extracts them on first run",
    "extracted on first run",
    "extract to <code>~/.quaid/skills",
    "extracts every embedded skill into",
];

/// If a forbidden phrase appears, `skills extract` must appear within this many
/// lines on either side to prove the doc describes the on-demand command.
const PROXIMITY_LINES: usize = 12;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Collect doc files to lint: top-level README/CLAUDE, everything under docs/,
/// and the website's markdown/MDX content.
fn doc_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for top in ["README.md", "CLAUDE.md"] {
        let path = root.join(top);
        if path.exists() {
            files.push(path);
        }
    }
    collect_markdown(&root.join("docs"), &mut files);
    collect_markdown(&root.join("website").join("src"), &mut files);
    files
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md") | Some("mdx")
        ) {
            out.push(path);
        }
    }
}

fn nearby_has_extract_command(lines: &[&str], center: usize) -> bool {
    let start = center.saturating_sub(PROXIMITY_LINES);
    let end = (center + PROXIMITY_LINES + 1).min(lines.len());
    lines[start..end]
        .iter()
        .any(|line| line.contains("skills extract"))
}

#[test]
fn no_first_run_extraction_claims_without_extract_command() {
    let root = repo_root();
    let files = doc_files(&root);
    assert!(
        files.len() >= 8,
        "expected to lint many doc files, found {}",
        files.len()
    );

    let mut violations = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read doc file");
        let lines: Vec<&str> = text.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            for phrase in FORBIDDEN_PHRASES {
                if line.contains(phrase) && !nearby_has_extract_command(&lines, idx) {
                    violations.push(format!(
                        "{}:{}: `{}` (no `skills extract` within {} lines)",
                        file.display(),
                        idx + 1,
                        line.trim(),
                        PROXIMITY_LINES
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "stale skill-extraction claims found:\n{}",
        violations.join("\n")
    );
}
