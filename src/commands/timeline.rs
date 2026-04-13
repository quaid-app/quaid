use anyhow::Result;
use rusqlite::Connection;

pub fn run(_db: &Connection, _slug: &str, _limit: u32, _json: bool) -> Result<()> {
    todo!("timeline: show timeline entries for page")
}
