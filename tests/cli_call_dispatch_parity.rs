#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would add noise"
)]

//! CLI ↔ MCP surface parity: `quaid call` / `quaid pipe` route through
//! `commands::call::dispatch_tool`, which keeps its own match over tool
//! names. This test guarantees the dispatcher accepts every tool that the
//! MCP `tool_box!` registry exposes, so the CLI can never silently lag the
//! wire surface again (issue: `memory_correct` / `memory_correct_continue`
//! hit the "unknown tool" fallthrough while being listed by `tools/list`).

use quaid::commands::call::dispatch_tool;
use quaid::core::{db, inference::default_model};
use quaid::mcp::server::QuaidServer;
use serde_json::json;

#[test]
fn dispatch_tool_accepts_every_registered_mcp_tool() {
    let names = QuaidServer::registered_tool_names();
    assert!(
        names.len() >= 24,
        "expected at least the 24 known tools, got {}: {names:?}",
        names.len()
    );

    let conn = db::init(":memory:", &default_model()).expect("init in-memory db");
    let server = QuaidServer::new(conn);

    for name in &names {
        // Probe with empty params: tools either succeed or fail on params /
        // domain validation. The only unacceptable outcome is the dispatcher
        // not knowing the tool at all.
        match dispatch_tool(&server, name, json!({})) {
            Ok(_) => {}
            Err(message) => {
                assert!(
                    !message.contains("unknown tool"),
                    "tool `{name}` is exposed by the MCP registry but is not \
                     dispatchable via `quaid call`/`quaid pipe`: {message}"
                );
            }
        }
    }
}

#[test]
fn dispatch_tool_routes_correction_tools_past_unknown_tool_fallthrough() {
    let conn = db::init(":memory:", &default_model()).expect("init in-memory db");
    let server = QuaidServer::new(conn);

    // Minimal valid-shaped params; both calls fail in the domain layer
    // (page / correction session does not exist), proving the arm routed
    // into the handler instead of the "unknown tool" fallthrough.
    let correct = dispatch_tool(
        &server,
        "memory_correct",
        json!({"fact_slug": "facts/none", "correction": "value is wrong"}),
    )
    .expect_err("nonexistent fact page must error in the handler");
    assert!(!correct.contains("unknown tool"), "got: {correct}");

    let cont = dispatch_tool(
        &server,
        "memory_correct_continue",
        json!({"correction_id": "no-such-correction"}),
    )
    .expect_err("nonexistent correction session must error in the handler");
    assert!(!cont.contains("unknown tool"), "got: {cont}");
}

#[test]
fn dispatch_tool_still_rejects_truly_unknown_tools() {
    let conn = db::init(":memory:", &default_model()).expect("init in-memory db");
    let server = QuaidServer::new(conn);

    let err = dispatch_tool(&server, "memory_does_not_exist", json!({}))
        .expect_err("unknown tool must return Err");
    assert!(err.contains("unknown tool"));
}
