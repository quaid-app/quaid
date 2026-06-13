#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Public-API integration tests for `quaid::mcp::server` covering the
//! diagnostic tool surfaces — `memory_gap`, `memory_gaps`,
//! `memory_stats`, plus `memory_raw` input-validation. Exercises gap
//! redaction, idempotency, context length boundaries, the
//! resolved/unresolved default, the stats schema, and rejection paths
//! for invalid `memory_raw` slugs/sources/payloads.

#[path = "common/mcp_harness.rs"]
mod harness;
use harness::{create_page, extract_text, open_test_db};
use quaid::mcp::server::{
    MemoryGapInput, MemoryGapsInput, MemoryRawInput, MemoryStatsInput, QuaidServer,
};
use rmcp::model::ErrorCode;
use serde_json::json;

#[test]
fn memory_gap_with_empty_query_returns_invalid_params() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_gap(MemoryGapInput {
            query: "".to_string(),
            slug: None,
            context: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}

#[test]
fn memory_gap_duplicate_is_idempotent() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let r1 = server
        .memory_gap(MemoryGapInput {
            query: "same query".to_string(),
            slug: None,
            context: None,
        })
        .unwrap();
    let r2 = server
        .memory_gap(MemoryGapInput {
            query: "same query".to_string(),
            slug: None,
            context: None,
        })
        .unwrap();

    let id1: serde_json::Value = serde_json::from_str(&extract_text(&r1)).unwrap();
    let id2: serde_json::Value = serde_json::from_str(&extract_text(&r2)).unwrap();
    assert_eq!(id1["id"], id2["id"]);
}

#[test]
fn memory_gap_context_is_redacted_in_listings() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    server
        .memory_gap(MemoryGapInput {
            query: "sensitive query".to_string(),
            slug: None,
            context: Some("sensitive query with extra details".to_string()),
        })
        .unwrap();

    let result = server
        .memory_gaps(MemoryGapsInput {
            resolved: None,
            limit: None,
        })
        .unwrap();

    let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(parsed.len(), 1);
    let context = parsed[0]["context"].as_str().unwrap_or_default();
    assert!(context.is_empty());
}

#[test]
fn memory_gap_rejects_oversized_context() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let big_context = "x".repeat(501);
    let error = server
        .memory_gap(MemoryGapInput {
            query: "who invented quantum socks".to_string(),
            slug: None,
            context: Some(big_context),
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
    assert!(error.message.contains("context"));
}

#[test]
fn memory_gap_accepts_context_at_max_length() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let exact_context = "y".repeat(500);
    server
        .memory_gap(MemoryGapInput {
            query: "boundary test query".to_string(),
            slug: None,
            context: Some(exact_context),
        })
        .unwrap();
}

#[test]
fn memory_gap_with_slug_response_includes_page_id() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/response-gap",
        "---\ntitle: Response Gap\ntype: note\n---\ncontent\n",
    );

    let result = server
        .memory_gap(MemoryGapInput {
            query: "gap with page bound".to_string(),
            slug: Some("notes/response-gap".to_string()),
            context: None,
        })
        .unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    assert!(
        parsed["page_id"].as_i64().is_some(),
        "memory_gap with slug must return page_id in response: {parsed}"
    );
}

#[test]
fn memory_gap_without_slug_response_has_null_page_id() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let result = server
        .memory_gap(MemoryGapInput {
            query: "global gap no page".to_string(),
            slug: None,
            context: None,
        })
        .unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    assert!(
        parsed["page_id"].is_null(),
        "memory_gap without slug must return null page_id: {parsed}"
    );
}

#[test]
fn memory_gaps_returns_array_with_limit() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    for i in 0..5 {
        server
            .memory_gap(MemoryGapInput {
                query: format!("gap query {i}"),
                slug: None,
                context: None,
            })
            .unwrap();
    }

    let result = server
        .memory_gaps(MemoryGapsInput {
            resolved: None,
            limit: Some(3),
        })
        .unwrap();

    let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(parsed.len(), 3);
}

#[test]
fn memory_gaps_defaults_to_unresolved() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    server
        .memory_gap(MemoryGapInput {
            query: "unresolved gap".to_string(),
            slug: None,
            context: None,
        })
        .unwrap();

    let result = server
        .memory_gaps(MemoryGapsInput {
            resolved: None,
            limit: None,
        })
        .unwrap();

    let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(parsed.len(), 1);
    assert!(parsed[0]["resolved_at"].is_null());
}

#[test]
fn memory_stats_returns_all_expected_fields() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );

    let result = server.memory_stats(MemoryStatsInput {}).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(parsed["page_count"], 1);
    assert!(parsed["link_count"].is_number());
    assert!(parsed["assertion_count"].is_number());
    assert!(parsed["contradiction_count"].is_number());
    assert!(parsed["gap_count"].is_number());
    assert!(parsed["embedding_count"].is_number());
    assert!(parsed["active_model"].is_string());
    assert!(parsed["db_size_bytes"].is_number());
}

#[test]
fn memory_raw_with_unknown_slug_returns_not_found() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_raw(MemoryRawInput {
            namespace: None,
            slug: "nobody/ghost".to_string(),
            source: "test".to_string(),
            data: json!({"key": "value"}),
            overwrite: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32001));
}

#[test]
fn memory_raw_rejects_empty_source() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );

    let error = server
        .memory_raw(MemoryRawInput {
            namespace: None,
            slug: "people/alice".to_string(),
            source: "".to_string(),
            data: json!({}),
            overwrite: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}

#[test]
fn memory_raw_rejects_invalid_slug() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_raw(MemoryRawInput {
            namespace: None,
            slug: "Invalid/SLUG!".to_string(),
            source: "test".to_string(),
            data: json!({}),
            overwrite: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}

#[test]
fn memory_raw_rejects_array_payload() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );

    let error = server
        .memory_raw(MemoryRawInput {
            namespace: None,
            slug: "people/alice".to_string(),
            source: "crustdata".to_string(),
            data: json!([1, 2, 3]),
            overwrite: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
    assert!(error.message.contains("JSON object"));
}

#[test]
fn memory_raw_rejects_scalar_payload() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );

    for bad in [json!("string"), json!(42), json!(true), json!(null)] {
        let error = server
            .memory_raw(MemoryRawInput {
                namespace: None,
                slug: "people/alice".to_string(),
                source: "crustdata".to_string(),
                data: bad,
                overwrite: None,
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode(-32602));
    }
}

#[test]
fn memory_raw_rejects_duplicate_source_without_overwrite() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );

    server
        .memory_raw(MemoryRawInput {
            namespace: None,
            slug: "people/alice".to_string(),
            source: "crustdata".to_string(),
            data: json!({"v": 1}),
            overwrite: None,
        })
        .unwrap();

    // Second write without overwrite must fail.
    let error = server
        .memory_raw(MemoryRawInput {
            namespace: None,
            slug: "people/alice".to_string(),
            source: "crustdata".to_string(),
            data: json!({"v": 2}),
            overwrite: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32003));
    assert!(error.message.contains("overwrite=true"));
}
