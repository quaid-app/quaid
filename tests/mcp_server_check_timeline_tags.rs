#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Public-API integration tests for `quaid::mcp::server` covering the
//! contradiction-and-metadata tool surfaces — `memory_check`,
//! `memory_timeline`, and `memory_tags`. Exercises clean/contradicting
//! page detection, slug-scoped filtering, timeline rendering for pages
//! with and without entries, and tag list/add/remove round-trips.

#[path = "common/mcp_harness.rs"]
mod harness;
use harness::{
    create_page, create_page_in_collection, extract_text, insert_collection, open_test_db,
};
use quaid::mcp::server::{MemoryCheckInput, MemoryTagsInput, MemoryTimelineInput, QuaidServer};
use rmcp::model::ErrorCode;

#[test]
fn memory_check_on_clean_page_returns_empty_array() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice is a person.\n",
    );

    let result = server
        .memory_check(MemoryCheckInput {
            namespace: None,
            slug: Some("people/alice".to_string()),
            resolve: None,
            keep: None,
        })
        .unwrap();

    let text = extract_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 0);
}

#[test]
fn memory_check_detects_contradiction_on_page() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\n## Assertions\nAlice works at Acme Corp.\nAlice works at Beta Corp.\n",
    );

    let result = server
        .memory_check(MemoryCheckInput {
            namespace: None,
            slug: Some("people/alice".to_string()),
            resolve: None,
            keep: None,
        })
        .unwrap();

    let text = extract_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(!parsed.as_array().unwrap().is_empty());
}

#[test]
fn memory_check_filters_output_to_requested_slug() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\n## Assertions\nAlice works at Acme Corp.\nAlice works at Beta Corp.\n",
    );
    create_page(
        &server,
        "people/bob",
        "---\ntitle: Bob\ntype: person\n---\n## Assertions\nBob works at Gamma LLC.\nBob works at Delta LLC.\n",
    );

    server
        .memory_check(MemoryCheckInput {
            namespace: None,
            slug: Some("people/bob".to_string()),
            resolve: None,
            keep: None,
        })
        .unwrap();

    let result = server
        .memory_check(MemoryCheckInput {
            namespace: None,
            slug: Some("people/alice".to_string()),
            resolve: None,
            keep: None,
        })
        .unwrap();

    let text = extract_text(&result);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    assert!(!parsed.is_empty());
    assert!(parsed.iter().all(|row| {
        row["page_slug"] == "default::people/alice"
            || row["other_page_slug"] == "default::people/alice"
    }));
}

#[test]
fn memory_check_explicit_collection_slug_filters_to_resolved_page_when_slug_collides() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "memory", false);
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\n## Assertions\nAlice works at Acme Corp.\nAlice works at Beta Corp.\n",
    );
    create_page_in_collection(
        &server,
        "memory",
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nMemory Alice is a person.\n",
    );

    server
        .memory_check(MemoryCheckInput {
            namespace: None,
            slug: Some("default::people/alice".to_string()),
            resolve: None,
            keep: None,
        })
        .unwrap();

    let result = server
        .memory_check(MemoryCheckInput {
            namespace: None,
            slug: Some("memory::people/alice".to_string()),
            resolve: None,
            keep: None,
        })
        .unwrap();

    let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert!(parsed.is_empty());
}

#[test]
fn memory_check_without_slug_returns_all_unresolved_contradictions() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\n## Assertions\nAlice works at Acme Corp.\nAlice works at Beta Corp.\n",
    );
    create_page(
        &server,
        "people/bob",
        "---\ntitle: Bob\ntype: person\n---\n## Assertions\nBob works at Gamma LLC.\nBob works at Delta LLC.\n",
    );

    let result = server
        .memory_check(MemoryCheckInput {
            namespace: None,
            slug: None,
            resolve: None,
            keep: None,
        })
        .unwrap();

    let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(parsed.len(), 2);
}

#[test]
fn memory_timeline_on_unknown_slug_returns_not_found() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_timeline(MemoryTimelineInput {
            namespace: None,
            slug: "nobody/ghost".to_string(),
            limit: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32001));
}

#[test]
fn memory_timeline_returns_entries_for_page_with_timeline() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice bio\n\n## Timeline\n\n2024-01: Joined Acme\n---\n2024-06: Promoted\n",
    );

    let result = server
        .memory_timeline(MemoryTimelineInput {
            namespace: None,
            slug: "people/alice".to_string(),
            limit: Some(10),
        })
        .unwrap();

    let text = extract_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["slug"], "default::people/alice");
}

#[test]
fn memory_timeline_returns_empty_entries_for_page_without_timeline_data() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice bio\n",
    );

    let result = server
        .memory_timeline(MemoryTimelineInput {
            namespace: None,
            slug: "people/alice".to_string(),
            limit: None,
        })
        .unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    assert!(parsed["entries"].as_array().unwrap().is_empty());
}

#[test]
fn memory_tags_list_add_remove_round_trip() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );

    // List tags — should be empty
    let result = server
        .memory_tags(MemoryTagsInput {
            namespace: None,
            slug: "people/alice".to_string(),
            add: None,
            remove: None,
        })
        .unwrap();
    let text = extract_text(&result);
    let tags: Vec<String> = serde_json::from_str(&text).unwrap();
    assert!(tags.is_empty());

    // Add tags
    let result = server
        .memory_tags(MemoryTagsInput {
            namespace: None,
            slug: "people/alice".to_string(),
            add: Some(vec!["investor".to_string(), "founder".to_string()]),
            remove: None,
        })
        .unwrap();
    let text = extract_text(&result);
    let tags: Vec<String> = serde_json::from_str(&text).unwrap();
    assert_eq!(tags, vec!["founder", "investor"]);

    // Remove a tag
    let result = server
        .memory_tags(MemoryTagsInput {
            namespace: None,
            slug: "people/alice".to_string(),
            add: None,
            remove: Some(vec!["investor".to_string()]),
        })
        .unwrap();
    let text = extract_text(&result);
    let tags: Vec<String> = serde_json::from_str(&text).unwrap();
    assert_eq!(tags, vec!["founder"]);
}

#[test]
fn memory_tags_unknown_slug_returns_not_found() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_tags(MemoryTagsInput {
            namespace: None,
            slug: "nobody/ghost".to_string(),
            add: Some(vec!["tag".to_string()]),
            remove: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32001));
}

#[test]
fn memory_tags_rejects_invalid_tag_values() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );

    let error = server
        .memory_tags(MemoryTagsInput {
            namespace: None,
            slug: "people/alice".to_string(),
            add: Some(vec!["bad tag".to_string()]),
            remove: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}
