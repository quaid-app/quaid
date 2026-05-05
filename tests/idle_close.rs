use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use quaid::core::conversation::{format, idle_close};
use quaid::core::db;
use quaid::core::types::ConversationStatus;
use quaid::mcp::server::{MemoryAddTurnInput, MemoryCloseSessionInput, QuaidServer};
use rusqlite::Connection;

fn open_turn_server(root: &Path) -> (tempfile::TempDir, PathBuf, QuaidServer) {
    let db_dir = tempfile::TempDir::new().unwrap();
    let db_path = db_dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active'
         WHERE id = 1",
        [root.display().to_string()],
    )
    .unwrap();
    (db_dir, db_path, QuaidServer::new(conn))
}

fn configure_idle_close(db_path: &Path, idle_close_ms: i64) -> Connection {
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO config(key, value)
         VALUES ('extraction.idle_close_ms', ?1)",
        [idle_close_ms.to_string()],
    )
    .unwrap();
    conn
}

#[test]
fn idle_close_enqueues_session_close_and_marks_file_closed() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, db_path, server) = open_turn_server(vault_root.path());
    let db = configure_idle_close(&db_path, 100);

    server
        .memory_add_turn(MemoryAddTurnInput {
            session_id: "idle-auto".to_string(),
            role: "user".to_string(),
            content: "hello".to_string(),
            timestamp: Some("2026-05-03T09:14:22Z".to_string()),
            metadata: None,
            namespace: None,
        })
        .unwrap();

    let results = idle_close::scan_due_sessions_at(
        &db,
        db_path.to_str().unwrap(),
        Instant::now() + Duration::from_millis(150),
    )
    .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].session_id, "idle-auto");
    assert!(results[0].newly_closed);

    let conversation_path = vault_root
        .path()
        .join("conversations")
        .join("2026-05-03")
        .join("idle-auto.md");
    let conversation = format::parse(&conversation_path).unwrap();
    assert_eq!(conversation.frontmatter.status, ConversationStatus::Closed);
    assert!(conversation.frontmatter.closed_at.is_some());

    let queue_row: (i64, String, String) = db
        .query_row(
            "SELECT COUNT(*), trigger_kind, status
             FROM extraction_queue
             WHERE session_id = 'idle-auto'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(queue_row.0, 1);
    assert_eq!(queue_row.1, "session_close");
    assert_eq!(queue_row.2, "pending");
}

#[test]
fn idle_close_activity_resets_timer() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, db_path, server) = open_turn_server(vault_root.path());
    let db = configure_idle_close(&db_path, 150);

    server
        .memory_add_turn(MemoryAddTurnInput {
            session_id: "idle-reset".to_string(),
            role: "user".to_string(),
            content: "first".to_string(),
            timestamp: Some("2026-05-03T09:14:22Z".to_string()),
            metadata: None,
            namespace: None,
        })
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    server
        .memory_add_turn(MemoryAddTurnInput {
            session_id: "idle-reset".to_string(),
            role: "assistant".to_string(),
            content: "second".to_string(),
            timestamp: Some("2026-05-03T09:14:23Z".to_string()),
            metadata: None,
            namespace: None,
        })
        .unwrap();

    let before_timeout = idle_close::scan_due_sessions_at(
        &db,
        db_path.to_str().unwrap(),
        Instant::now() + Duration::from_millis(40),
    )
    .unwrap();
    assert!(before_timeout.is_empty());

    let after_timeout = idle_close::scan_due_sessions_at(
        &db,
        db_path.to_str().unwrap(),
        Instant::now() + Duration::from_millis(170),
    )
    .unwrap();
    assert_eq!(after_timeout.len(), 1);
    assert_eq!(after_timeout[0].session_id, "idle-reset");
}

#[test]
fn explicit_session_close_clears_idle_tracker() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, db_path, server) = open_turn_server(vault_root.path());
    let db = configure_idle_close(&db_path, 50);

    server
        .memory_add_turn(MemoryAddTurnInput {
            session_id: "idle-explicit".to_string(),
            role: "user".to_string(),
            content: "wrap up".to_string(),
            timestamp: Some("2026-05-03T09:14:22Z".to_string()),
            metadata: None,
            namespace: None,
        })
        .unwrap();
    server
        .memory_close_session(MemoryCloseSessionInput {
            session_id: "idle-explicit".to_string(),
            namespace: None,
        })
        .unwrap();

    let results = idle_close::scan_due_sessions_at(
        &db,
        db_path.to_str().unwrap(),
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    assert!(results.is_empty());

    let queue_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM extraction_queue WHERE session_id = 'idle-explicit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(queue_count, 1);
}
