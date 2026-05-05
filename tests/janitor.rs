use std::path::Path;

use quaid::core::conversation::janitor;
use quaid::core::db;
use rusqlite::Connection;

const FIXED_NOW: &str = "2026-05-05T09:00:00Z";

fn open_queue_db(path: &Path) -> Connection {
    let conn = db::open(path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active'
         WHERE id = 1",
        [path.parent().unwrap().display().to_string()],
    )
    .unwrap();
    conn
}

#[test]
fn janitor_tick_purges_old_terminal_queue_rows_but_keeps_live_rows_and_expires_old_open_corrections(
) {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_queue_db(&db_path);

    conn.execute(
        "INSERT INTO extraction_queue
             (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
         VALUES
              ('old-done', 'conversations/2026-05-03/old-done.md', 'manual', '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', 0, NULL, 'done'),
              ('old-failed', 'conversations/2026-05-03/old-failed.md', 'manual', '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', 3, 'boom', 'failed'),
              ('old-pending', 'conversations/2026-05-03/old-pending.md', 'manual', '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', 0, NULL, 'pending'),
              ('old-running', 'conversations/2026-05-03/old-running.md', 'manual', '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', 1, NULL, 'running'),
              ('recent-done', 'conversations/2026-05-03/recent-done.md', 'manual', '2026-05-04T09:00:00Z', '2026-05-04T09:00:00Z', 0, NULL, 'done')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO correction_sessions
             (correction_id, fact_slug, exchange_log, turns_used, status, created_at, expires_at)
         VALUES
             ('expired-open', 'facts/example', '[]', 1, 'open', '2026-05-05T00:00:00Z', '2000-01-01T00:00:00Z'),
             ('future-open', 'facts/example', '[]', 1, 'open', '2026-05-05T00:00:00Z', '2099-01-01T00:00:00Z')",
        [],
    )
    .unwrap();

    let result = janitor::run_tick_at(&conn, FIXED_NOW).unwrap();

    assert_eq!(result.queue_rows_purged, 2);
    assert_eq!(result.correction_sessions_expired, 1);

    let remaining_queue: Vec<(String, String)> = conn
        .prepare(
            "SELECT session_id, status
             FROM extraction_queue
             ORDER BY session_id",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        remaining_queue,
        vec![
            ("old-pending".to_string(), "pending".to_string()),
            ("old-running".to_string(), "running".to_string()),
            ("recent-done".to_string(), "done".to_string()),
        ]
    );

    let correction_statuses: Vec<(String, String)> = conn
        .prepare(
            "SELECT correction_id, status
             FROM correction_sessions
             ORDER BY correction_id",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        correction_statuses,
        vec![
            ("expired-open".to_string(), "expired".to_string()),
            ("future-open".to_string(), "open".to_string()),
        ]
    );
}

#[test]
fn janitor_tick_leaves_non_expired_or_non_open_correction_sessions_unchanged() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_queue_db(&db_path);

    conn.execute(
        "INSERT INTO correction_sessions
             (correction_id, fact_slug, exchange_log, turns_used, status, created_at, expires_at)
         VALUES
             ('future-open', 'facts/example', '[]', 1, 'open', '2026-05-05T00:00:00Z', '2099-01-01T00:00:00Z'),
             ('committed-past', 'facts/example', '[]', 2, 'committed', '2026-05-05T00:00:00Z', '2000-01-01T00:00:00Z'),
             ('abandoned-past', 'facts/example', '[]', 2, 'abandoned', '2026-05-05T00:00:00Z', '2000-01-01T00:00:00Z'),
             ('already-expired', 'facts/example', '[]', 2, 'expired', '2026-05-05T00:00:00Z', '2000-01-01T00:00:00Z')",
        [],
    )
    .unwrap();

    let result = janitor::run_tick_at(&conn, FIXED_NOW).unwrap();

    assert_eq!(result.queue_rows_purged, 0);
    assert_eq!(result.correction_sessions_expired, 0);

    let correction_statuses: Vec<(String, String)> = conn
        .prepare(
            "SELECT correction_id, status
             FROM correction_sessions
             ORDER BY correction_id",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        correction_statuses,
        vec![
            ("abandoned-past".to_string(), "abandoned".to_string()),
            ("already-expired".to_string(), "expired".to_string()),
            ("committed-past".to_string(), "committed".to_string()),
            ("future-open".to_string(), "open".to_string()),
        ]
    );
}
