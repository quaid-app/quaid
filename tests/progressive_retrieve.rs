#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would generate noise across thousands of test sites"
)]

//! `progressive_retrieve` integration tests for the four ranking signals
//! (openspec change `retrieval-quality-rerank` tasks 7.3 and 7.4).
//!
//! Scenarios tested:
//!   1. Below-floor initial candidates are dropped and never expanded
//!   2. Per-page dedup applies to the initial set
//!   3. MMR runs once on the initial set (reorders the top-level candidates)
//!   4. Cross-reference boost on the initial set can rescue a candidate above
//!      the floor
//!
//! The pass order on the initial set mirrors `hybrid_search`
//! (dedup → boost → floor → MMR); expansion steps re-apply dedup + floor only.

use quaid::core::db;
use quaid::core::inference::embedding_to_blob;
use quaid::core::progressive::progressive_retrieve;
use quaid::core::types::SearchResult;
use rusqlite::Connection;

const DIM: usize = 384;

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

fn unit_vec(angle_deg: f64) -> Vec<f32> {
    let radians = angle_deg.to_radians();
    let mut v = vec![0.0f32; DIM];
    v[0] = radians.cos() as f32;
    v[1] = radians.sin() as f32;
    v
}

fn insert_page(conn: &Connection, slug: &str, truth: &str, embedding: Option<&[f32]>) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, 'concept', ?1, ?1, ?2, '', '{}', 'notes', '', 1)",
        rusqlite::params![slug, truth],
    )
    .expect("insert page");
    let page_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .expect("page id");
    if let Some(vector) = embedding {
        conn.execute(
            "INSERT INTO page_embeddings_vec_384(rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![page_id, embedding_to_blob(vector)],
        )
        .expect("insert vec row");
        conn.execute(
            "INSERT INTO page_embeddings \
                 (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, \
                  content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', ?1, 'truth_section', 0, ?2, 'hash', 2, '')",
            rusqlite::params![page_id, truth],
        )
        .expect("insert embedding metadata");
    }
}

fn insert_link(conn: &Connection, from: &str, to: &str, weight: f64) {
    let from_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [from], |row| {
            row.get(0)
        })
        .expect("from id");
    let to_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [to], |row| {
            row.get(0)
        })
        .expect("to id");
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind, edge_weight) \
         VALUES (?1, ?2, 'related', 'programmatic', ?3)",
        rusqlite::params![from_id, to_id, weight],
    )
    .expect("insert link");
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

fn slugs(results: &[SearchResult]) -> Vec<&str> {
    results.iter().map(|result| result.slug.as_str()).collect()
}

#[test]
fn below_floor_initial_candidate_is_not_expanded() {
    let conn = open_db();
    set_config(&conn, "search.relevance_floor", "0.3");
    // "noise" is below the floor; it links to "secret" which would otherwise
    // be pulled in by expansion. The floor must drop "noise" before expansion.
    insert_page(&conn, "noise", &"x".repeat(40), None);
    insert_page(&conn, "secret", &"y".repeat(40), None);
    insert_link(&conn, "noise", "secret", 1.0);

    let initial = vec![candidate("noise", 0.10)];
    let results = progressive_retrieve(initial, 100_000, 2, None, false, &conn).expect("retrieve");

    assert!(
        results.is_empty(),
        "below-floor candidate must be dropped and its links never expanded, got {:?}",
        slugs(&results)
    );
}

#[test]
fn above_floor_initial_candidate_expands_normally() {
    let conn = open_db();
    // Floor at the seeded identity default (0.0): the above-floor seed is kept
    // and expansion proceeds. (Graph neighbours carry score 0.0, so any
    // positive floor would correctly filter them — see the companion
    // `below_floor_initial_candidate_is_not_expanded` test.)
    insert_page(&conn, "seed", &"x".repeat(40), None);
    insert_page(&conn, "neighbour", &"y".repeat(40), None);
    insert_link(&conn, "seed", "neighbour", 1.0);

    let initial = vec![candidate("seed", 0.9)];
    let results = progressive_retrieve(initial, 100_000, 1, None, false, &conn).expect("retrieve");

    let got = slugs(&results);
    assert!(got.contains(&"seed"), "seed kept: {got:?}");
    assert!(got.contains(&"neighbour"), "neighbour expanded: {got:?}");
}

#[test]
fn dedup_collapses_initial_same_page_rows() {
    let conn = open_db();
    set_config(&conn, "search.max_chunks_per_doc_default", "1");
    insert_page(&conn, "dup", &"x".repeat(40), None);

    // Two rows for the same page in the initial set collapse to one.
    let initial = vec![candidate("dup", 0.9), candidate("dup", 0.7)];
    let results = progressive_retrieve(initial, 100_000, 0, None, false, &conn).expect("retrieve");

    assert_eq!(slugs(&results), vec!["dup"]);
    assert_eq!(
        results[0].dedup_collapsed_count, 1,
        "one sibling row collapsed into the representative"
    );
}

#[test]
fn mmr_reorders_initial_set_once() {
    let conn = open_db();
    set_config(&conn, "search.mmr_lambda", "0.7");
    // c1 at 0°, c2 near-duplicate of c1 (~18°), c3 diverse (90°).
    insert_page(&conn, "c1", &"x".repeat(40), Some(&unit_vec(0.0)));
    insert_page(&conn, "c2", &"y".repeat(40), Some(&unit_vec(18.19)));
    insert_page(&conn, "c3", &"z".repeat(40), Some(&unit_vec(90.0)));

    let initial = vec![
        candidate("c1", 0.80),
        candidate("c2", 0.79),
        candidate("c3", 0.60),
    ];
    // depth 0 so only the initial-set passes run (MMR included).
    let results = progressive_retrieve(initial, 100_000, 0, None, false, &conn).expect("retrieve");

    assert_eq!(
        slugs(&results),
        vec!["c1", "c3", "c2"],
        "MMR should diversify the initial set: c3 ahead of the near-duplicate c2"
    );
}

#[test]
fn cross_ref_boost_rescues_initial_candidate_above_floor() {
    let conn = open_db();
    set_config(&conn, "search.relevance_floor", "0.30");
    set_config(&conn, "search.cross_ref_boost_weight", "0.05");
    set_config(&conn, "search.cross_ref_boost_cap", "0.15");
    insert_page(&conn, "anchor", &"x".repeat(40), None);
    insert_page(&conn, "rescued", &"y".repeat(40), None);
    // anchor → rescued with a large edge weight: boost = 0.05 * 5.0 = 0.25,
    // capped to 0.15 → rescued score 0.28 + 0.15 = 0.43 ≥ floor.
    insert_link(&conn, "anchor", "rescued", 5.0);

    let initial = vec![candidate("anchor", 0.9), candidate("rescued", 0.28)];
    let results = progressive_retrieve(initial, 100_000, 0, None, false, &conn).expect("retrieve");

    let got = slugs(&results);
    assert!(
        got.contains(&"rescued"),
        "cross-ref boost should lift rescued above the 0.30 floor: {got:?}"
    );
}
