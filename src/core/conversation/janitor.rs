//! Periodic cleanup tick for conversation-side state.
//!
//! The janitor runs from the same loop that processes the extraction
//! queue; each tick deletes `done` / `failed` extraction-queue rows
//! older than the retention window and flips lapsed
//! `correction_sessions` rows from `open` to `expired`. Live `pending`
//! and `running` queue rows are never touched, and committed or
//! abandoned correction sessions are left alone.
//!
//! See also: `super::queue` for the extraction-queue rows being
//! purged, and `super::correction` for the correction-session
//! lifecycle whose expiry this enforces.

use rusqlite::{params, Connection};
use thiserror::Error;

use crate::core::db;

const DEFAULT_RETENTION_DAYS: i64 = 30;

/// Errors surfaced by janitor passes.
#[derive(Debug, Error)]
pub enum JanitorError {
    /// Underlying SQLite failure during a cleanup statement.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Janitor config value was missing or malformed (e.g.
    /// non-numeric or negative `extraction.retention_days`).
    #[error("config error: {message}")]
    Config {
        /// Human-readable description of the offending config value.
        message: String,
    },
}

/// Per-tick counts emitted by [`run_tick`] so callers can log or
/// surface janitor activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JanitorResult {
    /// Number of terminal `extraction_queue` rows deleted this tick.
    pub queue_rows_purged: u64,
    /// Number of `correction_sessions` rows transitioned from
    /// `open` to `expired` this tick.
    pub correction_sessions_expired: u64,
}

/// Runs both janitor operations (queue purge + correction-session expiry) in a single call.
pub fn run_tick(conn: &Connection) -> Result<JanitorResult, JanitorError> {
    let now = current_timestamp(conn)?;
    run_tick_at(conn, &now)
}

/// Run both janitor passes against an explicit `now` timestamp; the
/// time-injection form used by [`run_tick`] and by tests that need
/// deterministic clock control.
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

/// Time-injection variant of [`purge_old_queue_rows`]: compares
/// `enqueued_at` against the caller-supplied `now` instead of the
/// SQLite clock.
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

/// Time-injection variant of
/// [`expire_stale_correction_sessions`]: compares `expires_at`
/// against the caller-supplied `now`.
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
