use anyhow::Result;
use rusqlite::Connection;

pub fn run(_db: &Connection, _path: &str, _force: bool) -> Result<()> {
    todo!("ingest: source document ingestion with novelty check")
}
