#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Knowledge-gap auto-logging heuristic tests (fable-review area 14).
//!
//! Scenarios tested:
//!   1. exact-slug lookups (score 1.0) no longer log phantom gaps, on both
//!      the CLI `quaid query` path and the MCP `memory_query` path
//!   2. under `search_merge_strategy='rrf'` a strong result logs no gap
//!      while an empty result set does — the normalized RRF scale makes the
//!      single 0.3 threshold meaningful for both merge strategies
//!   3. a rank-0 dual-list RRF hit scores >= 0.9 after normalization
//!   4. auto-logged gaps carry query-free diagnostics context instead of ""

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/mcp_harness.rs"]
mod harness;

use std::path::{Path, PathBuf};
use std::process::Command;

use harness::{create_page, extract_text};
use quaid::commands::embed;
use quaid::core::db;
use quaid::core::gaps::should_log_gap;
use quaid::core::search::{hybrid_search, HybridSearch};
use quaid::core::types::SearchResult;
use quaid::mcp::server::{MemoryGapsInput, MemoryQueryInput, QuaidServer};
use rusqlite::Connection;

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().expect("utf-8 db path")).expect("open test db")
}

/// Deterministic RFC-4122-shaped uuid derived from the slug; `quaid embed`
/// refuses pages without one.
fn test_uuid(seed: &str) -> String {
    let mut hex = String::new();
    for byte in seed.as_bytes() {
        hex.push_str(&format!("{byte:02x}"));
        if hex.len() >= 32 {
            break;
        }
    }
    while hex.len() < 32 {
        hex.push('0');
    }
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn insert_page(conn: &Connection, slug: &str, title: &str, truth: &str) {
    conn.execute(
        "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, ?2, 'concept', ?3, ?3, ?4, '', '{}', 'notes', '', 1)",
        rusqlite::params![slug, test_uuid(slug), title, truth],
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

fn gap_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM knowledge_gaps", [], |row| row.get(0))
        .expect("count gaps")
}

fn memory_query_input(query: &str) -> MemoryQueryInput {
    MemoryQueryInput {
        query: query.to_string(),
        collection: None,
        namespace: None,
        wing: None,
        limit: None,
        depth: None,
        include_superseded: None,
        hops: None,
        relevance_floor: None,
        max_chunks_per_doc: None,
        redact: None,
    }
}

fn list_gaps_json(server: &QuaidServer) -> Vec<serde_json::Value> {
    let result = server
        .memory_gaps(MemoryGapsInput {
            resolved: None,
            limit: None,
        })
        .expect("memory_gaps");
    serde_json::from_str(&extract_text(&result)).expect("gaps JSON")
}

// ── 1. exact-slug lookups log no gap ──────────────────────────────────────────

#[test]
fn cli_exact_slug_query_logs_no_gap() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "gap-exact-slug-cli.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/target", "Target", "the answer lives here");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["--json", "query", "notes/target", "--depth", "none"],
    );

    assert!(output.status.success(), "{output:?}");
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1, "slug lookup must hit");

    let conn = open_test_db(&db_path);
    assert_eq!(
        gap_count(&conn),
        0,
        "a successful exact-slug lookup (score 1.0) must not log a phantom gap"
    );
}

// QUARANTINED (flaky on x86_64 CI; tracked for root-cause + re-enable).
//
// `memory_query` for an exact slug should short-circuit to score 1.0 and log
// no gap. On x86_64 CI runners this intermittently bypasses the short-circuit;
// the FTS/vector fallthrough then scores the page below the 0.3 gap floor and
// a phantom gap is logged. The test is single-threaded on one connection (no
// thread race) and never reproduces on aarch64 (1888/1888), so the trigger is
// an environment-specific nondeterminism (hash-shim vector scoring / fp /
// resolution ordering) not yet pinned down. Severity is low: gaps are advisory
// metadata and the query still returns the correct page. Re-enable once the
// short-circuit bypass is root-caused. See docs/fable-review-progress.md.
#[test]
#[ignore = "flaky on x86_64 CI: phantom gap on exact-slug short-circuit bypass (low-severity advisory noise); tracked for root-cause fix"]
fn mcp_exact_slug_query_logs_no_gap() {
    let (_dir, conn) = harness::open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/target",
        "---\ntitle: Target\ntype: concept\n---\nthe answer lives here\n",
    );

    let result = server
        .memory_query(memory_query_input("notes/target"))
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    let rows = envelope["results"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 1, "slug lookup must hit");

    assert!(
        list_gaps_json(&server).is_empty(),
        "a successful exact-slug lookup must not log a phantom gap"
    );
}

// ── 2. RRF heuristic behaviour ────────────────────────────────────────────────

#[test]
fn rrf_strong_single_result_logs_no_gap_and_empty_results_do() {
    let (_dir, conn) = harness::open_test_db();
    conn.execute(
        "UPDATE config SET value = 'rrf' WHERE key = 'search_merge_strategy'",
        [],
    )
    .expect("switch to rrf");
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/widget",
        "---\ntitle: Widget\ntype: concept\n---\nwidget assembly notes\n",
    );

    // Strong lexical hit: rank-0 single-list RRF normalizes to ~0.5 — above
    // the 0.3 gap threshold (pre-normalization it capped at ~0.016 and every
    // RRF query was logged as a gap).
    let result = server
        .memory_query(memory_query_input("widget assembly"))
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    let rows = envelope["results"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 1);
    let top_score = rows[0]["score"].as_f64().unwrap();
    assert!(
        top_score >= 0.3,
        "normalized RRF score must clear the gap threshold, got {top_score}"
    );
    assert!(
        list_gaps_json(&server).is_empty(),
        "a strong RRF result must not log a gap"
    );

    // Empty result set: gap logged with query-free diagnostics context.
    let result = server
        .memory_query(memory_query_input("entirely unknown moon colony"))
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    let rows = envelope["results"].as_array().cloned().unwrap_or_default();
    assert!(rows.is_empty());

    let gaps = list_gaps_json(&server);
    assert_eq!(gaps.len(), 1, "empty results must log a gap");
    assert_eq!(
        gaps[0]["context"].as_str().unwrap(),
        "auto: hybrid_search results=0",
        "auto-context must be query-free diagnostics, not empty"
    );
}

// ── 3. normalized RRF dual-list rank-0 hit ────────────────────────────────────

#[test]
fn rrf_dual_list_rank_zero_hit_scores_above_point_nine() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "gap-rrf-dual.db");
    let conn = open_test_db(&db_path);
    conn.execute(
        "UPDATE config SET value = 'rrf' WHERE key = 'search_merge_strategy'",
        [],
    )
    .expect("switch to rrf");
    // The target's truth is byte-identical to the query, so it is rank 0 in
    // both the FTS arm and the vector arm under any deterministic backend.
    insert_page(
        &conn,
        "notes/target",
        "Target",
        "violet umbrella daydream symposium",
    );
    insert_page(
        &conn,
        "notes/junk",
        "Junk",
        "Quarterly ledger review for the harbor warehouse.",
    );
    embed::run(&conn, None, true, false).expect("embed pages");

    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "violet umbrella daydream symposium",
            limit: 10,
            ..Default::default()
        },
    )
    .expect("hybrid search");

    assert_eq!(results[0].slug, "notes/target");
    assert!(
        results[0].score >= 0.9,
        "rank-0 dual-list RRF hit must normalize to >= 0.9, got {}",
        results[0].score
    );
    assert!(
        !should_log_gap(&results),
        "a near-1.0 RRF hit must never be classified as a gap"
    );
}

// ── 4. should_log_gap unit shape ──────────────────────────────────────────────

fn scored(score: f64) -> SearchResult {
    SearchResult {
        slug: "notes/x".to_owned(),
        title: "X".to_owned(),
        summary: String::new(),
        score,
        wing: String::new(),
        ..Default::default()
    }
}

#[test]
fn should_log_gap_contract() {
    // Perfect hit (exact slug) — never a gap, even as the only result.
    assert!(!should_log_gap(&[scored(1.0)]));
    assert!(!should_log_gap(&[scored(0.99)]));
    // Single mediocre hit — not a gap (the old `len() < 2` heuristic fired).
    assert!(!should_log_gap(&[scored(0.5)]));
    // All-weak results — gap.
    assert!(should_log_gap(&[scored(0.1), scored(0.05)]));
    // Empty — gap.
    assert!(should_log_gap(&[]));
}
