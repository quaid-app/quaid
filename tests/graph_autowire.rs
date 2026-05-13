#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests legitimately panic on setup failure"
)]

//! Task 11.2 — spec coverage for `specs/frontmatter-link-autowiring/spec.md`.
//!
//! This file pins the end-to-end write-path behavior of the autowiring
//! contract against the spec scenarios. Lower-level parse-only and
//! sync-primitive coverage already lives in
//! `tests/frontmatter_edge_expansion.rs` and `tests/links_derived_edge_sync.rs`;
//! the write-path persistence is exercised in `tests/wave3_graph_write_paths.rs`.
//! These tests close the loop by exercising one full scenario per spec
//! requirement via the `put` write path so the autowiring contract has a
//! single, scenario-named home.

#[path = "common/put_fixtures.rs"]
mod put_fixtures;

use put_fixtures::{open_test_db, page_id_for_slug};
use quaid::commands::put::put_from_string;
use rusqlite::Connection;

fn seed_target(conn: &Connection, slug: &str) {
    put_from_string(
        conn,
        slug,
        "---\ntitle: target\ntype: concept\n---\nstub\n",
        None,
    )
    .unwrap();
}

fn link_row(
    conn: &Connection,
    from_slug: &str,
    to_slug: &str,
    relationship: &str,
    source_kind: &str,
) -> Option<(f64, Option<String>, Option<String>)> {
    let from_id = page_id_for_slug(conn, from_slug);
    let to_id = page_id_for_slug(conn, to_slug);
    conn.query_row(
        "SELECT edge_weight, valid_from, valid_until FROM links \
         WHERE from_page_id = ?1 AND to_page_id = ?2 AND relationship = ?3 AND source_kind = ?4",
        rusqlite::params![from_id, to_id, relationship, source_kind],
        |row| {
            Ok((
                row.get::<_, f64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )
    .ok()
}

fn count_links(conn: &Connection, from_slug: &str, source_kind: &str) -> i64 {
    let from_id = page_id_for_slug(conn, from_slug);
    conn.query_row(
        "SELECT COUNT(*) FROM links WHERE from_page_id = ?1 AND source_kind = ?2",
        rusqlite::params![from_id, source_kind],
        |row| row.get(0),
    )
    .unwrap()
}

// ── Requirement: Frontmatter `links` array produces typed graph edges ───

#[test]
fn scenario_object_link_creates_typed_edge_with_temporal_validity() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    let md = "---\ntitle: Alice\ntype: person\nlinks:\n  - target: companies/brex\n    type: founded\n    valid_from: 2017-01-01\n---\nbody\n";
    put_from_string(&conn, "people/alice", md, None).unwrap();

    let row = link_row(
        &conn,
        "people/alice",
        "companies/brex",
        "founded",
        "frontmatter",
    )
    .expect("frontmatter row missing");
    assert!(
        (row.0 - 1.0).abs() < 1e-9,
        "edge_weight should default to 1.0, got {}",
        row.0
    );
    assert_eq!(row.1.as_deref(), Some("2017-01-01"));
    assert_eq!(row.2, None);
}

#[test]
fn scenario_string_link_defaults_to_related() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    let md = "---\ntitle: Alice\ntype: person\nlinks:\n  - companies/brex\n---\nbody\n";
    put_from_string(&conn, "people/alice", md, None).unwrap();

    let row = link_row(
        &conn,
        "people/alice",
        "companies/brex",
        "related",
        "frontmatter",
    )
    .expect("string-form frontmatter row missing");
    assert!((row.0 - 1.0).abs() < 1e-9);
    assert_eq!(row.1, None);
    assert_eq!(row.2, None);
}

#[test]
fn scenario_missing_target_logs_knowledge_gap_and_skips_edge() {
    let conn = open_test_db();
    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM knowledge_gaps", [], |row| row.get(0))
        .unwrap();
    let md = "---\ntitle: Alice\ntype: person\nlinks:\n  - target: companies/does-not-exist\n---\nbody\n";
    put_from_string(&conn, "people/alice", md, None).unwrap();

    assert_eq!(count_links(&conn, "people/alice", "frontmatter"), 0);
    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM knowledge_gaps", [], |row| row.get(0))
        .unwrap();
    assert!(
        after > before,
        "expected a knowledge_gap to be logged for unresolved target (before={before}, after={after})"
    );
}

// ── Requirement: parent / children / related fixed relationships ────────

#[test]
fn scenario_parent_field_produces_single_parent_edge() {
    let conn = open_test_db();
    seed_target(&conn, "programs/yc-w17");
    let md = "---\ntitle: Alice\ntype: person\nparent: programs/yc-w17\n---\nbody\n";
    put_from_string(&conn, "people/alice", md, None).unwrap();

    assert!(link_row(
        &conn,
        "people/alice",
        "programs/yc-w17",
        "parent",
        "frontmatter"
    )
    .is_some());
}

#[test]
fn scenario_children_field_produces_one_child_edge_per_entry() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    seed_target(&conn, "companies/scale");
    let md = "---\ntitle: Founder\ntype: person\nchildren:\n  - companies/brex\n  - companies/scale\n---\nbody\n";
    put_from_string(&conn, "people/founder", md, None).unwrap();

    assert!(link_row(
        &conn,
        "people/founder",
        "companies/brex",
        "child",
        "frontmatter"
    )
    .is_some());
    assert!(link_row(
        &conn,
        "people/founder",
        "companies/scale",
        "child",
        "frontmatter"
    )
    .is_some());
    assert_eq!(count_links(&conn, "people/founder", "frontmatter"), 2);
}

// ── Requirement: Body wikilinks produce soft graph edges ────────────────

#[test]
fn scenario_wikilink_in_compiled_truth_produces_soft_edge() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    let md = "---\ntitle: Alice\ntype: person\n---\nAlice founded [[companies/brex]].\n";
    put_from_string(&conn, "people/alice", md, None).unwrap();

    let row = link_row(
        &conn,
        "people/alice",
        "companies/brex",
        "related",
        "wiki_link",
    )
    .expect("wiki_link row missing");
    assert!(
        (row.0 - 0.5).abs() < 1e-9,
        "wiki_link edge_weight should default to 0.5, got {}",
        row.0
    );
}

// ── Requirement: tags populate `tags` only, not `links` ─────────────────

#[test]
fn scenario_tags_do_not_create_edges() {
    let conn = open_test_db();
    let md = "---\ntitle: Alice\ntype: person\ntags: [fintech, yc-w17]\n---\nbody\n";
    put_from_string(&conn, "people/alice", md, None).unwrap();

    let alice_id = page_id_for_slug(&conn, "people/alice");
    let tags: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT tag FROM tags WHERE page_id = ?1 ORDER BY tag")
            .unwrap();
        stmt.query_map([alice_id], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(tags, vec!["fintech", "yc-w17"]);
    assert_eq!(count_links(&conn, "people/alice", "frontmatter"), 0);
    assert_eq!(count_links(&conn, "people/alice", "wiki_link"), 0);
}

#[test]
fn scenario_removed_tag_is_removed_on_re_ingest() {
    let conn = open_test_db();
    let md1 = "---\ntitle: Alice\ntype: person\ntags: [fintech, yc-w17]\n---\nbody\n";
    put_from_string(&conn, "people/alice", md1, None).unwrap();
    let md2 = "---\ntitle: Alice\ntype: person\ntags: [fintech]\n---\nbody\n";
    put_from_string(&conn, "people/alice", md2, Some(1)).unwrap();

    let alice_id = page_id_for_slug(&conn, "people/alice");
    let tags: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT tag FROM tags WHERE page_id = ?1 ORDER BY tag")
            .unwrap();
        stmt.query_map([alice_id], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(tags, vec!["fintech"]);
}

// ── Requirement: derived-edge idempotency + temporal replacement ────────

#[test]
fn scenario_reingest_unchanged_frontmatter_link_yields_no_duplicate() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    let md = "---\ntitle: Alice\ntype: person\nlinks:\n  - target: companies/brex\n    type: founded\n---\nbody\n";
    put_from_string(&conn, "people/alice", md, None).unwrap();
    put_from_string(&conn, "people/alice", md, Some(1)).unwrap();

    let count: i64 = {
        let from_id = page_id_for_slug(&conn, "people/alice");
        let to_id = page_id_for_slug(&conn, "companies/brex");
        conn.query_row(
            "SELECT COUNT(*) FROM links WHERE from_page_id = ?1 AND to_page_id = ?2 \
             AND relationship = 'founded' AND source_kind = 'frontmatter'",
            rusqlite::params![from_id, to_id],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(count, 1);
}

#[test]
fn scenario_reingest_with_updated_date_replaces_temporal_range() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    let md1 = "---\ntitle: Alice\ntype: person\nlinks:\n  - target: companies/brex\n    type: founded\n    valid_from: 2017-01-01\n---\nbody\n";
    let md2 = "---\ntitle: Alice\ntype: person\nlinks:\n  - target: companies/brex\n    type: founded\n    valid_from: 2017-02-01\n---\nbody\n";
    put_from_string(&conn, "people/alice", md1, None).unwrap();
    put_from_string(&conn, "people/alice", md2, Some(1)).unwrap();

    let row = link_row(
        &conn,
        "people/alice",
        "companies/brex",
        "founded",
        "frontmatter",
    )
    .expect("updated row missing");
    assert_eq!(row.1.as_deref(), Some("2017-02-01"));
}

#[test]
fn scenario_removing_frontmatter_link_removes_derived_edge() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    let md1 = "---\ntitle: Alice\ntype: person\nlinks:\n  - target: companies/brex\n---\nbody\n";
    let md2 = "---\ntitle: Alice\ntype: person\n---\nbody\n";
    put_from_string(&conn, "people/alice", md1, None).unwrap();
    assert_eq!(count_links(&conn, "people/alice", "frontmatter"), 1);
    put_from_string(&conn, "people/alice", md2, Some(1)).unwrap();
    assert_eq!(count_links(&conn, "people/alice", "frontmatter"), 0);
}

// ── Requirement: programmatic temporal duplicates remain allowed ────────

#[test]
fn scenario_programmatic_temporal_duplicates_allowed() {
    let conn = open_test_db();
    seed_target(&conn, "people/alice");
    seed_target(&conn, "companies/brex");
    let from_id = page_id_for_slug(&conn, "people/alice");
    let to_id = page_id_for_slug(&conn, "companies/brex");

    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind, edge_weight, valid_from, valid_until) \
         VALUES (?1, ?2, 'worked_at', 'programmatic', 1.0, '2017-01-01', '2019-12-31')",
        rusqlite::params![from_id, to_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind, edge_weight, valid_from, valid_until) \
         VALUES (?1, ?2, 'worked_at', 'programmatic', 1.0, '2020-01-01', NULL)",
        rusqlite::params![from_id, to_id],
    )
    .unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM links WHERE from_page_id = ?1 AND to_page_id = ?2 \
             AND source_kind = 'programmatic'",
            rusqlite::params![from_id, to_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2, "programmatic temporal duplicates must coexist");
}
