//! Recovery in-progress guards and the on-disk recovery directory.
//!
//! Two related concerns live here:
//!
//! 1. The in-process [`RecoveryInProgressGuard`] tracks which
//!    collections currently hold the post-rename recovery lock. The
//!    set lives in `RuntimeRegistries::recovering_collections`;
//!    the guard takes its membership and Drop releases it. Querying
//!    the set is a fast `pub` read via
//!    [`collection_recovery_in_progress`].
//!
//! 2. Path-style helpers locate the per-collection recovery directory
//!    under `<db_dir>/recovery/<collection_id>/`,
//!    [`bootstrap_recovery_directories`] creates the directory tree
//!    on startup, and [`recovery_sentinel_paths`] enumerates the
//!    `*.needs_full_sync` sentinel files that record post-crash work
//!    the supervisor must reconcile.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::{RuntimeRegistries, VaultSyncError, PROCESS_REGISTRIES};

pub(super) fn with_recovering_collections<T>(
    f: impl FnOnce(&mut HashSet<i64>) -> T,
) -> Result<T, VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    let mut recovering =
        registries
            .recovering_collections
            .lock()
            .map_err(|_| VaultSyncError::RegistryPoisoned {
                registry: "recovering_collections",
            })?;
    Ok(f(&mut recovering))
}

pub(super) struct RecoveryInProgressGuard {
    collection_id: i64,
}

impl RecoveryInProgressGuard {
    pub(super) fn enter(collection_id: i64) -> Result<Self, VaultSyncError> {
        with_recovering_collections(|recovering| {
            recovering.insert(collection_id);
        })?;
        Ok(Self { collection_id })
    }
}

impl Drop for RecoveryInProgressGuard {
    fn drop(&mut self) {
        if let Some(registries) = PROCESS_REGISTRIES.get() {
            if let Ok(mut recovering) = registries.recovering_collections.lock() {
                recovering.remove(&self.collection_id);
            }
        }
    }
}

pub fn collection_recovery_in_progress(collection_id: i64) -> bool {
    PROCESS_REGISTRIES
        .get()
        .and_then(|registries| {
            registries
                .recovering_collections
                .lock()
                .ok()
                .map(|recovering| recovering.contains(&collection_id))
        })
        .unwrap_or(false)
}

#[cfg(test)]
pub(crate) fn set_collection_recovery_in_progress_for_test(collection_id: i64, in_progress: bool) {
    let _ = with_recovering_collections(|recovering| {
        if in_progress {
            recovering.insert(collection_id);
        } else {
            recovering.remove(&collection_id);
        }
    });
}

pub fn recovery_root_for_db_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("recovery")
}

pub fn collection_recovery_dir(recovery_root: &Path, collection_id: i64) -> PathBuf {
    recovery_root.join(collection_id.to_string())
}

pub(super) fn bootstrap_recovery_directories(
    conn: &Connection,
    recovery_root: &Path,
) -> Result<(), VaultSyncError> {
    fs::create_dir_all(recovery_root)?;
    let mut stmt = conn.prepare("SELECT id FROM collections")?;
    let collection_ids = stmt
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for collection_id in collection_ids {
        fs::create_dir_all(collection_recovery_dir(recovery_root, collection_id))?;
    }
    Ok(())
}

pub(super) fn recovery_sentinel_paths(
    recovery_root: &Path,
    collection_id: i64,
) -> Result<Vec<PathBuf>, VaultSyncError> {
    let recovery_dir = collection_recovery_dir(recovery_root, collection_id);
    if !recovery_dir.exists() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for entry in fs::read_dir(recovery_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .ends_with(".needs_full_sync")
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}
