#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    dead_code,
    unreachable_pub,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites; common test helpers are pub-shared across test modules but are unreachable from non-test crates"
)]

//! Shared fixtures for the `cli_collection_truth_*` integration test suite.
//!
//! Loaded via `#[path = "common/truth_fixtures.rs"] mod truth_fixtures;` from
//! each per-command test file. Internally relies on `crate::common::quaid_bin`
//! and `crate::common_subprocess::configure_test_command`, so any test file
//! that loads this module must also declare `mod common;` and the
//! `#[path = "common/subprocess.rs"] mod common_subprocess;` shim.

use quaid::core::db;
use quaid::core::markdown::{extract_summary, parse_frontmatter, split_content};
#[cfg(unix)]
use rusqlite::OpenFlags;
use rusqlite::{params, Connection};
use serde_json::Value;
use sha2::Digest;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;

pub fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().expect("utf-8 db path")).expect("open test db")
}

pub fn bin_path() -> &'static Path {
    crate::common::quaid_bin()
}

pub fn run_quaid(db_path: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(bin_path());
    crate::common_subprocess::configure_test_command(&mut command);
    command.arg("--db").arg(db_path).args(args);
    command.output().expect("run quaid")
}

pub fn run_quaid_with_env(
    db_path: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> std::process::Output {
    let mut command = Command::new(bin_path());
    crate::common_subprocess::configure_test_command(&mut command);
    command
        .arg("--db")
        .arg(db_path)
        .args(args)
        .envs(envs.iter().copied());
    command.output().expect("run quaid")
}

pub fn run_quaid_with_stdin(db_path: &Path, args: &[&str], stdin: &str) -> std::process::Output {
    let mut command = Command::new(bin_path());
    crate::common_subprocess::configure_test_command(&mut command);
    command
        .arg("--db")
        .arg(db_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn quaid");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for quaid")
}

pub fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout must be valid JSON: {err}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

pub fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

pub fn assert_ambiguous_slug_failure(
    output: &std::process::Output,
    slug: &str,
    candidates: &[&str],
) {
    assert!(
        !output.status.success(),
        "ambiguous bare slug should fail: {output:?}"
    );
    let text = combined_output(output);
    assert!(
        text.contains(&format!("ambiguous slug: {slug}")),
        "ambiguous bare slug must surface the routing failure: {text}"
    );
    for candidate in candidates {
        assert!(
            text.contains(candidate),
            "ambiguous bare slug must include candidate {candidate}: {text}"
        );
    }
}

pub fn test_db_path(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

#[cfg(unix)]
pub fn init_db(dir: &tempfile::TempDir) -> PathBuf {
    let db_path = test_db_path(dir, "test.db");
    drop(open_test_db(&db_path));
    db_path
}

#[cfg(unix)]
pub fn wait_for_db_value<T>(
    db_path: &Path,
    timeout: Duration,
    mut probe: impl FnMut(&Connection) -> Option<T>,
) -> Option<T> {
    let started = std::time::Instant::now();
    while started.elapsed() <= timeout {
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open read-only poll db");
        if let Some(value) = probe(&conn) {
            return Some(value);
        }
        drop(conn);
        thread::sleep(Duration::from_millis(25));
    }
    None
}

pub fn insert_collection(conn: &Connection, name: &str, root_path: &Path) -> i64 {
    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target)
         VALUES (?1, ?2, 'active', 1, 0)",
        params![name, root_path.display().to_string()],
    )
    .expect("insert collection");
    conn.last_insert_rowid()
}

pub fn insert_page(conn: &Connection, collection_id: i64, slug: &str) {
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, 'note', ?2, '', 'compiled', '', '{}', 'notes', '', 1)",
        params![collection_id, slug, uuid::Uuid::now_v7().to_string()],
    )
    .expect("insert page");
}

pub fn insert_page_with_truth(conn: &Connection, collection_id: i64, slug: &str, truth: &str) {
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, 'note', ?2, '', ?4, '', '{}', 'notes', '', 1)",
        params![
            collection_id,
            slug,
            uuid::Uuid::now_v7().to_string(),
            truth
        ],
    )
    .expect("insert page with truth");
}

pub fn page_id(conn: &Connection, collection_id: i64, slug: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
        params![collection_id, slug],
        |row| row.get(0),
    )
    .expect("load page id")
}

pub fn insert_timeline_entry(conn: &Connection, page_id: i64, date: &str, summary: &str) {
    let summary_hash = format!("{:x}", sha2::Sha256::digest(summary.as_bytes()));
    conn.execute(
        "INSERT INTO timeline_entries (page_id, date, source, summary, summary_hash, detail)
         VALUES (?1, ?2, '', ?3, ?4, '')",
        params![page_id, date, summary, summary_hash],
    )
    .expect("insert timeline entry");
}

pub fn quarantine_page(conn: &Connection, page_id: i64, quarantined_at: &str) {
    conn.execute(
        "UPDATE pages SET quarantined_at = ?2 WHERE id = ?1",
        params![page_id, quarantined_at],
    )
    .expect("quarantine page");
}

pub fn insert_programmatic_link(conn: &Connection, from_page_id: i64, to_page_id: i64) {
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, context, source_kind)
         VALUES (?1, ?2, 'related', '', 'programmatic')",
        params![from_page_id, to_page_id],
    )
    .expect("insert programmatic link");
}

pub fn insert_knowledge_gap(conn: &Connection, page_id: i64, hash: &str) {
    conn.execute(
        "INSERT INTO knowledge_gaps (page_id, query_hash, context)
         VALUES (?1, ?2, 'gap context')",
        params![page_id, hash],
    )
    .expect("insert knowledge gap");
}

pub fn insert_page_with_raw_import(
    conn: &Connection,
    collection_id: i64,
    slug: &str,
    uuid: &str,
    raw_bytes: &[u8],
    relative_path: &str,
) {
    let raw = String::from_utf8_lossy(raw_bytes);
    let (frontmatter, body) = parse_frontmatter(&raw);
    let (compiled_truth, timeline) = split_content(&body);
    let title = frontmatter
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| slug.to_string());
    let page_type = frontmatter
        .get("type")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| "concept".to_string());
    let summary = extract_summary(&compiled_truth);
    let frontmatter_json = serde_json::to_string(&frontmatter).expect("serialize frontmatter");
    conn.execute(
        "INSERT INTO pages
              (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'notes', '', 1)",
        params![
            collection_id,
            slug,
            uuid,
            page_type,
            title,
            summary,
            compiled_truth,
            timeline,
            frontmatter_json
        ],
    )
    .expect("insert page with uuid");
    let page_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, content_hash, raw_bytes, file_path)
         VALUES (?1, ?2, 1, ?3, ?4, ?5)",
        params![
            page_id,
            uuid::Uuid::now_v7().to_string(),
            format!("{:x}", sha2::Sha256::digest(raw_bytes)),
            raw_bytes,
            relative_path
        ],
    )
    .expect("insert raw import");
    let sha256 = format!("{:x}", sha2::Sha256::digest(raw_bytes));
    conn.execute(
        "INSERT INTO file_state (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256)
         VALUES (?1, ?2, ?3, 1, 1, ?4, 1, ?5)",
        params![
            collection_id,
            relative_path,
            page_id,
            raw_bytes.len() as i64,
            sha256
        ],
    )
    .expect("insert file state");
}

#[cfg(unix)]
pub fn raw_import_counts(conn: &Connection, page_id: i64) -> (i64, i64) {
    conn.query_row(
        "SELECT
             SUM(CASE WHEN is_active = 1 THEN 1 ELSE 0 END),
             COUNT(*)
         FROM raw_imports
         WHERE page_id = ?1",
        [page_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("load raw import counts")
}

#[cfg(unix)]
pub fn assert_cli_lease_released(conn: &Connection, collection_id: i64) {
    let owner_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .expect("load owner count");
    assert_eq!(owner_count, 0, "short-lived owner lease must be released");

    let cli_session_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_type = 'cli'",
            [],
            |row| row.get(0),
        )
        .expect("load cli session count");
    assert_eq!(
        cli_session_count, 0,
        "short-lived CLI session must be cleaned up after inline completion"
    );
}
