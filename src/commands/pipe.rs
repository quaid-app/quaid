use anyhow::Result;
use rusqlite::Connection;
use serde::Deserialize;
use std::io::BufRead;

use crate::commands::call::dispatch_tool;
use crate::mcp::server::GigaBrainServer;

/// Maximum JSONL line size accepted on stdin (5 MB).
const MAX_LINE_BYTES: usize = 5_242_880;

#[derive(Deserialize)]
struct PipeCommand {
    tool: String,
    input: serde_json::Value,
}

pub fn run(db: Connection) -> Result<()> {
    let server = GigaBrainServer::new(db);
    let stdin = std::io::stdin();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let err = serde_json::json!({"error": format!("stdin read error: {e}")});
                println!("{err}");
                continue;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.len() > MAX_LINE_BYTES {
            let err = serde_json::json!({
                "error": format!(
                    "line exceeds maximum size of {MAX_LINE_BYTES} bytes; rejected"
                )
            });
            println!("{}", serde_json::to_string(&err).unwrap_or_default());
            continue;
        }

        let cmd: PipeCommand = match serde_json::from_str(trimmed) {
            Ok(c) => c,
            Err(e) => {
                let err = serde_json::json!({"error": format!("parse error: {e}")});
                println!("{err}");
                continue;
            }
        };

        match dispatch_tool(&server, &cmd.tool, cmd.input) {
            Ok(result) => {
                // Output as single-line JSON (JSONL)
                println!("{}", serde_json::to_string(&result).unwrap_or_default());
            }
            Err(e) => {
                let err = serde_json::json!({"error": e});
                println!("{}", serde_json::to_string(&err).unwrap_or_default());
            }
        }
    }

    Ok(())
}
