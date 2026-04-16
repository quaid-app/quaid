use anyhow::{anyhow, Result};
use rusqlite::Connection;
use serde_json::Value;

use crate::mcp::server::*;

/// Dispatch a tool name + JSON params to the MCP handler, return JSON result.
pub fn dispatch_tool(server: &GigaBrainServer, tool: &str, params: Value) -> Result<Value, String> {
    let result = match tool {
        "brain_get" => {
            let input: BrainGetInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_get(input).map_err(|e| e.message.to_string())
        }
        "brain_put" => {
            let input: BrainPutInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_put(input).map_err(|e| e.message.to_string())
        }
        "brain_query" => {
            let input: BrainQueryInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_query(input).map_err(|e| e.message.to_string())
        }
        "brain_search" => {
            let input: BrainSearchInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .brain_search(input)
                .map_err(|e| e.message.to_string())
        }
        "brain_list" => {
            let input: BrainListInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_list(input).map_err(|e| e.message.to_string())
        }
        "brain_link" => {
            let input: BrainLinkInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_link(input).map_err(|e| e.message.to_string())
        }
        "brain_link_close" => {
            let input: BrainLinkCloseInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .brain_link_close(input)
                .map_err(|e| e.message.to_string())
        }
        "brain_backlinks" => {
            let input: BrainBacklinksInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .brain_backlinks(input)
                .map_err(|e| e.message.to_string())
        }
        "brain_graph" => {
            let input: BrainGraphInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_graph(input).map_err(|e| e.message.to_string())
        }
        "brain_check" => {
            let input: BrainCheckInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_check(input).map_err(|e| e.message.to_string())
        }
        "brain_timeline" => {
            let input: BrainTimelineInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server
                .brain_timeline(input)
                .map_err(|e| e.message.to_string())
        }
        "brain_tags" => {
            let input: BrainTagsInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_tags(input).map_err(|e| e.message.to_string())
        }
        "brain_gap" => {
            let input: BrainGapInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_gap(input).map_err(|e| e.message.to_string())
        }
        "brain_gaps" => {
            let input: BrainGapsInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_gaps(input).map_err(|e| e.message.to_string())
        }
        "brain_stats" => {
            let input: BrainStatsInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_stats(input).map_err(|e| e.message.to_string())
        }
        "brain_raw" => {
            let input: BrainRawInput =
                serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
            server.brain_raw(input).map_err(|e| e.message.to_string())
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

    let server = GigaBrainServer::new(db);

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
