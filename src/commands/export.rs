use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection, path: &str, raw: bool, import_id: Option<String>) -> Result<()> {
    todo!("export: export brain to markdown directory")
}
