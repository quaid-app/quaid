use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection) -> Result<()> {
    todo!("compact: WAL checkpoint to single file")
}
