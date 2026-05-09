#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    dead_code,
    unreachable_pub,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites; common test helpers are pub-shared across test modules but are unreachable from non-test crates and not all helpers are used by every consumer"
)]

//! Shared fixtures for reconciler integration tests.
//!
//! Verbatim copies of helpers that previously lived inside
//! `src/core/reconciler.rs::mod tests`, with `crate::` paths rewritten
//! to `quaid::`. Only public items are referenced.

use quaid::core::collections::Collection;
use quaid::core::file_state::{self, upsert_file_state, FileStat};
use quaid::core::page_uuid;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct SeededPageIdentity<'a> {
    pub slug: &'a str,
    pub uuid: &'a str,
    pub relative_path: &'a str,
    pub stat: &'a FileStat,
    pub sha256: &'a str,
    pub compiled_truth: &'a str,
    pub timeline: &'a str,
}

pub fn open_test_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../src/schema.sql"))
        .unwrap();
    conn
}

pub fn open_test_db_file() -> (TempDir, PathBuf, rusqlite::Connection) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(include_str!("../../src/schema.sql"))
        .unwrap();
    (dir, db_path, conn)
}

pub fn insert_collection(conn: &Connection, root_path: &Path) -> Collection {
    insert_collection_with_state(
        conn,
        root_path,
        quaid::core::collections::CollectionState::Active,
        false,
    )
}

pub fn insert_collection_with_state(
    conn: &Connection,
    root_path: &Path,
    state: quaid::core::collections::CollectionState,
    needs_full_sync: bool,
) -> Collection {
    let (active_lease_session_id, restore_command_id, restore_lease_session_id) =
        owner_identity_defaults_for_state(state);
    conn.execute(
        "INSERT INTO collections
                 (name, root_path, state, needs_full_sync,
                  active_lease_session_id, restore_command_id, restore_lease_session_id)
             VALUES ('test', ?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            root_path.to_string_lossy(),
            state.as_str(),
            if needs_full_sync { 1 } else { 0 },
            active_lease_session_id,
            restore_command_id,
            restore_lease_session_id,
        ],
    )
    .unwrap();
    Collection {
        id: 1,
        name: "test".to_owned(),
        root_path: root_path.to_string_lossy().into_owned(),
        state,
        writable: true,
        is_write_target: false,
        ignore_patterns: None,
        ignore_parse_errors: None,
        needs_full_sync,
        last_sync_at: None,
        active_lease_session_id: active_lease_session_id.map(str::to_owned),
        restore_command_id: restore_command_id.map(str::to_owned),
        restore_lease_session_id: restore_lease_session_id.map(str::to_owned),
        reload_generation: 0,
        watcher_released_session_id: None,
        watcher_released_generation: None,
        watcher_released_at: None,
        pending_command_heartbeat_at: None,
        pending_root_path: None,
        pending_restore_manifest: None,
        restore_command_pid: None,
        restore_command_host: None,
        integrity_failed_at: None,
        pending_manifest_incomplete_at: None,
        reconcile_halted_at: None,
        reconcile_halt_reason: None,
        created_at: "2024-01-01T00:00:00Z".to_owned(),
        updated_at: "2024-01-01T00:00:00Z".to_owned(),
    }
}

pub fn sample_collection_in_state(state: quaid::core::collections::CollectionState) -> Collection {
    let (active_lease_session_id, restore_command_id, restore_lease_session_id) =
        owner_identity_defaults_for_state(state);
    Collection {
        id: 1,
        name: "test".to_owned(),
        root_path: "/vault".to_owned(),
        state,
        writable: true,
        is_write_target: false,
        ignore_patterns: None,
        ignore_parse_errors: None,
        needs_full_sync: false,
        last_sync_at: None,
        active_lease_session_id: active_lease_session_id.map(str::to_owned),
        restore_command_id: restore_command_id.map(str::to_owned),
        restore_lease_session_id: restore_lease_session_id.map(str::to_owned),
        reload_generation: 0,
        watcher_released_session_id: None,
        watcher_released_generation: None,
        watcher_released_at: None,
        pending_command_heartbeat_at: None,
        pending_root_path: None,
        pending_restore_manifest: None,
        restore_command_pid: None,
        restore_command_host: None,
        integrity_failed_at: None,
        pending_manifest_incomplete_at: None,
        reconcile_halted_at: None,
        reconcile_halt_reason: None,
        created_at: "2024-01-01T00:00:00Z".to_owned(),
        updated_at: "2024-01-01T00:00:00Z".to_owned(),
    }
}

pub fn owner_identity_defaults_for_state(
    state: quaid::core::collections::CollectionState,
) -> (
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
) {
    match state {
        quaid::core::collections::CollectionState::Active => (Some("lease-1"), None, None),
        quaid::core::collections::CollectionState::Detached => (None, None, None),
        quaid::core::collections::CollectionState::Restoring => {
            (None, Some("restore-1"), Some("restore-lease-1"))
        }
    }
}

pub fn set_collection_dirty_flag(conn: &Connection, collection_id: i64, needs_full_sync: bool) {
    conn.execute(
        "UPDATE collections
             SET needs_full_sync = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
        rusqlite::params![collection_id, if needs_full_sync { 1 } else { 0 }],
    )
    .unwrap();
}

pub fn insert_page(conn: &Connection, collection_id: i64, slug: &str) -> i64 {
    conn.execute(
        "INSERT INTO pages (collection_id, slug, uuid, type, title, compiled_truth, timeline)
             VALUES (?1, ?2, ?3, 'concept', ?2, 'Body', '')",
        rusqlite::params![collection_id, slug, page_uuid::generate_uuid_v7()],
    )
    .unwrap();
    conn.last_insert_rowid()
}

pub fn stat_for(root_path: &Path, relative_path: &str) -> FileStat {
    file_state::stat_file(&root_path.join(relative_path)).unwrap()
}

pub fn unique_old_stat(current: &FileStat) -> FileStat {
    FileStat {
        mtime_ns: current.mtime_ns.saturating_sub(1),
        ctime_ns: current.ctime_ns.map(|value| value.saturating_sub(1)),
        size_bytes: current.size_bytes.saturating_add(1),
        inode: current.inode.map(|value| value.saturating_add(1)),
    }
}

pub fn production_reconciler_source() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("core")
        .join("reconciler.rs");
    std::fs::read_to_string(path).expect("read production reconciler source")
}

pub fn seed_file_state(
    conn: &Connection,
    collection_id: i64,
    slug: &str,
    relative_path: &str,
    stat: &FileStat,
) -> i64 {
    let page_id = insert_page(conn, collection_id, slug);
    upsert_file_state(conn, collection_id, relative_path, page_id, stat, "abc123").unwrap();
    page_id
}

pub fn active_raw_import_count(conn: &Connection, page_id: i64) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1 AND is_active = 1",
        [page_id],
        |row| row.get(0),
    )
    .unwrap()
}

pub fn active_raw_import_bytes(conn: &Connection, page_id: i64) -> Vec<u8> {
    conn.query_row(
        "SELECT raw_bytes FROM raw_imports WHERE page_id = ?1 AND is_active = 1",
        [page_id],
        |row| row.get(0),
    )
    .unwrap()
}

pub fn seed_page_with_identity(
    conn: &Connection,
    collection_id: i64,
    identity: SeededPageIdentity<'_>,
) -> i64 {
    conn.execute(
        "INSERT INTO pages (collection_id, slug, uuid, type, title, compiled_truth, timeline)
         VALUES (?1, ?2, ?3, 'concept', ?2, ?4, ?5)",
        rusqlite::params![
            collection_id,
            identity.slug,
            identity.uuid,
            identity.compiled_truth,
            identity.timeline
        ],
    )
    .unwrap();
    let page_id = conn.last_insert_rowid();
    upsert_file_state(
        conn,
        collection_id,
        identity.relative_path,
        page_id,
        identity.stat,
        identity.sha256,
    )
    .unwrap();
    page_id
}

pub fn total_raw_import_count(conn: &Connection, page_id: i64) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
        [page_id],
        |row| row.get(0),
    )
    .unwrap()
}

pub fn insert_programmatic_link(conn: &Connection, page_a: i64, page_b: i64) {
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
         VALUES (?1, ?2, 'related', 'programmatic')",
        rusqlite::params![page_a, page_b],
    )
    .unwrap();
}
