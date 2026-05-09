#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Public-API integration tests for `quaid::mcp::server` covering
//! infrastructure surfaces — `get_info` capability advertising,
//! `memory_add_turn` validation paths (invalid shapes, missing writable
//! root, closed sessions, default timestamps), and
//! `memory_close_action` status validation. Tests that reach into the
//! server's private database handle remain inline.

#[path = "common/mcp_harness.rs"]
mod harness;
use harness::{create_page, extract_text, open_test_db};
use quaid::mcp::server::{
    MemoryAddTurnInput, MemoryCloseActionInput, MemoryCloseSessionInput, MemoryCollectionsInput,
    MemoryGetInput, MemoryListInput, MemoryNamespaceCreateInput, MemoryNamespaceDestroyInput,
    MemoryPutInput, MemoryQueryInput, MemorySearchInput, QuaidServer,
};
use rmcp::model::{CallToolResult, ErrorCode};
use rmcp::ServerHandler;
use serde_json::json;
use std::fs;

#[test]
fn get_info_enables_tools_capability_and_exposes_core_tool_methods() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    let info = <QuaidServer as ServerHandler>::get_info(&server);

    let _tool_methods = (
        QuaidServer::memory_get
            as fn(&QuaidServer, MemoryGetInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_add_turn
            as fn(&QuaidServer, MemoryAddTurnInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_close_session
            as fn(&QuaidServer, MemoryCloseSessionInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_close_action
            as fn(&QuaidServer, MemoryCloseActionInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_put
            as fn(&QuaidServer, MemoryPutInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_query
            as fn(&QuaidServer, MemoryQueryInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_search
            as fn(&QuaidServer, MemorySearchInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_list
            as fn(&QuaidServer, MemoryListInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_collections
            as fn(&QuaidServer, MemoryCollectionsInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_namespace_create
            as fn(&QuaidServer, MemoryNamespaceCreateInput) -> Result<CallToolResult, rmcp::Error>,
        QuaidServer::memory_namespace_destroy
            as fn(&QuaidServer, MemoryNamespaceDestroyInput) -> Result<CallToolResult, rmcp::Error>,
    );

    assert!(info.capabilities.tools.is_some());
}

#[test]
fn memory_add_turn_returns_conflict_error_for_closed_session() {
    let (dir, conn) = open_test_db();
    let conversation_dir = dir
        .path()
        .join("vault")
        .join("conversations")
        .join("2026-05-03");
    fs::create_dir_all(&conversation_dir).unwrap();
    fs::write(
        conversation_dir.join("session-closed.md"),
        concat!(
            "---\n",
            "type: conversation\n",
            "session_id: session-closed\n",
            "date: 2026-05-03\n",
            "started_at: 2026-05-03T09:14:22Z\n",
            "status: closed\n",
            "closed_at: 2026-05-03T09:15:00Z\n",
            "last_extracted_at: null\n",
            "last_extracted_turn: 1\n",
            "---\n\n",
            "## Turn 1 · user · 2026-05-03T09:14:22Z\n\n",
            "done\n"
        ),
    )
    .unwrap();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_add_turn(MemoryAddTurnInput {
            session_id: "session-closed".to_string(),
            role: "assistant".to_string(),
            content: "late reply".to_string(),
            timestamp: Some("2026-05-03T09:16:00Z".to_string()),
            metadata: None,
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32009));
    assert!(error.message.contains("ConflictError"));
}

#[test]
fn memory_add_turn_returns_config_error_for_missing_writable_root() {
    let (_dir, conn) = open_test_db();
    conn.execute("UPDATE collections SET root_path = '' WHERE id = 1", [])
        .unwrap();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_add_turn(MemoryAddTurnInput {
            session_id: "session-config".to_string(),
            role: "user".to_string(),
            content: "hello".to_string(),
            timestamp: Some("2026-05-03T09:14:22Z".to_string()),
            metadata: None,
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32002));
    assert!(error.message.contains("ConfigError"));
}

#[test]
fn memory_add_turn_uses_current_timestamp_when_omitted() {
    let (dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let result = server
        .memory_add_turn(MemoryAddTurnInput {
            session_id: "session-now".to_string(),
            role: "tool".to_string(),
            content: "ran tool".to_string(),
            timestamp: None,
            metadata: Some(json!({"tool_name": "bash"})),
            namespace: Some("alpha".to_string()),
        })
        .unwrap();

    let payload: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    assert!(payload["conversation_path"]
        .as_str()
        .unwrap()
        .starts_with("alpha/conversations/"));
    let conversation_path = payload["conversation_path"]
        .as_str()
        .unwrap()
        .split('/')
        .fold(dir.path().join("vault"), |path, segment| path.join(segment));
    let parsed = quaid::core::conversation::format::parse(&conversation_path).unwrap();
    assert_eq!(parsed.turns.len(), 1);
    assert_eq!(parsed.turns[0].role, quaid::core::types::TurnRole::Tool);
    assert!(parsed.turns[0].timestamp.ends_with('Z'));
    assert_eq!(
        parsed.turns[0].metadata.as_ref().unwrap()["tool_name"],
        "bash"
    );
}

#[test]
fn memory_add_turn_rejects_non_object_metadata() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_add_turn(MemoryAddTurnInput {
            session_id: "session-bad-metadata".to_string(),
            role: "user".to_string(),
            content: "hello".to_string(),
            timestamp: Some("2026-05-03T09:14:22Z".to_string()),
            metadata: Some(json!(["not", "an", "object"])),
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
    assert!(error.message.contains("metadata must be a JSON object"));
}

#[test]
fn memory_add_turn_rejects_invalid_timestamp_shape() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let error = server
        .memory_add_turn(MemoryAddTurnInput {
            session_id: "session-bad-time".to_string(),
            role: "user".to_string(),
            content: "hello".to_string(),
            timestamp: Some("2026-05-03 09:14:22".to_string()),
            metadata: None,
            namespace: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
    assert!(error.message.contains("invalid timestamp"));
}

#[test]
fn memory_close_action_rejects_invalid_status_values() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "actions/review-tests",
        concat!(
            "---\n",
            "title: Review tests\n",
            "type: action_item\n",
            "status: open\n",
            "---\n",
            "Review the close-action coverage.\n",
        ),
    );

    let error = server
        .memory_close_action(MemoryCloseActionInput {
            slug: "actions/review-tests".to_string(),
            status: "blocked".to_string(),
            note: None,
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode(-32602));
    assert!(error.message.contains("invalid status"));
}
