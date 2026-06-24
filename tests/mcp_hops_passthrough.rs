#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! MCP `memory_query` graph-hops pass-through tests (fable-review #1 step 5).
//!
//! The shipped graph-expansion layer is gated behind `config.graph_depth`
//! (seeded `0`); the `hops` input field lets an MCP caller override the
//! depth per query without flipping the seed. Scenarios tested:
//!   1. `hops: 1` surfaces a linked neighbour alongside the direct hit
//!   2. absent `hops` honours the seeded `graph_depth = 0` (no expansion)
//!   3. `hops: 0` explicitly disables expansion even when `graph_depth > 0`

#[path = "common/mcp_harness.rs"]
mod harness;

use harness::{create_page, extract_text, open_test_db};
use quaid::mcp::server::{MemoryLinkInput, MemoryQueryInput, QuaidServer};

fn memory_query_input(query: &str, hops: Option<u32>) -> MemoryQueryInput {
    MemoryQueryInput {
        query: query.to_string(),
        collection: None,
        namespace: None,
        wing: None,
        limit: None,
        depth: None,
        include_superseded: None,
        hops,
        relevance_floor: None,
        max_chunks_per_doc: None,
        mmr_lambda: None,
        redact: None,
    }
}

fn seed_linked_corpus(server: &QuaidServer) {
    create_page(
        server,
        "concepts/anchor",
        "---\ntitle: Anchor\ntype: concept\n---\nalpha anchor content\n",
    );
    create_page(
        server,
        "concepts/neighbour",
        "---\ntitle: Neighbour\ntype: concept\n---\nlinked neighbour content\n",
    );
    server
        .memory_link(MemoryLinkInput {
            from_slug: "concepts/anchor".to_string(),
            to_slug: "concepts/neighbour".to_string(),
            relationship: "related".to_string(),
            valid_from: None,
            valid_until: None,
            namespace: None,
        })
        .unwrap();
}

fn query_slugs(server: &QuaidServer, query: &str, hops: Option<u32>) -> Vec<String> {
    let result = server
        .memory_query(memory_query_input(query, hops))
        .unwrap();
    // memory_query returns a {results, pending_embedding_jobs?} envelope.
    let envelope: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    let rows = envelope["results"].as_array().cloned().unwrap_or_default();
    rows.into_iter()
        .map(|row| row["slug"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn memory_query_hops_one_surfaces_linked_neighbour() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    seed_linked_corpus(&server);

    let slugs = query_slugs(&server, "alpha anchor", Some(1));

    assert!(
        slugs.contains(&"default::concepts/anchor".to_owned()),
        "direct hit must surface: {slugs:?}"
    );
    assert!(
        slugs.contains(&"default::concepts/neighbour".to_owned()),
        "hops=1 must walk the link graph and surface the neighbour: {slugs:?}"
    );
}

#[test]
fn memory_query_without_hops_honours_seeded_graph_depth_zero() {
    let (_dir, conn) = open_test_db();
    let seeded: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'graph_depth'",
            [],
            |row| row.get(0),
        )
        .expect("graph_depth must be seeded");
    assert_eq!(seeded, "0", "graph_depth seed stays 0 pending the DAB gate");
    let server = QuaidServer::new(conn);
    seed_linked_corpus(&server);

    let slugs = query_slugs(&server, "alpha anchor", None);

    assert!(slugs.contains(&"default::concepts/anchor".to_owned()));
    assert!(
        !slugs.contains(&"default::concepts/neighbour".to_owned()),
        "absent hops must keep the seeded no-expansion default: {slugs:?}"
    );
}

#[test]
fn memory_query_hops_zero_disables_configured_expansion() {
    let (_dir, conn) = open_test_db();
    conn.execute(
        "UPDATE config SET value = '2' WHERE key = 'graph_depth'",
        [],
    )
    .unwrap();
    let server = QuaidServer::new(conn);
    seed_linked_corpus(&server);

    let expanded = query_slugs(&server, "alpha anchor", None);
    assert!(
        expanded.contains(&"default::concepts/neighbour".to_owned()),
        "config graph_depth=2 must expand when no override is given: {expanded:?}"
    );

    let disabled = query_slugs(&server, "alpha anchor", Some(0));
    assert!(
        !disabled.contains(&"default::concepts/neighbour".to_owned()),
        "hops=0 must override the configured depth and disable expansion: {disabled:?}"
    );
}
