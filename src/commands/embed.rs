use anyhow::Result;
use rusqlite::Connection;

pub fn run(db: &Connection, all: bool, stale: bool) -> Result<()> {
    todo!("embed: generate or refresh embeddings")
}
