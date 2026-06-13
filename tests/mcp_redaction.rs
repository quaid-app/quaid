#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration test fixtures favour direct unwraps for readable failure output"
)]

//! Public-API integration tests for the MCP outbound-redaction surface
//! (issue #159 phase 1). Seeds a page containing an email + API key and
//! asserts:
//!   * `memory_get` / `memory_query` / `memory_search` mask secrets when
//!     `mcp.redact_outbound = patterns` (or per-call `redact: true`);
//!   * FTS5 still indexes the ORIGINALS (redaction is outbound-only);
//!   * `memory_rehydrate` reverses the session token map;
//!   * output is byte-identical to the unredacted payload when `off`.

#[path = "common/mcp_harness.rs"]
mod harness;

use harness::{create_page, extract_text, open_test_db};
use quaid::mcp::server::{
    MemoryGetInput, MemoryQueryInput, MemoryRehydrateInput, MemorySearchInput, QuaidServer,
};
use rusqlite::Connection;

const EMAIL: &str = "alice@example.com";
const API_KEY: &str = "sk-AbCdEf0123456789ZyXwVuTs";

/// Body whose first paragraph (and thus the derived summary) carries the
/// secrets, so they surface through both `compiled_truth` (memory_get) and
/// `summary` (memory_query / memory_search).
fn secret_page_body() -> String {
    format!(
        "---\ntitle: Alice\ntype: person\n---\nContact alice at {EMAIL} using key {API_KEY} for access.\n"
    )
}

fn set_config(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT OR REPLACE INTO config(key, value) VALUES (?1, ?2)",
        rusqlite::params![key, value],
    )
    .unwrap();
}

#[test]
fn memory_get_masks_secrets_when_redaction_enabled() {
    let (_dir, conn) = open_test_db();
    set_config(&conn, "mcp.redact_outbound", "patterns");
    let server = QuaidServer::new(conn);
    create_page(&server, "people/alice", &secret_page_body());

    let result = server
        .memory_get(MemoryGetInput {
            slug: "people/alice".to_string(),
            redact: None, // falls back to config default = patterns

            namespace: None,
        })
        .unwrap();
    let text = extract_text(&result);

    assert!(
        !text.contains(EMAIL),
        "email leaked through memory_get: {text}"
    );
    assert!(
        !text.contains(API_KEY),
        "api key leaked through memory_get: {text}"
    );
    assert!(text.contains("<EMAIL_1>"), "email token missing: {text}");
    assert!(text.contains("<SECRET_1>"), "secret token missing: {text}");
}

#[test]
fn memory_get_off_is_byte_identical_to_unredacted() {
    // Two servers over equivalent DBs: one with redaction off (default),
    // one explicitly forced off per-call. Both must equal the raw payload.
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(&server, "people/alice", &secret_page_body());

    let default_off = extract_text(
        &server
            .memory_get(MemoryGetInput {
                slug: "people/alice".to_string(),
                redact: None,

                namespace: None,
            })
            .unwrap(),
    );
    let forced_off = extract_text(
        &server
            .memory_get(MemoryGetInput {
                slug: "people/alice".to_string(),
                redact: Some(false),

                namespace: None,
            })
            .unwrap(),
    );

    assert_eq!(
        default_off, forced_off,
        "config-off and per-call-off diverge"
    );
    assert!(
        default_off.contains(EMAIL),
        "off mode must pass secrets through"
    );
    assert!(
        default_off.contains(API_KEY),
        "off mode must pass secrets through"
    );
}

#[test]
fn per_call_redact_true_overrides_config_off() {
    let (_dir, conn) = open_test_db();
    // config default off
    let server = QuaidServer::new(conn);
    create_page(&server, "people/alice", &secret_page_body());

    let masked = extract_text(
        &server
            .memory_get(MemoryGetInput {
                slug: "people/alice".to_string(),
                redact: Some(true),

                namespace: None,
            })
            .unwrap(),
    );
    assert!(!masked.contains(EMAIL), "per-call redact ignored: {masked}");
    assert!(masked.contains("<EMAIL_1>"));
}

#[test]
fn memory_search_masks_summary_but_fts_indexes_originals() {
    let (_dir, conn) = open_test_db();
    set_config(&conn, "mcp.redact_outbound", "patterns");
    let server = QuaidServer::new(conn);
    create_page(&server, "people/alice", &secret_page_body());

    // FTS proof: search a fragment that exists ONLY inside the email. If the
    // index held the original, the page is found even though the returned
    // summary is masked.
    let result = server
        .memory_search(MemorySearchInput {
            query: "example.com".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: Some(10),
            include_superseded: None,
            redact: None,

            relevance_floor: None,
            max_chunks_per_doc: None,
        })
        .unwrap();
    let text = extract_text(&result);

    assert!(
        text.contains("people/alice"),
        "FTS did not index original email: {text}"
    );
    assert!(
        !text.contains(EMAIL),
        "email leaked through search summary: {text}"
    );
    assert!(
        text.contains("<EMAIL_1>"),
        "summary email token missing: {text}"
    );
}

#[test]
fn memory_query_masks_summary_when_enabled() {
    let (_dir, conn) = open_test_db();
    set_config(&conn, "mcp.redact_outbound", "patterns");
    let server = QuaidServer::new(conn);
    create_page(&server, "people/alice", &secret_page_body());

    let result = server
        .memory_query(MemoryQueryInput {
            query: "alice contact access".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: Some(10),
            depth: None,
            include_superseded: None,
            redact: None,

            hops: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
        })
        .unwrap();
    let text = extract_text(&result);

    assert!(
        text.contains("people/alice"),
        "query missed the seeded page: {text}"
    );
    assert!(
        !text.contains(EMAIL),
        "email leaked through query summary: {text}"
    );
    assert!(
        !text.contains(API_KEY),
        "api key leaked through query summary: {text}"
    );
}

#[test]
fn rehydrate_reverses_session_tokens() {
    let (_dir, conn) = open_test_db();
    set_config(&conn, "mcp.redact_outbound", "patterns");
    let server = QuaidServer::new(conn);
    create_page(&server, "people/alice", &secret_page_body());

    // Populate the session token map via a redacted read.
    let masked = extract_text(
        &server
            .memory_get(MemoryGetInput {
                slug: "people/alice".to_string(),
                redact: None,

                namespace: None,
            })
            .unwrap(),
    );
    assert!(masked.contains("<EMAIL_1>"));

    let restored = extract_text(
        &server
            .memory_rehydrate(MemoryRehydrateInput { text: masked })
            .unwrap(),
    );
    let restored_json: serde_json::Value = serde_json::from_str(&restored).unwrap();
    let inner = restored_json
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap();

    assert!(inner.contains(EMAIL), "email not rehydrated: {inner}");
    assert!(inner.contains(API_KEY), "api key not rehydrated: {inner}");
}

#[test]
fn rehydrate_partial_token_string_round_trips() {
    let (_dir, conn) = open_test_db();
    set_config(&conn, "mcp.redact_outbound", "patterns");
    let server = QuaidServer::new(conn);
    create_page(&server, "people/alice", &secret_page_body());

    // Establish the map.
    let _ = server
        .memory_get(MemoryGetInput {
            slug: "people/alice".to_string(),
            redact: None,

            namespace: None,
        })
        .unwrap();

    // Rehydrate a hand-written snippet that mixes a known token with prose.
    let restored = extract_text(
        &server
            .memory_rehydrate(MemoryRehydrateInput {
                text: "the address is <EMAIL_1> per the record".to_string(),
            })
            .unwrap(),
    );
    let inner: serde_json::Value = serde_json::from_str(&restored).unwrap();
    assert_eq!(
        inner
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap(),
        format!("the address is {EMAIL} per the record")
    );
}
