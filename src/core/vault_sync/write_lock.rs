//! Per-slug write serialisation and the in-memory write-dedup
//! registry that detects duplicate / replayed writer-side
//! attempts.
//!
//! [`with_write_slug_lock`] guards the
//! `(root_path, relative_path)` pair so two writers in the same
//! process can't race on the same slug. The lock map lives in
//! `RuntimeRegistries::slug_writes`; the lock itself is a plain
//! `Arc<Mutex<()>>` and Drop releases it.
//!
//! [`insert_write_dedup`], [`remove_write_dedup`], and
//! [`has_write_dedup`] manage the in-process dedup set in
//! `RuntimeRegistries::dedup`. A duplicate key is fail-closed:
//! `insert_write_dedup` returns
//! `VaultSyncError::Watcher(WatcherError::DuplicateWriteDedup)`
//! rather than silently re-inserting, so the writer immediately
//! aborts instead of double-applying a write.
//!
//! The `writer_side_*` helpers are `cfg(test, unix)` — they
//! exist purely to drive the writer-side crash-core proofs in the
//! integration test suite.

#[cfg(all(test, unix))]
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::{RuntimeRegistries, VaultSyncError, PROCESS_REGISTRIES};

#[cfg(all(test, unix))]
use super::sha256_hex;
#[cfg(unix)]
use super::WatcherError;

#[cfg(unix)]
pub fn insert_write_dedup(key: &str) -> Result<(), VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    let inserted = registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .insert(key.to_owned());
    if inserted {
        Ok(())
    } else {
        Err(VaultSyncError::Watcher(WatcherError::DuplicateWriteDedup {
            key: key.to_owned(),
        }))
    }
}

#[cfg(unix)]
pub fn remove_write_dedup(key: &str) -> Result<(), VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .remove(key);
    Ok(())
}

#[cfg(all(test, unix))]
pub fn has_write_dedup(key: &str) -> Result<bool, VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    Ok(registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .contains(key))
}

pub fn with_write_slug_lock<T, F>(
    root_path: &str,
    relative_path: &str,
    action: F,
) -> Result<T, VaultSyncError>
where
    F: FnOnce() -> T,
{
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    let lock = {
        let mut slug_writes =
            registries
                .slug_writes
                .lock()
                .map_err(|_| VaultSyncError::RegistryPoisoned {
                    registry: "slug_writes",
                })?;
        slug_writes
            .entry(format!("{root_path}:{relative_path}"))
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().map_err(|_| VaultSyncError::RegistryPoisoned {
        registry: "slug_write_guard",
    })?;
    Ok(action())
}

#[cfg(all(test, unix))]
pub(super) fn writer_side_dedup_key(target_path: &Path, bytes: &[u8]) -> String {
    format!("{}::{}", target_path.display(), sha256_hex(bytes))
}

#[cfg(all(test, unix))]
pub(super) fn insert_writer_side_dedup_entry(key: &str) -> Result<(), VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .insert(key.to_owned());
    Ok(())
}

#[cfg(all(test, unix))]
pub(super) fn remove_writer_side_dedup_entry(key: &str) -> Result<(), VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .remove(key);
    Ok(())
}

#[cfg(all(test, unix))]
pub(super) fn writer_side_dedup_contains(key: &str) -> bool {
    PROCESS_REGISTRIES
        .get_or_init(RuntimeRegistries::new)
        .dedup
        .lock()
        .unwrap()
        .contains(key)
}
