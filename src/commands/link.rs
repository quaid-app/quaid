use anyhow::Result;
use rusqlite::Connection;

pub fn run(
    _db: &Connection,
    _from: &str,
    _to: &str,
    _relationship: &str,
    _valid_from: Option<String>,
    _valid_until: Option<String>,
) -> Result<()> {
    todo!("link: create typed temporal link")
}

pub fn unlink(_db: &Connection, _link_id: u64) -> Result<()> {
    todo!("unlink: close temporal link")
}

pub fn backlinks(
    _db: &Connection,
    _slug: &str,
    _temporal: Option<String>,
    _json: bool,
) -> Result<()> {
    todo!("backlinks: list backlinks for a page")
}
