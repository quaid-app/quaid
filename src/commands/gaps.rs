use anyhow::Result;
use rusqlite::Connection;

pub fn run(_db: &Connection, _limit: u32, _resolved: bool, _json: bool) -> Result<()> {
    todo!("gaps: list knowledge gaps")
}
