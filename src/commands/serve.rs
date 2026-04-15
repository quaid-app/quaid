use anyhow::Result;
use rusqlite::Connection;

pub async fn run(db: Connection) -> Result<()> {
    crate::mcp::server::run(db).await
}
