use anyhow::Result;
use rusqlite::Connection;

pub fn run(_db: &Connection, _slug: &str, _depth: u32, _temporal: &str, _json: bool) -> Result<()> {
    todo!("graph: N-hop graph neighbourhood")
}
