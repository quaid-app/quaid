#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration tests legitimately panic on setup failure"
)]

//! Tasks 9.x / 10.x — graph-aware retrieval, path output, and CLI `--hops`
//! plumbing. Behaviour comes from `specs/graph-aware-retrieval/spec.md`.

use quaid::core::db;
use quaid::core::graph::{self, TemporalFilter};
use quaid::core::search::{expand_graph, hybrid_search, HybridSearch};
use quaid::core::types::SearchResult;
use rusqlite::Connection;

fn open_db() -> Connection {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("retr.db");
    let conn = db::open(path.to_str().unwrap()).unwrap();
    std::mem::forget(dir);
    conn
}

fn insert_page(conn: &Connection, slug: &str) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
         VALUES (?1, 'concept', ?1, ?1, '', '', '{}', '', '', 1)",
        rusqlite::params![slug],
    )
    .unwrap();
}

fn insert_link(
    conn: &Connection,
    from: &str,
    to: &str,
    rel: &str,
    source_kind: &str,
    edge_weight: f64,
    valid_until: Option<&str>,
) {
    let from_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [from], |r| r.get(0))
        .unwrap();
    let to_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [to], |r| r.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind, edge_weight, valid_until) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![from_id, to_id, rel, source_kind, edge_weight, valid_until],
    )
    .unwrap();
}

fn result(slug: &str, score: f64) -> SearchResult {
    SearchResult {
        slug: slug.to_owned(),
        title: slug.to_owned(),
        summary: slug.to_owned(),
        score,
        wing: String::new(),
    }
}

// ── 9.1/9.2 expand_graph mechanics ───────────────────────────

#[test]
fn expand_graph_adds_1_hop_neighbour_with_decay() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(&conn, "alice", "brex", "founded", "frontmatter", 1.0, None);

    let added = expand_graph(&conn, &[result("alice", 1.0)], 1, 50, 0.5, None).unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].slug, "brex");
    assert!(
        (added[0].score - 0.5).abs() < 1e-9,
        "expected 1.0 * 1.0 * 0.5 = 0.5, got {}",
        added[0].score
    );
}

#[test]
fn expand_graph_respects_depth_bound() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_page(&conn, "fintech-investor");
    insert_link(&conn, "alice", "brex", "founded", "frontmatter", 1.0, None);
    insert_link(
        &conn,
        "brex",
        "fintech-investor",
        "related",
        "wiki_link",
        0.5,
        None,
    );

    let added = expand_graph(&conn, &[result("alice", 1.0)], 1, 50, 0.5, None).unwrap();
    assert!(added.iter().any(|r| r.slug == "brex"));
    assert!(
        !added.iter().any(|r| r.slug == "fintech-investor"),
        "depth=1 must not reach 2-hop node"
    );
}

#[test]
fn expand_graph_excludes_expired_temporal_edges() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(
        &conn,
        "alice",
        "brex",
        "founded",
        "frontmatter",
        1.0,
        Some("2020-01-01"),
    );

    let added = expand_graph(&conn, &[result("alice", 1.0)], 2, 50, 0.5, None).unwrap();
    assert!(
        !added.iter().any(|r| r.slug == "brex"),
        "expired edge must not appear in expansion"
    );
}

#[test]
fn expand_graph_ranks_frontmatter_above_wikilink_at_equal_depth() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_page(&conn, "scale");
    insert_link(&conn, "alice", "brex", "founded", "frontmatter", 1.0, None);
    insert_link(&conn, "alice", "scale", "mentions", "wiki_link", 0.5, None);

    let added = expand_graph(&conn, &[result("alice", 1.0)], 1, 50, 0.5, None).unwrap();
    let brex_idx = added.iter().position(|r| r.slug == "brex").unwrap();
    let scale_idx = added.iter().position(|r| r.slug == "scale").unwrap();
    assert!(
        brex_idx < scale_idx,
        "higher edge_weight must rank first: brex idx={brex_idx} scale idx={scale_idx}"
    );
}

#[test]
fn expand_graph_caps_max_added() {
    let conn = open_db();
    insert_page(&conn, "root");
    for i in 0..20 {
        let slug = format!("n{i}");
        insert_page(&conn, &slug);
        insert_link(&conn, "root", &slug, "related", "frontmatter", 1.0, None);
    }
    let added = expand_graph(&conn, &[result("root", 1.0)], 1, 5, 0.5, None).unwrap();
    assert_eq!(added.len(), 5, "expansion must respect max_added=5");
}

#[test]
fn expand_graph_skips_initial_candidates() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(&conn, "alice", "brex", "founded", "frontmatter", 1.0, None);

    let initial = vec![result("alice", 1.0), result("brex", 0.9)];
    let added = expand_graph(&conn, &initial, 1, 50, 0.5, None).unwrap();
    assert!(
        added.is_empty(),
        "slugs already in candidates must not be re-added"
    );
}

#[test]
fn expand_graph_depth_zero_returns_empty() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(&conn, "alice", "brex", "founded", "frontmatter", 1.0, None);
    let added = expand_graph(&conn, &[result("alice", 1.0)], 0, 50, 0.5, None).unwrap();
    assert!(added.is_empty());
}

#[test]
fn expand_graph_respects_collection_filter() {
    let conn = open_db();
    // Create a second collection 'work'
    conn.execute(
        "INSERT INTO collections (id, name, root_path, state, writable, is_write_target) \
         VALUES (2, 'work', '/tmp/work', 'active', 0, 0)",
        [],
    )
    .unwrap();
    insert_page(&conn, "alice");
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version, collection_id) \
         VALUES ('outside', 'concept', 'outside', '', '', '', '{}', '', '', 1, 2)",
        [],
    )
    .unwrap();
    let from_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = 'alice'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let to_id: i64 = conn
        .query_row(
            "SELECT id FROM pages WHERE slug = 'outside' AND collection_id = 2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind, edge_weight) \
         VALUES (?1, ?2, 'related', 'frontmatter', 1.0)",
        rusqlite::params![from_id, to_id],
    )
    .unwrap();

    // With collection filter = 1 (default), the work-collection neighbour is excluded.
    let added = expand_graph(&conn, &[result("alice", 1.0)], 1, 50, 0.5, Some(1)).unwrap();
    assert!(
        !added.iter().any(|r| r.slug == "outside"),
        "collection_filter must exclude cross-collection targets: {added:?}"
    );

    // Without a collection filter, the cross-collection neighbour is reachable.
    let added_unfiltered = expand_graph(&conn, &[result("alice", 1.0)], 1, 50, 0.5, None).unwrap();
    assert!(added_unfiltered.iter().any(|r| r.slug == "outside"));
}

// ── 9.3 hybrid_search invokes expansion at default config ────

#[test]
fn hybrid_search_with_hops_zero_disables_expansion() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(&conn, "alice", "brex", "founded", "frontmatter", 1.0, None);

    // Force exact-slug short-circuit to deterministically seed top-K = [alice].
    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "alice",
            limit: 10,
            hops: Some(0),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(results.iter().any(|r| r.slug == "alice"));
    assert!(
        !results.iter().any(|r| r.slug == "brex"),
        "hops=0 must keep baseline behaviour"
    );
}

#[test]
fn hybrid_search_with_hops_one_adds_neighbour() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(&conn, "alice", "brex", "founded", "frontmatter", 1.0, None);

    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "alice",
            limit: 10,
            hops: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        results.iter().any(|r| r.slug == "brex"),
        "hops=1 must add the 1-hop neighbour: got {:?}",
        results.iter().map(|r| &r.slug).collect::<Vec<_>>()
    );
}

#[test]
fn hybrid_search_reads_graph_depth_from_config_when_hops_none() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(&conn, "alice", "brex", "founded", "frontmatter", 1.0, None);

    // graph_depth defaults to '1' from the seeded config.
    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "alice",
            limit: 10,
            hops: None,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(results.iter().any(|r| r.slug == "brex"));
}

#[test]
fn hybrid_search_config_graph_depth_zero_disables_expansion() {
    let conn = open_db();
    conn.execute(
        "UPDATE config SET value = '0' WHERE key = 'graph_depth'",
        [],
    )
    .unwrap();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(&conn, "alice", "brex", "founded", "frontmatter", 1.0, None);

    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "alice",
            limit: 10,
            hops: None,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        !results.iter().any(|r| r.slug == "brex"),
        "config graph_depth=0 must preserve baseline behaviour"
    );
}

// ── 10.x neighborhood_graph paths field ──────────────────────

#[test]
fn neighborhood_graph_paths_root_is_empty() {
    let conn = open_db();
    insert_page(&conn, "alice");
    let result = graph::neighborhood_graph("alice", 0, TemporalFilter::Active, &conn).unwrap();
    let root_path = result
        .paths
        .get("alice")
        .expect("root must have a path entry");
    assert!(root_path.is_empty(), "root path must be empty");
}

#[test]
fn neighborhood_graph_paths_returns_two_hop_chain() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_page(&conn, "fintech-investor");
    insert_link(&conn, "alice", "brex", "founded", "programmatic", 1.0, None);
    insert_link(
        &conn,
        "brex",
        "fintech-investor",
        "related",
        "programmatic",
        1.0,
        None,
    );

    let result = graph::neighborhood_graph("alice", 2, TemporalFilter::Active, &conn).unwrap();
    let path = result
        .paths
        .get("fintech-investor")
        .expect("fintech-investor must have a path");
    assert_eq!(path.len(), 2, "expected 2 hops, got {path:?}");
    assert_eq!(
        path[0],
        (
            "alice".to_string(),
            "founded".to_string(),
            "brex".to_string()
        )
    );
    assert_eq!(
        path[1],
        (
            "brex".to_string(),
            "related".to_string(),
            "fintech-investor".to_string()
        )
    );
}

// ── 10.3 / 10.5 quaid graph CLI renders paths ────────────────

#[test]
fn quaid_graph_text_output_includes_paths_block() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(&conn, "alice", "brex", "founded", "programmatic", 1.0, None);

    let mut out = Vec::<u8>::new();
    quaid::commands::graph::run_to(&conn, "alice", 1, "current", false, &mut out).unwrap();
    let output = String::from_utf8(out).unwrap();
    assert!(output.contains("paths:"), "missing paths block: {output}");
    assert!(
        output.contains("default::brex: default::alice -[founded]-> default::brex"),
        "missing path triple: {output}"
    );
}

#[test]
fn quaid_graph_json_output_includes_paths_field() {
    let conn = open_db();
    insert_page(&conn, "alice");
    insert_page(&conn, "brex");
    insert_link(&conn, "alice", "brex", "founded", "programmatic", 1.0, None);

    let mut out = Vec::<u8>::new();
    quaid::commands::graph::run_to(&conn, "alice", 1, "current", true, &mut out).unwrap();
    let output = String::from_utf8(out).unwrap();
    let v: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert!(v.get("paths").is_some(), "missing paths field: {output}");
    let paths = v.get("paths").unwrap();
    assert!(
        paths.get("default::brex").is_some(),
        "missing brex path key"
    );
}
