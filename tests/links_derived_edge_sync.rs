#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure"
)]

//! Wave 2 tests for derived-edge sync primitives in `src/core/links.rs`:
//!
//! - `upsert_derived_edge` idempotency and on-conflict field replacement
//! - `sync_frontmatter_edges` stale deletion scoped to source page + source_kind
//! - `sync_wikilink_edges` extraction, scoping, and stale deletion
//! - Unresolved-target gap logging is deduplicated across re-runs
//! - Programmatic temporal link history is unaffected by either sync

use quaid::core::db::open;
use quaid::core::links::{
    sync_frontmatter_edges, sync_wikilink_edges, upsert_derived_edge, FrontmatterLink,
};
use rusqlite::{params, Connection};

fn collection_id(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT id FROM collections ORDER BY id LIMIT 1",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

fn insert_page(conn: &Connection, slug: &str) -> i64 {
    let cid = collection_id(conn);
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline,
              frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, 'concept', ?2, '', '', '', '{}', 'notes', '', 1)",
        params![cid, slug, uuid::Uuid::now_v7().to_string()],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn count_links(conn: &Connection, from: i64, source_kind: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM links WHERE from_page_id = ?1 AND source_kind = ?2",
        params![from, source_kind],
        |row| row.get(0),
    )
    .unwrap()
}

fn count_gaps(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM knowledge_gaps", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn upsert_derived_edge_inserts_then_updates_on_conflict() {
    let conn = open(":memory:").unwrap();
    let from = insert_page(&conn, "notes/from");
    let to = insert_page(&conn, "notes/to");

    upsert_derived_edge(
        &conn,
        from,
        to,
        "related",
        "frontmatter",
        1.0,
        Some("2024-01-01"),
        None,
        "ctx-one",
    )
    .unwrap();
    assert_eq!(count_links(&conn, from, "frontmatter"), 1);

    upsert_derived_edge(
        &conn,
        from,
        to,
        "related",
        "frontmatter",
        0.42,
        Some("2025-06-01"),
        Some("2025-12-31"),
        "ctx-two",
    )
    .unwrap();

    let (weight, vf, vu, ctx): (f64, Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT edge_weight, valid_from, valid_until, context \
             FROM links WHERE from_page_id = ?1 AND source_kind = 'frontmatter'",
            params![from],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(count_links(&conn, from, "frontmatter"), 1);
    assert!((weight - 0.42).abs() < f64::EPSILON);
    assert_eq!(vf.as_deref(), Some("2025-06-01"));
    assert_eq!(vu.as_deref(), Some("2025-12-31"));
    assert_eq!(ctx, "ctx-two");
}

#[test]
fn upsert_derived_edge_rejects_programmatic_kind() {
    let conn = open(":memory:").unwrap();
    let from = insert_page(&conn, "notes/from");
    let to = insert_page(&conn, "notes/to");

    let err = upsert_derived_edge(
        &conn,
        from,
        to,
        "related",
        "programmatic",
        1.0,
        None,
        None,
        "",
    )
    .expect_err("programmatic is not a derived edge kind");
    let msg = format!("{err}");
    assert!(msg.contains("programmatic"));
}

#[test]
fn sync_frontmatter_edges_is_idempotent_and_replaces_stale_rows() {
    let conn = open(":memory:").unwrap();
    let from = insert_page(&conn, "notes/from");
    let alice = insert_page(&conn, "people/alice");
    let bob = insert_page(&conn, "people/bob");
    let cid = collection_id(&conn);

    let first = vec![
        FrontmatterLink {
            target: "people/alice".to_string(),
            relationship: "related".to_string(),
            valid_from: Some("2024-01-01".to_string()),
            valid_until: None,
        },
        FrontmatterLink {
            target: "people/bob".to_string(),
            relationship: "knows".to_string(),
            valid_from: None,
            valid_until: None,
        },
    ];

    sync_frontmatter_edges(&conn, from, cid, &first).unwrap();
    assert_eq!(count_links(&conn, from, "frontmatter"), 2);

    // Idempotent re-run: same edges, no duplicates.
    sync_frontmatter_edges(&conn, from, cid, &first).unwrap();
    assert_eq!(count_links(&conn, from, "frontmatter"), 2);

    // Drop bob, change alice's valid_from to prove temporal replacement.
    let second = vec![FrontmatterLink {
        target: "people/alice".to_string(),
        relationship: "related".to_string(),
        valid_from: Some("2025-06-01".to_string()),
        valid_until: None,
    }];
    sync_frontmatter_edges(&conn, from, cid, &second).unwrap();

    // Bob's frontmatter row is gone; alice's row was updated in place.
    assert_eq!(count_links(&conn, from, "frontmatter"), 1);
    let (to_id, vf): (i64, Option<String>) = conn
        .query_row(
            "SELECT to_page_id, valid_from FROM links \
             WHERE from_page_id = ?1 AND source_kind = 'frontmatter'",
            params![from],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(to_id, alice);
    assert_eq!(vf.as_deref(), Some("2025-06-01"));

    let _ = bob; // bob still exists as a page, just no longer linked
}

#[test]
fn sync_frontmatter_edges_only_deletes_for_this_source_page_and_kind() {
    let conn = open(":memory:").unwrap();
    let from_a = insert_page(&conn, "notes/from-a");
    let from_b = insert_page(&conn, "notes/from-b");
    let target = insert_page(&conn, "people/alice");
    let cid = collection_id(&conn);

    // Both pages start with a frontmatter edge to alice.
    let edges = vec![FrontmatterLink {
        target: "people/alice".to_string(),
        relationship: "related".to_string(),
        valid_from: None,
        valid_until: None,
    }];
    sync_frontmatter_edges(&conn, from_a, cid, &edges).unwrap();
    sync_frontmatter_edges(&conn, from_b, cid, &edges).unwrap();

    // Page A also has a manual programmatic temporal link to alice — a second
    // row with the same (from, to, relationship) is legal because the unique
    // index excludes `programmatic`.
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind, valid_from) \
         VALUES (?1, ?2, 'related', 'programmatic', '2020-01-01')",
        params![from_a, target],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind, valid_from) \
         VALUES (?1, ?2, 'related', 'programmatic', '2021-01-01')",
        params![from_a, target],
    )
    .unwrap();

    // Page A also has a wiki_link row that must survive a frontmatter sync.
    upsert_derived_edge(
        &conn,
        from_a,
        target,
        "related",
        "wiki_link",
        0.5,
        None,
        None,
        "",
    )
    .unwrap();

    // Sync A with an empty frontmatter set: only A's frontmatter row may go.
    sync_frontmatter_edges(&conn, from_a, cid, &[]).unwrap();

    assert_eq!(count_links(&conn, from_a, "frontmatter"), 0);
    assert_eq!(count_links(&conn, from_b, "frontmatter"), 1);
    assert_eq!(count_links(&conn, from_a, "wiki_link"), 1);
    assert_eq!(count_links(&conn, from_a, "programmatic"), 2);
}

#[test]
fn sync_wikilink_edges_extracts_body_targets_and_dedupes() {
    let conn = open(":memory:").unwrap();
    let from = insert_page(&conn, "notes/from");
    let alice = insert_page(&conn, "people/alice");
    let bob = insert_page(&conn, "people/bob");
    let cid = collection_id(&conn);

    let truth = "See [[people/alice]] and again [[people/alice]].";
    let timeline = "Earlier note about [[people/bob]].";

    sync_wikilink_edges(&conn, from, cid, truth, timeline).unwrap();
    assert_eq!(count_links(&conn, from, "wiki_link"), 2);

    // Re-run: idempotent.
    sync_wikilink_edges(&conn, from, cid, truth, timeline).unwrap();
    assert_eq!(count_links(&conn, from, "wiki_link"), 2);

    // edge_weight comes from config (`edge_weight_wikilink = 0.5`).
    let weight: f64 = conn
        .query_row(
            "SELECT edge_weight FROM links \
             WHERE from_page_id = ?1 AND to_page_id = ?2 AND source_kind = 'wiki_link'",
            params![from, alice],
            |row| row.get(0),
        )
        .unwrap();
    assert!((weight - 0.5).abs() < f64::EPSILON);

    // Drop bob from the body — his wiki_link row must be deleted.
    sync_wikilink_edges(&conn, from, cid, "Only [[people/alice]].", "").unwrap();
    assert_eq!(count_links(&conn, from, "wiki_link"), 1);
    let surviving: i64 = conn
        .query_row(
            "SELECT to_page_id FROM links \
             WHERE from_page_id = ?1 AND source_kind = 'wiki_link'",
            params![from],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(surviving, alice);
    let _ = bob;
}

#[test]
fn unresolved_targets_are_logged_once_via_knowledge_gap_dedup() {
    let conn = open(":memory:").unwrap();
    let from = insert_page(&conn, "notes/from");
    let cid = collection_id(&conn);

    let edges = vec![FrontmatterLink {
        target: "people/ghost".to_string(),
        relationship: "knows".to_string(),
        valid_from: None,
        valid_until: None,
    }];

    sync_frontmatter_edges(&conn, from, cid, &edges).unwrap();
    sync_frontmatter_edges(&conn, from, cid, &edges).unwrap();
    sync_frontmatter_edges(&conn, from, cid, &edges).unwrap();

    assert_eq!(count_gaps(&conn), 1);

    // Same source page, same source_kind, same target, different relationship
    // → a separate dedup key, so a second gap is logged exactly once.
    let edges_two = vec![FrontmatterLink {
        target: "people/ghost".to_string(),
        relationship: "related".to_string(),
        valid_from: None,
        valid_until: None,
    }];
    sync_frontmatter_edges(&conn, from, cid, &edges_two).unwrap();
    sync_frontmatter_edges(&conn, from, cid, &edges_two).unwrap();
    assert_eq!(count_gaps(&conn), 2);

    // Same target seen via a wikilink → distinct dedup key from frontmatter.
    sync_wikilink_edges(&conn, from, cid, "Mentions [[people/ghost]].", "").unwrap();
    sync_wikilink_edges(&conn, from, cid, "Mentions [[people/ghost]].", "").unwrap();
    assert_eq!(count_gaps(&conn), 3);

    // Gaps are bound to the source page and use 'internal' sensitivity.
    let (page_id, sensitivity): (Option<i64>, String) = conn
        .query_row(
            "SELECT page_id, sensitivity FROM knowledge_gaps ORDER BY id LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(page_id, Some(from));
    assert_eq!(sensitivity, "internal");
}

#[test]
fn sync_wikilink_edges_does_not_resolve_across_collections() {
    let conn = open(":memory:").unwrap();
    // Add a second collection and put the target there.
    conn.execute(
        "INSERT INTO collections (name, root_path) VALUES ('other', '/tmp/other-collection')",
        [],
    )
    .unwrap();
    let other_cid: i64 = conn
        .query_row(
            "SELECT id FROM collections WHERE name = 'other'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let from = insert_page(&conn, "notes/from");
    let source_cid = collection_id(&conn);
    // Cross-collection page with same slug — must NOT be linked.
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline,
              frontmatter, wing, room, version)
         VALUES (?1, 'people/alice', ?2, 'concept', 'Alice', '', '', '', '{}', 'notes', '', 1)",
        params![other_cid, uuid::Uuid::now_v7().to_string()],
    )
    .unwrap();

    sync_wikilink_edges(&conn, from, source_cid, "Hi [[people/alice]].", "").unwrap();

    assert_eq!(count_links(&conn, from, "wiki_link"), 0);
    assert_eq!(count_gaps(&conn), 1);
}
