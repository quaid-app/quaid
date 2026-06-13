#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Validates that every fenced `quaid …` command in `skills/*/SKILL.md`
//! parses against the real clap surface, and exercises the
//! `quaid skills extract` command (write, refuse-modified, `--force`).
//!
//! The command-parsing sweep catches the classes of bug the skills shipped
//! with for months: `quaid gaps --resolved true` (a value passed to a bare
//! boolean flag) and `quaid alerts` (a subcommand that does not exist). clap
//! rejects both with exit code 2 before any side effect runs, so a subprocess
//! that exits 2 is a parse regression in the skill.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// clap exits with this status when argument parsing fails (unknown
/// subcommand, unexpected value, invalid value, missing required arg, …).
const CLAP_USAGE_ERROR: i32 = 2;

fn skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills")
}

fn run_quaid_isolated(args: &[String], home: &Path) -> std::process::Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    // Point HOME at a throwaway dir so any command that *does* parse and try to
    // touch the default DB fails at runtime (exit 1), never at parse time.
    command
        .env("HOME", home)
        .args(args)
        .output()
        .expect("run quaid")
}

/// Normalize one fenced shell line into an argv suitable for clap.
///
/// Returns `None` for lines that are not a single self-contained `quaid`
/// invocation (multi-line JSON args, etc.). Handles: trailing `#` comments,
/// line-continuation backslashes, top-level `|`/`&&`/`;` operators, `<`/`>`
/// redirects, and `<placeholder>` tokens (substituted with `1`, which parses
/// as both a string slug and a number).
fn normalize_command(raw: &str) -> Option<Vec<String>> {
    // Odd single-quote count => this line opens a multi-line quoted arg
    // (e.g. `quaid call memory_raw '{`). Not a standalone command.
    if !raw.matches('\'').count().is_multiple_of(2) {
        return None;
    }

    let mut cmd = raw.to_string();

    // Drop a line-continuation backslash and anything after it.
    if let Some(idx) = cmd.find('\\') {
        cmd.truncate(idx);
    }

    // Drop a trailing `# comment`.
    if let Some(idx) = cmd.find(" #") {
        cmd.truncate(idx);
    }

    // Cut at the first top-level shell operator that lives outside quotes.
    for op in ["|", "&&", ";"] {
        if let Some(idx) = cmd.find(op) {
            let prefix = &cmd[..idx];
            if prefix.matches('\'').count().is_multiple_of(2)
                && prefix.matches('"').count().is_multiple_of(2)
            {
                cmd.truncate(idx);
            }
        }
    }

    // Substitute `<placeholder>` tokens BEFORE stripping redirects, so a
    // placeholder like `< stub.md` is not mistaken for a redirect (and a real
    // `< file` redirect is correctly removed afterward).
    let placeholder = regex_lite_replace(&cmd);
    let mut cmd = strip_redirects(&placeholder);

    cmd = cmd.trim().to_string();
    if cmd.is_empty() {
        return None;
    }

    let argv = shell_split(&cmd)?;
    if argv.first().map(String::as_str) != Some("quaid") {
        return None;
    }
    Some(argv)
}

/// Replace every `<...>` placeholder token with `1`.
fn regex_lite_replace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            // Consume up to and including the matching `>`.
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == '>' {
                    closed = true;
                    break;
                }
            }
            if closed {
                out.push('1');
            } else {
                // Unbalanced `<` — keep it literal.
                out.push('<');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Remove ` < word` / ` > word` shell redirects.
fn strip_redirects(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if (c == '<' || c == '>') && i > 0 && bytes[i - 1] == ' ' {
            // Skip the operator, optional spaces, and the following word.
            i += 1;
            while i < bytes.len() && bytes[i] == ' ' {
                i += 1;
            }
            while i < bytes.len() && bytes[i] != ' ' {
                i += 1;
            }
            // Trim the trailing space we already pushed before the operator.
            while out.ends_with(' ') {
                out.pop();
            }
            out.push(' ');
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Minimal POSIX-ish word splitter honoring single and double quotes.
fn shell_split(input: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut has_token = false;

    for c in input.chars() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                has_token = true;
            }
            '"' if !in_single => {
                in_double = !in_double;
                has_token = true;
            }
            ' ' | '\t' if !in_single && !in_double => {
                if has_token {
                    words.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            _ => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if in_single || in_double {
        return None;
    }
    if has_token {
        words.push(current);
    }
    Some(words)
}

fn fenced_quaid_commands(skill_text: &str) -> Vec<(usize, String)> {
    let mut commands = Vec::new();
    let mut in_fence = false;
    for (idx, line) in skill_text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence && trimmed.starts_with("quaid ") {
            commands.push((idx + 1, trimmed.to_string()));
        }
    }
    commands
}

#[test]
fn every_fenced_skill_command_parses_with_clap() {
    let home = tempfile::tempdir().expect("temp home");
    let dir = skills_dir();
    let mut checked = 0usize;
    let mut failures = Vec::new();

    let entries = fs::read_dir(&dir).expect("read skills dir");
    for entry in entries {
        let entry = entry.expect("dir entry");
        let skill_file = entry.path().join("SKILL.md");
        if !skill_file.exists() {
            continue;
        }
        let text = fs::read_to_string(&skill_file).expect("read SKILL.md");
        for (line_no, raw) in fenced_quaid_commands(&text) {
            let Some(argv) = normalize_command(&raw) else {
                continue;
            };
            checked += 1;
            let output = run_quaid_isolated(&argv[1..], home.path());
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Exit code 2 is clap's usage-error code, but some subcommands also
            // use exit 2 as an intentional runtime state signal (e.g. `quaid
            // status` returns 2 for "daemon not installed"). A genuine clap
            // parse failure writes a usage banner to stderr, so require that
            // signature before flagging — otherwise a parse-valid command that
            // merely exits non-zero at runtime is a false positive.
            let looks_like_clap_error = stderr.contains("error:")
                && (stderr.contains("Usage:") || stderr.contains("unexpected argument"));
            if output.status.code() == Some(CLAP_USAGE_ERROR) && looks_like_clap_error {
                let first = stderr.lines().next().unwrap_or("");
                failures.push(format!(
                    "{}:{line_no}: `{raw}`\n  argv={:?}\n  {first}",
                    skill_file.display(),
                    &argv[1..]
                ));
            }
        }
    }

    assert!(
        checked >= 40,
        "expected to validate many commands, got {checked}"
    );
    assert!(
        failures.is_empty(),
        "{} skill command(s) failed clap parsing:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn skills_extract_writes_all_embedded_skills() {
    let home = tempfile::tempdir().expect("temp home");
    let target = home.path().join("out");

    let output = run_quaid_isolated(
        &[
            "skills".into(),
            "extract".into(),
            "--dir".into(),
            target.display().to_string(),
        ],
        home.path(),
    );
    assert!(
        output.status.success(),
        "extract failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for name in ["ingest", "query", "maintain", "alerts", "upgrade"] {
        let written = target.join(name).join("SKILL.md");
        assert!(written.exists(), "expected {name} extracted to {written:?}");
        let body = fs::read_to_string(&written).expect("read extracted skill");
        assert!(body.contains("---"), "{name} should carry frontmatter");
    }
}

#[test]
fn skills_extract_single_skill_only() {
    let home = tempfile::tempdir().expect("temp home");
    let target = home.path().join("out");

    let output = run_quaid_isolated(
        &[
            "skills".into(),
            "extract".into(),
            "query".into(),
            "--dir".into(),
            target.display().to_string(),
        ],
        home.path(),
    );
    assert!(output.status.success(), "single extract failed");

    assert!(target.join("query").join("SKILL.md").exists());
    assert!(
        !target.join("ingest").exists(),
        "named extract must not write other skills"
    );
}

#[test]
fn skills_extract_unknown_skill_errors() {
    let home = tempfile::tempdir().expect("temp home");
    let target = home.path().join("out");

    let output = run_quaid_isolated(
        &[
            "skills".into(),
            "extract".into(),
            "does-not-exist".into(),
            "--dir".into(),
            target.display().to_string(),
        ],
        home.path(),
    );
    assert!(
        !output.status.success(),
        "extracting an unknown skill should fail"
    );
}

#[test]
fn skills_extract_refuses_modified_without_force() {
    let home = tempfile::tempdir().expect("temp home");
    let target = home.path().join("out");
    let query_skill = target.join("query").join("SKILL.md");

    // Pre-seed a modified local copy.
    fs::create_dir_all(query_skill.parent().unwrap()).expect("mkdir");
    fs::write(&query_skill, "LOCAL EDIT — do not clobber\n").expect("seed local skill");

    let output = run_quaid_isolated(
        &[
            "skills".into(),
            "extract".into(),
            "query".into(),
            "--dir".into(),
            target.display().to_string(),
        ],
        home.path(),
    );
    assert!(
        output.status.success(),
        "extract should succeed (skip), not error: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let preserved = fs::read_to_string(&query_skill).expect("read skill");
    assert_eq!(
        preserved, "LOCAL EDIT — do not clobber\n",
        "modified local skill must be left untouched without --force"
    );
}

#[test]
fn skills_extract_force_overwrites_modified() {
    let home = tempfile::tempdir().expect("temp home");
    let target = home.path().join("out");
    let query_skill = target.join("query").join("SKILL.md");

    fs::create_dir_all(query_skill.parent().unwrap()).expect("mkdir");
    fs::write(&query_skill, "LOCAL EDIT\n").expect("seed local skill");

    let output = run_quaid_isolated(
        &[
            "skills".into(),
            "extract".into(),
            "query".into(),
            "--dir".into(),
            target.display().to_string(),
            "--force".into(),
        ],
        home.path(),
    );
    assert!(
        output.status.success(),
        "force extract failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let overwritten = fs::read_to_string(&query_skill).expect("read skill");
    assert_ne!(overwritten, "LOCAL EDIT\n", "--force must overwrite");
    assert!(
        overwritten.contains("name: quaid-query"),
        "--force must write the embedded query skill"
    );
}
