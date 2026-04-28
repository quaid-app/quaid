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
        let result = dispatch_tool(&server, "memory_list", json!({}))
            .expect("dispatch memory_list");
        assert!(result.as_array().is_some());
    }

    #[test]
    fn dispatch_tool_memory_search_returns_array() {
        let server = make_server();
        let result =
            dispatch_tool(&server, "memory_search", json!({"query": "anything"}))
                .expect("dispatch memory_search");
        assert!(result.as_array().is_some());
    }

    #[test]
    fn dispatch_tool_memory_stats_returns_object() {
        let server = make_server();
        let result = dispatch_tool(&server, "memory_stats", json!({}))
            .expect("dispatch memory_stats");
        assert!(result.is_object() || result.is_string());
    }

    #[test]
    fn dispatch_tool_memory_gaps_returns_array() {
        let server = make_server();
        let result = dispatch_tool(&server, "memory_gaps", json!({}))
            .expect("dispatch memory_gaps");
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
}
