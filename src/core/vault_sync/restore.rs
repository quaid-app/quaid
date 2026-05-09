//! Restore-flow types, errors, and the `begin_restore` orchestrator.
//!
//! This module owns the entry path for an operator-initiated restore:
//! [`begin_restore`] composes [`validate_target`], [`stage_pending`],
//! and [`register_manifest`] (each ≤ 100 LOC). It also owns the
//! restore-only helpers `ensure_restore_not_blocked`,
//! `ensure_restore_target_is_empty`, `materialize_collection_to_path`,
//! `infer_restore_relative_path`, `staging_path_for_target`, and
//! `remove_empty_target_then_rename`.
//!
//! Other restore-touching functions still live in `vault_sync::mod`
//! because they cross many subsystems and only branch through this
//! module on a tail call:
//! - `finalize_pending_restore` (and its `_via_cli` sibling) — the
//!   apply path that runs the safety pipeline, swaps `pending_root_path`
//!   onto `root_path`, and clears the manifest.
//! - `restore_reset` — operator escape-hatch that reverts an aborted
//!   restore.
//! - `build_restore_manifest_for_directory` — directory-tree to
//!   manifest inventory; reused by `compare_manifest`.
//! - `complete_attach` and `convert_reconcile_error` — orchestration
//!   shims used by both restore and remap; widened to
//!   `pub(in crate::core::vault_sync)` so this module can call them.
//!
//! What lives here:
//! - `RestoreError` and `ConflictError` (each child enum sits next to
//!   the subsystem that produces it; the parent `VaultSyncError`
//!   imports both back via `super::restore`).
//! - `RestoreManifest` / `RestoreManifestEntry` — the directory
//!   inventory written into `pending_restore_manifest`.
//! - `FinalizeCaller` / `FinalizeOutcome` / `FinalizeCliOutcome` /
//!   `AttachReason` / `WriteBackOutcome` — the lifecycle enums
//!   surfaced by `finalize_pending_restore` and friends.
//! - `finalize_outcome_label` — `Display`-style helper used by
//!   `RestoreCommandBlocked` error formatting.
//! - `begin_restore` and its three phase helpers; the leaf restore
//!   helpers listed above.

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
