//! Filesystem precondition checks for vault writes.
//!
//! Before a write commits, the caller asks
//! [`check_fs_precondition`] (or one of its `pub(crate)` siblings) to
//! decide whether the on-disk state matches what the database
//! believes is there. Three outcomes are possible:
//!
//! - [`FsPreconditionOutcome::FastPath`] — stat agrees with the
//!   stored row, no rehash needed.
//! - [`FsPreconditionOutcome::SlowPathSelfHeal`] — stat drifted but
//!   content hash still matches; the caller can refresh the stored
//!   `file_state` row inline.
//! - [`FsPreconditionOutcome::FreshCreate`] — neither row nor file
//!   exists, the path is being newly written.
//!
//! Any external mutation (delete, create, hash drift, symlink) is
//! surfaced as a `ConflictError` rather than a precondition outcome
//! so the writer fails closed.

use std::io;
use std::path::Path;

use rusqlite::Connection;
use rustix::fd::AsFd;

use crate::core::file_state;
use crate::core::fs_safety;

use super::{ConflictError, VaultSyncError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsPreconditionOutcome {
    FastPath,
    SlowPathSelfHeal,
    FreshCreate,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone)]
struct FsPreconditionInspection {
    outcome: FsPreconditionOutcome,
    current_stat: Option<file_state::FileStat>,
    stored_row: Option<file_state::FileStateRow>,
}

fn inspect_fs_precondition(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
) -> Result<FsPreconditionInspection, VaultSyncError> {
    let relative_path_str = relative_path.to_string_lossy().into_owned();
    let stored_row = file_state::get_file_state(conn, collection_id, &relative_path_str)?;
    let root_fd = fs_safety::open_root_fd(root_path)?;
    let parent_fd = match fs_safety::walk_to_parent(&root_fd, relative_path) {
        Ok(parent_fd) => Some(parent_fd),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };

    inspect_fs_precondition_with_parent_fd(
        conn,
        collection_id,
        root_path,
        relative_path,
        parent_fd.as_ref(),
        stored_row,
    )
}

fn inspect_fs_precondition_with_parent_fd<Fd: AsFd>(
    _conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
    parent_fd: Option<&Fd>,
    stored_row: Option<file_state::FileStateRow>,
) -> Result<FsPreconditionInspection, VaultSyncError> {
    let relative_path_str = relative_path.to_string_lossy().into_owned();
    let target_name =
        relative_path
            .file_name()
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: format!(
                    "relative path has no terminal component: {}",
                    relative_path.display()
                ),
            })?;

    let current_stat = match parent_fd {
        Some(parent_fd) => match fs_safety::stat_at_nofollow(parent_fd, Path::new(target_name)) {
            Ok(stat) => {
                if stat.is_symlink() {
                    return Err(io::Error::other("target path is a symlink").into());
                }
                Some(file_state::FileStat {
                    mtime_ns: stat.mtime_ns,
                    ctime_ns: Some(stat.ctime_ns),
                    size_bytes: stat.size_bytes,
                    inode: Some(stat.inode),
                })
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        },
        None => None,
    };

    match (stored_row, current_stat) {
        (None, None) => Ok(FsPreconditionInspection {
            outcome: FsPreconditionOutcome::FreshCreate,
            current_stat: None,
            stored_row: None,
        }),
        (Some(_), None) => Err(VaultSyncError::Conflict(ConflictError::ExternalDelete {
            collection_id,
            relative_path: relative_path_str,
        })),
        (None, Some(_)) => Err(VaultSyncError::Conflict(ConflictError::ExternalCreate {
            collection_id,
            relative_path: relative_path_str,
        })),
        (Some(stored_row), Some(current_stat)) => {
            if !file_state::needs_rehash(&current_stat, &stored_row) {
                return Ok(FsPreconditionInspection {
                    outcome: FsPreconditionOutcome::FastPath,
                    current_stat: Some(current_stat),
                    stored_row: Some(stored_row),
                });
            }

            let actual_sha256 = file_state::hash_file(&root_path.join(relative_path))?;
            if actual_sha256 == stored_row.sha256 {
                Ok(FsPreconditionInspection {
                    outcome: FsPreconditionOutcome::SlowPathSelfHeal,
                    current_stat: Some(current_stat),
                    stored_row: Some(stored_row),
                })
            } else {
                Err(VaultSyncError::Conflict(ConflictError::HashMismatch {
                    collection_id,
                    relative_path: relative_path_str,
                    stored_sha256: stored_row.sha256,
                    actual_sha256,
                }))
            }
        }
    }
}

#[expect(
    dead_code,
    reason = "addressed in decompose-vault-sync-module — pre-sentinel fs-precondition check helper kept for proposal #4's split"
)]
pub(crate) fn check_fs_precondition_before_sentinel(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
) -> Result<FsPreconditionOutcome, VaultSyncError> {
    Ok(inspect_fs_precondition(conn, collection_id, root_path, relative_path)?.outcome)
}

pub(crate) fn check_fs_precondition_with_parent_fd<Fd: AsFd>(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
    parent_fd: &Fd,
) -> Result<FsPreconditionOutcome, VaultSyncError> {
    let stored_row =
        file_state::get_file_state(conn, collection_id, &relative_path.to_string_lossy())?;
    Ok(inspect_fs_precondition_with_parent_fd(
        conn,
        collection_id,
        root_path,
        relative_path,
        Some(parent_fd),
        stored_row,
    )?
    .outcome)
}

#[cfg(test)]
pub fn check_fs_precondition(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
) -> Result<FsPreconditionOutcome, VaultSyncError> {
    let inspection = inspect_fs_precondition(conn, collection_id, root_path, relative_path)?;
    if inspection.outcome == FsPreconditionOutcome::SlowPathSelfHeal {
        if let (Some(current_stat), Some(stored_row)) = (
            inspection.current_stat.as_ref(),
            inspection.stored_row.as_ref(),
        ) {
            file_state::upsert_file_state(
                conn,
                collection_id,
                &stored_row.relative_path,
                stored_row.page_id,
                current_stat,
                &stored_row.sha256,
            )?;
        }
    }
    Ok(inspection.outcome)
}
