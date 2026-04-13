use anyhow::Result;
use rusqlite::Connection;

pub async fn run(
    _db: &Connection,
    _query: &str,
    _depth: &str,
    _token_budget: u32,
    _wing: Option<String>,
    _json: bool,
) -> Result<()> {
    todo!("query: hybrid semantic query with progressive retrieval")
}
