use anyhow::Result;
use rusqlite::Connection;

pub fn tag(_db: &Connection, _slug: &str, _tags: &[String]) -> Result<()> {
    todo!("tag: add tags to page")
}

pub fn untag(_db: &Connection, _slug: &str, _tags: &[String]) -> Result<()> {
    todo!("untag: remove tags from page")
}
