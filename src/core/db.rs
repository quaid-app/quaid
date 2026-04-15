use std::path::Path;
use std::sync::Once;

use rusqlite::Connection;

use super::types::DbError;

static SQLITE_VEC_INIT: Once = Once::new();

/// Register sqlite-vec as an auto-extension (process-global, idempotent).
fn ensure_sqlite_vec() {
    SQLITE_VEC_INIT.call_once(|| {
        // SAFETY: sqlite3_vec_init is a valid SQLite extension entry point
        // provided by the statically linked sqlite-vec crate. The transmute
        // is required because the actual C entry-point signature differs from
        // the auto_extension callback typedef.
        unsafe {
            let init_fn = std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *const std::ffi::c_char,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::ffi::c_int,
            >(sqlite_vec::sqlite3_vec_init as *const ());
            rusqlite::ffi::sqlite3_auto_extension(Some(init_fn));
        }
    });
}

/// Open (or create) a brain database at `path`.
///
/// Applies the full v4 DDL from `schema.sql`, enables WAL journal mode and
/// foreign keys, loads sqlite-vec, creates the vec0 virtual table, seeds the
/// default embedding model, and sets `PRAGMA user_version = 4`.
pub fn open(path: &str) -> Result<Connection, DbError> {
    let db_path = Path::new(path);
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(DbError::PathNotFound {
                path: parent.display().to_string(),
            });
        }
    }

    ensure_sqlite_vec();

    let conn = Connection::open(path)?;

    // Full v4 DDL — includes PRAGMA journal_mode=WAL, foreign_keys=ON,
    // all CREATE TABLE/INDEX/TRIGGER IF NOT EXISTS, config seed inserts.
    conn.execute_batch(include_str!("../schema.sql"))?;

    // vec0 virtual table for vector search (requires sqlite-vec)
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS page_embeddings_vec_384 \
         USING vec0(embedding float[384]);",
    )?;

    // Seed default embedding model
    conn.execute(
        "INSERT OR IGNORE INTO embedding_models (name, dimensions, vec_table, active) \
         VALUES ('bge-small-en-v1.5', 384, 'page_embeddings_vec_384', 1)",
        [],
    )?;

    set_version(&conn)?;

    Ok(conn)
}

/// Checkpoint the WAL back into the main database file.
pub fn compact(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

/// Set the database schema version to v4.
pub fn set_version(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch("PRAGMA user_version = 4;")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_all_expected_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();

        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        let expected = [
            "assertions",
            "config",
            "contradictions",
            "embedding_models",
            "import_manifest",
            "ingest_log",
            "knowledge_gaps",
            "links",
            "page_embeddings",
            "page_fts",
            "pages",
            "raw_data",
            "raw_imports",
            "tags",
            "timeline_entries",
        ];

        for name in &expected {
            assert!(
                tables.contains(&(*name).to_string()),
                "missing table: {name}"
            );
        }

        // Verify sqlite-vec loaded — vec_version() is only available if the extension is active
        let vec_version: String = conn
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .unwrap();
        assert!(
            vec_version.starts_with("v"),
            "unexpected vec_version: {vec_version}"
        );
    }

    #[test]
    fn open_sets_user_version_to_4() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
    }

    #[test]
    fn open_enables_wal_and_foreign_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();

        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");

        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn open_rejects_nonexistent_parent_dir() {
        let result = open("/nonexistent/dir/brain.db");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DbError::PathNotFound { .. }));
    }

    #[test]
    fn open_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let path_str = db_path.to_str().unwrap();

        let conn1 = open(path_str).unwrap();
        drop(conn1);

        // Re-open same database — should succeed without errors
        let conn2 = open(path_str).unwrap();
        let version: i64 = conn2
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
    }

    #[test]
    fn compact_checkpoints_wal() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();
        assert!(compact(&conn).is_ok());
    }

    #[test]
    fn open_seeds_default_embedding_model() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();

        let (name, dims, active): (String, i64, i64) = conn
            .query_row(
                "SELECT name, dimensions, active FROM embedding_models WHERE active = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(name, "bge-small-en-v1.5");
        assert_eq!(dims, 384);
        assert_eq!(active, 1);
    }
}
