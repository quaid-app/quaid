use quaid::core::db;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeSet;
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
    env!("CARGO_BIN_EXE_quaid")
}

fn run_quaid(db_path: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(bin_path());
    command.arg("--db").arg(db_path).args(args);
    command.output().expect("run quaid")
}

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON")
}

fn test_db_path(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

fn result_slugs(results: &[Value]) -> BTreeSet<String> {
    results
        .iter()
        .map(|result| {
            result
                .get("slug")
                .and_then(Value::as_str)
                .expect("each result must include a slug")
                .to_owned()
        })
        .collect()
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

    let output = run_quaid(&db_path, &["--json", "search", "50% fee reduction"]);

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

    let output = run_quaid(&db_path, &["--json", "search", "--raw", "?invalid"]);

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
fn call_memory_search_question_query_returns_json_array() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "memory-search-call.db");
    let conn = open_test_db(&db_path);
    insert_page(
        &conn,
        "concepts/clarity",
        "concept",
        "CLARITY",
        "CLARITY note",
    );
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "call",
            "memory_search",
            r#"{"query":"what is CLARITY?","limit":5}"#,
        ],
    );

    assert!(
        output.status.success(),
        "call memory_search should exit cleanly: {output:?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "call memory_search should not write errors to stderr: {:?}",
        output.stderr
    );

    let parsed = parse_stdout_json(&output);
    assert!(
        parsed.is_array(),
        "memory_search call must emit a JSON array"
    );
}

// ── Regression: compound-term recall issues #67 and #69 ──────────────────────

/// Regression #67/#69: `quaid search "neural network inference"` must not return
/// an empty array when the corpus contains pages about neural networks and inference
/// separately.  The OR fallback must surface them.
#[test]
fn search_compound_terms_finds_docs_when_any_token_matches() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "compound-recall.db");
    let conn = open_test_db(&db_path);

    // Three pages — each covers only ONE of the three query tokens.
    insert_page(
        &conn,
        "concepts/neural",
        "concept",
        "Neural Networks",
        "A neural network is a layered ML model.",
    );
    insert_page(
        &conn,
        "concepts/inference",
        "concept",
        "Inference Engines",
        "Inference engines deploy trained models.",
    );
    insert_page(
        &conn,
        "concepts/network",
        "concept",
        "Computer Networks",
        "Packet-switched network topology.",
    );
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "search", "neural network inference"]);

    assert!(
        output.status.success(),
        "search should exit cleanly: {output:?}"
    );

    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    let slugs = result_slugs(results);
    assert!(
        slugs
            == BTreeSet::from([
                "default::concepts/inference".to_owned(),
                "default::concepts/network".to_owned(),
                "default::concepts/neural".to_owned(),
            ]),
        "compound-term search must widen to canonical OR results when no page satisfies the \
         implicit-AND pass (regression for issues #67 and #69)"
    );
}

/// When at least one page matches ALL tokens (AND path), results must include it.
#[test]
fn search_compound_terms_and_path_takes_precedence() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "compound-and.db");
    let conn = open_test_db(&db_path);

    // One page contains all three tokens; others contain only one.
    insert_page(
        &conn,
        "concepts/combined",
        "concept",
        "Neural Network Inference",
        "Neural network inference is the deployment of a trained model.",
    );
    insert_page(
        &conn,
        "concepts/neural-only",
        "concept",
        "Neural",
        "Neural science article unrelated to networks.",
    );
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "search", "neural network inference"]);
    assert!(output.status.success(), "search should exit cleanly: {output:?}");

    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert_eq!(
        result_slugs(results),
        BTreeSet::from(["default::concepts/combined".to_owned()]),
        "non-raw CLI search must keep canonical slugs and stop at the AND hit instead of \
         widening to OR results"
    );
}

/// `--raw` flag must bypass OR fallback — expert FTS5 query behaviour is unchanged.
#[test]
fn search_raw_mode_bypasses_or_fallback() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "raw-no-fallback.db");
    let conn = open_test_db(&db_path);

    // Corpus: two pages, each with one token.
    insert_page(
        &conn,
        "concepts/neural",
        "concept",
        "Neural",
        "Neural network model.",
    );
    insert_page(
        &conn,
        "concepts/inference",
        "concept",
        "Inference",
        "Inference engine.",
    );
    drop(conn);

    // Raw search preserves implicit AND semantics — no single page matches all terms.
    let output = run_quaid(
        &db_path,
        &["--json", "search", "--raw", "neural network inference"],
    );
    assert!(output.status.success(), "raw search should exit cleanly: {output:?}");
    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert!(
        results.is_empty(),
        "--raw query must not get an OR fallback; expected empty array"
    );
}

/// MCP memory_search returns a valid JSON array (safety contract — no crash).
/// NOTE: compound-term OR fallback is NOT applied to memory_search per the
/// compound-term-recall design decision (MCP agents needing compound recall
/// should use memory_query which includes the hybrid/vector arm).
#[test]
fn memory_search_compound_query_returns_valid_json_array() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "mcp-compound-safety.db");
    let conn = open_test_db(&db_path);

    insert_page(
        &conn,
        "concepts/neural",
        "concept",
        "Neural Networks",
        "A neural network is a machine learning model.",
    );
    insert_page(
        &conn,
        "concepts/inference",
        "concept",
        "Inference Engines",
        "Inference engines deploy trained models.",
    );
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "call",
            "memory_search",
            r#"{"query":"neural network inference","limit":10}"#,
        ],
    );

    assert!(
        output.status.success(),
        "call memory_search should exit cleanly: {output:?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "call memory_search should not write errors to stderr: {:?}",
        output.stderr
    );

    let parsed = parse_stdout_json(&output);
    assert!(
        parsed.is_array(),
        "memory_search must always emit a JSON array (may be empty for compound-term AND miss)"
    );
}
