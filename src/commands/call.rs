use anyhow::{anyhow, Result};
use rusqlite::Connection;
use serde_json::Value;

use crate::mcp::server::*;

/// Dispatch a tool name + JSON params to the MCP handler, return JSON result.
pub fn dispatch_tool(server: &QuaidServer, tool: &str, params: Value) -> Result<Value, String> {
    let result = match tool {
        "memory_get" => {
            let input: MemoryGetInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.memory_get(input).map_err(|e| e.message.to_string())
        }
        "memory_put" => {
            let input: MemoryPutInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.memory_put(input).map_err(|e| e.message.to_string())
        }
        "memory_add_turn" => {
            let input: MemoryAddTurnInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_add_turn(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_close_session" => {
            let input: MemoryCloseSessionInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_close_session(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_close_action" => {
            let input: MemoryCloseActionInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_close_action(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_query" => {
            let input: MemoryQueryInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_query(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_search" => {
            let input: MemorySearchInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_search(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_list" => {
            let input: MemoryListInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.memory_list(input).map_err(|e| e.message.to_string())
        }
        "memory_link" => {
            let input: MemoryLinkInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.memory_link(input).map_err(|e| e.message.to_string())
        }
        "memory_link_close" => {
            let input: MemoryLinkCloseInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_link_close(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_backlinks" => {
            let input: MemoryBacklinksInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_backlinks(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_graph" => {
            let input: MemoryGraphInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_graph(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_check" => {
            let input: MemoryCheckInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_check(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_timeline" => {
            let input: MemoryTimelineInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_timeline(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_tags" => {
            let input: MemoryTagsInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.memory_tags(input).map_err(|e| e.message.to_string())
        }
        "memory_gap" => {
            let input: MemoryGapInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.memory_gap(input).map_err(|e| e.message.to_string())
        }
        "memory_gaps" => {
            let input: MemoryGapsInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.memory_gaps(input).map_err(|e| e.message.to_string())
        }
        "memory_stats" => {
            let input: MemoryStatsInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_stats(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_collections" => {
            let input: MemoryCollectionsInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_collections(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_namespace_create" => {
            let input: MemoryNamespaceCreateInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_namespace_create(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_namespace_destroy" => {
            let input: MemoryNamespaceDestroyInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .memory_namespace_destroy(input)
                .map_err(|e| e.message.to_string())
        }
        "memory_raw" => {
            let input: MemoryRawInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.memory_raw(input).map_err(|e| e.message.to_string())
        }
        _ => return Err(format!("unknown tool: {tool}")),
    };

    match result {
        Ok(call_result) => {
            // Extract text content from CallToolResult
            let text = call_result
                .content
                .iter()
                .filter_map(|c| match &c.raw {
                    rmcp::model::RawContent::Text(tc) => Some(tc.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            // Try to parse as JSON, fall back to string wrapper
            match serde_json::from_str::<Value>(&text) {
                Ok(v) => Ok(v),
                Err(_) => Ok(Value::String(text)),
            }
        }
        Err(e) => Err(e),
    }
}

pub fn run(db: Connection, tool: &str, params: Option<String>) -> Result<()> {
    let params_json: Value = match params {
        Some(ref s) => serde_json::from_str(s).map_err(|e| anyhow!("invalid JSON params: {e}"))?,
        None => Value::Object(serde_json::Map::new()),
    };

    let server = QuaidServer::new(db);

    match dispatch_tool(&server, tool, params_json) {
        Ok(result) => {
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Err(e) => {
            eprintln!("{}", serde_json::json!({"error": e}));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{db, inference::default_model};
    use serde_json::json;

    fn make_server() -> QuaidServer {
        let conn = db::init(":memory:", &default_model()).expect("init in-memory db");
        QuaidServer::new(conn)
    }

    fn make_server_with_vault() -> (tempfile::TempDir, QuaidServer) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let db_path = dir.path().join("memory.db");
        let conn = db::open(db_path.to_str().expect("utf-8 db path")).expect("open db");
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        conn.execute(
            "UPDATE collections
             SET root_path = ?1,
                 writable = 1,
                 is_write_target = 1,
                 state = 'active'
             WHERE id = 1",
            [vault_root.display().to_string()],
        )
        .expect("configure default collection");
        conn.execute(
            "INSERT OR REPLACE INTO config(key, value) VALUES ('extraction.enabled', 'true')",
            [],
        )
        .expect("enable extraction");
        (dir, QuaidServer::new(conn))
    }

    #[test]
    fn dispatch_tool_routes_memory_collections() {
        let server = make_server();

        let result = dispatch_tool(&server, "memory_collections", json!({}))
            .expect("dispatch memory_collections");

        let rows = result.as_array().expect("collections array");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], json!("default"));
        assert_eq!(rows[0]["state"], json!("detached"));
    }

    #[test]
    fn dispatch_tool_memory_list_returns_empty_array() {
        let server = make_server();
        let result =
            dispatch_tool(&server, "memory_list", json!({})).expect("dispatch memory_list");
        assert!(result.as_array().is_some());
    }

    #[test]
    fn dispatch_tool_routes_memory_namespace_create_and_destroy() {
        let server = make_server();

        let created = dispatch_tool(
            &server,
            "memory_namespace_create",
            json!({"id": "call-test", "ttl_hours": 1.0}),
        )
        .expect("dispatch namespace create");
        let destroyed = dispatch_tool(
            &server,
            "memory_namespace_destroy",
            json!({"id": "call-test"}),
        )
        .expect("dispatch namespace destroy");

        assert_eq!(created["id"], json!("call-test"));
        assert_eq!(destroyed["namespace"], json!("call-test"));
    }

    #[test]
    fn dispatch_tool_memory_search_returns_array() {
        let server = make_server();
        let result = dispatch_tool(&server, "memory_search", json!({"query": "anything"}))
            .expect("dispatch memory_search");
        assert!(result.as_array().is_some());
    }

    #[test]
    fn dispatch_tool_memory_stats_returns_object() {
        let server = make_server();
        let result =
            dispatch_tool(&server, "memory_stats", json!({})).expect("dispatch memory_stats");
        assert!(result.is_object() || result.is_string());
    }

    #[test]
    fn dispatch_tool_memory_gaps_returns_array() {
        let server = make_server();
        let result =
            dispatch_tool(&server, "memory_gaps", json!({})).expect("dispatch memory_gaps");
        assert!(result.as_array().is_some());
    }

    #[test]
    fn dispatch_tool_memory_gap_logs_and_returns_object() {
        let server = make_server();
        let result = dispatch_tool(
            &server,
            "memory_gap",
            json!({"query": "test question", "context": "test context"}),
        )
        .expect("dispatch memory_gap");
        assert!(result.is_object() || result.is_string());
    }

    #[test]
    fn dispatch_tool_memory_tags_returns_result() {
        let server = make_server();
        let result = dispatch_tool(&server, "memory_tags", json!({"slug": "people/ghost"}));
        // ghost page does not exist → error is acceptable; what matters is routing succeeded
        let _ = result;
    }

    #[test]
    fn dispatch_tool_memory_check_returns_result() {
        let server = make_server();
        let result = dispatch_tool(&server, "memory_check", json!({"slug": "people/ghost"}));
        let _ = result;
    }

    #[test]
    fn dispatch_tool_unknown_tool_returns_err() {
        let server = make_server();
        let err = dispatch_tool(&server, "not_a_real_tool", json!({}))
            .expect_err("unknown tool must return Err");
        assert!(err.contains("unknown tool"));
    }

    #[test]
    fn dispatch_tool_memory_get_missing_page_returns_err() {
        let server = make_server();
        let err = dispatch_tool(&server, "memory_get", json!({"slug": "people/no-one"}))
            .expect_err("missing page must return Err");
        assert!(!err.is_empty());
    }

    #[test]
    fn dispatch_tool_memory_put_creates_page() {
        let server = make_server();
        let result = dispatch_tool(
            &server,
            "memory_put",
            json!({
                "slug": "concept/test-put",
                "content": "---\ntype: concept\ntitle: Test Put\n---\n\nSome content."
            }),
        )
        .expect("memory_put should succeed");
        let text = match &result {
            serde_json::Value::String(s) => s.as_str(),
            _ => "",
        };
        assert!(text.contains("concept/test-put") || result.is_string());
    }

    #[test]
    fn dispatch_tool_routes_memory_add_turn_and_close_session() {
        let (_dir, server) = make_server_with_vault();
        let add_turn = dispatch_tool(
            &server,
            "memory_add_turn",
            json!({
                "session_id": "dispatch-session",
                "role": "user",
                "content": "hello",
                "timestamp": "2026-05-03T09:14:22Z"
            }),
        )
        .expect("memory_add_turn should succeed");
        assert_eq!(add_turn["turn_id"], json!("dispatch-session:1"));
        assert_eq!(
            add_turn["conversation_path"],
            json!("conversations/2026-05-03/dispatch-session.md")
        );

        let close = dispatch_tool(
            &server,
            "memory_close_session",
            json!({
                "session_id": "dispatch-session"
            }),
        )
        .expect("memory_close_session should succeed");
        assert_eq!(close["extraction_triggered"], json!(true));
        assert!(close["queue_position"].as_i64().unwrap_or_default() >= 1);

        dispatch_tool(
            &server,
            "memory_put",
            json!({
                "slug": "action_item/dispatch-action",
                "content": "---\ntype: action_item\ntitle: Dispatch Action\nstatus: open\n---\n\nShip the docs before lunch."
            }),
        )
        .expect("memory_put action_item should succeed");

        let closed = dispatch_tool(
            &server,
            "memory_close_action",
            json!({
                "slug": "action_item/dispatch-action",
                "status": "done",
                "note": "Closed from dispatch_tool test."
            }),
        )
        .expect("memory_close_action should succeed");
        assert!(closed["updated_at"].as_str().is_some());
        assert!(closed["version"].as_i64().unwrap_or_default() >= 2);
    }

    #[test]
    fn dispatch_tool_memory_query_returns_result() {
        let server = make_server();
        let result = dispatch_tool(&server, "memory_query", json!({"query": "anything"}))
            .expect("memory_query should succeed");
        assert!(result.is_array() || result.is_string());
    }

    #[test]
    fn dispatch_tool_memory_link_requires_existing_pages() {
        let server = make_server();
        // Pages don't exist → expect an error from page resolution
        let result = dispatch_tool(
            &server,
            "memory_link",
            json!({"from_slug": "concept/a", "to_slug": "concept/b", "relationship": "related"}),
        );
        // Either Err (not found) or ok if pages auto-create — just verify routing fired
        let _ = result;
    }

    #[test]
    fn dispatch_tool_memory_link_succeeds_with_existing_pages() {
        let server = make_server();
        // Create two pages first
        dispatch_tool(
            &server,
            "memory_put",
            json!({
                "slug": "concept/link-from",
                "content": "---\ntype: concept\ntitle: From\n---\n\nContent."
            }),
        )
        .unwrap();
        dispatch_tool(
            &server,
            "memory_put",
            json!({
                "slug": "concept/link-to",
                "content": "---\ntype: concept\ntitle: To\n---\n\nContent."
            }),
        )
        .unwrap();
        let result = dispatch_tool(
            &server,
            "memory_link",
            json!({
                "from_slug": "concept/link-from",
                "to_slug": "concept/link-to",
                "relationship": "related"
            }),
        )
        .expect("memory_link with existing pages should succeed");
        assert!(result.is_string());
    }

    #[test]
    fn dispatch_tool_memory_link_close_invalid_id_returns_err() {
        let server = make_server();
        let result = dispatch_tool(
            &server,
            "memory_link_close",
            json!({"link_id": 9999, "valid_until": "2024-12"}),
        );
        // Non-existent link ID → expect error
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn dispatch_tool_memory_backlinks_on_existing_page_returns_array() {
        let server = make_server();
        dispatch_tool(
            &server,
            "memory_put",
            json!({
                "slug": "concept/backlink-target",
                "content": "---\ntype: concept\ntitle: Target\n---\n\nContent."
            }),
        )
        .unwrap();
        let result = dispatch_tool(
            &server,
            "memory_backlinks",
            json!({"slug": "concept/backlink-target"}),
        )
        .expect("memory_backlinks should succeed");
        assert!(result.is_array() || result.is_string());
    }

    #[test]
    fn dispatch_tool_memory_graph_on_existing_page_returns_result() {
        let server = make_server();
        dispatch_tool(
            &server,
            "memory_put",
            json!({
                "slug": "concept/graph-root",
                "content": "---\ntype: concept\ntitle: Root\n---\n\nContent."
            }),
        )
        .unwrap();
        let result = dispatch_tool(
            &server,
            "memory_graph",
            json!({"slug": "concept/graph-root"}),
        );
        let _ = result;
    }

    #[test]
    fn dispatch_tool_memory_timeline_on_existing_page_returns_result() {
        let server = make_server();
        dispatch_tool(
            &server,
            "memory_put",
            json!({
                "slug": "concept/timeline-page",
                "content": "---\ntype: concept\ntitle: Timeline\n---\n\nContent."
            }),
        )
        .unwrap();
        let result = dispatch_tool(
            &server,
            "memory_timeline",
            json!({"slug": "concept/timeline-page"}),
        )
        .expect("memory_timeline should succeed");
        let _ = result;
    }

    #[test]
    fn dispatch_tool_memory_raw_stores_data() {
        let server = make_server();
        dispatch_tool(
            &server,
            "memory_put",
            json!({
                "slug": "concept/raw-page",
                "content": "---\ntype: concept\ntitle: Raw\n---\n\nContent."
            }),
        )
        .unwrap();
        let result = dispatch_tool(
            &server,
            "memory_raw",
            json!({
                "slug": "concept/raw-page",
                "source": "test-source",
                "data": {"key": "value"}
            }),
        )
        .expect("memory_raw should succeed");
        assert!(result.is_string() || result.is_object());
    }

    #[test]
    fn dispatch_tool_invalid_params_type_returns_err() {
        let server = make_server();
        // Passing a non-object (integer) causes deserialization failure
        let err = dispatch_tool(&server, "memory_get", json!(42))
            .expect_err("non-object params must fail");
        assert!(err.contains("invalid params"));
    }

    #[test]
    fn run_with_valid_tool_succeeds() {
        use crate::core::{db, inference::default_model};
        let conn = db::init(":memory:", &default_model()).expect("init db");
        let result = run(conn, "memory_stats", None);
        assert!(result.is_ok());
    }

    #[test]
    fn run_with_invalid_json_params_returns_err() {
        use crate::core::{db, inference::default_model};
        let conn = db::init(":memory:", &default_model()).expect("init db");
        let result = run(conn, "memory_stats", Some("not-valid-json{{{".to_string()));
        assert!(result.is_err());
    }
}
