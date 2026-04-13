use anyhow::Result;
use rusqlite::Connection;

pub fn tag(db: &Connection, slug: &str, tags: &[String]) -> Result<()> {
    todo!("tag: add tags to page")
}

pub fn untag(db: &Connection, slug: &str, tags: &[String]) -> Result<()> {
    todo!("untag: remove tags from page")
}
