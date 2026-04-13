use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection, path: &str, validate_only: bool) -> Result<()> {
    todo!("import: import markdown directory")
}
