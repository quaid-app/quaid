use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection, path: &str, force: bool) -> Result<()> {
    todo!("ingest: source document ingestion with novelty check")
}
