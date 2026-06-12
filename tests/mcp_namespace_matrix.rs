#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Namespace matrix across the MCP tool surface: one slug exists in the
//! global namespace and in two named namespaces, and every namespace-aware
//! tool input must bind to the requested page deterministically. Pre-#212
//! these tools were slug-only and bound to an arbitrary row.

#[path = "common/mcp_harness.rs"]
mod harness;
use harness::{extract_text, open_test_db};
use quaid::mcp::server::{
    MemoryBacklinksInput, MemoryGetInput, MemoryGraphInput, MemoryLinkInput, MemoryPutInput,
    MemoryRawInput, MemoryTagsInput, MemoryTimelineInput, QuaidServer,
};
use rusqlite::Connection;

const SLUG: &str = "notes/shared";
const TARGET_SLUG: &str = "notes/target";

fn put(server: &QuaidServer, slug: &str, namespace: Option<&str>, marker: &str) {
    server
        .memory_put(MemoryPutInput {
            slug: slug.to_string(),
            content: format!(
                "---\ntitle: {marker}\ntype: concept\n---\n{marker} body\n\n---\n\n2026-06-01: {marker} event\n"
            ),
            expected_version: None,
            namespace: namespace.map(str::to_string),
        })
        .unwrap();
}

/// Returns the server plus a second connection to the same database file so
/// assertions can inspect state without reaching into the server's private
/// `db` handle.
fn seeded_server() -> (tempfile::TempDir, QuaidServer, Connection) {
    let (dir, conn) = open_test_db();
    let verify = quaid::core::db::open(dir.path().join("server.db").to_str().unwrap()).unwrap();
    let server = QuaidServer::new(conn);
    for (namespace, marker) in [
        (None, "global"),
        (Some("ns-a"), "alpha"),
        (Some("ns-b"), "bravo"),
    ] {
        put(&server, SLUG, namespace, marker);
        put(&server, TARGET_SLUG, namespace, &format!("{marker}-target"));
    }
    (dir, server, verify)
}

fn page_id(verify: &Connection, namespace: &str, slug: &str) -> i64 {
    verify
        .query_row(
            "SELECT id FROM pages WHERE namespace = ?1 AND slug = ?2",
            rusqlite::params![namespace, slug],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn memory_get_resolves_requested_namespace_with_global_fallback() {
    let (_dir, server, _verify) = seeded_server();

    for (namespace, marker) in [
        (Some("ns-a"), "alpha"),
        (Some("ns-b"), "bravo"),
        (Some(""), "global"),
        (None, "global"),
        // documented fallback: unknown namespace reads global memory
        (Some("ns-c"), "global"),
    ] {
        let result = server
            .memory_get(MemoryGetInput {
                slug: SLUG.to_string(),
                namespace: namespace.map(str::to_string),
            })
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(
            payload["title"], *marker,
            "namespace {namespace:?} must bind to the {marker} page"
        );
    }
}

#[test]
fn memory_link_and_backlinks_bind_to_namespaced_endpoints() {
    let (_dir, server, verify) = seeded_server();

    server
        .memory_link(MemoryLinkInput {
            from_slug: SLUG.to_string(),
            to_slug: TARGET_SLUG.to_string(),
            namespace: Some("ns-a".to_string()),
            relationship: "related".to_string(),
            valid_from: None,
            valid_until: None,
        })
        .unwrap();

    let from_id = page_id(&verify, "ns-a", SLUG);
    let to_id = page_id(&verify, "ns-a", TARGET_SLUG);
    let (linked_from, linked_to): (i64, i64) = verify
        .query_row(
            "SELECT from_page_id, to_page_id FROM links WHERE relationship = 'related'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((linked_from, linked_to), (from_id, to_id));

    // Backlinks of the ns-a target see the link; the global target does not.
    let ns_backlinks = server
        .memory_backlinks(MemoryBacklinksInput {
            slug: TARGET_SLUG.to_string(),
            namespace: Some("ns-a".to_string()),
            limit: None,
            temporal: None,
        })
        .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&ns_backlinks)).unwrap();
    assert_eq!(rows.len(), 1, "ns-a target must list the inbound link");

    let global_backlinks = server
        .memory_backlinks(MemoryBacklinksInput {
            slug: TARGET_SLUG.to_string(),
            namespace: Some("".to_string()),
            limit: None,
            temporal: None,
        })
        .unwrap();
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&extract_text(&global_backlinks)).unwrap();
    assert!(rows.is_empty(), "global target must have no backlinks");
}

#[test]
fn memory_graph_roots_at_the_namespaced_page() {
    let (_dir, server, _verify) = seeded_server();
    server
        .memory_link(MemoryLinkInput {
            from_slug: SLUG.to_string(),
            to_slug: TARGET_SLUG.to_string(),
            namespace: Some("ns-b".to_string()),
            relationship: "related".to_string(),
            valid_from: None,
            valid_until: None,
        })
        .unwrap();

    let graph_ns = server
        .memory_graph(MemoryGraphInput {
            slug: SLUG.to_string(),
            namespace: Some("ns-b".to_string()),
            depth: Some(1),
            temporal: None,
        })
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&extract_text(&graph_ns)).unwrap();
    let edges = payload["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1, "ns-b root must reach its ns-b neighbour");

    let graph_global = server
        .memory_graph(MemoryGraphInput {
            slug: SLUG.to_string(),
            namespace: Some("".to_string()),
            depth: Some(1),
            temporal: None,
        })
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&extract_text(&graph_global)).unwrap();
    let edges = payload["edges"].as_array().unwrap();
    assert!(edges.is_empty(), "global root has no outbound links");
}

#[test]
fn memory_tags_and_raw_attach_to_the_namespaced_page() {
    let (_dir, server, verify) = seeded_server();

    server
        .memory_tags(MemoryTagsInput {
            slug: SLUG.to_string(),
            namespace: Some("ns-a".to_string()),
            add: Some(vec!["scoped".to_string()]),
            remove: None,
        })
        .unwrap();
    server
        .memory_raw(MemoryRawInput {
            slug: SLUG.to_string(),
            namespace: Some("ns-a".to_string()),
            source: "matrix-test".to_string(),
            data: serde_json::json!({"k": "v"}),
            overwrite: None,
        })
        .unwrap();

    let ns_a_id = page_id(&verify, "ns-a", SLUG);
    let tagged: i64 = verify
        .query_row("SELECT page_id FROM tags WHERE tag = 'scoped'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let raw_bound: i64 = verify
        .query_row(
            "SELECT page_id FROM raw_data WHERE source = 'matrix-test'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tagged, ns_a_id, "tag must attach to the ns-a page");
    assert_eq!(raw_bound, ns_a_id, "raw data must attach to the ns-a page");
}

#[test]
fn memory_timeline_reads_the_namespaced_page_timeline() {
    let (_dir, server, _verify) = seeded_server();

    let result = server
        .memory_timeline(MemoryTimelineInput {
            slug: SLUG.to_string(),
            namespace: Some("ns-b".to_string()),
            limit: None,
        })
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    let entries = payload["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry.as_str().unwrap_or_default().contains("bravo event")),
        "timeline must come from the ns-b page, got {entries:?}"
    );
}
