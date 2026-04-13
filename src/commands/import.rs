use anyhow::Result;
use rusqlite::Connection;

pub fn run(_db: &Connection, _path: &str, _validate_only: bool) -> Result<()> {
    todo!("import: import markdown directory")
}
