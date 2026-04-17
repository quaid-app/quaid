use gbrain::core::db;
use rusqlite::Connection;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().expect("utf-8 db path")).expect("open test db")
}

fn insert_page(conn: &Connection, slug: &str, page_type: &str, title: &str, summary: &str) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, ?2, ?3, ?4, '', '', '{}', '', '', 1)",
        rusqlite::params![slug, page_type, title, summary],
    )
    .expect("insert page");
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_gbrain")
}

fn run_gbrain(db_path: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(bin_path());
    command.arg("--db").arg(db_path).args(args);
    command.output().expect("run gbrain")
}

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON")
}

fn test_db_path(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

#[test]
fn search_json_percent_query_returns_json_array() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "search-json.db");
    let conn = open_test_db(&db_path);
    insert_page(
        &conn,
        "projects/pricing",
        "project",
        "Pricing",
        "Fee reduction workstream",
    );
    drop(conn);

    let output = run_gbrain(&db_path, &["--json", "search", "50% fee reduction"]);

    assert!(
        output.status.success(),
        "search --json should exit cleanly: {output:?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "search --json should not write errors to stderr: {:?}",
        output.stderr
    );

    let parsed = parse_stdout_json(&output);
    assert!(parsed.is_array(), "search --json must emit a JSON array");
}

#[test]
fn search_raw_json_invalid_query_returns_error_object() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "search-raw-json.db");
    let _conn = open_test_db(&db_path);

    let output = run_gbrain(&db_path, &["--json", "search", "--raw", "?invalid"]);

    assert!(
        output.status.success(),
        "search --raw --json should exit cleanly: {output:?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "search --raw --json should not write to stderr: {:?}",
        output.stderr
    );

    let parsed = parse_stdout_json(&output);
    let error = parsed
        .get("error")
        .and_then(Value::as_str)
        .expect("raw invalid query must emit an error string");
    assert!(!error.is_empty(), "error message must not be empty");
}

#[test]
fn call_brain_search_question_query_returns_json_array() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "brain-search-call.db");
    let conn = open_test_db(&db_path);
    insert_page(
        &conn,
        "concepts/clarity",
        "concept",
        "CLARITY",
        "CLARITY note",
    );
    drop(conn);

    let output = run_gbrain(
        &db_path,
        &[
            "call",
            "brain_search",
            r#"{"query":"what is CLARITY?","limit":5}"#,
        ],
    );

    assert!(
        output.status.success(),
        "call brain_search should exit cleanly: {output:?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "call brain_search should not write errors to stderr: {:?}",
        output.stderr
    );

    let parsed = parse_stdout_json(&output);
    assert!(
        parsed.is_array(),
        "brain_search call must emit a JSON array"
    );
}
