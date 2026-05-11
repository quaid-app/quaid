//! Serve and CLI session lifecycle: register, heartbeat, unregister,
//! and sweep.
//!
//! Sessions live in the `serve_sessions` table. The `session_type`
//! column distinguishes four flavours:
//!
//! - `daemon` — long-running runtime under launchd/systemd (`quaid daemon run`).
//!   Unique per database; owns watchers + extraction worker + every supervised duty.
//! - `serve_host` — no-daemon-fallback runtime owner. Promoted from
//!   `serve` via [`try_promote_to_serve_host`] when no `daemon` is registered.
//!   Unique per database while the promotion holds.
//! - `serve` — transport-only MCP session. Coexists with a live `daemon`
//!   or `serve_host` and owns nothing beyond its MCP transport.
//! - `cli` — short-lived single-command owner (e.g. `quaid get`, `quaid put`).
//!
//! Heartbeat liveness threshold `SESSION_LIVENESS_SECS` is owned by the
//! parent module and applies uniformly across all four session types.
//!
//! [`register_session`] and [`register_cli_session`] insert a row;
//! [`unregister_session`] removes it transactionally together with
//! any owner-lease rows that pointed at it (so the session row and
//! the ownership rows that depend on it never go out of sync).
//! [`heartbeat_session`] is the keepalive; [`sweep_stale_sessions`]
//! is the per-tick GC the supervisor runs against the global
//! `serve_sessions` table.
//!
//! [`try_promote_to_serve_host`] is the single-transaction lease-claim
//! used by `quaid serve` to elect the no-daemon runtime owner; it
//! sweeps stale rows, refuses promotion when a live `daemon` or
//! `serve_host` already exists, and atomically updates the caller's
//! `session_type` from `'serve'` to `'serve_host'` on success.
//!
//! `current_host()` and `SESSION_LIVENESS_SECS` are imported from
//! the parent module rather than redefined here so the host-name and
//! heartbeat threshold remain a single source of truth across the
//! crate.

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use super::{current_host, VaultSyncError, SESSION_LIVENESS_SECS};

/// Distinguishes the four `serve_sessions.session_type` values
/// recognized by the daemon-and-http-transport runtime ownership
/// model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionType {
    /// `quaid daemon run` — long-running runtime under launchd/systemd.
    /// Unique per database; the canonical runtime owner.
    Daemon,
    /// A `quaid serve` instance that has been promoted to runtime
    /// owner via [`try_promote_to_serve_host`] because no `daemon`
    /// session was registered. Unique per database while live.
    ServeHost,
    /// A transport-only `quaid serve` instance — owns its MCP
    /// transport and nothing else. Coexists with a live `Daemon` or
    /// `ServeHost`.
    Serve,
    /// A short-lived `quaid <one-shot>` invocation (e.g. `quaid get`).
    Cli,
}

impl SessionType {
    /// Maps the enum to the string persisted in
    /// `serve_sessions.session_type`.
    pub fn to_db_str(self) -> &'static str {
        match self {
            SessionType::Daemon => "daemon",
            SessionType::ServeHost => "serve_host",
            SessionType::Serve => "serve",
            SessionType::Cli => "cli",
        }
    }

    /// Returns `true` when the session type represents a runtime
    /// owner — i.e., the process that hosts watchers, extraction
    /// worker, and every supervised background duty.
    pub fn is_runtime_host(self) -> bool {
        matches!(self, SessionType::Daemon | SessionType::ServeHost)
    }
}

/// Snapshot of a live `serve_sessions` row returned by the
/// `find_active_*` helpers — minimal shape consumers need to make
/// ownership decisions without re-querying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSessionInfo {
    /// The session row's primary-key `session_id`.
    pub session_id: String,
    /// OS pid that holds the session.
    pub pid: i64,
    /// Hostname reported by the holding process at registration.
    pub host: String,
    /// One of `'daemon' | 'serve_host' | 'serve' | 'cli'`.
    pub session_type: String,
}

/// Inserts a new session row of the requested [`SessionType`] and
/// returns its generated `session_id` — the handle every subsequent
/// ownership and heartbeat operation pivots on.
pub fn register_session(
    conn: &Connection,
    session_type: SessionType,
) -> Result<String, VaultSyncError> {
    let session_id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type) VALUES (?1, ?2, ?3, ?4)",
        params![
            session_id,
            std::process::id() as i64,
            current_host(),
            session_type.to_db_str()
        ],
    )?;
    Ok(session_id)
}

/// Inserts a short-lived `cli` session row used by single-command
/// operations that need ownership semantics without a watcher loop.
pub fn register_cli_session(conn: &Connection) -> Result<String, VaultSyncError> {
    register_session(conn, SessionType::Cli)
}

/// Returns the live `daemon` session for this database, or `None` if
/// none exists. Stale rows (heartbeat past `SESSION_LIVENESS_SECS`)
/// are filtered out at query time so this is safe to call without
/// running a sweep first; callers that want sweep-and-find atomicity
/// should wrap their own transaction.
pub fn find_active_daemon_session(
    conn: &Connection,
) -> Result<Option<ActiveSessionInfo>, VaultSyncError> {
    conn.query_row(
        "SELECT session_id, pid, host, session_type
         FROM serve_sessions
         WHERE session_type = 'daemon'
           AND heartbeat_at >= datetime('now', ?1)
         LIMIT 1",
        params![format!("-{SESSION_LIVENESS_SECS} seconds")],
        |row| {
            Ok(ActiveSessionInfo {
                session_id: row.get(0)?,
                pid: row.get(1)?,
                host: row.get(2)?,
                session_type: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Returns the live runtime-host session for this database — either
/// the `daemon` (preferred when both exist, though by invariant they
/// cannot) or the `serve_host`. `None` if no runtime host is live.
/// Used by `quaid status` and by the `serve` startup probe to decide
/// whether to attempt a `serve_host` promotion.
pub fn find_active_runtime_host(
    conn: &Connection,
) -> Result<Option<ActiveSessionInfo>, VaultSyncError> {
    conn.query_row(
        "SELECT session_id, pid, host, session_type
         FROM serve_sessions
         WHERE session_type IN ('daemon', 'serve_host')
           AND heartbeat_at >= datetime('now', ?1)
         ORDER BY CASE session_type WHEN 'daemon' THEN 0 ELSE 1 END
         LIMIT 1",
        params![format!("-{SESSION_LIVENESS_SECS} seconds")],
        |row| {
            Ok(ActiveSessionInfo {
                session_id: row.get(0)?,
                pid: row.get(1)?,
                host: row.get(2)?,
                session_type: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Atomically promotes the caller's session from `'serve'` to
/// `'serve_host'` when no live `daemon` or `serve_host` exists.
/// Returns `true` on successful promotion (or when the caller is
/// already `'serve_host'` — idempotent), `false` when another live
/// runtime owner already holds the lease.
///
/// Runs inside a single `BEGIN IMMEDIATE` transaction that
/// (a) sweeps stale rows, (b) checks for a live `daemon` or live
/// `serve_host`, (c) if both absent, UPDATEs the caller's row's
/// `session_type` from `'serve'` to `'serve_host'`. The
/// `BEGIN IMMEDIATE` guarantees concurrent invocations cannot both
/// observe the "no live owner" state and both promote.
pub fn try_promote_to_serve_host(
    conn: &Connection,
    session_id: &str,
) -> Result<bool, VaultSyncError> {
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;

    let result = try_promote_inner(conn, session_id);

    match &result {
        Ok(_) => {
            if let Err(commit_err) = conn.execute_batch("COMMIT TRANSACTION") {
                let _ = conn.execute_batch("ROLLBACK TRANSACTION");
                return Err(VaultSyncError::from(commit_err));
            }
        }
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK TRANSACTION");
        }
    }

    result
}

fn try_promote_inner(conn: &Connection, session_id: &str) -> Result<bool, VaultSyncError> {
    conn.execute(
        "DELETE FROM serve_sessions
         WHERE heartbeat_at < datetime('now', ?1)",
        [format!("-{SESSION_LIVENESS_SECS} seconds")],
    )?;

    let caller_type: Option<String> = conn
        .query_row(
            "SELECT session_type FROM serve_sessions WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?;

    match caller_type.as_deref() {
        Some("serve_host") => return Ok(true),
        Some("serve") => {} // proceed to promotion attempt
        Some(other) => {
            return Err(VaultSyncError::InvariantViolation {
                message: format!(
                    "try_promote_to_serve_host: caller session_type={other} is not promotable"
                ),
            });
        }
        None => {
            return Err(VaultSyncError::InvariantViolation {
                message: format!(
                    "try_promote_to_serve_host: session_id={session_id} not found"
                ),
            });
        }
    }

    let live_owner_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM serve_sessions
         WHERE session_type IN ('daemon', 'serve_host')
           AND heartbeat_at >= datetime('now', ?1)",
        params![format!("-{SESSION_LIVENESS_SECS} seconds")],
        |row| row.get(0),
    )?;

    if live_owner_count > 0 {
        return Ok(false);
    }

    conn.execute(
        "UPDATE serve_sessions
         SET session_type = 'serve_host'
         WHERE session_id = ?1 AND session_type = 'serve'",
        [session_id],
    )?;

    Ok(true)
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
