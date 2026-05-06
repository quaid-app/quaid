use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

use crate::core::db;
use crate::core::types::{ExtractionJob, ExtractionTriggerKind};

pub const DEFAULT_EXTRACTION_MAX_RETRIES: i64 = 3;
pub const DEFAULT_LEASE_EXPIRY_SECONDS: i64 = 300;

#[derive(Debug, Error)]
pub enum ExtractionQueueError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("config error: {message}")]
    Config { message: String },

    #[error("stale extraction lease for job {job_id} attempt {attempts}")]
    StaleLease { job_id: i64, attempts: i64 },
}

/// Enqueue a debounce / session_close / non-force manual job, collapsing
/// pending rows per `session_id`. Force-reset enqueues use
/// [`enqueue_force_path`] instead so each day-file gets its own pending row.
pub fn enqueue(
    conn: &Connection,
    session_id: &str,
    conversation_path: &str,
    trigger_kind: ExtractionTriggerKind,
    scheduled_for: &str,
) -> Result<(), ExtractionQueueError> {
    with_immediate_transaction(conn, |conn| {
        let existing = conn
            .query_row(
                "SELECT id, trigger_kind, scheduled_for
                 FROM extraction_queue
                 WHERE session_id = ?1 AND status = 'pending'
                 ORDER BY id
                 LIMIT 1",
                [session_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((job_id, existing_trigger, existing_scheduled_for)) = existing {
            let existing_trigger = existing_trigger
                .parse::<ExtractionTriggerKind>()
                .map_err(|message| ExtractionQueueError::Config { message })?;
            let (trigger_kind, scheduled_for) = merged_pending_job(
                existing_trigger,
                &existing_scheduled_for,
                trigger_kind,
                scheduled_for,
            );
            conn.execute(
                "UPDATE extraction_queue
                 SET conversation_path = ?2,
                     trigger_kind = ?3,
                     scheduled_for = ?4,
                     attempts = 0,
                     last_error = NULL,
                     enqueued_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE id = ?1",
                params![
                    job_id,
                    conversation_path,
                    trigger_kind.as_str(),
                    scheduled_for
                ],
            )?;
        } else {
            conn.execute(
                "INSERT INTO extraction_queue
                     (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
                 VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?4, 0, NULL, 'pending')",
                params![session_id, conversation_path, trigger_kind.as_str(), scheduled_for],
            )?;
        }

        Ok(())
    })
}

/// Enqueue a manual job for a specific `(session_id, conversation_path)` pair,
/// idempotent within that pair so back-to-back force resets don't grow the
/// queue. Unlike [`enqueue`], this does NOT collapse across day-files of the
/// same session — `extract --force` against a multi-day session produces one
/// pending row per day-file, all with `trigger_kind = 'manual'`. If a pending
/// debounce row already exists for the same `(session_id, conversation_path)`,
/// it is upgraded in place to `manual`.
pub fn enqueue_force_path(
    conn: &Connection,
    session_id: &str,
    conversation_path: &str,
    scheduled_for: &str,
) -> Result<(), ExtractionQueueError> {
    with_immediate_transaction(conn, |conn| {
        let existing = conn
            .query_row(
                "SELECT id
                 FROM extraction_queue
                 WHERE session_id = ?1 AND conversation_path = ?2 AND status = 'pending'
                 ORDER BY id
                 LIMIT 1",
                params![session_id, conversation_path],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;

        if let Some(job_id) = existing {
            conn.execute(
                "UPDATE extraction_queue
                 SET trigger_kind = 'manual',
                     scheduled_for = ?2,
                     attempts = 0,
                     last_error = NULL,
                     enqueued_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE id = ?1",
                params![job_id, scheduled_for],
            )?;
        } else {
            conn.execute(
                "INSERT INTO extraction_queue
                     (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
                 VALUES (?1, ?2, 'manual', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?3, 0, NULL, 'pending')",
                params![session_id, conversation_path, scheduled_for],
            )?;
        }

        Ok(())
    })
}

pub fn session_queue_key(namespace: Option<&str>, session_id: &str) -> String {
    match namespace.filter(|value| !value.is_empty()) {
        Some(namespace) => format!("{namespace}::{session_id}"),
        None => session_id.to_owned(),
    }
}

pub fn scheduled_timestamp_after_ms(
    conn: &Connection,
    offset_ms: i64,
) -> Result<String, ExtractionQueueError> {
    conn.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%SZ', julianday('now') + (?1 / 86400000.0))",
        [offset_ms],
        |row| row.get(0),
    )
    .map_err(ExtractionQueueError::from)
}

pub fn pending_queue_position(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<u32>, ExtractionQueueError> {
    let pending = conn
        .query_row(
            "SELECT id, scheduled_for
             FROM extraction_queue
             WHERE session_id = ?1 AND status = 'pending'
             ORDER BY scheduled_for, id
             LIMIT 1",
            [session_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;

    let Some((job_id, scheduled_for)) = pending else {
        return Ok(None);
    };

    let position: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM extraction_queue
         WHERE status = 'pending'
           AND (scheduled_for < ?1 OR (scheduled_for = ?1 AND id <= ?2))",
        params![scheduled_for, job_id],
        |row| row.get(0),
    )?;
    Ok(Some(position as u32))
}

pub fn dequeue(conn: &Connection) -> Result<Option<ExtractionJob>, ExtractionQueueError> {
    recover_expired_leases(conn)?;
    let now = current_timestamp(conn)?;
    let mut stmt = conn.prepare(
        "UPDATE extraction_queue
         SET status = 'running',
             scheduled_for = ?1
         WHERE id = (
             SELECT id
             FROM extraction_queue
             WHERE status = 'pending' AND scheduled_for <= ?1
             ORDER BY scheduled_for, id
             LIMIT 1
         )
           AND status = 'pending'
         RETURNING id, session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status",
    )?;

    stmt.query_row([&now], read_job)
        .optional()
        .map_err(ExtractionQueueError::from)
}

pub fn mark_done(
    conn: &Connection,
    job_id: i64,
    attempts: i64,
) -> Result<(), ExtractionQueueError> {
    let updated = conn.execute(
        "UPDATE extraction_queue
         SET status = 'done',
              last_error = NULL
         WHERE id = ?1 AND status = 'running' AND attempts = ?2",
        params![job_id, attempts],
    )?;
    if updated == 0 {
        return Err(ExtractionQueueError::StaleLease { job_id, attempts });
    }
    Ok(())
}

pub fn mark_failed(
    conn: &Connection,
    job_id: i64,
    attempts: i64,
    error_message: &str,
) -> Result<(), ExtractionQueueError> {
    let max_retries = max_retries(conn)?;
    let updated = conn.execute(
        "UPDATE extraction_queue
         SET attempts = attempts + 1,
              last_error = ?3,
              status = CASE
                  WHEN attempts + 1 >= ?2 THEN 'failed'
                  ELSE 'pending'
              END
         WHERE id = ?1 AND status = 'running' AND attempts = ?4",
        params![job_id, max_retries, error_message, attempts],
    )?;
    if updated == 0 {
        return Err(ExtractionQueueError::StaleLease { job_id, attempts });
    }
    Ok(())
}

fn recover_expired_leases(conn: &Connection) -> Result<(), ExtractionQueueError> {
    let max_retries = max_retries(conn)?;
    conn.execute(
        "UPDATE extraction_queue
         SET attempts = attempts + 1,
             status = CASE
                 WHEN attempts + 1 >= ?1 THEN 'failed'
                 ELSE 'pending'
             END,
             last_error = COALESCE(last_error, 'lease expired')
         WHERE status = 'running'
           AND julianday('now') >= julianday(scheduled_for) + (?2 / 86400.0)",
        params![max_retries, DEFAULT_LEASE_EXPIRY_SECONDS],
    )?;
    Ok(())
}

fn merged_pending_job(
    existing_trigger: ExtractionTriggerKind,
    existing_scheduled_for: &str,
    new_trigger: ExtractionTriggerKind,
    new_scheduled_for: &str,
) -> (ExtractionTriggerKind, String) {
    match (existing_trigger, new_trigger) {
        (ExtractionTriggerKind::SessionClose, _) => (
            ExtractionTriggerKind::SessionClose,
            existing_scheduled_for.to_owned(),
        ),
        (_, ExtractionTriggerKind::SessionClose) => (
            ExtractionTriggerKind::SessionClose,
            std::cmp::min(existing_scheduled_for, new_scheduled_for).to_owned(),
        ),
        (ExtractionTriggerKind::Debounce, ExtractionTriggerKind::Debounce) => (
            ExtractionTriggerKind::Debounce,
            std::cmp::max(existing_scheduled_for, new_scheduled_for).to_owned(),
        ),
        (ExtractionTriggerKind::Manual, ExtractionTriggerKind::Debounce) => (
            ExtractionTriggerKind::Manual,
            existing_scheduled_for.to_owned(),
        ),
        (ExtractionTriggerKind::Debounce, ExtractionTriggerKind::Manual) => {
            (ExtractionTriggerKind::Manual, new_scheduled_for.to_owned())
        }
        (ExtractionTriggerKind::Manual, ExtractionTriggerKind::Manual) => (
            ExtractionTriggerKind::Manual,
            std::cmp::min(existing_scheduled_for, new_scheduled_for).to_owned(),
        ),
    }
}

fn max_retries(conn: &Connection) -> Result<i64, ExtractionQueueError> {
    let raw = db::read_config_value_or(
        conn,
        "extraction.max_retries",
        &DEFAULT_EXTRACTION_MAX_RETRIES.to_string(),
    )
    .map_err(|error| ExtractionQueueError::Config {
        message: error.to_string(),
    })?;

    raw.parse::<i64>()
        .map_err(|_| ExtractionQueueError::Config {
            message: format!("invalid extraction.max_retries value: {raw}"),
        })
}

pub fn current_timestamp(conn: &Connection) -> Result<String, ExtractionQueueError> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })
    .map_err(ExtractionQueueError::from)
}

fn read_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExtractionJob> {
    Ok(ExtractionJob {
        id: row.get(0)?,
        session_id: row.get(1)?,
        conversation_path: row.get(2)?,
        trigger_kind: row
            .get::<_, String>(3)?
            .parse()
            .map_err(|message: String| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::other(message)),
                )
            })?,
        enqueued_at: row.get(4)?,
        scheduled_for: row.get(5)?,
        attempts: row.get(6)?,
        last_error: row.get(7)?,
        status: row
            .get::<_, String>(8)?
            .parse()
            .map_err(|message: String| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::other(message)),
                )
            })?,
    })
}

fn with_immediate_transaction<T>(
    conn: &Connection,
    action: impl FnOnce(&Connection) -> Result<T, ExtractionQueueError>,
) -> Result<T, ExtractionQueueError> {
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;
    match action(conn) {
        Ok(value) => match conn.execute_batch("COMMIT TRANSACTION") {
            Ok(()) => Ok(value),
            Err(commit_error) => {
                // SQLITE_BUSY on COMMIT does not auto-rollback, so the transaction
                // would otherwise stay open and wedge subsequent BEGIN IMMEDIATEs
                // on this shared connection.
                let _ = conn.execute_batch("ROLLBACK TRANSACTION");
                Err(ExtractionQueueError::from(commit_error))
            }
        },
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK TRANSACTION");
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn configured_connection() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        conn.execute(
            "UPDATE collections
             SET root_path = ?1,
                 state = 'active'
             WHERE id = 1",
            [dir.path().display().to_string()],
        )
        .unwrap();
        std::mem::forget(dir);
        conn
    }

    #[test]
    fn merged_pending_job_keeps_existing_session_close_over_later_debounce() {
        let (trigger, scheduled_for) = merged_pending_job(
            ExtractionTriggerKind::SessionClose,
            "2026-05-03T10:00:01Z",
            ExtractionTriggerKind::Debounce,
            "2026-05-03T10:00:05Z",
        );

        assert_eq!(trigger, ExtractionTriggerKind::SessionClose);
        assert_eq!(scheduled_for, "2026-05-03T10:00:01Z");
    }

    #[test]
    fn merged_pending_job_prefers_manual_over_debounce_and_keeps_earliest_manual() {
        let (existing_manual_trigger, existing_manual_schedule) = merged_pending_job(
            ExtractionTriggerKind::Manual,
            "2026-05-03T10:00:01Z",
            ExtractionTriggerKind::Debounce,
            "2026-05-03T10:00:05Z",
        );
        let (upgraded_trigger, upgraded_schedule) = merged_pending_job(
            ExtractionTriggerKind::Debounce,
            "2026-05-03T10:00:05Z",
            ExtractionTriggerKind::Manual,
            "2026-05-03T10:00:02Z",
        );
        let (earliest_manual_trigger, earliest_manual_schedule) = merged_pending_job(
            ExtractionTriggerKind::Manual,
            "2026-05-03T10:00:05Z",
            ExtractionTriggerKind::Manual,
            "2026-05-03T10:00:02Z",
        );

        assert_eq!(existing_manual_trigger, ExtractionTriggerKind::Manual);
        assert_eq!(existing_manual_schedule, "2026-05-03T10:00:01Z");
        assert_eq!(upgraded_trigger, ExtractionTriggerKind::Manual);
        assert_eq!(upgraded_schedule, "2026-05-03T10:00:02Z");
        assert_eq!(earliest_manual_trigger, ExtractionTriggerKind::Manual);
        assert_eq!(earliest_manual_schedule, "2026-05-03T10:00:02Z");
    }

    #[test]
    fn enqueue_does_not_collapse_with_running_rows() {
        let conn = configured_connection();
        conn.execute(
            "INSERT INTO extraction_queue
                 (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
             VALUES
                 ('s1', 'conversations/2026-05-03/s1.md', 'debounce', '2026-05-03T10:00:00Z', '2026-05-03T10:00:00Z', 0, NULL, 'running')",
            [],
        )
        .unwrap();

        enqueue(
            &conn,
            "s1",
            "conversations/2026-05-03/s1.md",
            ExtractionTriggerKind::Debounce,
            "2026-05-03T10:00:05Z",
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM extraction_queue WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn enqueue_merge_resets_attempts_and_clears_last_error() {
        let conn = configured_connection();
        conn.execute(
            "INSERT INTO extraction_queue
                 (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
             VALUES
                 ('s1', 'conversations/2026-05-03/original.md', 'debounce', '2026-05-03T10:00:00Z', '2026-05-03T10:00:00Z', 2, 'old failure', 'pending')",
            [],
        )
        .unwrap();

        enqueue(
            &conn,
            "s1",
            "conversations/2026-05-03/updated.md",
            ExtractionTriggerKind::Manual,
            "2026-05-03T10:00:02Z",
        )
        .unwrap();

        let row: (String, String, i64, Option<String>) = conn
            .query_row(
                "SELECT conversation_path, trigger_kind, attempts, last_error
                 FROM extraction_queue
                 WHERE session_id = 's1' AND status = 'pending'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(row.0, "conversations/2026-05-03/updated.md");
        assert_eq!(row.1, "manual");
        assert_eq!(row.2, 0);
        assert_eq!(row.3, None);
    }

    #[test]
    fn dequeue_returns_none_when_only_future_jobs_exist() {
        let conn = configured_connection();
        enqueue(
            &conn,
            "future",
            "conversations/2099-01-01/future.md",
            ExtractionTriggerKind::Debounce,
            "2099-01-01T00:00:00Z",
        )
        .unwrap();

        assert_eq!(dequeue(&conn).unwrap(), None);
    }

    #[test]
    fn max_retries_rejects_non_numeric_config() {
        let conn = configured_connection();
        conn.execute(
            "UPDATE config SET value = 'oops' WHERE key = 'extraction.max_retries'",
            [],
        )
        .unwrap();

        let error = max_retries(&conn).unwrap_err();

        assert!(error.to_string().contains("invalid extraction.max_retries"));
    }

    #[test]
    fn session_queue_key_prefixes_namespace_only_when_present() {
        assert_eq!(session_queue_key(None, "s1"), "s1");
        assert_eq!(session_queue_key(Some(""), "s1"), "s1");
        assert_eq!(session_queue_key(Some("alpha"), "s1"), "alpha::s1");
    }

    #[test]
    fn pending_queue_position_counts_jobs_ahead_of_session() {
        let conn = configured_connection();
        enqueue(
            &conn,
            "first",
            "conversations/2026-05-03/first.md",
            ExtractionTriggerKind::Debounce,
            "2000-01-01T00:00:01Z",
        )
        .unwrap();
        enqueue(
            &conn,
            "second",
            "conversations/2026-05-03/second.md",
            ExtractionTriggerKind::Debounce,
            "2000-01-01T00:00:02Z",
        )
        .unwrap();

        assert_eq!(pending_queue_position(&conn, "first").unwrap(), Some(1));
        assert_eq!(pending_queue_position(&conn, "second").unwrap(), Some(2));
        assert_eq!(pending_queue_position(&conn, "missing").unwrap(), None);
    }

    #[test]
    fn scheduled_timestamp_after_ms_returns_iso_timestamp() {
        let conn = configured_connection();

        let scheduled = scheduled_timestamp_after_ms(&conn, 5000).unwrap();

        assert_eq!(scheduled.len(), 20);
        assert!(scheduled.ends_with('Z'));
    }

    #[test]
    fn with_immediate_transaction_recovers_when_commit_is_aborted() {
        // Force the next COMMIT to fail by registering a commit_hook that
        // returns true (aborts the commit, surfacing as a SQLite error).
        let conn = configured_connection();
        conn.commit_hook(Some(|| true));

        let aborted = enqueue(
            &conn,
            "s1",
            "conversations/2026-05-03/s1.md",
            ExtractionTriggerKind::Debounce,
            "2026-05-03T10:00:00Z",
        );
        assert!(
            aborted.is_err(),
            "commit_hook abort must surface as ExtractionQueueError"
        );

        // Clear the hook so the recovery enqueue can commit normally. If the
        // wrapper failed to roll back after the aborted commit, the next
        // BEGIN IMMEDIATE will fail with "cannot start a transaction within a
        // transaction" and this call will return Err.
        conn.commit_hook::<fn() -> bool>(None);

        enqueue(
            &conn,
            "s2",
            "conversations/2026-05-03/s2.md",
            ExtractionTriggerKind::Debounce,
            "2026-05-03T10:00:00Z",
        )
        .expect(
            "follow-up enqueue must succeed; connection must not be wedged inside a transaction \
             after an aborted commit",
        );

        // The aborted enqueue must have left no row behind.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM extraction_queue WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "aborted commit must roll back its inserts");
    }

    #[test]
    fn lease_recovery_preserves_existing_last_error_and_can_fail_at_retry_cap() {
        let conn = configured_connection();
        conn.execute(
            "UPDATE config SET value = '2' WHERE key = 'extraction.max_retries'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO extraction_queue
                 (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
             VALUES
                 ('preserve', 'conversations/2026-05-03/preserve.md', 'debounce', '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', 0, 'worker panic', 'running'),
                 ('fail-now', 'conversations/2026-05-03/fail-now.md', 'debounce', '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', 1, NULL, 'running')",
            [],
        )
        .unwrap();

        let recovered = dequeue(&conn).unwrap().unwrap();
        let preserved_row: (i64, String, String) = conn
            .query_row(
                "SELECT attempts, status, last_error FROM extraction_queue WHERE session_id = 'preserve'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let failed_row: (i64, String, String) = conn
            .query_row(
                "SELECT attempts, status, last_error FROM extraction_queue WHERE session_id = 'fail-now'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(recovered.session_id, "preserve");
        assert_eq!(preserved_row.0, 1);
        assert_eq!(preserved_row.1, "running");
        assert_eq!(preserved_row.2, "worker panic");
        assert_eq!(failed_row.0, 2);
        assert_eq!(failed_row.1, "failed");
        assert_eq!(failed_row.2, "lease expired");
    }
}
