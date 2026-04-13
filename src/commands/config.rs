use anyhow::Result;
use clap::Subcommand;
use rusqlite::Connection;

#[derive(Subcommand)]
pub enum ConfigAction {
    Get { key: String },
    Set { key: String, value: String },
    List,
}

pub fn run(db: &Connection, action: ConfigAction) -> Result<()> {
    todo!("config: get/set/list brain config")
}
