#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Cross-reference boost tests (`cross-reference-scoring` capability,
//! openspec change `retrieval-quality-rerank` task 4.7).
//!
//! Scenarios tested:
//!   1. Co-cited candidate receives the weighted sum of incoming edge weights
//!   2. Candidate with no incoming edges from the working set gets no boost
//!   3. Expired edges (`valid_until` in the past) do not contribute
//!   4. Hub-page boost saturates at `cross_ref_boost_cap`
//!   5. `weight == 0.0` short-circuits the lookup entirely (identity no-op)
//!   6. Empty graph is a graceful no-op
//!
//! `compute_cross_ref_boost` is exercised directly so the tests do not depend
//! on the embedding model.

use quaid::core::db;
use quaid::core::search::compute_cross_ref_boost;
use quaid::core::types::SearchResult;
use rusqlite::Connection;

fn open_db() -> Connection {
    db::open(":memory:").expect("open in-memory db")
}

fn insert_page(conn: &Connection, slug: &str) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, 'concept', ?1, ?1, '', '', '{}', 'notes', '', 1)",
        rusqlite::params![slug],
    )
    .expect("insert page");
}

fn page_id(conn: &Connection, slug: &str) -> i64 {
    conn.query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })
    .expect("page id")
}

fn insert_edge(conn: &Connection, from: &str, to: &str, weight: f64, valid_until: Option<&str>) {
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind, edge_weight, valid_until) \
         VALUES (?1, ?2, 'related', 'programmatic', ?3, ?4)",
        rusqlite::params![page_id(conn, from), page_id(conn, to), weight, valid_until],
    )
    .expect("insert edge");
}

fn candidate(slug: &str, score: f64) -> SearchResult {
    SearchResult {
        slug: slug.to_owned(),
        title: slug.to_owned(),
        summary: slug.to_owned(),
        score,
        wing: "notes".to_owned(),
        ..Default::default()
    }
}

fn find<'a>(results: &'a [SearchResult], slug: &str) -> &'a SearchResult {
    results
        .iter()
        .find(|result| result.slug == slug)
        .unwrap_or_else(|| panic!("missing candidate {slug}"))
}

#[test]
fn co_cited_candidate_receives_weighted_edge_sum() {
    let conn = open_db();
    for slug in ["alice", "brex", "yc-w17"] {
        insert_page(&conn, slug);
    }
    insert_edge(&conn, "alice", "brex", 1.0, None);
    insert_edge(&conn, "yc-w17", "brex", 0.5, None);

    let candidates = vec![
        candidate("alice", 0.9),
        candidate("brex", 0.4),
        candidate("yc-w17", 0.3),
    ];

    let boosted = compute_cross_ref_boost(&conn, candidates, 0.05, 0.15).expect("boost");

    // boost(brex) = 0.05 * (1.0 + 0.5) = 0.075
    let brex = find(&boosted, "brex");
    assert!(
        (f64::from(brex.cross_ref_boost) - 0.075).abs() < 1e-6,
        "brex cross_ref_boost should be 0.075, got {}",
        brex.cross_ref_boost
    );
}

#[test]
fn candidate_without_incoming_edges_gets_no_boost() {
    let conn = open_db();
    for slug in ["alice", "brex"] {
        insert_page(&conn, slug);
    }
    insert_edge(&conn, "alice", "brex", 1.0, None);

    let candidates = vec![candidate("alice", 0.9), candidate("brex", 0.4)];
    let boosted = compute_cross_ref_boost(&conn, candidates, 0.05, 0.15).expect("boost");

    let alice = find(&boosted, "alice");
    assert_eq!(
        alice.cross_ref_boost, 0.0,
        "alice has no incoming edge from the working set"
    );
    assert!(
        (alice.score - 0.9).abs() < 1e-9,
        "alice fused score unchanged"
    );
}

#[test]
fn expired_edges_do_not_contribute() {
    let conn = open_db();
    for slug in ["alice", "brex"] {
        insert_page(&conn, slug);
    }
    // Edge expired in the past → excluded by the temporal filter.
    insert_edge(&conn, "alice", "brex", 1.0, Some("2020-01-01"));

    let candidates = vec![candidate("alice", 0.9), candidate("brex", 0.4)];
    let boosted = compute_cross_ref_boost(&conn, candidates, 0.05, 0.15).expect("boost");

    let brex = find(&boosted, "brex");
    assert_eq!(
        brex.cross_ref_boost, 0.0,
        "expired edge must not contribute a boost"
    );
}

#[test]
fn hub_page_boost_saturates_at_cap() {
    let conn = open_db();
    insert_page(&conn, "hub");
    let mut candidates = vec![candidate("hub", 0.5)];
    // Ten distinct sources each linking to hub with weight 1.0 → uncapped
    // boost = 0.05 * 10 = 0.50, capped to 0.15.
    for index in 0..10 {
        let src = format!("src-{index}");
        insert_page(&conn, &src);
        insert_edge(&conn, &src, "hub", 1.0, None);
        candidates.push(candidate(&src, 0.1));
    }

    let boosted = compute_cross_ref_boost(&conn, candidates, 0.05, 0.15).expect("boost");

    let hub = find(&boosted, "hub");
    assert!(
        (f64::from(hub.cross_ref_boost) - 0.15).abs() < 1e-6,
        "hub boost must saturate at the cap 0.15, got {}",
        hub.cross_ref_boost
    );
}

#[test]
fn weight_zero_short_circuits_and_applies_no_boost() {
    let conn = open_db();
    for slug in ["alice", "brex"] {
        insert_page(&conn, slug);
    }
    insert_edge(&conn, "alice", "brex", 1.0, None);

    let candidates = vec![candidate("alice", 0.9), candidate("brex", 0.4)];
    let boosted = compute_cross_ref_boost(&conn, candidates, 0.0, 0.15).expect("boost");

    assert!(
        boosted.iter().all(|result| result.cross_ref_boost == 0.0),
        "weight 0.0 must leave every cross_ref_boost at 0.0"
    );
    // Scores untouched and order preserved (identity no-op).
    assert_eq!(boosted[0].slug, "alice");
    assert!((boosted[0].score - 0.9).abs() < 1e-9);
}

#[test]
fn empty_graph_is_graceful_no_op() {
    let conn = open_db();
    for slug in ["alice", "brex"] {
        insert_page(&conn, slug);
    }
    // No edges inserted.
    let candidates = vec![candidate("alice", 0.9), candidate("brex", 0.4)];
    let boosted = compute_cross_ref_boost(&conn, candidates, 0.05, 0.15).expect("boost");

    assert!(
        boosted.iter().all(|result| result.cross_ref_boost == 0.0),
        "empty graph must produce zero boosts without error"
    );
}
