use rusqlite::{params, Connection};
use thiserror::Error;

use crate::core::db;

const DEFAULT_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Error)]
pub enum JanitorError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("config error: {message}")]
    Config { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JanitorResult {
    pub queue_rows_purged: u64,
    pub correction_sessions_expired: u64,
}

/// Runs both janitor operations (queue purge + correction-session expiry) in a single call.
pub fn run_tick(conn: &Connection) -> Result<JanitorResult, JanitorError> {
    let now = current_timestamp(conn)?;
    run_tick_at(conn, &now)
}

pub fn run_tick_at(conn: &Connection, now: &str) -> Result<JanitorResult, JanitorError> {
    let queue_rows_purged = purge_old_queue_rows_at(conn, now)?;
    let correction_sessions_expired = expire_stale_correction_sessions_at(conn, now)?;
    Ok(JanitorResult {
        queue_rows_purged,
        correction_sessions_expired,
    })
}

/// Deletes `extraction_queue` rows whose `status IN ('done', 'failed')` and whose
/// `enqueued_at` is older than `extraction.retention_days` days.  `pending` and
/// `running` rows are never touched.
pub fn purge_old_queue_rows(conn: &Connection) -> Result<u64, JanitorError> {
    let now = current_timestamp(conn)?;
    purge_old_queue_rows_at(conn, &now)
}

pub fn purge_old_queue_rows_at(conn: &Connection, now: &str) -> Result<u64, JanitorError> {
    let retention_days = retention_days(conn)?;
    let changed = conn.execute(
        "DELETE FROM extraction_queue
         WHERE status IN ('done', 'failed')
            AND julianday(enqueued_at) < julianday(?1) - ?2",
        params![now, retention_days],
    )?;
    Ok(changed as u64)
}

/// Updates `correction_sessions` rows that are still `open` but whose `expires_at`
/// is in the past to `status = 'expired'`.  Sessions already committed or abandoned
/// are never modified.
pub fn expire_stale_correction_sessions(conn: &Connection) -> Result<u64, JanitorError> {
    let now = current_timestamp(conn)?;
    expire_stale_correction_sessions_at(conn, &now)
}

pub fn expire_stale_correction_sessions_at(
    conn: &Connection,
    now: &str,
) -> Result<u64, JanitorError> {
    let changed = conn.execute(
        "UPDATE correction_sessions
         SET status = 'expired'
         WHERE status = 'open'
            AND julianday(expires_at) < julianday(?1)",
        [now],
    )?;
    Ok(changed as u64)
}

fn retention_days(conn: &Connection) -> Result<i64, JanitorError> {
    let raw = db::read_config_value_or(
        conn,
        "extraction.retention_days",
        &DEFAULT_RETENTION_DAYS.to_string(),
    )
    .map_err(|error| JanitorError::Config {
        message: error.to_string(),
    })?;
    let retention_days = raw.parse::<i64>().map_err(|_| JanitorError::Config {
        message: format!("invalid extraction.retention_days value: {raw}"),
    })?;
    if retention_days < 0 {
        return Err(JanitorError::Config {
            message: format!("invalid extraction.retention_days value: {raw}"),
        });
    }
    Ok(retention_days)
}

fn current_timestamp(conn: &Connection) -> Result<String, JanitorError> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })
    .map_err(JanitorError::from)
}
