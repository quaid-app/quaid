use anyhow::Result;
use rusqlite::Connection;

pub fn run(
    _db: &Connection,
    _wing: Option<String>,
    _page_type: Option<String>,
    _limit: u32,
    _json: bool,
) -> Result<()> {
    todo!("list: list pages with filters")
}
