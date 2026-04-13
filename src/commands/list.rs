use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection, wing: Option<String>, page_type: Option<String>, limit: u32, json: bool) -> Result<()> {
    todo!("list: list pages with filters")
}
