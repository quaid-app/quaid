use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::core::migrate;

pub fn run(db: &Connection, path: &str, validate_only: bool) -> Result<()> {
    let dir = Path::new(path);
    let stats = migrate::import_dir(db, dir, validate_only)?;

    if validate_only {
        println!("Validation passed: {} file(s) OK", stats.imported);
    } else {
        println!(
            "Imported {} page(s) ({} skipped)",
            stats.imported, stats.skipped
        );
    }

    Ok(())
}
