use anyhow::Result;
use rusqlite::Connection;

pub fn run(_db: &Connection, _slug: &str, _json: bool) -> Result<()> {
    todo!("get: read page by slug")
}
