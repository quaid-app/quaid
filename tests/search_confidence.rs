#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Confidence-threshold filter tests (`confidence-thresholding` capability,
//! openspec change `retrieval-quality-rerank` task 3.7).
//!
//! Scenarios tested:
//!   1. `filter_below_floor` mechanics: below dropped, at/above kept,
//!      `0.0` disables (identity)
//!   2. post-fusion floor in `hybrid_search`: config-driven, per-call
//!      override, empty-result success path (fewer-than-k contract)
//!   3. `--relevance-floor` CLI flag including the `0.0` disable escape
//!      hatch and out-of-range rejection
//!   4. MCP `memory_query` parameter override and `[0,1]` validation
//!   5. `progressive_retrieve` floor application (below-floor candidates
//!      are not expanded)

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/mcp_harness.rs"]
mod harness;

use std::path::{Path, PathBuf};
use std::process::Command;

use harness::{create_page, extract_text};
use quaid::core::db;
use quaid::core::progressive::progressive_retrieve;
use quaid::core::search::{filter_below_floor, hybrid_search, HybridSearch};
use quaid::core::types::SearchResult;
use quaid::mcp::server::{MemoryQueryInput, QuaidServer};
use rusqlite::Connection;

fn candidate(slug: &str, score: f64) -> SearchResult {
    SearchResult {
        slug: slug.to_owned(),
        title: slug.to_owned(),
        summary: format!("summary for {slug}"),
        score,
        wing: "notes".to_owned(),
        ..Default::default()
    }
}

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().expect("utf-8 db path")).expect("open test db")
}

fn insert_page(conn: &Connection, slug: &str, title: &str, truth: &str) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, 'concept', ?2, ?2, ?3, '', '{}', 'notes', '', 1)",
        rusqlite::params![slug, title, truth],
    )
    .expect("insert page");
}

fn run_quaid(db_path: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command.arg("--db").arg(db_path).args(args);
    command.output().expect("run quaid")
}

fn test_db_path(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

// ── 1. filter_below_floor mechanics ───────────────────────────────────────────

#[test]
fn floor_drops_below_keeps_at_and_above() {
    let candidates = vec![
        candidate("notes/above", 0.8),
        candidate("notes/at", 0.5),
        candidate("notes/below", 0.49),
    ];

    let filtered = filter_below_floor(candidates, 0.5);

    let slugs: Vec<&str> = filtered.iter().map(|r| r.slug.as_str()).collect();
    assert_eq!(
        slugs,
        vec!["notes/above", "notes/at"],
        "below-floor dropped; at-floor and above kept"
    );
}

#[test]
fn floor_zero_disables_filtering_identity() {
    let candidates = vec![
        candidate("notes/strong", 0.9),
        candidate("notes/weak", 0.01),
        candidate("notes/zero", 0.0),
    ];

    let filtered = filter_below_floor(candidates.clone(), 0.0);

    assert_eq!(filtered.len(), candidates.len(), "0.0 disables the floor");
}

// ── 2. hybrid_search post-fusion floor ────────────────────────────────────────

/// A lexical-only hit normalizes to 0.4 under set-union (single-arm FTS
/// weight); a floor above that drops it even though it leaves zero results.
#[test]
fn hybrid_floor_config_drops_weak_fused_hit_and_underfills() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "confidence-hybrid.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/widget", "Widget", "widget assembly notes");

    let baseline = hybrid_search(
        &conn,
        HybridSearch {
            query: "widget assembly",
            limit: 10,
            ..Default::default()
        },
    )
    .expect("baseline hybrid search");
    assert_eq!(baseline.len(), 1, "FTS-only hit surfaces at identity floor");
    assert!(
        (baseline[0].score - 0.4).abs() < 1e-9,
        "single-arm FTS hit normalizes to 0.4, got {}",
        baseline[0].score
    );

    conn.execute(
        "UPDATE config SET value = '0.5' WHERE key = 'search.relevance_floor'",
        [],
    )
    .expect("raise floor");

    let floored = hybrid_search(
        &conn,
        HybridSearch {
            query: "widget assembly",
            limit: 10,
            ..Default::default()
        },
    )
    .expect("floored hybrid search must succeed (no error, no padding)");
    assert!(
        floored.is_empty(),
        "below-floor hit must be dropped even when it under-fills"
    );
}

#[test]
fn hybrid_floor_per_call_override_beats_config() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "confidence-override.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/widget", "Widget", "widget assembly notes");
    conn.execute(
        "UPDATE config SET value = '0.5' WHERE key = 'search.relevance_floor'",
        [],
    )
    .expect("raise config floor");

    // Per-call 0.0 disables the config floor; the 0.4 fused hit returns.
    let disabled = hybrid_search(
        &conn,
        HybridSearch {
            query: "widget assembly",
            limit: 10,
            relevance_floor: Some(0.0),
            ..Default::default()
        },
    )
    .expect("override hybrid search");
    assert_eq!(disabled.len(), 1, "0.0 override disables the config floor");

    // Per-call high floor drops the hit even when the config floor is 0.0.
    conn.execute(
        "UPDATE config SET value = '0.0' WHERE key = 'search.relevance_floor'",
        [],
    )
    .expect("reset config floor");
    let raised = hybrid_search(
        &conn,
        HybridSearch {
            query: "widget assembly",
            limit: 10,
            relevance_floor: Some(0.5),
            ..Default::default()
        },
    )
    .expect("override hybrid search");
    assert!(raised.is_empty(), "per-call floor drops the 0.4 fused hit");
}

// ── 3. CLI flag ───────────────────────────────────────────────────────────────

#[test]
fn query_cli_relevance_floor_zero_is_documented_escape_hatch() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "confidence-cli.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/widget", "Widget", "widget assembly notes");
    conn.execute(
        "UPDATE config SET value = '0.5' WHERE key = 'search.relevance_floor'",
        [],
    )
    .expect("raise config floor");
    drop(conn);

    let floored = run_quaid(
        &db_path,
        &["--json", "query", "widget assembly", "--depth", "none"],
    );
    assert!(floored.status.success(), "{floored:?}");
    let floored_json: serde_json::Value = serde_json::from_slice(&floored.stdout).unwrap();
    assert_eq!(
        floored_json.as_array().unwrap().len(),
        0,
        "config floor drops the weak hit"
    );

    let disabled = run_quaid(
        &db_path,
        &[
            "--json",
            "query",
            "widget assembly",
            "--depth",
            "none",
            "--relevance-floor",
            "0.0",
        ],
    );
    assert!(disabled.status.success(), "{disabled:?}");
    let disabled_json: serde_json::Value = serde_json::from_slice(&disabled.stdout).unwrap();
    assert_eq!(
        disabled_json.as_array().unwrap().len(),
        1,
        "--relevance-floor 0.0 must disable the config floor"
    );
}

#[test]
fn cli_relevance_floor_rejects_out_of_range_values() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "confidence-cli-range.db");
    let _conn = open_test_db(&db_path);

    let output = run_quaid(&db_path, &["query", "anything", "--relevance-floor", "1.5"]);

    assert!(
        !output.status.success(),
        "out-of-range floor must be rejected at parse time"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("0.0") && stderr.contains("1.0"),
        "error should state the valid range: {stderr}"
    );
}

// ── 4. MCP parameter override ─────────────────────────────────────────────────

fn memory_query_input(query: &str, relevance_floor: Option<f64>) -> MemoryQueryInput {
    MemoryQueryInput {
        query: query.to_string(),
        collection: None,
        namespace: None,
        wing: None,
        limit: None,
        depth: None,
        include_superseded: None,
        hops: None,
        relevance_floor,
        max_chunks_per_doc: None,
        redact: None,
    }
}

#[test]
fn memory_query_relevance_floor_overrides_config() {
    let (_dir, conn) = harness::open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/widget",
        "---\ntitle: Widget\ntype: concept\n---\nwidget assembly notes\n",
    );

    let unfloored = server
        .memory_query(memory_query_input("widget assembly", None))
        .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&unfloored)).unwrap();
    assert_eq!(rows.len(), 1, "identity default returns the weak hit");

    let floored = server
        .memory_query(memory_query_input("widget assembly", Some(0.5)))
        .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&floored)).unwrap();
    assert!(
        rows.is_empty(),
        "per-call floor must drop the 0.4 fused hit (success, no padding)"
    );
}

#[test]
fn memory_query_rejects_out_of_range_relevance_floor() {
    let (_dir, conn) = harness::open_test_db();
    let server = QuaidServer::new(conn);

    let err = server
        .memory_query(memory_query_input("anything", Some(1.5)))
        .expect_err("floor > 1.0 must be rejected");
    assert!(
        err.message.contains("relevance_floor"),
        "error should name the parameter: {}",
        err.message
    );
}

// ── 5. progressive_retrieve floor ─────────────────────────────────────────────

#[test]
fn progressive_retrieve_does_not_expand_below_floor_candidates() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "confidence-progressive.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/weak", "Weak", &"x".repeat(100));
    insert_page(&conn, "notes/neighbour", "Neighbour", &"y".repeat(100));
    let weak_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = 'notes/weak'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let neighbour_id: i64 = conn
        .query_row(
            "SELECT id FROM pages WHERE slug = 'notes/neighbour'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind) \
         VALUES (?1, ?2, 'related', 'programmatic')",
        rusqlite::params![weak_id, neighbour_id],
    )
    .unwrap();

    // Identity default: the weak candidate survives and is expanded.
    let initial = vec![candidate("notes/weak", 0.1)];
    let expanded = progressive_retrieve(initial.clone(), 100_000, 1, None, false, &conn).unwrap();
    assert!(
        expanded.iter().any(|r| r.slug == "notes/neighbour"),
        "identity floor expands the weak candidate"
    );

    // With an active floor the candidate is dropped before expansion.
    conn.execute(
        "UPDATE config SET value = '0.5' WHERE key = 'search.relevance_floor'",
        [],
    )
    .expect("raise floor");
    let filtered = progressive_retrieve(initial, 100_000, 1, None, false, &conn).unwrap();
    assert!(
        filtered.is_empty(),
        "below-floor candidates must not be expanded"
    );
}
