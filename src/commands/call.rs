use anyhow::Result;
use rusqlite::Connection;

pub async fn run(db: &Connection, tool: &str, params: Option<String>) -> Result<()> {
    todo!("call: raw MCP tool call")
}
