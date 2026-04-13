use anyhow::Result;
use rusqlite::Connection;

pub fn run(
    db: &Connection,
    from: &str,
    to: &str,
    relationship: &str,
    valid_from: Option<String>,
    valid_until: Option<String>,
) -> Result<()> {
    todo!("link: create typed temporal link")
}

pub fn unlink(db: &Connection, link_id: u64) -> Result<()> {
    todo!("unlink: close temporal link")
}

pub fn backlinks(db: &Connection, slug: &str, temporal: Option<String>, json: bool) -> Result<()> {
    todo!("backlinks: list backlinks for a page")
}
