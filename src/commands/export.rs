use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::core::migrate;

pub fn run(db: &Connection, path: &str, _raw: bool, _import_id: Option<String>) -> Result<()> {
    let output = Path::new(path);
    let count = migrate::export_dir(db, output)?;
    println!("Exported {count} page(s) to {path}");
    Ok(())
}
