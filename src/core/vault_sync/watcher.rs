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
//! [`watch_debounce_duration`] / [`watcher_backoff_duration`] are
//! the single source of truth for the supervisor's debounce window
//! and the per-watcher exponential back-off after a crash. The
//! event-classification leaf bundle ([`relative_markdown_path`],
//! [`is_root_ignore_path`], [`should_suppress_self_write_rename`],
//! [`classify_watch_event`], [`watch_callback`]) turns a raw
//! `notify::Event` into the one-or-many [`WatchEvent`]s the
//! supervisor consumes; it lives here because the only callers are
//! the watcher orchestrators.
//!
//! [`mark_watcher_crashed`] flips a `CollectionWatcherState` into
//! crashed-with-back-off; [`reconcile_halt_details`] turns a
//! reconciler error into the (`reason_code`, human message) pair
//! `convert_reconcile_error` writes into `collections.reconcile_halt_*`.
//!
//! The heavier orchestrators (`start_collection_watcher`,
//! `sync_collection_watchers`, `poll_collection_watcher`,
//! `run_overflow_recovery_pass`, `publish_watcher_health`) still
//! live in `vault_sync::mod` because they reach into the supervisor
//! handle map and per-session liveness state.

#[cfg(unix)]
use std::collections::HashSet;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(all(unix, test))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(unix)]
use notify::{
    event::ModifyKind, Event as NotifyEvent, EventKind as NotifyEventKind, PollWatcher,
    RecommendedWatcher,
};
#[cfg(unix)]
use tokio::sync::mpsc;

#[cfg(unix)]
use crate::core::conversation::file_edit::is_history_sidecar_path;
#[cfg(unix)]
use crate::core::reconciler::{is_markdown_file, ReconcileError};

#[cfg(unix)]
pub(super) const WATCH_CHANNEL_CAPACITY: usize = 4096;
#[cfg(unix)]
pub(super) const DEFAULT_WATCH_DEBOUNCE_MS: u64 = 1500;

/// Operational mode reported for a per-collection watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatcherMode {
    /// Linux `inotify` (or platform equivalent) backed watcher.
    Native,
    /// Periodic polling fallback used when the native backend is
    /// unavailable or has failed to initialise.
    Poll,
    /// Watcher has crashed and is currently inside its back-off
    /// window before the supervisor retries arming it.
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

/// JSON-shaped read-out of a watcher's current health, surfaced via
/// `vault_sync::collection_watcher_health` for CLI and IPC consumers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatcherHealthView {
    /// Stringified [`WatcherMode`] (`"native"`, `"poll"`, `"crashed"`).
    pub mode: String,
    /// ISO-8601 timestamp of the last event the watcher observed.
    pub last_event_at: Option<String>,
    /// Current depth of the mpsc channel buffering pending events.
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

/// Errors surfaced by the watcher subsystem and wrapped by
/// [`super::VaultSyncError::Watcher`] at the API boundary.
#[cfg(unix)]
#[derive(Debug, Error)]
pub enum WatcherError {
    /// Raised when an in-process write tries to claim a write-dedup
    /// key that is already held — the writer aborts rather than risk
    /// double-applying the same edit.
    #[error("DuplicateWriteDedupError: key={key}")]
    DuplicateWriteDedup {
        /// The dedup key the writer attempted to insert.
        key: String,
    },

    /// Raised when the post-rename recovery sentinel for a path is
    /// missing or malformed, indicating an integrity-breaking
    /// state the watcher refuses to recover from automatically.
    #[error(
        "RecoverySentinelError: collection_id={collection_id} relative_path={relative_path} sentinel={sentinel_path} reason={reason}"
    )]
    RecoverySentinel {
        /// Collection whose sentinel was checked.
        collection_id: i64,
        /// Vault-relative path the sentinel describes.
        relative_path: String,
        /// On-disk path of the sentinel file.
        sentinel_path: String,
        /// Human-readable failure reason.
        reason: String,
    },

    /// Test-only variant used by the durability fault-injection
    /// fixtures to simulate fsync failure on a write.
    #[cfg(test)]
    #[error("DurabilityError: collection_id={collection_id} relative_path={relative_path}")]
    Durability {
        /// Collection that experienced the simulated durability fault.
        collection_id: i64,
        /// Vault-relative path that the durability fault affected.
        relative_path: String,
    },

    /// Raised when a watcher event arrives for a path that is still
    /// inside a post-rename recovery window and the supervisor must
    /// wait for the recovery stage to complete before reconciling.
    #[error(
        "PostRenameRecoveryPendingError: collection_id={collection_id} relative_path={relative_path} sentinel={sentinel_path} stage={stage} reason={reason}"
    )]
    PostRenameRecoveryPending {
        /// Collection whose recovery is still pending.
        collection_id: i64,
        /// Vault-relative path waiting on recovery.
        relative_path: String,
        /// On-disk path of the sentinel that gates recovery.
        sentinel_path: String,
        /// Static label naming the recovery stage that has not yet
        /// completed.
        stage: &'static str,
        /// Human-readable description of why recovery is still pending.
        reason: String,
    },
}

/// Default supervisor debounce window between the first dirty-event
/// in a batch and the reconcile that drains it. Operators can
/// override via the `QUAID_WATCH_DEBOUNCE_MS` env var (positive
/// values only; zero/negative falls back to the default).
#[cfg(unix)]
pub(super) fn watch_debounce_duration() -> Duration {
    std::env::var("QUAID_WATCH_DEBOUNCE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_WATCH_DEBOUNCE_MS))
}

/// Exponential back-off (capped at 60s) the supervisor honours
/// before re-arming a watcher after a crash. `consecutive_failures`
/// is the counter on `CollectionWatcherState`; it grows by one on
/// every `mark_watcher_crashed` call and resets on the first
/// successful event.
#[cfg(unix)]
pub(super) fn watcher_backoff_duration(consecutive_failures: u32) -> Duration {
    let shift = consecutive_failures.saturating_sub(1).min(6);
    Duration::from_secs((1_u64 << shift).min(60))
}

/// Returns `Some(relative_path)` if `path` is a markdown file
/// inside `root_path` (and not a conversation history sidecar);
/// `None` otherwise. The watcher uses this to ignore non-markdown
/// noise (lockfiles, hidden temp files) without paying for a full
/// reconcile pass.
#[cfg(unix)]
pub(super) fn relative_markdown_path(root_path: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(root_path).ok()?;
    (is_markdown_file(relative) && !is_history_sidecar_path(relative))
        .then(|| relative.to_path_buf())
}

#[cfg(unix)]
pub(super) fn is_root_ignore_path(root_path: &Path, path: &Path) -> bool {
    path.strip_prefix(root_path)
        .ok()
        .is_some_and(|relative| relative == Path::new(".quaidignore"))
}

/// True if a notify rename event's pair of `(from, to)` paths
/// looks like a writer-side self-write (CLI vault write or
/// IPC-proxied put) and the supervisor should suppress it. The
/// rule: suppress when the destination is in the dedup window;
/// also suppress when the source is in the dedup window or is a
/// non-markdown path (a tempfile being renamed into place).
#[cfg(unix)]
pub(super) fn should_suppress_self_write_rename(
    root_path: &Path,
    event_paths: &[PathBuf],
) -> Result<bool, super::VaultSyncError> {
    let Some(target_path) = event_paths.get(1) else {
        return Ok(false);
    };
    if !super::maybe_suppress_self_write_event(target_path)? {
        return Ok(false);
    }
    let Some(source_path) = event_paths.first() else {
        return Ok(true);
    };
    if relative_markdown_path(root_path, source_path).is_none() {
        return Ok(true);
    }
    super::maybe_suppress_self_write_event(source_path)
}

/// Turns a raw `notify::Event` into the per-collection
/// [`WatchEvent`]s the supervisor consumes. Renames produce a
/// `NativeRename` plus two `DirtyPath`s (so reconcile sees both
/// the old and new identity); single-path events produce one
/// `DirtyPath`; paths that match `.quaidignore` produce an
/// `IgnoreFileChanged`. Self-writes are suppressed via
/// `super::maybe_suppress_self_write_event`.
#[cfg(unix)]
pub(super) fn classify_watch_event(
    root_path: &Path,
    event: NotifyEvent,
) -> Result<Vec<WatchEvent>, super::VaultSyncError> {
    let mut actions = Vec::new();
    let ignore_file_changed = event
        .paths
        .iter()
        .any(|path| is_root_ignore_path(root_path, path));
    if ignore_file_changed {
        actions.push(WatchEvent::IgnoreFileChanged);
    }
    if matches!(event.kind, NotifyEventKind::Modify(ModifyKind::Name(_))) && event.paths.len() >= 2
    {
        let from_path = relative_markdown_path(root_path, &event.paths[0]);
        let to_path = relative_markdown_path(root_path, &event.paths[1]);
        if should_suppress_self_write_rename(root_path, &event.paths)? {
            return Ok(actions);
        }
        if let (Some(from_path), Some(to_path)) = (from_path, to_path) {
            actions.push(WatchEvent::NativeRename(
                crate::core::reconciler::NativeRename {
                    from_path: from_path.clone(),
                    to_path: to_path.clone(),
                },
            ));
            actions.push(WatchEvent::DirtyPath(from_path));
            actions.push(WatchEvent::DirtyPath(to_path));
            return Ok(actions);
        }
    }

    for full_path in event.paths {
        if is_root_ignore_path(root_path, &full_path) {
            continue;
        }
        let Some(relative_path) = relative_markdown_path(root_path, &full_path) else {
            continue;
        };
        if super::maybe_suppress_self_write_event(&full_path)? {
            continue;
        }
        actions.push(WatchEvent::DirtyPath(relative_path));
    }
    Ok(actions)
}

/// Builds the closure passed to `notify::Watcher::new`. Drops
/// notify errors silently, classifies the event into one or more
/// [`WatchEvent`]s, and pushes each onto the per-collection mpsc
/// channel. On `try_send`-full it marks the collection
/// `needs_full_sync = 1` via a fresh connection so the next
/// supervisor pass picks up the lost events; on closed it returns
/// (the collection has been detached).
#[cfg(unix)]
pub(super) fn watch_callback(
    collection_id: i64,
    callback_root: PathBuf,
    db_path: String,
    sender: mpsc::Sender<WatchEvent>,
) -> impl FnMut(notify::Result<NotifyEvent>) + Send + 'static {
    move |result: notify::Result<NotifyEvent>| {
        let Ok(event) = result else {
            return;
        };
        let Ok(actions) = classify_watch_event(&callback_root, event) else {
            return;
        };
        for action in actions {
            match sender.try_send(action) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    if let Ok(conn) = crate::core::db::open_runtime(&db_path) {
                        let _ = super::mark_collection_needs_full_sync(&conn, collection_id);
                    }
                    eprintln!(
                        "WARN: watch_channel_full collection_id={} root={}",
                        collection_id,
                        callback_root.display()
                    );
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    }
}

/// Flips a watcher state into `Crashed`, schedules the next
/// re-arm via [`watcher_backoff_duration`], and returns the
/// chosen back-off (so the supervisor can log it).
#[cfg(unix)]
pub(super) fn mark_watcher_crashed(
    collection_id: i64,
    state: &mut CollectionWatcherState,
) -> Duration {
    let now = Instant::now();
    state.mode = WatcherMode::Crashed;
    state.watcher = None;
    state.buffer = WatchBatchBuffer::default();
    state.last_watcher_error = Some(now);
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    let backoff = watcher_backoff_duration(state.consecutive_failures);
    state.backoff_until = Some(now + backoff);
    eprintln!(
        "WARN: watcher_crashed collection_id={} backoff_secs={}",
        collection_id,
        backoff.as_secs()
    );
    backoff
}

/// Maps a `ReconcileError` to the `(reason_code, rendered_reason)`
/// pair `convert_reconcile_error` writes into the
/// `reconcile_halted_at` / `reconcile_halt_reason` columns. Returns
/// `None` for transient errors that should not halt reconcile.
#[cfg(unix)]
pub(super) fn reconcile_halt_details(err: &ReconcileError) -> Option<(&'static str, String)> {
    match err {
        ReconcileError::DuplicateUuidError { uuid, paths } => Some((
            "duplicate_uuid",
            format!(
                "DuplicateUuidError: uuid={} paths={}",
                uuid,
                paths.join(",")
            ),
        )),
        ReconcileError::UnresolvableTrivialContentError {
            missing_path,
            candidate_paths,
            reason,
        } => Some((
            "unresolvable_trivial_content",
            format!(
                "UnresolvableTrivialContentError: missing={} candidates={} reason={}",
                missing_path,
                candidate_paths.join(","),
                reason
            ),
        )),
        _ => None,
    }
}

#[cfg(all(unix, test))]
pub(super) static FORCE_NATIVE_WATCHER_INIT_FAILURE: AtomicBool = AtomicBool::new(false);

#[cfg(all(unix, test))]
pub(super) fn set_force_native_watcher_init_failure(enabled: bool) {
    FORCE_NATIVE_WATCHER_INIT_FAILURE.store(enabled, Ordering::SeqCst);
}
