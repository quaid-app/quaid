//! Restore-flow types and per-subsystem error enums.
//!
//! This file currently holds the data types and error variants the
//! restore subsystem hands back to callers. The behaviour functions
//! that drive a restore — `begin_restore`, `finalize_pending_restore`,
//! `finalize_pending_restore_via_cli`, `restore_reset`,
//! `build_restore_manifest_for_directory` — still live in
//! `vault_sync::mod` because they reach into many private helpers
//! (mark_collection_restoring_for_handshake, wait_for_exact_ack,
//! materialize_collection_to_path, complete_attach,
//! ensure_restore_*, staging_path_for_target,
//! convert_reconcile_error). Migrating that surface requires either
//! widening every helper to `pub(super)` or moving them all together
//! and is deferred to a follow-up commit (see openspec change
//! `decompose-vault-sync-module` task 4.7's deviation note).
//!
//! What does live here today:
//! - `RestoreError` and `ConflictError` (moved out of
//!   `error.rs` so each child enum sits next to the subsystem that
//!   produces it; the parent `VaultSyncError` now imports both back
//!   via `super::restore`).
//! - `RestoreManifest` / `RestoreManifestEntry` — the directory
//!   inventory written into `pending_restore_manifest` on the
//!   `collections` row.
//! - `FinalizeCaller` / `FinalizeOutcome` / `FinalizeCliOutcome` /
//!   `AttachReason` / `WriteBackOutcome` — the lifecycle enums
//!   surfaced by `finalize_pending_restore` and friends.
//! - `finalize_outcome_label` — `Display`-style helper used by
//!   `RestoreCommandBlocked` error formatting.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreManifestEntry {
    pub relative_path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreManifest {
    pub entries: Vec<RestoreManifestEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBackOutcome {
    Migrated,
    SkippedReadOnly,
    AlreadyHadUuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizeCaller {
    RestoreOriginator { command_id: String },
    StartupRecovery { session_id: String },
    ExternalFinalize { session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizeOutcome {
    Finalized,
    Deferred,
    ManifestIncomplete,
    IntegrityFailed,
    OrphanRecovered,
    Aborted,
    NoPendingWork,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizeCliOutcome {
    Attached,
    OrphanRecovered,
    Deferred,
    ManifestIncomplete,
    IntegrityFailed,
    Aborted,
    NoPendingWork,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachReason {
    RestorePostFinalize,
    RemapPostReconcile,
}

pub(super) fn finalize_outcome_label(outcome: &FinalizeOutcome) -> &'static str {
    match outcome {
        FinalizeOutcome::Finalized => "Finalized",
        FinalizeOutcome::Deferred => "Deferred",
        FinalizeOutcome::ManifestIncomplete => "ManifestIncomplete",
        FinalizeOutcome::IntegrityFailed => "IntegrityFailed",
        FinalizeOutcome::OrphanRecovered => "OrphanRecovered",
        FinalizeOutcome::Aborted => "Aborted",
        FinalizeOutcome::NoPendingWork => "NoPendingWork",
    }
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
