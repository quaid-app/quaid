//! `VaultSyncError` parent enum and per-subsystem child enums.
//!
//! `VaultSyncError` composes child enums (`IpcError`, `RestoreError`,
//! `ConflictError`, `WatcherError`) via `#[from]`-bearing
//! `#[error(transparent)]` variants and carries shared cross-cutting
//! variants (`Sqlite`, `Io`, `InvariantViolation`, …) directly.
//!
//! Today every child enum lives in this file. As the surrounding
//! submodules are extracted (see `vault-sync-module-layout` spec), each
//! child enum migrates next to the code that produces it: `IpcError` →
//! `ipc/`, `RestoreError` → `restore.rs`, `ConflictError` →
//! `restore.rs` (or its successor), `WatcherError` → `watcher.rs`.
//! When adding a new error variant, place it on the child enum whose
//! subsystem produced it; only put it on the parent if it is genuinely
//! cross-cutting (e.g., `Sqlite`, `Io`).

use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::core::collections::CollectionError;
use crate::core::reconciler::ReconcileError;

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

#[cfg(unix)]
#[derive(Debug, Error)]
pub enum IpcError {
    #[error("IpcDirectoryInsecureError: path={path} reason={reason}")]
    IpcDirectoryInsecure { path: String, reason: String },

    #[error("IpcSocketPermissionError: path={path} reason={reason}")]
    IpcSocketPermission { path: String, reason: String },

    #[error("IpcSocketCollisionError: path={path} reason={reason}")]
    IpcSocketCollision { path: String, reason: String },

    #[error("IpcPeerAuthFailedError: path={path} reason={reason}")]
    IpcPeerAuthFailed { path: String, reason: String },
}

#[derive(Debug, Error)]
pub enum RestoreError {
    #[error("RestoreInProgressError: collection={collection_name}")]
    RestoreInProgress { collection_name: String },

    #[error("RestorePendingFinalizeError: collection={collection_name} pending_root_path={pending_root_path}")]
    RestorePendingFinalize {
        collection_name: String,
        pending_root_path: String,
    },

    #[error("RestoreIntegrityBlockedError: collection={collection_name} blocking_column={blocking_column}")]
    RestoreIntegrityBlocked {
        collection_name: String,
        blocking_column: &'static str,
    },

    #[error("RestoreResetBlockedError: collection={collection_name} reason={reason}")]
    RestoreResetBlocked {
        collection_name: String,
        reason: &'static str,
    },

    #[error("RestoreNonEmptyTargetError: target={target}")]
    RestoreNonEmptyTarget { target: String },

    #[error(
        "ServeDiedDuringHandshakeError: collection={collection_name} expected_session_id={expected_session_id}"
    )]
    ServeDiedDuringHandshake {
        collection_name: String,
        expected_session_id: String,
    },

    #[error(
        "ServeOwnershipChangedError: collection={collection_name} expected_session_id={expected_session_id} actual_session_id={actual_session_id}"
    )]
    ServeOwnershipChanged {
        collection_name: String,
        expected_session_id: String,
        actual_session_id: String,
    },

    #[error(
        "HandshakeTimeoutError: collection={collection_name} expected_session_id={expected_session_id} reload_generation={reload_generation}"
    )]
    HandshakeTimeout {
        collection_name: String,
        expected_session_id: String,
        reload_generation: i64,
    },

    #[error(
        "NewRootVerificationFailedError: collection={collection_name} missing={missing} mismatched={mismatched} extra={extra} missing_samples={} mismatched_samples={} extra_samples={}",
        fmt_paths(missing_samples),
        fmt_paths(mismatched_samples),
        fmt_paths(extra_samples)
    )]
    NewRootVerificationFailed {
        collection_name: String,
        missing: usize,
        mismatched: usize,
        extra: usize,
        missing_samples: Vec<PathBuf>,
        mismatched_samples: Vec<PathBuf>,
        extra_samples: Vec<PathBuf>,
    },

    #[error("NewRootUnstableError: collection={collection_name}")]
    NewRootUnstable { collection_name: String },

    #[error("RestoreCommandBlockedError: collection={collection_name} outcome={outcome}")]
    RestoreCommandBlocked {
        collection_name: String,
        outcome: &'static str,
    },
}

/// Joins path samples for the human-readable `Display` form used by
/// `RestoreError::NewRootVerificationFailed`. Empty list renders as
/// `-` to preserve byte-for-byte compatibility with the previous
/// `String`-typed CSV field.
fn fmt_paths(samples: &[PathBuf]) -> String {
    if samples.is_empty() {
        "-".to_owned()
    } else {
        samples
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(unix)]
#[derive(Debug, Error)]
pub enum ConflictError {
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=MissingExpectedVersion current_version={current_version}"
    )]
    MissingExpectedVersion {
        collection_id: i64,
        relative_path: String,
        current_version: i64,
    },

    #[error(
        "Conflict: ConflictError StaleExpectedVersion collection_id={collection_id} relative_path={relative_path} expected_version={expected_version} current version: {current_version}"
    )]
    StaleExpectedVersion {
        collection_id: i64,
        relative_path: String,
        expected_version: i64,
        current_version: i64,
    },

    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=ExternalDelete"
    )]
    ExternalDelete {
        collection_id: i64,
        relative_path: String,
    },

    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=ExternalCreate"
    )]
    ExternalCreate {
        collection_id: i64,
        relative_path: String,
    },

    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=HashMismatch stored_sha256={stored_sha256} actual_sha256={actual_sha256}"
    )]
    HashMismatch {
        collection_id: i64,
        relative_path: String,
        stored_sha256: String,
        actual_sha256: String,
    },

    #[error(
        "ConcurrentRenameError: collection_id={collection_id} relative_path={relative_path} sentinel={sentinel_path}"
    )]
    ConcurrentRename {
        collection_id: i64,
        relative_path: String,
        sentinel_path: String,
    },
}
