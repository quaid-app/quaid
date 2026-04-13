use anyhow::Result;
use clap::Subcommand;
use rusqlite::Connection;

#[derive(Subcommand)]
pub enum ConfigAction {
    Get { key: String },
    Set { key: String, value: String },
    List,
}

pub fn run(_db: &Connection, _action: ConfigAction) -> Result<()> {
    todo!("config: get/set/list brain config")
}
