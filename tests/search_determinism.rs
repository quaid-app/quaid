#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Determinism test for the reranked retrieval pipeline (openspec change
//! `retrieval-quality-rerank` task 7.1).
//!
//! Running the same query twice against an unchanged database with the same
//! (non-identity) config must return element-for-element identical
//! `SearchResult` lists, including `mmr_score`, `cross_ref_boost`, and
//! `dedup_collapsed_count`. The reranking passes must not depend on hash-map
//! iteration order.

use quaid::commands::embed;
use quaid::core::db;
use quaid::core::search::{hybrid_search, HybridSearch};
use rusqlite::Connection;

fn open_db() -> Connection {
    db::open(":memory:").expect("open in-memory db")
}

fn set_config(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO config (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .expect("set config");
}

fn insert_page(conn: &Connection, slug: &str, title: &str, truth: &str) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, 'concept', ?2, ?2, ?3, '', '{}', 'concepts', '', 1)",
        rusqlite::params![slug, title, truth],
    )
    .expect("insert page");
}

#[test]
fn identical_query_twice_returns_identical_results() {
    let conn = open_db();
    // Non-identity config across all four signals to exercise every pass.
    set_config(&conn, "search.mmr_lambda", "0.7");
    set_config(&conn, "search.relevance_floor", "0.0");
    set_config(&conn, "search.max_chunks_per_doc_default", "1");
    set_config(&conn, "search.cross_ref_boost_weight", "0.05");
    set_config(&conn, "search.cross_ref_boost_cap", "0.15");

    insert_page(
        &conn,
        "concepts/agents",
        "AI Agents",
        "AI agents coordinate tools and memory for autonomous workflows.",
    );
    insert_page(
        &conn,
        "concepts/memory",
        "Agent Memory",
        "Memory systems give AI agents persistent recall across sessions.",
    );
    insert_page(
        &conn,
        "concepts/tools",
        "Tool Use",
        "Tool use lets AI agents act on external systems and data.",
    );
    embed::run(&conn, None, true, false).expect("embed pages");

    let run_once = || {
        hybrid_search(
            &conn,
            HybridSearch {
                query: "AI agents memory",
                limit: 10,
                ..Default::default()
            },
        )
        .expect("hybrid search")
    };

    let first = run_once();
    let second = run_once();

    // Compare the full serialized shape so every field (including mmr_score,
    // cross_ref_boost, dedup_collapsed_count) participates in the assertion.
    let first_json = serde_json::to_string(&first).expect("serialize first");
    let second_json = serde_json::to_string(&second).expect("serialize second");
    assert_eq!(
        first_json, second_json,
        "two identical queries must return byte-identical SearchResult lists"
    );
    assert!(!first.is_empty(), "the query should return results");
}
