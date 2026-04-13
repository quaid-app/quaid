use anyhow::Result;
use rusqlite::Connection;

pub async fn run(_db: &Connection, _tool: &str, _params: Option<String>) -> Result<()> {
    todo!("call: raw MCP tool call")
}
