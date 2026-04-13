use anyhow::Result;
use rusqlite::Connection;

pub fn run(_db: &Connection, _all: bool, _stale: bool) -> Result<()> {
    todo!("embed: generate or refresh embeddings")
}
