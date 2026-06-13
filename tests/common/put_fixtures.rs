#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    dead_code,
    unreachable_pub,
    reason = "test fixtures legitimately panic on setup failure; pub helpers are shared across `tests/cli_put_*.rs` files but unreachable from non-test crates; `dead_code` because individual test files only use a subset of the helpers"
)]

//! Shared test fixtures for `tests/cli_put_*.rs` integration tests.
//!
//! Mirrors a subset of the inline helpers that previously lived inside
//! `src/commands/put.rs::tests` — only the helpers that the moved
//! public-API tests need. White-box helpers (e.g. `HookGuard`,
//! `PutTestHooks`, `open_test_db_with_vault_guarded`) stay inline because
//! they reference private items and per the test-organization spec
//! visibility cannot be widened.

use std::path::PathBuf;

use quaid::core::db;
use rusqlite::Connection;

pub fn open_test_db() -> Connection {
    open_test_db_with_path().0
}

/// Open an on-disk test database (in a leaked tempdir) and return both the
/// connection and the database path, so callers can open additional
/// connections to the same file and exercise cross-connection write
/// contention.
pub fn open_test_db_with_path() -> (Connection, PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let db_path = dir.path().join("test_memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    let vault_root = dir.path().join("vault");
    std::fs::create_dir_all(&vault_root).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             writable = 1,
             is_write_target = 1,
             state = 'active',
             needs_full_sync = 0
         WHERE id = 1",
        [vault_root.display().to_string()],
    )
    .unwrap();
    std::mem::forget(dir);
    (conn, db_path)
}

/// Open an additional connection to an already-initialized test database.
pub fn reopen_test_db(db_path: &std::path::Path) -> Connection {
    db::open(db_path.to_str().unwrap()).unwrap()
}

pub fn active_raw_import_count_for_slug(conn: &Connection, slug: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM raw_imports \
         WHERE page_id = (SELECT id FROM pages WHERE slug = ?1) AND is_active = 1",
        [slug],
        |row| row.get(0),
    )
    .unwrap()
}

pub fn active_raw_import_bytes_for_slug(conn: &Connection, slug: &str) -> Vec<u8> {
    conn.query_row(
        "SELECT raw_bytes FROM raw_imports \
         WHERE page_id = (SELECT id FROM pages WHERE slug = ?1) AND is_active = 1",
        [slug],
        |row| row.get(0),
    )
    .unwrap()
}

/// Helper: read a page back from the database.
pub fn read_page(conn: &Connection, slug: &str) -> Option<(i64, String, String, String, String)> {
    conn.prepare("SELECT version, type, title, compiled_truth, timeline FROM pages WHERE slug = ?1")
        .unwrap()
        .query_row([slug], |row| {
            let compiled_truth: String = row.get(3)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                compiled_truth.trim_end_matches('\n').to_owned(),
                row.get(4)?,
            ))
        })
        .ok()
}

pub fn superseded_by_for_slug(conn: &Connection, slug: &str) -> Option<i64> {
    conn.query_row(
        "SELECT superseded_by FROM pages WHERE slug = ?1",
        [slug],
        |row| row.get(0),
    )
    .ok()
    .flatten()
}

pub fn page_id_for_slug(conn: &Connection, slug: &str) -> i64 {
    conn.query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })
    .unwrap()
}
