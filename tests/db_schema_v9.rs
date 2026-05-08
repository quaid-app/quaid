#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for the fresh v9 SQLite schema produced by
//! `quaid::core::db::open`.
//!
//! Locks in:
//!   - tables/columns/indices required for conversation memory and fact
//!     supersession,
//!   - the partial-index predicates and check constraints that gate
//!     `extraction_queue` and `correction_sessions`,
//!   - the configuration defaults seeded at first open.

use quaid::core::db::open;

#[test]
fn fresh_v9_schema_includes_conversation_memory_artifacts_and_defaults() {
    let conn = open(":memory:").unwrap();

    let page_columns: Vec<String> = conn
        .prepare("PRAGMA table_info(pages)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(page_columns.contains(&"superseded_by".to_string()));

    let page_foreign_keys: Vec<(String, String)> = conn
        .prepare("PRAGMA foreign_key_list(pages)")
        .unwrap()
        .query_map([], |row| Ok((row.get(2)?, row.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(page_foreign_keys.contains(&("pages".to_string(), "superseded_by".to_string())));

    let supersede_index_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_pages_supersede_head'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(supersede_index_sql.contains("WHERE superseded_by IS NULL"));

    let session_index_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_pages_session'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(session_index_sql.contains("json_valid(frontmatter)"));
    assert!(session_index_sql.contains("$.session_id"));

    let queue_table_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'extraction_queue'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(queue_table_sql.contains("trigger_kind IN ('debounce', 'session_close', 'manual')"));
    assert!(queue_table_sql.contains("status IN ('pending', 'running', 'done', 'failed')"));

    let pending_index_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_extraction_queue_pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(pending_index_sql.contains("WHERE status = 'pending'"));

    let correction_table_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'correction_sessions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        correction_table_sql.contains("status IN ('open', 'committed', 'abandoned', 'expired')")
    );
    assert!(correction_table_sql.contains("json_valid(exchange_log)"));

    let correction_index_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_correction_open'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(correction_index_sql.contains("WHERE status = 'open'"));

    let config_rows: Vec<(String, String)> = conn
        .prepare(
            "SELECT key, value FROM config
             WHERE key IN (
                 'version',
                 'memory.location',
                 'corrections.history_on_disk',
                 'extraction.max_retries',
                 'extraction.enabled',
                 'extraction.model_alias',
                 'extraction.window_turns',
                 'extraction.debounce_ms',
                 'extraction.idle_close_ms',
                 'extraction.retention_days',
                 'fact_resolution.dedup_cosine_min',
                 'fact_resolution.supersede_cosine_min'
             )
             ORDER BY key",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        config_rows,
        vec![
            (
                "corrections.history_on_disk".to_string(),
                "false".to_string()
            ),
            ("extraction.debounce_ms".to_string(), "5000".to_string()),
            ("extraction.enabled".to_string(), "false".to_string()),
            ("extraction.idle_close_ms".to_string(), "60000".to_string()),
            ("extraction.max_retries".to_string(), "3".to_string()),
            (
                "extraction.model_alias".to_string(),
                "phi-3.5-mini".to_string()
            ),
            ("extraction.retention_days".to_string(), "30".to_string()),
            ("extraction.window_turns".to_string(), "5".to_string()),
            (
                "fact_resolution.dedup_cosine_min".to_string(),
                "0.92".to_string()
            ),
            (
                "fact_resolution.supersede_cosine_min".to_string(),
                "0.4".to_string()
            ),
            ("memory.location".to_string(), "vault-subdir".to_string()),
            ("version".to_string(), "9".to_string()),
        ]
    );
}

#[test]
fn fresh_v9_schema_enforces_superseded_by_foreign_key() {
    let conn = open(":memory:").unwrap();

    let err = conn
        .execute(
            "INSERT INTO pages
                 (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, superseded_by, version)
             VALUES ('notes/invalid-supersede', ?1, 'concept', 'Invalid', '', '', '', '{}', 'notes', '', 9999, 1)",
            [uuid::Uuid::now_v7().to_string()],
        )
        .expect_err("invalid superseded_by should fail");

    assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
}

#[test]
fn fresh_v9_schema_rejects_invalid_extraction_queue_trigger_kind() {
    let conn = open(":memory:").unwrap();

    let err = conn
        .execute(
            "INSERT INTO extraction_queue
                 (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, status)
             VALUES ('s1', 'conversations/2026-05-04/s1.md', 'arbitrary', '2026-05-04T00:00:00Z', '2026-05-04T00:00:05Z', 'pending')",
            [],
        )
        .expect_err("invalid trigger_kind should fail");

    assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
}

#[test]
fn fresh_v9_schema_rejects_invalid_extraction_queue_status() {
    let conn = open(":memory:").unwrap();

    let err = conn
        .execute(
            "INSERT INTO extraction_queue
                 (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, status)
             VALUES ('s1', 'conversations/2026-05-04/s1.md', 'debounce', '2026-05-04T00:00:00Z', '2026-05-04T00:00:05Z', 'queued')",
            [],
        )
        .expect_err("invalid status should fail");

    assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
}

#[test]
fn fresh_v9_schema_rejects_invalid_correction_session_status() {
    let conn = open(":memory:").unwrap();

    let err = conn
        .execute(
            "INSERT INTO correction_sessions
                 (correction_id, fact_slug, exchange_log, turns_used, status, created_at, expires_at)
             VALUES (?1, 'facts/example', '[]', 0, 'paused', '2026-05-05T00:00:00Z', '2026-05-05T01:00:00Z')",
            [uuid::Uuid::now_v7().to_string()],
        )
        .expect_err("invalid correction session status should fail");

    assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
}
