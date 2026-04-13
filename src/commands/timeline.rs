use anyhow::Result;
use rusqlite::Connection;

pub fn run(_db: &Connection, _slug: &str, _limit: u32, _json: bool) -> Result<()> {
    todo!("timeline: show timeline entries for page")
}

pub fn add(
    _db: &Connection,
    _slug: &str,
    _date: &str,
    _summary: &str,
    _source: Option<String>,
    _detail: Option<String>,
) -> Result<()> {
    todo!("timeline-add: add a structured timeline entry")
}
