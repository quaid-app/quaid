#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Public-API integration tests for `quaid::mcp::server` covering the
//! `memory_get` and `memory_put` MCP tool surfaces — slug validation,
//! ambiguous-slug payloads, optimistic concurrency conflicts, oversized/
//! empty-slug rejection, frontmatter rendering, and the `put_from_string`
//! variant guard. Tests that reach into the server's private database
//! handle remain inline in `src/mcp/server.rs::tests`.

#[path = "common/mcp_harness.rs"]
mod harness;
use harness::{
    create_page, create_page_in_collection, extract_text, insert_collection, open_test_db,
};
use quaid::mcp::server::{MemoryGetInput, MemoryPutInput, QuaidServer};
use rmcp::model::ErrorCode;
use serde_json::json;

#[test]
fn memory_get_returns_not_found_error_code_for_missing_slug() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_get(MemoryGetInput {
            slug: "definitely-does-not-exist".to_string(),
            redact: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32001));
}

#[test]
fn memory_get_rejects_invalid_slug() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_get(MemoryGetInput {
            slug: "Invalid/SLUG!".to_string(),
            redact: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}

#[test]
fn memory_get_returns_structured_ambiguity_payload_for_colliding_bare_slug() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "memory", false);
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nDefault Alice\n",
    );
    create_page_in_collection(
        &server,
        "memory",
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nMemory Alice\n",
    );

    let error = server
        .memory_get(MemoryGetInput {
            slug: "people/alice".to_string(),
            redact: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32002));
    let data = error.data.unwrap();
    assert_eq!(data["code"], "ambiguous_slug");
    let mut candidates = data["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    candidates.sort();
    assert_eq!(
        candidates,
        vec![
            "default::people/alice".to_string(),
            "memory::people/alice".to_string()
        ]
    );
}

#[test]
fn memory_get_explicit_collection_slug_reads_resolved_page_when_slug_collides() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "memory", false);
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nDefault Alice\n",
    );
    create_page_in_collection(
        &server,
        "memory",
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nMemory Alice\n",
    );

    let result = server
        .memory_get(MemoryGetInput {
            slug: "memory::people/alice".to_string(),
            redact: None,
        })
        .unwrap();

    let payload: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(payload["slug"], "memory::people/alice");
    assert_eq!(payload["compiled_truth"], "Memory Alice");
}

#[test]
fn memory_get_renders_persisted_memory_id_after_update_omits_frontmatter_uuid() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    server
        .memory_put(MemoryPutInput {
            slug: "notes/uuid".to_string(),
            content: "---\nquaid_id: 01969f11-9448-7d79-8d3f-c68f54761234\ntitle: UUID\ntype: note\n---\nOriginal\n".to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap();
    server
        .memory_put(MemoryPutInput {
            slug: "notes/uuid".to_string(),
            content: "---\ntitle: UUID\ntype: note\n---\nUpdated\n".to_string(),
            expected_version: Some(1),
            namespace: None,
        })
        .unwrap();

    let result = server
        .memory_get(MemoryGetInput {
            slug: "notes/uuid".to_string(),
            redact: None,
        })
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();

    assert_eq!(
        payload["frontmatter"]["quaid_id"],
        "01969f11-9448-7d79-8d3f-c68f54761234"
    );
    assert_eq!(payload["slug"], "default::notes/uuid");
    assert_eq!(payload["compiled_truth"], "Updated");
}

#[test]
fn memory_put_returns_occ_conflict_error_with_current_version_for_stale_write() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    server
        .memory_put(MemoryPutInput {
            slug: "notes/test".to_string(),
            content: "---\ntitle: Test\ntype: note\n---\nInitial content\n".to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap();

    let error = server
        .memory_put(MemoryPutInput {
            slug: "notes/test".to_string(),
            content: "---\ntitle: Test\ntype: note\n---\nUpdated content\n".to_string(),
            expected_version: Some(0),
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32009));
    assert_eq!(error.data, Some(json!({ "current_version": 1 })));
}

#[test]
fn memory_put_rejects_update_without_expected_version() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    server
        .memory_put(MemoryPutInput {
            slug: "notes/occ".to_string(),
            content: "---\ntitle: Test\ntype: note\n---\nInitial\n".to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap();

    let error = server
        .memory_put(MemoryPutInput {
            slug: "notes/occ".to_string(),
            content: "---\ntitle: Test\ntype: note\n---\nSneaky overwrite\n".to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32009));
    assert_eq!(error.data, Some(json!({ "current_version": 1 })));
}

#[test]
fn memory_put_rejects_oversized_content() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let large_content = "x".repeat(1_048_577);
    let error = server
        .memory_put(MemoryPutInput {
            slug: "test/large".to_string(),
            content: large_content,
            expected_version: None,
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}

#[test]
fn memory_put_rejects_empty_slug() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_put(MemoryPutInput {
            slug: "".to_string(),
            content: "content".to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
}

#[test]
fn memory_put_rejects_create_with_expected_version_when_page_does_not_exist() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    // Page does not exist; supplying expected_version is a client bug — reject as OCC conflict.
    let error = server
        .memory_put(MemoryPutInput {
            slug: "notes/ghost".to_string(),
            content: "---\ntitle: Ghost\ntype: note\n---\nContent\n".to_string(),
            expected_version: Some(3),
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32009));
    assert_eq!(error.data, Some(json!({ "current_version": null })));
}

#[test]
fn memory_put_does_not_call_printing_put_from_string_variant() {
    // memory_put lives in src/mcp/tools/pages.rs after the
    // decompose-mcp-server-module split.
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("mcp")
            .join("tools")
            .join("pages.rs"),
    )
    .unwrap();
    // Locate the memory_put function body.
    let fn_start = source
        .find("pub fn memory_put(")
        .expect("memory_put fn present");
    // Find the closing brace of memory_put by looking at the next tool-decorated fn.
    let fn_body_end = source[fn_start..]
        .find("\n    #[tool(")
        .map(|offset| fn_start + offset)
        .expect("next tool fn after memory_put");
    let fn_body = &source[fn_start..fn_body_end];

    // The body must NOT contain the bare `put_from_string(` call (the printing variant).
    assert!(
        !fn_body.contains("put_from_string("),
        "memory_put must not call the printing put_from_string variant; \
         use put_from_string_quiet or put_from_string_status instead"
    );
    // The body MUST use a non-printing variant.
    assert!(
        fn_body.contains("put_from_string_quiet(")
            || fn_body.contains("put_from_string_quiet_with_namespace(")
            || fn_body.contains("put_from_string_status("),
        "memory_put must call put_from_string_quiet or put_from_string_status"
    );
}
