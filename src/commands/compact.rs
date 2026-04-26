use anyhow::Result;
use rusqlite::Connection;

use crate::core::db;

/// Checkpoint the WAL and compact the database to a single file.
pub fn run(conn: &Connection) -> Result<()> {
    db::compact(conn)?;
    println!("Compacted database (WAL checkpoint complete)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db as test_db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let conn = test_db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    #[test]
    fn compact_succeeds_on_live_database() {
        let conn = open_test_db();

        // Write something to generate WAL activity
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES ('test/compact', 'concept', 'Test', '', 'content', '', '{}', '', '', 1)",
            [],
        )
        .unwrap();

        let result = run(&conn);
        assert!(result.is_ok());
    }

    #[test]
    fn compact_succeeds_on_empty_database() {
        let conn = open_test_db();
        let result = run(&conn);
        assert!(result.is_ok());
    }
}
