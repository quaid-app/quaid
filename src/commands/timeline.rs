use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection, slug: &str, limit: u32, json: bool) -> Result<()> {
    todo!("timeline: show timeline entries for page")
}
