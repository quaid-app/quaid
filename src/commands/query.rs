use anyhow::Result;
use rusqlite::Connection;

pub async fn run(db: &Connection, query: &str, depth: &str, token_budget: u32, wing: Option<String>, json: bool) -> Result<()> {
    todo!("query: hybrid semantic query with progressive retrieval")
}
