use anyhow::Result;
use rusqlite::Connection;

pub fn run(
    _db: &Connection,
    _query: &str,
    _wing: Option<String>,
    _limit: u32,
    _json: bool,
) -> Result<()> {
    todo!("search: FTS5 full-text search")
}
