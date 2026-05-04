use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

use quaid::core::conversation::queue::{dequeue, enqueue, mark_done, mark_failed};
use quaid::core::db;
use quaid::core::types::{ExtractionJobStatus, ExtractionTriggerKind};
use rusqlite::Connection;

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
fn enqueue_collapses_burst_and_session_close_takes_precedence() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_queue_db(&db_path);

    enqueue(
        &conn,
        "s1",
        "conversations/2026-05-03/s1.md",
        ExtractionTriggerKind::Debounce,
        "2026-05-03T10:00:05Z",
    )
    .unwrap();
    enqueue(
        &conn,
        "s1",
        "conversations/2026-05-03/s1.md",
        ExtractionTriggerKind::Debounce,
        "2026-05-03T10:00:10Z",
    )
    .unwrap();
    enqueue(
        &conn,
        "s1",
        "conversations/2026-05-03/s1.md",
        ExtractionTriggerKind::SessionClose,
        "2026-05-03T10:00:01Z",
    )
    .unwrap();

    let row: (i64, String, String) = conn
        .query_row(
            "SELECT COUNT(*), trigger_kind, scheduled_for
             FROM extraction_queue
             WHERE session_id = 's1' AND status = 'pending'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();

    assert_eq!(row.0, 1);
    assert_eq!(row.1, "session_close");
    assert_eq!(row.2, "2026-05-03T10:00:01Z");
}

#[test]
fn dequeue_returns_jobs_in_scheduled_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_queue_db(&db_path);

    enqueue(
        &conn,
        "slow",
        "conversations/2026-05-03/slow.md",
        ExtractionTriggerKind::Debounce,
        "2000-01-01T00:00:05Z",
    )
    .unwrap();
    enqueue(
        &conn,
        "fast",
        "conversations/2026-05-03/fast.md",
        ExtractionTriggerKind::Debounce,
        "2000-01-01T00:00:01Z",
    )
    .unwrap();

    let first = dequeue(&conn).unwrap().unwrap();
    let second = dequeue(&conn).unwrap().unwrap();

    assert_eq!(first.session_id, "fast");
    assert_eq!(second.session_id, "slow");
}

#[test]
fn concurrent_dequeue_claims_single_pending_row_once() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_queue_db(&db_path);
    enqueue(
        &conn,
        "s1",
        "conversations/2026-05-03/s1.md",
        ExtractionTriggerKind::Debounce,
        "2000-01-01T00:00:01Z",
    )
    .unwrap();
    drop(conn);

    let barrier = Arc::new(Barrier::new(3));
    let first_path = db_path.clone();
    let second_path = db_path.clone();
    let first_barrier = barrier.clone();
    let second_barrier = barrier.clone();

    let first = thread::spawn(move || {
        let conn = open_queue_db(&first_path);
        first_barrier.wait();
        dequeue(&conn).unwrap().map(|job| job.id)
    });
    let second = thread::spawn(move || {
        let conn = open_queue_db(&second_path);
        second_barrier.wait();
        dequeue(&conn).unwrap().map(|job| job.id)
    });

    barrier.wait();
    let results = [first.join().unwrap(), second.join().unwrap()];
    let claimed = results.iter().filter(|job| job.is_some()).count();

    assert_eq!(claimed, 1);
}

#[test]
fn mark_failed_retries_then_marks_failed_at_cap() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_queue_db(&db_path);
    enqueue(
        &conn,
        "s1",
        "conversations/2026-05-03/s1.md",
        ExtractionTriggerKind::Debounce,
        "2000-01-01T00:00:01Z",
    )
    .unwrap();
    let job = dequeue(&conn).unwrap().unwrap();

    mark_failed(&conn, job.id, job.attempts, "first").unwrap();
    let retried: (i64, String, String) = conn
        .query_row(
            "SELECT attempts, status, last_error FROM extraction_queue WHERE id = ?1",
            [job.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(retried.0, 1);
    assert_eq!(retried.1, "pending");
    assert_eq!(retried.2, "first");

    let second_job = dequeue(&conn).unwrap().unwrap();
    mark_failed(&conn, second_job.id, second_job.attempts, "second").unwrap();
    let third_job = dequeue(&conn).unwrap().unwrap();
    mark_failed(&conn, third_job.id, third_job.attempts, "third").unwrap();
    let failed: (i64, String) = conn
        .query_row(
            "SELECT attempts, status FROM extraction_queue WHERE id = ?1",
            [job.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(failed.0, 3);
    assert_eq!(failed.1, "failed");
}

#[test]
fn lease_expiry_recovers_running_job_and_increments_attempts() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_queue_db(&db_path);
    conn.execute(
        "INSERT INTO extraction_queue
             (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
         VALUES
             ('s1', 'conversations/2026-05-03/s1.md', 'debounce', '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', 0, NULL, 'running')",
        [],
    )
    .unwrap();

    let recovered = dequeue(&conn).unwrap().unwrap();
    let row: (i64, String) = conn
        .query_row(
            "SELECT attempts, status FROM extraction_queue WHERE id = ?1",
            [recovered.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(recovered.attempts, 1);
    assert_eq!(row.0, 1);
    assert_eq!(row.1, "running");
}

#[test]
fn queue_rows_survive_reopen_and_done_rows_stop_dequeueing() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_queue_db(&db_path);
    enqueue(
        &conn,
        "s1",
        "conversations/2026-05-03/s1.md",
        ExtractionTriggerKind::Debounce,
        "2000-01-01T00:00:01Z",
    )
    .unwrap();
    drop(conn);

    let conn = open_queue_db(&db_path);
    let claimed = dequeue(&conn).unwrap().unwrap();
    mark_done(&conn, claimed.id, claimed.attempts).unwrap();

    assert_eq!(dequeue(&conn).unwrap(), None);
    let status: String = conn
        .query_row(
            "SELECT status FROM extraction_queue WHERE id = ?1",
            [claimed.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, ExtractionJobStatus::Done.as_str());
}

#[test]
fn stale_worker_cannot_finish_released_job_after_lease_expiry() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_queue_db(&db_path);
    conn.execute(
        "INSERT INTO extraction_queue
             (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
         VALUES
             ('s1', 'conversations/2026-05-03/s1.md', 'debounce', '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', 0, NULL, 'running')",
        [],
    )
    .unwrap();

    let recovered = dequeue(&conn).unwrap().unwrap();
    let stale_done = mark_done(&conn, recovered.id, 0).unwrap_err();
    let stale_failed = mark_failed(&conn, recovered.id, 0, "late failure").unwrap_err();

    assert!(matches!(
        stale_done,
        quaid::core::conversation::queue::ExtractionQueueError::StaleLease { .. }
    ));
    assert!(matches!(
        stale_failed,
        quaid::core::conversation::queue::ExtractionQueueError::StaleLease { .. }
    ));

    mark_done(&conn, recovered.id, recovered.attempts).unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM extraction_queue WHERE id = ?1",
            [recovered.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, ExtractionJobStatus::Done.as_str());
}
