use anyhow::Result;
use rusqlite::Connection;

pub fn open(_path: &str) -> Result<Connection> {
    todo!("db: open connection, init schema, WAL, sqlite-vec load")
}
