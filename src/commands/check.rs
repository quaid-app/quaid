use anyhow::Result;
use rusqlite::Connection;

pub fn run(
    _db: &Connection,
    _slug: Option<String>,
    _all: bool,
    _check_type: Option<String>,
    _json: bool,
) -> Result<()> {
    todo!("check: contradiction detection")
}
