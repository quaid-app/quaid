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

fn current_timestamp(conn: &Connection) -> Result<String, ExtractionQueueError> {
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
        Ok(value) => {
            conn.execute_batch("COMMIT TRANSACTION")?;
            Ok(value)
        }
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
}
