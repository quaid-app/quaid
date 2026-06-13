#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Intra-document deduplication tests (`result-deduplication` capability,
//! openspec change `retrieval-quality-rerank` task 2.6).
//!
//! Scenarios tested:
//!   1. `dedup_chunks_per_page` collapse mechanics on synthetic candidate
//!      lists (three-row collapse, passthrough, count correctness,
//!      `max_per_page=2`, `0` = unlimited identity)
//!   2. the seeded `search.max_chunks_per_doc_default` identity default
//!   3. `--max-chunks-per-doc` CLI flag plumbing on `quaid search` and
//!      `quaid query`
//!   4. `progressive_retrieve` re-application of the dedup pass

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::path::{Path, PathBuf};
use std::process::Command;

use quaid::core::db;
use quaid::core::progressive::progressive_retrieve;
use quaid::core::search::dedup_chunks_per_page;
use quaid::core::types::SearchResult;
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

// ── 1. dedup_chunks_per_page mechanics ────────────────────────────────────────

#[test]
fn dedup_collapses_three_rows_of_same_page_to_strongest() {
    let candidates = vec![
        candidate("notes/long-page", 0.9),
        candidate("notes/long-page", 0.7),
        candidate("notes/long-page", 0.5),
    ];

    let deduped = dedup_chunks_per_page(candidates, 1);

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].slug, "notes/long-page");
    assert!((deduped[0].score - 0.9).abs() < 1e-12, "strongest row wins");
    assert_eq!(
        deduped[0].dedup_collapsed_count, 2,
        "two siblings collapsed into the representative"
    );
}

#[test]
fn dedup_passes_through_single_row_per_page_unchanged() {
    let candidates = vec![
        candidate("notes/alpha", 0.9),
        candidate("notes/beta", 0.7),
        candidate("notes/gamma", 0.5),
    ];

    let deduped = dedup_chunks_per_page(candidates.clone(), 1);

    assert_eq!(deduped.len(), 3);
    for (kept, original) in deduped.iter().zip(candidates.iter()) {
        assert_eq!(kept.slug, original.slug, "order preserved");
        assert_eq!(kept.dedup_collapsed_count, 0, "nothing collapsed");
    }
}

#[test]
fn dedup_collapsed_count_tracks_per_page_collapse_magnitude() {
    let candidates = vec![
        candidate("notes/alpha", 0.9),
        candidate("notes/beta", 0.8),
        candidate("notes/alpha", 0.6),
        candidate("notes/alpha", 0.4),
        candidate("notes/beta", 0.3),
    ];

    let deduped = dedup_chunks_per_page(candidates, 1);

    assert_eq!(deduped.len(), 2);
    let alpha = deduped.iter().find(|r| r.slug == "notes/alpha").unwrap();
    let beta = deduped.iter().find(|r| r.slug == "notes/beta").unwrap();
    assert_eq!(alpha.dedup_collapsed_count, 2);
    assert_eq!(beta.dedup_collapsed_count, 1);
}

#[test]
fn dedup_max_two_keeps_two_strongest_rows_per_page() {
    let candidates = vec![
        candidate("notes/alpha", 0.9),
        candidate("notes/alpha", 0.7),
        candidate("notes/alpha", 0.5),
    ];

    let deduped = dedup_chunks_per_page(candidates, 2);

    assert_eq!(deduped.len(), 2);
    assert!((deduped[0].score - 0.9).abs() < 1e-12);
    assert!((deduped[1].score - 0.7).abs() < 1e-12);
    assert_eq!(
        deduped[0].dedup_collapsed_count, 1,
        "the third row collapses into the strongest representative"
    );
    assert_eq!(deduped[1].dedup_collapsed_count, 0);
}

#[test]
fn dedup_zero_means_unlimited_identity() {
    let candidates = vec![
        candidate("notes/alpha", 0.9),
        candidate("notes/alpha", 0.7),
        candidate("notes/alpha", 0.5),
    ];

    let deduped = dedup_chunks_per_page(candidates.clone(), 0);

    assert_eq!(deduped.len(), candidates.len(), "0 = unlimited (identity)");
    assert!(deduped.iter().all(|r| r.dedup_collapsed_count == 0));
}

// ── 2. seeded identity default ────────────────────────────────────────────────

#[test]
fn schema_seeds_max_chunks_per_doc_identity_default() {
    let conn = db::open(":memory:").expect("open in-memory db");
    let seeded: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'search.max_chunks_per_doc_default'",
            [],
            |row| row.get(0),
        )
        .expect("seeded key");
    assert_eq!(seeded, "0", "identity default: unlimited");
}

// ── 3. CLI flag plumbing ──────────────────────────────────────────────────────

#[test]
fn search_cli_accepts_max_chunks_per_doc_flag() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "dedup-search-cli.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/widget", "Widget", "widget assembly notes");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["--json", "search", "widget", "--max-chunks-per-doc", "2"],
    );

    assert!(
        output.status.success(),
        "search must exit cleanly: {output:?}"
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    let results = parsed.as_array().expect("JSON array");
    assert_eq!(results.len(), 1, "page-level FTS hit survives the cap");
}

#[test]
fn query_cli_accepts_max_chunks_per_doc_flag() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "dedup-query-cli.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/widget", "Widget", "widget assembly notes");
    drop(conn);

    let output = run_quaid(
        &db_path,
        &[
            "--json",
            "query",
            "widget assembly",
            "--depth",
            "none",
            "--max-chunks-per-doc",
            "1",
        ],
    );

    assert!(
        output.status.success(),
        "query must exit cleanly: {output:?}"
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    assert_eq!(
        parsed.as_array().expect("JSON array").len(),
        1,
        "page-level hybrid hit survives the cap"
    );
}

// ── 4. progressive_retrieve re-application ────────────────────────────────────

#[test]
fn progressive_retrieve_reapplies_dedup_on_initial_set() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "dedup-progressive.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/alpha", "Alpha", &"x".repeat(100));
    conn.execute(
        "UPDATE config SET value = '1' WHERE key = 'search.max_chunks_per_doc_default'",
        [],
    )
    .expect("activate dedup");

    // Two rows for the same page in the initial set: dedup collapses them
    // before the budget walk.
    let initial = vec![candidate("notes/alpha", 0.9), candidate("notes/alpha", 0.6)];
    let results = progressive_retrieve(initial, 100_000, 0, None, false, &conn).unwrap();

    assert_eq!(results.len(), 1, "duplicate-page rows collapse to one");
    assert_eq!(results[0].dedup_collapsed_count, 1);
}

#[test]
fn progressive_retrieve_identity_default_keeps_duplicate_rows() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "dedup-progressive-identity.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/alpha", "Alpha", &"x".repeat(100));

    let initial = vec![candidate("notes/alpha", 0.9), candidate("notes/alpha", 0.6)];
    let results = progressive_retrieve(initial, 100_000, 0, None, false, &conn).unwrap();

    assert_eq!(
        results.len(),
        2,
        "seeded default (0 = unlimited) must not collapse rows"
    );
}
