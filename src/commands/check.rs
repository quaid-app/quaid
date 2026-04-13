use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection, slug: Option<String>, all: bool, check_type: Option<String>, json: bool) -> Result<()> {
    todo!("check: contradiction detection")
}
