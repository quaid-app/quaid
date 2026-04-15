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
    match action {
        ConfigAction::Get { key } => {
            let value: Result<String, _> =
                db.query_row("SELECT value FROM config WHERE key = ?1", [&key], |row| {
                    row.get(0)
                });
            match value {
                Ok(v) => println!("{v}"),
                Err(_) => println!("Not set"),
            }
        }
        ConfigAction::Set { key, value } => {
            db.execute(
                "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )?;
            println!("Set {key} = {value}");
        }
        ConfigAction::List => {
            let mut stmt = db.prepare("SELECT key, value FROM config ORDER BY key")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (k, v) = row?;
                println!("{k}={v}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    #[test]
    fn set_then_get_returns_value() {
        let conn = open_test_db();
        run(
            &conn,
            ConfigAction::Set {
                key: "test_key".into(),
                value: "test_value".into(),
            },
        )
        .unwrap();

        let value: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'test_key'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "test_value");
    }

    #[test]
    fn set_overwrites_existing_value() {
        let conn = open_test_db();
        run(
            &conn,
            ConfigAction::Set {
                key: "version".into(),
                value: "99".into(),
            },
        )
        .unwrap();

        let value: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "99");
    }
}
