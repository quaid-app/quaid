//! Per-collection ownership: who currently holds the live runtime
//! lease and how leases are acquired/released.
//!
//! Three concerns live here:
//!
//! 1. [`LiveCollectionOwner`] is the row shape returned when a
//!    collection has a live runtime-host session attached. The
//!    `session_type` field distinguishes whether the owner is the
//!    long-lived `daemon`, a no-daemon-fallback `serve_host`, or
//!    (during partial-rollback windows) an older binary's `serve`.
//! 2. The query helpers [`live_collection_owner`] and
//!    [`ensure_no_live_serve_owner`] (plus their `_for_root_path`
//!    siblings) check whether a runtime-host is alive within the
//!    `SESSION_LIVENESS_SECS` heartbeat window before letting an
//!    operation continue. They surface
//!    [`VaultSyncError::RuntimeOwnsCollectionError`] (renamed from
//!    the prior `ServeOwnsCollectionError` so the error name matches
//!    the role-agnostic ownership model) when the answer is
//!    "yes, somebody else owns this".
//! 3. [`acquire_owner_lease`] and [`release_owner_lease`] write the
//!    owner row on `collection_owners` and the redundant
//!    `active_lease_session_id` column on `collections`. These are
//!    the only places that mutate ownership state during normal
//!    runtime-host startup/shutdown; restore/recovery paths use the
//!    short-lived lease helpers in `start_serve_runtime`.
//!
//! `SESSION_LIVENESS_SECS` (the 15-second heartbeat threshold) and
//! [`load_collection_by_id`] are imported from the parent module
//! because session-lifetime accounting and collection lookup remain
//! load-bearing for the watcher/restore/IPC paths and have not yet
//! been split into their own submodules.

use rusqlite::{params, Connection, OptionalExtension};

use super::{load_collection_by_id, VaultSyncError, SESSION_LIVENESS_SECS};

/// Snapshot of the runtime-host session currently holding the owner
/// lease on a collection, returned by [`live_collection_owner`] when
/// one exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveCollectionOwner {
    /// Session id of the live runtime-host process owning the lease.
    pub session_id: String,
    /// OS pid of the live runtime-host process.
    pub pid: i64,
    /// Hostname reported by the live runtime-host process at registration.
    pub host: String,
    /// `serve_sessions.session_type` of the owning session — one of
    /// `'daemon'`, `'serve_host'`, or `'serve'`. Lets callers surface the
    /// actual role in operator-facing error messages so the suggestion
    /// (`quaid daemon stop` vs `kill <pid>`) is accurate.
    pub session_type: String,
}

/// Returns the live owner of a collection, or `None` if no
/// runtime-host session has heartbeated within the liveness window.
///
/// The `session_type` filter accepts `'daemon'`, `'serve_host'`, and
/// `'serve'`. The third (`'serve'`) is included only to keep older
/// `quaid serve` rows (written by a binary that predates the
/// daemon-and-http-transport change) visible as owners during
/// partial-rollback windows; new code never assigns `collection_owners`
/// to a plain `'serve'`-typed session.
pub fn live_collection_owner(
    conn: &Connection,
    collection_id: i64,
) -> Result<Option<LiveCollectionOwner>, VaultSyncError> {
    conn.query_row(
        "SELECT o.session_id, s.pid, s.host, s.session_type
         FROM collection_owners o
         JOIN serve_sessions s ON s.session_id = o.session_id
         WHERE o.collection_id = ?1
           AND s.heartbeat_at >= datetime('now', ?2)
           AND s.session_type IN ('daemon', 'serve_host', 'serve')",
        params![collection_id, format!("-{SESSION_LIVENESS_SECS} seconds")],
        |row| {
            Ok(LiveCollectionOwner {
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

fn live_collection_owner_for_root_path(
    conn: &Connection,
    root_path: &str,
) -> Result<Option<(String, LiveCollectionOwner)>, VaultSyncError> {
    conn.query_row(
        "SELECT c.name, o.session_id, s.pid, s.host, s.session_type
         FROM collections c
         JOIN collection_owners o ON o.collection_id = c.id
         JOIN serve_sessions s ON s.session_id = o.session_id
         WHERE c.root_path = ?1
           AND s.heartbeat_at >= datetime('now', ?2)
           AND s.session_type IN ('daemon', 'serve_host', 'serve')
         ORDER BY c.id
         LIMIT 1",
        params![root_path, format!("-{SESSION_LIVENESS_SECS} seconds")],
        |row| {
            Ok((
                row.get(0)?,
                LiveCollectionOwner {
                    session_id: row.get(1)?,
                    pid: row.get(2)?,
                    host: row.get(3)?,
                    session_type: row.get(4)?,
                },
            ))
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Errors with `RuntimeOwnsCollectionError` if a live runtime-host
/// session currently holds the lease on the collection; used by CLI
/// paths that must refuse to mutate a collection while a runtime
/// owns it.
pub fn ensure_no_live_serve_owner(
    conn: &Connection,
    collection_id: i64,
) -> Result<(), VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    if let Some(owner) = live_collection_owner(conn, collection_id)? {
        return Err(VaultSyncError::RuntimeOwnsCollectionError {
            collection_name: collection.name,
            owner_session_id: owner.session_id,
            owner_pid: owner.pid,
            owner_host: owner.host,
            owner_session_type: owner.session_type,
        });
    }
    Ok(())
}

/// Errors with `RuntimeOwnsCollectionError` if any collection rooted at
/// `root_path` is held by a live runtime-host session — the root-path
/// variant used by attach and import paths that don't yet have a
/// collection id.
pub fn ensure_no_live_serve_owner_for_root_path(
    conn: &Connection,
    root_path: &str,
) -> Result<(), VaultSyncError> {
    if let Some((collection_name, owner)) = live_collection_owner_for_root_path(conn, root_path)? {
        return Err(VaultSyncError::RuntimeOwnsCollectionError {
            collection_name,
            owner_session_id: owner.session_id,
            owner_pid: owner.pid,
            owner_host: owner.host,
            owner_session_type: owner.session_type,
        });
    }
    Ok(())
}

/// Returns the recorded owner `session_id` for a collection without
/// checking heartbeat liveness; callers that need a live owner should
/// use [`live_collection_owner`] instead.
pub fn owner_session_id(
    conn: &Connection,
    collection_id: i64,
) -> Result<Option<String>, VaultSyncError> {
    conn.query_row(
        "SELECT session_id FROM collection_owners WHERE collection_id = ?1",
        [collection_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Records `session_id` as the current owner of the collection and
/// mirrors the assignment onto `collections.active_lease_session_id`;
/// errors if a different live runtime-host session already holds the
/// lease.
pub fn acquire_owner_lease(
    conn: &Connection,
    collection_id: i64,
    session_id: &str,
) -> Result<(), VaultSyncError> {
    if let Some(owner) = live_collection_owner(conn, collection_id)? {
        if owner.session_id != session_id {
            let collection = load_collection_by_id(conn, collection_id)?;
            return Err(VaultSyncError::RuntimeOwnsCollectionError {
                collection_name: collection.name,
                owner_session_id: owner.session_id,
                owner_pid: owner.pid,
                owner_host: owner.host,
                owner_session_type: owner.session_type,
            });
        }
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO collection_owners (collection_id, session_id)
         VALUES (?1, ?2)
         ON CONFLICT(collection_id) DO UPDATE SET
             session_id = excluded.session_id,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        params![collection_id, session_id],
    )?;
    tx.execute(
        "UPDATE collections
         SET active_lease_session_id = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![collection_id, session_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Clears the owner lease and `active_lease_session_id` when the
/// caller's `session_id` matches the recorded owner; a no-op
/// otherwise so stale releases don't disturb a newer owner.
pub fn release_owner_lease(
    conn: &Connection,
    collection_id: i64,
    session_id: &str,
) -> Result<(), VaultSyncError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2",
        params![collection_id, session_id],
    )?;
    tx.execute(
        "UPDATE collections
         SET active_lease_session_id = CASE
                 WHEN active_lease_session_id = ?2 THEN NULL
                 ELSE active_lease_session_id
             END,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![collection_id, session_id],
    )?;
    tx.commit()?;
    Ok(())
}
