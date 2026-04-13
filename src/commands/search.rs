use anyhow::Result;
use rusqlite::Connection;

pub fn run(
    db: &Connection,
    query: &str,
    wing: Option<String>,
    limit: u32,
    json: bool,
) -> Result<()> {
    todo!("search: FTS5 full-text search")
}
