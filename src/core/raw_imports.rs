use rusqlite::Connection;

use crate::core::page_uuid;

const DEFAULT_KEEP: i64 = 10;
const DEFAULT_TTL_DAYS: i64 = 90;

pub fn rotate_active_raw_import(
    conn: &Connection,
    page_id: i64,
    file_path: &str,
    raw_bytes: &[u8],
) -> rusqlite::Result<()> {
    assert_existing_active_row_invariant(conn, page_id)?;

    conn.execute(
        "UPDATE raw_imports
         SET is_active = 0
         WHERE page_id = ?1 AND is_active = 1",
        [page_id],
    )?;

    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
         VALUES (?1, ?2, 1, ?3, ?4)",
        rusqlite::params![page_id, page_uuid::generate_uuid_v7(), raw_bytes, file_path],
    )?;

    if let Ok(raw_str) = std::str::from_utf8(raw_bytes) {
        let (fm, _) = crate::core::markdown::parse_frontmatter(raw_str);
        if !fm.is_empty() {
            if let Ok(json) = serde_json::to_string(&fm) {
                let _ = conn.execute(
                    "UPDATE pages SET frontmatter = ?1 WHERE id = ?2",
                    rusqlite::params![json, page_id],
                );
            }
        }
    }

    prune_inactive_rows(conn, page_id)?;
    assert_exactly_one_active_row(conn, page_id)
}

pub fn enqueue_embedding_job(conn: &Connection, page_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO embedding_jobs (page_id)
         VALUES (?1)
         ON CONFLICT(page_id) DO UPDATE SET
             started_at = NULL,
             enqueued_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        [page_id],
    )?;
    Ok(())
}

pub fn assert_exactly_one_active_row(conn: &Connection, page_id: i64) -> rusqlite::Result<()> {
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM raw_imports
         WHERE page_id = ?1 AND is_active = 1",
        [page_id],
        |row| row.get(0),
    )?;

    if active_count == 1 {
        return Ok(());
    }

    Err(rusqlite::Error::InvalidParameterName(format!(
        "InvariantViolationError: page_id={page_id} has {active_count} active raw_imports rows"
    )))
}

#[cfg(test)]
pub fn active_raw_import_count(conn: &Connection, page_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*)
         FROM raw_imports
         WHERE page_id = ?1 AND is_active = 1",
        [page_id],
        |row| row.get(0),
    )
}

fn assert_existing_active_row_invariant(conn: &Connection, page_id: i64) -> rusqlite::Result<()> {
    let (row_count, active_count): (i64, i64) = conn.query_row(
        "SELECT
             COUNT(*) AS row_count,
             COALESCE(SUM(CASE WHEN is_active = 1 THEN 1 ELSE 0 END), 0) AS active_count
         FROM raw_imports
         WHERE page_id = ?1",
        [page_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    if row_count == 0 || active_count == 1 {
        return Ok(());
    }

    Err(rusqlite::Error::InvalidParameterName(format!(
        "InvariantViolationError: page_id={page_id} entered rotation with {active_count} active raw_imports rows across {row_count} historical rows"
    )))
}

fn prune_inactive_rows(conn: &Connection, page_id: i64) -> rusqlite::Result<()> {
    if keep_all_enabled() {
        return Ok(());
    }

    let keep = integer_env("GBRAIN_RAW_IMPORTS_KEEP", DEFAULT_KEEP).max(0);
    let ttl_days = integer_env("GBRAIN_RAW_IMPORTS_TTL_DAYS", DEFAULT_TTL_DAYS).max(0);
    let ttl_cutoff = format!("-{ttl_days} days");
    let mut stmt = conn.prepare(
        "SELECT id, julianday(created_at) < julianday('now', ?2) AS expired
         FROM raw_imports
         WHERE page_id = ?1 AND is_active = 0
         ORDER BY created_at DESC, id DESC",
    )?;
    let rows = stmt.query_map(rusqlite::params![page_id, ttl_cutoff], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? != 0))
    })?;

    let mut ids_to_delete = Vec::new();
    for (index, row) in rows.enumerate() {
        let (id, expired) = row?;
        if expired || index >= keep as usize {
            ids_to_delete.push(id);
        }
    }

    for id in ids_to_delete {
        conn.execute("DELETE FROM raw_imports WHERE id = ?1", [id])?;
    }

    Ok(())
}

fn keep_all_enabled() -> bool {
    std::env::var("GBRAIN_RAW_IMPORTS_KEEP_ALL").as_deref() == Ok("1")
}

fn integer_env(name: &str, default_value: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(default_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    use rusqlite::Connection;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    static ENV_MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_mutation_lock() -> &'static Mutex<()> {
        ENV_MUTATION_LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }

        fn clear(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(value) = self.previous.as_ref() {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pages (collection_id, slug, uuid, type, title)
             VALUES (1, 'notes/test', ?1, 'concept', 'notes/test')",
            [page_uuid::generate_uuid_v7()],
        )
        .unwrap();
        conn
    }

    fn page_id(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT id FROM pages WHERE slug = 'notes/test'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn rotation_bootstraps_first_active_row() {
        let conn = open_test_db();
        let page_id = page_id(&conn);

        rotate_active_raw_import(&conn, page_id, "notes/test.md", b"first").unwrap();

        assert_eq!(active_raw_import_count(&conn, page_id).unwrap(), 1);
    }

    #[test]
    fn rotation_marks_prior_row_inactive() {
        let conn = open_test_db();
        let page_id = page_id(&conn);

        rotate_active_raw_import(&conn, page_id, "notes/test.md", b"first").unwrap();
        rotate_active_raw_import(&conn, page_id, "notes/test.md", b"second").unwrap();

        let inactive_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM raw_imports
                 WHERE page_id = ?1 AND is_active = 0",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(active_raw_import_count(&conn, page_id).unwrap(), 1);
        assert_eq!(inactive_count, 1);
    }

    #[test]
    fn inline_gc_keeps_only_ten_inactive_rows_by_default() {
        let _env_guard = env_mutation_lock().lock().unwrap();
        let _keep_all = EnvVarGuard::clear("GBRAIN_RAW_IMPORTS_KEEP_ALL");
        let _keep = EnvVarGuard::clear("GBRAIN_RAW_IMPORTS_KEEP");
        let _ttl = EnvVarGuard::clear("GBRAIN_RAW_IMPORTS_TTL_DAYS");
        let conn = open_test_db();
        let page_id = page_id(&conn);

        for revision in 0..12 {
            rotate_active_raw_import(
                &conn,
                page_id,
                "notes/test.md",
                format!("revision-{revision}").as_bytes(),
            )
            .unwrap();
        }

        let inactive_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM raw_imports
                 WHERE page_id = ?1 AND is_active = 0",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(inactive_count, 10);
        assert_eq!(active_raw_import_count(&conn, page_id).unwrap(), 1);
    }

    #[test]
    fn inline_gc_prunes_ttl_expired_inactive_rows() {
        let _env_guard = env_mutation_lock().lock().unwrap();
        let _keep_all = EnvVarGuard::clear("GBRAIN_RAW_IMPORTS_KEEP_ALL");
        let _keep = EnvVarGuard::clear("GBRAIN_RAW_IMPORTS_KEEP");
        let _ttl = EnvVarGuard::clear("GBRAIN_RAW_IMPORTS_TTL_DAYS");
        let conn = open_test_db();
        let page_id = page_id(&conn);

        rotate_active_raw_import(&conn, page_id, "notes/test.md", b"first").unwrap();
        rotate_active_raw_import(&conn, page_id, "notes/test.md", b"second").unwrap();
        conn.execute(
            "UPDATE raw_imports
             SET created_at = '2000-01-01T00:00:00Z'
             WHERE page_id = ?1 AND is_active = 0",
            [page_id],
        )
        .unwrap();

        rotate_active_raw_import(&conn, page_id, "notes/test.md", b"third").unwrap();

        let inactive_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM raw_imports
                 WHERE page_id = ?1 AND is_active = 0",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(inactive_count, 1);
    }

    #[test]
    fn keep_all_disables_inline_gc() {
        let _env_guard = env_mutation_lock().lock().unwrap();
        let _keep_all = EnvVarGuard::set("GBRAIN_RAW_IMPORTS_KEEP_ALL", "1");
        let conn = open_test_db();
        let page_id = page_id(&conn);

        for revision in 0..12 {
            rotate_active_raw_import(
                &conn,
                page_id,
                "notes/test.md",
                format!("revision-{revision}").as_bytes(),
            )
            .unwrap();
        }

        let inactive_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM raw_imports
                 WHERE page_id = ?1 AND is_active = 0",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(inactive_count, 11);
        assert_eq!(active_raw_import_count(&conn, page_id).unwrap(), 1);
    }

    #[test]
    fn rotation_refuses_pages_with_zero_active_rows_in_history() {
        let conn = open_test_db();
        let page_id = page_id(&conn);
        conn.execute(
            "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
             VALUES (?1, ?2, 0, ?3, ?4)",
            rusqlite::params![
                page_id,
                page_uuid::generate_uuid_v7(),
                b"stale",
                "notes/test.md"
            ],
        )
        .unwrap();

        let error = rotate_active_raw_import(&conn, page_id, "notes/test.md", b"new bytes")
            .unwrap_err()
            .to_string();

        assert!(error.contains("InvariantViolationError"));
        assert!(error.contains("0 active raw_imports rows"));
    }
}
