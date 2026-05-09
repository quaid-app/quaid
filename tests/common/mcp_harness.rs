#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    dead_code,
    unreachable_pub,
    reason = "test fixtures legitimately panic on setup failure; pub helpers are shared across `tests/mcp_server_*.rs` files but unreachable from non-test crates; `dead_code` because individual test files only use a subset of the helpers"
)]

//! Shared test fixtures for `tests/mcp_server_*.rs` integration tests.
//!
//! Mirrors the inline helpers that previously lived inside
//! `src/mcp/server.rs::tests`. Test bodies depend on these helpers
//! by name and signature, so they are kept verbatim here aside from
//! visibility (`pub`) and the public-API import paths (`quaid::core::db`,
//! `quaid::mcp::server`, etc.).

use std::fs;

use quaid::core::db;
use quaid::mcp::server::{MemoryPutInput, QuaidServer};
use rmcp::model::{CallToolResult, RawContent};
use rusqlite::Connection;

pub fn open_test_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("server.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    let vault_root = dir.path().join("vault");
    fs::create_dir_all(&vault_root).unwrap();
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
    (dir, conn)
}

pub fn create_page(server: &QuaidServer, slug: &str, content: &str) {
    server
        .memory_put(MemoryPutInput {
            slug: slug.to_string(),
            content: content.to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap();
}

pub fn create_page_in_collection(
    server: &QuaidServer,
    collection_name: &str,
    slug: &str,
    content: &str,
) {
    server
        .memory_put(MemoryPutInput {
            slug: format!("{collection_name}::{slug}"),
            content: content.to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap();
}

pub fn insert_collection(conn: &Connection, id: i64, name: &str, is_write_target: bool) {
    let root_path = std::env::temp_dir()
        .join(format!(
            "quaid-mcp-{id}-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
        .display()
        .to_string();
    fs::create_dir_all(&root_path).unwrap();
    conn.execute(
        "INSERT INTO collections (id, name, root_path, state, writable, is_write_target) \
         VALUES (?1, ?2, ?3, 'active', 1, ?4)",
        rusqlite::params![id, name, root_path, if is_write_target { 1 } else { 0 }],
    )
    .unwrap();
}

pub fn set_collection_state(conn: &Connection, name: &str, state: &str) {
    conn.execute(
        "UPDATE collections SET state = ?1 WHERE name = ?2",
        rusqlite::params![state, name],
    )
    .unwrap();
}

pub fn extract_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(tc) => Some(tc.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}
