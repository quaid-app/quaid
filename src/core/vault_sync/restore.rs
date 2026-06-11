//! Restore-flow types, errors, and the `begin_restore` orchestrator
//! for the vault-sync engine.
//!
//! Owns the entry path for an operator-initiated restore:
//! [`begin_restore`] composes the validate/stage/register phases and
//! the leaf restore helpers (`ensure_restore_not_blocked`,
//! `ensure_restore_target_is_empty`, `materialize_collection_to_path`,
//! `infer_restore_relative_path`, `staging_path_for_target`,
//! `remove_empty_target_then_rename`). Also home to [`RestoreError`]
//! and [`ConflictError`] (re-exported by the parent
//! `VaultSyncError`), the [`RestoreManifest`] / [`RestoreManifestEntry`]
//! inventory written into `pending_restore_manifest`, and the
//! lifecycle enums surfaced by `finalize_pending_restore`
//! ([`FinalizeCaller`], [`FinalizeOutcome`], [`FinalizeCliOutcome`],
//! [`AttachReason`], [`WriteBackOutcome`]). The actual
//! `finalize_pending_restore`, `restore_reset`,
//! `build_restore_manifest_for_directory`, `complete_attach`, and
//! `convert_reconcile_error` orchestration shims still live in
//! `vault_sync::mod` because they cross many subsystems and are only
//! called through here on a tail call.
//!
//! See also: `super::recovery` for the recovery-directory bootstrap
//! that restore depends on, `super::watcher` for the handshake that
//! issues the online restore lease, and `super::error` for the
//! parent `VaultSyncError` that wraps the errors defined here.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::core::collections::{self, Collection, CollectionState};
#[cfg(unix)]
use crate::core::reconciler::{
    run_restore_remap_safety_pipeline_without_mount_check, FullHashReconcileAuthorization,
    RestoreRemapOperation, RestoreRemapSafetyRequest,
};

#[cfg(unix)]
use super::convert_reconcile_error;
use super::recovery::{bootstrap_recovery_directories, recovery_root_for_db_path};
use super::{
    build_restore_manifest_for_directory, complete_attach, current_host, database_path,
    finalize_pending_restore, mark_collection_restoring_for_handshake,
    start_short_lived_owner_lease, wait_for_exact_ack, ShortLivedLease, VaultSyncError,
};

/// One file in a [`RestoreManifest`]: the path relative to the
/// restore root and the content hash and byte size used to detect
/// staging-tree drift before the swap-onto-root step finalizes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreManifestEntry {
    /// Path of the file relative to the restore target root, using
    /// forward-slash separators for stable cross-platform comparison.
    pub relative_path: String,
    /// Lowercase hex SHA-256 of the file bytes at manifest-build time.
    pub sha256: String,
    /// File size in bytes at manifest-build time, used as a cheap
    /// pre-check before the SHA-256 comparison.
    pub size_bytes: u64,
}

/// Directory inventory captured for a pending restore: every file
/// materialized into the staging tree, persisted into
/// `collections.pending_restore_manifest`, and re-verified during
/// `finalize_pending_restore` to detect tampering between stage and
/// swap.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreManifest {
    /// All files captured in the manifest, in directory-walk order
    /// produced by `build_restore_manifest_for_directory`.
    pub entries: Vec<RestoreManifestEntry>,
}

/// Result of the optional UUID write-back step that stamps a fresh
/// frontmatter `uuid` into a newly attached vault file. Reported back
/// to operator-facing callers so they can tell why a file was — or
/// was not — modified during attach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBackOutcome {
    /// A new UUID was written into the file's frontmatter and the
    /// file on disk now reflects the database row.
    Migrated,
    /// The file lives on a read-only mount, so the write-back was
    /// skipped without raising an error.
    SkippedReadOnly,
    /// The file already had a frontmatter UUID matching the database
    /// row; no write was needed.
    AlreadyHadUuid,
}

/// Identifies which code path invoked `finalize_pending_restore`,
/// so the finalizer can apply caller-specific authorization (the
/// originator owns the command id; recovery owns a session id; an
/// external finalize must prove a fresh handshake session).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizeCaller {
    /// The same process that called `begin_restore` is finalizing
    /// the restore it just staged; identified by the
    /// `restore_command_id` it wrote into `collections`.
    RestoreOriginator {
        /// The `restore_command_id` UUID emitted by `begin_restore`
        /// for this restore.
        command_id: String,
    },
    /// A process restart found pending restore state on a
    /// collection that this host owns and is finalizing as part of
    /// startup recovery.
    StartupRecovery {
        /// Session id of the owner lease driving the recovery
        /// finalize.
        session_id: String,
    },
    /// A non-originator process is being asked to finalize (for
    /// example, the operator ran `quaid vault-sync finalize` on a
    /// different host than the one that ran `begin_restore`).
    ExternalFinalize {
        /// Session id the caller currently holds, used to prove
        /// the caller can act on the collection's lease.
        session_id: String,
    },
}

/// Terminal classification reported by `finalize_pending_restore` to
/// internal callers (CLI, recovery, originator). Surfaces *why* the
/// finalize step ended where it did so the caller can decide whether
/// to retry, escalate, or move on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizeOutcome {
    /// The pending tree passed every safety check and the swap onto
    /// `root_path` completed; the collection is back to `Attached`.
    Finalized,
    /// Finalize ran but is waiting on an external precondition
    /// (typically a watcher handshake or owner-lease acquisition);
    /// safe to retry later.
    Deferred,
    /// The manifest captured at `begin_restore` no longer matches
    /// the staged tree (files added, removed, or hash-changed in
    /// between); marks `pending_manifest_incomplete_at` and blocks.
    ManifestIncomplete,
    /// The safety pipeline reported an integrity failure on the
    /// pending tree; marks `integrity_failed_at` and blocks until
    /// an operator resets the restore.
    IntegrityFailed,
    /// A previously orphaned pending tree from a crashed restore
    /// was recovered and cleaned up; the collection itself had no
    /// further work to do.
    OrphanRecovered,
    /// The finalize was aborted by an explicit operator action
    /// (typically `restore_reset`) before it could complete.
    Aborted,
    /// The collection had no pending restore state, so finalize
    /// was a no-op.
    NoPendingWork,
}

/// CLI-friendly projection of [`FinalizeOutcome`] returned by the
/// `_via_cli` wrapper. Collapses the `Finalized` + post-attach
/// sequence the CLI cares about into a single `Attached` variant so
/// the command can render a single line of feedback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizeCliOutcome {
    /// The pending tree was finalized and the collection was
    /// re-attached on this host; restore is complete.
    Attached,
    /// An orphaned pending tree was discovered and reclaimed
    /// without any further restore work pending.
    OrphanRecovered,
    /// Finalize is waiting on a precondition and the operator can
    /// safely re-run the command.
    Deferred,
    /// The manifest captured at `begin_restore` no longer matches
    /// the staged tree; operator intervention required.
    ManifestIncomplete,
    /// The safety pipeline blocked the restore on integrity
    /// grounds; operator intervention required.
    IntegrityFailed,
    /// The restore was aborted by an operator before finalize
    /// could complete.
    Aborted,
    /// There was no pending restore state for this collection.
    NoPendingWork,
}

/// Why `complete_attach` is being called — restore and remap each
/// produce a post-orchestration attach but with different
/// preconditions, and this discriminator lets the attach routine
/// pick the right state-transition path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachReason {
    /// Attach is following a successful `finalize_pending_restore`;
    /// the staged tree has just been swapped onto `root_path`.
    RestorePostFinalize,
    /// Attach is following a successful remap reconcile; the
    /// collection's `root_path` has just moved to a new mount.
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

/// Failure modes specific to the restore pipeline — both the
/// `begin_restore` entry path and the `finalize_pending_restore`
/// apply path. Wrapped by `VaultSyncError::Restore` so the parent
/// error type stays the single surface vault-sync callers handle.
#[derive(Debug, Error)]
pub enum RestoreError {
    /// Fires when `begin_restore` is called against a collection
    /// whose `state` is already `Restoring` and that has no
    /// `pending_root_path` recorded (a restore is mid-flight).
    #[error("RestoreInProgressError: collection={collection_name}")]
    RestoreInProgress {
        /// Name of the collection that is already mid-restore.
        collection_name: String,
    },

    /// Fires when a new restore is attempted on a collection whose
    /// prior restore staged a pending tree but never finalized;
    /// the operator must finalize or reset that one first.
    #[error("RestorePendingFinalizeError: collection={collection_name} pending_root_path={pending_root_path}")]
    RestorePendingFinalize {
        /// Name of the collection holding the un-finalized pending
        /// tree.
        collection_name: String,
        /// Filesystem path of the still-pending staging tree the
        /// operator must finalize or reset.
        pending_root_path: String,
    },

    /// Fires when a restore is attempted on a collection that has
    /// already been marked as integrity-failed by an earlier
    /// safety-pipeline run; further restores are blocked until the
    /// flag is cleared by an operator action.
    #[error("RestoreIntegrityBlockedError: collection={collection_name} blocking_column={blocking_column}")]
    RestoreIntegrityBlocked {
        /// Name of the collection blocked by an integrity flag.
        collection_name: String,
        /// SQLite column whose non-NULL timestamp records the
        /// integrity failure (e.g. `integrity_failed_at` or
        /// `pending_manifest_incomplete_at`).
        blocking_column: &'static str,
    },

    /// Fires when `restore_reset` refuses to revert an aborted
    /// restore because the collection is in a state the reset
    /// helper can't safely roll back from.
    #[error("RestoreResetBlockedError: collection={collection_name} reason={reason}")]
    RestoreResetBlocked {
        /// Name of the collection whose reset was refused.
        collection_name: String,
        /// Short machine-readable reason describing the precondition
        /// that blocked the reset.
        reason: &'static str,
    },

    /// Fires when the operator-supplied restore target exists and
    /// is either a file, a symlink, or a non-empty directory; the
    /// restore staging step refuses to risk clobbering operator
    /// data.
    #[error("RestoreNonEmptyTargetError: target={target}")]
    RestoreNonEmptyTarget {
        /// Filesystem path of the non-empty restore target.
        target: String,
    },

    /// Fires when the watcher process owning the handshake exits
    /// before the expected handshake ack arrives, leaving the
    /// restore lease unowned.
    #[error(
        "ServeDiedDuringHandshakeError: collection={collection_name} expected_session_id={expected_session_id}"
    )]
    ServeDiedDuringHandshake {
        /// Name of the collection whose handshake was orphaned.
        collection_name: String,
        /// Session id `begin_restore` was waiting on the watcher
        /// to acknowledge.
        expected_session_id: String,
    },

    /// Fires when the handshake ack arrives but the acknowledging
    /// session id differs from the one `begin_restore` published
    /// — typically another process raced in and acquired the
    /// lease first.
    #[error(
        "ServeOwnershipChangedError: collection={collection_name} expected_session_id={expected_session_id} actual_session_id={actual_session_id}"
    )]
    ServeOwnershipChanged {
        /// Name of the collection whose lease was stolen.
        collection_name: String,
        /// Session id `begin_restore` published and expected the
        /// watcher to ack.
        expected_session_id: String,
        /// Session id actually acknowledged by the watcher.
        actual_session_id: String,
    },

    /// Fires when `wait_for_exact_ack` times out waiting for the
    /// watcher to acknowledge the restore session — the watcher
    /// is alive but hasn't reloaded onto the new handshake.
    #[error(
        "HandshakeTimeoutError: collection={collection_name} expected_session_id={expected_session_id} reload_generation={reload_generation}"
    )]
    HandshakeTimeout {
        /// Name of the collection whose handshake timed out.
        collection_name: String,
        /// Session id the watcher was expected to acknowledge.
        expected_session_id: String,
        /// Reload generation counter at the time of the timeout,
        /// used to distinguish a stuck watcher from one that simply
        /// hasn't seen the new state yet.
        reload_generation: i64,
    },

    /// Fires when `finalize_pending_restore`'s post-swap
    /// verification finds the on-disk tree no longer matches the
    /// manifest captured at `begin_restore` — files are missing,
    /// have mismatched hashes, or are extras not in the manifest.
    #[error(
        "NewRootVerificationFailedError: collection={collection_name} missing={missing} mismatched={mismatched} extra={extra} missing_samples={} mismatched_samples={} extra_samples={}",
        fmt_paths(missing_samples),
        fmt_paths(mismatched_samples),
        fmt_paths(extra_samples)
    )]
    NewRootVerificationFailed {
        /// Name of the collection whose verification failed.
        collection_name: String,
        /// Count of manifest entries with no corresponding file on
        /// the swapped-in root.
        missing: usize,
        /// Count of files whose on-disk SHA-256 differs from the
        /// manifest.
        mismatched: usize,
        /// Count of files on the swapped-in root that were not in
        /// the manifest at all.
        extra: usize,
        /// Bounded sample of `missing` entries for operator
        /// triage, formatted into the `Display` form by the private
        /// `fmt_paths` helper.
        missing_samples: Vec<PathBuf>,
        /// Bounded sample of `mismatched` entries for operator
        /// triage.
        mismatched_samples: Vec<PathBuf>,
        /// Bounded sample of `extra` entries for operator triage.
        extra_samples: Vec<PathBuf>,
    },

    /// Fires when the post-swap directory hash isn't stable across
    /// successive scans — the filesystem is still being mutated
    /// by something outside vault-sync and finalize refuses to
    /// proceed against a moving target.
    #[error("NewRootUnstableError: collection={collection_name}")]
    NewRootUnstable {
        /// Name of the collection whose post-swap root failed to
        /// stabilize.
        collection_name: String,
    },

    /// Fires when an internal restore-command path receives a
    /// non-`Finalized` outcome from `finalize_pending_restore`
    /// and propagates the label so callers can render an
    /// operator-readable explanation.
    #[error("RestoreCommandBlockedError: collection={collection_name} outcome={outcome}")]
    RestoreCommandBlocked {
        /// Name of the collection whose restore command was
        /// blocked.
        collection_name: String,
        /// Static label produced by the private
        /// `finalize_outcome_label` helper describing the terminal
        /// finalize outcome.
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

/// Optimistic-concurrency and reconcile-time conflicts surfaced by
/// the reconcile-driven write paths used during restore and remap.
/// Variants describe the specific way the in-memory expectation of a
/// file's state disagrees with what the SQLite row or on-disk file
/// reports at apply time.
#[cfg(unix)]
#[derive(Debug, Error)]
pub enum ConflictError {
    /// Fires when a write was attempted without an
    /// `expected_version` against a row that does have one — the
    /// caller did not refresh before writing and would clobber
    /// concurrent edits.
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=MissingExpectedVersion current_version={current_version}"
    )]
    MissingExpectedVersion {
        /// Collection id the conflicting row belongs to.
        collection_id: i64,
        /// Path of the file (relative to the collection root) that
        /// raised the conflict.
        relative_path: String,
        /// Current version number on the row at conflict-detection
        /// time.
        current_version: i64,
    },

    /// Fires when an `expected_version` was supplied but does not
    /// match the row's current version — another writer
    /// incremented the version between the caller's read and the
    /// commit attempt.
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=StaleExpectedVersion expected_version={expected_version} current version: {current_version}"
    )]
    StaleExpectedVersion {
        /// Collection id the conflicting row belongs to.
        collection_id: i64,
        /// Path of the file (relative to the collection root) that
        /// raised the conflict.
        relative_path: String,
        /// Version the caller expected to find on the row.
        expected_version: i64,
        /// Version actually present on the row at commit time.
        current_version: i64,
    },

    /// Fires when reconcile finds a row but the corresponding file
    /// on disk has been deleted out-of-band — an external delete
    /// must be resolved before the write can proceed.
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=ExternalDelete"
    )]
    ExternalDelete {
        /// Collection id the missing file belonged to.
        collection_id: i64,
        /// Path of the file (relative to the collection root) that
        /// disappeared from disk.
        relative_path: String,
    },

    /// Fires when reconcile finds a file on disk that has no
    /// matching row — an external create must be classified
    /// (import vs. ignore) before the write can proceed.
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=ExternalCreate"
    )]
    ExternalCreate {
        /// Collection id the unexpected file would belong to.
        collection_id: i64,
        /// Path of the file (relative to the collection root) that
        /// appeared without a row.
        relative_path: String,
    },

    /// Fires when reconcile finds a row whose stored content hash
    /// disagrees with the file currently on disk — the file was
    /// mutated out-of-band relative to the database's record.
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=HashMismatch stored_sha256={stored_sha256} actual_sha256={actual_sha256}"
    )]
    HashMismatch {
        /// Collection id the mismatched file belongs to.
        collection_id: i64,
        /// Path of the file (relative to the collection root) that
        /// raised the mismatch.
        relative_path: String,
        /// SHA-256 stored in the database for the file's last
        /// reconciled state.
        stored_sha256: String,
        /// SHA-256 computed from the file's current bytes on disk.
        actual_sha256: String,
    },

    /// Fires when reconcile detects an in-progress rename
    /// (sentinel file present) racing with the current write — the
    /// caller must retry once the rename has settled.
    #[error(
        "ConcurrentRenameError: collection_id={collection_id} relative_path={relative_path} sentinel={sentinel_path}"
    )]
    ConcurrentRename {
        /// Collection id the renaming file belongs to.
        collection_id: i64,
        /// Path of the file (relative to the collection root) that
        /// is mid-rename.
        relative_path: String,
        /// Filesystem path of the rename-in-progress sentinel
        /// captured at conflict-detection time.
        sentinel_path: String,
    },
}

/// Entry point for an operator-initiated restore: validates the
/// target, stages a pending tree, registers the manifest, and on
/// the offline path runs the full attach. Returns the
/// `restore_command_id` the caller must hand to
/// `finalize_pending_restore` (online) or that has already been
/// finalized for them (offline).
pub fn begin_restore(
    conn: &Connection,
    collection_name: &str,
    target_path: &Path,
    online: bool,
) -> Result<String, VaultSyncError> {
    let prep = validate_target(conn, collection_name, target_path, online)?;
    let command_id = stage_pending(conn, &prep, target_path)?;
    register_manifest(conn, &prep, &command_id)?;
    Ok(command_id)
}

struct RestorePrep {
    collection: Collection,
    db_path: PathBuf,
    recovery_root: PathBuf,
    online: bool,
    /// The session id that owns the restore lease — the expected
    /// session id from the handshake on the online path, or the
    /// short-lived lease session id on the offline path.
    session_id: String,
    /// Held for Drop semantics on the offline path so the
    /// short-lived lease keeps heartbeating until `register_manifest`
    /// has run. `None` on the online path because the watcher owns
    /// the lease via the handshake.
    #[expect(
        dead_code,
        reason = "field is owned for Drop semantics (keeps the short-lived owner lease's heartbeat thread alive across stage_pending and register_manifest); not read directly"
    )]
    lease: Option<ShortLivedLease>,
}

fn validate_target(
    conn: &Connection,
    collection_name: &str,
    target_path: &Path,
    online: bool,
) -> Result<RestorePrep, VaultSyncError> {
    let collection = collections::get_by_name(conn, collection_name)?.ok_or_else(|| {
        VaultSyncError::CollectionNotFound {
            name: collection_name.to_owned(),
        }
    })?;
    ensure_restore_not_blocked(&collection)?;
    ensure_restore_target_is_empty(target_path)?;
    let db_path = PathBuf::from(database_path(conn)?);
    let recovery_root = recovery_root_for_db_path(&db_path);
    bootstrap_recovery_directories(conn, &recovery_root)?;

    let (session_id, lease) = if online {
        let (_, expected_session_id, generation) =
            mark_collection_restoring_for_handshake(conn, collection.id)?;
        wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?;
        conn.execute(
            "UPDATE collections
             SET restore_lease_session_id = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![collection.id, expected_session_id],
        )?;
        (expected_session_id, None)
    } else {
        let lease = start_short_lived_owner_lease(conn, collection.id)?;
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                  restore_lease_session_id = ?2,
                  updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE id = ?1",
            params![collection.id, lease.session_id.clone()],
        )?;
        (lease.session_id.clone(), Some(lease))
    };

    Ok(RestorePrep {
        collection,
        db_path,
        recovery_root,
        online,
        session_id,
        lease,
    })
}

fn stage_pending(
    conn: &Connection,
    prep: &RestorePrep,
    target_path: &Path,
) -> Result<String, VaultSyncError> {
    #[cfg(unix)]
    {
        let request = RestoreRemapSafetyRequest {
            collection_id: prep.collection.id,
            db_path: &prep.db_path,
            recovery_root: &prep.recovery_root,
            operation: RestoreRemapOperation::Restore,
            authorization: FullHashReconcileAuthorization::RestoreLease {
                lease_session_id: prep.session_id.clone(),
            },
            allow_finalize_pending: false,
            stability_max_iters: 0,
        };
        if let Err(err) = run_restore_remap_safety_pipeline_without_mount_check(conn, &request) {
            return Err(convert_reconcile_error(
                conn,
                prep.collection.id,
                &prep.collection.name,
                err,
            )?);
        }
    }
    let command_id = Uuid::now_v7().to_string();
    let staging_path = staging_path_for_target(target_path);
    if staging_path.exists() {
        let _ = fs::remove_dir_all(&staging_path);
    }
    materialize_collection_to_path(conn, &prep.collection, &staging_path)?;
    let manifest = build_restore_manifest_for_directory(&staging_path)?;
    let manifest_json = serde_json::to_string(&manifest)?;
    if prep.online {
        conn.execute(
            "UPDATE collections
             SET pending_root_path = ?2,
                 pending_restore_manifest = ?3,
                 restore_command_id = ?4,
                 restore_command_pid = ?5,
                 restore_command_host = ?6,
                 pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                 restore_lease_session_id = ?7,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![
                prep.collection.id,
                target_path.display().to_string(),
                manifest_json,
                command_id,
                std::process::id() as i64,
                current_host(),
                prep.session_id,
            ],
        )?;
    } else {
        conn.execute(
            "UPDATE collections
             SET pending_root_path = ?2,
                  pending_restore_manifest = ?3,
                  restore_command_id = ?4,
                  restore_command_pid = ?5,
                  restore_command_host = ?6,
                  pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                  updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE id = ?1",
            params![
                prep.collection.id,
                target_path.display().to_string(),
                manifest_json,
                command_id,
                std::process::id() as i64,
                current_host(),
            ],
        )?;
    }
    remove_empty_target_then_rename(&staging_path, target_path)?;
    Ok(command_id)
}

fn register_manifest(
    conn: &Connection,
    prep: &RestorePrep,
    command_id: &str,
) -> Result<(), VaultSyncError> {
    let outcome = finalize_pending_restore(
        conn,
        prep.collection.id,
        FinalizeCaller::RestoreOriginator {
            command_id: command_id.to_owned(),
        },
    )?;
    if prep.online {
        let _ = outcome;
        return Ok(());
    }
    match outcome {
        FinalizeOutcome::Finalized => {}
        other => {
            return Err(VaultSyncError::Restore(
                RestoreError::RestoreCommandBlocked {
                    collection_name: prep.collection.name.clone(),
                    outcome: finalize_outcome_label(&other),
                },
            ));
        }
    }
    let attached = complete_attach(
        conn,
        prep.collection.id,
        &prep.session_id,
        AttachReason::RestorePostFinalize,
    )?;
    if !attached {
        return Err(VaultSyncError::InvariantViolation {
            message: format!(
                "collection={} restore offline path did not complete attach",
                prep.collection.name
            ),
        });
    }
    Ok(())
}

pub(super) fn ensure_restore_not_blocked(collection: &Collection) -> Result<(), VaultSyncError> {
    if collection.state == CollectionState::Restoring {
        if let Some(pending_root_path) = collection.pending_root_path.clone() {
            if collection.integrity_failed_at.is_some() {
                return Err(VaultSyncError::Restore(
                    RestoreError::RestoreIntegrityBlocked {
                        collection_name: collection.name.clone(),
                        blocking_column: "integrity_failed_at",
                    },
                ));
            }
            if collection.pending_manifest_incomplete_at.is_some() {
                return Err(VaultSyncError::Restore(
                    RestoreError::RestoreIntegrityBlocked {
                        collection_name: collection.name.clone(),
                        blocking_column: "pending_manifest_incomplete_at",
                    },
                ));
            }
            return Err(VaultSyncError::Restore(
                RestoreError::RestorePendingFinalize {
                    collection_name: collection.name.clone(),
                    pending_root_path,
                },
            ));
        }
        return Err(VaultSyncError::Restore(RestoreError::RestoreInProgress {
            collection_name: collection.name.clone(),
        }));
    }
    if collection.integrity_failed_at.is_some() {
        return Err(VaultSyncError::Restore(
            RestoreError::RestoreIntegrityBlocked {
                collection_name: collection.name.clone(),
                blocking_column: "integrity_failed_at",
            },
        ));
    }
    if collection.pending_manifest_incomplete_at.is_some() {
        return Err(VaultSyncError::Restore(
            RestoreError::RestoreIntegrityBlocked {
                collection_name: collection.name.clone(),
                blocking_column: "pending_manifest_incomplete_at",
            },
        ));
    }
    Ok(())
}

pub(super) fn ensure_restore_target_is_empty(target_path: &Path) -> Result<(), VaultSyncError> {
    if !target_path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(target_path)?;
    if metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(VaultSyncError::Restore(
            RestoreError::RestoreNonEmptyTarget {
                target: target_path.display().to_string(),
            },
        ));
    }
    if metadata.is_dir() && fs::read_dir(target_path)?.next().is_none() {
        return Ok(());
    }
    Err(VaultSyncError::Restore(
        RestoreError::RestoreNonEmptyTarget {
            target: target_path.display().to_string(),
        },
    ))
}

fn remove_empty_target_then_rename(
    staging_path: &Path,
    target_path: &Path,
) -> Result<(), VaultSyncError> {
    if target_path.exists() {
        fs::remove_dir(target_path)?;
    }
    fs::rename(staging_path, target_path)?;
    Ok(())
}

fn staging_path_for_target(target_path: &Path) -> PathBuf {
    let name = format!(
        "{}.quaid-restoring-{}",
        target_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("target"),
        Uuid::now_v7()
    );
    target_path.with_file_name(name)
}

pub(super) fn materialize_collection_to_path(
    conn: &Connection,
    collection: &Collection,
    path: &Path,
) -> Result<(), VaultSyncError> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;

    let mut stmt = conn.prepare(
        "SELECT p.id, p.slug, ri.raw_bytes, ri.file_path, fs.relative_path
         FROM pages p
         LEFT JOIN raw_imports ri
           ON ri.page_id = p.id AND ri.is_active = 1
         LEFT JOIN file_state fs
           ON fs.page_id = p.id AND fs.collection_id = p.collection_id
         WHERE p.collection_id = ?1 AND p.quarantined_at IS NULL
         ORDER BY p.slug",
    )?;
    let rows = stmt.query_map([collection.id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<Vec<u8>>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    for row in rows {
        let (page_id, slug, raw_bytes, raw_path, relative_path) = row?;
        let raw_bytes = raw_bytes.ok_or_else(|| VaultSyncError::InvariantViolation {
            message: format!(
                "collection={} slug={} page_id={} missing active raw_imports",
                collection.name, slug, page_id
            ),
        })?;
        let relative_path = infer_restore_relative_path(
            collection,
            &slug,
            raw_path.as_deref(),
            relative_path.as_deref(),
        );
        let output_path = path.join(&relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output_path, raw_bytes)?;
    }
    Ok(())
}

pub(super) fn infer_restore_relative_path(
    collection: &Collection,
    slug: &str,
    raw_path: Option<&str>,
    relative_path: Option<&str>,
) -> PathBuf {
    if let Some(relative_path) = relative_path {
        return PathBuf::from(relative_path);
    }
    if let Some(raw_path) = raw_path {
        let raw_path = PathBuf::from(raw_path);
        if raw_path.is_relative() {
            return raw_path;
        }
        let root_path = PathBuf::from(&collection.root_path);
        if !collection.root_path.is_empty() {
            if let Ok(relative) = raw_path.strip_prefix(root_path) {
                return relative.to_path_buf();
            }
        }
    }
    PathBuf::from(format!("{slug}.md"))
}
