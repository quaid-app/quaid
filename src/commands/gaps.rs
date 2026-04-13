use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection, limit: u32, resolved: bool, json: bool) -> Result<()> {
    todo!("gaps: list knowledge gaps")
}
