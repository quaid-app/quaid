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
    insert_page_with_namespace(conn, "", slug, page_type, title, summary, "");
}

fn insert_page_with_namespace(
    conn: &Connection,
    namespace: &str,
    slug: &str,
    page_type: &str,
    title: &str,
    summary: &str,
    compiled_truth: &str,
) {
    conn.execute(
        "INSERT INTO pages (namespace, slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', '{}', '', '', 1)",
        rusqlite::params![namespace, slug, page_type, title, summary, compiled_truth],
    )
    .expect("insert page");
}

fn bin_path() -> &'static Path {
    common::quaid_bin()
}

fn run_quaid(db_path: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(bin_path());
    common_subprocess::configure_test_command(&mut command);
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
fn search_namespace_filter_returns_requested_namespace_plus_global_only() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "search-namespace-filter.db");
    let conn = open_test_db(&db_path);
    insert_page_with_namespace(
        &conn,
        "",
        "notes/global-bitcoin",
        "concept",
        "Global Bitcoin",
        "Global namespace page",
        "bitcoin evidence in global namespace",
    );
    insert_page_with_namespace(
        &conn,
        "workns",
        "notes/work-bitcoin",
        "concept",
        "Work Bitcoin",
        "Work namespace page",
        "bitcoin evidence in work namespace",
    );
    insert_page_with_namespace(
        &conn,
        "otherns",
        "notes/other-bitcoin",
        "concept",
        "Other Bitcoin",
        "Other namespace page",
        "bitcoin evidence in another namespace",
    );
    drop(conn);

    let namespaced_output = run_quaid(
        &db_path,
        &["--json", "search", "--namespace", "workns", "bitcoin"],
    );
    assert!(
        namespaced_output.status.success(),
        "namespaced search should exit cleanly: {namespaced_output:?}"
    );
    assert!(
        String::from_utf8_lossy(&namespaced_output.stderr)
            .trim()
            .is_empty(),
        "namespaced search should not write errors to stderr: {:?}",
        namespaced_output.stderr
    );

    let parsed = parse_stdout_json(&namespaced_output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert_eq!(
        result_slugs(results),
        BTreeSet::from([
            "default::notes/global-bitcoin".to_owned(),
            "default::notes/work-bitcoin".to_owned(),
        ]),
        "--namespace workns must return only workns plus global matches"
    );

    let global_output = run_quaid(&db_path, &["--json", "search", "bitcoin"]);
    assert!(
        global_output.status.success(),
        "global search should exit cleanly: {global_output:?}"
    );
    assert!(
        String::from_utf8_lossy(&global_output.stderr)
            .trim()
            .is_empty(),
        "global search should not write errors to stderr: {:?}",
        global_output.stderr
    );

    let parsed = parse_stdout_json(&global_output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert_eq!(
        result_slugs(results),
        BTreeSet::from(["default::notes/global-bitcoin".to_owned()]),
        "omitted --namespace must default to global-only search"
    );
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
        parsed
            .get("results")
            .is_some_and(serde_json::Value::is_array),
        "memory_search call must emit a `results` array: {parsed:?}"
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

/// When at least one page matches ALL tokens (AND path), it must rank first;
/// partial-match pages blend in below it instead of being suppressed.
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
    assert!(
        output.status.success(),
        "search should exit cleanly: {output:?}"
    );

    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert_eq!(
        results[0].get("slug").and_then(Value::as_str),
        Some("default::concepts/combined"),
        "the AND hit must rank first"
    );
    assert_eq!(
        result_slugs(results),
        BTreeSet::from([
            "default::concepts/combined".to_owned(),
            "default::concepts/neural-only".to_owned(),
        ]),
        "non-raw CLI search must blend OR-recall hits below the AND hit"
    );
}

/// Tiered blending: a page matching all four query terms ranks first, and a
/// page matching only three of four still surfaces below it instead of being
/// suppressed by the AND pass.
#[test]
fn search_blends_three_of_four_term_match_below_full_match() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "compound-blend.db");
    let conn = open_test_db(&db_path);

    insert_page_with_namespace(
        &conn,
        "",
        "notes/marginal",
        "concept",
        "Marginal",
        "Marginal note",
        "A glancing aside on quasar harmonics, tideflow, and basalt drift.",
    );
    insert_page_with_namespace(
        &conn,
        "",
        "notes/relevant",
        "concept",
        "Relevant",
        "Relevant note",
        "Quasar harmonics and tideflow interact strongly. Harmonics dominate \
         tideflow measurements when quasar emissions spike.",
    );
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["--json", "search", "quasar harmonics tideflow basalt"],
    );
    assert!(
        output.status.success(),
        "search should exit cleanly: {output:?}"
    );

    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert_eq!(
        results[0].get("slug").and_then(Value::as_str),
        Some("default::notes/marginal"),
        "the 4-of-4 AND hit must rank first: {results:?}"
    );
    assert_eq!(
        result_slugs(results),
        BTreeSet::from([
            "default::notes/marginal".to_owned(),
            "default::notes/relevant".to_owned(),
        ]),
        "the 3-of-4 match must blend in below the AND hit instead of being suppressed: {results:?}"
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
    assert!(
        output.status.success(),
        "raw search should exit cleanly: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert!(
        results.is_empty(),
        "--raw query must not get an OR fallback; expected empty array"
    );
}

/// MCP memory_search returns a valid JSON array (safety contract — no crash).
/// memory_search now routes through the tiered AND→OR path (surface parity
/// with CLI `quaid search`), so the compound query also yields results here.
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
    // memory_search now returns a {results, pending_embedding_jobs?} envelope.
    let results = parsed["results"]
        .as_array()
        .expect("memory_search must emit a `results` array");
    assert_eq!(
        result_slugs(results),
        BTreeSet::from([
            "default::concepts/neural".to_owned(),
            "default::concepts/inference".to_owned(),
        ]),
        "memory_search must widen to OR recall when the AND pass misses"
    );
}

#[test]
fn search_numeric_query_matches_abbreviated_number_forms() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "search-numeric-alias.db");
    let conn = open_test_db(&db_path);

    insert_page_with_namespace(
        &conn,
        "",
        "journal/2026-04-15",
        "journal",
        "Journal 2026-04-15",
        "BTC update",
        "BTC clearing $75K in April.",
    );
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "search", "75000 April"]);

    assert!(
        output.status.success(),
        "search should exit cleanly for numeric alias queries: {output:?}"
    );

    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert!(
        result_slugs(results).contains("default::journal/2026-04-15"),
        "numeric alias expansion should match $75K content: {results:?}"
    );
}

#[test]
fn memory_search_numeric_query_matches_abbreviated_number_forms() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "memory-search-numeric-alias.db");
    let conn = open_test_db(&db_path);

    insert_page_with_namespace(
        &conn,
        "",
        "journal/2026-04-15",
        "journal",
        "Journal 2026-04-15",
        "BTC update",
        "BTC clearing $75K in April.",
    );
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "call",
            "memory_search",
            r#"{"query":"75000 April","limit":10}"#,
        ],
    );

    assert!(
        output.status.success(),
        "memory_search should exit cleanly for numeric alias queries: {output:?}"
    );

    let parsed = parse_stdout_json(&output);
    let results = parsed["results"]
        .as_array()
        .expect("output must carry a `results` array");
    assert!(
        result_slugs(results).contains("default::journal/2026-04-15"),
        "memory_search numeric alias expansion should match $75K content: {results:?}"
    );
}

#[test]
fn search_numeric_query_matches_grouped_number_forms() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "search-numeric-grouped.db");
    let conn = open_test_db(&db_path);

    insert_page_with_namespace(
        &conn,
        "",
        "journal/2026-04-16",
        "journal",
        "Journal 2026-04-16",
        "Revenue update",
        "Revenue reached 75,000 in April.",
    );
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "search", "75000 April"]);

    assert!(
        output.status.success(),
        "search should exit cleanly for grouped numeric queries: {output:?}"
    );

    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert!(
        result_slugs(results).contains("default::journal/2026-04-16"),
        "numeric alias expansion should match 75,000 content: {results:?}"
    );
}

#[test]
fn memory_search_numeric_query_matches_grouped_number_forms() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "memory-search-numeric-grouped.db");
    let conn = open_test_db(&db_path);

    insert_page_with_namespace(
        &conn,
        "",
        "journal/2026-04-16",
        "journal",
        "Journal 2026-04-16",
        "Revenue update",
        "Revenue reached 75,000 in April.",
    );
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "call",
            "memory_search",
            r#"{"query":"75000 April","limit":10}"#,
        ],
    );

    assert!(
        output.status.success(),
        "memory_search should exit cleanly for grouped numeric queries: {output:?}"
    );

    let parsed = parse_stdout_json(&output);
    let results = parsed["results"]
        .as_array()
        .expect("output must carry a `results` array");
    assert!(
        result_slugs(results).contains("default::journal/2026-04-16"),
        "memory_search numeric alias expansion should match 75,000 content: {results:?}"
    );
}

// ── Regression: reverse-direction numeric aliases (issue #196 symmetry) ──────

/// Seed one page with grouped content (`75,000`) and one with the plain
/// numeral (`75000`); both must be reachable from abbreviated queries.
fn seed_reverse_numeric_corpus(conn: &Connection) {
    insert_page_with_namespace(
        conn,
        "",
        "journal/grouped",
        "journal",
        "Journal grouped",
        "Revenue update",
        "Revenue reached 75,000 in April.",
    );
    insert_page_with_namespace(
        conn,
        "",
        "journal/plain",
        "journal",
        "Journal plain",
        "Salary update",
        "Salary set to 75000 in April.",
    );
}

#[test]
fn search_abbreviated_numeric_query_matches_full_and_grouped_content() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "search-numeric-reverse.db");
    let conn = open_test_db(&db_path);
    seed_reverse_numeric_corpus(&conn);
    drop(conn);

    for query in ["75k April", "$75K April"] {
        let output = run_quaid(&db_path, &["--json", "search", query]);
        assert!(
            output.status.success(),
            "search should exit cleanly for abbreviated numeric query {query:?}: {output:?}"
        );

        let parsed = parse_stdout_json(&output);
        let results = parsed.as_array().expect("output must be a JSON array");
        let slugs = result_slugs(results);
        assert!(
            slugs.contains("default::journal/grouped") && slugs.contains("default::journal/plain"),
            "query {query:?} must match both 75,000 and 75000 content: {results:?}"
        );
    }
}

#[test]
fn memory_search_abbreviated_numeric_query_matches_full_and_grouped_content() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "memory-search-numeric-reverse.db");
    let conn = open_test_db(&db_path);
    seed_reverse_numeric_corpus(&conn);
    drop(conn);

    for payload in [
        r#"{"query":"75k April","limit":10}"#,
        r#"{"query":"$75K April","limit":10}"#,
    ] {
        let output = run_quaid(&db_path, &["call", "memory_search", payload]);
        assert!(
            output.status.success(),
            "memory_search should exit cleanly for abbreviated numeric payload {payload}: {output:?}"
        );

        let parsed = parse_stdout_json(&output);
        let results = parsed["results"]
            .as_array()
            .expect("output must be a JSON array");
        let slugs = result_slugs(results);
        assert!(
            slugs.contains("default::journal/grouped") && slugs.contains("default::journal/plain"),
            "memory_search payload {payload} must match both 75,000 and 75000 content: {results:?}"
        );
    }
}

/// Formatted query `$75,000` sanitizes to the two tokens `75 000`; the fused
/// single-token alias must reach content holding the plain numeral `75000`.
#[test]
fn search_formatted_numeric_query_matches_single_token_content() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "search-numeric-formatted.db");
    let conn = open_test_db(&db_path);

    insert_page_with_namespace(
        &conn,
        "",
        "journal/plain",
        "journal",
        "Journal plain",
        "Salary update",
        "Salary set to 75000 in April.",
    );
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "search", "$75,000"]);
    assert!(
        output.status.success(),
        "search should exit cleanly for formatted numeric query: {output:?}"
    );

    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert!(
        result_slugs(results).contains("default::journal/plain"),
        "formatted query $75,000 must match single-token 75000 content: {results:?}"
    );
}

/// One-decimal abbreviations must alias in both directions: plain-numeral
/// query ↔ `$1.5M` content and `$1.5M` query ↔ plain-numeral content.
#[test]
fn search_decimal_abbreviated_numeric_aliases_work_in_both_directions() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "search-numeric-decimal.db");
    let conn = open_test_db(&db_path);

    insert_page_with_namespace(
        &conn,
        "",
        "journal/decimal",
        "journal",
        "Journal decimal",
        "Raise update",
        "Raise closed at $1.5M in May.",
    );
    insert_page_with_namespace(
        &conn,
        "",
        "journal/numeral",
        "journal",
        "Journal numeral",
        "Round update",
        "Round closed at 1500000 in May.",
    );
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "search", "1500000"]);
    assert!(
        output.status.success(),
        "search should exit cleanly for plain-numeral query: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert!(
        result_slugs(results).contains("default::journal/decimal"),
        "query 1500000 must match $1.5M content: {results:?}"
    );

    let output = run_quaid(&db_path, &["--json", "search", "$1.5M"]);
    assert!(
        output.status.success(),
        "search should exit cleanly for decimal-abbreviated query: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    let results = parsed.as_array().expect("output must be a JSON array");
    assert!(
        result_slugs(results).contains("default::journal/numeral"),
        "query $1.5M must match 1500000 content: {results:?}"
    );
}

// ── memory_search tiered fallback (surface parity with CLI search) ───────────

/// A two-term memory_search query where only one term exists in the corpus
/// must return results via the tiered OR fallback instead of an empty array.
#[test]
fn memory_search_two_term_query_falls_back_to_or_when_and_misses() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "memory-search-or-fallback.db");
    let conn = open_test_db(&db_path);

    insert_page(
        &conn,
        "concepts/quantum",
        "concept",
        "Quantum Computing",
        "Quantum computing uses qubits for parallel computation.",
    );
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "call",
            "memory_search",
            r#"{"query":"quantum jellyfish","limit":10}"#,
        ],
    );
    assert!(
        output.status.success(),
        "memory_search should exit cleanly: {output:?}"
    );

    let parsed = parse_stdout_json(&output);
    let results = parsed["results"]
        .as_array()
        .expect("output must be a JSON array");
    assert_eq!(
        result_slugs(results),
        BTreeSet::from(["default::concepts/quantum".to_owned()]),
        "memory_search must widen to OR when only one of two terms matches: {results:?}"
    );
}
