use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection, slug: &str, depth: u32, temporal: &str, json: bool) -> Result<()> {
    todo!("graph: N-hop graph neighbourhood")
}
