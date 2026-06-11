#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would add noise"
)]

//! Conflict-message unification: every `-32009` conflict surfaced by the
//! MCP layer and every optimistic-concurrency failure raised by the write
//! path must carry the single canonical `ConflictError: ` prefix (the
//! legacy spellings were `conflict: `, `Conflict: `, and a smuggled
//! `rusqlite::Error::InvalidParameterName`).

#[path = "common/mcp_harness.rs"]
mod harness;

use harness::{create_page, open_test_db};
use quaid::commands::put::put_from_string;
use quaid::core::{db, inference::default_model};
use quaid::mcp::server::{MemoryPutInput, QuaidServer};
use rmcp::model::ErrorCode;

const CONTENT: &str = "---\ntitle: Conflict Fixture\ntype: concept\n---\nBody.\n";

#[test]
fn memory_put_existing_page_without_expected_version_uses_canonical_prefix() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(&server, "notes/conflict-a", CONTENT);

    let error = server
        .memory_put(MemoryPutInput {
            slug: "notes/conflict-a".to_string(),
            content: CONTENT.to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32009));
    assert!(
        error.message.starts_with("ConflictError: "),
        "got: {}",
        error.message
    );
}

#[test]
fn memory_put_ghost_expected_version_uses_canonical_prefix() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_put(MemoryPutInput {
            slug: "notes/conflict-ghost".to_string(),
            content: CONTENT.to_string(),
            expected_version: Some(3),
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32009));
    assert!(
        error.message.starts_with("ConflictError: "),
        "got: {}",
        error.message
    );
}

#[test]
fn memory_put_stale_expected_version_uses_canonical_prefix() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(&server, "notes/conflict-stale", CONTENT);

    let error = server
        .memory_put(MemoryPutInput {
            slug: "notes/conflict-stale".to_string(),
            content: CONTENT.to_string(),
            expected_version: Some(7),
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32009));
    assert!(
        error.message.starts_with("ConflictError: "),
        "got: {}",
        error.message
    );
}

/// The CLI write path (file-backed vault) raises its OCC failure through
/// `vault_sync` — the message must carry the canonical prefix and still
/// name the current version for caller re-fetch.
#[test]
fn cli_put_stale_expected_version_uses_canonical_prefix() {
    let (_dir, conn) = open_test_db();
    put_from_string(&conn, "notes/occ", CONTENT, None).unwrap();
    conn.execute("UPDATE pages SET version = 2 WHERE slug = 'notes/occ'", [])
        .unwrap();

    let error = put_from_string(&conn, "notes/occ", CONTENT, Some(1)).unwrap_err();
    let message = error.to_string();

    assert!(message.starts_with("ConflictError: "), "got: {message}");
    assert!(message.contains("current version: 2"), "got: {message}");
    assert!(
        !message.contains("Invalid parameter name"),
        "rusqlite smuggling resurfaced: {message}"
    );
}

/// The in-memory write path skips the vault precondition check and loses
/// the compare-and-swap inside `stage_page_record` — formerly the
/// `InvalidParameterName` smuggling site. It must now surface the typed
/// `OccError` with the canonical prefix.
#[test]
fn in_memory_put_cas_failure_uses_typed_occ_error() {
    let conn = db::init(":memory:", &default_model()).unwrap();
    put_from_string(&conn, "notes/occ-mem", CONTENT, None).unwrap();
    conn.execute(
        "UPDATE pages SET version = 2 WHERE slug = 'notes/occ-mem'",
        [],
    )
    .unwrap();

    let error = put_from_string(&conn, "notes/occ-mem", CONTENT, Some(1)).unwrap_err();
    let message = error.to_string();

    assert!(
        message.starts_with("ConflictError: page updated elsewhere"),
        "got: {message}"
    );
    assert!(message.contains("current version: 2"), "got: {message}");
}
