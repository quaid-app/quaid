#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Public-API integration tests for `quaid::mcp::server` covering the
//! graph-and-link tool surfaces — `memory_link`, `memory_link_close`,
//! `memory_backlinks`, and `memory_graph`. Exercises link creation/close,
//! relationship validation, backlink limits and temporal filters, and
//! depth-bounded graph traversal.

#[path = "common/mcp_harness.rs"]
mod harness;
use harness::{create_page, extract_text, open_test_db};
use quaid::mcp::server::{
    MemoryBacklinksInput, MemoryGraphInput, MemoryLinkCloseInput, MemoryLinkInput, QuaidServer,
};
use rmcp::model::ErrorCode;

#[test]
fn memory_link_with_unknown_from_slug_returns_not_found() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme Corp\n",
    );

    let error = server
        .memory_link(MemoryLinkInput {
            namespace: None,
            from_slug: "people/ghost".to_string(),
            to_slug: "companies/acme".to_string(),
            relationship: "works_at".to_string(),
            valid_from: None,
            valid_until: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32001));
}

#[test]
fn memory_link_creates_link_between_existing_pages() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme\n",
    );

    let result = server
        .memory_link(MemoryLinkInput {
            namespace: None,
            from_slug: "people/alice".to_string(),
            to_slug: "companies/acme".to_string(),
            relationship: "works_at".to_string(),
            valid_from: Some("2024-01".to_string()),
            valid_until: None,
        })
        .unwrap();

    let text = extract_text(&result);
    assert!(text.contains("Linked"));
    assert!(text.contains("default::people/alice"));
    assert!(text.contains("default::companies/acme"));
}

#[test]
fn memory_link_rejects_invalid_relationship() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme\n",
    );

    let error = server
        .memory_link(MemoryLinkInput {
            namespace: None,
            from_slug: "people/alice".to_string(),
            to_slug: "companies/acme".to_string(),
            relationship: "works at".to_string(),
            valid_from: None,
            valid_until: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}

#[test]
fn memory_link_close_with_unknown_id_returns_not_found() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_link_close(MemoryLinkCloseInput {
            link_id: 99999,
            valid_until: "2025-06".to_string(),
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32001));
}

#[test]
fn memory_link_close_rejects_invalid_temporal_value() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_link_close(MemoryLinkCloseInput {
            link_id: 1,
            valid_until: "not-a-date".to_string(),
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}

#[test]
fn memory_backlinks_returns_link_array() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme\n",
    );

    server
        .memory_link(MemoryLinkInput {
            namespace: None,
            from_slug: "people/alice".to_string(),
            to_slug: "companies/acme".to_string(),
            relationship: "works_at".to_string(),
            valid_from: None,
            valid_until: None,
        })
        .unwrap();

    let result = server
        .memory_backlinks(MemoryBacklinksInput {
            namespace: None,
            slug: "companies/acme".to_string(),
            limit: None,
            temporal: None,
        })
        .unwrap();

    let text = extract_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["from_slug"], "default::people/alice");
    assert_eq!(arr[0]["relationship"], "works_at");
}

#[test]
fn memory_backlinks_unknown_slug_returns_not_found() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_backlinks(MemoryBacklinksInput {
            namespace: None,
            slug: "nobody/ghost".to_string(),
            limit: None,
            temporal: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32001));
}

#[test]
fn memory_backlinks_applies_limit() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme\n",
    );

    for slug in ["people/alice", "people/bob", "people/carla"] {
        create_page(
            &server,
            slug,
            &format!("---\ntitle: {slug}\ntype: person\n---\n{slug}\n"),
        );
        server
            .memory_link(MemoryLinkInput {
                namespace: None,
                from_slug: slug.to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap();
    }

    let result = server
        .memory_backlinks(MemoryBacklinksInput {
            namespace: None,
            slug: "companies/acme".to_string(),
            limit: Some(2),
            temporal: None,
        })
        .unwrap();

    let text = extract_text(&result);
    let arr: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn memory_backlinks_temporal_all_includes_closed_links() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme\n",
    );

    server
        .memory_link(MemoryLinkInput {
            namespace: None,
            from_slug: "people/alice".to_string(),
            to_slug: "companies/acme".to_string(),
            relationship: "works_at".to_string(),
            valid_from: Some("2020-01-01".to_string()),
            valid_until: Some("2020-12-31".to_string()),
        })
        .unwrap();

    let result = server
        .memory_backlinks(MemoryBacklinksInput {
            namespace: None,
            slug: "companies/acme".to_string(),
            limit: None,
            temporal: Some("all".to_string()),
        })
        .unwrap();

    let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(rows.len(), 1);
}

#[test]
fn memory_backlinks_rejects_invalid_temporal_filter() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_backlinks(MemoryBacklinksInput {
            namespace: None,
            slug: "people/alice".to_string(),
            limit: None,
            temporal: Some("future".to_string()),
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}

#[test]
fn memory_graph_returns_nodes_and_edges_json() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme\n",
    );

    server
        .memory_link(MemoryLinkInput {
            namespace: None,
            from_slug: "people/alice".to_string(),
            to_slug: "companies/acme".to_string(),
            relationship: "works_at".to_string(),
            valid_from: None,
            valid_until: None,
        })
        .unwrap();

    let result = server
        .memory_graph(MemoryGraphInput {
            namespace: None,
            slug: "people/alice".to_string(),
            depth: Some(2),
            temporal: None,
        })
        .unwrap();

    let text = extract_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let node_slugs = parsed["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["slug"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(node_slugs.contains(&"default::people/alice"));
    assert!(node_slugs.contains(&"default::companies/acme"));
    assert!(parsed["edges"].as_array().unwrap().iter().any(|edge| {
        edge["from"] == "default::people/alice" && edge["to"] == "default::companies/acme"
    }));
}

#[test]
fn memory_graph_unknown_slug_returns_not_found() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_graph(MemoryGraphInput {
            namespace: None,
            slug: "people/ghost".to_string(),
            depth: None,
            temporal: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32001));
}

#[test]
fn memory_graph_temporal_all_includes_closed_links() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme\n",
    );

    server
        .memory_link(MemoryLinkInput {
            namespace: None,
            from_slug: "people/alice".to_string(),
            to_slug: "companies/acme".to_string(),
            relationship: "works_at".to_string(),
            valid_from: Some("2020-01-01".to_string()),
            valid_until: Some("2020-12-31".to_string()),
        })
        .unwrap();

    let result = server
        .memory_graph(MemoryGraphInput {
            namespace: None,
            slug: "people/alice".to_string(),
            depth: None,
            temporal: Some("all".to_string()),
        })
        .unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(parsed["edges"].as_array().unwrap().len(), 1);
}
