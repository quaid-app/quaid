//! `VaultSyncError` parent enum.
//!
//! `VaultSyncError` composes child enums (`IpcError`, `RestoreError`,
//! `ConflictError`, `WatcherError`) via `#[from]`-bearing
//! `#[error(transparent)]` variants and carries shared cross-cutting
//! variants (`Sqlite`, `Io`, `InvariantViolation`, …) directly.
//!
//! Each child enum lives next to the code that produces it:
//! `IpcError` → `ipc/` (in this file until ipc/ is extracted),
//! `RestoreError` → `restore.rs`,
//! `ConflictError` → `restore.rs`,
//! `WatcherError` → `watcher.rs`.
//! When adding a new error variant, place it on the child enum whose
//! subsystem produced it; only put it on the parent if it is genuinely
//! cross-cutting (e.g., `Sqlite`, `Io`).

use std::io;

use thiserror::Error;

use crate::core::collections::CollectionError;
use crate::core::reconciler::ReconcileError;

#[cfg(unix)]
use super::ipc::IpcError;
#[cfg(unix)]
use super::restore::ConflictError;
use super::restore::RestoreError;
#[cfg(unix)]
use super::watcher::WatcherError;

/// Parent error type returned from `vault_sync` entry points.
///
/// Aggregates the failure modes of every subsystem in the module —
/// IPC, restore, watcher, conflict detection — alongside cross-cutting
/// SQLite / I/O / collection failures so that callers can match a
/// single enum at the API boundary.
#[derive(Debug, Error)]
pub enum VaultSyncError {
    /// Raised when a caller names a collection that does not exist in
    /// the `collections` table.
    #[error("collection not found: {name}")]
    CollectionNotFound {
        /// The collection name the caller asked for.
        name: String,
    },

    /// Raised when a bare slug resolves to multiple pages across
    /// collections and the caller has not disambiguated.
    #[error("ambiguous slug: {slug} ({candidates})")]
    AmbiguousSlug {
        /// The slug that matched more than one page.
        slug: String,
        /// Human-readable summary of the matching candidates.
        candidates: String,
    },

    /// Raised when the requested page does not exist in the collection.
    #[error("page not found: {slug}")]
    PageNotFound {
        /// Slug of the page that could not be found.
        slug: String,
    },

    /// Raised when a write or sync is attempted against a collection
    /// that is mid-restore and therefore not safe to mutate.
    #[error(
        "CollectionRestoringError: collection={collection_name} state={state} needs_full_sync={needs_full_sync}"
    )]
    CollectionRestoring {
        /// Name of the collection that is currently restoring.
        collection_name: String,
        /// Stringified `CollectionState` at the time of the rejection.
        state: String,
        /// Whether the collection still needs a full reconcile sweep
        /// after restore completes.
        needs_full_sync: bool,
    },

    /// Raised when a write is attempted against a collection that the
    /// operator has marked read-only.
    #[error("CollectionReadOnlyError: collection={collection_name}")]
    CollectionReadOnly {
        /// Name of the read-only collection.
        collection_name: String,
    },

    /// Raised when a CLI-side operation tries to mutate a collection
    /// while a live runtime-host session (a `daemon`, a promoted
    /// `serve_host`, or — during partial-rollback windows — an older
    /// binary's `serve`) still holds the owner lease for it.
    ///
    /// Renamed from `ServeOwnsCollectionError` in the
    /// daemon-and-http-transport change: the error fires for any
    /// runtime owner regardless of which session_type holds the lease,
    /// so the operator-facing message should reflect the actual role
    /// (`daemon` → suggest `quaid daemon stop`; `serve_host` / `serve`
    /// → suggest `kill <pid>`).
    #[error(
        "RuntimeOwnsCollectionError: collection={collection_name} owner_session_id={owner_session_id} owner_pid={owner_pid} owner_host={owner_host} owner_session_type={owner_session_type}"
    )]
    RuntimeOwnsCollectionError {
        /// Name of the collection held by a live runtime-host session.
        collection_name: String,
        /// Session id of the owning runtime-host process.
        owner_session_id: String,
        /// OS pid of the owning runtime-host process.
        owner_pid: i64,
        /// Hostname of the owning runtime-host process.
        owner_host: String,
        /// `serve_sessions.session_type` of the owning session — one of
        /// `'daemon'`, `'serve_host'`, or `'serve'`. Determines whether
        /// the operator-facing suggestion is `quaid daemon stop`
        /// (`daemon` role) or `kill <pid>` (any other role).
        owner_session_type: String,
    },

    /// Surfaces an IPC subsystem failure (handshake, framing, peer
    /// credentials, accept loop).
    #[cfg(unix)]
    #[error(transparent)]
    Ipc(#[from] IpcError),

    /// Surfaces a restore-flow failure (manifest, materialize, attach,
    /// finalize, write-back).
    #[error(transparent)]
    Restore(#[from] RestoreError),

    /// Surfaces an optimistic-concurrency conflict on a vault write
    /// (expected-version mismatch or missing expected version).
    #[cfg(unix)]
    #[error(transparent)]
    Conflict(#[from] ConflictError),

    /// Surfaces an optimistic-concurrency conflict from the page write
    /// path itself (the compare-and-swap UPDATE matched zero rows).
    /// Cross-platform, unlike the unix-only vault [`ConflictError`].
    #[error(transparent)]
    Occ(#[from] crate::core::types::OccError),

    /// Surfaces a watcher-thread failure (initialisation, crash,
    /// channel exhaustion).
    #[cfg(unix)]
    #[error(transparent)]
    Watcher(#[from] WatcherError),

    /// Raised when an internal invariant check fails — indicates a bug
    /// or schema/state corruption rather than a user-recoverable error.
    #[error("InvariantViolationError: {message}")]
    InvariantViolation {
        /// Human-readable description of the violated invariant.
        message: String,
    },

    /// Raised when the reconciler has halted further automatic sync
    /// because of a duplicate UUID, unresolvable trivial content, or
    /// similar integrity guardrail.
    #[error("ReconcileHaltedError: collection={collection_name} reason={reason}")]
    ReconcileHalted {
        /// Name of the collection whose reconciler is halted.
        collection_name: String,
        /// Halt reason as recorded in `collections.reconcile_halt_reason`.
        reason: String,
    },

    /// Raised when a plain (non-restore) sync is attempted on a
    /// collection that is not in an active state with a usable root.
    #[error("PlainSyncActiveRootRequiredError: collection={collection_name} state={state}")]
    PlainSyncActiveRootRequired {
        /// Name of the collection lacking an active root.
        collection_name: String,
        /// Stringified `CollectionState` at the time of the rejection.
        state: String,
    },

    /// Raised when a shared `Mutex`/`RwLock` inside the runtime
    /// registries was poisoned by a panic on another thread.
    #[error("RegistryPoisonedError: registry={registry}")]
    RegistryPoisoned {
        /// Static name of the poisoned registry (e.g., `"dedup"`).
        registry: &'static str,
    },

    /// Raised on non-unix builds when an operator invokes a command
    /// that requires unix-only primitives (e.g., the IPC socket).
    #[cfg(not(unix))]
    #[error("UnsupportedPlatformError: command={command} requires=unix")]
    UnsupportedPlatform {
        /// The CLI command name that is unavailable on this platform.
        command: &'static str,
    },

    /// Surfaces a `rusqlite` error from an internal SQL operation.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// Surfaces a [`crate::core::types::DbError`] from the connection
    /// factory (`db::open_runtime`) or default-collection provisioning.
    #[error(transparent)]
    Db(#[from] crate::core::types::DbError),

    /// Surfaces a `std::io::Error` from a filesystem operation.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// Surfaces a [`CollectionError`] bubbled up from
    /// [`crate::core::collections`].
    #[error(transparent)]
    Collections(#[from] CollectionError),

    /// Surfaces a [`ReconcileError`] bubbled up from
    /// [`crate::core::reconciler`].
    #[error(transparent)]
    Reconcile(#[from] ReconcileError),

    /// Surfaces a `serde_json` (de)serialization error on an IPC or
    /// persisted payload.
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}
