//! Watcher state, watch-event batching, and the per-subsystem
//! `WatcherError`.
//!
//! The types here are the in-memory shape of a per-collection
//! watcher: which mode it is in (native vs poll vs crashed), the
//! mpsc receiver fed by the notify callback, the debounced batch
//! buffer that survives between supervisor wakeups, and the
//! crash/back-off bookkeeping. The Unix-gated types live behind
//! `cfg(unix)` because the Linux `notify` crate is the only watcher
//! backend supported today.
//!
//! `WatcherHealthView` is the JSON-shaped read-out exposed via
//! `vault_sync::collection_watcher_health`. `WatcherError` is the
//! child enum surfaced through `VaultSyncError::Watcher` for
//! watcher-specific failure paths (write-dedup poisoning, recovery
//! sentinel violations, post-rename recovery, durability bugs).
//!
//! The watcher *logic* (start/run/sync/poll/reconcile) currently
//! still lives in `vault_sync::mod` and imports these types via
//! `super::watcher`. A follow-up commit moves that logic here once
//! the supervisor-handle plumbing is widened to `pub(super)`.

#[cfg(unix)]
use std::collections::HashSet;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Instant;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(unix)]
use notify::{PollWatcher, RecommendedWatcher};
#[cfg(unix)]
use tokio::sync::mpsc;

#[cfg(unix)]
pub(super) const WATCH_CHANNEL_CAPACITY: usize = 4096;
#[cfg(unix)]
pub(super) const DEFAULT_WATCH_DEBOUNCE_MS: u64 = 1500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatcherMode {
    Native,
    Poll,
    Crashed,
}

impl WatcherMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Poll => "poll",
            Self::Crashed => "crashed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WatcherHealthSnapshot {
    pub(super) mode: WatcherMode,
    pub(super) last_event_at: Option<String>,
    pub(super) channel_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatcherHealthView {
    pub mode: String,
    pub last_event_at: Option<String>,
    pub channel_depth: i64,
}

#[cfg(unix)]
pub(super) enum WatcherHandle {
    // Fields are held for Drop semantics (keeping the watcher alive), not read directly.
    #[expect(
        dead_code,
        reason = "field is owned for Drop semantics (keeps the underlying notify watcher alive); not read directly"
    )]
    Native(RecommendedWatcher),
    #[expect(
        dead_code,
        reason = "field is owned for Drop semantics (keeps the underlying poll watcher alive); not read directly"
    )]
    Poll(PollWatcher),
}

#[cfg(unix)]
pub(super) struct CollectionWatcherState {
    pub(super) root_path: PathBuf,
    pub(super) generation: i64,
    pub(super) receiver: mpsc::Receiver<WatchEvent>,
    pub(super) watcher: Option<WatcherHandle>,
    pub(super) buffer: WatchBatchBuffer,
    pub(super) mode: WatcherMode,
    pub(super) last_event_at: Option<String>,
    pub(super) last_watcher_error: Option<Instant>,
    pub(super) backoff_until: Option<Instant>,
    pub(super) consecutive_failures: u32,
}

#[cfg(unix)]
#[derive(Debug, Default)]
pub(super) struct WatchBatchBuffer {
    pub(super) dirty_paths: HashSet<PathBuf>,
    pub(super) native_renames: Vec<crate::core::reconciler::NativeRename>,
    pub(super) ignore_file_changed: bool,
    pub(super) debounce_deadline: Option<Instant>,
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum WatchEvent {
    DirtyPath(PathBuf),
    NativeRename(crate::core::reconciler::NativeRename),
    IgnoreFileChanged,
}

#[cfg(unix)]
#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("DuplicateWriteDedupError: key={key}")]
    DuplicateWriteDedup { key: String },

    #[error(
        "RecoverySentinelError: collection_id={collection_id} relative_path={relative_path} sentinel={sentinel_path} reason={reason}"
    )]
    RecoverySentinel {
        collection_id: i64,
        relative_path: String,
        sentinel_path: String,
        reason: String,
    },

    #[cfg(test)]
    #[error("DurabilityError: collection_id={collection_id} relative_path={relative_path}")]
    Durability {
        collection_id: i64,
        relative_path: String,
    },

    #[error(
        "PostRenameRecoveryPendingError: collection_id={collection_id} relative_path={relative_path} sentinel={sentinel_path} stage={stage} reason={reason}"
    )]
    PostRenameRecoveryPending {
        collection_id: i64,
        relative_path: String,
        sentinel_path: String,
        stage: &'static str,
        reason: String,
    },
}
