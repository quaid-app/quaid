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

#[derive(Debug, Error)]
pub enum VaultSyncError {
    #[error("collection not found: {name}")]
    CollectionNotFound { name: String },

    #[error("ambiguous slug: {slug} ({candidates})")]
    AmbiguousSlug { slug: String, candidates: String },

    #[error("page not found: {slug}")]
    PageNotFound { slug: String },

    #[error(
        "CollectionRestoringError: collection={collection_name} state={state} needs_full_sync={needs_full_sync}"
    )]
    CollectionRestoring {
        collection_name: String,
        state: String,
        needs_full_sync: bool,
    },

    #[error("CollectionReadOnlyError: collection={collection_name}")]
    CollectionReadOnly { collection_name: String },

    #[error(
        "ServeOwnsCollectionError: collection={collection_name} owner_session_id={owner_session_id} owner_pid={owner_pid} owner_host={owner_host}"
    )]
    ServeOwnsCollectionError {
        collection_name: String,
        owner_session_id: String,
        owner_pid: i64,
        owner_host: String,
    },

    #[cfg(unix)]
    #[error(transparent)]
    Ipc(#[from] IpcError),

    #[error(transparent)]
    Restore(#[from] RestoreError),

    #[cfg(unix)]
    #[error(transparent)]
    Conflict(#[from] ConflictError),

    #[cfg(unix)]
    #[error(transparent)]
    Watcher(#[from] WatcherError),

    #[error("InvariantViolationError: {message}")]
    InvariantViolation { message: String },

    #[error("ReconcileHaltedError: collection={collection_name} reason={reason}")]
    ReconcileHalted {
        collection_name: String,
        reason: String,
    },

    #[error("PlainSyncActiveRootRequiredError: collection={collection_name} state={state}")]
    PlainSyncActiveRootRequired {
        collection_name: String,
        state: String,
    },

    #[error("RegistryPoisonedError: registry={registry}")]
    RegistryPoisoned { registry: &'static str },

    #[cfg(not(unix))]
    #[error("UnsupportedPlatformError: command={command} requires=unix")]
    UnsupportedPlatform { command: &'static str },

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Collections(#[from] CollectionError),

    #[error(transparent)]
    Reconcile(#[from] ReconcileError),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}
