//! Serve and CLI session lifecycle: register, heartbeat, unregister,
//! and sweep.
//!
//! Sessions live in the `serve_sessions` table. Two flavours exist:
//! `serve` sessions (a long-running watcher/IPC owner) and `cli`
//! sessions (a short-lived single-command owner). Both share the
//! same heartbeat liveness threshold, `SESSION_LIVENESS_SECS`, owned
//! by the parent module.
//!
//! [`register_session`] and [`register_cli_session`] insert a row;
//! [`unregister_session`] removes it transactionally together with
//! any owner-lease rows that pointed at it (so the session row and
//! the ownership rows that depend on it never go out of sync).
//! [`heartbeat_session`] is the keepalive; [`sweep_stale_sessions`]
//! is the per-tick GC the supervisor runs against the global
//! `serve_sessions` table.
//!
//! `current_host()` and `SESSION_LIVENESS_SECS` are imported from
//! the parent module rather than redefined here so the host-name and
//! heartbeat threshold remain a single source of truth across the
//! crate.

use rusqlite::{params, Connection};
use uuid::Uuid;

use super::{current_host, VaultSyncError, SESSION_LIVENESS_SECS};

/// Inserts a new long-running `serve` session row and returns its
/// generated `session_id` — the handle every subsequent ownership
/// and heartbeat operation pivots on.
pub fn register_session(conn: &Connection) -> Result<String, VaultSyncError> {
    let session_id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES (?1, ?2, ?3)",
        params![session_id, std::process::id() as i64, current_host()],
    )?;
    Ok(session_id)
}

/// Inserts a short-lived `cli` session row used by single-command
/// operations that need ownership semantics without a watcher loop.
pub fn register_cli_session(conn: &Connection) -> Result<String, VaultSyncError> {
    let session_id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type) VALUES (?1, ?2, ?3, 'cli')",
        params![session_id, std::process::id() as i64, current_host()],
    )?;
    Ok(session_id)
}

/// Atomically removes a session and any owner / lease rows that
/// referenced it so the session table and the lease columns on
/// `collections` never drift out of sync.
pub fn unregister_session(conn: &Connection, session_id: &str) -> Result<(), VaultSyncError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM collection_owners WHERE session_id = ?1",
        [session_id],
    )?;
    tx.execute(
        "DELETE FROM serve_sessions WHERE session_id = ?1",
        [session_id],
    )?;
    tx.execute(
        "UPDATE collections
         SET active_lease_session_id = CASE
                 WHEN active_lease_session_id = ?1 THEN NULL
                 ELSE active_lease_session_id
             END,
             restore_lease_session_id = CASE
                 WHEN restore_lease_session_id = ?1 THEN NULL
                 ELSE restore_lease_session_id
             END,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE active_lease_session_id = ?1 OR restore_lease_session_id = ?1",
        [session_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Refreshes the `heartbeat_at` timestamp for a session so it stays
/// inside the liveness window observed by ownership checks and the
/// stale-session sweeper.
pub fn heartbeat_session(conn: &Connection, session_id: &str) -> Result<(), VaultSyncError> {
    conn.execute(
        "UPDATE serve_sessions
         SET heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE session_id = ?1",
        [session_id],
    )?;
    Ok(())
}

/// Deletes session rows whose `heartbeat_at` has fallen outside the
/// liveness window and returns the number reaped — the GC the
/// supervisor runs each tick to keep `serve_sessions` bounded.
pub fn sweep_stale_sessions(conn: &Connection) -> Result<usize, VaultSyncError> {
    let removed = conn.execute(
        "DELETE FROM serve_sessions
         WHERE heartbeat_at < datetime('now', ?1)",
        [format!("-{SESSION_LIVENESS_SECS} seconds")],
    )?;
    Ok(removed)
}
