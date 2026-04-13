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

pub fn close(_db: &Connection, _link_id: u64, _valid_until: &str) -> Result<()> {
    todo!("link-close: close a temporal link interval by ID")
}

pub fn links(_db: &Connection, _slug: &str, _temporal: Option<String>, _json: bool) -> Result<()> {
    todo!("links: list all outbound links for a page")
}

pub fn unlink(
    _db: &Connection,
    _from: &str,
    _to: &str,
    _relationship: Option<String>,
) -> Result<()> {
    todo!("unlink: remove cross-reference entirely")
}

pub fn backlinks(
    _db: &Connection,
    _slug: &str,
    _temporal: Option<String>,
    _json: bool,
) -> Result<()> {
    todo!("backlinks: list backlinks for a page")
}
