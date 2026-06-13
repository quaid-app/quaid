#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Hybrid-search fusion integration tests.
//!
//! Scenarios tested:
//!   1. Vector-arm depth — `limit > 10` propagates into the vector arm's k
//!      instead of the historical hardcoded k=10
//!   2. Relevance floor — `search.relevance_floor` drops low-cosine vector
//!      hits before the merge; the seeded `0.0` default keeps identity
//!      behaviour

use std::collections::BTreeSet;

use quaid::commands::embed;
use quaid::core::db;
use quaid::core::search::{hybrid_search, HybridSearch};

fn open_test_db() -> rusqlite::Connection {
    db::open(":memory:").expect("open in-memory DB")
}

/// Deterministic RFC-4122-shaped uuid derived from the slug, mirroring the
/// fixture style used by the search unit tests.
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

fn insert_page(conn: &rusqlite::Connection, slug: &str, title: &str, truth: &str) {
    conn.execute(
        "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, ?2, 'concept', ?3, '', ?4, '', '{}', 'notes', '', 1)",
        rusqlite::params![slug, test_uuid(slug), title, truth],
    )
    .expect("insert page");
}

fn result_slugs(results: &[quaid::core::types::SearchResult]) -> BTreeSet<String> {
    results.iter().map(|result| result.slug.clone()).collect()
}

// ── 1. Vector-arm k propagation ───────────────────────────────────────────────

/// `limit > 10` must reach the vector arm: with 15 embedded pages, none of
/// which match the query lexically, the merged output is vector-arm-only and
/// must exceed the historical hardcoded k=10.
#[test]
fn hybrid_search_vector_arm_honours_limit_above_ten() {
    let conn = open_test_db();
    for index in 0..15 {
        insert_page(
            &conn,
            &format!("notes/entry-{index:02}"),
            &format!("Entry {index:02}"),
            &format!("Entry {index:02} covers granite ledger and willow maintenance routines."),
        );
    }
    embed::run(&conn, None, true, false).expect("embed pages");

    // No page contains any query token, so the FTS arm (AND and OR passes)
    // stays empty and every merged result comes from the vector arm.
    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "cosmic jellyfish parade",
            limit: 25,
            ..Default::default()
        },
    )
    .expect("hybrid search");

    assert!(
        results.len() > 10,
        "limit=25 must propagate into the vector arm instead of capping at k=10, got {} results",
        results.len()
    );
    assert_eq!(
        results.len(),
        15,
        "all embedded pages should surface through the vector arm at limit=25"
    );
}

// ── 2. Relevance floor on the vector arm ──────────────────────────────────────

fn seed_floor_corpus(conn: &rusqlite::Connection) {
    // The target page's truth is byte-identical to the query, so its chunk
    // embedding matches the query embedding at cosine ~1.0 under any
    // deterministic backend.
    insert_page(
        conn,
        "notes/target",
        "Target",
        "violet umbrella daydream symposium",
    );
    insert_page(
        conn,
        "notes/junk-1",
        "Junk One",
        "Quarterly ledger review for the harbor warehouse.",
    );
    insert_page(
        conn,
        "notes/junk-2",
        "Junk Two",
        "Granite trail erosion report from the northern ridge.",
    );
    insert_page(
        conn,
        "notes/junk-3",
        "Junk Three",
        "Recipe rotation notes for the winter pantry.",
    );
    embed::run(conn, None, true, false).expect("embed pages");
}

const FLOOR_QUERY: &str = "violet umbrella daydream symposium";

/// With the seeded default floor (`0.0`) the behaviour is identity: the
/// vector arm surfaces every page, junk included.
#[test]
fn relevance_floor_seeded_zero_keeps_low_cosine_vector_hits() {
    let conn = open_test_db();
    seed_floor_corpus(&conn);

    let seeded: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'search.relevance_floor'",
            [],
            |row| row.get(0),
        )
        .expect("relevance floor must be seeded");
    assert_eq!(seeded, "0.0", "schema must seed the identity floor");

    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: FLOOR_QUERY,
            limit: 10,
            ..Default::default()
        },
    )
    .expect("hybrid search");

    let slugs = result_slugs(&results);
    assert!(
        slugs.contains("notes/target"),
        "the exact-match page must always surface: {slugs:?}"
    );
    assert_eq!(
        results.len(),
        4,
        "identity floor must keep every vector-arm hit, junk included: {slugs:?}"
    );
}

/// A high floor drops the junk vector hits while the exact match (cosine
/// ~1.0, plus its FTS hit) survives.
#[test]
fn relevance_floor_drops_low_cosine_vector_hits() {
    let conn = open_test_db();
    seed_floor_corpus(&conn);

    conn.execute(
        "UPDATE config SET value = '0.995' WHERE key = 'search.relevance_floor'",
        [],
    )
    .expect("raise relevance floor");

    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: FLOOR_QUERY,
            limit: 10,
            ..Default::default()
        },
    )
    .expect("hybrid search");

    assert_eq!(
        result_slugs(&results),
        BTreeSet::from(["notes/target".to_owned()]),
        "a high floor must drop junk vector hits and keep the exact match"
    );
}
