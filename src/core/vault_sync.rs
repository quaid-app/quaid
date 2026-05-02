use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
#[cfg(unix)]
use std::io::{BufRead, BufReader, BufWriter, Write};
#[cfg(unix)]
use std::mem::size_of;
#[cfg(target_os = "linux")]
use std::mem::zeroed;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::atomic::AtomicUsize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

#[cfg(unix)]
use notify::{
    event::ModifyKind, Config as NotifyConfig, Event as NotifyEvent, EventKind as NotifyEventKind,
    PollWatcher, RecommendedWatcher, RecursiveMode, Watcher,
};
#[cfg(unix)]
use rustix::fd::AsFd;
#[cfg(all(test, unix))]
use rustix::fs::fsync;
#[cfg(unix)]
use tokio::sync::mpsc::{self, error::TryRecvError};

use crate::commands::{get::get_page_by_key, put};
use crate::core::collections::{
    self, Collection, CollectionError, CollectionState, OpKind, SlugResolution,
};
#[cfg(all(test, unix))]
use crate::core::db;
#[cfg(unix)]
use crate::core::file_state;
#[cfg(unix)]
use crate::core::fs_safety;
use crate::core::ignore_patterns;
use crate::core::markdown;
use crate::core::page_uuid;
use crate::core::quarantine;
use crate::core::raw_imports;
use crate::core::reconciler::{
    fresh_attach_reconcile_and_activate, full_hash_reconcile_authorized, is_markdown_file,
    reconcile, resolve_page_identity, run_restore_remap_safety_pipeline_without_mount_check,
    scheduled_full_hash_audit_authorized, CanonicalIdentityRecord, FullHashReconcileAuthorization,
    FullHashReconcileMode, PageIdentityResolution, ReconcileError, ReconcileStats,
    RestoreRemapOperation, RestoreRemapSafetyRequest,
};

const SESSION_LIVENESS_SECS: i64 = 15;
const HANDSHAKE_POLL_MS: u64 = 100;
const HANDSHAKE_TIMEOUT_SECS: u64 = 30;
const HEARTBEAT_INTERVAL_SECS: u64 = 5;
const DEFERRED_RETRY_SECS: u64 = 1;
const DEFAULT_MANIFEST_INCOMPLETE_ESCALATION_SECS: i64 = 1800;
const REMAP_VERIFICATION_SAMPLE_LIMIT: usize = 5;
const QUARANTINE_SWEEP_INTERVAL_SECS: u64 = 24 * 60 * 60;
const FULL_HASH_AUDIT_SWEEP_INTERVAL_SECS: u64 = 24 * 60 * 60;
const RAW_IMPORT_TTL_SWEEP_INTERVAL_SECS: u64 = 24 * 60 * 60;
const DEFAULT_FULL_HASH_AUDIT_DAYS: i64 = 7;
#[cfg(unix)]
const WATCH_CHANNEL_CAPACITY: usize = 4096;
#[cfg(unix)]
const DEFAULT_WATCH_DEBOUNCE_MS: u64 = 1500;
#[cfg(unix)]
const SELF_WRITE_DEDUP_TTL_SECS: u64 = 5;
#[cfg(unix)]
const SELF_WRITE_DEDUP_SWEEP_SECS: u64 = 10;

struct RuntimeRegistries {
    supervisor_handles: Mutex<HashMap<i64, SupervisorHandle>>,
    dedup: Mutex<HashSet<String>>,
    #[cfg(unix)]
    self_write_dedup: Mutex<HashMap<PathBuf, SelfWriteDedupEntry>>,
    slug_writes: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    recovering_collections: Mutex<HashSet<i64>>,
}

impl RuntimeRegistries {
    fn new() -> Self {
        Self {
            supervisor_handles: Mutex::new(HashMap::new()),
            dedup: Mutex::new(HashSet::new()),
            #[cfg(unix)]
            self_write_dedup: Mutex::new(HashMap::new()),
            slug_writes: Mutex::new(HashMap::new()),
            recovering_collections: Mutex::new(HashSet::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SupervisorHandle {
    session_id: String,
    generation: i64,
    watcher_health: Option<WatcherHealthSnapshot>,
}

#[cfg(unix)]
#[derive(Debug, Clone)]
struct SelfWriteDedupEntry {
    sha256: String,
    inserted_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum WatcherMode {
    Native,
    Poll,
    Crashed,
}

impl WatcherMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Poll => "poll",
            Self::Crashed => "crashed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WatcherHealthSnapshot {
    mode: WatcherMode,
    last_event_at: Option<String>,
    channel_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatcherHealthView {
    pub mode: String,
    pub last_event_at: Option<String>,
    pub channel_depth: i64,
}

#[cfg(unix)]
enum WatcherHandle {
    // Fields are held for Drop semantics (keeping the watcher alive), not read directly.
    #[allow(dead_code)]
    Native(RecommendedWatcher),
    #[allow(dead_code)]
    Poll(PollWatcher),
}

#[cfg(unix)]
struct CollectionWatcherState {
    root_path: PathBuf,
    generation: i64,
    receiver: mpsc::Receiver<WatchEvent>,
    watcher: Option<WatcherHandle>,
    buffer: WatchBatchBuffer,
    mode: WatcherMode,
    last_event_at: Option<String>,
    last_watcher_error: Option<Instant>,
    backoff_until: Option<Instant>,
    consecutive_failures: u32,
}

#[cfg(unix)]
#[derive(Debug, Default)]
struct WatchBatchBuffer {
    dirty_paths: HashSet<PathBuf>,
    native_renames: Vec<crate::core::reconciler::NativeRename>,
    ignore_file_changed: bool,
    debounce_deadline: Option<Instant>,
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum WatchEvent {
    DirtyPath(PathBuf),
    NativeRename(crate::core::reconciler::NativeRename),
    IgnoreFileChanged,
}

static PROCESS_REGISTRIES: OnceLock<RuntimeRegistries> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IgnoreParseErrorView {
    pub code: String,
    pub line: Option<i64>,
    pub raw: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryCollectionView {
    pub name: String,
    pub root_path: Option<String>,
    pub state: String,
    pub writable: bool,
    pub is_write_target: bool,
    pub page_count: i64,
    pub last_sync_at: Option<String>,
    pub embedding_queue_depth: i64,
    #[serde(skip_serializing)]
    pub failing_jobs: i64,
    pub ignore_parse_errors: Option<Vec<IgnoreParseErrorView>>,
    pub needs_full_sync: bool,
    pub recovery_in_progress: bool,
    pub integrity_blocked: Option<String>,
    pub restore_in_progress: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSlug {
    pub collection_id: i64,
    pub collection_name: String,
    pub slug: String,
}

impl ResolvedSlug {
    pub fn canonical_slug(&self) -> String {
        format!("{}::{}", self.collection_name, self.slug)
    }
}

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
pub struct LiveCollectionOwner {
    pub session_id: String,
    pub pid: i64,
    pub host: String,
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

fn finalize_outcome_label(outcome: &FinalizeOutcome) -> &'static str {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemapVerificationSummary {
    pub resolved_pages: usize,
    pub missing_pages: usize,
    pub mismatched_pages: usize,
    pub extra_files: usize,
}

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
    #[error("IpcDirectoryInsecureError: path={path} reason={reason}")]
    IpcDirectoryInsecure { path: String, reason: String },

    #[cfg(unix)]
    #[error("IpcSocketPermissionError: path={path} reason={reason}")]
    IpcSocketPermission { path: String, reason: String },

    #[cfg(unix)]
    #[error("IpcSocketCollisionError: path={path} reason={reason}")]
    IpcSocketCollision { path: String, reason: String },

    #[cfg(unix)]
    #[error("IpcPeerAuthFailedError: path={path} reason={reason}")]
    IpcPeerAuthFailed { path: String, reason: String },

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
        "NewRootVerificationFailedError: collection={collection_name} missing={missing} mismatched={mismatched} extra={extra} missing_samples={missing_samples} mismatched_samples={mismatched_samples} extra_samples={extra_samples}"
    )]
    NewRootVerificationFailed {
        collection_name: String,
        missing: usize,
        mismatched: usize,
        extra: usize,
        missing_samples: String,
        mismatched_samples: String,
        extra_samples: String,
    },

    #[error("NewRootUnstableError: collection={collection_name}")]
    NewRootUnstable { collection_name: String },

    #[error("InvariantViolationError: {message}")]
    InvariantViolation { message: String },

    #[error("RestoreCommandBlockedError: collection={collection_name} outcome={outcome}")]
    RestoreCommandBlocked {
        collection_name: String,
        outcome: &'static str,
    },

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

    #[cfg(unix)]
    #[error("DuplicateWriteDedupError: key={key}")]
    DuplicateWriteDedup { key: String },

    #[cfg(unix)]
    #[error(
        "RecoverySentinelError: collection_id={collection_id} relative_path={relative_path} sentinel={sentinel_path} reason={reason}"
    )]
    RecoverySentinel {
        collection_id: i64,
        relative_path: String,
        sentinel_path: String,
        reason: String,
    },

    #[cfg(unix)]
    #[error(
        "ConcurrentRenameError: collection_id={collection_id} relative_path={relative_path} sentinel={sentinel_path}"
    )]
    ConcurrentRename {
        collection_id: i64,
        relative_path: String,
        sentinel_path: String,
    },

    #[cfg(all(test, unix))]
    #[error("DurabilityError: collection_id={collection_id} relative_path={relative_path}")]
    Durability {
        collection_id: i64,
        relative_path: String,
    },

    #[cfg(unix)]
    #[allow(dead_code)]
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

    #[cfg(unix)]
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=MissingExpectedVersion current_version={current_version}"
    )]
    MissingExpectedVersion {
        collection_id: i64,
        relative_path: String,
        current_version: i64,
    },

    #[cfg(unix)]
    #[error(
        "Conflict: ConflictError StaleExpectedVersion collection_id={collection_id} relative_path={relative_path} expected_version={expected_version} current version: {current_version}"
    )]
    StaleExpectedVersion {
        collection_id: i64,
        relative_path: String,
        expected_version: i64,
        current_version: i64,
    },

    #[cfg(unix)]
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=ExternalDelete"
    )]
    ExternalDelete {
        collection_id: i64,
        relative_path: String,
    },

    #[cfg(unix)]
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=ExternalCreate"
    )]
    ExternalCreate {
        collection_id: i64,
        relative_path: String,
    },

    #[cfg(unix)]
    #[error(
        "ConflictError: collection_id={collection_id} relative_path={relative_path} reason=HashMismatch stored_sha256={stored_sha256} actual_sha256={actual_sha256}"
    )]
    HashMismatch {
        collection_id: i64,
        relative_path: String,
        stored_sha256: String,
        actual_sha256: String,
    },

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

pub fn ensure_unix_platform(command: &'static str) -> Result<(), VaultSyncError> {
    #[cfg(unix)]
    {
        let _ = command;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        Err(VaultSyncError::UnsupportedPlatform { command })
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsPreconditionOutcome {
    FastPath,
    SlowPathSelfHeal,
    FreshCreate,
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone)]
struct FsPreconditionInspection {
    outcome: FsPreconditionOutcome,
    current_stat: Option<file_state::FileStat>,
    stored_row: Option<file_state::FileStateRow>,
}

#[allow(dead_code)]
pub struct ServeRuntime {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    pub session_id: String,
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IpcPeerCredentials {
    pub pid: i32,
    pub uid: u32,
}

#[cfg(unix)]
#[derive(Debug, Clone)]
pub(crate) struct LiveServeEndpoint {
    pub session_id: String,
    pub pid: i64,
    pub ipc_path: String,
}

#[cfg(unix)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum IpcRequest {
    WhoAmI,
    Put {
        slug: String,
        content: String,
        expected_version: Option<i64>,
    },
}

#[cfg(unix)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum IpcResponse {
    WhoAmI { session_id: String },
    PutOk { status: String },
    Error { error: String },
}

#[cfg(unix)]
struct PublishedIpcSocket {
    listener: UnixListener,
    path: PathBuf,
}

#[cfg(unix)]
struct IpcSocketLocation {
    runtime_root: PathBuf,
    socket_dir: PathBuf,
    create_runtime_root: bool,
}

impl Drop for ServeRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn current_host() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "unknown-host".to_owned())
}

#[cfg(unix)]
pub(crate) fn current_effective_uid() -> u32 {
    unsafe { libc::geteuid() as u32 }
}

#[cfg(unix)]
pub fn check_update_expected_version(
    collection_id: i64,
    relative_path: &str,
    current_version: Option<i64>,
    expected_version: Option<i64>,
) -> Result<(), VaultSyncError> {
    match (current_version, expected_version) {
        (Some(current_version), None) => Err(VaultSyncError::MissingExpectedVersion {
            collection_id,
            relative_path: relative_path.to_owned(),
            current_version,
        }),
        (Some(current_version), Some(expected_version)) if current_version != expected_version => {
            Err(VaultSyncError::StaleExpectedVersion {
                collection_id,
                relative_path: relative_path.to_owned(),
                expected_version,
                current_version,
            })
        }
        _ => Ok(()),
    }
}

#[cfg(unix)]
#[allow(dead_code)]
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

#[cfg(unix)]
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
        (Some(_), None) => Err(VaultSyncError::ExternalDelete {
            collection_id,
            relative_path: relative_path_str,
        }),
        (None, Some(_)) => Err(VaultSyncError::ExternalCreate {
            collection_id,
            relative_path: relative_path_str,
        }),
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
                Err(VaultSyncError::HashMismatch {
                    collection_id,
                    relative_path: relative_path_str,
                    stored_sha256: stored_row.sha256,
                    actual_sha256,
                })
            }
        }
    }
}

#[cfg(unix)]
#[allow(dead_code)]
pub(crate) fn check_fs_precondition_before_sentinel(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
) -> Result<FsPreconditionOutcome, VaultSyncError> {
    Ok(inspect_fs_precondition(conn, collection_id, root_path, relative_path)?.outcome)
}

#[cfg(unix)]
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

#[cfg(all(test, unix))]
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

fn init_process_registries() -> Result<&'static RuntimeRegistries, VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .clear();
    #[cfg(unix)]
    registries
        .self_write_dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned {
            registry: "self_write_dedup",
        })?
        .clear();
    registries
        .supervisor_handles
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned {
            registry: "supervisor_handles",
        })?
        .clear();
    registries
        .slug_writes
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned {
            registry: "slug_writes",
        })?
        .clear();
    registries
        .recovering_collections
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned {
            registry: "recovering_collections",
        })?
        .clear();
    Ok(registries)
}

fn with_supervisor_handles<T>(
    f: impl FnOnce(&mut HashMap<i64, SupervisorHandle>) -> T,
) -> Result<T, VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    let mut handles =
        registries
            .supervisor_handles
            .lock()
            .map_err(|_| VaultSyncError::RegistryPoisoned {
                registry: "supervisor_handles",
            })?;
    Ok(f(&mut handles))
}

fn with_recovering_collections<T>(
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

struct RecoveryInProgressGuard {
    collection_id: i64,
}

impl RecoveryInProgressGuard {
    fn enter(collection_id: i64) -> Result<Self, VaultSyncError> {
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

pub fn collection_watcher_health(collection_id: i64) -> Option<WatcherHealthView> {
    with_supervisor_handles(|handles| {
        handles
            .get(&collection_id)
            .and_then(|handle| handle.watcher_health.clone())
            .map(|health| WatcherHealthView {
                mode: health.mode.as_str().to_owned(),
                last_event_at: health.last_event_at,
                channel_depth: health.channel_depth as i64,
            })
    })
    .ok()
    .flatten()
}

#[cfg(test)]
pub(crate) fn set_collection_watcher_health_for_test(
    collection_id: i64,
    session_id: &str,
    generation: i64,
    mode: Option<WatcherMode>,
    last_event_at: Option<String>,
    channel_depth: usize,
) {
    let _ = with_supervisor_handles(|handles| {
        handles.insert(
            collection_id,
            SupervisorHandle {
                session_id: session_id.to_owned(),
                generation,
                watcher_health: mode.map(|mode| WatcherHealthSnapshot {
                    mode,
                    last_event_at,
                    channel_depth,
                }),
            },
        );
    });
}

#[cfg(test)]
pub(crate) fn clear_collection_watcher_health_for_test(collection_id: i64) {
    let _ = with_supervisor_handles(|handles| {
        handles.remove(&collection_id);
    });
}

fn parse_ignore_parse_errors(
    raw: Option<String>,
) -> Result<Option<Vec<IgnoreParseErrorView>>, VaultSyncError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut parsed: Vec<IgnoreParseErrorView> =
        serde_json::from_str(&raw).map_err(|error| VaultSyncError::InvariantViolation {
            message: format!("invalid ignore_parse_errors JSON: {error}"),
        })?;
    for entry in &mut parsed {
        if entry.code == "file_stably_absent_but_clear_not_confirmed" {
            entry.line = None;
            entry.raw = None;
        }
    }
    Ok((!parsed.is_empty()).then_some(parsed))
}

fn manifest_incomplete_escalation_secs() -> i64 {
    std::env::var("QUAID_MANIFEST_INCOMPLETE_ESCALATION_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MANIFEST_INCOMPLETE_ESCALATION_SECS)
}

fn integrity_blocked_label(
    integrity_failed_at: &Option<String>,
    manifest_incomplete_escalated: bool,
    reconcile_halted_at: &Option<String>,
    reconcile_halt_reason: &Option<String>,
) -> Option<String> {
    if integrity_failed_at.is_some() {
        return Some("manifest_tampering".to_owned());
    }
    if manifest_incomplete_escalated {
        return Some("manifest_incomplete_escalated".to_owned());
    }
    if reconcile_halted_at.is_none() {
        return None;
    }
    match reconcile_halt_reason.as_deref() {
        Some("duplicate_uuid") => Some("duplicate_uuid".to_owned()),
        Some("unresolvable_trivial_content") => Some("unresolvable_trivial_content".to_owned()),
        _ => None,
    }
}

fn restore_in_progress(collection: &Collection) -> bool {
    matches!(collection.state, CollectionState::Restoring)
        && collection.restore_command_id.is_some()
        && collection.watcher_released_at.is_some()
}

pub fn list_memory_collections(
    conn: &Connection,
) -> Result<Vec<MemoryCollectionView>, VaultSyncError> {
    let manifest_incomplete_escalation_secs = manifest_incomplete_escalation_secs();
    let mut stmt = conn.prepare(
        "SELECT
             c.id,
             c.name,
             c.root_path,
             c.state,
             c.writable,
             c.is_write_target,
             c.ignore_parse_errors,
             c.needs_full_sync,
              c.last_sync_at,
              c.integrity_failed_at,
              c.pending_manifest_incomplete_at,
              c.reconcile_halted_at,
              c.reconcile_halt_reason,
              c.restore_command_id,
              c.watcher_released_at,
             COALESCE((
                 SELECT COUNT(*)
                 FROM pages p
                 WHERE p.collection_id = c.id
                   AND p.quarantined_at IS NULL
             ), 0) AS page_count,
             COALESCE((
                 SELECT COUNT(*)
                 FROM embedding_jobs ej
                 JOIN pages p ON p.id = ej.page_id
                 WHERE p.collection_id = c.id
                   AND ej.job_state IN ('pending', 'running')
             ), 0) AS embedding_queue_depth,
             COALESCE((
                 SELECT COUNT(*)
                 FROM embedding_jobs ej
                 JOIN pages p ON p.id = ej.page_id
                 WHERE p.collection_id = c.id
                   AND ej.job_state = 'failed'
             ), 0) AS failing_jobs,
             CASE
                 WHEN c.pending_manifest_incomplete_at IS NOT NULL
                  AND strftime('%s', 'now') - strftime('%s', c.pending_manifest_incomplete_at) >= ?1
                 THEN 1
                 ELSE 0
             END AS manifest_incomplete_escalated
         FROM collections c
         ORDER BY c.name",
    )?;

    let rows = stmt
        .query_map([manifest_incomplete_escalation_secs], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)? != 0,
                row.get::<_, i64>(5)? != 0,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)? != 0,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, i64>(15)?,
                row.get::<_, i64>(16)?,
                row.get::<_, i64>(17)?,
                row.get::<_, i64>(18)? != 0,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(
            |(
                id,
                name,
                root_path,
                state_raw,
                writable,
                is_write_target,
                ignore_parse_errors_raw,
                needs_full_sync,
                last_sync_at,
                integrity_failed_at,
                pending_manifest_incomplete_at,
                reconcile_halted_at,
                reconcile_halt_reason,
                restore_command_id,
                watcher_released_at,
                page_count,
                embedding_queue_depth,
                failing_jobs,
                manifest_incomplete_escalated,
            )| {
                let state = state_raw.parse()?;
                let collection = Collection {
                    id,
                    name,
                    root_path,
                    state,
                    writable,
                    is_write_target,
                    ignore_patterns: None,
                    ignore_parse_errors: ignore_parse_errors_raw,
                    needs_full_sync,
                    last_sync_at,
                    active_lease_session_id: None,
                    restore_command_id,
                    restore_lease_session_id: None,
                    reload_generation: 0,
                    watcher_released_session_id: None,
                    watcher_released_generation: None,
                    watcher_released_at,
                    pending_command_heartbeat_at: None,
                    pending_root_path: None,
                    pending_restore_manifest: None,
                    restore_command_pid: None,
                    restore_command_host: None,
                    integrity_failed_at,
                    pending_manifest_incomplete_at,
                    reconcile_halted_at,
                    reconcile_halt_reason,
                    created_at: String::new(),
                    updated_at: String::new(),
                };

                Ok(MemoryCollectionView {
                    name: collection.name.clone(),
                    root_path: matches!(collection.state, CollectionState::Active)
                        .then_some(collection.root_path.clone()),
                    state: collection.state.as_str().to_owned(),
                    writable: collection.writable,
                    is_write_target: collection.is_write_target,
                    page_count,
                    last_sync_at: collection.last_sync_at.clone(),
                    embedding_queue_depth,
                    failing_jobs,
                    ignore_parse_errors: parse_ignore_parse_errors(
                        collection.ignore_parse_errors.clone(),
                    )?,
                    needs_full_sync: collection.needs_full_sync,
                    recovery_in_progress: collection.needs_full_sync
                        && collection_recovery_in_progress(collection.id),
                    integrity_blocked: integrity_blocked_label(
                        &collection.integrity_failed_at,
                        manifest_incomplete_escalated,
                        &collection.reconcile_halted_at,
                        &collection.reconcile_halt_reason,
                    ),
                    restore_in_progress: restore_in_progress(&collection),
                })
            },
        )
        .collect()
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

fn has_supervisor_handle(collection_id: i64, session_id: &str) -> Result<bool, VaultSyncError> {
    with_supervisor_handles(|handles| {
        handles
            .get(&collection_id)
            .map(|handle| handle.session_id == session_id)
            .unwrap_or(false)
    })
}

fn register_supervisor_handle(
    collection_id: i64,
    session_id: &str,
    generation: i64,
) -> Result<(), VaultSyncError> {
    with_supervisor_handles(|handles| {
        handles.insert(
            collection_id,
            SupervisorHandle {
                session_id: session_id.to_owned(),
                generation,
                watcher_health: None,
            },
        );
    })?;
    Ok(())
}

fn remove_supervisor_handle(collection_id: i64, session_id: &str) -> Result<(), VaultSyncError> {
    with_supervisor_handles(|handles| {
        if handles
            .get(&collection_id)
            .map(|handle| handle.session_id == session_id)
            .unwrap_or(false)
        {
            handles.remove(&collection_id);
        }
    })?;
    Ok(())
}

fn clear_supervisor_handles_for_session(session_id: &str) -> Result<(), VaultSyncError> {
    with_supervisor_handles(|handles| {
        handles.retain(|_, handle| handle.session_id != session_id);
    })?;
    Ok(())
}

fn sync_supervisor_handles(conn: &Connection, session_id: &str) -> Result<(), VaultSyncError> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.state, c.needs_full_sync, c.reload_generation
         FROM collections c
         JOIN collection_owners o ON o.collection_id = c.id
         WHERE o.session_id = ?1",
    )?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut owned = HashSet::new();
    for (collection_id, state, needs_full_sync, generation) in rows {
        owned.insert(collection_id);
        let _ = needs_full_sync;
        if state == CollectionState::Active.as_str() {
            register_supervisor_handle(collection_id, session_id, generation)?;
        } else {
            remove_supervisor_handle(collection_id, session_id)?;
        }
    }

    with_supervisor_handles(|handles| {
        handles.retain(|collection_id, handle| {
            handle.session_id != session_id || owned.contains(collection_id)
        });
    })?;
    Ok(())
}

fn claim_owned_collections(conn: &Connection, session_id: &str) -> Result<(), VaultSyncError> {
    let mut stmt = conn.prepare("SELECT id FROM collections")?;
    let collection_ids = stmt
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for collection_id in collection_ids {
        match acquire_owner_lease(conn, collection_id, session_id) {
            Ok(()) | Err(VaultSyncError::ServeOwnsCollectionError { .. }) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
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

fn bootstrap_recovery_directories(
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

fn recovery_sentinel_paths(
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

fn mark_collection_needs_full_sync(
    conn: &Connection,
    collection_id: i64,
) -> Result<(), VaultSyncError> {
    conn.execute(
        "UPDATE collections
         SET needs_full_sync = 1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection_id],
    )?;
    Ok(())
}

#[cfg(unix)]
fn watch_debounce_duration() -> Duration {
    std::env::var("QUAID_WATCH_DEBOUNCE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_WATCH_DEBOUNCE_MS))
}

#[cfg(unix)]
fn watcher_backoff_duration(consecutive_failures: u32) -> Duration {
    let shift = consecutive_failures.saturating_sub(1).min(6);
    Duration::from_secs((1_u64 << shift).min(60))
}

#[cfg(unix)]
fn current_timestamp(conn: &Connection) -> Result<String, VaultSyncError> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

#[cfg(unix)]
fn self_write_dedup_ttl() -> Duration {
    Duration::from_secs(SELF_WRITE_DEDUP_TTL_SECS)
}

#[cfg(unix)]
fn self_write_dedup_sweep_interval() -> Duration {
    Duration::from_secs(SELF_WRITE_DEDUP_SWEEP_SECS)
}

#[cfg(unix)]
fn with_self_write_dedup<T>(
    f: impl FnOnce(&mut HashMap<PathBuf, SelfWriteDedupEntry>) -> T,
) -> Result<T, VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    let mut dedup =
        registries
            .self_write_dedup
            .lock()
            .map_err(|_| VaultSyncError::RegistryPoisoned {
                registry: "self_write_dedup",
            })?;
    Ok(f(&mut dedup))
}

#[cfg(unix)]
fn remember_self_write_path_at(
    path: &Path,
    sha256: &str,
    inserted_at: Instant,
) -> Result<(), VaultSyncError> {
    with_self_write_dedup(|entries| {
        entries.insert(
            path.to_path_buf(),
            SelfWriteDedupEntry {
                sha256: sha256.to_owned(),
                inserted_at,
            },
        );
    })
}

#[cfg(unix)]
pub(crate) fn remember_self_write_path(path: &Path, sha256: &str) -> Result<(), VaultSyncError> {
    remember_self_write_path_at(path, sha256, Instant::now())
}

#[cfg(unix)]
pub(crate) fn forget_self_write_path(path: &Path) -> Result<(), VaultSyncError> {
    with_self_write_dedup(|entries| {
        entries.remove(path);
    })
}

#[cfg(unix)]
fn self_write_should_suppress_at(
    path: &Path,
    current_sha256: &str,
    now: Instant,
) -> Result<bool, VaultSyncError> {
    with_self_write_dedup(|entries| {
        entries
            .get(path)
            .map(|entry| {
                now.duration_since(entry.inserted_at) < self_write_dedup_ttl()
                    && entry.sha256 == current_sha256
            })
            .unwrap_or(false)
    })
}

#[cfg(unix)]
fn sweep_expired_self_write_entries_at(now: Instant) -> Result<usize, VaultSyncError> {
    let ttl = self_write_dedup_ttl();
    with_self_write_dedup(|entries| {
        let before = entries.len();
        entries.retain(|_, entry| now.duration_since(entry.inserted_at) < ttl);
        before.saturating_sub(entries.len())
    })
}

#[cfg(unix)]
fn maybe_suppress_self_write_event(path: &Path) -> Result<bool, VaultSyncError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Ok(false);
    }
    let hash = match file_state::hash_file(path) {
        Ok(hash) => hash,
        Err(_) => return Ok(false),
    };
    self_write_should_suppress_at(path, &hash, Instant::now())
}

#[cfg(unix)]
#[allow(dead_code)]
pub fn mark_collection_needs_full_sync_via_fresh_connection(
    conn: &Connection,
    collection_id: i64,
) -> Result<(), VaultSyncError> {
    let db_path = database_path(conn)?;
    let fresh = Connection::open(db_path)?;
    fresh.busy_timeout(Duration::from_millis(0))?;
    mark_collection_needs_full_sync(&fresh, collection_id)
}

#[cfg(unix)]
#[allow(dead_code)]
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
        Err(VaultSyncError::DuplicateWriteDedup {
            key: key.to_owned(),
        })
    }
}

#[cfg(unix)]
#[allow(dead_code)]
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
#[allow(dead_code)]
pub fn has_write_dedup(key: &str) -> Result<bool, VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    Ok(registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .contains(key))
}

pub fn with_write_slug_lock<T, F>(
    collection_id: i64,
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
            .entry(format!("{collection_id}:{relative_path}"))
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().map_err(|_| VaultSyncError::RegistryPoisoned {
        registry: "slug_write_guard",
    })?;
    Ok(action())
}

#[cfg(all(test, unix))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum WriterSideSentinelCrashMode {
    SentinelCreateFail,
    PreRenameAbortAfterDedup,
    RenameFail,
    FsyncParentFail,
    ForeignRenameBetweenRenameAndStat { foreign_bytes: Vec<u8> },
}

#[cfg(all(test, unix))]
fn writer_side_sentinel_name(write_id: &str) -> String {
    format!("{write_id}.needs_full_sync")
}

#[cfg(all(test, unix))]
fn writer_side_tempfile_name(write_id: &str) -> String {
    format!("{write_id}.tmp")
}

#[cfg(all(test, unix))]
fn writer_side_foreign_tempfile_name(write_id: &str) -> String {
    format!("{write_id}.foreign.tmp")
}

#[cfg(all(test, unix))]
fn writer_side_dedup_key(target_path: &Path, bytes: &[u8]) -> String {
    format!("{}::{}", target_path.display(), sha256_hex(bytes))
}

#[cfg(all(test, unix))]
fn insert_writer_side_dedup_entry(key: &str) -> Result<(), VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .insert(key.to_owned());
    Ok(())
}

#[cfg(all(test, unix))]
fn remove_writer_side_dedup_entry(key: &str) -> Result<(), VaultSyncError> {
    let registries = PROCESS_REGISTRIES.get_or_init(RuntimeRegistries::new);
    registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .remove(key);
    Ok(())
}

#[cfg(all(test, unix))]
fn writer_side_dedup_contains(key: &str) -> bool {
    PROCESS_REGISTRIES
        .get_or_init(RuntimeRegistries::new)
        .dedup
        .lock()
        .unwrap()
        .contains(key)
}

#[cfg(all(test, unix))]
fn best_effort_mark_collection_needs_full_sync_fresh(
    conn: &Connection,
    collection_id: i64,
) -> Result<(), VaultSyncError> {
    let db_path = database_path(conn)?;
    if db_path.is_empty() || db_path == ":memory:" {
        return Ok(());
    }
    let fresh = db::open(&db_path).map_err(|err| VaultSyncError::InvariantViolation {
        message: format!("fresh db open failed for needs_full_sync escalation: {err}"),
    })?;
    mark_collection_needs_full_sync(&fresh, collection_id)
}

#[cfg(all(test, unix))]
fn cleanup_pre_rename_writer_side_failure(
    parent_fd: &rustix::fd::OwnedFd,
    tempfile_name: &Path,
    dedup_key: &str,
    sentinel_path: &Path,
) {
    let _ = remove_writer_side_dedup_entry(dedup_key);
    let _ = fs_safety::unlinkat_parent_fd(parent_fd, tempfile_name);
    let _ = fs::remove_file(sentinel_path);
}

#[cfg(all(test, unix))]
fn cleanup_post_rename_writer_side_abort(conn: &Connection, collection_id: i64, dedup_key: &str) {
    let _ = remove_writer_side_dedup_entry(dedup_key);
    let _ = best_effort_mark_collection_needs_full_sync_fresh(conn, collection_id);
}

#[cfg(all(test, unix))]
fn exercise_writer_side_sentinel_crash_core(
    conn: &Connection,
    collection_id: i64,
    relative_path: &Path,
    bytes: &[u8],
    write_id: &str,
    mode: &WriterSideSentinelCrashMode,
) -> Result<(), VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    let root_path = PathBuf::from(&collection.root_path);
    let target_path = root_path.join(relative_path);
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let db_path = PathBuf::from(database_path(conn)?);
    let recovery_root = recovery_root_for_db_path(&db_path);
    bootstrap_recovery_directories(conn, &recovery_root)?;
    let sentinel_path = collection_recovery_dir(&recovery_root, collection_id)
        .join(writer_side_sentinel_name(write_id));
    let target_name =
        relative_path
            .file_name()
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: format!(
                    "relative path has no terminal component: {}",
                    relative_path.display()
                ),
            })?;
    let tempfile_name = PathBuf::from(writer_side_tempfile_name(write_id));
    let dedup_key = writer_side_dedup_key(&target_path, bytes);

    if matches!(mode, WriterSideSentinelCrashMode::SentinelCreateFail) {
        return Err(VaultSyncError::RecoverySentinel {
            collection_id,
            relative_path: relative_path.display().to_string(),
            sentinel_path: sentinel_path.display().to_string(),
            reason: "injected sentinel create failure".to_owned(),
        });
    }

    let recovery_dir =
        sentinel_path
            .parent()
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: format!("sentinel path has no parent: {}", sentinel_path.display()),
            })?;
    let recovery_dir_fd =
        fs_safety::open_root_fd(recovery_dir).map_err(|err| VaultSyncError::RecoverySentinel {
            collection_id,
            relative_path: relative_path.display().to_string(),
            sentinel_path: sentinel_path.display().to_string(),
            reason: err.to_string(),
        })?;
    let sentinel_name =
        Path::new(
            sentinel_path
                .file_name()
                .ok_or_else(|| VaultSyncError::InvariantViolation {
                    message: format!(
                        "sentinel path has no file name: {}",
                        sentinel_path.display()
                    ),
                })?,
        );
    fs_safety::openat_create_excl(&recovery_dir_fd, sentinel_name).map_err(|err| {
        VaultSyncError::RecoverySentinel {
            collection_id,
            relative_path: relative_path.display().to_string(),
            sentinel_path: sentinel_path.display().to_string(),
            reason: err.to_string(),
        }
    })?;
    fsync(&recovery_dir_fd).map_err(|err| VaultSyncError::RecoverySentinel {
        collection_id,
        relative_path: relative_path.display().to_string(),
        sentinel_path: sentinel_path.display().to_string(),
        reason: err.to_string(),
    })?;

    let root_fd = fs_safety::open_root_fd(&root_path)?;
    let parent_fd = fs_safety::walk_to_parent(&root_fd, relative_path)?;
    let tempfile_fd = fs_safety::openat_create_excl(&parent_fd, &tempfile_name)?;
    let mut tempfile = std::fs::File::from(tempfile_fd);
    tempfile.write_all(bytes)?;
    tempfile.sync_all()?;
    let tempfile_inode = tempfile.metadata()?.ino() as i64;
    drop(tempfile);

    if let Ok(stat) = fs_safety::stat_at_nofollow(&parent_fd, Path::new(target_name)) {
        if stat.is_symlink() {
            cleanup_pre_rename_writer_side_failure(
                &parent_fd,
                &tempfile_name,
                &dedup_key,
                &sentinel_path,
            );
            return Err(VaultSyncError::InvariantViolation {
                message: format!(
                    "symlink target rejected during writer crash core proof: {}",
                    relative_path.display()
                ),
            });
        }
    }

    insert_writer_side_dedup_entry(&dedup_key)?;
    if matches!(mode, WriterSideSentinelCrashMode::PreRenameAbortAfterDedup) {
        cleanup_pre_rename_writer_side_failure(
            &parent_fd,
            &tempfile_name,
            &dedup_key,
            &sentinel_path,
        );
        return Err(VaultSyncError::InvariantViolation {
            message: format!(
                "injected pre-rename abort after dedup for {}",
                relative_path.display()
            ),
        });
    }

    if matches!(mode, WriterSideSentinelCrashMode::RenameFail) {
        cleanup_pre_rename_writer_side_failure(
            &parent_fd,
            &tempfile_name,
            &dedup_key,
            &sentinel_path,
        );
        return Err(io::Error::other(format!(
            "injected rename failure for {}",
            relative_path.display()
        ))
        .into());
    }

    fs_safety::renameat_parent_fd(&parent_fd, &tempfile_name, Path::new(target_name))?;

    if matches!(mode, WriterSideSentinelCrashMode::FsyncParentFail) {
        cleanup_post_rename_writer_side_abort(conn, collection_id, &dedup_key);
        return Err(VaultSyncError::Durability {
            collection_id,
            relative_path: relative_path.display().to_string(),
        });
    }
    fsync(&parent_fd).map_err(|err| io::Error::from_raw_os_error(err.raw_os_error()))?;

    if let WriterSideSentinelCrashMode::ForeignRenameBetweenRenameAndStat { foreign_bytes } = mode {
        let foreign_tempfile_name = PathBuf::from(writer_side_foreign_tempfile_name(write_id));
        let foreign_fd = fs_safety::openat_create_excl(&parent_fd, &foreign_tempfile_name)?;
        let mut foreign_tempfile = std::fs::File::from(foreign_fd);
        foreign_tempfile.write_all(foreign_bytes)?;
        foreign_tempfile.sync_all()?;
        drop(foreign_tempfile);
        fs_safety::renameat_parent_fd(&parent_fd, &foreign_tempfile_name, Path::new(target_name))?;
    }

    let target_stat = fs_safety::stat_at_nofollow(&parent_fd, Path::new(target_name))?;
    if target_stat.inode != tempfile_inode {
        cleanup_post_rename_writer_side_abort(conn, collection_id, &dedup_key);
        return Err(VaultSyncError::ConcurrentRename {
            collection_id,
            relative_path: relative_path.display().to_string(),
            sentinel_path: sentinel_path.display().to_string(),
        });
    }

    cleanup_post_rename_writer_side_abort(conn, collection_id, &dedup_key);
    Err(VaultSyncError::InvariantViolation {
        message: format!(
            "writer crash core proof seam stops before happy-path commit for {}",
            relative_path.display()
        ),
    })
}

fn recover_owned_collection_sentinels(
    conn: &Connection,
    recovery_root: &Path,
    session_id: &str,
) -> Result<(), VaultSyncError> {
    bootstrap_recovery_directories(conn, recovery_root)?;
    let mut stmt = conn.prepare(
        "SELECT c.id
         FROM collections c
         JOIN collection_owners o ON o.collection_id = c.id
         WHERE o.session_id = ?1",
    )?;
    let collection_ids = stmt
        .query_map([session_id], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    for collection_id in collection_ids {
        let sentinel_paths = recovery_sentinel_paths(recovery_root, collection_id)?;
        if sentinel_paths.is_empty() {
            continue;
        }

        let collection = load_collection_by_id(conn, collection_id)?;
        if collection.state != CollectionState::Active
            || collection.pending_root_path.is_some()
            || collection.restore_command_id.is_some()
            || collection.integrity_failed_at.is_some()
            || collection.pending_manifest_incomplete_at.is_some()
            || collection.reconcile_halted_at.is_some()
        {
            continue;
        }

        mark_collection_needs_full_sync(conn, collection_id)?;
        if matches!(
            complete_attach(
                conn,
                collection_id,
                session_id,
                AttachReason::RemapPostReconcile,
            ),
            Ok(true)
        ) {
            for sentinel_path in sentinel_paths {
                let _ = fs::remove_file(sentinel_path);
            }
        }
    }

    Ok(())
}

fn run_startup_sequence(
    conn: &Connection,
    db_path: &Path,
    session_id: &str,
) -> Result<(), VaultSyncError> {
    sweep_stale_sessions(conn)?;
    claim_owned_collections(conn, session_id)?;
    recover_owned_collection_sentinels(conn, &recovery_root_for_db_path(db_path), session_id)?;
    let _ = resume_orphaned_embedding_jobs(conn)?;
    let _ = quarantine::sweep_expired_quarantined_pages(conn);
    let _ = run_rcrt_pass(conn, session_id);
    sync_supervisor_handles(conn, session_id)?;
    Ok(())
}

pub(crate) fn resume_orphaned_embedding_jobs(conn: &Connection) -> Result<usize, VaultSyncError> {
    conn.execute(
        "UPDATE embedding_jobs
         SET job_state = 'pending',
             started_at = NULL
         WHERE job_state = 'running'",
        [],
    )
    .map_err(Into::into)
}

pub fn database_path(conn: &Connection) -> Result<String, VaultSyncError> {
    conn.query_row("PRAGMA database_list", [], |row| row.get::<_, String>(2))
        .map_err(Into::into)
}

pub fn register_session(conn: &Connection) -> Result<String, VaultSyncError> {
    let session_id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host) VALUES (?1, ?2, ?3)",
        params![session_id, std::process::id() as i64, current_host()],
    )?;
    Ok(session_id)
}

pub fn register_cli_session(conn: &Connection) -> Result<String, VaultSyncError> {
    let session_id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type) VALUES (?1, ?2, ?3, 'cli')",
        params![session_id, std::process::id() as i64, current_host()],
    )?;
    Ok(session_id)
}

pub fn unregister_session(conn: &Connection, session_id: &str) -> Result<(), VaultSyncError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM collection_owners WHERE session_id = ?1",
        [session_id],
    )?;
    tx.execute(
        "DELETE FROM serve_sessions WHERE session_id = ?1",
        [session_id],
    )?;
    tx.execute(
        "UPDATE collections
         SET active_lease_session_id = CASE
                 WHEN active_lease_session_id = ?1 THEN NULL
                 ELSE active_lease_session_id
             END,
             restore_lease_session_id = CASE
                 WHEN restore_lease_session_id = ?1 THEN NULL
                 ELSE restore_lease_session_id
             END,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE active_lease_session_id = ?1 OR restore_lease_session_id = ?1",
        [session_id],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn heartbeat_session(conn: &Connection, session_id: &str) -> Result<(), VaultSyncError> {
    conn.execute(
        "UPDATE serve_sessions
         SET heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE session_id = ?1",
        [session_id],
    )?;
    Ok(())
}

pub fn sweep_stale_sessions(conn: &Connection) -> Result<usize, VaultSyncError> {
    let removed = conn.execute(
        "DELETE FROM serve_sessions
         WHERE heartbeat_at < datetime('now', ?1)",
        [format!("-{SESSION_LIVENESS_SECS} seconds")],
    )?;
    Ok(removed)
}

pub fn live_collection_owner(
    conn: &Connection,
    collection_id: i64,
) -> Result<Option<LiveCollectionOwner>, VaultSyncError> {
    conn.query_row(
        "SELECT o.session_id, s.pid, s.host
         FROM collection_owners o
         JOIN serve_sessions s ON s.session_id = o.session_id
         WHERE o.collection_id = ?1
           AND s.heartbeat_at >= datetime('now', ?2)
           AND s.session_type = 'serve'",
        params![collection_id, format!("-{SESSION_LIVENESS_SECS} seconds")],
        |row| {
            Ok(LiveCollectionOwner {
                session_id: row.get(0)?,
                pid: row.get(1)?,
                host: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn collection_ids_for_root_path(
    conn: &Connection,
    root_path: &str,
) -> Result<Vec<i64>, VaultSyncError> {
    let mut stmt = conn.prepare(
        "SELECT id
         FROM collections
         WHERE root_path = ?1
         ORDER BY id",
    )?;
    let rows = stmt.query_map([root_path], |row| row.get::<_, i64>(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn live_collection_owner_for_root_path(
    conn: &Connection,
    root_path: &str,
) -> Result<Option<(String, LiveCollectionOwner)>, VaultSyncError> {
    conn.query_row(
        "SELECT c.name, o.session_id, s.pid, s.host
         FROM collections c
         JOIN collection_owners o ON o.collection_id = c.id
         JOIN serve_sessions s ON s.session_id = o.session_id
         WHERE c.root_path = ?1
           AND s.heartbeat_at >= datetime('now', ?2)
           AND s.session_type = 'serve'
         ORDER BY c.id
         LIMIT 1",
        params![root_path, format!("-{SESSION_LIVENESS_SECS} seconds")],
        |row| {
            Ok((
                row.get(0)?,
                LiveCollectionOwner {
                    session_id: row.get(1)?,
                    pid: row.get(2)?,
                    host: row.get(3)?,
                },
            ))
        },
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(unix)]
pub(crate) fn live_serve_endpoint_for_root_path(
    conn: &Connection,
    root_path: &str,
) -> Result<Option<LiveServeEndpoint>, VaultSyncError> {
    let row = conn
        .query_row(
            "SELECT o.session_id, s.pid, s.ipc_path
             FROM collections c
             JOIN collection_owners o ON o.collection_id = c.id
             JOIN serve_sessions s ON s.session_id = o.session_id
             WHERE c.root_path = ?1
               AND s.heartbeat_at >= datetime('now', ?2)
               AND s.session_type = 'serve'
             ORDER BY c.id
             LIMIT 1",
            params![root_path, format!("-{SESSION_LIVENESS_SECS} seconds")],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    match row {
        None => Ok(None),
        Some((session_id, pid, Some(ipc_path))) => Ok(Some(LiveServeEndpoint {
            session_id,
            pid,
            ipc_path,
        })),
        // ipc_path may be NULL during the shutdown race window (socket cleared before
        // the session row is unregistered).  Treat this as "no live owner" so that the
        // caller falls back to the direct-write path, where owner-lease checks still
        // protect the collection.
        Some((_session_id, _pid, None)) => Ok(None),
    }
}

#[allow(dead_code)]
pub fn ensure_no_live_serve_owner(
    conn: &Connection,
    collection_id: i64,
) -> Result<(), VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    if let Some(owner) = live_collection_owner(conn, collection_id)? {
        return Err(VaultSyncError::ServeOwnsCollectionError {
            collection_name: collection.name,
            owner_session_id: owner.session_id,
            owner_pid: owner.pid,
            owner_host: owner.host,
        });
    }
    Ok(())
}

pub fn ensure_no_live_serve_owner_for_root_path(
    conn: &Connection,
    root_path: &str,
) -> Result<(), VaultSyncError> {
    if let Some((collection_name, owner)) = live_collection_owner_for_root_path(conn, root_path)? {
        return Err(VaultSyncError::ServeOwnsCollectionError {
            collection_name,
            owner_session_id: owner.session_id,
            owner_pid: owner.pid,
            owner_host: owner.host,
        });
    }
    Ok(())
}

pub fn owner_session_id(
    conn: &Connection,
    collection_id: i64,
) -> Result<Option<String>, VaultSyncError> {
    conn.query_row(
        "SELECT session_id FROM collection_owners WHERE collection_id = ?1",
        [collection_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn acquire_owner_lease(
    conn: &Connection,
    collection_id: i64,
    session_id: &str,
) -> Result<(), VaultSyncError> {
    if let Some(owner) = live_collection_owner(conn, collection_id)? {
        if owner.session_id != session_id {
            let collection = load_collection_by_id(conn, collection_id)?;
            return Err(VaultSyncError::ServeOwnsCollectionError {
                collection_name: collection.name,
                owner_session_id: owner.session_id,
                owner_pid: owner.pid,
                owner_host: owner.host,
            });
        }
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO collection_owners (collection_id, session_id)
         VALUES (?1, ?2)
         ON CONFLICT(collection_id) DO UPDATE SET
             session_id = excluded.session_id,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        params![collection_id, session_id],
    )?;
    tx.execute(
        "UPDATE collections
         SET active_lease_session_id = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![collection_id, session_id],
    )?;
    tx.commit()?;
    Ok(())
}

#[allow(dead_code)]
pub fn release_owner_lease(
    conn: &Connection,
    collection_id: i64,
    session_id: &str,
) -> Result<(), VaultSyncError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2",
        params![collection_id, session_id],
    )?;
    tx.execute(
        "UPDATE collections
         SET active_lease_session_id = CASE
                 WHEN active_lease_session_id = ?2 THEN NULL
                 ELSE active_lease_session_id
             END,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![collection_id, session_id],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn sync_collection(
    conn: &Connection,
    collection_name: &str,
) -> Result<ReconcileStats, VaultSyncError> {
    let collection = collections::get_by_name(conn, collection_name)?.ok_or_else(|| {
        VaultSyncError::CollectionNotFound {
            name: collection_name.to_owned(),
        }
    })?;
    ensure_plain_sync_allowed(&collection)?;

    let _lease = start_short_lived_owner_lease(conn, collection.id)?;
    let collection = load_collection_by_id(conn, collection.id)?;
    let stats = match reconcile(conn, &collection) {
        Ok(stats) => stats,
        Err(err) => {
            return Err(convert_reconcile_error(
                conn,
                collection.id,
                &collection.name,
                err,
            )?)
        }
    };

    conn.execute(
        "UPDATE collections
         SET needs_full_sync = 0,
             last_sync_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1 AND state = 'active'",
        [collection.id],
    )?;
    Ok(stats)
}

pub fn ensure_collection_write_allowed(
    conn: &Connection,
    collection_id: i64,
) -> Result<(), VaultSyncError> {
    check_writable(conn, collection_id)
}

pub fn check_writable(conn: &Connection, collection_id: i64) -> Result<(), VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    if collection.state == CollectionState::Restoring || collection.needs_full_sync {
        return Err(VaultSyncError::CollectionRestoring {
            collection_name: collection.name,
            state: collection.state.as_str().to_owned(),
            needs_full_sync: collection.needs_full_sync,
        });
    }
    Ok(())
}

pub fn ensure_collection_vault_write_allowed(
    conn: &Connection,
    collection_id: i64,
) -> Result<(), VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    check_writable(conn, collection_id)?;
    if !collection.writable {
        return Err(VaultSyncError::CollectionReadOnly {
            collection_name: collection.name,
        });
    }
    Ok(())
}

pub fn write_quaid_id_to_file(
    conn: &Connection,
    collection: &Collection,
    page_id: i64,
) -> Result<WriteBackOutcome, VaultSyncError> {
    ensure_collection_vault_write_allowed(conn, collection.id)?;

    let (slug, version, frontmatter_json): (String, i64, String) = conn.query_row(
        "SELECT slug, version, frontmatter
         FROM pages
         WHERE id = ?1 AND collection_id = ?2",
        params![page_id, collection.id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let frontmatter: HashMap<String, String> =
        serde_json::from_str(&frontmatter_json).unwrap_or_default();
    if frontmatter.contains_key(page_uuid::QUAID_ID_FRONTMATTER_KEY) {
        return Ok(WriteBackOutcome::AlreadyHadUuid);
    }

    let page = get_page_by_key(conn, collection.id, &slug).map_err(|error| {
        VaultSyncError::InvariantViolation {
            message: error.to_string(),
        }
    })?;
    let rendered = markdown::render_page(&page);
    let canonical_slug = format!("{}::{}", collection.name, slug);

    match put::put_from_string_quiet(conn, &canonical_slug, &rendered, Some(version)) {
        Ok(()) => Ok(WriteBackOutcome::Migrated),
        Err(error) => match error.downcast::<VaultSyncError>() {
            Ok(vault_error) => match vault_error {
                VaultSyncError::Io(io_error) => {
                    let raw = io_error.raw_os_error().unwrap_or_default();
                    if io_error.kind() == io::ErrorKind::PermissionDenied || raw == 30 {
                        eprintln!(
                            "WARN: quaid_id_write_back_skipped_read_only collection={} slug={} error={}",
                            collection.name, slug, io_error
                        );
                        Ok(WriteBackOutcome::SkippedReadOnly)
                    } else {
                        Err(VaultSyncError::Io(io_error))
                    }
                }
                other => Err(other),
            },
            Err(other) => Err(VaultSyncError::InvariantViolation {
                message: other.to_string(),
            }),
        },
    }
}

fn ensure_plain_sync_allowed(collection: &Collection) -> Result<(), VaultSyncError> {
    if collection.reconcile_halted_at.is_some() {
        return Err(VaultSyncError::ReconcileHalted {
            collection_name: collection.name.clone(),
            reason: collection
                .reconcile_halt_reason
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
        });
    }
    if collection.integrity_failed_at.is_some() {
        return Err(VaultSyncError::RestoreIntegrityBlocked {
            collection_name: collection.name.clone(),
            blocking_column: "integrity_failed_at",
        });
    }
    if collection.pending_manifest_incomplete_at.is_some() {
        return Err(VaultSyncError::RestoreIntegrityBlocked {
            collection_name: collection.name.clone(),
            blocking_column: "pending_manifest_incomplete_at",
        });
    }

    match collection.state {
        CollectionState::Active => Ok(()),
        CollectionState::Restoring => {
            if let Some(pending_root_path) = collection.pending_root_path.clone() {
                return Err(VaultSyncError::RestorePendingFinalize {
                    collection_name: collection.name.clone(),
                    pending_root_path,
                });
            }
            Err(VaultSyncError::RestoreInProgress {
                collection_name: collection.name.clone(),
            })
        }
        CollectionState::Detached => Err(VaultSyncError::PlainSyncActiveRootRequired {
            collection_name: collection.name.clone(),
            state: collection.state.as_str().to_owned(),
        }),
    }
}

fn convert_reconcile_error(
    conn: &Connection,
    collection_id: i64,
    collection_name: &str,
    err: ReconcileError,
) -> Result<VaultSyncError, VaultSyncError> {
    let Some((halt_reason, rendered_reason)) = reconcile_halt_details(&err) else {
        return Ok(VaultSyncError::Reconcile(err));
    };
    conn.execute(
        "UPDATE collections
         SET reconcile_halted_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             reconcile_halt_reason = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![collection_id, halt_reason],
    )?;
    Ok(VaultSyncError::ReconcileHalted {
        collection_name: collection_name.to_owned(),
        reason: rendered_reason,
    })
}

#[cfg(unix)]
fn relative_markdown_path(root_path: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(root_path).ok()?;
    is_markdown_file(relative).then(|| relative.to_path_buf())
}

#[cfg(unix)]
fn is_root_ignore_path(root_path: &Path, path: &Path) -> bool {
    path.strip_prefix(root_path)
        .ok()
        .is_some_and(|relative| relative == Path::new(".quaidignore"))
}

#[cfg(unix)]
fn should_suppress_self_write_rename(
    root_path: &Path,
    event_paths: &[PathBuf],
) -> Result<bool, VaultSyncError> {
    let Some(target_path) = event_paths.get(1) else {
        return Ok(false);
    };
    if !maybe_suppress_self_write_event(target_path)? {
        return Ok(false);
    }
    let Some(source_path) = event_paths.first() else {
        return Ok(true);
    };
    if relative_markdown_path(root_path, source_path).is_none() {
        return Ok(true);
    }
    maybe_suppress_self_write_event(source_path)
}

#[cfg(unix)]
fn classify_watch_event(
    root_path: &Path,
    event: NotifyEvent,
) -> Result<Vec<WatchEvent>, VaultSyncError> {
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
        if maybe_suppress_self_write_event(&full_path)? {
            continue;
        }
        actions.push(WatchEvent::DirtyPath(relative_path));
    }
    Ok(actions)
}

#[cfg(unix)]
fn watch_callback(
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
                    if let Ok(conn) = Connection::open(&db_path) {
                        let _ = mark_collection_needs_full_sync(&conn, collection_id);
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

#[cfg(unix)]
#[cfg(test)]
static FORCE_NATIVE_WATCHER_INIT_FAILURE: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
#[cfg(test)]
fn set_force_native_watcher_init_failure(enabled: bool) {
    FORCE_NATIVE_WATCHER_INIT_FAILURE.store(enabled, Ordering::SeqCst);
}

#[cfg(unix)]
fn start_collection_watcher(
    collection_id: i64,
    root_path: &Path,
    db_path: &str,
) -> Result<CollectionWatcherState, VaultSyncError> {
    let (sender, receiver) = mpsc::channel(WATCH_CHANNEL_CAPACITY);
    let watch_root = root_path.to_path_buf();
    let db_path = db_path.to_owned();
    #[cfg(test)]
    let native_init_forced_error = FORCE_NATIVE_WATCHER_INIT_FAILURE.load(Ordering::SeqCst);
    #[cfg(not(test))]
    let native_init_forced_error = false;
    let native_result = if native_init_forced_error {
        Err("forced native watcher init failure".to_owned())
    } else {
        // Wrap the native init sequence in a closure so that failures produce
        // Err(String) into `native_result` rather than propagating with `?`
        // out of `start_collection_watcher`, which would bypass the poll-watcher
        // fallback in the `match native_result` block below.
        (|| -> Result<WatcherHandle, String> {
            let mut watcher = notify::recommended_watcher(watch_callback(
                collection_id,
                watch_root.clone(),
                db_path.clone(),
                sender.clone(),
            ))
            .map_err(|e| e.to_string())?;
            watcher
                .configure(NotifyConfig::default())
                .map_err(|e| e.to_string())?;
            watcher
                .watch(&watch_root, RecursiveMode::Recursive)
                .map_err(|e| e.to_string())?;
            Ok(WatcherHandle::Native(watcher))
        })()
    };
    let (watcher, mode) = match native_result {
        Ok(watcher) => (Some(watcher), WatcherMode::Native),
        Err(error) => {
            eprintln!(
                "WARN: watcher_native_init_failed collection_id={} error={} falling_back_to_poll",
                collection_id, error
            );
            let mut watcher = PollWatcher::new(
                watch_callback(collection_id, watch_root.clone(), db_path, sender),
                NotifyConfig::default(),
            )
            .map_err(|poll_error| VaultSyncError::InvariantViolation {
                message: format!(
                    "failed to create poll watcher for collection_id={collection_id}: {poll_error}"
                ),
            })?;
            watcher
                .watch(&watch_root, RecursiveMode::Recursive)
                .map_err(|poll_error| VaultSyncError::InvariantViolation {
                    message: format!(
                        "failed to watch root {} with poll watcher for collection_id={collection_id}: {poll_error}",
                        watch_root.display()
                    ),
                })?;
            (Some(WatcherHandle::Poll(watcher)), WatcherMode::Poll)
        }
    };
    Ok(CollectionWatcherState {
        root_path: watch_root,
        generation: 0,
        receiver,
        watcher,
        buffer: WatchBatchBuffer::default(),
        mode,
        last_event_at: None,
        last_watcher_error: None,
        backoff_until: None,
        consecutive_failures: 0,
    })
}

#[cfg(unix)]
fn sync_collection_watchers(
    conn: &Connection,
    db_path: &str,
    watchers: &mut HashMap<i64, CollectionWatcherState>,
) -> Result<(), VaultSyncError> {
    let _ = detach_active_collections_with_empty_root_path(conn)?;
    let mut active = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, root_path, reload_generation
         FROM collections
         WHERE state = 'active' AND trim(root_path) != ''",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (collection_id, root_path, generation) in rows {
        active.insert(collection_id, (root_path, generation));
    }

    watchers.retain(|collection_id, _| active.contains_key(collection_id));

    for (collection_id, (root_path, generation)) in active {
        if let Some(state) = watchers.get(&collection_id) {
            let same_target = state.root_path == root_path && state.generation == generation;
            if same_target
                && matches!(state.mode, WatcherMode::Crashed)
                && state
                    .backoff_until
                    .is_some_and(|backoff_until| Instant::now() < backoff_until)
            {
                continue;
            }
        }
        let needs_replace = watchers
            .get(&collection_id)
            .map(|state| {
                state.root_path != root_path
                    || state.generation != generation
                    || matches!(state.mode, WatcherMode::Crashed)
            })
            .unwrap_or(true);
        if !needs_replace {
            continue;
        }
        let previous_failures = watchers
            .get(&collection_id)
            .map(|state| state.consecutive_failures)
            .unwrap_or(0);
        let previous_last_error = watchers
            .get(&collection_id)
            .and_then(|state| state.last_watcher_error);
        let mut state = start_collection_watcher(collection_id, &root_path, db_path)?;
        state.generation = generation;
        state.consecutive_failures = previous_failures;
        state.last_watcher_error = previous_last_error;
        watchers.insert(collection_id, state);
    }
    Ok(())
}

#[cfg(any(test, unix))]
fn detach_active_collections_with_empty_root_path(
    conn: &Connection,
) -> Result<Vec<i64>, VaultSyncError> {
    let mut stmt = conn.prepare(
        "SELECT id, name
         FROM collections
         WHERE state = 'active' AND trim(root_path) = ''",
    )?;
    let collections = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if !collections.is_empty() {
        conn.execute(
            "UPDATE collections
             SET state = 'detached',
                  updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE state = 'active' AND trim(root_path) = ''",
            [],
        )?;
        for (collection_id, name) in &collections {
            eprintln!(
                "WARN: serve_detached_empty_root collection={} collection_id={}",
                name, collection_id
            );
        }
    }
    Ok(collections
        .into_iter()
        .map(|(collection_id, _)| collection_id)
        .collect())
}

#[cfg(unix)]
fn run_watcher_reconcile(
    conn: &Connection,
    collection_id: i64,
    native_renames: &[crate::core::reconciler::NativeRename],
) -> Result<(), VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    if collection.state != CollectionState::Active {
        return Ok(());
    }

    match crate::core::reconciler::reconcile_with_native_events(conn, &collection, native_renames) {
        Ok(_) => {
            conn.execute(
                "UPDATE collections
                 SET needs_full_sync = 0,
                     last_sync_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE id = ?1 AND state = 'active'",
                [collection.id],
            )?;
            Ok(())
        }
        Err(err) => Err(convert_reconcile_error(
            conn,
            collection.id,
            &collection.name,
            err,
        )?),
    }
}

#[cfg(unix)]
fn poll_collection_watcher(
    conn: &Connection,
    collection_id: i64,
    state: &mut CollectionWatcherState,
) -> Result<(), VaultSyncError> {
    if matches!(state.mode, WatcherMode::Crashed) {
        return Ok(());
    }
    let debounce = watch_debounce_duration();
    let mut received_event = false;
    loop {
        match state.receiver.try_recv() {
            Ok(WatchEvent::DirtyPath(path)) => {
                received_event = true;
                state.buffer.dirty_paths.insert(path);
                state.buffer.debounce_deadline = Some(Instant::now() + debounce);
            }
            Ok(WatchEvent::NativeRename(rename)) => {
                received_event = true;
                state.buffer.native_renames.push(rename);
                state.buffer.debounce_deadline = Some(Instant::now() + debounce);
            }
            Ok(WatchEvent::IgnoreFileChanged) => {
                received_event = true;
                state.buffer.ignore_file_changed = true;
                state.buffer.debounce_deadline = Some(Instant::now() + debounce);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                return Err(VaultSyncError::InvariantViolation {
                    message: format!(
                        "watch channel disconnected for collection_id={collection_id}"
                    ),
                });
            }
        }
    }
    if received_event {
        state.last_event_at = Some(current_timestamp(conn)?);
        state.consecutive_failures = 0;
        state.backoff_until = None;
    }

    let Some(deadline) = state.buffer.debounce_deadline else {
        return Ok(());
    };
    if Instant::now() < deadline {
        return Ok(());
    }

    let native_renames = std::mem::take(&mut state.buffer.native_renames);
    let ignore_file_changed = state.buffer.ignore_file_changed;
    state.buffer.ignore_file_changed = false;
    state.buffer.dirty_paths.clear();
    state.buffer.debounce_deadline = None;
    if ignore_file_changed {
        match crate::core::ignore_patterns::reload_patterns(conn, collection_id, &state.root_path) {
            Ok(()) => {}
            Err(error) => {
                eprintln!(
                    "WARN: watch_ignore_reload_failed collection_id={} root={} error={}",
                    collection_id,
                    state.root_path.display(),
                    error
                );
                return Ok(());
            }
        }
    }
    run_watcher_reconcile(conn, collection_id, &native_renames)
}

#[cfg(unix)]
fn mark_watcher_crashed(collection_id: i64, state: &mut CollectionWatcherState) -> Duration {
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

#[cfg(unix)]
fn publish_watcher_health(
    session_id: &str,
    watchers: &HashMap<i64, CollectionWatcherState>,
) -> Result<(), VaultSyncError> {
    with_supervisor_handles(|handles| {
        for (collection_id, handle) in handles.iter_mut() {
            if handle.session_id != session_id {
                continue;
            }
            handle.watcher_health =
                watchers
                    .get(collection_id)
                    .map(|state| WatcherHealthSnapshot {
                        mode: state.mode,
                        last_event_at: state.last_event_at.clone(),
                        channel_depth: state.receiver.len(),
                    });
        }
    })?;
    Ok(())
}

#[cfg(unix)]
fn run_overflow_recovery_pass(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<(i64, String)>, VaultSyncError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, active_lease_session_id
         FROM collections
         WHERE state = 'active' AND needs_full_sync = 1",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut actions = Vec::new();
    for (collection_id, collection_name, active_lease_session_id) in rows {
        let Some(lease_session_id) = active_lease_session_id else {
            eprintln!(
                "WARN: overflow_recovery_skipped_lease_mismatch collection={} collection_id={} expected_session_id={} actual_session_id=null",
                collection_name, collection_id, session_id
            );
            actions.push((collection_id, format!("{collection_name}:lease-mismatch")));
            continue;
        };
        if lease_session_id != session_id {
            eprintln!(
                "WARN: overflow_recovery_skipped_lease_mismatch collection={} collection_id={} expected_session_id={} actual_session_id={}",
                collection_name, collection_id, session_id, lease_session_id
            );
            actions.push((collection_id, format!("{collection_name}:lease-mismatch")));
            continue;
        }
        match full_hash_reconcile_authorized(
            conn,
            collection_id,
            FullHashReconcileMode::OverflowRecovery,
            FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: lease_session_id.clone(),
            },
        ) {
            Ok(_) => {
                conn.execute(
                    "UPDATE collections
                     SET needs_full_sync = 0,
                         last_sync_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                     WHERE id = ?1 AND state = 'active'",
                    [collection_id],
                )?;
                eprintln!(
                    "INFO: overflow_recovery_complete collection={}",
                    collection_name
                );
                actions.push((collection_id, format!("{collection_name}:reconciled")));
            }
            Err(error) => {
                eprintln!(
                    "WARN: overflow_recovery_failed collection={} collection_id={} error={}",
                    collection_name, collection_id, error
                );
                actions.push((collection_id, format!("{collection_name}:failed")));
            }
        }
    }
    Ok(actions)
}

fn reconcile_halt_details(err: &ReconcileError) -> Option<(&'static str, String)> {
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

pub fn ensure_all_collections_write_allowed(conn: &Connection) -> Result<(), VaultSyncError> {
    let mut stmt = conn.prepare(
        "SELECT id FROM collections WHERE state = 'restoring' OR needs_full_sync = 1 LIMIT 1",
    )?;
    let blocked: Option<i64> = stmt.query_row([], |row| row.get(0)).optional()?;
    if let Some(collection_id) = blocked {
        ensure_collection_write_allowed(conn, collection_id)?;
    }
    Ok(())
}

pub fn resolve_slug_for_op(
    conn: &Connection,
    input: &str,
    op_kind: OpKind,
) -> Result<ResolvedSlug, VaultSyncError> {
    match collections::parse_slug(conn, input, op_kind)? {
        SlugResolution::Resolved {
            collection_id,
            collection_name,
            slug,
        } => Ok(ResolvedSlug {
            collection_id,
            collection_name,
            slug,
        }),
        SlugResolution::NotFound { slug } => Err(VaultSyncError::PageNotFound { slug }),
        SlugResolution::Ambiguous { slug, candidates } => Err(VaultSyncError::AmbiguousSlug {
            slug,
            candidates: candidates
                .into_iter()
                .map(|candidate| candidate.full_address)
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

pub fn load_collection_by_id(
    conn: &Connection,
    collection_id: i64,
) -> Result<Collection, VaultSyncError> {
    conn.query_row(
        "SELECT id, name, root_path, state, writable, is_write_target, \
                ignore_patterns, ignore_parse_errors, needs_full_sync, last_sync_at, \
                active_lease_session_id, restore_command_id, restore_lease_session_id, \
                reload_generation, watcher_released_session_id, watcher_released_generation, \
                watcher_released_at, pending_command_heartbeat_at, pending_root_path, \
                pending_restore_manifest, restore_command_pid, restore_command_host, \
                integrity_failed_at, pending_manifest_incomplete_at, reconcile_halted_at, \
                reconcile_halt_reason, created_at, updated_at \
         FROM collections WHERE id = ?1",
        [collection_id],
        |row| {
            let state: String = row.get(3)?;
            Ok(Collection {
                id: row.get(0)?,
                name: row.get(1)?,
                root_path: row.get(2)?,
                state: state.parse().map_err(|_| {
                    rusqlite::Error::InvalidParameterName(format!(
                        "invalid collection state for collection_id={collection_id}: {state}"
                    ))
                })?,
                writable: row.get::<_, i64>(4)? != 0,
                is_write_target: row.get::<_, i64>(5)? != 0,
                ignore_patterns: row.get(6)?,
                ignore_parse_errors: row.get(7)?,
                needs_full_sync: row.get::<_, i64>(8)? != 0,
                last_sync_at: row.get(9)?,
                active_lease_session_id: row.get(10)?,
                restore_command_id: row.get(11)?,
                restore_lease_session_id: row.get(12)?,
                reload_generation: row.get(13)?,
                watcher_released_session_id: row.get(14)?,
                watcher_released_generation: row.get(15)?,
                watcher_released_at: row.get(16)?,
                pending_command_heartbeat_at: row.get(17)?,
                pending_root_path: row.get(18)?,
                pending_restore_manifest: row.get(19)?,
                restore_command_pid: row.get(20)?,
                restore_command_host: row.get(21)?,
                integrity_failed_at: row.get(22)?,
                pending_manifest_incomplete_at: row.get(23)?,
                reconcile_halted_at: row.get(24)?,
                reconcile_halt_reason: row.get(25)?,
                created_at: row.get(26)?,
                updated_at: row.get(27)?,
            })
        },
    )
    .map_err(Into::into)
}

pub fn mark_collection_restoring_for_handshake(
    conn: &Connection,
    collection_id: i64,
) -> Result<(Collection, String, i64), VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    // Must use live_collection_owner (not untyped owner_session_id) so that a live
    // CLI lease in collection_owners is never mistaken for the serve supervisor that
    // must write the ack.  live_collection_owner enforces session_type = 'serve' AND
    // heartbeat liveness in one typed query (design.md §404-408).
    let owner = live_collection_owner(conn, collection_id)?.ok_or_else(|| {
        VaultSyncError::ServeOwnsCollectionError {
            collection_name: collection.name.clone(),
            owner_session_id: "none".to_owned(),
            owner_pid: 0,
            owner_host: "unknown".to_owned(),
        }
    })?;
    let expected_session_id = owner.session_id;

    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             reload_generation = reload_generation + 1,
             watcher_released_session_id = NULL,
             watcher_released_generation = NULL,
             watcher_released_at = NULL,
             pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection_id],
    )?;
    let generation: i64 = conn.query_row(
        "SELECT reload_generation FROM collections WHERE id = ?1",
        [collection_id],
        |row| row.get(0),
    )?;
    Ok((
        load_collection_by_id(conn, collection_id)?,
        expected_session_id,
        generation,
    ))
}

pub fn wait_for_exact_ack(
    conn: &Connection,
    collection_id: i64,
    expected_session_id: &str,
    reload_generation: i64,
) -> Result<(), VaultSyncError> {
    let started = Instant::now();
    loop {
        let collection = load_collection_by_id(conn, collection_id)?;
        // Re-check owner via typed live_collection_owner so that a CLI lease or a
        // stale/non-serve session can never satisfy the ownership invariant
        // mid-handshake (design.md §404-408 do-not-impersonate rule).
        match live_collection_owner(conn, collection_id)? {
            None => {
                return Err(VaultSyncError::ServeDiedDuringHandshake {
                    collection_name: collection.name,
                    expected_session_id: expected_session_id.to_owned(),
                });
            }
            Some(ref owner) if owner.session_id != expected_session_id => {
                return Err(VaultSyncError::ServeOwnershipChanged {
                    collection_name: collection.name,
                    expected_session_id: expected_session_id.to_owned(),
                    actual_session_id: owner.session_id.clone(),
                });
            }
            Some(_) => {}
        }
        if collection.watcher_released_session_id.as_deref() == Some(expected_session_id)
            && collection.watcher_released_generation == Some(reload_generation)
            && collection.watcher_released_at.is_some()
        {
            return Ok(());
        }
        if started.elapsed() >= Duration::from_secs(HANDSHAKE_TIMEOUT_SECS) {
            return Err(VaultSyncError::HandshakeTimeout {
                collection_name: collection.name,
                expected_session_id: expected_session_id.to_owned(),
                reload_generation,
            });
        }
        thread::sleep(Duration::from_millis(HANDSHAKE_POLL_MS));
    }
}

fn configured_full_hash_audit_days() -> i64 {
    std::env::var("QUAID_FULL_HASH_AUDIT_DAYS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_FULL_HASH_AUDIT_DAYS)
        .max(0)
}

fn full_hash_audit_ttl_cutoff() -> String {
    format!("-{} days", configured_full_hash_audit_days())
}

fn scheduled_full_hash_audit_budget(total_files: usize) -> usize {
    if total_files == 0 {
        return 0;
    }
    // Keep the serve loop bounded even when the TTL is configured to "all files are due now".
    let audit_days = usize::try_from(configured_full_hash_audit_days())
        .ok()
        .filter(|days| *days > 0)
        .unwrap_or(DEFAULT_FULL_HASH_AUDIT_DAYS as usize);
    total_files.div_ceil(audit_days).max(1)
}

pub fn run_full_hash_audit_pass(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<(i64, String, ReconcileStats)>, VaultSyncError> {
    let ttl_cutoff = full_hash_audit_ttl_cutoff();
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name
         FROM collections c
         WHERE c.state = 'active'
           AND c.active_lease_session_id = ?1
           AND EXISTS (
               SELECT 1
               FROM file_state fs
               WHERE fs.collection_id = c.id
                 AND (fs.last_full_hash_at IS NULL OR fs.last_full_hash_at < datetime('now', ?2))
           )
         ORDER BY c.id ASC",
    )?;
    let due = stmt
        .query_map(params![session_id, ttl_cutoff], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let mut audited = Vec::new();
    for (collection_id, collection_name) in due {
        let total_files: i64 = conn.query_row(
            "SELECT COUNT(*) FROM file_state WHERE collection_id = ?1",
            [collection_id],
            |row| row.get(0),
        )?;
        let audit_budget = scheduled_full_hash_audit_budget(total_files.max(0) as usize);
        if audit_budget == 0 {
            continue;
        }
        let mut due_stmt = conn.prepare(
            "SELECT relative_path
             FROM file_state
             WHERE collection_id = ?1
               AND (last_full_hash_at IS NULL OR last_full_hash_at < datetime('now', ?2))
             ORDER BY COALESCE(last_full_hash_at, ''), relative_path ASC
             LIMIT ?3",
        )?;
        let due_relative_paths = due_stmt
            .query_map(
                params![collection_id, ttl_cutoff, audit_budget as i64],
                |row| Ok(PathBuf::from(row.get::<_, String>(0)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        drop(due_stmt);
        if due_relative_paths.is_empty() {
            continue;
        }

        let stats = scheduled_full_hash_audit_authorized(
            conn,
            collection_id,
            &due_relative_paths,
            FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: session_id.to_owned(),
            },
        )?;
        conn.execute(
            "UPDATE collections
             SET last_sync_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            [collection_id],
        )?;
        audited.push((collection_id, collection_name, stats));
    }

    Ok(audited)
}

pub fn audit_collection(
    conn: &Connection,
    collection_name: &str,
) -> Result<ReconcileStats, VaultSyncError> {
    let collection = collections::get_by_name(conn, collection_name)?.ok_or_else(|| {
        VaultSyncError::CollectionNotFound {
            name: collection_name.to_owned(),
        }
    })?;
    let stats = full_hash_reconcile_authorized(
        conn,
        collection.id,
        FullHashReconcileMode::Audit,
        FullHashReconcileAuthorization::AuditCommand,
    )?;
    conn.execute(
        "UPDATE collections
         SET last_sync_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection.id],
    )?;
    Ok(stats)
}

pub fn sweep_raw_import_ttl(conn: &Connection) -> Result<usize, VaultSyncError> {
    raw_imports::sweep_expired_inactive_rows(conn).map_err(VaultSyncError::from)
}

pub fn write_supervisor_ack_if_needed(
    conn: &Connection,
    collection_id: i64,
    session_id: &str,
    observed_generation: i64,
) -> Result<bool, VaultSyncError> {
    if owner_session_id(conn, collection_id)?.as_deref() != Some(session_id) {
        return Ok(false);
    }
    let rows = conn.execute(
        "UPDATE collections
         SET watcher_released_session_id = ?2,
             watcher_released_generation = reload_generation,
             watcher_released_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1
           AND state = 'restoring'
           AND reload_generation = ?3
           AND watcher_released_at IS NULL",
        params![collection_id, session_id, observed_generation],
    )?;
    Ok(rows != 0)
}

pub fn build_restore_manifest_for_directory(
    path: &Path,
) -> Result<RestoreManifest, VaultSyncError> {
    let mut entries = Vec::new();
    let walked = walk_tree(path)?;
    for relative_path in walked.keys() {
        let absolute = path.join(relative_path);
        let bytes = fs::read(&absolute)?;
        entries.push(RestoreManifestEntry {
            relative_path: path_string(relative_path),
            sha256: sha256_hex(&bytes),
            size_bytes: bytes.len() as u64,
        });
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(RestoreManifest { entries })
}

pub fn finalize_pending_restore(
    conn: &Connection,
    collection_id: i64,
    caller: FinalizeCaller,
) -> Result<FinalizeOutcome, VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    if collection.integrity_failed_at.is_some() {
        return Ok(FinalizeOutcome::IntegrityFailed);
    }
    let fresh_heartbeat = has_fresh_command_heartbeat(conn, &collection)?;
    let caller_is_originator = matches!(
        caller,
        FinalizeCaller::RestoreOriginator { ref command_id }
            if collection.restore_command_id.as_deref() == Some(command_id.as_str())
    );
    if !caller_is_originator && fresh_heartbeat {
        return Ok(FinalizeOutcome::Deferred);
    }

    if collection.pending_root_path.is_none() {
        if collection.state == CollectionState::Restoring {
            revert_orphan_restore_state(conn, &collection)?;
            return Ok(FinalizeOutcome::OrphanRecovered);
        }
        return Ok(FinalizeOutcome::NoPendingWork);
    }

    let pending_root_path = PathBuf::from(collection.pending_root_path.clone().unwrap());
    if !pending_root_path.exists() {
        revert_aborted_restore(conn, &collection)?;
        return Ok(FinalizeOutcome::Aborted);
    }

    let manifest_json = collection
        .pending_restore_manifest
        .as_deref()
        .ok_or_else(|| VaultSyncError::InvariantViolation {
            message: format!(
                "collection={} missing pending_restore_manifest for pending root",
                collection.name
            ),
        })?;
    let manifest: RestoreManifest = serde_json::from_str(manifest_json)?;
    match compare_manifest(&pending_root_path, &manifest)? {
        ManifestComparison::Matches => {
            clear_manifest_incomplete(conn, collection_id)?;
            run_tx_b(conn, collection_id)?;
            Ok(FinalizeOutcome::Finalized)
        }
        ManifestComparison::MissingFiles => {
            let age = pending_manifest_age_seconds(conn, &collection)?;
            if age >= manifest_incomplete_escalation_secs() {
                conn.execute(
                    "UPDATE collections
                     SET integrity_failed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                     WHERE id = ?1",
                    [collection_id],
                )?;
                Ok(FinalizeOutcome::IntegrityFailed)
            } else {
                conn.execute(
                    "UPDATE collections
                     SET pending_manifest_incomplete_at = COALESCE(
                             pending_manifest_incomplete_at,
                             strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                         ),
                         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                     WHERE id = ?1",
                    [collection_id],
                )?;
                Ok(FinalizeOutcome::ManifestIncomplete)
            }
        }
        ManifestComparison::Mismatch => {
            conn.execute(
                "UPDATE collections
                 SET integrity_failed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE id = ?1",
                [collection_id],
            )?;
            Ok(FinalizeOutcome::IntegrityFailed)
        }
    }
}

pub fn run_tx_b(conn: &Connection, collection_id: i64) -> Result<bool, VaultSyncError> {
    let pending_root_path: Option<String> = conn
        .query_row(
            "SELECT pending_root_path FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(pending_root_path) = pending_root_path else {
        return Ok(false);
    };
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE collections
         SET root_path = ?2,
             state = 'restoring',
             needs_full_sync = 1,
             reload_generation = reload_generation + 1,
             watcher_released_session_id = NULL,
             watcher_released_generation = NULL,
             watcher_released_at = NULL,
             pending_command_heartbeat_at = NULL,
             pending_root_path = NULL,
             pending_restore_manifest = NULL,
             restore_command_id = NULL,
             restore_command_pid = NULL,
             restore_command_host = NULL,
             integrity_failed_at = NULL,
             pending_manifest_incomplete_at = NULL,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1 AND pending_root_path IS NOT NULL",
        params![collection_id, pending_root_path],
    )?;
    tx.commit()?;
    Ok(true)
}

pub fn finalize_pending_restore_via_cli(
    conn: &Connection,
    collection_id: i64,
) -> Result<FinalizeCliOutcome, VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    if collection.reconcile_halted_at.is_some() {
        return Err(VaultSyncError::ReconcileHalted {
            collection_name: collection.name,
            reason: collection
                .reconcile_halt_reason
                .unwrap_or_else(|| "unknown".to_owned()),
        });
    }

    let _lease = start_short_lived_owner_lease(conn, collection_id)?;
    let session_id = owner_session_id(conn, collection_id)?.ok_or_else(|| {
        VaultSyncError::InvariantViolation {
            message: format!(
                "collection_id={} missing owner lease during finalize-pending CLI path",
                collection_id
            ),
        }
    })?;

    let collection = load_collection_by_id(conn, collection_id)?;
    if collection.pending_root_path.is_some() || collection.restore_command_id.is_some() {
        match finalize_pending_restore(
            conn,
            collection_id,
            FinalizeCaller::ExternalFinalize {
                session_id: session_id.clone(),
            },
        )? {
            FinalizeOutcome::Finalized => {
                let _ = complete_attach(
                    conn,
                    collection_id,
                    &session_id,
                    AttachReason::RestorePostFinalize,
                )?;
                return Ok(FinalizeCliOutcome::Attached);
            }
            FinalizeOutcome::OrphanRecovered => return Ok(FinalizeCliOutcome::OrphanRecovered),
            FinalizeOutcome::Deferred => return Ok(FinalizeCliOutcome::Deferred),
            FinalizeOutcome::ManifestIncomplete => {
                return Ok(FinalizeCliOutcome::ManifestIncomplete);
            }
            FinalizeOutcome::IntegrityFailed => return Ok(FinalizeCliOutcome::IntegrityFailed),
            FinalizeOutcome::Aborted => return Ok(FinalizeCliOutcome::Aborted),
            FinalizeOutcome::NoPendingWork => return Ok(FinalizeCliOutcome::NoPendingWork),
        }
    }

    let collection = load_collection_by_id(conn, collection_id)?;
    if collection.needs_full_sync
        && collection.integrity_failed_at.is_none()
        && collection.pending_manifest_incomplete_at.is_none()
        && matches!(
            collection.state,
            CollectionState::Restoring | CollectionState::Active
        )
    {
        let _ = complete_attach(
            conn,
            collection_id,
            &session_id,
            AttachReason::RestorePostFinalize,
        )?;
        return Ok(FinalizeCliOutcome::Attached);
    }

    Ok(FinalizeCliOutcome::NoPendingWork)
}

fn complete_attach(
    conn: &Connection,
    collection_id: i64,
    session_id: &str,
    reason: AttachReason,
) -> Result<bool, VaultSyncError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    if collection.reconcile_halted_at.is_some() {
        return Err(VaultSyncError::ReconcileHalted {
            collection_name: collection.name,
            reason: collection
                .reconcile_halt_reason
                .unwrap_or_else(|| "unknown".to_owned()),
        });
    }
    if !collection.needs_full_sync
        || !matches!(
            collection.state,
            CollectionState::Restoring | CollectionState::Active
        )
    {
        return Ok(false);
    }
    let _recovery_guard = RecoveryInProgressGuard::enter(collection_id)?;
    if let Err(err) = full_hash_reconcile_authorized(
        conn,
        collection_id,
        match reason {
            AttachReason::RestorePostFinalize => FullHashReconcileMode::Restore,
            AttachReason::RemapPostReconcile => FullHashReconcileMode::RemapRoot,
        },
        FullHashReconcileAuthorization::ActiveLease {
            lease_session_id: session_id.to_owned(),
        },
    ) {
        return Err(convert_reconcile_error(
            conn,
            collection_id,
            &collection.name,
            err,
        )?);
    }
    let rows = conn.execute(
        "UPDATE collections
         SET state = 'active',
              needs_full_sync = 0,
              reload_generation = reload_generation + 1,
              last_sync_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
              updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1
           AND needs_full_sync = 1
           AND (state = 'restoring' OR state = 'active')",
        [collection_id],
    )?;
    Ok(rows != 0)
}

pub fn run_rcrt_pass(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<(i64, String)>, VaultSyncError> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name
         FROM collections c
         JOIN collection_owners o ON o.collection_id = c.id
         WHERE o.session_id = ?1 AND c.state = 'restoring'",
    )?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut actions = Vec::new();
    for (collection_id, name) in rows {
        if has_supervisor_handle(collection_id, session_id)? {
            actions.push((collection_id, format!("{name}:supervised")));
            continue;
        }
        let collection = load_collection_by_id(conn, collection_id)?;
        if collection.reconcile_halted_at.is_some() {
            actions.push((collection_id, format!("{name}:skipped-halt")));
            continue;
        }
        if collection.pending_root_path.is_some() || collection.restore_command_id.is_some() {
            match finalize_pending_restore(
                conn,
                collection_id,
                FinalizeCaller::StartupRecovery {
                    session_id: session_id.to_owned(),
                },
            )? {
                FinalizeOutcome::Finalized => {
                    let _ = complete_attach(
                        conn,
                        collection_id,
                        session_id,
                        AttachReason::RestorePostFinalize,
                    )?;
                    register_supervisor_handle(
                        collection_id,
                        session_id,
                        load_collection_by_id(conn, collection_id)?.reload_generation,
                    )?;
                    actions.push((collection_id, format!("{name}:finalized")));
                }
                outcome => actions.push((collection_id, format!("{name}:{outcome:?}"))),
            }
            continue;
        }
        if collection.needs_full_sync
            && collection.integrity_failed_at.is_none()
            && collection.pending_manifest_incomplete_at.is_none()
        {
            let reason = if collection.restore_command_id.is_none()
                && collection.pending_root_path.is_none()
            {
                AttachReason::RemapPostReconcile
            } else {
                AttachReason::RestorePostFinalize
            };
            if complete_attach(conn, collection_id, session_id, reason)? {
                register_supervisor_handle(
                    collection_id,
                    session_id,
                    load_collection_by_id(conn, collection_id)?.reload_generation,
                )?;
                actions.push((collection_id, format!("{name}:attached")));
            }
        }
    }
    Ok(actions)
}

#[cfg(unix)]
fn publish_ipc_socket(
    conn: &Connection,
    session_id: &str,
) -> Result<PublishedIpcSocket, VaultSyncError> {
    let location = ipc_socket_location()?;
    ensure_secure_ipc_directory(&location.runtime_root, location.create_runtime_root)?;
    ensure_secure_ipc_directory(&location.socket_dir, true)?;
    let socket_path = location.socket_dir.join(format!("{session_id}.sock"));
    if socket_path.exists() {
        clear_stale_ipc_socket(&socket_path)?;
    }
    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    listen_with_backlog(&listener)?;
    audit_bound_ipc_socket(&socket_path)?;
    conn.execute(
        "UPDATE serve_sessions SET ipc_path = ?1 WHERE session_id = ?2",
        params![socket_path.display().to_string(), session_id],
    )?;
    Ok(PublishedIpcSocket {
        listener,
        path: socket_path,
    })
}

#[cfg(unix)]
fn ipc_socket_location() -> Result<IpcSocketLocation, VaultSyncError> {
    #[cfg(target_os = "linux")]
    {
        if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
            let runtime_root = PathBuf::from(runtime_dir);
            return Ok(IpcSocketLocation {
                socket_dir: runtime_root.join("quaid"),
                runtime_root,
                create_runtime_root: false,
            });
        }
        dirs::home_dir()
            .map(|home| {
                let runtime_root = home.join(".cache").join("quaid");
                IpcSocketLocation {
                    socket_dir: runtime_root.join("run"),
                    runtime_root,
                    create_runtime_root: true,
                }
            })
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: "unable to resolve HOME for IPC directory".to_owned(),
            })
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .map(|home| {
                let runtime_root = home
                    .join("Library")
                    .join("Application Support")
                    .join("quaid");
                IpcSocketLocation {
                    socket_dir: runtime_root.join("run"),
                    runtime_root,
                    create_runtime_root: true,
                }
            })
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: "unable to resolve HOME for IPC directory".to_owned(),
            })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        dirs::home_dir()
            .map(|home| {
                let runtime_root = home.join(".cache").join("quaid");
                IpcSocketLocation {
                    socket_dir: runtime_root.join("run"),
                    runtime_root,
                    create_runtime_root: true,
                }
            })
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: "unable to resolve HOME for IPC directory".to_owned(),
            })
    }
}

#[cfg(unix)]
fn ensure_secure_ipc_directory(path: &Path, create_if_missing: bool) -> Result<(), VaultSyncError> {
    if !path.exists() {
        if !create_if_missing {
            return Err(VaultSyncError::IpcDirectoryInsecure {
                path: path.display().to_string(),
                reason: "path does not exist".to_owned(),
            });
        }
        fs::create_dir_all(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let metadata = fs::symlink_metadata(path)?;
    let mode = metadata.mode() & 0o777;
    if !metadata.file_type().is_dir() {
        return Err(VaultSyncError::IpcDirectoryInsecure {
            path: path.display().to_string(),
            reason: "path is not a directory".to_owned(),
        });
    }
    if metadata.uid() != current_effective_uid() {
        return Err(VaultSyncError::IpcDirectoryInsecure {
            path: path.display().to_string(),
            reason: format!(
                "owner uid {} does not match current uid {}",
                metadata.uid(),
                current_effective_uid()
            ),
        });
    }
    if mode != 0o700 {
        return Err(VaultSyncError::IpcDirectoryInsecure {
            path: path.display().to_string(),
            reason: format!("mode {:o} is not 700", mode),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn clear_stale_ipc_socket(path: &Path) -> Result<(), VaultSyncError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket() {
        return Err(VaultSyncError::IpcSocketCollision {
            path: path.display().to_string(),
            reason: "existing path is not a unix socket".to_owned(),
        });
    }
    match UnixStream::connect(path) {
        Ok(stream) => {
            let creds = peer_credentials_for_stream(&stream)?;
            return Err(VaultSyncError::IpcSocketCollision {
                path: path.display().to_string(),
                reason: format!("live listener already bound by pid {}", creds.pid),
            });
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::NotFound
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::ConnectionAborted
            ) =>
        {
            fs::remove_file(path)?;
        }
        Err(error) => {
            return Err(VaultSyncError::IpcSocketCollision {
                path: path.display().to_string(),
                reason: error.to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn listen_with_backlog(listener: &UnixListener) -> Result<(), VaultSyncError> {
    let rc = unsafe { libc::listen(listener.as_raw_fd(), 16) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error().into())
    }
}

#[cfg(unix)]
fn audit_bound_ipc_socket(path: &Path) -> Result<(), VaultSyncError> {
    let metadata = fs::symlink_metadata(path)?;
    let mode = metadata.mode() & 0o777;
    if !metadata.file_type().is_socket() {
        return Err(VaultSyncError::IpcSocketPermission {
            path: path.display().to_string(),
            reason: "bound path is not a unix socket".to_owned(),
        });
    }
    if metadata.uid() != current_effective_uid() {
        return Err(VaultSyncError::IpcSocketPermission {
            path: path.display().to_string(),
            reason: format!(
                "owner uid {} does not match current uid {}",
                metadata.uid(),
                current_effective_uid()
            ),
        });
    }
    if mode != 0o600 {
        return Err(VaultSyncError::IpcSocketPermission {
            path: path.display().to_string(),
            reason: format!("mode {:o} is not 600", mode),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn cleanup_published_ipc_socket(
    conn: &Connection,
    session_id: &str,
    socket_path: &Path,
) -> Result<(), VaultSyncError> {
    if socket_path.exists() {
        let _ = fs::remove_file(socket_path);
    }
    conn.execute(
        "UPDATE serve_sessions SET ipc_path = NULL WHERE session_id = ?1",
        [session_id],
    )?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn session_id_from_ipc_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
}

#[cfg(unix)]
pub(crate) fn peer_credentials_for_stream(
    stream: &UnixStream,
) -> Result<IpcPeerCredentials, VaultSyncError> {
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "linux")]
    {
        let mut creds: libc::ucred = unsafe { zeroed() };
        let mut len = size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut creds as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(IpcPeerCredentials {
            pid: creds.pid,
            uid: creds.uid,
        })
    }
    #[cfg(target_os = "macos")]
    {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        let rc = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut pid: libc::pid_t = 0;
        let mut len = size_of::<libc::pid_t>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                0,
                libc::LOCAL_PEERPID,
                (&mut pid as *mut libc::pid_t).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(IpcPeerCredentials {
            pid,
            uid: uid as u32,
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(VaultSyncError::InvariantViolation {
            message: "peer credentials unsupported on this unix platform".to_owned(),
        })
    }
}

#[cfg(unix)]
pub(crate) fn authorize_server_peer(
    socket_path: &Path,
    peer: &IpcPeerCredentials,
) -> Result<(), VaultSyncError> {
    if peer.uid != current_effective_uid() {
        return Err(VaultSyncError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "peer uid {} does not match current uid {}",
                peer.uid,
                current_effective_uid()
            ),
        });
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn authorize_client_peer(
    socket_path: &Path,
    path_session_id: &str,
    owner_session_id: &str,
    owner_pid: i64,
    peer: &IpcPeerCredentials,
    whoami_session_id: &str,
) -> Result<(), VaultSyncError> {
    if path_session_id != owner_session_id {
        return Err(VaultSyncError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "path session {} does not match owner session {}",
                path_session_id, owner_session_id
            ),
        });
    }
    if peer.uid != current_effective_uid() {
        return Err(VaultSyncError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "peer uid {} does not match current uid {}",
                peer.uid,
                current_effective_uid()
            ),
        });
    }
    if i64::from(peer.pid) != owner_pid {
        return Err(VaultSyncError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "peer pid {} does not match owner pid {}",
                peer.pid, owner_pid
            ),
        });
    }
    if whoami_session_id != path_session_id {
        return Err(VaultSyncError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "whoami session {} does not match path session {}",
                whoami_session_id, path_session_id
            ),
        });
    }
    Ok(())
}

/// Maximum concurrent in-flight IPC handler threads.  Connections that arrive
/// when this cap is reached are immediately closed so a rogue same-UID caller
/// cannot exhaust OS thread resources and impact serve liveness.
#[cfg(unix)]
const IPC_HANDLER_LIMIT: usize = 8;

/// RAII guard: decrements the in-flight counter when dropped so the slot is
/// always released even if the handler returns early or panics.
#[cfg(unix)]
struct IpcHandlerGuard(Arc<AtomicUsize>);

#[cfg(unix)]
impl Drop for IpcHandlerGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(unix)]
fn accept_ipc_clients(
    listener: &UnixListener,
    socket_path: &Path,
    db_path: &str,
    session_id: &str,
    in_flight: &Arc<AtomicUsize>,
) {
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => {
                // Enforce the in-flight cap before spawning.  fetch_add is atomic
                // so concurrent accept calls on the same listener are race-free.
                if in_flight.fetch_add(1, Ordering::AcqRel) >= IPC_HANDLER_LIMIT {
                    // Already at or over the cap: roll back and discard the stream.
                    in_flight.fetch_sub(1, Ordering::AcqRel);
                    eprintln!(
                        "WARN: ipc_handler_limit_reached path={} limit={} connection_closed",
                        socket_path.display(),
                        IPC_HANDLER_LIMIT,
                    );
                    // `stream` is dropped here, closing the connection immediately.
                    // Break (not continue) so a same-UID flood at saturation cannot
                    // drain the kernel accept queue indefinitely and starve heartbeats
                    // and watchers in the main serve loop.  At most one connection is
                    // discarded per tick before we return control to the serve loop.
                    break;
                }
                // Offload each client to its own thread so that blocking reads/writes
                // (up to 5 s per IPC timeout) cannot stall the main serve loop and
                // cause a false-dead live-owner verdict.
                // `guard` is moved into the closure and drops when the thread exits,
                // decrementing the in-flight counter even on panic or early return.
                let guard = IpcHandlerGuard(Arc::clone(in_flight));
                let socket_path_owned = socket_path.to_path_buf();
                let db_path_owned = db_path.to_owned();
                let session_id_owned = session_id.to_owned();
                thread::spawn(move || {
                    let _guard = guard;
                    if let Err(error) = handle_ipc_client(
                        stream,
                        &socket_path_owned,
                        &db_path_owned,
                        &session_id_owned,
                    ) {
                        eprintln!(
                            "WARN: ipc_client_failed path={} error={}",
                            socket_path_owned.display(),
                            error
                        );
                    }
                });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) => {
                eprintln!(
                    "WARN: ipc_accept_failed path={} error={}",
                    socket_path.display(),
                    error
                );
                break;
            }
        }
    }
}

#[cfg(unix)]
fn handle_ipc_client(
    stream: UnixStream,
    socket_path: &Path,
    db_path: &str,
    session_id: &str,
) -> Result<(), VaultSyncError> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let peer = peer_credentials_for_stream(&stream)?;
    authorize_server_peer(socket_path, &peer)?;
    eprintln!(
        "INFO: ipc_peer_authenticated session_id={} peer_pid={} peer_uid={}",
        session_id, peer.pid, peer.uid
    );

    let read_stream = stream.try_clone()?;
    let mut reader = BufReader::new(read_stream);
    let mut writer = BufWriter::new(stream);
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }
        let request = match serde_json::from_str::<IpcRequest>(line.trim_end()) {
            Ok(request) => request,
            Err(error) => {
                write_ipc_response(
                    &mut writer,
                    &IpcResponse::Error {
                        error: format!("invalid ipc request: {error}"),
                    },
                )?;
                break;
            }
        };
        match request {
            IpcRequest::WhoAmI => {
                write_ipc_response(
                    &mut writer,
                    &IpcResponse::WhoAmI {
                        session_id: session_id.to_owned(),
                    },
                )?;
            }
            IpcRequest::Put {
                slug,
                content,
                expected_version,
            } => {
                let conn = Connection::open(db_path)?;
                match put::put_from_string_status(&conn, &slug, &content, expected_version) {
                    Ok(status) => {
                        write_ipc_response(&mut writer, &IpcResponse::PutOk { status })?;
                    }
                    Err(error) => {
                        write_ipc_response(
                            &mut writer,
                            &IpcResponse::Error {
                                error: error.to_string(),
                            },
                        )?;
                    }
                }
                break;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn write_ipc_response(
    writer: &mut BufWriter<UnixStream>,
    response: &IpcResponse,
) -> Result<(), VaultSyncError> {
    serde_json::to_writer(&mut *writer, response).map_err(|error| {
        VaultSyncError::InvariantViolation {
            message: format!("failed to serialize ipc response: {error}"),
        }
    })?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub fn start_serve_runtime(db_path: String) -> Result<ServeRuntime, VaultSyncError> {
    init_process_registries()?;
    let conn = Connection::open(&db_path)?;
    sweep_stale_sessions(&conn)?;
    let session_id = register_session(&conn)?;
    #[cfg(unix)]
    let published_ipc = match publish_ipc_socket(&conn, &session_id) {
        Ok(published) => published,
        Err(error) => {
            let _ = unregister_session(&conn, &session_id);
            return Err(error);
        }
    };
    if let Err(error) = run_startup_sequence(&conn, Path::new(&db_path), &session_id) {
        #[cfg(unix)]
        let _ = cleanup_published_ipc_socket(&conn, &session_id, &published_ipc.path);
        let _ = unregister_session(&conn, &session_id);
        return Err(error);
    }
    #[cfg(unix)]
    let mut watchers: HashMap<i64, CollectionWatcherState> = HashMap::new();
    #[cfg(unix)]
    if let Err(error) = sync_collection_watchers(&conn, &db_path, &mut watchers) {
        let _ = cleanup_published_ipc_socket(&conn, &session_id, &published_ipc.path);
        let _ = unregister_session(&conn, &session_id);
        return Err(error);
    }
    let mut stmt = conn.prepare("SELECT id, reload_generation FROM collections")?;
    let initial_generations = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
        .collect::<Result<HashMap<_, _>, _>>()?;
    drop(stmt);
    drop(conn);

    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = Arc::clone(&stop);
    let session_id_for_thread = session_id.clone();
    #[cfg(unix)]
    let ipc_in_flight = Arc::new(AtomicUsize::new(0));
    let handle = thread::spawn(move || {
        #[cfg(unix)]
        let published_ipc = published_ipc;
        #[cfg(unix)]
        let ipc_in_flight = ipc_in_flight;
        let mut last_heartbeat = Instant::now();
        let mut last_quarantine_sweep = Instant::now();
        let mut last_generations = initial_generations;
        #[cfg(unix)]
        let mut watchers = watchers;
        #[cfg(unix)]
        let mut last_dedup_sweep = Instant::now();
        #[cfg(unix)]
        let mut last_overflow_recovery = Instant::now();
        let mut last_full_hash_audit =
            Instant::now() - Duration::from_secs(FULL_HASH_AUDIT_SWEEP_INTERVAL_SECS);
        let mut last_raw_import_ttl_sweep =
            Instant::now() - Duration::from_secs(RAW_IMPORT_TTL_SWEEP_INTERVAL_SECS);
        let mut last_embedding_drain =
            Instant::now() - Duration::from_secs(embedding_drain_interval_secs());
        while !stop_signal.load(Ordering::SeqCst) {
            if let Ok(conn) = Connection::open(&db_path) {
                if last_heartbeat.elapsed() >= Duration::from_secs(HEARTBEAT_INTERVAL_SECS) {
                    let _ = sweep_stale_sessions(&conn);
                    let _ = heartbeat_session(&conn, &session_id_for_thread);
                    last_heartbeat = Instant::now();
                }
                if last_quarantine_sweep.elapsed()
                    >= Duration::from_secs(QUARANTINE_SWEEP_INTERVAL_SECS)
                {
                    let _ = quarantine::sweep_expired_quarantined_pages(&conn);
                    last_quarantine_sweep = Instant::now();
                }
                #[cfg(unix)]
                {
                    accept_ipc_clients(
                        &published_ipc.listener,
                        &published_ipc.path,
                        &db_path,
                        &session_id_for_thread,
                        &ipc_in_flight,
                    );
                    let _ = sync_collection_watchers(&conn, &db_path, &mut watchers);
                    for (collection_id, state) in &mut watchers {
                        if let Err(error) = poll_collection_watcher(&conn, *collection_id, state) {
                            if matches!(error, VaultSyncError::InvariantViolation { .. }) {
                                let _ = mark_watcher_crashed(*collection_id, state);
                            } else {
                                eprintln!(
                                    "WARN: watcher_poll_failed collection_id={} error={}",
                                    collection_id, error
                                );
                            }
                        }
                    }
                    if last_dedup_sweep.elapsed() >= self_write_dedup_sweep_interval() {
                        let _ = sweep_expired_self_write_entries_at(Instant::now());
                        last_dedup_sweep = Instant::now();
                    }
                }
                if let Ok(mut stmt) = conn.prepare("SELECT id, reload_generation FROM collections")
                {
                    if let Ok(rows) = stmt
                        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
                        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
                    {
                        for (collection_id, generation) in rows {
                            let _ =
                                acquire_owner_lease(&conn, collection_id, &session_id_for_thread);
                            let supervisor_generation = with_supervisor_handles(|handles| {
                                handles.get(&collection_id).and_then(|handle| {
                                    (handle.session_id == session_id_for_thread)
                                        .then_some(handle.generation)
                                })
                            })
                            .ok()
                            .flatten();
                            let previous_generation = last_generations
                                .get(&collection_id)
                                .copied()
                                .unwrap_or(generation);
                            if generation > previous_generation
                                && supervisor_generation
                                    .map(|tracked_generation| tracked_generation < generation)
                                    .unwrap_or(false)
                            {
                                let _ = write_supervisor_ack_if_needed(
                                    &conn,
                                    collection_id,
                                    &session_id_for_thread,
                                    generation,
                                );
                                let _ =
                                    remove_supervisor_handle(collection_id, &session_id_for_thread);
                            }
                            last_generations.insert(collection_id, generation);
                        }
                    }
                }
                if last_full_hash_audit.elapsed()
                    >= Duration::from_secs(FULL_HASH_AUDIT_SWEEP_INTERVAL_SECS)
                {
                    if let Err(error) = run_full_hash_audit_pass(&conn, &session_id_for_thread) {
                        eprintln!(
                            "WARN: scheduled_full_hash_audit_failed session_id={} error={}",
                            session_id_for_thread, error
                        );
                    }
                    last_full_hash_audit = Instant::now();
                }
                if last_raw_import_ttl_sweep.elapsed()
                    >= Duration::from_secs(RAW_IMPORT_TTL_SWEEP_INTERVAL_SECS)
                {
                    if let Err(error) = sweep_raw_import_ttl(&conn) {
                        eprintln!(
                            "WARN: raw_import_ttl_sweep_failed session_id={} error={}",
                            session_id_for_thread, error
                        );
                    }
                    last_raw_import_ttl_sweep = Instant::now();
                }
                let _ = run_rcrt_pass(&conn, &session_id_for_thread);
                let _ = sync_supervisor_handles(&conn, &session_id_for_thread);
                if last_embedding_drain.elapsed()
                    >= Duration::from_secs(embedding_drain_interval_secs())
                {
                    let _ = drain_embedding_queue(&conn);
                    last_embedding_drain = Instant::now();
                }
                #[cfg(unix)]
                {
                    let _ = publish_watcher_health(&session_id_for_thread, &watchers);
                    if last_overflow_recovery.elapsed() >= Duration::from_millis(500) {
                        let _ = run_overflow_recovery_pass(&conn, &session_id_for_thread);
                        last_overflow_recovery = Instant::now();
                    }
                }
            }
            thread::sleep(Duration::from_millis(DEFERRED_RETRY_SECS * 200));
        }
        if let Ok(conn) = Connection::open(&db_path) {
            #[cfg(unix)]
            let _ =
                cleanup_published_ipc_socket(&conn, &session_id_for_thread, &published_ipc.path);
            let _ = clear_supervisor_handles_for_session(&session_id_for_thread);
            let _ = unregister_session(&conn, &session_id_for_thread);
        }
    });

    Ok(ServeRuntime {
        stop,
        handle: Some(handle),
        session_id,
    })
}

#[derive(Debug, Clone)]
struct EmbeddingJobClaim {
    id: i64,
    page_id: i64,
    attempt_count: i64,
}

fn embedding_drain_interval_secs() -> u64 {
    2
}

fn configured_embedding_concurrency() -> usize {
    std::env::var("QUAID_EMBEDDING_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .min(4)
        })
}

fn embedding_backoff_secs(attempt_count: i64) -> i64 {
    match attempt_count {
        count if count <= 0 => 0,
        count => 1_i64 << ((count - 1).min(4) as u32),
    }
}

/// (id, page_id, job_state, attempt_count, last_attempt_epoch)
type EmbeddingJobRow = (i64, i64, String, i64, i64);

fn load_embedding_job_candidates(
    conn: &Connection,
) -> Result<Vec<EmbeddingJobRow>, VaultSyncError> {
    let mut stmt = conn.prepare(
        "SELECT id,
                page_id,
                job_state,
                attempt_count,
                COALESCE(
                    CAST(strftime('%s', started_at) AS INTEGER),
                    CAST(strftime('%s', enqueued_at) AS INTEGER),
                    0
                ) AS last_attempt_epoch
         FROM embedding_jobs
         WHERE job_state IN ('pending', 'failed')
           AND attempt_count < 5
         ORDER BY priority DESC, enqueued_at ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn claim_embedding_jobs(conn: &Connection) -> Result<Vec<EmbeddingJobClaim>, VaultSyncError> {
    let now_epoch = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let limit = configured_embedding_concurrency();
    let candidates = load_embedding_job_candidates(conn)?;
    let ready = candidates
        .into_iter()
        .filter(|(_, _, state, attempt_count, last_attempt_epoch)| {
            state == "pending"
                || now_epoch - *last_attempt_epoch >= embedding_backoff_secs(*attempt_count)
        })
        .take(limit)
        .collect::<Vec<_>>();

    if ready.is_empty() {
        return Ok(Vec::new());
    }

    let tx = conn.unchecked_transaction()?;
    let mut claimed = Vec::new();
    for (id, page_id, _state, _attempt_count, _last_attempt_epoch) in ready {
        let updated = tx.execute(
            "UPDATE embedding_jobs
             SET job_state = 'running',
                 started_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                 attempt_count = attempt_count + 1,
                 last_error = NULL
             WHERE id = ?1
               AND job_state IN ('pending', 'failed')
               AND attempt_count < 5",
            [id],
        )?;
        if updated == 1 {
            let attempt_count = tx.query_row(
                "SELECT attempt_count FROM embedding_jobs WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?;
            claimed.push(EmbeddingJobClaim {
                id,
                page_id,
                attempt_count,
            });
        }
    }
    tx.commit()?;
    Ok(claimed)
}

fn load_page_for_embedding_job(
    conn: &Connection,
    page_id: i64,
) -> Result<Option<crate::core::types::Page>, VaultSyncError> {
    let row = conn
        .query_row(
            "SELECT collection_id, slug FROM pages WHERE id = ?1",
            [page_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((collection_id, slug)) = row else {
        return Ok(None);
    };
    get_page_by_key(conn, collection_id, &slug)
        .map(Some)
        .map_err(|error| VaultSyncError::InvariantViolation {
            message: error.to_string(),
        })
}

fn process_embedding_job_on_connection(
    conn: &Connection,
    job_id: i64,
    page_id: i64,
) -> Result<(), VaultSyncError> {
    let Some(page) = load_page_for_embedding_job(conn, page_id)? else {
        conn.execute("DELETE FROM embedding_jobs WHERE id = ?1", [job_id])?;
        return Ok(());
    };

    crate::core::inference::refresh_page_embeddings(conn, page_id, &page).map_err(|err| {
        VaultSyncError::InvariantViolation {
            message: format!("embedding refresh failed for page_id={page_id}: {err}"),
        }
    })?;
    conn.execute("DELETE FROM embedding_jobs WHERE id = ?1", [job_id])?;
    Ok(())
}

fn mark_embedding_job_failed(
    conn: &Connection,
    job_id: i64,
    error: &VaultSyncError,
) -> Result<(), VaultSyncError> {
    conn.execute(
        "UPDATE embedding_jobs
         SET job_state = 'failed',
             last_error = ?2
         WHERE id = ?1",
        params![job_id, error.to_string()],
    )?;
    Ok(())
}

pub fn drain_embedding_queue(conn: &Connection) -> Result<(), VaultSyncError> {
    let claimed = claim_embedding_jobs(conn)?;
    if claimed.is_empty() {
        return Ok(());
    }

    let db_path = database_path(conn).unwrap_or_default();
    if db_path.is_empty() || db_path == ":memory:" || claimed.len() == 1 {
        for job in claimed {
            if let Err(error) = process_embedding_job_on_connection(conn, job.id, job.page_id) {
                mark_embedding_job_failed(conn, job.id, &error)?;
                if job.attempt_count >= 5 {
                    eprintln!(
                        "WARN: embedding_job_failed_permanently job_id={} page_id={} error={}",
                        job.id, job.page_id, error
                    );
                }
            }
        }
        return Ok(());
    }

    let mut handles = Vec::new();
    for job in claimed {
        let path = db_path.clone();
        handles.push(thread::spawn(move || {
            let conn = crate::core::db::open(&path).map_err(|error| {
                VaultSyncError::InvariantViolation {
                    message: format!("embedding worker failed to open database: {error}"),
                }
            })?;
            let result = process_embedding_job_on_connection(&conn, job.id, job.page_id);
            if let Err(ref error) = result {
                let _ = mark_embedding_job_failed(&conn, job.id, error);
            }
            result.map(|_| (job.id, job.page_id, job.attempt_count))
        }));
    }

    for handle in handles {
        match handle.join() {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                eprintln!("WARN: embedding_job_failed error={error}");
            }
            Err(_) => {
                eprintln!("WARN: embedding_worker_thread_panicked");
            }
        }
    }

    Ok(())
}

pub fn begin_restore(
    conn: &Connection,
    collection_name: &str,
    target_path: &Path,
    online: bool,
) -> Result<String, VaultSyncError> {
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

    if online {
        let (_, expected_session_id, generation) =
            mark_collection_restoring_for_handshake(conn, collection.id)?;
        wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?;
        #[cfg(unix)]
        {
            let request = RestoreRemapSafetyRequest {
                collection_id: collection.id,
                db_path: &db_path,
                recovery_root: &recovery_root,
                operation: RestoreRemapOperation::Restore,
                authorization: FullHashReconcileAuthorization::RestoreLease {
                    lease_session_id: expected_session_id.clone(),
                },
                allow_finalize_pending: false,
                stability_max_iters: 0,
            };
            if let Err(err) = run_restore_remap_safety_pipeline_without_mount_check(conn, &request)
            {
                return Err(convert_reconcile_error(
                    conn,
                    collection.id,
                    &collection.name,
                    err,
                )?);
            }
        }
        let command_id = Uuid::now_v7().to_string();
        let staging_path = staging_path_for_target(target_path);
        if staging_path.exists() {
            let _ = fs::remove_dir_all(&staging_path);
        }
        materialize_collection_to_path(conn, &collection, &staging_path)?;
        let manifest = build_restore_manifest_for_directory(&staging_path)?;
        let manifest_json = serde_json::to_string(&manifest)?;
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
                collection.id,
                target_path.display().to_string(),
                manifest_json,
                command_id,
                std::process::id() as i64,
                current_host(),
                expected_session_id
            ],
        )?;
        remove_empty_target_then_rename(&staging_path, target_path)?;
        let _ = finalize_pending_restore(
            conn,
            collection.id,
            FinalizeCaller::RestoreOriginator {
                command_id: command_id.clone(),
            },
        )?;
        Ok(command_id)
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
        #[cfg(unix)]
        {
            let request = RestoreRemapSafetyRequest {
                collection_id: collection.id,
                db_path: &db_path,
                recovery_root: &recovery_root,
                operation: RestoreRemapOperation::Restore,
                authorization: FullHashReconcileAuthorization::RestoreLease {
                    lease_session_id: lease.session_id.clone(),
                },
                allow_finalize_pending: false,
                stability_max_iters: 0,
            };
            if let Err(err) = run_restore_remap_safety_pipeline_without_mount_check(conn, &request)
            {
                return Err(convert_reconcile_error(
                    conn,
                    collection.id,
                    &collection.name,
                    err,
                )?);
            }
        }
        let command_id = Uuid::now_v7().to_string();
        let staging_path = staging_path_for_target(target_path);
        if staging_path.exists() {
            let _ = fs::remove_dir_all(&staging_path);
        }
        materialize_collection_to_path(conn, &collection, &staging_path)?;
        let manifest = build_restore_manifest_for_directory(&staging_path)?;
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
                collection.id,
                target_path.display().to_string(),
                serde_json::to_string(&manifest)?,
                command_id.clone(),
                std::process::id() as i64,
                current_host()
            ],
        )?;
        remove_empty_target_then_rename(&staging_path, target_path)?;
        match finalize_pending_restore(
            conn,
            collection.id,
            FinalizeCaller::RestoreOriginator {
                command_id: command_id.clone(),
            },
        )? {
            FinalizeOutcome::Finalized => {}
            outcome => {
                return Err(VaultSyncError::RestoreCommandBlocked {
                    collection_name: collection.name,
                    outcome: finalize_outcome_label(&outcome),
                })
            }
        }
        let attached = complete_attach(
            conn,
            collection.id,
            &lease.session_id,
            AttachReason::RestorePostFinalize,
        )?;
        if !attached {
            return Err(VaultSyncError::InvariantViolation {
                message: format!(
                    "collection={} restore offline path did not complete attach",
                    collection.name
                ),
            });
        }
        Ok(command_id)
    }
}

pub fn remap_collection(
    conn: &Connection,
    collection_name: &str,
    new_root: &Path,
    online: bool,
) -> Result<RemapVerificationSummary, VaultSyncError> {
    let collection = collections::get_by_name(conn, collection_name)?.ok_or_else(|| {
        VaultSyncError::CollectionNotFound {
            name: collection_name.to_owned(),
        }
    })?;
    ensure_restore_not_blocked(&collection)?;
    let db_path = PathBuf::from(database_path(conn)?);
    let recovery_root = recovery_root_for_db_path(&db_path);
    bootstrap_recovery_directories(conn, &recovery_root)?;

    if online {
        let (_, expected_session_id, generation) =
            mark_collection_restoring_for_handshake(conn, collection.id)?;
        wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?;
        let request = RestoreRemapSafetyRequest {
            collection_id: collection.id,
            db_path: &db_path,
            recovery_root: &recovery_root,
            operation: RestoreRemapOperation::Remap,
            authorization: FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: expected_session_id.clone(),
            },
            allow_finalize_pending: false,
            stability_max_iters: 0,
        };
        if let Err(err) = run_restore_remap_safety_pipeline_without_mount_check(conn, &request) {
            return Err(convert_reconcile_error(
                conn,
                collection.id,
                &collection.name,
                err,
            )?);
        }
        let summary = verify_remap_root(conn, &collection, new_root)?;
        conn.execute(
            "UPDATE collections
             SET root_path = ?2,
                 state = 'restoring',
                 reload_generation = reload_generation + 1,
                 watcher_released_session_id = NULL,
                 watcher_released_generation = NULL,
                 watcher_released_at = NULL,
                 pending_command_heartbeat_at = NULL,
                 needs_full_sync = 1,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![collection.id, new_root.display().to_string()],
        )?;
        conn.execute(
            "DELETE FROM file_state WHERE collection_id = ?1",
            [collection.id],
        )?;
        Ok(summary)
    } else {
        let lease = start_short_lived_owner_lease(conn, collection.id)?;
        let request = RestoreRemapSafetyRequest {
            collection_id: collection.id,
            db_path: &db_path,
            recovery_root: &recovery_root,
            operation: RestoreRemapOperation::Remap,
            authorization: FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: lease.session_id.clone(),
            },
            allow_finalize_pending: false,
            stability_max_iters: 0,
        };
        if let Err(err) = run_restore_remap_safety_pipeline_without_mount_check(conn, &request) {
            return Err(convert_reconcile_error(
                conn,
                collection.id,
                &collection.name,
                err,
            )?);
        }
        let summary = verify_remap_root(conn, &collection, new_root)?;
        conn.execute(
            "UPDATE collections
             SET root_path = ?2,
                  state = 'restoring',
                  needs_full_sync = 1,
                  restore_lease_session_id = NULL,
                  updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![collection.id, new_root.display().to_string()],
        )?;
        conn.execute(
            "DELETE FROM file_state WHERE collection_id = ?1",
            [collection.id],
        )?;
        let attached = complete_attach(
            conn,
            collection.id,
            &lease.session_id,
            AttachReason::RemapPostReconcile,
        )?;
        if !attached {
            return Err(VaultSyncError::InvariantViolation {
                message: format!(
                    "collection={} remap offline path did not complete attach",
                    collection.name
                ),
            });
        }
        Ok(summary)
    }
}

pub fn verify_remap_root(
    conn: &Connection,
    collection: &Collection,
    new_root: &Path,
) -> Result<RemapVerificationSummary, VaultSyncError> {
    let before = take_tree_fence(new_root)?;
    let page_rows = load_remap_page_rows(conn, collection.id)?;
    let file_rows = load_new_root_files(new_root)?;
    let page_matches = resolve_page_matches(&page_rows, &file_rows);
    let after = take_tree_fence(new_root)?;
    if before != after {
        return Err(VaultSyncError::NewRootUnstable {
            collection_name: collection.name.clone(),
        });
    }
    let missing = page_matches.missing_count;
    let mismatched = page_matches.mismatched_pages;
    let extra = page_matches.extra_count;
    if missing != 0 || mismatched != 0 || extra != 0 {
        return Err(VaultSyncError::NewRootVerificationFailed {
            collection_name: collection.name.clone(),
            missing,
            mismatched,
            extra,
            missing_samples: format_diff_samples(&page_matches.missing_pages),
            mismatched_samples: format_diff_samples(&page_matches.mismatched_samples),
            extra_samples: format_diff_samples(&page_matches.extra_files),
        });
    }
    Ok(RemapVerificationSummary {
        resolved_pages: page_matches.resolved_page_ids.len(),
        missing_pages: missing,
        mismatched_pages: mismatched,
        extra_files: extra,
    })
}

pub fn restore_reset(conn: &Connection, collection_name: &str) -> Result<(), VaultSyncError> {
    let collection = collections::get_by_name(conn, collection_name)?.ok_or_else(|| {
        VaultSyncError::CollectionNotFound {
            name: collection_name.to_owned(),
        }
    })?;
    if collection.integrity_failed_at.is_none() {
        let reason = if collection.pending_manifest_incomplete_at.is_some() {
            "manifest_incomplete_retryable"
        } else if collection.pending_root_path.is_some() {
            "pending_finalize"
        } else if collection.state == CollectionState::Restoring || collection.needs_full_sync {
            "restore_in_progress"
        } else {
            "no_integrity_failure"
        };
        return Err(VaultSyncError::RestoreResetBlocked {
            collection_name: collection.name,
            reason,
        });
    }
    conn.execute(
        "UPDATE collections
         SET state = CASE WHEN root_path = '' THEN 'detached' ELSE 'active' END,
              pending_root_path = NULL,
              pending_restore_manifest = NULL,
              restore_command_id = NULL,
              restore_command_pid = NULL,
              restore_command_host = NULL,
              restore_lease_session_id = NULL,
              pending_command_heartbeat_at = NULL,
              watcher_released_session_id = NULL,
              watcher_released_generation = NULL,
              watcher_released_at = NULL,
              integrity_failed_at = NULL,
             pending_manifest_incomplete_at = NULL,
             needs_full_sync = 0,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection.id],
    )?;
    Ok(())
}

pub fn reconcile_reset(conn: &Connection, collection_name: &str) -> Result<(), VaultSyncError> {
    let collection = collections::get_by_name(conn, collection_name)?.ok_or_else(|| {
        VaultSyncError::CollectionNotFound {
            name: collection_name.to_owned(),
        }
    })?;
    conn.execute(
        "UPDATE collections
         SET reconcile_halted_at = NULL,
             reconcile_halt_reason = NULL,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection.id],
    )?;
    Ok(())
}

pub fn fresh_attach_collection(
    conn: &Connection,
    collection_id: i64,
    attach_command_id: &str,
) -> Result<ReconcileStats, VaultSyncError> {
    let _lease = start_short_lived_owner_lease(conn, collection_id)?;
    fresh_attach_reconcile_and_activate(conn, collection_id, attach_command_id)
        .map_err(VaultSyncError::from)
}

pub(crate) struct ShortLivedLease {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    _guard: LeaseGuard,
    #[allow(dead_code)]
    session_id: String,
}

impl Drop for ShortLivedLease {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct LeaseGuard {
    db_path: Option<String>,
    session_id: String,
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if let Some(db_path) = self.db_path.as_deref() {
            if let Ok(conn) = Connection::open(db_path) {
                let _ = unregister_session(&conn, &self.session_id);
            }
        }
    }
}

pub(crate) fn start_short_lived_owner_lease(
    conn: &Connection,
    collection_id: i64,
) -> Result<ShortLivedLease, VaultSyncError> {
    start_short_lived_owner_lease_with_interval(
        conn,
        collection_id,
        Duration::from_secs(HEARTBEAT_INTERVAL_SECS),
    )
}

pub(crate) fn start_short_lived_owner_lease_for_root_path(
    conn: &Connection,
    root_path: &str,
) -> Result<ShortLivedLease, VaultSyncError> {
    let collection_ids = collection_ids_for_root_path(conn, root_path)?;
    if collection_ids.is_empty() {
        return Err(VaultSyncError::InvariantViolation {
            message: format!(
                "root_path={} missing collection rows for short-lived owner lease",
                root_path
            ),
        });
    }
    start_short_lived_owner_leases_with_interval(
        conn,
        &collection_ids,
        Duration::from_secs(HEARTBEAT_INTERVAL_SECS),
    )
}

fn start_short_lived_owner_lease_with_interval(
    conn: &Connection,
    collection_id: i64,
    heartbeat_interval: Duration,
) -> Result<ShortLivedLease, VaultSyncError> {
    start_short_lived_owner_leases_with_interval(conn, &[collection_id], heartbeat_interval)
}

fn start_short_lived_owner_leases_with_interval(
    conn: &Connection,
    collection_ids: &[i64],
    heartbeat_interval: Duration,
) -> Result<ShortLivedLease, VaultSyncError> {
    let db_path = database_path(conn)?;
    let session_id = register_cli_session(conn)?;
    for collection_id in collection_ids {
        if let Err(err) = acquire_owner_lease(conn, *collection_id, &session_id) {
            let _ = unregister_session(conn, &session_id);
            return Err(err);
        }
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = Arc::clone(&stop);
    let db_path_for_thread = db_path.clone();
    let session_id_for_thread = session_id.clone();
    let handle = thread::spawn(move || {
        while !stop_signal.load(Ordering::SeqCst) {
            thread::sleep(heartbeat_interval);
            if stop_signal.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(conn) = Connection::open(&db_path_for_thread) {
                let _ = heartbeat_session(&conn, &session_id_for_thread);
            }
        }
    });

    Ok(ShortLivedLease {
        stop,
        handle: Some(handle),
        _guard: LeaseGuard {
            db_path: Some(db_path),
            session_id: session_id.clone(),
        },
        session_id,
    })
}

fn ensure_restore_not_blocked(collection: &Collection) -> Result<(), VaultSyncError> {
    if collection.state == CollectionState::Restoring {
        if let Some(pending_root_path) = collection.pending_root_path.clone() {
            if collection.integrity_failed_at.is_some() {
                return Err(VaultSyncError::RestoreIntegrityBlocked {
                    collection_name: collection.name.clone(),
                    blocking_column: "integrity_failed_at",
                });
            }
            if collection.pending_manifest_incomplete_at.is_some() {
                return Err(VaultSyncError::RestoreIntegrityBlocked {
                    collection_name: collection.name.clone(),
                    blocking_column: "pending_manifest_incomplete_at",
                });
            }
            return Err(VaultSyncError::RestorePendingFinalize {
                collection_name: collection.name.clone(),
                pending_root_path,
            });
        }
        return Err(VaultSyncError::RestoreInProgress {
            collection_name: collection.name.clone(),
        });
    }
    if collection.integrity_failed_at.is_some() {
        return Err(VaultSyncError::RestoreIntegrityBlocked {
            collection_name: collection.name.clone(),
            blocking_column: "integrity_failed_at",
        });
    }
    if collection.pending_manifest_incomplete_at.is_some() {
        return Err(VaultSyncError::RestoreIntegrityBlocked {
            collection_name: collection.name.clone(),
            blocking_column: "pending_manifest_incomplete_at",
        });
    }
    Ok(())
}

fn ensure_restore_target_is_empty(target_path: &Path) -> Result<(), VaultSyncError> {
    if !target_path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(target_path)?;
    if metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(VaultSyncError::RestoreNonEmptyTarget {
            target: target_path.display().to_string(),
        });
    }
    if metadata.is_dir() && fs::read_dir(target_path)?.next().is_none() {
        return Ok(());
    }
    Err(VaultSyncError::RestoreNonEmptyTarget {
        target: target_path.display().to_string(),
    })
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

fn materialize_collection_to_path(
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

fn infer_restore_relative_path(
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

fn has_fresh_command_heartbeat(
    conn: &Connection,
    collection: &Collection,
) -> Result<bool, VaultSyncError> {
    let Some(heartbeat_at) = collection.pending_command_heartbeat_at.as_deref() else {
        return Ok(false);
    };
    let fresh = conn.query_row(
        "SELECT (?1 >= datetime('now', ?2))",
        params![heartbeat_at, format!("-{SESSION_LIVENESS_SECS} seconds")],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(fresh != 0)
}

fn pending_manifest_age_seconds(
    conn: &Connection,
    collection: &Collection,
) -> Result<i64, VaultSyncError> {
    let Some(pending_at) = collection.pending_manifest_incomplete_at.as_deref() else {
        return Ok(0);
    };
    let age = conn.query_row(
        "SELECT CAST(strftime('%s','now') - strftime('%s', ?1) AS INTEGER)",
        [pending_at],
        |row| row.get(0),
    )?;
    Ok(age)
}

fn clear_manifest_incomplete(conn: &Connection, collection_id: i64) -> Result<(), VaultSyncError> {
    conn.execute(
        "UPDATE collections
         SET pending_manifest_incomplete_at = NULL,
             integrity_failed_at = NULL,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection_id],
    )?;
    Ok(())
}

fn revert_orphan_restore_state(
    conn: &Connection,
    collection: &Collection,
) -> Result<(), VaultSyncError> {
    conn.execute(
        "UPDATE collections
         SET state = CASE WHEN root_path = '' THEN 'detached' ELSE 'active' END,
             pending_root_path = NULL,
             pending_restore_manifest = NULL,
             pending_command_heartbeat_at = NULL,
             restore_command_id = NULL,
             restore_command_pid = NULL,
             restore_command_host = NULL,
             watcher_released_session_id = NULL,
             watcher_released_generation = NULL,
             watcher_released_at = NULL,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection.id],
    )?;
    Ok(())
}

fn revert_aborted_restore(
    conn: &Connection,
    collection: &Collection,
) -> Result<(), VaultSyncError> {
    conn.execute(
        "UPDATE collections
         SET state = CASE WHEN root_path = '' THEN 'detached' ELSE 'active' END,
             pending_root_path = NULL,
             pending_restore_manifest = NULL,
             pending_command_heartbeat_at = NULL,
             restore_command_id = NULL,
             restore_command_pid = NULL,
             restore_command_host = NULL,
             watcher_released_session_id = NULL,
             watcher_released_generation = NULL,
             watcher_released_at = NULL,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection.id],
    )?;
    Ok(())
}

enum ManifestComparison {
    Matches,
    MissingFiles,
    Mismatch,
}

fn compare_manifest(
    path: &Path,
    manifest: &RestoreManifest,
) -> Result<ManifestComparison, VaultSyncError> {
    let mut actual = build_restore_manifest_for_directory(path)?;
    actual
        .entries
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    if actual.entries.len() != manifest.entries.len() {
        let actual_paths: HashSet<_> = actual
            .entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect();
        let manifest_paths: HashSet<_> = manifest
            .entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect();
        if manifest_paths.difference(&actual_paths).next().is_some() {
            return Ok(ManifestComparison::MissingFiles);
        }
        return Ok(ManifestComparison::Mismatch);
    }
    for (expected, actual) in manifest.entries.iter().zip(actual.entries.iter()) {
        if expected.relative_path != actual.relative_path {
            return Ok(ManifestComparison::Mismatch);
        }
        if expected.sha256 != actual.sha256 || expected.size_bytes != actual.size_bytes {
            return Ok(ManifestComparison::Mismatch);
        }
    }
    Ok(ManifestComparison::Matches)
}

#[derive(Debug, Clone)]
struct RemapPageRow {
    page_id: i64,
    slug: String,
    uuid: Option<String>,
    sha256: String,
    body_size_bytes: i64,
    has_nonempty_body: bool,
}

#[derive(Debug, Clone)]
struct NewRootFileRow {
    relative_path: PathBuf,
    uuid: Option<String>,
    sha256: String,
    body_size_bytes: i64,
    has_nonempty_body: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeFenceEntry {
    mtime_ns: i64,
    ctime_ns: i64,
    size_bytes: u64,
    inode: u64,
    quaidignore_sha256: Option<String>,
}

struct PageMatchResolution {
    resolved_page_ids: HashSet<i64>,
    mismatched_pages: usize,
    mismatched_samples: Vec<String>,
    missing_count: usize,
    missing_pages: Vec<String>,
    extra_count: usize,
    extra_files: Vec<String>,
}

fn load_remap_page_rows(
    conn: &Connection,
    collection_id: i64,
) -> Result<Vec<RemapPageRow>, VaultSyncError> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.slug, p.uuid, p.compiled_truth, p.timeline, ri.raw_bytes
         FROM pages p
         JOIN raw_imports ri
           ON ri.page_id = p.id AND ri.is_active = 1
         WHERE p.collection_id = ?1 AND p.quarantined_at IS NULL
         ORDER BY p.slug",
    )?;
    let rows = stmt
        .query_map([collection_id], |row| {
            let compiled_truth: String = row.get(3)?;
            let timeline: String = row.get(4)?;
            let raw_bytes: Vec<u8> = row.get(5)?;
            let body = format!("{compiled_truth}\n{timeline}");
            Ok(RemapPageRow {
                page_id: row.get(0)?,
                slug: row.get(1)?,
                uuid: row.get(2)?,
                sha256: sha256_hex(&raw_bytes),
                body_size_bytes: body.trim().len() as i64,
                has_nonempty_body: !body.trim().is_empty(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn load_new_root_files(root: &Path) -> Result<Vec<NewRootFileRow>, VaultSyncError> {
    let ignore_globset = build_new_root_ignore_globset(root)?;
    let walked = walk_tree(root)?;
    let mut rows = Vec::new();
    for relative_path in walked.keys() {
        if relative_path == Path::new(".quaidignore") {
            continue;
        }
        if !is_markdown_file(relative_path) {
            continue;
        }
        if ignore_globset.is_match(relative_path) {
            continue;
        }
        let bytes = fs::read(root.join(relative_path))?;
        let text = String::from_utf8_lossy(&bytes);
        let (frontmatter, body) = markdown::parse_frontmatter(&text);
        let (compiled_truth, timeline) = markdown::split_content(&body);
        let trimmed_ct = compiled_truth.trim();
        let trimmed_tl = timeline.trim();
        rows.push(NewRootFileRow {
            relative_path: relative_path.clone(),
            uuid: page_uuid::parse_frontmatter_uuid(&frontmatter).map_err(|error| {
                VaultSyncError::InvariantViolation {
                    message: error.to_string(),
                }
            })?,
            sha256: sha256_hex(&bytes),
            body_size_bytes: (trimmed_ct.len() + trimmed_tl.len()) as i64,
            has_nonempty_body: !(trimmed_ct.is_empty() && trimmed_tl.is_empty()),
        });
    }
    Ok(rows)
}

fn build_new_root_ignore_globset(root: &Path) -> Result<globset::GlobSet, VaultSyncError> {
    let ignore_path = root.join(".quaidignore");
    let user_patterns_json = if ignore_path.exists() {
        let content = fs::read_to_string(&ignore_path)?;
        match ignore_patterns::parse_ignore_file(&content) {
            ignore_patterns::ParseResult::Valid(patterns) => Some(
                serde_json::to_string(&patterns)
                    .expect("serializing validated .quaidignore patterns should never fail"),
            ),
            ignore_patterns::ParseResult::Invalid(errors) => {
                let first = errors
                    .first()
                    .expect("invalid parse must include at least one error");
                return Err(VaultSyncError::InvariantViolation {
                    message: format!(
                        "invalid .quaidignore in remap root at line {}: {}",
                        first.line, first.message
                    ),
                });
            }
        }
    } else {
        None
    };
    ignore_patterns::build_globset_from_patterns(user_patterns_json.as_deref()).map_err(|message| {
        VaultSyncError::InvariantViolation {
            message: format!("failed to build remap ignore matcher: {message}"),
        }
    })
}

fn resolve_page_matches(pages: &[RemapPageRow], files: &[NewRootFileRow]) -> PageMatchResolution {
    let file_records = files
        .iter()
        .map(|file| CanonicalIdentityRecord {
            key: file.relative_path.clone(),
            label: path_string(&file.relative_path),
            uuid: file.uuid.clone(),
            sha256: file.sha256.clone(),
            body_size_bytes: file.body_size_bytes,
            has_nonempty_body: file.has_nonempty_body,
        })
        .collect::<Vec<_>>();
    let file_lookup = files
        .iter()
        .map(|file| (file.relative_path.clone(), file))
        .collect::<HashMap<_, _>>();
    let mut page_hash_counts = HashMap::new();
    for page in pages {
        *page_hash_counts
            .entry(page.sha256.as_str())
            .or_insert(0usize) += 1;
    }
    let mut resolved_page_ids = HashSet::new();
    let mut accounted_page_ids = HashSet::new();
    let mut accounted_file_paths = HashSet::new();
    let mut mismatched_pages = 0;
    let mut mismatched_samples = Vec::new();

    for page in pages {
        let page_record = CanonicalIdentityRecord {
            key: (),
            label: page.slug.clone(),
            uuid: page.uuid.clone(),
            sha256: page.sha256.clone(),
            body_size_bytes: page.body_size_bytes,
            has_nonempty_body: page.has_nonempty_body,
        };
        match resolve_page_identity(
            &page_record,
            page_hash_counts
                .get(page.sha256.as_str())
                .copied()
                .unwrap_or(0),
            &file_records,
        ) {
            PageIdentityResolution::Matched(relative_path) => {
                let Some(file) = file_lookup.get(&relative_path) else {
                    continue;
                };
                accounted_page_ids.insert(page.page_id);
                accounted_file_paths.insert(relative_path.clone());
                if file.sha256 == page.sha256 {
                    resolved_page_ids.insert(page.page_id);
                } else {
                    mismatched_pages += 1;
                    push_sample(
                        &mut mismatched_samples,
                        format!(
                            "{} -> {} sha256_mismatch",
                            page.slug,
                            path_string(&relative_path)
                        ),
                    );
                }
            }
            PageIdentityResolution::DuplicateUuid { candidate_labels } => {
                mismatched_pages += 1;
                push_sample(
                    &mut mismatched_samples,
                    format!("{} -> {}", page.slug, candidate_labels.join("|")),
                );
            }
            PageIdentityResolution::Missing
            | PageIdentityResolution::AmbiguousHash
            | PageIdentityResolution::TrivialHashRefusal { .. } => {}
        }
    }

    let missing_count = pages
        .iter()
        .filter(|page| !accounted_page_ids.contains(&page.page_id))
        .count();
    let missing_pages = pages
        .iter()
        .filter(|page| !accounted_page_ids.contains(&page.page_id))
        .map(|page| page.slug.clone())
        .take(REMAP_VERIFICATION_SAMPLE_LIMIT)
        .collect::<Vec<_>>();
    let extra_count = files
        .iter()
        .filter(|file| !accounted_file_paths.contains(&file.relative_path))
        .count();
    let extra_files = files
        .iter()
        .filter(|file| !accounted_file_paths.contains(&file.relative_path))
        .map(|file| path_string(&file.relative_path))
        .take(REMAP_VERIFICATION_SAMPLE_LIMIT)
        .collect::<Vec<_>>();

    PageMatchResolution {
        resolved_page_ids,
        mismatched_pages,
        mismatched_samples,
        missing_count,
        missing_pages,
        extra_count,
        extra_files,
    }
}

#[cfg(unix)]
fn metadata_timestamp_ns(seconds: i64, nanos: i64) -> i64 {
    seconds.saturating_mul(1_000_000_000).saturating_add(nanos)
}

#[cfg(unix)]
fn metadata_fence_tuple(metadata: &fs::Metadata) -> (i64, i64, u64) {
    (
        metadata_timestamp_ns(metadata.mtime(), metadata.mtime_nsec()),
        metadata_timestamp_ns(metadata.ctime(), metadata.ctime_nsec()),
        metadata.ino(),
    )
}

#[cfg(not(unix))]
fn metadata_fence_tuple(metadata: &fs::Metadata) -> (i64, i64, u64) {
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .and_then(|value| i64::try_from(value.as_nanos()).ok())
        .unwrap_or_default();
    (mtime_ns, 0, 0)
}

fn take_tree_fence(root: &Path) -> Result<BTreeMap<String, TreeFenceEntry>, VaultSyncError> {
    let mut fence = BTreeMap::new();
    for (relative_path, metadata) in walk_tree(root)? {
        let (mtime_ns, ctime_ns, inode) = metadata_fence_tuple(&metadata);
        let quaidignore_sha256 = if relative_path == Path::new(".quaidignore") {
            Some(sha256_hex(&fs::read(root.join(&relative_path))?))
        } else {
            None
        };
        fence.insert(
            path_string(&relative_path),
            TreeFenceEntry {
                mtime_ns,
                ctime_ns,
                size_bytes: metadata.len(),
                inode,
                quaidignore_sha256,
            },
        );
    }
    Ok(fence)
}

fn walk_tree(root: &Path) -> Result<BTreeMap<PathBuf, fs::Metadata>, VaultSyncError> {
    fn walk_dir(
        root: &Path,
        current: &Path,
        output: &mut BTreeMap<PathBuf, fs::Metadata>,
    ) -> Result<(), VaultSyncError> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.clone());
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                eprintln!("WARN: skipping symlinked entry {}", relative.display());
                continue;
            }
            if file_type.is_dir() {
                walk_dir(root, &path, output)?;
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let metadata = entry.metadata()?;
            output.insert(relative, metadata);
        }
        Ok(())
    }

    let mut output = BTreeMap::new();
    if root.exists() {
        walk_dir(root, root, &mut output)?;
    }
    Ok(output)
}

fn path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn push_sample(samples: &mut Vec<String>, sample: String) {
    if samples.len() < REMAP_VERIFICATION_SAMPLE_LIMIT {
        samples.push(sample);
    }
}

fn format_diff_samples(samples: &[String]) -> String {
    if samples.is_empty() {
        "-".to_owned()
    } else {
        samples.join(",")
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

pub fn parse_slug_input(input: &str) -> Result<(), VaultSyncError> {
    fn valid_token(value: &str) -> bool {
        value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'/'
                || byte == b'_'
                || byte == b'-'
        })
    }
    if let Some((collection_name, slug)) = input.split_once("::") {
        collections::validate_collection_name(collection_name)?;
        collections::validate_relative_path(slug)?;
        if !valid_token(collection_name) || !valid_token(slug) {
            return Err(VaultSyncError::InvariantViolation {
                message: "invalid slug: allowed characters are [a-z0-9/_-]".to_owned(),
            });
        }
        return Ok(());
    }
    collections::validate_relative_path(input)?;
    if !valid_token(input) {
        return Err(VaultSyncError::InvariantViolation {
            message: "invalid slug: allowed characters are [a-z0-9/_-]".to_owned(),
        });
    }
    Ok(())
}

pub fn resolve_page_for_read(
    conn: &Connection,
    input: &str,
) -> Result<ResolvedSlug, VaultSyncError> {
    resolve_slug_for_op(conn, input, OpKind::Read)
}

pub fn get_page_by_input(
    conn: &Connection,
    input: &str,
) -> Result<crate::core::types::Page, VaultSyncError> {
    let resolved = resolve_page_for_read(conn, input)?;
    get_page_by_key(conn, resolved.collection_id, &resolved.slug).map_err(|err| {
        let message = err.to_string();
        if message.contains("page not found") {
            VaultSyncError::PageNotFound {
                slug: input.to_owned(),
            }
        } else {
            VaultSyncError::InvariantViolation { message }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;
    use std::ffi::OsString;

    static ENV_MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_mutation_lock() -> &'static Mutex<()> {
        ENV_MUTATION_LOCK.get_or_init(|| Mutex::new(()))
    }

    #[cfg(all(unix, target_os = "linux"))]
    fn secure_runtime_root() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }

        fn clear(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(value) = self.previous.as_ref() {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn open_test_db() -> Connection {
        db::open(":memory:").unwrap()
    }

    fn open_test_db_file() -> (tempfile::TempDir, String, Connection) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        (dir, db_path.display().to_string(), conn)
    }

    fn insert_collection(conn: &Connection, name: &str, root_path: &Path) -> i64 {
        conn.execute(
            "INSERT INTO collections (name, root_path, state, writable, is_write_target)
             VALUES (?1, ?2, 'active', 1, 0)",
            params![name, root_path.display().to_string()],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_collection_with_id(
        conn: &Connection,
        collection_id: i64,
        name: &str,
        root_path: &Path,
    ) -> i64 {
        conn.execute(
            "INSERT INTO collections (id, name, root_path, state, writable, is_write_target)
             VALUES (?1, ?2, ?3, 'active', 1, 0)",
            params![collection_id, name, root_path.display().to_string()],
        )
        .unwrap();
        collection_id
    }

    fn insert_page_with_raw_import(
        conn: &Connection,
        collection_id: i64,
        slug: &str,
        uuid: &str,
        compiled_truth: &str,
        raw_bytes: &[u8],
        relative_path: &str,
    ) -> i64 {
        let frontmatter_json = std::str::from_utf8(raw_bytes)
            .ok()
            .map(|s| {
                let (fm, _) = markdown::parse_frontmatter(s);
                serde_json::to_string(&fm).unwrap_or_else(|_| "{}".to_owned())
            })
            .unwrap_or_else(|| "{}".to_owned());
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES (?1, ?2, ?3, 'concept', ?2, '', ?4, '', ?5, '', '', 1)",
            params![collection_id, slug, uuid, compiled_truth, frontmatter_json],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
             VALUES (?1, ?2, 1, ?3, ?4)",
            params![
                page_id,
                Uuid::now_v7().to_string(),
                raw_bytes,
                relative_path
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_state (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256)
             VALUES (?1, ?2, ?3, 1, 1, ?4, 1, ?5)",
            params![collection_id, relative_path, page_id, raw_bytes.len() as i64, sha256_hex(raw_bytes)],
        )
        .unwrap();
        page_id
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn start_serve_runtime_refuses_insecure_ipc_directory_permissions() {
        let _env_lock = env_mutation_lock().lock().unwrap();
        let runtime_root = secure_runtime_root();
        let socket_dir = runtime_root.path().join("quaid");
        fs::create_dir_all(&socket_dir).unwrap();
        fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o755)).unwrap();
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", runtime_root.path().to_str().unwrap());
        let (_dir, db_path, _conn) = open_test_db_file();

        let error = match start_serve_runtime(db_path) {
            Ok(_) => panic!("expected insecure socket directory to refuse startup"),
            Err(error) => error,
        };

        assert!(matches!(error, VaultSyncError::IpcDirectoryInsecure { .. }));
        assert!(error.to_string().contains("IpcDirectoryInsecureError"));
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn start_serve_runtime_refuses_insecure_xdg_runtime_root_permissions() {
        let _env_lock = env_mutation_lock().lock().unwrap();
        let runtime_root = tempfile::TempDir::new().unwrap();
        fs::set_permissions(runtime_root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", runtime_root.path().to_str().unwrap());
        let (_dir, db_path, _conn) = open_test_db_file();

        let error = match start_serve_runtime(db_path) {
            Ok(_) => panic!("expected insecure XDG runtime root to refuse startup"),
            Err(error) => error,
        };

        assert!(matches!(error, VaultSyncError::IpcDirectoryInsecure { .. }));
        assert!(error.to_string().contains("IpcDirectoryInsecureError"));
        assert!(error.to_string().contains("mode 755 is not 700"));
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn start_serve_runtime_refuses_insecure_fallback_runtime_root_permissions() {
        let _env_lock = env_mutation_lock().lock().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let runtime_root = home.path().join(".cache").join("quaid");
        fs::create_dir_all(&runtime_root).unwrap();
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o755)).unwrap();
        let _xdg = EnvVarGuard::clear("XDG_RUNTIME_DIR");
        let _home = EnvVarGuard::set("HOME", home.path().to_str().unwrap());
        let (_dir, db_path, _conn) = open_test_db_file();

        let error = match start_serve_runtime(db_path) {
            Ok(_) => panic!("expected insecure fallback runtime root to refuse startup"),
            Err(error) => error,
        };

        assert!(matches!(error, VaultSyncError::IpcDirectoryInsecure { .. }));
        assert!(error.to_string().contains("IpcDirectoryInsecureError"));
        assert!(error.to_string().contains(runtime_root.to_str().unwrap()));
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn publish_ipc_socket_unlinks_stale_socket_before_bind() {
        let _env_lock = env_mutation_lock().lock().unwrap();
        let runtime_root = secure_runtime_root();
        let socket_dir = runtime_root.path().join("quaid");
        fs::create_dir_all(&socket_dir).unwrap();
        fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", runtime_root.path().to_str().unwrap());
        let (_dir, _db_path, conn) = open_test_db_file();
        let session_id = "stale-session";
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES (?1, 42, 'host')",
            [session_id],
        )
        .unwrap();
        let socket_path = socket_dir.join(format!("{session_id}.sock"));
        let stale_listener = UnixListener::bind(&socket_path).unwrap();
        drop(stale_listener);

        let published = publish_ipc_socket(&conn, session_id).unwrap();

        assert_eq!(published.path, socket_path);
        assert!(published.path.exists());
        cleanup_published_ipc_socket(&conn, session_id, &published.path).unwrap();
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn audit_bound_ipc_socket_rejects_mode_regression() {
        let _env_lock = env_mutation_lock().lock().unwrap();
        let runtime_root = secure_runtime_root();
        let socket_dir = runtime_root.path().join("quaid");
        fs::create_dir_all(&socket_dir).unwrap();
        fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", runtime_root.path().to_str().unwrap());
        let socket_path = socket_dir.join("mode-regression.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o644)).unwrap();

        let error = audit_bound_ipc_socket(&socket_path).unwrap_err();

        assert!(matches!(error, VaultSyncError::IpcSocketPermission { .. }));
        drop(listener);
        let _ = fs::remove_file(&socket_path);
    }

    #[cfg(unix)]
    #[test]
    fn authorize_server_peer_rejects_cross_uid_peer() {
        let socket_path = Path::new("D:\\repos\\quaid-vault-sync-batch5-v0140\\fake.sock");
        let peer = IpcPeerCredentials {
            pid: 77,
            uid: current_effective_uid() + 1,
        };

        let error = authorize_server_peer(socket_path, &peer).unwrap_err();

        assert!(matches!(error, VaultSyncError::IpcPeerAuthFailed { .. }));
        assert!(error.to_string().contains("IpcPeerAuthFailedError"));
    }

    #[cfg(unix)]
    #[test]
    fn authorize_client_peer_rejects_cross_uid_even_with_matching_whoami() {
        let socket_path = Path::new("D:\\repos\\quaid-vault-sync-batch5-v0140\\fake.sock");
        let peer = IpcPeerCredentials {
            pid: 88,
            uid: current_effective_uid() + 1,
        };

        let error = authorize_client_peer(
            socket_path,
            "session-a",
            "session-a",
            88,
            &peer,
            "session-a",
        )
        .unwrap_err();

        assert!(matches!(error, VaultSyncError::IpcPeerAuthFailed { .. }));
        assert!(error.to_string().contains("IpcPeerAuthFailedError"));
    }

    #[test]
    fn serve_ipc_source_publishes_after_audit_and_cleans_up_before_unregister() {
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("core")
                .join("vault_sync.rs"),
        )
        .unwrap();
        let publish_start = source
            .find("fn publish_ipc_socket(")
            .expect("publish helper present");
        let publish_end = source[publish_start..]
            .find("pub fn start_serve_runtime(")
            .map(|offset| publish_start + offset)
            .expect("publish helper boundary");
        let publish_source = &source[publish_start..publish_end];
        let runtime_root_idx = publish_source
            .find("ensure_secure_ipc_directory(&location.runtime_root")
            .expect("secure runtime root check");
        let dir_idx = publish_source
            .find("ensure_secure_ipc_directory(&location.socket_dir, true)")
            .expect("secure socket directory check");
        let stale_idx = publish_source
            .find("clear_stale_ipc_socket(&socket_path)")
            .expect("stale socket cleanup");
        let set_perms_idx = publish_source
            .find("fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))")
            .expect("explicit socket permissions set after bind");
        let audit_idx = publish_source
            .find("audit_bound_ipc_socket(&socket_path)")
            .expect("bind-time audit");
        let publish_idx = publish_source
            .find("UPDATE serve_sessions SET ipc_path = ?1 WHERE session_id = ?2")
            .expect("ipc path publish");
        assert!(
            runtime_root_idx < dir_idx
                && dir_idx < stale_idx
                && stale_idx < set_perms_idx
                && set_perms_idx < audit_idx
                && audit_idx < publish_idx
        );

        let runtime_start = source
            .find("pub fn start_serve_runtime(")
            .expect("serve runtime present");
        let runtime_end = source[runtime_start..]
            .find("#[derive(Debug, Clone)]")
            .map(|offset| runtime_start + offset)
            .expect("serve runtime boundary");
        let runtime_source = &source[runtime_start..runtime_end];
        let cleanup_idx = runtime_source
            .find(
                "cleanup_published_ipc_socket(&conn, &session_id_for_thread, &published_ipc.path)",
            )
            .expect("ipc cleanup call");
        let unregister_idx = runtime_source
            .find("unregister_session(&conn, &session_id_for_thread)")
            .expect("session unregister call");
        assert!(cleanup_idx < unregister_idx);
    }

    #[test]
    fn serve_ipc_source_refuses_cross_uid_peer_before_request_dispatch() {
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("core")
                .join("vault_sync.rs"),
        )
        .unwrap();
        let handler_start = source
            .find("fn handle_ipc_client(")
            .expect("ipc handler present");
        let handler_end = source[handler_start..]
            .find("fn write_ipc_response(")
            .map(|offset| handler_start + offset)
            .expect("ipc handler boundary");
        let handler_source = &source[handler_start..handler_end];
        let peer_idx = handler_source
            .find("let peer = peer_credentials_for_stream(&stream)?;")
            .expect("peer credential lookup");
        let auth_idx = handler_source
            .find("authorize_server_peer(socket_path, &peer)?;")
            .expect("server peer auth");
        let parse_idx = handler_source
            .find("serde_json::from_str::<IpcRequest>(line.trim_end())")
            .expect("request parse");
        assert!(peer_idx < auth_idx && auth_idx < parse_idx);
    }

    #[test]
    fn drain_embedding_queue_marks_failed_jobs_and_retries_after_backoff() {
        let conn = open_test_db();
        let page_id = insert_page_with_raw_import(
            &conn,
            1,
            "notes/retry",
            &Uuid::now_v7().to_string(),
            "Retry candidate truth.",
            b"---\ntitle: Retry\ntype: note\n---\nRetry candidate truth.\n",
            "notes/retry.md",
        );
        crate::core::raw_imports::enqueue_embedding_job(&conn, page_id).unwrap();
        conn.execute(
            "UPDATE embedding_models SET vec_table = 'bad-table' WHERE active = 1",
            [],
        )
        .unwrap();

        drain_embedding_queue(&conn).unwrap();

        let failed_row: (String, i64, Option<String>) = conn
            .query_row(
                "SELECT job_state, attempt_count, last_error
                 FROM embedding_jobs
                 WHERE page_id = ?1",
                [page_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(failed_row.0, "failed");
        assert_eq!(failed_row.1, 1);
        assert!(failed_row
            .2
            .as_deref()
            .is_some_and(|message| message.contains("unsafe vec table name")));

        drain_embedding_queue(&conn).unwrap();
        let attempt_count_after_immediate_retry: i64 = conn
            .query_row(
                "SELECT attempt_count FROM embedding_jobs WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attempt_count_after_immediate_retry, 1);

        conn.execute(
            "UPDATE embedding_jobs
             SET started_at = datetime('now', '-2 seconds')
             WHERE page_id = ?1",
            [page_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE embedding_models
             SET vec_table = 'page_embeddings_vec_384'
             WHERE active = 1",
            [],
        )
        .unwrap();

        drain_embedding_queue(&conn).unwrap();

        let remaining_jobs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM embedding_jobs WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_jobs, 0);
    }

    #[test]
    fn configured_embedding_concurrency_uses_positive_env_override_when_present() {
        let _lock = env_mutation_lock().lock().unwrap();
        let _clear = EnvVarGuard::clear("QUAID_EMBEDDING_CONCURRENCY");
        let _override = EnvVarGuard::set("QUAID_EMBEDDING_CONCURRENCY", "3");

        assert_eq!(configured_embedding_concurrency(), 3);
    }

    #[test]
    fn configured_embedding_concurrency_falls_back_for_zero_or_invalid_values() {
        let _lock = env_mutation_lock().lock().unwrap();
        let _clear = EnvVarGuard::clear("QUAID_EMBEDDING_CONCURRENCY");
        let fallback = configured_embedding_concurrency();

        let _zero = EnvVarGuard::set("QUAID_EMBEDDING_CONCURRENCY", "0");
        assert_eq!(configured_embedding_concurrency(), fallback);
        drop(_zero);

        let _invalid = EnvVarGuard::set("QUAID_EMBEDDING_CONCURRENCY", "bogus");
        assert_eq!(configured_embedding_concurrency(), fallback);
    }

    #[test]
    fn drain_embedding_queue_leaves_five_attempt_jobs_failed_without_reclaiming() {
        let conn = open_test_db();
        let page_id = insert_page_with_raw_import(
            &conn,
            1,
            "notes/permanent-failure",
            &Uuid::now_v7().to_string(),
            "Permanent failure truth.",
            b"---\ntitle: Permanent Failure\ntype: note\n---\nPermanent failure truth.\n",
            "notes/permanent-failure.md",
        );
        conn.execute(
            "INSERT INTO embedding_jobs (page_id, job_state, attempt_count, last_error, started_at)
             VALUES (?1, 'failed', 5, 'still broken', '2026-04-28T00:00:00Z')",
            [page_id],
        )
        .unwrap();

        drain_embedding_queue(&conn).unwrap();

        let row: (String, i64, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT job_state, attempt_count, last_error, started_at
                 FROM embedding_jobs
                 WHERE page_id = ?1",
                [page_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "failed");
        assert_eq!(row.1, 5);
        assert_eq!(row.2.as_deref(), Some("still broken"));
        assert_eq!(row.3.as_deref(), Some("2026-04-28T00:00:00Z"));
    }

    #[test]
    fn process_embedding_job_on_connection_deletes_orphaned_jobs_when_page_is_missing() {
        let conn = open_test_db();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        conn.execute(
            "INSERT INTO embedding_jobs (id, page_id, job_state) VALUES (1, 999, 'pending')",
            [],
        )
        .unwrap();

        process_embedding_job_on_connection(&conn, 1, 999).unwrap();

        let remaining_jobs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM embedding_jobs WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_jobs, 0);
    }

    #[test]
    fn resume_orphaned_embedding_jobs_resets_running_rows_to_pending() {
        let conn = open_test_db();
        let page_id = insert_page_with_raw_import(
            &conn,
            1,
            "notes/resume",
            &Uuid::now_v7().to_string(),
            "Resume candidate truth.",
            b"---\ntitle: Resume\ntype: note\n---\nResume candidate truth.\n",
            "notes/resume.md",
        );
        conn.execute(
            "INSERT INTO embedding_jobs (page_id, job_state, attempt_count, started_at)
             VALUES (?1, 'running', 3, '2026-04-28T00:00:00Z')",
            [page_id],
        )
        .unwrap();

        let resumed = resume_orphaned_embedding_jobs(&conn).unwrap();
        assert_eq!(resumed, 1);

        let row: (String, i64, Option<String>) = conn
            .query_row(
                "SELECT job_state, attempt_count, started_at
                 FROM embedding_jobs
                 WHERE page_id = ?1",
                [page_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, "pending");
        assert_eq!(row.1, 3);
        assert!(row.2.is_none());
    }

    #[test]
    fn run_startup_sequence_resumes_running_embedding_jobs_before_runtime_loop_starts() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let page_id = insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/startup-resume",
            &Uuid::now_v7().to_string(),
            "Startup resume truth.",
            b"---\ntitle: Startup Resume\ntype: note\n---\nStartup resume truth.\n",
            "notes/startup-resume.md",
        );
        conn.execute(
            "INSERT INTO embedding_jobs (page_id, job_state, attempt_count, started_at)
             VALUES (?1, 'running', 2, '2026-04-28T00:00:00Z')",
            [page_id],
        )
        .unwrap();
        let session_id = register_session(&conn).unwrap();

        run_startup_sequence(&conn, Path::new(&db_path), &session_id).unwrap();

        let row: (String, i64, Option<String>) = conn
            .query_row(
                "SELECT job_state, attempt_count, started_at
                 FROM embedding_jobs
                 WHERE page_id = ?1",
                [page_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, "pending");
        assert_eq!(row.1, 2);
        assert!(row.2.is_none());

        unregister_session(&conn, &session_id).unwrap();
    }

    #[test]
    fn list_memory_collections_counts_pending_and_running_separately_from_failed_jobs() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let pending_page_id = insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/pending",
            &Uuid::now_v7().to_string(),
            "Pending truth.",
            b"---\ntitle: Pending\ntype: note\n---\nPending truth.\n",
            "notes/pending.md",
        );
        let running_page_id = insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/running",
            &Uuid::now_v7().to_string(),
            "Running truth.",
            b"---\ntitle: Running\ntype: note\n---\nRunning truth.\n",
            "notes/running.md",
        );
        let failed_page_id = insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/failed",
            &Uuid::now_v7().to_string(),
            "Failed truth.",
            b"---\ntitle: Failed\ntype: note\n---\nFailed truth.\n",
            "notes/failed.md",
        );
        conn.execute(
            "INSERT INTO embedding_jobs (page_id, job_state) VALUES (?1, 'pending')",
            [pending_page_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embedding_jobs (page_id, job_state, started_at)
             VALUES (?1, 'running', '2026-04-28T00:00:00Z')",
            [running_page_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embedding_jobs (page_id, job_state, attempt_count, last_error)
             VALUES (?1, 'failed', 5, 'boom')",
            [failed_page_id],
        )
        .unwrap();

        let view = list_memory_collections(&conn)
            .unwrap()
            .into_iter()
            .find(|view| view.name == "work")
            .unwrap();
        assert_eq!(view.embedding_queue_depth, 2);
        assert_eq!(view.failing_jobs, 1);
    }

    fn write_restore_file(root: &Path, relative_path: &str, bytes: &[u8]) {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }

    fn production_vault_sync_source() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("core")
            .join("vault_sync.rs");
        let source = std::fs::read_to_string(path).unwrap();
        let test_module_start = source.rfind("#[cfg(test)]").unwrap();
        source[..test_module_start].to_owned()
    }

    fn manifest_json_for_directory(root: &Path) -> String {
        serde_json::to_string(&build_restore_manifest_for_directory(root).unwrap()).unwrap()
    }

    #[cfg(unix)]
    fn wait_for_collection_update<T, F>(
        db_path: &str,
        collection_id: i64,
        timeout: Duration,
        read: F,
    ) -> T
    where
        F: Fn(&Connection, i64) -> Option<T>,
    {
        let started = Instant::now();
        loop {
            let verify = Connection::open(db_path).unwrap();
            if let Some(result) = read(&verify, collection_id) {
                return result;
            }
            drop(verify);
            assert!(
                started.elapsed() < timeout,
                "timed out waiting for collection_id={collection_id} after {:?}",
                timeout
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(unix)]
    fn create_startup_recovery_sentinel(recovery_root: &Path, collection_id: i64, name: &str) {
        let dir = collection_recovery_dir(recovery_root, collection_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), b"dirty").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn relative_markdown_path_accepts_only_markdown_within_root() {
        let root = Path::new("/vault");

        assert_eq!(
            relative_markdown_path(root, &root.join("notes").join("a.md")),
            Some(PathBuf::from("notes").join("a.md"))
        );
        assert_eq!(
            relative_markdown_path(root, &root.join("notes").join("a.MD")),
            Some(PathBuf::from("notes").join("a.MD"))
        );
        assert_eq!(
            relative_markdown_path(root, &root.join("notes").join("a.txt")),
            None
        );
        assert_eq!(
            relative_markdown_path(root, &PathBuf::from("/elsewhere").join("a.md")),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn poll_collection_watcher_returns_invariant_violation_when_channel_disconnects() {
        let conn = open_test_db();
        let (_sender, receiver) = mpsc::channel(1);
        drop(_sender);
        let watcher = notify::recommended_watcher(|_| {}).unwrap();
        let mut state = CollectionWatcherState {
            root_path: PathBuf::from("/vault"),
            generation: 0,
            receiver,
            watcher: Some(WatcherHandle::Native(watcher)),
            buffer: WatchBatchBuffer::default(),
            mode: WatcherMode::Native,
            last_event_at: None,
            last_watcher_error: None,
            backoff_until: None,
            consecutive_failures: 0,
        };

        let err = poll_collection_watcher(&conn, 42, &mut state).unwrap_err();

        assert!(matches!(err, VaultSyncError::InvariantViolation { .. }));
        assert!(err.to_string().contains("watch channel disconnected"));
    }

    #[cfg(unix)]
    #[test]
    fn classify_watch_event_emits_ignore_reload_without_markdown_dirty_path() {
        let root = tempfile::TempDir::new().unwrap();
        let ignore_path = root.path().join(".quaidignore");
        fs::write(&ignore_path, "notes/**\n").unwrap();
        let event = NotifyEvent {
            kind: NotifyEventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            paths: vec![ignore_path],
            attrs: Default::default(),
        };

        let actions = classify_watch_event(root.path(), event).unwrap();

        assert_eq!(actions, vec![WatchEvent::IgnoreFileChanged]);
    }

    #[cfg(unix)]
    #[test]
    fn ignore_file_change_reloads_mirror_and_triggers_reconcile() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        fs::write(root.path().join(".quaidignore"), "notes/**\n").unwrap();
        let (sender, receiver) = mpsc::channel(1);
        let watcher = notify::recommended_watcher(|_| {}).unwrap();
        let mut state = CollectionWatcherState {
            root_path: root.path().to_path_buf(),
            generation: 0,
            receiver,
            watcher: Some(WatcherHandle::Native(watcher)),
            buffer: WatchBatchBuffer::default(),
            mode: WatcherMode::Native,
            last_event_at: None,
            last_watcher_error: None,
            backoff_until: None,
            consecutive_failures: 0,
        };

        sender.blocking_send(WatchEvent::IgnoreFileChanged).unwrap();
        poll_collection_watcher(&conn, collection_id, &mut state).unwrap();
        state.buffer.debounce_deadline = Some(Instant::now());
        poll_collection_watcher(&conn, collection_id, &mut state).unwrap();

        let row: (Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT ignore_patterns, ignore_parse_errors, last_sync_at
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0.as_deref(), Some("[\"notes/**\"]"));
        assert!(row.1.is_none());
        assert!(row.2.is_some(), "ignore change should trigger reconcile");
    }

    #[cfg(unix)]
    #[test]
    fn invalid_ignore_file_change_preserves_mirror_and_skips_reconcile() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections
             SET ignore_patterns = ?2
             WHERE id = ?1",
            params![collection_id, "[\"notes/**\"]"],
        )
        .unwrap();
        fs::write(root.path().join(".quaidignore"), "[broken\n").unwrap();
        let (sender, receiver) = mpsc::channel(1);
        let watcher = notify::recommended_watcher(|_| {}).unwrap();
        let mut state = CollectionWatcherState {
            root_path: root.path().to_path_buf(),
            generation: 0,
            receiver,
            watcher: Some(WatcherHandle::Native(watcher)),
            buffer: WatchBatchBuffer::default(),
            mode: WatcherMode::Native,
            last_event_at: None,
            last_watcher_error: None,
            backoff_until: None,
            consecutive_failures: 0,
        };

        sender.blocking_send(WatchEvent::IgnoreFileChanged).unwrap();
        poll_collection_watcher(&conn, collection_id, &mut state).unwrap();
        state.buffer.debounce_deadline = Some(Instant::now());
        poll_collection_watcher(&conn, collection_id, &mut state).unwrap();

        let row: (Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT ignore_patterns, ignore_parse_errors, last_sync_at
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0.as_deref(), Some("[\"notes/**\"]"));
        assert!(row.1.unwrap().contains("parse_error"));
        assert!(row.2.is_none(), "invalid ignore file must skip reconcile");
    }

    #[cfg(unix)]
    #[test]
    fn deleted_ignore_file_with_prior_mirror_preserves_mirror_and_skips_reconcile() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections
             SET ignore_patterns = ?2
             WHERE id = ?1",
            params![collection_id, "[\"notes/**\"]"],
        )
        .unwrap();
        let (sender, receiver) = mpsc::channel(1);
        let watcher = notify::recommended_watcher(|_| {}).unwrap();
        let mut state = CollectionWatcherState {
            root_path: root.path().to_path_buf(),
            generation: 0,
            receiver,
            watcher: Some(WatcherHandle::Native(watcher)),
            buffer: WatchBatchBuffer::default(),
            mode: WatcherMode::Native,
            last_event_at: None,
            last_watcher_error: None,
            backoff_until: None,
            consecutive_failures: 0,
        };

        sender.blocking_send(WatchEvent::IgnoreFileChanged).unwrap();
        poll_collection_watcher(&conn, collection_id, &mut state).unwrap();
        state.buffer.debounce_deadline = Some(Instant::now());
        poll_collection_watcher(&conn, collection_id, &mut state).unwrap();

        let row: (Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT ignore_patterns, ignore_parse_errors, last_sync_at
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0.as_deref(), Some("[\"notes/**\"]"));
        let ignore_errors = row.1.unwrap();
        assert!(ignore_errors.contains("file_stably_absent_but_clear_not_confirmed"));
        assert!(
            row.2.is_none(),
            "missing ignore file must not trigger reconcile"
        );
    }

    #[test]
    fn list_memory_collections_only_marks_restore_in_progress_after_release_ack() {
        let conn = open_test_db();
        let active_id = insert_collection(&conn, "active", Path::new("vault-active"));
        let restoring_pending_id = insert_collection(
            &conn,
            "restoring-pending",
            Path::new("vault-restoring-pending"),
        );
        let restoring_live_id =
            insert_collection(&conn, "restoring-live", Path::new("vault-restoring-live"));
        conn.execute(
            "UPDATE collections
             SET state = 'active',
                 restore_command_id = 'restore-active',
                 watcher_released_at = '2026-04-25T00:00:00Z'
             WHERE id = ?1",
            [active_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 restore_command_id = 'restore-pending',
                 watcher_released_at = NULL
             WHERE id = ?1",
            [restoring_pending_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 restore_command_id = 'restore-live',
                 watcher_released_at = '2026-04-25T00:00:00Z'
             WHERE id = ?1",
            [restoring_live_id],
        )
        .unwrap();

        let views = list_memory_collections(&conn).unwrap();

        let active = views.iter().find(|view| view.name == "active").unwrap();
        assert_eq!(active.state, "active");
        assert!(!active.restore_in_progress);

        let restoring_pending = views
            .iter()
            .find(|view| view.name == "restoring-pending")
            .unwrap();
        assert_eq!(restoring_pending.state, "restoring");
        assert!(!restoring_pending.restore_in_progress);

        let restoring_live = views
            .iter()
            .find(|view| view.name == "restoring-live")
            .unwrap();
        assert_eq!(restoring_live.state, "restoring");
        assert!(restoring_live.restore_in_progress);
    }

    #[test]
    fn collection_recovery_in_progress_guard_sets_and_clears_flag() {
        init_process_registries().unwrap();
        let collection_id = 77;

        assert!(!collection_recovery_in_progress(collection_id));
        {
            let _guard = RecoveryInProgressGuard::enter(collection_id).unwrap();
            assert!(collection_recovery_in_progress(collection_id));
        }
        assert!(!collection_recovery_in_progress(collection_id));
    }

    #[test]
    fn load_collection_by_id_round_trips_optional_restore_metadata() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn, "work", Path::new("vault"));
        conn.execute(
            "UPDATE collections
             SET active_lease_session_id = 'serve-1',
                 restore_command_id = 'restore-1',
                 restore_lease_session_id = 'cli-lease',
                 reload_generation = 9,
                 watcher_released_session_id = 'serve-1',
                 watcher_released_generation = 8,
                 watcher_released_at = '2026-04-28T00:00:00Z',
                 pending_command_heartbeat_at = '2026-04-28T00:01:00Z',
                 pending_root_path = 'D:\\restored',
                 pending_restore_manifest = '{\"entries\":[]}',
                 restore_command_pid = 42,
                 restore_command_host = 'host-a',
                 integrity_failed_at = '2026-04-28T00:02:00Z',
                 pending_manifest_incomplete_at = '2026-04-28T00:03:00Z',
                 reconcile_halted_at = '2026-04-28T00:04:00Z',
                 reconcile_halt_reason = 'duplicate_uuid'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();

        assert_eq!(
            collection.active_lease_session_id.as_deref(),
            Some("serve-1")
        );
        assert_eq!(collection.restore_command_id.as_deref(), Some("restore-1"));
        assert_eq!(
            collection.restore_lease_session_id.as_deref(),
            Some("cli-lease")
        );
        assert_eq!(collection.reload_generation, 9);
        assert_eq!(
            collection.watcher_released_session_id.as_deref(),
            Some("serve-1")
        );
        assert_eq!(collection.watcher_released_generation, Some(8));
        assert_eq!(
            collection.pending_root_path.as_deref(),
            Some("D:\\restored")
        );
        assert_eq!(
            collection.pending_restore_manifest.as_deref(),
            Some("{\"entries\":[]}")
        );
        assert_eq!(collection.restore_command_pid, Some(42));
        assert_eq!(collection.restore_command_host.as_deref(), Some("host-a"));
        assert_eq!(
            collection.reconcile_halt_reason.as_deref(),
            Some("duplicate_uuid")
        );
    }

    #[test]
    fn load_collection_by_id_rejects_invalid_collection_state() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn, "work", Path::new("vault"));
        conn.pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        conn.execute(
            "UPDATE collections SET state = 'bogus' WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let error = load_collection_by_id(&conn, collection_id)
            .unwrap_err()
            .to_string();

        assert!(error.contains("invalid collection state"));
        assert!(error.contains("bogus"));
    }

    #[cfg(not(unix))]
    #[test]
    fn ensure_unix_platform_fails_closed_on_windows() {
        let error = ensure_unix_platform("quaid collection sync")
            .unwrap_err()
            .to_string();
        assert!(error.contains("UnsupportedPlatformError"));
        assert!(error.contains("quaid collection sync"));
    }

    #[test]
    fn build_restore_manifest_for_directory_sorts_paths_and_hashes_contents() {
        let root = tempfile::TempDir::new().unwrap();
        write_restore_file(root.path(), "notes/z.md", b"zeta");
        write_restore_file(root.path(), "notes/a.md", b"alpha");

        let manifest = build_restore_manifest_for_directory(root.path()).unwrap();

        assert_eq!(
            manifest
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["notes/a.md", "notes/z.md"]
        );
        assert_eq!(manifest.entries[0].sha256, sha256_hex(b"alpha"));
        assert_eq!(manifest.entries[0].size_bytes, 5);
        assert_eq!(manifest.entries[1].sha256, sha256_hex(b"zeta"));
        assert_eq!(manifest.entries[1].size_bytes, 4);
    }

    #[test]
    fn list_memory_collections_reports_integrity_blocked_variants() {
        let conn = open_test_db();
        let manifest_tamper = insert_collection(&conn, "manifest-tamper", Path::new("vault-a"));
        let manifest_retry = insert_collection(&conn, "manifest-retry", Path::new("vault-b"));
        let duplicate_uuid = insert_collection(&conn, "duplicate-uuid", Path::new("vault-c"));
        let trivial = insert_collection(&conn, "trivial", Path::new("vault-d"));
        let unknown = insert_collection(&conn, "unknown", Path::new("vault-e"));
        conn.execute(
            "UPDATE collections
             SET integrity_failed_at = '2026-04-28T00:00:00Z'
             WHERE id = ?1",
            [manifest_tamper],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET pending_manifest_incomplete_at = datetime('now', '-7200 seconds')
             WHERE id = ?1",
            [manifest_retry],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET reconcile_halted_at = '2026-04-28T00:00:00Z',
                 reconcile_halt_reason = 'duplicate_uuid'
             WHERE id = ?1",
            [duplicate_uuid],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET reconcile_halted_at = '2026-04-28T00:00:00Z',
                 reconcile_halt_reason = 'unresolvable_trivial_content'
             WHERE id = ?1",
            [trivial],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET reconcile_halted_at = '2026-04-28T00:00:00Z',
                 reconcile_halt_reason = 'mystery'
             WHERE id = ?1",
            [unknown],
        )
        .unwrap();

        let views = list_memory_collections(&conn).unwrap();

        assert_eq!(
            views
                .iter()
                .find(|view| view.name == "manifest-tamper")
                .unwrap()
                .integrity_blocked
                .as_deref(),
            Some("manifest_tampering")
        );
        assert_eq!(
            views
                .iter()
                .find(|view| view.name == "manifest-retry")
                .unwrap()
                .integrity_blocked
                .as_deref(),
            Some("manifest_incomplete_escalated")
        );
        assert_eq!(
            views
                .iter()
                .find(|view| view.name == "duplicate-uuid")
                .unwrap()
                .integrity_blocked
                .as_deref(),
            Some("duplicate_uuid")
        );
        assert_eq!(
            views
                .iter()
                .find(|view| view.name == "trivial")
                .unwrap()
                .integrity_blocked
                .as_deref(),
            Some("unresolvable_trivial_content")
        );
        assert!(views
            .iter()
            .find(|view| view.name == "unknown")
            .unwrap()
            .integrity_blocked
            .is_none());
    }

    #[test]
    fn integrity_blocked_reason_and_restore_in_progress_cover_all_helper_branches() {
        assert_eq!(
            integrity_blocked_label(
                &Some("2026-04-28T00:00:00Z".to_owned()),
                false,
                &None,
                &None
            )
            .as_deref(),
            Some("manifest_tampering")
        );
        assert_eq!(
            integrity_blocked_label(&None, true, &None, &None).as_deref(),
            Some("manifest_incomplete_escalated")
        );
        assert_eq!(
            integrity_blocked_label(
                &None,
                false,
                &Some("2026-04-28T00:00:00Z".to_owned()),
                &Some("duplicate_uuid".to_owned())
            )
            .as_deref(),
            Some("duplicate_uuid")
        );
        assert_eq!(
            integrity_blocked_label(
                &None,
                false,
                &Some("2026-04-28T00:00:00Z".to_owned()),
                &Some("unresolvable_trivial_content".to_owned())
            )
            .as_deref(),
            Some("unresolvable_trivial_content")
        );
        assert!(integrity_blocked_label(
            &None,
            false,
            &Some("2026-04-28T00:00:00Z".to_owned()),
            &Some("mystery".to_owned())
        )
        .is_none());
        assert!(integrity_blocked_label(&None, false, &None, &None).is_none());

        let conn = open_test_db();
        let collection_id = insert_collection(&conn, "work", Path::new("vault"));
        let mut collection = load_collection_by_id(&conn, collection_id).unwrap();
        collection.state = CollectionState::Restoring;
        collection.restore_command_id = Some("restore-1".to_owned());
        collection.watcher_released_at = Some("2026-04-28T00:00:00Z".to_owned());
        assert!(restore_in_progress(&collection));
        collection.watcher_released_at = None;
        assert!(!restore_in_progress(&collection));
        collection.watcher_released_at = Some("2026-04-28T00:00:00Z".to_owned());
        collection.restore_command_id = None;
        assert!(!restore_in_progress(&collection));
        collection.state = CollectionState::Active;
        collection.restore_command_id = Some("restore-1".to_owned());
        assert!(!restore_in_progress(&collection));
    }

    #[test]
    fn ensure_restore_not_blocked_reports_pending_finalize_progress_and_integrity_failures() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn, "work", Path::new("vault"));
        let base = load_collection_by_id(&conn, collection_id).unwrap();

        let mut integrity_blocked = base.clone();
        integrity_blocked.state = CollectionState::Restoring;
        integrity_blocked.pending_root_path = Some("D:\\restored".to_owned());
        integrity_blocked.integrity_failed_at = Some("2026-04-28T00:00:00Z".to_owned());
        assert!(matches!(
            ensure_restore_not_blocked(&integrity_blocked).unwrap_err(),
            VaultSyncError::RestoreIntegrityBlocked {
                blocking_column: "integrity_failed_at",
                ..
            }
        ));

        let mut manifest_blocked = base.clone();
        manifest_blocked.state = CollectionState::Restoring;
        manifest_blocked.pending_root_path = Some("D:\\restored".to_owned());
        manifest_blocked.pending_manifest_incomplete_at = Some("2026-04-28T00:00:00Z".to_owned());
        assert!(matches!(
            ensure_restore_not_blocked(&manifest_blocked).unwrap_err(),
            VaultSyncError::RestoreIntegrityBlocked {
                blocking_column: "pending_manifest_incomplete_at",
                ..
            }
        ));

        let mut pending_finalize = base.clone();
        pending_finalize.state = CollectionState::Restoring;
        pending_finalize.pending_root_path = Some("D:\\restored".to_owned());
        let pending_error = ensure_restore_not_blocked(&pending_finalize)
            .unwrap_err()
            .to_string();
        assert!(pending_error.contains("RestorePendingFinalizeError"));
        assert!(pending_error.contains("D:\\restored"));

        let mut in_progress = base.clone();
        in_progress.state = CollectionState::Restoring;
        let progress_error = ensure_restore_not_blocked(&in_progress)
            .unwrap_err()
            .to_string();
        assert!(progress_error.contains("RestoreInProgressError"));

        let mut active_integrity = base.clone();
        active_integrity.integrity_failed_at = Some("2026-04-28T00:00:00Z".to_owned());
        assert!(matches!(
            ensure_restore_not_blocked(&active_integrity).unwrap_err(),
            VaultSyncError::RestoreIntegrityBlocked {
                blocking_column: "integrity_failed_at",
                ..
            }
        ));

        let mut active_manifest = base.clone();
        active_manifest.pending_manifest_incomplete_at = Some("2026-04-28T00:00:00Z".to_owned());
        assert!(matches!(
            ensure_restore_not_blocked(&active_manifest).unwrap_err(),
            VaultSyncError::RestoreIntegrityBlocked {
                blocking_column: "pending_manifest_incomplete_at",
                ..
            }
        ));

        assert!(ensure_restore_not_blocked(&base).is_ok());
    }

    #[test]
    fn ensure_restore_target_is_empty_accepts_missing_and_empty_paths_only() {
        let root = tempfile::TempDir::new().unwrap();
        let missing = root.path().join("missing");
        assert!(ensure_restore_target_is_empty(&missing).is_ok());

        let empty_dir = root.path().join("empty");
        fs::create_dir_all(&empty_dir).unwrap();
        assert!(ensure_restore_target_is_empty(&empty_dir).is_ok());

        let file_target = root.path().join("file.txt");
        fs::write(&file_target, b"occupied").unwrap();
        let file_error = ensure_restore_target_is_empty(&file_target)
            .unwrap_err()
            .to_string();
        assert!(file_error.contains("RestoreNonEmptyTargetError"));

        let busy_dir = root.path().join("busy");
        fs::create_dir_all(&busy_dir).unwrap();
        fs::write(busy_dir.join("child.txt"), b"occupied").unwrap();
        let busy_error = ensure_restore_target_is_empty(&busy_dir)
            .unwrap_err()
            .to_string();
        assert!(busy_error.contains("RestoreNonEmptyTargetError"));
    }

    #[test]
    fn infer_restore_relative_path_prefers_relative_inputs_and_strips_collection_root() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        #[cfg(unix)]
        let foreign_raw_path = "/elsewhere/foreign.md";
        #[cfg(not(unix))]
        let foreign_raw_path = r"D:\elsewhere\foreign.md";

        assert_eq!(
            infer_restore_relative_path(
                &collection,
                "notes/example",
                Some("ignored.md"),
                Some("notes/explicit.md")
            ),
            PathBuf::from("notes/explicit.md")
        );
        assert_eq!(
            infer_restore_relative_path(&collection, "notes/example", Some("notes/raw.md"), None),
            PathBuf::from("notes/raw.md")
        );
        assert_eq!(
            infer_restore_relative_path(
                &collection,
                "notes/example",
                Some(
                    root.path()
                        .join("notes")
                        .join("absolute.md")
                        .to_string_lossy()
                        .as_ref()
                ),
                None
            ),
            PathBuf::from("notes").join("absolute.md")
        );
        assert_eq!(
            infer_restore_relative_path(&collection, "notes/example", Some(foreign_raw_path), None),
            PathBuf::from("notes/example.md")
        );
        assert_eq!(
            infer_restore_relative_path(&collection, "notes/example", None, None),
            PathBuf::from("notes/example.md")
        );
    }

    #[test]
    fn materialize_collection_to_path_replaces_existing_tree_and_writes_nested_files() {
        let conn = open_test_db();
        let source_root = tempfile::TempDir::new().unwrap();
        let output_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", source_root.path());
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
            "nested/notes/a.md",
        );
        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let materialized = output_root.path().join("materialized");
        fs::create_dir_all(&materialized).unwrap();
        fs::write(materialized.join("stale.txt"), b"stale").unwrap();

        materialize_collection_to_path(&conn, &collection, &materialized).unwrap();

        assert!(!materialized.join("stale.txt").exists());
        assert_eq!(
            fs::read(materialized.join("nested").join("notes").join("a.md")).unwrap(),
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a"
        );
    }

    #[test]
    fn materialize_collection_to_path_rejects_missing_active_raw_imports() {
        let conn = open_test_db();
        let source_root = tempfile::TempDir::new().unwrap();
        let output_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", source_root.path());
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES (?1, 'notes/missing', ?2, 'concept', 'missing', '', 'compiled', '', '{}', '', '', 1)",
            params![collection_id, Uuid::now_v7().to_string()],
        )
        .unwrap();
        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let error =
            materialize_collection_to_path(&conn, &collection, output_root.path()).unwrap_err();

        assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
        assert!(error.to_string().contains("missing active raw_imports"));
    }

    #[test]
    fn verify_remap_root_rejects_invalid_quaidignore_in_new_root() {
        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        let raw_bytes =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            raw_bytes,
            "notes/a.md",
        );
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::write(new_root.path().join("notes").join("a.md"), raw_bytes).unwrap();
        fs::write(new_root.path().join(".quaidignore"), "[broken\n").unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();

        assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
        assert!(error
            .to_string()
            .contains("invalid .quaidignore in remap root"));
    }

    #[test]
    fn restore_reset_reports_each_block_reason_before_terminal_reset() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());

        conn.execute(
            "UPDATE collections
             SET pending_manifest_incomplete_at = '2026-04-28T00:00:00Z'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let manifest_error = restore_reset(&conn, "work").unwrap_err().to_string();
        assert!(manifest_error.contains("RestoreResetBlockedError"));
        assert!(manifest_error.contains("manifest_incomplete_retryable"));

        conn.execute(
            "UPDATE collections
             SET pending_manifest_incomplete_at = NULL,
                 state = 'restoring',
                 pending_root_path = 'D:\\restored'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let pending_error = restore_reset(&conn, "work").unwrap_err().to_string();
        assert!(pending_error.contains("RestoreResetBlockedError"));
        assert!(pending_error.contains("pending_finalize"));

        conn.execute(
            "UPDATE collections
             SET pending_root_path = NULL,
                 state = 'restoring',
                 needs_full_sync = 1
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let progress_error = restore_reset(&conn, "work").unwrap_err().to_string();
        assert!(progress_error.contains("RestoreResetBlockedError"));
        assert!(progress_error.contains("restore_in_progress"));

        conn.execute(
            "UPDATE collections
             SET state = 'active',
                 needs_full_sync = 0
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let clean_error = restore_reset(&conn, "work").unwrap_err().to_string();
        assert!(clean_error.contains("RestoreResetBlockedError"));
        assert!(clean_error.contains("no_integrity_failure"));
    }

    #[test]
    fn reconcile_reset_reports_missing_collection() {
        assert!(matches!(
            reconcile_reset(&open_test_db(), "missing"),
            Err(VaultSyncError::CollectionNotFound { name }) if name == "missing"
        ));
    }

    #[test]
    fn restore_reset_can_return_collection_to_detached_and_finalize_covers_remaining_outcomes() {
        let conn = open_test_db();
        let placeholder_id = insert_collection(&conn, "placeholder", Path::new("vault"));
        conn.execute(
            "UPDATE collections
             SET root_path = '',
                 state = 'restoring',
                 integrity_failed_at = '2026-04-28T00:00:00Z'
             WHERE id = ?1",
            [placeholder_id],
        )
        .unwrap();
        restore_reset(&conn, "placeholder").unwrap();
        let placeholder = load_collection_by_id(&conn, placeholder_id).unwrap();
        assert_eq!(placeholder.state, CollectionState::Detached);
        assert_eq!(placeholder.root_path, "");

        let no_pending_id = insert_collection(&conn, "no-pending", Path::new("vault-no-pending"));
        let no_pending = finalize_pending_restore(
            &conn,
            no_pending_id,
            FinalizeCaller::ExternalFinalize {
                session_id: "serve-1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(no_pending, FinalizeOutcome::NoPendingWork);

        let orphan_id = insert_collection(&conn, "orphan", Path::new("vault-orphan"));
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 restore_command_id = 'restore-1',
                 pending_root_path = NULL
             WHERE id = ?1",
            [orphan_id],
        )
        .unwrap();
        let orphan = finalize_pending_restore(
            &conn,
            orphan_id,
            FinalizeCaller::ExternalFinalize {
                session_id: "serve-1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(orphan, FinalizeOutcome::OrphanRecovered);

        let aborted_id = insert_collection(&conn, "aborted", Path::new("vault-aborted"));
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = 'D:\\missing-restore-root',
                 pending_restore_manifest = '{\"entries\":[]}',
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = datetime('now', '-120 seconds')
             WHERE id = ?1",
            [aborted_id],
        )
        .unwrap();
        let aborted = finalize_pending_restore(
            &conn,
            aborted_id,
            FinalizeCaller::ExternalFinalize {
                session_id: "serve-1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(aborted, FinalizeOutcome::Aborted);
    }

    #[cfg(unix)]
    fn startup_recovery_sentinel_count(recovery_root: &Path, collection_id: i64) -> usize {
        recovery_sentinel_paths(recovery_root, collection_id)
            .unwrap()
            .len()
    }

    #[cfg(unix)]
    fn writer_side_sentinel_path(
        recovery_root: &Path,
        collection_id: i64,
        write_id: &str,
    ) -> PathBuf {
        collection_recovery_dir(recovery_root, collection_id)
            .join(writer_side_sentinel_name(write_id))
    }

    #[cfg(unix)]
    fn writer_side_tempfile_path(root: &Path, relative_path: &Path, write_id: &str) -> PathBuf {
        let mut path = root.join(relative_path);
        path.set_file_name(writer_side_tempfile_name(write_id));
        path
    }

    #[cfg(unix)]
    fn insert_page_with_actual_file_state(
        conn: &Connection,
        collection_id: i64,
        root: &Path,
        slug: &str,
        relative_path: &str,
        raw_bytes: &[u8],
    ) -> i64 {
        write_restore_file(root, relative_path, raw_bytes);
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES (?1, ?2, ?3, 'concept', ?2, '', ?4, '', '{}', '', '', 1)",
            params![collection_id, slug, Uuid::now_v7().to_string(), String::from_utf8_lossy(raw_bytes)],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();
        let relative = Path::new(relative_path);
        let root_fd = fs_safety::open_root_fd(root).unwrap();
        let parent_fd = fs_safety::walk_to_parent(&root_fd, relative).unwrap();
        let stat =
            file_state::stat_file_fd(&parent_fd, relative.file_name().unwrap().as_ref()).unwrap();
        file_state::upsert_file_state(
            conn,
            collection_id,
            relative_path,
            page_id,
            &stat,
            &sha256_hex(raw_bytes),
        )
        .unwrap();
        page_id
    }

    #[cfg(unix)]
    fn stored_file_state(
        conn: &Connection,
        collection_id: i64,
        relative_path: &str,
    ) -> file_state::FileStateRow {
        file_state::get_file_state(conn, collection_id, relative_path)
            .unwrap()
            .unwrap()
    }

    #[cfg(unix)]
    fn actual_file_stat(root: &Path, relative_path: &str) -> file_state::FileStat {
        let relative = Path::new(relative_path);
        let root_fd = fs_safety::open_root_fd(root).unwrap();
        let parent_fd = fs_safety::walk_to_parent(&root_fd, relative).unwrap();
        file_state::stat_file_fd(&parent_fd, relative.file_name().unwrap().as_ref()).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn check_fs_precondition_returns_fast_path_when_all_four_stat_fields_match() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        insert_page_with_actual_file_state(
            &conn,
            collection_id,
            root.path(),
            "notes/fast",
            "notes/fast.md",
            b"fast path bytes",
        );

        let outcome = check_fs_precondition(
            &conn,
            collection_id,
            root.path(),
            Path::new("notes/fast.md"),
        )
        .unwrap();

        assert_eq!(outcome, FsPreconditionOutcome::FastPath);
    }

    #[cfg(unix)]
    #[test]
    fn check_fs_precondition_self_heals_ctime_only_drift_when_hash_matches() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        insert_page_with_actual_file_state(
            &conn,
            collection_id,
            root.path(),
            "notes/self-heal",
            "notes/self-heal.md",
            b"same bytes",
        );
        let path = root.path().join("notes").join("self-heal.md");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let before = stored_file_state(&conn, collection_id, "notes/self-heal.md");
        let current = actual_file_stat(root.path(), "notes/self-heal.md");
        assert_eq!(current.mtime_ns, before.mtime_ns);
        assert_eq!(current.size_bytes, before.size_bytes);
        assert_eq!(current.inode, before.inode);
        assert_ne!(current.ctime_ns, before.ctime_ns);

        let outcome = check_fs_precondition(
            &conn,
            collection_id,
            root.path(),
            Path::new("notes/self-heal.md"),
        )
        .unwrap();

        let after = stored_file_state(&conn, collection_id, "notes/self-heal.md");
        assert_eq!(outcome, FsPreconditionOutcome::SlowPathSelfHeal);
        assert_eq!(after.mtime_ns, current.mtime_ns);
        assert_eq!(after.ctime_ns, current.ctime_ns);
        assert_eq!(after.size_bytes, current.size_bytes);
        assert_eq!(after.inode, current.inode);
        assert_eq!(after.sha256, before.sha256);
    }

    #[cfg(unix)]
    #[test]
    fn check_fs_precondition_returns_hash_conflict_on_content_drift() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        insert_page_with_actual_file_state(
            &conn,
            collection_id,
            root.path(),
            "notes/hash-conflict",
            "notes/hash-conflict.md",
            b"old bytes",
        );
        std::fs::write(
            root.path().join("notes").join("hash-conflict.md"),
            b"new bytes",
        )
        .unwrap();

        let error = check_fs_precondition(
            &conn,
            collection_id,
            root.path(),
            Path::new("notes/hash-conflict.md"),
        )
        .unwrap_err();

        assert!(matches!(error, VaultSyncError::HashMismatch { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn check_fs_precondition_catches_same_size_rewrite_by_ctime_slow_path() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        insert_page_with_actual_file_state(
            &conn,
            collection_id,
            root.path(),
            "notes/ctime-rewrite",
            "notes/ctime-rewrite.md",
            &[b'a'; 32],
        );
        let path = root.path().join("notes").join("ctime-rewrite.md");
        let original_modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            use std::io::Write;
            file.write_all(&[b'b'; 32]).unwrap();
            file.sync_all().unwrap();
            let times = std::fs::FileTimes::new().set_modified(original_modified);
            file.set_times(times).unwrap();
        }

        let before = stored_file_state(&conn, collection_id, "notes/ctime-rewrite.md");
        let current = actual_file_stat(root.path(), "notes/ctime-rewrite.md");
        assert_eq!(current.mtime_ns, before.mtime_ns);
        assert_eq!(current.size_bytes, before.size_bytes);
        assert_eq!(current.inode, before.inode);
        assert_ne!(current.ctime_ns, before.ctime_ns);

        let error = check_fs_precondition(
            &conn,
            collection_id,
            root.path(),
            Path::new("notes/ctime-rewrite.md"),
        )
        .unwrap_err();

        assert!(matches!(error, VaultSyncError::HashMismatch { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn writer_side_sentinel_create_failure_leaves_no_tempfile_dedup_or_db_mutation() {
        let (_dir, _db_path, conn) = open_test_db_file();
        init_process_registries().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let relative_path = Path::new("notes/a.md");
        let write_id = "writer-sentinel-create";
        let recovery_root = recovery_root_for_db_path(Path::new(&database_path(&conn).unwrap()));
        let sentinel_path = writer_side_sentinel_path(&recovery_root, collection_id, write_id);
        let tempfile_path = writer_side_tempfile_path(root.path(), relative_path, write_id);
        let bytes = b"writer bytes that must never reach disk";
        let dedup_key = writer_side_dedup_key(&root.path().join(relative_path), bytes);

        let error = exercise_writer_side_sentinel_crash_core(
            &conn,
            collection_id,
            relative_path,
            bytes,
            write_id,
            &WriterSideSentinelCrashMode::SentinelCreateFail,
        )
        .unwrap_err();

        assert!(matches!(error, VaultSyncError::RecoverySentinel { .. }));
        assert!(!sentinel_path.exists());
        assert!(!tempfile_path.exists());
        assert!(!root.path().join(relative_path).exists());
        assert!(!writer_side_dedup_contains(&dedup_key));
        let row: (i64, i64, i64) = conn
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM pages WHERE collection_id = ?1),
                     (SELECT COUNT(*) FROM raw_imports),
                     (SELECT needs_full_sync FROM collections WHERE id = ?1)",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, 0);
        assert_eq!(row.1, 0);
        assert_eq!(row.2, 0);
    }

    #[cfg(unix)]
    #[test]
    fn writer_side_pre_rename_abort_cleans_tempfile_dedup_and_sentinel() {
        let (_dir, _db_path, conn) = open_test_db_file();
        init_process_registries().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let relative_path = Path::new("notes/a.md");
        let write_id = "writer-pre-rename";
        let recovery_root = recovery_root_for_db_path(Path::new(&database_path(&conn).unwrap()));
        let sentinel_path = writer_side_sentinel_path(&recovery_root, collection_id, write_id);
        let tempfile_path = writer_side_tempfile_path(root.path(), relative_path, write_id);
        let target_path = root.path().join(relative_path);
        let bytes = b"writer bytes that must roll back before rename";
        let dedup_key = writer_side_dedup_key(&target_path, bytes);

        let error = exercise_writer_side_sentinel_crash_core(
            &conn,
            collection_id,
            relative_path,
            bytes,
            write_id,
            &WriterSideSentinelCrashMode::PreRenameAbortAfterDedup,
        )
        .unwrap_err();

        assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
        assert!(!sentinel_path.exists());
        assert!(!tempfile_path.exists());
        assert!(!target_path.exists());
        assert!(!writer_side_dedup_contains(&dedup_key));
        let row: (i64, i64, i64) = conn
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM pages WHERE collection_id = ?1),
                     (SELECT COUNT(*) FROM raw_imports),
                     (SELECT needs_full_sync FROM collections WHERE id = ?1)",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, 0);
        assert_eq!(row.1, 0);
        assert_eq!(row.2, 0);
    }

    #[cfg(unix)]
    #[test]
    fn writer_side_rename_failure_cleans_tempfile_dedup_and_sentinel_without_touching_target() {
        let (_dir, _db_path, conn) = open_test_db_file();
        init_process_registries().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let relative_path = Path::new("notes/a.md");
        let write_id = "writer-rename-fail";
        let recovery_root = recovery_root_for_db_path(Path::new(&database_path(&conn).unwrap()));
        let sentinel_path = writer_side_sentinel_path(&recovery_root, collection_id, write_id);
        let tempfile_path = writer_side_tempfile_path(root.path(), relative_path, write_id);
        let target_path = root.path().join(relative_path);
        write_restore_file(root.path(), "notes/a.md", b"old on-disk bytes");
        let bytes = b"new bytes that never finish rename";
        let dedup_key = writer_side_dedup_key(&target_path, bytes);

        let error = exercise_writer_side_sentinel_crash_core(
            &conn,
            collection_id,
            relative_path,
            bytes,
            write_id,
            &WriterSideSentinelCrashMode::RenameFail,
        )
        .unwrap_err();

        assert!(matches!(error, VaultSyncError::Io(_)));
        assert!(!sentinel_path.exists());
        assert!(!tempfile_path.exists());
        assert_eq!(fs::read(&target_path).unwrap(), b"old on-disk bytes");
        assert!(!writer_side_dedup_contains(&dedup_key));
        let row: (i64, i64, i64) = conn
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM pages WHERE collection_id = ?1),
                     (SELECT COUNT(*) FROM raw_imports),
                     (SELECT needs_full_sync FROM collections WHERE id = ?1)",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, 0);
        assert_eq!(row.1, 0);
        assert_eq!(row.2, 0);
    }

    #[cfg(unix)]
    #[test]
    fn writer_side_post_rename_fsync_abort_retains_sentinel_removes_dedup_and_marks_full_sync() {
        let (_dir, db_path, conn) = open_test_db_file();
        init_process_registries().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let relative_path = Path::new("notes/a.md");
        let write_id = "writer-fsync-parent";
        let recovery_root = recovery_root_for_db_path(Path::new(&database_path(&conn).unwrap()));
        let sentinel_path = writer_side_sentinel_path(&recovery_root, collection_id, write_id);
        let tempfile_path = writer_side_tempfile_path(root.path(), relative_path, write_id);
        let target_path = root.path().join(relative_path);
        let bytes = b"new bytes that landed before the post-rename abort";
        let dedup_key = writer_side_dedup_key(&target_path, bytes);

        let error = exercise_writer_side_sentinel_crash_core(
            &conn,
            collection_id,
            relative_path,
            bytes,
            write_id,
            &WriterSideSentinelCrashMode::FsyncParentFail,
        )
        .unwrap_err();

        assert!(matches!(error, VaultSyncError::Durability { .. }));
        assert!(sentinel_path.exists());
        assert!(!tempfile_path.exists());
        assert_eq!(fs::read(&target_path).unwrap(), bytes);
        assert!(!writer_side_dedup_contains(&dedup_key));
        let verify = Connection::open(&db_path).unwrap();
        let row: (i64, i64, i64) = verify
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM pages WHERE collection_id = ?1),
                     (SELECT COUNT(*) FROM raw_imports),
                     (SELECT needs_full_sync FROM collections WHERE id = ?1)",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, 0);
        assert_eq!(row.1, 0);
        assert_eq!(row.2, 1);
    }

    #[cfg(unix)]
    #[test]
    fn insert_write_dedup_rejects_duplicate_key() {
        init_process_registries().unwrap();
        let key = format!(
            "test-duplicate-dedup-{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        insert_write_dedup(&key).unwrap();
        let error = insert_write_dedup(&key).unwrap_err();

        assert!(matches!(
            error,
            VaultSyncError::DuplicateWriteDedup { key: duplicate } if duplicate == key
        ));
        assert!(has_write_dedup(&key).unwrap());
        remove_write_dedup(&key).unwrap();
        assert!(!has_write_dedup(&key).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn self_write_dedup_suppresses_recent_matching_path_and_hash_only() {
        init_process_registries().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("notes").join("a.md");
        write_restore_file(root.path(), "notes/a.md", b"matching bytes");
        remember_self_write_path_at(
            &path,
            &sha256_hex(b"matching bytes"),
            Instant::now() - Duration::from_secs(1),
        )
        .unwrap();

        assert!(self_write_should_suppress_at(
            &path,
            &sha256_hex(b"matching bytes"),
            Instant::now()
        )
        .unwrap());

        assert!(!self_write_should_suppress_at(
            &path,
            &sha256_hex(b"different bytes"),
            Instant::now()
        )
        .unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn self_write_dedup_does_not_suppress_after_ttl_and_sweeps_expired_entries() {
        init_process_registries().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("notes").join("a.md");
        write_restore_file(root.path(), "notes/a.md", b"matching bytes");
        let stale_now = Instant::now();
        remember_self_write_path_at(
            &path,
            &sha256_hex(b"matching bytes"),
            stale_now - self_write_dedup_ttl() - Duration::from_millis(1),
        )
        .unwrap();

        assert!(
            !self_write_should_suppress_at(&path, &sha256_hex(b"matching bytes"), stale_now)
                .unwrap()
        );
        assert_eq!(sweep_expired_self_write_entries_at(stale_now).unwrap(), 1);
        assert_eq!(sweep_expired_self_write_entries_at(stale_now).unwrap(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn classify_watch_event_only_suppresses_rename_when_source_is_not_markdown_or_is_self_write() {
        init_process_registries().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let source_path = root.path().join("notes").join("from.md");
        let target_path = root.path().join("notes").join("to.md");
        let temp_path = root.path().join(".quaid-write-temp.tmp");
        let bytes = b"matching bytes";
        write_restore_file(root.path(), "notes/to.md", bytes);
        remember_self_write_path_at(&target_path, &sha256_hex(bytes), Instant::now()).unwrap();

        let external_rename = NotifyEvent {
            kind: NotifyEventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Both)),
            paths: vec![source_path.clone(), target_path.clone()],
            attrs: Default::default(),
        };
        let actions = classify_watch_event(root.path(), external_rename).unwrap();
        assert_eq!(actions.len(), 3);
        assert!(matches!(
            &actions[0],
            WatchEvent::NativeRename(rename)
                if rename.from_path == Path::new("notes/from.md")
                    && rename.to_path == Path::new("notes/to.md")
        ));
        assert!(matches!(
            &actions[1],
            WatchEvent::DirtyPath(path) if path == &PathBuf::from("notes/from.md")
        ));
        assert!(matches!(
            &actions[2],
            WatchEvent::DirtyPath(path) if path == &PathBuf::from("notes/to.md")
        ));

        let self_write_rename = NotifyEvent {
            kind: NotifyEventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Both)),
            paths: vec![temp_path, target_path],
            attrs: Default::default(),
        };
        assert!(classify_watch_event(root.path(), self_write_rename)
            .unwrap()
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_watcher_reconciles_external_edit_after_debounce() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        write_restore_file(
            root.path(),
            "notes/a.md",
            b"---\ntitle: A\ntype: note\n---\nOriginal body.\n",
        );
        sync_collection(&conn, "work").unwrap();
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();

        write_restore_file(
            root.path(),
            "notes/a.md",
            b"---\ntitle: A\ntype: note\n---\nUpdated by watcher.\n",
        );

        let compiled_truth = wait_for_collection_update(
            &db_path,
            collection_id,
            Duration::from_secs(8),
            |verify, collection_id| {
                verify
                    .query_row(
                        "SELECT compiled_truth
                         FROM pages
                         WHERE collection_id = ?1 AND slug = 'notes/a'",
                        [collection_id],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
                    .and_then(|compiled_truth| {
                        compiled_truth
                            .contains("Updated by watcher.")
                            .then_some(compiled_truth)
                    })
            },
        );
        assert!(compiled_truth.contains("Updated by watcher."));

        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_watcher_rejects_path_only_dedup_match() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let path = root.path().join("notes").join("a.md");
        write_restore_file(
            root.path(),
            "notes/a.md",
            b"---\ntitle: A\ntype: note\n---\nOriginal body.\n",
        );
        sync_collection(&conn, "work").unwrap();
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();
        remember_self_write_path_at(
            &path,
            &sha256_hex(b"---\ntitle: A\ntype: note\n---\nDifferent self-write body.\n"),
            Instant::now(),
        )
        .unwrap();
        write_restore_file(
            root.path(),
            "notes/a.md",
            b"---\ntitle: A\ntype: note\n---\nPath-only mismatch should ingest.\n",
        );

        let compiled_truth = wait_for_collection_update(
            &db_path,
            collection_id,
            Duration::from_secs(8),
            |verify, collection_id| {
                verify
                    .query_row(
                        "SELECT compiled_truth
                         FROM pages
                         WHERE collection_id = ?1 AND slug = 'notes/a'",
                        [collection_id],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
                    .and_then(|compiled_truth| {
                        compiled_truth
                            .contains("Path-only mismatch should ingest.")
                            .then_some(compiled_truth)
                    })
            },
        );
        assert!(compiled_truth.contains("Path-only mismatch should ingest."));

        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_watcher_ignores_stale_dedup_entries_after_ttl() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let path = root.path().join("notes").join("a.md");
        let updated_bytes = b"---\ntitle: A\ntype: note\n---\nStale dedup must not block ingest.\n";
        write_restore_file(
            root.path(),
            "notes/a.md",
            b"---\ntitle: A\ntype: note\n---\nOriginal body.\n",
        );
        sync_collection(&conn, "work").unwrap();
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();
        remember_self_write_path_at(
            &path,
            &sha256_hex(updated_bytes),
            Instant::now() - self_write_dedup_ttl() - Duration::from_millis(1),
        )
        .unwrap();
        write_restore_file(root.path(), "notes/a.md", updated_bytes);

        let compiled_truth = wait_for_collection_update(
            &db_path,
            collection_id,
            Duration::from_secs(8),
            |verify, collection_id| {
                verify
                    .query_row(
                        "SELECT compiled_truth
                         FROM pages
                         WHERE collection_id = ?1 AND slug = 'notes/a'",
                        [collection_id],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
                    .and_then(|compiled_truth| {
                        compiled_truth
                            .contains("Stale dedup must not block ingest.")
                            .then_some(compiled_truth)
                    })
            },
        );
        assert!(compiled_truth.contains("Stale dedup must not block ingest."));

        drop(runtime);
    }

    #[test]
    fn sync_collection_watchers_production_logic_stays_active_only_and_generation_aware() {
        let source = production_vault_sync_source();
        let start = source.find("fn sync_collection_watchers(").unwrap();
        let end = source[start..]
            .find("fn run_watcher_reconcile(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("detach_active_collections_with_empty_root_path(conn)?;"),
            "watcher sync must normalize empty root paths before watching: {snippet}"
        );
        assert!(
            snippet.contains("WHERE state = 'active'"),
            "watcher sync must only enumerate active collections: {snippet}"
        );
        assert!(
            snippet
                .contains("watchers.retain(|collection_id, _| active.contains_key(collection_id))"),
            "watcher sync must drop watchers for non-active collections: {snippet}"
        );
        assert!(
            snippet.contains("state.root_path != root_path")
                && snippet.contains("state.generation != generation"),
            "watcher sync must replace watchers when root or reload_generation changes: {snippet}"
        );
        assert!(
            snippet.contains("state.generation = generation;"),
            "replacement watchers must inherit the new reload_generation: {snippet}"
        );
    }

    #[test]
    fn run_overflow_recovery_pass_production_logic_uses_active_lease_and_active_gate() {
        let source = production_vault_sync_source();
        let start = source.find("fn run_overflow_recovery_pass(").unwrap();
        let end = source[start..]
            .find("pub fn start_serve_runtime(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("WHERE state = 'active' AND needs_full_sync = 1"),
            "overflow recovery must gate itself to active collections: {snippet}"
        );
        assert!(
            snippet.contains("FullHashReconcileMode::OverflowRecovery"),
            "overflow recovery must use the repaired mode label: {snippet}"
        );
        assert!(
            snippet.contains("FullHashReconcileAuthorization::ActiveLease"),
            "overflow recovery must reuse the active lease authorization: {snippet}"
        );
        assert!(
            snippet.contains("overflow_recovery_skipped_lease_mismatch"),
            "overflow recovery must warn on lease mismatch rather than bypassing ownership: {snippet}"
        );
    }

    #[test]
    fn start_serve_runtime_logs_scheduled_maintenance_failures() {
        let source = production_vault_sync_source();
        let start = source.find("pub fn start_serve_runtime(").unwrap();
        let end = source[start..]
            .find("pub fn begin_restore(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("WARN: scheduled_full_hash_audit_failed")
                && snippet.contains("if let Err(error) = run_full_hash_audit_pass(&conn, &session_id_for_thread)")
                && snippet.contains("WARN: raw_import_ttl_sweep_failed")
                && snippet.contains("if let Err(error) = sweep_raw_import_ttl(&conn)"),
            "serve loop must log scheduled audit and TTL sweep failures instead of discarding them silently"
        );
    }

    #[test]
    fn start_collection_watcher_production_logic_keeps_native_first_poll_fallback() {
        let source = production_vault_sync_source();
        let start = source.find("fn start_collection_watcher(").unwrap();
        let end = source[start..]
            .find("fn sync_collection_watchers(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("watcher_native_init_failed"),
            "native watcher failures must warn before fallback: {snippet}"
        );
        assert!(
            snippet.contains("PollWatcher::new"),
            "native watcher failures must fall back to poll mode: {snippet}"
        );
        assert!(
            snippet.contains("WatcherMode::Poll"),
            "fallback path must record poll mode explicitly: {snippet}"
        );
    }

    #[test]
    fn watcher_supervisor_production_logic_tracks_crash_state_and_backoff() {
        let source = production_vault_sync_source();
        let start = source.find("fn mark_watcher_crashed(").unwrap();
        let end = source[start..]
            .find("fn publish_watcher_health(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("state.mode = WatcherMode::Crashed;"),
            "watcher crashes must be surfaced as an explicit crashed mode: {snippet}"
        );
        assert!(
            snippet.contains("state.backoff_until = Some(now + backoff);"),
            "watcher crashes must record restart backoff: {snippet}"
        );
        assert!(
            snippet.contains("watcher_backoff_duration"),
            "watcher crashes must use exponential backoff helper: {snippet}"
        );
    }

    #[test]
    fn detach_active_collections_with_empty_root_path_normalizes_default_collection() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let work_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET state = 'active', root_path = '' WHERE id = 1",
            [],
        )
        .unwrap();

        let detached = detach_active_collections_with_empty_root_path(&conn).unwrap();
        assert_eq!(detached, vec![1]);

        let active_ids = conn
            .prepare("SELECT id FROM collections WHERE state = 'active' ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(active_ids, vec![work_id]);

        let default_state: String = conn
            .query_row("SELECT state FROM collections WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(default_state, "detached");
    }

    /// Regression test for issue #81.
    ///
    /// `quaid serve` must not attempt to watch an empty collection root.
    #[cfg(unix)]
    #[test]
    fn sync_collection_watchers_skips_active_collection_with_empty_root_path() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let work_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET state = 'active', root_path = '' WHERE id = 1",
            [],
        )
        .unwrap();

        let mut watchers = HashMap::new();
        sync_collection_watchers(&conn, &db_path, &mut watchers).unwrap();

        assert!(watchers.contains_key(&work_id));
        assert!(!watchers.contains_key(&1));

        let default_state: String = conn
            .query_row("SELECT state FROM collections WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(default_state, "detached");
    }

    #[cfg(unix)]
    #[test]
    fn start_collection_watcher_falls_back_to_poll_mode_when_native_init_fails() {
        set_force_native_watcher_init_failure(true);
        let root = tempfile::TempDir::new().unwrap();

        let state = start_collection_watcher(7, root.path(), ":memory:").unwrap();

        set_force_native_watcher_init_failure(false);
        assert_eq!(state.mode, WatcherMode::Poll);
        assert!(matches!(state.watcher, Some(WatcherHandle::Poll(_))));
    }

    #[cfg(unix)]
    #[test]
    fn sync_collection_watchers_preserves_crashed_watcher_until_backoff_expires() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let (_sender, receiver) = mpsc::channel(1);
        let backoff_until = Instant::now() + Duration::from_secs(5);
        let crashed = CollectionWatcherState {
            root_path: root.path().to_path_buf(),
            generation: 0,
            receiver,
            watcher: None,
            buffer: WatchBatchBuffer::default(),
            mode: WatcherMode::Crashed,
            last_event_at: Some("2026-04-28T00:00:00Z".to_owned()),
            last_watcher_error: None,
            backoff_until: Some(backoff_until),
            consecutive_failures: 2,
        };
        let mut watchers = HashMap::from([(collection_id, crashed)]);

        sync_collection_watchers(&conn, &db_path, &mut watchers).unwrap();

        let retained = watchers.get(&collection_id).unwrap();
        assert_eq!(retained.mode, WatcherMode::Crashed);
        assert!(retained.watcher.is_none());
        assert_eq!(
            retained.last_event_at.as_deref(),
            Some("2026-04-28T00:00:00Z")
        );
        assert_eq!(retained.backoff_until, Some(backoff_until));
        assert_eq!(retained.consecutive_failures, 2);
    }

    #[cfg(unix)]
    #[test]
    fn poll_collection_watcher_marks_crashed_state_and_sync_restarts_after_backoff() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let (_sender, receiver) = mpsc::channel(1);
        drop(_sender);
        let watcher = notify::recommended_watcher(|_| {}).unwrap();
        let mut state = CollectionWatcherState {
            root_path: root.path().to_path_buf(),
            generation: 0,
            receiver,
            watcher: Some(WatcherHandle::Native(watcher)),
            buffer: WatchBatchBuffer::default(),
            mode: WatcherMode::Native,
            last_event_at: None,
            last_watcher_error: None,
            backoff_until: None,
            consecutive_failures: 0,
        };

        let error = poll_collection_watcher(&conn, collection_id, &mut state).unwrap_err();
        assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
        let backoff = mark_watcher_crashed(collection_id, &mut state);
        assert_eq!(state.mode, WatcherMode::Crashed);
        assert_eq!(backoff, Duration::from_secs(1));
        assert!(state.backoff_until.is_some());

        state.backoff_until = Some(Instant::now() - Duration::from_millis(1));
        let mut watchers = HashMap::from([(collection_id, state)]);
        sync_collection_watchers(&conn, &db_path, &mut watchers).unwrap();
        let restarted = watchers.get(&collection_id).unwrap();
        assert!(matches!(
            restarted.mode,
            WatcherMode::Native | WatcherMode::Poll
        ));
        assert!(!matches!(restarted.mode, WatcherMode::Crashed));
    }

    #[cfg(unix)]
    #[test]
    fn publish_watcher_health_updates_only_matching_session_handles() {
        init_process_registries().unwrap();
        register_supervisor_handle(41, "serve-1", 3).unwrap();
        register_supervisor_handle(42, "foreign-session", 7).unwrap();
        let (_sender, receiver) = mpsc::channel(2);
        let watchers = HashMap::from([(
            41,
            CollectionWatcherState {
                root_path: PathBuf::from("/vault"),
                generation: 3,
                receiver,
                watcher: None,
                buffer: WatchBatchBuffer::default(),
                mode: WatcherMode::Poll,
                last_event_at: Some("2026-04-28T00:00:00Z".to_owned()),
                last_watcher_error: None,
                backoff_until: None,
                consecutive_failures: 0,
            },
        )]);

        publish_watcher_health("serve-1", &watchers).unwrap();

        let health = collection_watcher_health(41).unwrap();
        assert_eq!(health.mode, "poll");
        assert_eq!(
            health.last_event_at.as_deref(),
            Some("2026-04-28T00:00:00Z")
        );
        assert_eq!(health.channel_depth, 0);
        assert!(collection_watcher_health(42).is_none());
        clear_collection_watcher_health_for_test(41);
        clear_collection_watcher_health_for_test(42);
    }

    #[cfg(unix)]
    #[test]
    fn run_overflow_recovery_pass_clears_needs_full_sync_for_active_matching_lease() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let note_path = root.path().join("notes").join("a.md");
        fs::create_dir_all(note_path.parent().unwrap()).unwrap();
        fs::write(&note_path, "# A\n").unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections
             SET needs_full_sync = 1,
                 active_lease_session_id = 'serve-session'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let actions = run_overflow_recovery_pass(&conn, "serve-session").unwrap();
        let collection = load_collection_by_id(&conn, collection_id).unwrap();

        assert!(actions
            .iter()
            .any(|(_, action)| action == "work:reconciled"));
        assert!(!collection.needs_full_sync);
        assert!(collection.last_sync_at.is_some());

        let runtime = start_serve_runtime(db_path).unwrap();
        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn run_overflow_recovery_pass_leaves_needs_full_sync_for_missing_or_foreign_lease() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root_a = tempfile::TempDir::new().unwrap();
        let root_b = tempfile::TempDir::new().unwrap();
        let missing_lease = insert_collection(&conn, "missing-lease", root_a.path());
        let foreign_lease = insert_collection(&conn, "foreign-lease", root_b.path());
        conn.execute(
            "UPDATE collections
             SET needs_full_sync = 1
             WHERE id = ?1",
            [missing_lease],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET needs_full_sync = 1,
                 active_lease_session_id = 'other-session'
             WHERE id = ?1",
            [foreign_lease],
        )
        .unwrap();

        let actions = run_overflow_recovery_pass(&conn, "serve-session").unwrap();

        assert!(actions
            .iter()
            .any(|(collection_id, action)| *collection_id == missing_lease
                && action == "missing-lease:lease-mismatch"));
        assert!(actions
            .iter()
            .any(|(collection_id, action)| *collection_id == foreign_lease
                && action == "foreign-lease:lease-mismatch"));
        let rows = conn
            .prepare(
                "SELECT name, needs_full_sync, last_sync_at
                 FROM collections
                 WHERE id IN (?1, ?2)
                 ORDER BY id",
            )
            .unwrap()
            .query_map(params![missing_lease, foreign_lease], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                ("missing-lease".to_owned(), true, None),
                ("foreign-lease".to_owned(), true, None),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_leaves_restoring_needs_full_sync_for_overflow_worker() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 needs_full_sync = 1,
                 integrity_failed_at = '2026-04-28T00:00:00Z'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();
        thread::sleep(Duration::from_millis(1200));

        let verify = Connection::open(&db_path).unwrap();
        let collection = load_collection_by_id(&verify, collection_id).unwrap();

        assert!(collection.needs_full_sync);
        assert_eq!(collection.state, CollectionState::Restoring);
        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn watch_callback_marks_collection_needs_full_sync_when_channel_is_full() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let file_path = root.path().join("notes").join("a.md");
        write_restore_file(root.path(), "notes/a.md", b"watch callback bytes");
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .blocking_send(WatchEvent::DirtyPath(PathBuf::from(
                "notes/already-buffered.md",
            )))
            .unwrap();
        let mut callback = watch_callback(
            collection_id,
            root.path().to_path_buf(),
            db_path.clone(),
            sender,
        );
        let event = NotifyEvent {
            kind: NotifyEventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            paths: vec![file_path],
            attrs: Default::default(),
        };

        callback(Ok(event));

        let needs_full_sync: bool = conn
            .query_row(
                "SELECT needs_full_sync FROM collections WHERE id = ?1",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(needs_full_sync);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            WatchEvent::DirtyPath(path) if path == Path::new("notes/already-buffered.md")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn watch_callback_drops_events_when_channel_is_closed_without_mutating_collection() {
        let (_dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let file_path = root.path().join("notes").join("a.md");
        write_restore_file(root.path(), "notes/a.md", b"watch callback bytes");
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);
        let mut callback =
            watch_callback(collection_id, root.path().to_path_buf(), db_path, sender);
        let event = NotifyEvent {
            kind: NotifyEventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            paths: vec![file_path],
            attrs: Default::default(),
        };

        callback(Ok(event));

        let needs_full_sync: bool = conn
            .query_row(
                "SELECT needs_full_sync FROM collections WHERE id = ?1",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!needs_full_sync);
    }

    #[test]
    fn ensure_collection_write_allowed_refuses_on_restoring_or_needs_full_sync() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn, "work", Path::new("vault"));
        conn.execute(
            "UPDATE collections SET state = 'restoring', needs_full_sync = 1 WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let error = ensure_collection_write_allowed(&conn, collection_id).unwrap_err();
        assert!(error.to_string().contains("CollectionRestoringError"));
        assert!(error.to_string().contains("needs_full_sync=true"));
    }

    #[test]
    fn ensure_collection_write_allowed_refuses_when_only_needs_full_sync_is_set() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn, "work", Path::new("vault"));
        conn.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let error = ensure_collection_write_allowed(&conn, collection_id).unwrap_err();
        assert!(error.to_string().contains("CollectionRestoringError"));
        assert!(error.to_string().contains("needs_full_sync=true"));
    }

    #[test]
    fn begin_restore_rejects_non_empty_target() {
        let conn = open_test_db();
        let source_root = tempfile::TempDir::new().unwrap();
        let target_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", source_root.path());
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
            "notes/a.md",
        );
        fs::write(target_root.path().join("occupied.txt"), b"x").unwrap();

        let error = begin_restore(&conn, "work", target_root.path(), false).unwrap_err();
        assert!(error.to_string().contains("RestoreNonEmptyTargetError"));
    }

    #[cfg(not(unix))]
    #[test]
    fn begin_restore_on_windows_releases_cli_lease_after_inline_attach_failure() {
        let (_db_dir, _db_path, conn) = open_test_db_file();
        let source_root = tempfile::TempDir::new().unwrap();
        let target_parent = tempfile::TempDir::new().unwrap();
        let target_root = target_parent.path().join("restored");
        let collection_id = insert_collection(&conn, "work", source_root.path());
        let raw_bytes =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            raw_bytes,
            "notes/a.md",
        );

        let error = begin_restore(&conn, "work", &target_root, false).unwrap_err();

        assert!(error
            .to_string()
            .contains("Vault sync commands require Unix"));
        type RestoreWindowsFailureRow = (
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
            i64,
        );

        let row: RestoreWindowsFailureRow = conn
            .query_row(
                "SELECT state,
                        root_path,
                        needs_full_sync,
                        pending_root_path,
                        restore_command_id,
                        restore_lease_session_id,
                        (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1),
                        (SELECT COUNT(*) FROM serve_sessions WHERE session_type = 'cli')
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, "restoring");
        assert_eq!(row.1, target_root.display().to_string());
        assert_eq!(row.2, 1);
        assert!(row.3.is_none());
        assert!(row.4.is_none());
        assert!(row.5.is_none());
        assert_eq!(row.6, 0);
        assert_eq!(row.7, 0);
        assert_eq!(
            fs::read(target_root.join("notes").join("a.md")).unwrap(),
            raw_bytes
        );
    }

    #[test]
    fn mark_collection_restoring_uses_collection_owners_and_clears_ack_residue() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        // Use a high explicit collection id so this test cannot collide with
        // parallel in-memory tests that share the process-global supervisor registry.
        let collection_id = insert_collection_with_id(&conn, 50_001, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-owner', 1, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-spoof', 2, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-owner')",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET active_lease_session_id = 'serve-spoof',
                 reload_generation = 4,
                 watcher_released_session_id = 'serve-spoof',
                 watcher_released_generation = 3,
                 watcher_released_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let (collection, expected_session_id, generation) =
            mark_collection_restoring_for_handshake(&conn, collection_id).unwrap();

        assert_eq!(expected_session_id, "serve-owner");
        assert_eq!(generation, 5);
        assert_eq!(collection.state, CollectionState::Restoring);
        assert!(collection.watcher_released_session_id.is_none());
        assert!(collection.watcher_released_generation.is_none());
        assert!(collection.watcher_released_at.is_none());
    }

    // design.md §404-408: mark_collection_restoring_for_handshake must use
    // live_collection_owner (session_type='serve') so a live CLI lease in
    // collection_owners is never treated as the expected serve supervisor.
    #[test]
    fn mark_collection_restoring_rejects_cli_session_as_handshake_owner() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        // Insert a CLI-type session directly into serve_sessions.
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, session_type)
             VALUES ('cli-lease', 1, 'host', 'cli')",
            [],
        )
        .unwrap();
        // Force the CLI session as the collection owner (bypasses acquire_owner_lease
        // type gate to exercise the production handshake path directly).
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'cli-lease')",
            [collection_id],
        )
        .unwrap();

        let err = mark_collection_restoring_for_handshake(&conn, collection_id).unwrap_err();

        // live_collection_owner finds no serve-type owner → ServeOwnsCollectionError,
        // NOT a timeout waiting for an ack only a serve supervisor can emit.
        assert!(
            err.to_string().contains("ServeOwnsCollectionError"),
            "expected ServeOwnsCollectionError but got: {err}"
        );
    }

    // Source-seam invariant: the production handshake paths must use
    // live_collection_owner (typed) rather than owner_session_id + session_is_live
    // (untyped).  This guards against regressions that re-open the CLI-as-owner hole.
    #[test]
    fn handshake_functions_use_typed_live_collection_owner_not_untyped_pair() {
        let src = include_str!("vault_sync.rs");
        // Locate the mark_collection_restoring_for_handshake body (up to its closing
        // brace) and the wait_for_exact_ack body, and assert they call
        // live_collection_owner rather than the untyped owner_session_id / session_is_live.
        for fn_name in &[
            "mark_collection_restoring_for_handshake",
            "wait_for_exact_ack",
        ] {
            let fn_start = src
                .find(&format!("pub fn {fn_name}"))
                .unwrap_or_else(|| panic!("could not find fn {fn_name} in source"));
            // Grab roughly 80 lines of body (sufficient for both functions).
            let body: String = src[fn_start..].chars().take(3000).collect();
            assert!(
                body.contains("live_collection_owner"),
                "{fn_name} must call live_collection_owner (typed) — \
                 regression guard for design.md §404-408 CLI-owner hole"
            );
            assert!(
                !body.contains("session_is_live(conn"),
                "{fn_name} must NOT call untyped session_is_live — \
                 use live_collection_owner instead"
            );
        }
    }

    #[test]
    fn wait_for_exact_ack_short_circuits_when_live_owner_disappears() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-1', 1, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-1')",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections SET state = 'restoring', reload_generation = 2 WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        unregister_session(&conn, "serve-1").unwrap();

        let started = Instant::now();
        let error = wait_for_exact_ack(&conn, collection_id, "serve-1", 2).unwrap_err();

        assert!(matches!(
            error,
            VaultSyncError::ServeDiedDuringHandshake { .. }
        ));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "owner-loss path must short-circuit instead of waiting for the full handshake timeout"
        );
    }

    #[test]
    fn acquire_owner_lease_refuses_live_foreign_owner_and_preserves_existing_claim() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-owner', 1, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('cli-owner', 2, 'host')",
            [],
        )
        .unwrap();
        acquire_owner_lease(&conn, collection_id, "serve-owner").unwrap();

        let error = acquire_owner_lease(&conn, collection_id, "cli-owner").unwrap_err();

        assert!(error.to_string().contains("ServeOwnsCollectionError"));
        assert_eq!(
            owner_session_id(&conn, collection_id).unwrap().as_deref(),
            Some("serve-owner")
        );
    }

    #[test]
    fn acquire_owner_lease_allows_same_session_reentrant_claim_and_keeps_single_row() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('cli-owner', 2, 'host')",
            [],
        )
        .unwrap();

        acquire_owner_lease(&conn, collection_id, "cli-owner").unwrap();
        acquire_owner_lease(&conn, collection_id, "cli-owner").unwrap();

        let row: (Option<String>, i64, Option<String>) = conn
            .query_row(
                "SELECT active_lease_session_id,
                        (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1),
                        (SELECT session_id FROM collection_owners WHERE collection_id = ?1)
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0.as_deref(), Some("cli-owner"));
        assert_eq!(row.1, 1);
        assert_eq!(row.2.as_deref(), Some("cli-owner"));
        assert_eq!(
            owner_session_id(&conn, collection_id).unwrap().as_deref(),
            Some("cli-owner")
        );
    }

    #[test]
    fn acquire_owner_lease_reclaims_stale_owner_residue_and_updates_mirror_column() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
             VALUES ('stale-owner', 1, 'host', datetime('now', '-120 seconds'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('cli-owner', 2, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'stale-owner')",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections SET active_lease_session_id = 'stale-owner' WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        acquire_owner_lease(&conn, collection_id, "cli-owner").unwrap();

        let row: (Option<String>, i64) = conn
            .query_row(
                "SELECT active_lease_session_id,
                        (SELECT COUNT(*) FROM collection_owners WHERE session_id = 'stale-owner')
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0.as_deref(), Some("cli-owner"));
        assert_eq!(row.1, 0);
        assert_eq!(
            owner_session_id(&conn, collection_id).unwrap().as_deref(),
            Some("cli-owner")
        );
    }

    #[test]
    fn finalize_pending_restore_requires_exact_originator_or_stale_heartbeat() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = 'C:/restored',
                 pending_restore_manifest = '{\"entries\":[]}',
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let outcome = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::StartupRecovery {
                session_id: "serve-1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(outcome, FinalizeOutcome::Deferred);
    }

    #[test]
    fn finalize_pending_restore_production_callers_pass_explicit_finalize_caller_variants() {
        let source = production_vault_sync_source();
        let callsites = source
            .match_indices("finalize_pending_restore(")
            .map(|(index, _)| index)
            .filter(|index| !source[..*index].ends_with("fn "))
            .collect::<Vec<_>>();

        assert_eq!(
            callsites.len(),
            4,
            "expected exactly four production finalize_pending_restore call sites"
        );

        for callsite in callsites {
            let snippet_end = std::cmp::min(callsite + 240, source.len());
            let snippet = &source[callsite..snippet_end];
            assert!(
                snippet.contains("FinalizeCaller::ExternalFinalize")
                    || snippet.contains("FinalizeCaller::StartupRecovery")
                    || snippet.contains("FinalizeCaller::RestoreOriginator"),
                "production finalize call site must pass an explicit FinalizeCaller variant: {snippet}"
            );
        }
    }

    #[test]
    fn finalize_pending_restore_startup_recovery_uses_shared_15_second_heartbeat_threshold() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let pending_root = temp.path().join("restored");
        fs::create_dir_all(&pending_root).unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = '{\"entries\":[]}',
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = datetime('now', '-14 seconds')
             WHERE id = ?1",
            params![collection_id, pending_root.display().to_string()],
        )
        .unwrap();

        let fresh = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::StartupRecovery {
                session_id: "serve-1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(fresh, FinalizeOutcome::Deferred);

        conn.execute(
            "UPDATE collections
             SET pending_command_heartbeat_at = datetime('now', '-16 seconds')
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let stale = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::StartupRecovery {
                session_id: "serve-1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(stale, FinalizeOutcome::Finalized);
    }

    #[test]
    fn finalize_pending_restore_allows_exact_originator_with_fresh_heartbeat() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let pending_root = temp.path().join("restored");
        fs::create_dir_all(&pending_root).unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = '{\"entries\":[]}',
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![collection_id, pending_root.display().to_string()],
        )
        .unwrap();

        let outcome = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::RestoreOriginator {
                command_id: "restore-1".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(outcome, FinalizeOutcome::Finalized);
        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(collection.root_path, pending_root.display().to_string());
        assert!(collection.pending_root_path.is_none());
        assert!(collection.needs_full_sync);
    }

    #[test]
    fn finalize_pending_restore_rejects_foreign_originator_with_fresh_heartbeat() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let pending_root = temp.path().join("restored");
        fs::create_dir_all(&pending_root).unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = '{\"entries\":[]}',
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![collection_id, pending_root.display().to_string()],
        )
        .unwrap();

        let outcome = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::RestoreOriginator {
                command_id: "restore-2".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(outcome, FinalizeOutcome::Deferred);
        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(
            collection.pending_root_path.as_deref(),
            Some(pending_root.to_str().unwrap())
        );
        assert_eq!(collection.restore_command_id.as_deref(), Some("restore-1"));
    }

    #[test]
    fn finalize_pending_restore_external_finalize_runs_tx_b_canonical_state() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let pending_root = temp.path().join("restored");
        fs::create_dir_all(&pending_root).unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = '{\"entries\":[]}',
                 restore_command_id = 'restore-1',
                 restore_command_pid = 99,
                 restore_command_host = 'host',
                 pending_command_heartbeat_at = datetime('now', '-120 seconds'),
                 watcher_released_session_id = 'serve-1',
                 watcher_released_generation = 2,
                 watcher_released_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![collection_id, pending_root.display().to_string()],
        )
        .unwrap();

        let outcome = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::ExternalFinalize {
                session_id: "serve-1".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(outcome, FinalizeOutcome::Finalized);
        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(collection.root_path, pending_root.display().to_string());
        assert_eq!(collection.state, CollectionState::Restoring);
        assert!(collection.needs_full_sync);
        assert!(collection.pending_root_path.is_none());
        assert!(collection.pending_restore_manifest.is_none());
        assert!(collection.restore_command_id.is_none());
        assert!(collection.pending_command_heartbeat_at.is_none());
        assert!(collection.watcher_released_session_id.is_none());
        assert!(collection.watcher_released_generation.is_none());
        assert!(collection.watcher_released_at.is_none());
    }

    #[test]
    fn run_tx_b_is_idempotent_and_arms_write_gate() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = '{\"entries\":[]}'
             WHERE id = ?1",
            params![collection_id, temp.path().display().to_string()],
        )
        .unwrap();

        assert!(run_tx_b(&conn, collection_id).unwrap());
        assert!(!run_tx_b(&conn, collection_id).unwrap());

        let row: (String, i64) = conn
            .query_row(
                "SELECT state, needs_full_sync FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, "restoring");
        assert_eq!(row.1, 1);
    }

    #[cfg(unix)]
    #[test]
    fn remap_online_source_defers_attach_to_rcrt_and_remap_attach_reason_appears_once() {
        let source = production_vault_sync_source();

        let remap_start = source.find("pub fn remap_collection(").unwrap();
        let remap_end = source[remap_start..]
            .find("pub fn verify_remap_root(")
            .map(|offset| remap_start + offset)
            .unwrap();
        let remap_source = &source[remap_start..remap_end];
        assert!(
            remap_source.contains(
                "wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?"
            ),
            "online remap must wait for the exact watcher ack before mutating DB state"
        );
        assert!(
            remap_source.contains("needs_full_sync = 1"),
            "online remap must arm the write gate for the post-remap attach pass"
        );
        assert!(
            remap_source.contains("DELETE FROM file_state WHERE collection_id = ?1"),
            "online remap must limit itself to the DB state flip plus file_state reset"
        );
        let online_source = &remap_source[..remap_source.find("} else {").unwrap()];
        assert!(
            !online_source.contains("complete_attach(")
                && !online_source.contains("full_hash_reconcile_authorized("),
            "remap_collection must not run attach or full-hash reconcile inline"
        );

        let rcrt_start = source.find("pub fn run_rcrt_pass(").unwrap();
        let rcrt_end = source[rcrt_start..]
            .find("fn embedding_drain_interval_secs(")
            .map(|offset| rcrt_start + offset)
            .unwrap();
        let rcrt_source = &source[rcrt_start..rcrt_end];
        assert_eq!(
            rcrt_source
                .matches("AttachReason::RemapPostReconcile")
                .count(),
            1,
            "RCRT should have exactly one remap attach arm"
        );
        assert!(
            rcrt_source.contains("if complete_attach(conn, collection_id, session_id, reason)? {"),
            "RCRT must own the attach transition after remap"
        );
    }

    #[test]
    fn complete_attach_source_is_reentry_guarded_by_needs_full_sync() {
        let source = production_vault_sync_source();
        let start = source.find("fn complete_attach(").unwrap();
        let end = source[start..]
            .find("pub fn run_rcrt_pass(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("if !collection.needs_full_sync")
                && snippet.contains("return Ok(false);"),
            "complete_attach must fail closed to a no-op when the write gate is already cleared"
        );
        assert!(
            snippet.contains("reload_generation = reload_generation + 1"),
            "attach completion must advance generation exactly on the guarded transition"
        );
        assert!(
            snippet.contains("AND needs_full_sync = 1"),
            "the attach-completion UPDATE must be gated on needs_full_sync so re-entry cannot bump generation again"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_rcrt_pass_clears_needs_full_sync_after_tx_b() {
        const COLLECTION_ID: i64 = 50_000;

        init_process_registries().unwrap();
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection_with_id(&conn, COLLECTION_ID, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = '{\"entries\":[]}'
             WHERE id = ?1",
            params![collection_id, temp.path().display().to_string()],
        )
        .unwrap();
        assert!(run_tx_b(&conn, collection_id).unwrap());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-1', 1, 'host')",
            [],
        )
        .unwrap();
        acquire_owner_lease(&conn, collection_id, "serve-1").unwrap();

        let actions = run_rcrt_pass(&conn, "serve-1").unwrap();

        assert_eq!(actions, vec![(collection_id, "work:attached".to_owned())]);
        let row: (String, i64) = conn
            .query_row(
                "SELECT state, needs_full_sync FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, "active");
        assert_eq!(row.1, 0);
    }

    #[test]
    fn run_rcrt_pass_preserves_pending_root_path_when_manifest_is_incomplete() {
        const COLLECTION_ID: i64 = 50_001;

        init_process_registries().unwrap();
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let pending_root = temp.path().join("restored");
        write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
        let manifest_json = manifest_json_for_directory(&pending_root);
        fs::remove_file(pending_root.join("notes").join("a.md")).unwrap();
        let collection_id = insert_collection_with_id(&conn, COLLECTION_ID, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-1', 1, 'host')",
            [],
        )
        .unwrap();
        acquire_owner_lease(&conn, collection_id, "serve-1").unwrap();
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = ?3,
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = datetime('now', '-120 seconds')
             WHERE id = ?1",
            params![
                collection_id,
                pending_root.display().to_string(),
                manifest_json
            ],
        )
        .unwrap();

        let actions = run_rcrt_pass(&conn, "serve-1").unwrap();

        assert_eq!(
            actions,
            vec![(collection_id, "work:ManifestIncomplete".to_owned())]
        );
        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(
            collection.pending_root_path.as_deref(),
            Some(pending_root.to_str().unwrap())
        );
        assert!(collection.pending_manifest_incomplete_at.is_some());
        assert!(collection.integrity_failed_at.is_none());
    }

    #[test]
    fn begin_restore_preserves_tx_b_residue_and_plain_sync_cannot_consume_it() {
        let (_db_dir, _db_path, conn) = open_test_db_file();
        let source_root = tempfile::TempDir::new().unwrap();
        let target_parent = tempfile::TempDir::new().unwrap();
        let target_root = target_parent.path().join("restored");
        fs::create_dir_all(source_root.path().join("notes")).unwrap();
        let collection_id = insert_collection(&conn, "work", source_root.path());
        let raw_bytes =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            raw_bytes,
            "notes/a.md",
        );
        fs::write(source_root.path().join("notes").join("a.md"), raw_bytes).unwrap();
        conn.execute(
            "CREATE TRIGGER tx_b_fail
             BEFORE UPDATE ON collections
             WHEN OLD.pending_root_path IS NOT NULL AND NEW.pending_root_path IS NULL
             BEGIN
                 SELECT RAISE(FAIL, 'tx-b fail');
             END",
            [],
        )
        .unwrap();

        let error = begin_restore(&conn, "work", &target_root, false).unwrap_err();

        assert!(error.to_string().contains("tx-b fail"));
        let row: (String, Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT state, pending_root_path, restore_command_id, pending_restore_manifest
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "restoring");
        assert_eq!(row.1.as_deref(), Some(target_root.to_str().unwrap()));
        assert!(row.2.is_some());
        assert!(row.3.is_some());
        assert!(target_root.exists());

        let sync_error = sync_collection(&conn, "work").unwrap_err();

        assert!(sync_error
            .to_string()
            .contains("RestorePendingFinalizeError"));
        let retained_pending_root: Option<String> = conn
            .query_row(
                "SELECT pending_root_path FROM collections WHERE id = ?1",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            retained_pending_root.as_deref(),
            Some(target_root.to_str().unwrap())
        );
    }

    #[test]
    fn finalize_pending_restore_retries_manifest_incomplete_until_success() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let pending_root = temp.path().join("restored");
        write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
        let manifest_json = manifest_json_for_directory(&pending_root);
        fs::remove_file(pending_root.join("notes").join("a.md")).unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = ?3,
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![
                collection_id,
                pending_root.display().to_string(),
                manifest_json
            ],
        )
        .unwrap();

        let first = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::RestoreOriginator {
                command_id: "restore-1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(first, FinalizeOutcome::ManifestIncomplete);
        let first_incomplete_at: String = conn
            .query_row(
                "SELECT pending_manifest_incomplete_at FROM collections WHERE id = ?1",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();

        let second = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::RestoreOriginator {
                command_id: "restore-1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(second, FinalizeOutcome::ManifestIncomplete);
        let second_incomplete_at: String = conn
            .query_row(
                "SELECT pending_manifest_incomplete_at FROM collections WHERE id = ?1",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(first_incomplete_at, second_incomplete_at);

        write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
        let final_outcome = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::RestoreOriginator {
                command_id: "restore-1".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(final_outcome, FinalizeOutcome::Finalized);
        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert!(collection.pending_root_path.is_none());
        assert!(collection.pending_manifest_incomplete_at.is_none());
        assert!(collection.integrity_failed_at.is_none());
        assert_eq!(collection.root_path, pending_root.display().to_string());
    }

    #[test]
    fn finalize_pending_restore_escalates_manifest_incomplete_after_ttl() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let pending_root = temp.path().join("restored");
        write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
        let manifest_json = manifest_json_for_directory(&pending_root);
        fs::remove_file(pending_root.join("notes").join("a.md")).unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = ?3,
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                 pending_manifest_incomplete_at = datetime('now', '-31 minutes')
             WHERE id = ?1",
            params![
                collection_id,
                pending_root.display().to_string(),
                manifest_json
            ],
        )
        .unwrap();

        let outcome = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::RestoreOriginator {
                command_id: "restore-1".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(outcome, FinalizeOutcome::IntegrityFailed);
        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(
            collection.pending_root_path.as_deref(),
            Some(pending_root.to_str().unwrap())
        );
        assert!(collection.pending_manifest_incomplete_at.is_some());
        assert!(collection.integrity_failed_at.is_some());
    }

    #[test]
    fn finalize_pending_restore_detects_manifest_tamper_and_restore_reset_clears_it() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let pending_root = temp.path().join("restored");
        write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
        let manifest_json = manifest_json_for_directory(&pending_root);
        write_restore_file(&pending_root, "notes/a.md", b"tampered bytes");
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = ?3,
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![
                collection_id,
                pending_root.display().to_string(),
                manifest_json
            ],
        )
        .unwrap();

        let outcome = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::RestoreOriginator {
                command_id: "restore-1".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(outcome, FinalizeOutcome::IntegrityFailed);
        let blocked = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(
            blocked.pending_root_path.as_deref(),
            Some(pending_root.to_str().unwrap())
        );
        assert!(blocked.integrity_failed_at.is_some());

        write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
        let still_blocked = finalize_pending_restore(
            &conn,
            collection_id,
            FinalizeCaller::RestoreOriginator {
                command_id: "restore-1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(still_blocked, FinalizeOutcome::IntegrityFailed);

        restore_reset(&conn, "work").unwrap();

        let reset = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(reset.state, CollectionState::Active);
        assert!(reset.pending_root_path.is_none());
        assert!(reset.pending_restore_manifest.is_none());
        assert!(reset.restore_command_id.is_none());
        assert!(reset.integrity_failed_at.is_none());
    }

    #[test]
    fn unregister_session_clears_ownership_mirror_columns() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-1', 1, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-1')",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET active_lease_session_id = 'serve-1',
                 restore_lease_session_id = 'serve-1'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        unregister_session(&conn, "serve-1").unwrap();

        let row: (Option<String>, Option<String>, i64, i64) = conn
            .query_row(
                "SELECT active_lease_session_id,
                        restore_lease_session_id,
                        (SELECT COUNT(*) FROM collection_owners WHERE session_id = 'serve-1'),
                        (SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'serve-1')
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert!(row.0.is_none());
        assert!(row.1.is_none());
        assert_eq!(row.2, 0);
        assert_eq!(row.3, 0);
    }

    #[test]
    fn lease_guard_drop_releases_owner_lease_and_unregisters_short_lived_session() {
        let (_dir, db_path, conn) = open_test_db_file();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        let session_id = register_session(&conn).unwrap();
        acquire_owner_lease(&conn, collection_id, &session_id).unwrap();

        {
            let lease_guard = LeaseGuard {
                db_path: Some(db_path),
                session_id: session_id.clone(),
            };
            drop(lease_guard);
        }

        let row: (Option<String>, i64, i64) = conn
            .query_row(
                "SELECT active_lease_session_id,
                        (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1),
                        (SELECT COUNT(*) FROM serve_sessions WHERE session_id = ?2)
                 FROM collections
                 WHERE id = ?1",
                params![collection_id, session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(row.0.is_none());
        assert_eq!(row.1, 0);
        assert_eq!(row.2, 0);
    }

    #[cfg(unix)]
    #[test]
    fn offline_restore_runs_attach_inline_and_reopens_writes() {
        let (_db_dir, _db_path, conn) = open_test_db_file();
        let source_root = tempfile::TempDir::new().unwrap();
        let target_parent = tempfile::TempDir::new().unwrap();
        let target_root = target_parent.path().join("restored");
        fs::create_dir_all(source_root.path().join("notes")).unwrap();
        let collection_id = insert_collection(&conn, "work", source_root.path());
        let raw_bytes =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            raw_bytes,
            "notes/a.md",
        );
        fs::write(source_root.path().join("notes").join("a.md"), raw_bytes).unwrap();

        begin_restore(&conn, "work", &target_root, false).unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(collection.state, CollectionState::Active);
        assert!(!collection.needs_full_sync);
        assert!(collection.active_lease_session_id.is_none());
        assert!(collection.restore_lease_session_id.is_none());
        assert!(owner_session_id(&conn, collection_id).unwrap().is_none());
        ensure_collection_write_allowed(&conn, collection_id).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn offline_remap_runs_reconcile_inline_and_preserves_uuid_identity_across_reorganization() {
        let (_db_dir, _db_path, conn) = open_test_db_file();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(old_root.path().join("notes")).unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        let raw_bytes_a =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\nslug: notes/a\n---\nhello world from note a";
        let raw_bytes_b =
            b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\nslug: notes/b\n---\nhello world from note b";
        let page_a = insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            raw_bytes_a,
            "notes/old-a.md",
        );
        let page_b = insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/b",
            "22222222-2222-7222-8222-222222222222",
            "hello world from note b",
            raw_bytes_b,
            "notes/b.md",
        );
        fs::write(old_root.path().join("notes").join("old-a.md"), raw_bytes_a).unwrap();
        fs::write(old_root.path().join("notes").join("b.md"), raw_bytes_b).unwrap();
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::create_dir_all(new_root.path().join("nested")).unwrap();
        fs::write(
            new_root.path().join("nested").join("renamed-a.md"),
            raw_bytes_a,
        )
        .unwrap();
        fs::write(new_root.path().join("notes").join("b.md"), raw_bytes_b).unwrap();
        conn.execute(
            "INSERT INTO links (from_page_id, to_page_id, relationship, context, source_kind)
             VALUES (?1, ?2, 'depends_on', '', 'programmatic')",
            params![page_a, page_b],
        )
        .unwrap();

        let summary = remap_collection(&conn, "work", new_root.path(), false).unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(collection.root_path, new_root.path().display().to_string());
        assert_eq!(collection.state, CollectionState::Active);
        assert!(!collection.needs_full_sync);
        assert_eq!(summary.resolved_pages, 2);
        assert!(collection.active_lease_session_id.is_none());
        assert!(collection.restore_lease_session_id.is_none());
        assert!(owner_session_id(&conn, collection_id).unwrap().is_none());
        ensure_collection_write_allowed(&conn, collection_id).unwrap();
        let remapped_page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE collection_id = ?1 AND slug = 'notes/a'",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remapped_page_id, page_a);
        let relative_path: String = conn
            .query_row(
                "SELECT relative_path FROM file_state WHERE page_id = ?1",
                [page_a],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(relative_path, "nested/renamed-a.md");
        let link_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM links WHERE from_page_id = ?1 AND to_page_id = ?2",
                params![page_a, page_b],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(link_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn remap_collection_refuses_phase1_drift_until_new_root_catches_up() {
        let (_db_dir, _db_path, conn) = open_test_db_file();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(old_root.path().join("notes")).unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        let original =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            original,
            "notes/a.md",
        );
        fs::write(old_root.path().join("notes").join("a.md"), original).unwrap();
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::write(new_root.path().join("notes").join("a.md"), original).unwrap();

        let updated =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nupdated body before remap";
        fs::write(old_root.path().join("notes").join("a.md"), updated).unwrap();

        let first = remap_collection(&conn, "work", new_root.path(), false).unwrap_err();
        assert!(first.to_string().contains("RemapDriftConflictError"));
        let active_raw_import: Vec<u8> = conn
            .query_row(
                "SELECT raw_bytes FROM raw_imports WHERE page_id = (SELECT id FROM pages WHERE collection_id = ?1 AND slug = 'notes/a') AND is_active = 1",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active_raw_import, updated);

        fs::write(new_root.path().join("notes").join("a.md"), updated).unwrap();
        let second = remap_collection(&conn, "work", new_root.path(), false).unwrap();
        assert_eq!(second.missing_pages, 0);
        assert_eq!(second.mismatched_pages, 0);
        assert_eq!(second.extra_files, 0);
    }

    #[cfg(not(unix))]
    #[test]
    fn remap_collection_fails_closed_on_windows_before_mutating_collection_state() {
        let (_db_dir, _db_path, conn) = open_test_db_file();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        let original_root = old_root.path().display().to_string();

        let error = remap_collection(&conn, "work", new_root.path(), false).unwrap_err();

        assert!(error
            .to_string()
            .contains("Vault sync commands require Unix"));
        let row: (String, String, i64) = conn
            .query_row(
                "SELECT root_path, state, needs_full_sync FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, original_root);
        assert_eq!(row.1, "active");
        assert_eq!(row.2, 0);
    }

    #[test]
    fn verify_remap_root_uses_unique_hash_fallback_and_ignores_quaidignore_patterns() {
        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        let raw_bytes = b"---\ntitle: Hash Fallback\ntype: concept\n---\nthis body is intentionally long enough to cross the remap hash fallback threshold.\n";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/hash-fallback",
            "11111111-1111-7111-8111-111111111111",
            "this body is intentionally long enough to cross the remap hash fallback threshold.",
            raw_bytes,
            "notes/hash-fallback.md",
        );
        fs::create_dir_all(new_root.path().join("nested")).unwrap();
        fs::write(new_root.path().join("nested").join("moved.md"), raw_bytes).unwrap();
        fs::write(new_root.path().join(".quaidignore"), "ignored/**\n").unwrap();
        fs::create_dir_all(new_root.path().join("ignored")).unwrap();
        fs::write(
            new_root.path().join("ignored").join("secret.md"),
            b"top secret",
        )
        .unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let summary = verify_remap_root(&conn, &collection, new_root.path()).unwrap();

        assert_eq!(summary.resolved_pages, 1);
        assert_eq!(summary.missing_pages, 0);
        assert_eq!(summary.mismatched_pages, 0);
        assert_eq!(summary.extra_files, 0);
    }

    #[test]
    fn verify_remap_root_ignores_non_markdown_files() {
        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        let raw_bytes =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            raw_bytes,
            "notes/a.md",
        );
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::create_dir_all(new_root.path().join("assets")).unwrap();
        fs::write(new_root.path().join("notes").join("a.md"), raw_bytes).unwrap();
        fs::write(
            new_root.path().join("assets").join("logo.png"),
            b"not markdown, not a remap extra",
        )
        .unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let summary = verify_remap_root(&conn, &collection, new_root.path()).unwrap();

        assert_eq!(summary.resolved_pages, 1);
        assert_eq!(summary.missing_pages, 0);
        assert_eq!(summary.mismatched_pages, 0);
        assert_eq!(summary.extra_files, 0);
    }

    #[cfg(unix)]
    #[test]
    fn verify_remap_root_skips_symlinked_entries_in_new_root() {
        use std::os::unix::fs::symlink;

        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let linked = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        let raw_bytes =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            raw_bytes,
            "notes/a.md",
        );
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::write(new_root.path().join("notes").join("a.md"), raw_bytes).unwrap();
        fs::create_dir_all(linked.path().join("shadow")).unwrap();
        fs::write(
            linked.path().join("shadow").join("extra.md"),
            b"reachable only through symlink",
        )
        .unwrap();
        symlink(
            linked.path().join("shadow"),
            new_root.path().join("linked-shadow"),
        )
        .unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let summary = verify_remap_root(&conn, &collection, new_root.path()).unwrap();

        assert_eq!(summary.resolved_pages, 1);
        assert_eq!(summary.missing_pages, 0);
        assert_eq!(summary.mismatched_pages, 0);
        assert_eq!(summary.extra_files, 0);
    }

    #[test]
    fn verify_remap_root_rejects_invalid_frontmatter_uuid_in_new_root() {
        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
            "notes/a.md",
        );
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::write(
            new_root.path().join("notes").join("broken.md"),
            b"---\nmemory_id: definitely-not-a-uuid\n---\nhello world from note a",
        )
        .unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();

        assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
        assert!(error.to_string().contains("invalid"));
    }

    #[test]
    fn verify_remap_root_does_not_use_hash_fallback_for_short_body() {
        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        let raw_bytes = b"---\ntitle: Short\n---\nshort\n";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/short",
            "11111111-1111-7111-8111-111111111111",
            "short",
            raw_bytes,
            "notes/short.md",
        );
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::write(new_root.path().join("notes").join("moved.md"), raw_bytes).unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();

        assert!(matches!(
            error,
            VaultSyncError::NewRootVerificationFailed {
                missing: 1,
                mismatched: 0,
                extra: 1,
                ..
            }
        ));
    }

    #[test]
    fn verify_remap_root_rejects_duplicate_hash_candidates_without_uuid_match() {
        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        let raw_bytes = b"---\ntitle: Duplicate Hash\n---\nthis body is intentionally long enough to cross the remap hash fallback threshold.\n";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/hash-duplicate",
            "11111111-1111-7111-8111-111111111111",
            "this body is intentionally long enough to cross the remap hash fallback threshold.",
            raw_bytes,
            "notes/hash-duplicate.md",
        );
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::write(new_root.path().join("notes").join("one.md"), raw_bytes).unwrap();
        fs::write(new_root.path().join("notes").join("two.md"), raw_bytes).unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();

        assert!(matches!(
            error,
            VaultSyncError::NewRootVerificationFailed {
                missing: 1,
                mismatched: 0,
                extra: 2,
                ..
            }
        ));
    }

    #[test]
    fn verify_remap_root_reports_missing_and_extra_counts() {
        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
            "notes/a.md",
        );
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/b",
            "22222222-2222-7222-8222-222222222222",
            "hello world from note b",
            b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\n---\nhello world from note b",
            "notes/b.md",
        );
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::write(
            new_root.path().join("notes").join("a.md"),
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
        )
        .unwrap();
        fs::write(
            new_root.path().join("notes").join("extra.md"),
            b"extra file",
        )
        .unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();
        assert!(matches!(
            error,
            VaultSyncError::NewRootVerificationFailed {
                missing: 1,
                mismatched: 0,
                extra: 1,
                ..
            }
        ));
    }

    #[test]
    fn verify_remap_root_error_includes_sampled_diffs() {
        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
            "notes/a.md",
        );
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/b",
            "22222222-2222-7222-8222-222222222222",
            "hello world from note b",
            b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\n---\nhello world from note b",
            "notes/b.md",
        );
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::write(
            new_root.path().join("notes").join("a.md"),
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nchanged bytes",
        )
        .unwrap();
        fs::write(
            new_root.path().join("notes").join("extra.md"),
            b"extra file",
        )
        .unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();
        let rendered = error.to_string();

        assert!(rendered.contains("missing_samples=notes/b"));
        assert!(rendered.contains("mismatched_samples=notes/a -> notes/a.md sha256_mismatch"));
        assert!(rendered.contains("extra_samples=notes/extra.md"));
    }

    #[test]
    fn verify_remap_root_reports_mismatched_count_for_duplicate_uuid_candidates() {
        let conn = open_test_db();
        let old_root = tempfile::TempDir::new().unwrap();
        let new_root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", old_root.path());
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            "hello world from note a",
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
            "notes/a.md",
        );
        fs::create_dir_all(new_root.path().join("notes")).unwrap();
        fs::write(
            new_root.path().join("notes").join("a-one.md"),
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nfirst duplicate",
        )
        .unwrap();
        fs::write(
            new_root.path().join("notes").join("a-two.md"),
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nsecond duplicate",
        )
        .unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();
        assert!(matches!(
            error,
            VaultSyncError::NewRootVerificationFailed {
                missing: 1,
                mismatched: 1,
                extra: 2,
                ..
            }
        ));
    }

    #[test]
    fn verify_remap_root_source_uses_before_and_after_tree_fences() {
        let source = production_vault_sync_source();
        let start = source.find("pub fn verify_remap_root(").unwrap();
        let end = source[start..]
            .find("pub fn restore_reset(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("let before = take_tree_fence(new_root)?;")
                && snippet.contains("let after = take_tree_fence(new_root)?;"),
            "verify_remap_root must fence the tree before and after matching to catch mid-flight drift"
        );
        assert!(
            snippet.contains("return Err(VaultSyncError::NewRootUnstable"),
            "verify_remap_root must fail closed with NewRootUnstableError when the fence changes"
        );
    }

    #[test]
    fn load_new_root_files_source_applies_new_root_ignore_matcher() {
        let source = production_vault_sync_source();
        let start = source.find("fn load_new_root_files(").unwrap();
        let end = source[start..]
            .find("fn resolve_page_matches(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("let ignore_globset = build_new_root_ignore_globset(root)?;"),
            "load_new_root_files must build a fresh ignore matcher from the remap root before counting files"
        );
        assert!(
            snippet.contains("if ignore_globset.is_match(relative_path) {"),
            "load_new_root_files must exclude .quaidignore-matched files from remap verification counts"
        );
        assert!(
            snippet.contains("if !is_markdown_file(relative_path) {"),
            "load_new_root_files must reuse the reconciler's Markdown gate so Phase 4 extra counts stay in parity"
        );
    }

    #[test]
    fn resolve_page_matches_source_uses_canonical_resolver_helper() {
        let source = production_vault_sync_source();
        let start = source.find("fn resolve_page_matches(").unwrap();
        let end = source[start..]
            .find("fn take_tree_fence(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("resolve_page_identity("),
            "Phase 4 remap verification must invoke the canonical resolve_page_identity helper instead of bespoke matching"
        );
    }

    #[test]
    fn take_tree_fence_hashes_quaidignore_contents() {
        let root = tempfile::TempDir::new().unwrap();
        let bytes = b"ignored/**\n";
        let expected_hash = sha256_hex(bytes);
        fs::write(root.path().join(".quaidignore"), bytes).unwrap();

        let fence = take_tree_fence(root.path()).unwrap();
        let entry = fence.get(".quaidignore").unwrap();

        assert_eq!(entry.size_bytes, bytes.len() as u64);
        assert_eq!(
            entry.quaidignore_sha256.as_deref(),
            Some(expected_hash.as_str()),
            "Phase 4 tree fence must hash .quaidignore contents, not just trust size/mtime"
        );
    }

    #[cfg(unix)]
    #[test]
    fn take_tree_fence_detects_same_size_rewrite_with_preserved_mtime() {
        let root = tempfile::TempDir::new().unwrap();
        let notes_dir = root.path().join("notes");
        fs::create_dir_all(&notes_dir).unwrap();
        let path = notes_dir.join("rewrite.md");
        fs::write(&path, b"aaaaaaaaaaaaaaaa").unwrap();

        let before = take_tree_fence(root.path()).unwrap();
        let original_modified = fs::metadata(&path).unwrap().modified().unwrap();
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            use std::io::Write;
            file.write_all(b"bbbbbbbbbbbbbbbb").unwrap();
            file.sync_all().unwrap();
            let times = std::fs::FileTimes::new().set_modified(original_modified);
            file.set_times(times).unwrap();
        }
        let after = take_tree_fence(root.path()).unwrap();

        let before_entry = before.get("notes/rewrite.md").unwrap();
        let after_entry = after.get("notes/rewrite.md").unwrap();
        assert_eq!(after_entry.mtime_ns, before_entry.mtime_ns);
        assert_eq!(after_entry.size_bytes, before_entry.size_bytes);
        assert_eq!(after_entry.inode, before_entry.inode);
        assert_ne!(
            after_entry.ctime_ns, before_entry.ctime_ns,
            "Phase 4 tree fence must notice same-size rewrites even when mtime is preserved"
        );
        assert_ne!(
            before, after,
            "Phase 4 tree fence must fail closed when only ctime changes across the verified tree"
        );
    }

    #[cfg(unix)]
    #[test]
    fn take_tree_fence_detects_atomic_replace_with_preserved_mtime() {
        let root = tempfile::TempDir::new().unwrap();
        let notes_dir = root.path().join("notes");
        fs::create_dir_all(&notes_dir).unwrap();
        let path = notes_dir.join("replace.md");
        fs::write(&path, b"aaaaaaaaaaaaaaaa").unwrap();

        let before = take_tree_fence(root.path()).unwrap();
        let original_metadata = fs::metadata(&path).unwrap();
        let replacement = notes_dir.join("replace.tmp");
        fs::write(&replacement, b"cccccccccccccccc").unwrap();
        let replacement_file = fs::OpenOptions::new()
            .write(true)
            .open(&replacement)
            .unwrap();
        let times = std::fs::FileTimes::new().set_modified(original_metadata.modified().unwrap());
        replacement_file.set_times(times).unwrap();
        replacement_file.sync_all().unwrap();
        drop(replacement_file);
        fs::rename(&replacement, &path).unwrap();

        let after = take_tree_fence(root.path()).unwrap();

        let before_entry = before.get("notes/replace.md").unwrap();
        let after_entry = after.get("notes/replace.md").unwrap();
        assert_eq!(after_entry.mtime_ns, before_entry.mtime_ns);
        assert_eq!(after_entry.size_bytes, before_entry.size_bytes);
        assert_ne!(
            after_entry.inode, before_entry.inode,
            "Phase 4 tree fence must notice atomic replacements even when the replacement backdates mtime"
        );
        assert_ne!(
            before, after,
            "Phase 4 tree fence must fail closed when the verified file is atomically replaced"
        );
    }

    #[test]
    fn take_tree_fence_source_uses_full_stat_tuple() {
        let source = production_vault_sync_source();
        assert!(
            source.contains("metadata_timestamp_ns(metadata.mtime(), metadata.mtime_nsec())")
                && source.contains("metadata_timestamp_ns(metadata.ctime(), metadata.ctime_nsec())")
                && source.contains("metadata.ino()"),
            "Phase 4 tree fence must capture the full per-file stat tuple so same-size rewrites and atomic replacements cannot slip past remap verification"
        );
    }

    #[test]
    fn walk_tree_source_skips_symlinked_entries() {
        let source = production_vault_sync_source();
        let start = source.find("fn walk_tree(").unwrap();
        let end = source[start..]
            .find("fn path_string(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("let file_type = entry.file_type()?;")
                && snippet.contains("if file_type.is_symlink() {"),
            "Phase 4 tree walks must inspect entry file types without following symlinks"
        );
    }

    #[test]
    fn remap_source_runs_safety_pipeline_before_new_root_verification() {
        let source = production_vault_sync_source();
        let remap_start = source.find("pub fn remap_collection(").unwrap();
        let remap_end = source[remap_start..]
            .find("pub fn verify_remap_root(")
            .map(|offset| remap_start + offset)
            .unwrap();
        let remap_source = &source[remap_start..remap_end];

        let safety_idx = remap_source
            .find("run_restore_remap_safety_pipeline_without_mount_check")
            .unwrap();
        let verify_idx = remap_source
            .find("verify_remap_root(conn, &collection, new_root)?")
            .unwrap();
        assert!(
            safety_idx < verify_idx,
            "remap_collection must capture old-root drift before trusting the new root"
        );
    }

    #[test]
    fn remap_online_source_waits_for_exact_ack_before_safety_pipeline() {
        let source = production_vault_sync_source();
        let remap_start = source.find("pub fn remap_collection(").unwrap();
        let remap_end = source[remap_start..]
            .find("pub fn verify_remap_root(")
            .map(|offset| remap_start + offset)
            .unwrap();
        let remap_source = &source[remap_start..remap_end];
        let ack_idx = remap_source
            .find("wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?")
            .unwrap();
        let safety_idx = remap_source
            .find("run_restore_remap_safety_pipeline_without_mount_check")
            .unwrap();

        assert!(
            ack_idx < safety_idx,
            "online remap must release the live watcher and then capture old-root drift under the acknowledged owner lease"
        );
    }

    #[test]
    fn restore_source_runs_safety_pipeline_before_materialization() {
        let source = production_vault_sync_source();
        let restore_start = source.find("pub fn begin_restore(").unwrap();
        let restore_end = source[restore_start..]
            .find("pub fn remap_collection(")
            .map(|offset| restore_start + offset)
            .unwrap();
        let restore_source = &source[restore_start..restore_end];

        let online_start = restore_source
            .find("let (_, expected_session_id, generation) =")
            .unwrap();
        let online_end = restore_source[online_start..]
            .find("} else {")
            .map(|offset| online_start + offset)
            .unwrap();
        let online_source = &restore_source[online_start..online_end];
        let online_safety_idx = online_source
            .find("run_restore_remap_safety_pipeline_without_mount_check")
            .unwrap();
        let online_materialize_idx = online_source
            .find("materialize_collection_to_path(conn, &collection, &staging_path)?;")
            .unwrap();
        assert!(
            online_safety_idx < online_materialize_idx,
            "online restore must capture old-root drift before materializing raw_imports into the target"
        );

        let offline_start = restore_source
            .find("let lease = start_short_lived_owner_lease(conn, collection.id)?;")
            .unwrap();
        let offline_source = &restore_source[offline_start..];
        let offline_safety_idx = offline_source
            .find("run_restore_remap_safety_pipeline_without_mount_check")
            .unwrap();
        let offline_materialize_idx = offline_source
            .find("materialize_collection_to_path(conn, &collection, &staging_path)?;")
            .unwrap();
        assert!(
            offline_safety_idx < offline_materialize_idx,
            "offline restore must capture old-root drift before materializing raw_imports into the target"
        );
    }

    #[test]
    fn restore_online_source_waits_for_exact_ack_before_safety_pipeline() {
        let source = production_vault_sync_source();
        let restore_start = source.find("pub fn begin_restore(").unwrap();
        let restore_end = source[restore_start..]
            .find("pub fn remap_collection(")
            .map(|offset| restore_start + offset)
            .unwrap();
        let restore_source = &source[restore_start..restore_end];
        let online_start = restore_source
            .find("let (_, expected_session_id, generation) =")
            .unwrap();
        let online_end = restore_source[online_start..]
            .find("} else {")
            .map(|offset| online_start + offset)
            .unwrap();
        let online_source = &restore_source[online_start..online_end];
        let ack_idx = online_source
            .find("wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?")
            .unwrap();
        let safety_idx = online_source
            .find("run_restore_remap_safety_pipeline_without_mount_check")
            .unwrap();

        assert!(
            ack_idx < safety_idx,
            "online restore must wait for the acknowledged owner lease before drift capture"
        );
    }

    #[test]
    fn remap_source_verifies_new_root_before_switching_root_path() {
        let source = production_vault_sync_source();
        let remap_start = source.find("pub fn remap_collection(").unwrap();
        let remap_end = source[remap_start..]
            .find("pub fn verify_remap_root(")
            .map(|offset| remap_start + offset)
            .unwrap();
        let remap_source = &source[remap_start..remap_end];
        let verify_idx = remap_source
            .find("verify_remap_root(conn, &collection, new_root)?")
            .unwrap();
        let update_idx = remap_source.find("SET root_path = ?2,").unwrap();

        assert!(
            verify_idx < update_idx,
            "remap_collection must prove the target tree before rewriting the collection root"
        );
    }

    #[test]
    fn remap_offline_source_uses_short_lived_lease_and_inline_attach() {
        let source = production_vault_sync_source();
        let remap_start = source.find("pub fn remap_collection(").unwrap();
        let remap_end = source[remap_start..]
            .find("pub fn verify_remap_root(")
            .map(|offset| remap_start + offset)
            .unwrap();
        let remap_source = &source[remap_start..remap_end];
        let offline_start = remap_source
            .find("let lease = start_short_lived_owner_lease")
            .unwrap();
        let offline_end = remap_source[offline_start..]
            .find("Ok(summary)")
            .map(|offset| offline_start + offset)
            .unwrap();
        let offline_source = &remap_source[offline_start..offline_end];

        assert!(
            offline_source.contains("complete_attach(")
                && offline_source.contains("AttachReason::RemapPostReconcile"),
            "offline remap must run the attach/full-hash path inline while the CLI lease is live"
        );
        assert!(
            !offline_source.contains("unregister_session("),
            "offline remap must not drop its lease before the inline attach finishes"
        );
    }

    #[test]
    fn restore_offline_source_uses_short_lived_lease_and_inline_attach() {
        let source = production_vault_sync_source();
        let restore_start = source.find("pub fn begin_restore(").unwrap();
        let restore_end = source[restore_start..]
            .find("pub fn remap_collection(")
            .map(|offset| restore_start + offset)
            .unwrap();
        let restore_source = &source[restore_start..restore_end];
        let offline_start = restore_source
            .find("let lease = start_short_lived_owner_lease(conn, collection.id)?;")
            .unwrap();
        let offline_source = &restore_source[offline_start..];

        assert!(
            offline_source.contains("complete_attach(")
                && offline_source.contains("AttachReason::RestorePostFinalize"),
            "offline restore must run the attach/full-hash path inline while the CLI lease is live"
        );
        assert!(
            !offline_source.contains("unregister_session("),
            "offline restore must not drop its lease before the inline attach finishes"
        );
    }

    #[test]
    fn run_rcrt_pass_skips_reconcile_halted_collections() {
        const COLLECTION_ID: i64 = 50_002;

        init_process_registries().unwrap();
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection_with_id(&conn, COLLECTION_ID, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-1', 1, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-1')",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 reconcile_halted_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                 reconcile_halt_reason = 'duplicate_uuid'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let actions = run_rcrt_pass(&conn, "serve-1").unwrap();
        assert_eq!(
            actions,
            vec![(collection_id, "work:skipped-halt".to_owned())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_recovers_tx_b_orphan_exactly_once_before_supervisor_ack() {
        let (_dir, db_path, conn) = open_test_db_file();
        let source_root = tempfile::TempDir::new().unwrap();
        let pending_parent = tempfile::TempDir::new().unwrap();
        let pending_root = pending_parent.path().join("restored");
        let collection_id = insert_collection(&conn, "work", source_root.path());
        write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
        let manifest_json = manifest_json_for_directory(&pending_root);
        conn.execute(
            "CREATE TABLE startup_finalize_audit (
                 collection_id INTEGER NOT NULL,
                 cleared_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             )",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TRIGGER startup_finalize_exactly_once
             AFTER UPDATE ON collections
             WHEN OLD.pending_root_path IS NOT NULL AND NEW.pending_root_path IS NULL
             BEGIN
                 INSERT INTO startup_finalize_audit (collection_id) VALUES (NEW.id);
             END",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
             VALUES ('stale-owner', 1, 'host', datetime('now', '-16 seconds'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host)
             VALUES ('foreign-live', 2, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id)
             VALUES (?1, 'stale-owner')",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = ?3,
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = datetime('now', '-16 seconds')
             WHERE id = ?1",
            params![
                collection_id,
                pending_root.display().to_string(),
                manifest_json
            ],
        )
        .unwrap();
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();

        type StartupRecoveryStealsLeaseRow = (
            String,
            String,
            i64,
            Option<String>,
            i64,
            i64,
            i64,
            i64,
            Option<String>,
        );

        let row: StartupRecoveryStealsLeaseRow = wait_for_collection_update(
            &db_path,
            collection_id,
            Duration::from_secs(5),
            |verify, collection_id| {
                verify
                        .query_row(
                            "SELECT state,
                                    root_path,
                                    needs_full_sync,
                                    pending_root_path,
                                    (SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'stale-owner'),
                                    (SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'foreign-live'),
                                    (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2),
                                    (SELECT COUNT(*) FROM startup_finalize_audit WHERE collection_id = ?1),
                                    watcher_released_session_id
                             FROM collections
                             WHERE id = ?1",
                            params![collection_id, runtime.session_id.as_str()],
                            |row| {
                                Ok((
                                    row.get(0)?,
                                    row.get(1)?,
                                    row.get(2)?,
                                    row.get(3)?,
                                    row.get(4)?,
                                    row.get(5)?,
                                    row.get(6)?,
                                    row.get(7)?,
                                    row.get(8)?,
                                ))
                            },
                        )
                        .ok()
                        .and_then(|row| if row.0 == "active" { Some(row) } else { None })
            },
        );
        assert_eq!(row.0, "active");
        assert_eq!(row.1, pending_root.display().to_string());
        assert_eq!(row.2, 0);
        assert!(row.3.is_none());
        assert_eq!(row.4, 0);
        assert_eq!(row.5, 1);
        assert_eq!(row.6, 1);
        assert_eq!(row.7, 1);
        assert!(row.8.is_none());

        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_defers_fresh_restore_heartbeat_and_leaves_collection_blocked() {
        let (_dir, db_path, conn) = open_test_db_file();
        let source_root = tempfile::TempDir::new().unwrap();
        let pending_parent = tempfile::TempDir::new().unwrap();
        let pending_root = pending_parent.path().join("restored");
        let collection_id = insert_collection(&conn, "work", source_root.path());
        write_restore_file(&pending_root, "notes/a.md", b"hello from restore");
        let manifest_json = manifest_json_for_directory(&pending_root);
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
             VALUES ('stale-owner', 1, 'host', datetime('now', '-16 seconds'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id)
             VALUES (?1, 'stale-owner')",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = ?3,
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![
                collection_id,
                pending_root.display().to_string(),
                manifest_json
            ],
        )
        .unwrap();
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();
        thread::sleep(Duration::from_millis(500));

        type RecoveryRow = (
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
            i64,
            Option<String>,
            Option<i64>,
            Option<String>,
        );

        let verify = Connection::open(&db_path).unwrap();
        let row: RecoveryRow = verify
            .query_row(
                "SELECT state,
                        root_path,
                        needs_full_sync,
                        pending_root_path,
                        restore_command_id,
                        (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2),
                        watcher_released_session_id,
                        watcher_released_generation,
                        watcher_released_at
                 FROM collections
                 WHERE id = ?1",
                params![collection_id, runtime.session_id.as_str()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, "restoring");
        assert_eq!(row.1, source_root.path().display().to_string());
        assert_eq!(row.2, 0);
        assert_eq!(row.3.as_deref(), Some(pending_root.to_str().unwrap()));
        assert_eq!(row.4.as_deref(), Some("restore-1"));
        assert_eq!(row.5, 1);
        assert!(
            row.6.is_none() && row.7.is_none() && row.8.is_none(),
            "fresh serve must not impersonate the originator by writing the watcher ack triple"
        );

        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_bootstraps_recovery_directories_for_existing_collections() {
        #[cfg(target_os = "linux")]
        let _env_lock = env_mutation_lock().lock().unwrap();
        #[cfg(target_os = "linux")]
        let _runtime_root = secure_runtime_root();
        #[cfg(target_os = "linux")]
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", _runtime_root.path().to_str().unwrap());
        let (dir, db_path, conn) = open_test_db_file();
        let root_a = tempfile::TempDir::new().unwrap();
        let root_b = tempfile::TempDir::new().unwrap();
        let collection_a = insert_collection(&conn, "work", root_a.path());
        let collection_b = insert_collection(&conn, "notes", root_b.path());
        drop(conn);

        let runtime = start_serve_runtime(db_path).unwrap();

        let recovery_root = dir.path().join("recovery");
        assert!(collection_recovery_dir(&recovery_root, collection_a).is_dir());
        assert!(collection_recovery_dir(&recovery_root, collection_b).is_dir());

        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_recovers_owned_sentinel_dirty_collection_and_unlinks_all_sentinels() {
        #[cfg(target_os = "linux")]
        let _env_lock = env_mutation_lock().lock().unwrap();
        #[cfg(target_os = "linux")]
        let _runtime_root = secure_runtime_root();
        #[cfg(target_os = "linux")]
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", _runtime_root.path().to_str().unwrap());
        init_process_registries().unwrap();
        let (dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let uuid = "01969f11-9448-7d79-8d3f-c68f54768888";
        let old_bytes = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nOld body from db.\n"
        );
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            uuid,
            "Old body from db.",
            old_bytes.as_bytes(),
            "notes/a.md",
        );
        let new_bytes = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nNew body from disk after crash.\n"
        );
        write_restore_file(root.path(), "notes/a.md", new_bytes.as_bytes());
        let recovery_root = dir.path().join("recovery");
        create_startup_recovery_sentinel(&recovery_root, collection_id, "write-1.needs_full_sync");
        create_startup_recovery_sentinel(&recovery_root, collection_id, "write-2.needs_full_sync");
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();

        let row: (String, i64, String, i64, Vec<u8>, i64) = wait_for_collection_update(
            &db_path,
            collection_id,
            Duration::from_secs(5),
            |verify, collection_id| {
                let row = verify
                    .query_row(
                        "SELECT c.state,
                                c.needs_full_sync,
                                p.compiled_truth,
                                p.version,
                                ri.raw_bytes,
                                (SELECT COUNT(*) FROM raw_imports WHERE page_id = p.id AND is_active = 1)
                         FROM collections c
                         JOIN pages p ON p.collection_id = c.id AND p.slug = 'notes/a'
                         JOIN raw_imports ri ON ri.page_id = p.id AND ri.is_active = 1
                         WHERE c.id = ?1",
                        [collection_id],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                                row.get(5)?,
                            ))
                        },
                    )
                    .ok()?;
                (row.0 == "active"
                    && row.1 == 0
                    && row.2 == "New body from disk after crash."
                    && startup_recovery_sentinel_count(&recovery_root, collection_id) == 0)
                    .then_some(row)
            },
        );

        assert_eq!(row.0, "active");
        assert_eq!(row.1, 0);
        assert_eq!(row.2, "New body from disk after crash.");
        assert_eq!(row.3, 2);
        assert_eq!(row.4, new_bytes.as_bytes());
        assert_eq!(row.5, 1);

        drop(runtime);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();
        thread::sleep(Duration::from_millis(300));

        let verify = Connection::open(&db_path).unwrap();
        let version: i64 = verify
            .query_row(
                "SELECT version FROM pages WHERE collection_id = ?1 AND slug = 'notes/a'",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 2);
        assert_eq!(
            startup_recovery_sentinel_count(&recovery_root, collection_id),
            0
        );

        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_only_processes_owned_collection_sentinels() {
        #[cfg(target_os = "linux")]
        let _env_lock = env_mutation_lock().lock().unwrap();
        #[cfg(target_os = "linux")]
        let _runtime_root = secure_runtime_root();
        #[cfg(target_os = "linux")]
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", _runtime_root.path().to_str().unwrap());
        let (dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let uuid = "01969f11-9448-7d79-8d3f-c68f54768889";
        let old_bytes = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nOld body from db.\n"
        );
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            uuid,
            "Old body from db.",
            old_bytes.as_bytes(),
            "notes/a.md",
        );
        let new_bytes = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nNew body that foreign owner still needs to reconcile.\n"
        );
        write_restore_file(root.path(), "notes/a.md", new_bytes.as_bytes());
        let recovery_root = dir.path().join("recovery");
        create_startup_recovery_sentinel(&recovery_root, collection_id, "write-1.needs_full_sync");
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('foreign-live', 1, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'foreign-live')",
            [collection_id],
        )
        .unwrap();
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();
        thread::sleep(Duration::from_millis(500));

        let verify = Connection::open(&db_path).unwrap();
        let row: (String, i64, Option<String>, i64) = verify
            .query_row(
                "SELECT p.compiled_truth,
                        c.needs_full_sync,
                        (SELECT session_id FROM collection_owners WHERE collection_id = ?1),
                        (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2)
                 FROM collections c
                 JOIN pages p ON p.collection_id = c.id AND p.slug = 'notes/a'
                 WHERE c.id = ?1",
                params![collection_id, runtime.session_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(row.0, "Old body from db.");
        assert_eq!(row.1, 0);
        assert_eq!(row.2.as_deref(), Some("foreign-live"));
        assert_eq!(row.3, 0);
        assert_eq!(
            startup_recovery_sentinel_count(&recovery_root, collection_id),
            1
        );

        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_retains_sentinel_when_startup_reconcile_fails() {
        #[cfg(target_os = "linux")]
        let _env_lock = env_mutation_lock().lock().unwrap();
        #[cfg(target_os = "linux")]
        let _runtime_root = secure_runtime_root();
        #[cfg(target_os = "linux")]
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", _runtime_root.path().to_str().unwrap());
        let (dir, db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let uuid = "01969f11-9448-7d79-8d3f-c68f54768890";
        let note_a = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nThis body is comfortably above the minimum size for rename inference.\n"
        );
        let note_b = format!(
            "---\nmemory_id: {uuid}\nslug: notes/b\ntitle: B\ntype: concept\n---\nThis second body is also comfortably above the minimum size for rename inference.\n"
        );
        fs::write(root.path().join("a.md"), note_a).unwrap();
        fs::write(root.path().join("b.md"), note_b).unwrap();
        let recovery_root = dir.path().join("recovery");
        create_startup_recovery_sentinel(&recovery_root, collection_id, "write-1.needs_full_sync");
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();
        thread::sleep(Duration::from_millis(500));

        let verify = Connection::open(&db_path).unwrap();
        let row: (Option<String>, Option<String>, i64, i64) = verify
            .query_row(
                "SELECT reconcile_halted_at,
                        reconcile_halt_reason,
                        needs_full_sync,
                        (SELECT COUNT(*) FROM pages WHERE collection_id = ?1)
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert!(row.0.is_some());
        assert_eq!(row.1.as_deref(), Some("duplicate_uuid"));
        assert_eq!(row.2, 1);
        assert_eq!(row.3, 0);
        assert_eq!(
            startup_recovery_sentinel_count(&recovery_root, collection_id),
            1
        );

        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn writer_side_foreign_rename_with_sqlite_busy_recovers_from_sentinel_alone() {
        #[cfg(target_os = "linux")]
        let _env_lock = env_mutation_lock().lock().unwrap();
        #[cfg(target_os = "linux")]
        let _runtime_root = secure_runtime_root();
        #[cfg(target_os = "linux")]
        let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", _runtime_root.path().to_str().unwrap());
        let (dir, db_path, conn) = open_test_db_file();
        init_process_registries().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let uuid = "01969f11-9448-7d79-8d3f-c68f54768891";
        let old_bytes = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nOld body from db.\n"
        );
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            uuid,
            "Old body from db.",
            old_bytes.as_bytes(),
            "notes/a.md",
        );
        write_restore_file(root.path(), "notes/a.md", old_bytes.as_bytes());

        let our_bytes = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nOur writer bytes lose the race.\n"
        );
        let foreign_bytes = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nForeign bytes win before stat capture.\n"
        );
        let relative_path = Path::new("notes/a.md");
        let write_id = "writer-foreign-rename";
        let recovery_root = dir.path().join("recovery");
        let sentinel_path = writer_side_sentinel_path(&recovery_root, collection_id, write_id);
        let dedup_key =
            writer_side_dedup_key(&root.path().join(relative_path), our_bytes.as_bytes());

        let busy_conn = Connection::open(&db_path).unwrap();
        busy_conn
            .execute_batch("BEGIN IMMEDIATE; UPDATE collections SET updated_at = updated_at")
            .unwrap();

        let error = exercise_writer_side_sentinel_crash_core(
            &conn,
            collection_id,
            relative_path,
            our_bytes.as_bytes(),
            write_id,
            &WriterSideSentinelCrashMode::ForeignRenameBetweenRenameAndStat {
                foreign_bytes: foreign_bytes.as_bytes().to_vec(),
            },
        )
        .unwrap_err();

        assert!(matches!(error, VaultSyncError::ConcurrentRename { .. }));
        assert!(sentinel_path.exists());
        assert!(!writer_side_dedup_contains(&dedup_key));
        assert_eq!(
            fs::read(root.path().join(relative_path)).unwrap(),
            foreign_bytes.as_bytes()
        );

        busy_conn.execute_batch("ROLLBACK").unwrap();
        drop(busy_conn);

        let verify = Connection::open(&db_path).unwrap();
        let needs_full_sync: i64 = verify
            .query_row(
                "SELECT needs_full_sync FROM collections WHERE id = ?1",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(needs_full_sync, 0);
        drop(verify);
        drop(conn);

        let runtime = start_serve_runtime(db_path.clone()).unwrap();
        let row: (String, i64, Vec<u8>, i64) = wait_for_collection_update(
            &db_path,
            collection_id,
            Duration::from_secs(5),
            |verify, collection_id| {
                let row = verify
                    .query_row(
                        "SELECT p.compiled_truth,
                                c.needs_full_sync,
                                ri.raw_bytes,
                                p.version
                         FROM collections c
                         JOIN pages p ON p.collection_id = c.id AND p.slug = 'notes/a'
                         JOIN raw_imports ri ON ri.page_id = p.id AND ri.is_active = 1
                         WHERE c.id = ?1",
                        [collection_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .ok()?;
                (row.0 == "Foreign bytes win before stat capture."
                    && row.1 == 0
                    && row.2 == foreign_bytes.as_bytes()
                    && startup_recovery_sentinel_count(&recovery_root, collection_id) == 0)
                    .then_some(row)
            },
        );

        assert_eq!(row.0, "Foreign bytes win before stat capture.");
        assert_eq!(row.1, 0);
        assert_eq!(row.2, foreign_bytes.as_bytes());
        assert_eq!(row.3, 2);

        drop(runtime);
    }

    #[test]
    fn write_supervisor_ack_rejects_foreign_stale_and_replayed_acknowledgements() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-1', 1, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-1')",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-2', 2, 'host')",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections SET state = 'restoring', reload_generation = 2 WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        assert!(!write_supervisor_ack_if_needed(&conn, collection_id, "serve-2", 2).unwrap());
        assert!(!write_supervisor_ack_if_needed(&conn, collection_id, "serve-1", 1).unwrap());
        assert!(write_supervisor_ack_if_needed(&conn, collection_id, "serve-1", 2).unwrap());
        assert!(!write_supervisor_ack_if_needed(&conn, collection_id, "serve-1", 2).unwrap());

        let ack: (Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT watcher_released_session_id, watcher_released_generation
                 FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(ack.0.as_deref(), Some("serve-1"));
        assert_eq!(ack.1, Some(2));
    }

    #[test]
    fn wait_for_exact_ack_reports_when_serve_ownership_changes_mid_handshake() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
             VALUES ('serve-1', 1, 'host', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
             VALUES ('serve-2', 2, 'host', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-2')",
            [collection_id],
        )
        .unwrap();

        match wait_for_exact_ack(&conn, collection_id, "serve-1", 2) {
            Err(VaultSyncError::ServeOwnershipChanged {
                collection_name,
                expected_session_id,
                actual_session_id,
            }) => {
                assert_eq!(collection_name, "work");
                assert_eq!(expected_session_id, "serve-1");
                assert_eq!(actual_session_id, "serve-2");
            }
            other => panic!("expected ServeOwnershipChangedError, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn full_hash_audit_pass_rehashes_due_active_lease_collections_only() {
        let conn = open_test_db();
        let due_root = tempfile::TempDir::new().unwrap();
        let skipped_root = tempfile::TempDir::new().unwrap();
        let due_id = insert_collection(&conn, "due", due_root.path());
        let skipped_id = insert_collection(&conn, "skipped", skipped_root.path());

        let due_uuid = Uuid::now_v7().to_string();
        let due_bytes = format!(
            "---\nmemory_id: {due_uuid}\nslug: notes/due\ntitle: Due\ntype: concept\n---\nBody.\n"
        );
        fs::create_dir_all(due_root.path().join("notes")).unwrap();
        fs::write(due_root.path().join("notes/due.md"), due_bytes.as_bytes()).unwrap();
        insert_page_with_raw_import(
            &conn,
            due_id,
            "notes/due",
            &due_uuid,
            "Body.",
            due_bytes.as_bytes(),
            "notes/due.md",
        );
        conn.execute(
            "UPDATE file_state
             SET last_full_hash_at = datetime('now', '-8 days')
             WHERE collection_id = ?1",
            [due_id],
        )
        .unwrap();

        let skipped_uuid = Uuid::now_v7().to_string();
        let skipped_bytes = format!(
            "---\nmemory_id: {skipped_uuid}\nslug: notes/skipped\ntitle: Skipped\ntype: concept\n---\nBody.\n"
        );
        fs::create_dir_all(skipped_root.path().join("notes")).unwrap();
        fs::write(
            skipped_root.path().join("notes/skipped.md"),
            skipped_bytes.as_bytes(),
        )
        .unwrap();
        insert_page_with_raw_import(
            &conn,
            skipped_id,
            "notes/skipped",
            &skipped_uuid,
            "Body.",
            skipped_bytes.as_bytes(),
            "notes/skipped.md",
        );
        conn.execute(
            "UPDATE file_state
             SET last_full_hash_at = datetime('now', '-1 days')
             WHERE collection_id = ?1",
            [skipped_id],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
             VALUES ('serve-audit', 1, 'host', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-audit')",
            [due_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-audit')",
            [skipped_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections SET active_lease_session_id = 'serve-audit' WHERE id IN (?1, ?2)",
            rusqlite::params![due_id, skipped_id],
        )
        .unwrap();

        let audited = run_full_hash_audit_pass(&conn, "serve-audit").unwrap();

        assert_eq!(audited.len(), 1);
        assert_eq!(audited[0].0, due_id);
        assert_eq!(audited[0].1, "due");
    }

    #[cfg(unix)]
    #[test]
    fn full_hash_audit_pass_limits_each_cycle_to_a_daily_subset() {
        let _guard = env_mutation_lock().lock().unwrap();
        let _env = EnvVarGuard::set("QUAID_FULL_HASH_AUDIT_DAYS", "3");
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());

        fs::create_dir_all(root.path().join("notes")).unwrap();
        for index in 0..7 {
            let relative_path = format!("notes/{index:02}.md");
            let slug = format!("notes/{index:02}");
            let uuid = Uuid::now_v7().to_string();
            let raw = format!(
                "---\nmemory_id: {uuid}\nslug: {slug}\ntitle: Note {index}\ntype: concept\n---\nBody {index}.\n"
            );
            fs::write(root.path().join(&relative_path), raw.as_bytes()).unwrap();
            insert_page_with_raw_import(
                &conn,
                collection_id,
                &slug,
                &uuid,
                &format!("Body {index}."),
                raw.as_bytes(),
                &relative_path,
            );
        }
        conn.execute(
            "UPDATE file_state
             SET last_full_hash_at = datetime('now', '-8 days')
             WHERE collection_id = ?1",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
             VALUES ('serve-audit', 1, 'host', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-audit')",
            [collection_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE collections SET active_lease_session_id = 'serve-audit' WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let audited = run_full_hash_audit_pass(&conn, "serve-audit").unwrap();

        assert_eq!(audited.len(), 1);
        assert_eq!(
            audited[0].2.walked, 3,
            "7 files over 3 days should hash only ceil(7/3) files per cycle"
        );
        assert_eq!(audited[0].2.unchanged, 3);
        let fresh_paths = conn
            .prepare(
                "SELECT relative_path
                 FROM file_state
                 WHERE collection_id = ?1
                   AND last_full_hash_at > datetime('now', '-1 minute')
                 ORDER BY relative_path ASC",
            )
            .unwrap()
            .query_map([collection_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let stale_paths = conn
            .prepare(
                "SELECT relative_path
                 FROM file_state
                 WHERE collection_id = ?1
                   AND last_full_hash_at < datetime('now', '-7 days')
                 ORDER BY relative_path ASC",
            )
            .unwrap()
            .query_map([collection_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            fresh_paths,
            vec![
                "notes/00.md".to_owned(),
                "notes/01.md".to_owned(),
                "notes/02.md".to_owned()
            ],
            "scheduled audit must update only the first budgeted subset this cycle"
        );
        assert_eq!(stale_paths.len(), 4);

        let audited_again = run_full_hash_audit_pass(&conn, "serve-audit").unwrap();

        assert_eq!(audited_again.len(), 1);
        assert_eq!(
            audited_again[0].2.walked, 3,
            "the next serve-loop cycle must stay bounded to the same daily subset size"
        );
        let remaining_stale = conn
            .prepare(
                "SELECT relative_path
                 FROM file_state
                 WHERE collection_id = ?1
                   AND last_full_hash_at < datetime('now', '-7 days')
                 ORDER BY relative_path ASC",
            )
            .unwrap()
            .query_map([collection_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            remaining_stale,
            vec!["notes/06.md".to_owned()],
            "scheduled audit must advance to the next oldest subset instead of re-running a whole-vault pass"
        );
    }

    #[test]
    fn run_full_hash_audit_pass_batches_due_rows_instead_of_inline_full_vault_reconcile() {
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("core")
                .join("vault_sync.rs"),
        )
        .unwrap();
        let fn_start = source
            .find("pub fn run_full_hash_audit_pass(")
            .expect("run_full_hash_audit_pass fn present");
        let fn_end = source[fn_start..]
            .find("pub fn audit_collection(")
            .map(|offset| fn_start + offset)
            .expect("audit_collection fn follows run_full_hash_audit_pass");
        let fn_body = &source[fn_start..fn_end];

        assert!(
            fn_body.contains("scheduled_full_hash_audit_budget(total_files.max(0) as usize)"),
            "scheduled audit must compute a bounded per-cycle budget before hashing overdue rows"
        );
        assert!(
            fn_body.contains("LIMIT ?3"),
            "scheduled audit must select only the budgeted overdue rows per serve-loop cycle"
        );
        assert!(
            fn_body.contains("scheduled_full_hash_audit_authorized("),
            "scheduled audit must hash only the selected overdue subset"
        );
        assert!(
            !fn_body.contains("full_hash_reconcile_authorized("),
            "run_full_hash_audit_pass must not collapse back into a whole-vault inline reconcile"
        );
    }

    #[test]
    fn full_hash_audit_helpers_respect_env_override_and_floor_at_zero() {
        let _guard = env_mutation_lock().lock().unwrap();

        {
            let _env = EnvVarGuard::clear("QUAID_FULL_HASH_AUDIT_DAYS");
            assert_eq!(
                configured_full_hash_audit_days(),
                DEFAULT_FULL_HASH_AUDIT_DAYS
            );
            assert_eq!(
                full_hash_audit_ttl_cutoff(),
                format!("-{} days", DEFAULT_FULL_HASH_AUDIT_DAYS)
            );
        }

        {
            let _env = EnvVarGuard::set("QUAID_FULL_HASH_AUDIT_DAYS", "12");
            assert_eq!(configured_full_hash_audit_days(), 12);
            assert_eq!(full_hash_audit_ttl_cutoff(), "-12 days");
        }

        {
            let _env = EnvVarGuard::set("QUAID_FULL_HASH_AUDIT_DAYS", "-5");
            assert_eq!(configured_full_hash_audit_days(), 0);
            assert_eq!(full_hash_audit_ttl_cutoff(), "-0 days");
            assert_eq!(
                scheduled_full_hash_audit_budget(14),
                2,
                "a zero-day TTL may make every row due, but the serve loop must keep hashing bounded per cycle"
            );
        }

        {
            let _env = EnvVarGuard::set("QUAID_FULL_HASH_AUDIT_DAYS", "bogus");
            assert_eq!(
                configured_full_hash_audit_days(),
                DEFAULT_FULL_HASH_AUDIT_DAYS
            );
        }
    }

    #[test]
    fn scheduled_full_hash_audit_budget_spreads_work_across_audit_days() {
        let _guard = env_mutation_lock().lock().unwrap();

        {
            let _env = EnvVarGuard::set("QUAID_FULL_HASH_AUDIT_DAYS", "3");
            assert_eq!(scheduled_full_hash_audit_budget(0), 0);
            assert_eq!(scheduled_full_hash_audit_budget(1), 1);
            assert_eq!(scheduled_full_hash_audit_budget(7), 3);
            assert_eq!(scheduled_full_hash_audit_budget(10), 4);
        }

        {
            let _env = EnvVarGuard::set("QUAID_FULL_HASH_AUDIT_DAYS", "0");
            assert_eq!(
                scheduled_full_hash_audit_budget(7),
                1,
                "0 days still falls back to a bounded per-cycle budget"
            );
        }
    }

    #[test]
    fn finalize_outcome_label_covers_all_batch6_finalize_variants() {
        let cases = [
            (FinalizeOutcome::Finalized, "Finalized"),
            (FinalizeOutcome::Deferred, "Deferred"),
            (FinalizeOutcome::ManifestIncomplete, "ManifestIncomplete"),
            (FinalizeOutcome::IntegrityFailed, "IntegrityFailed"),
            (FinalizeOutcome::OrphanRecovered, "OrphanRecovered"),
            (FinalizeOutcome::Aborted, "Aborted"),
            (FinalizeOutcome::NoPendingWork, "NoPendingWork"),
        ];

        for (outcome, expected) in cases {
            assert_eq!(finalize_outcome_label(&outcome), expected);
        }
    }

    #[test]
    fn short_lived_owner_lease_heartbeats_and_cleans_up_residue() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());

        let lease = start_short_lived_owner_lease_with_interval(
            &conn,
            collection_id,
            Duration::from_secs(1),
        )
        .unwrap();
        let first_heartbeat: String = conn
            .query_row(
                "SELECT heartbeat_at FROM serve_sessions WHERE session_id = ?1",
                [lease.session_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();

        thread::sleep(Duration::from_millis(2200));

        let second_heartbeat: String = conn
            .query_row(
                "SELECT heartbeat_at FROM serve_sessions WHERE session_id = ?1",
                [lease.session_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(first_heartbeat, second_heartbeat);

        drop(lease);

        let row: (Option<String>, i64, i64) = conn
            .query_row(
                "SELECT active_lease_session_id,
                        (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1),
                        (SELECT COUNT(*) FROM serve_sessions)
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(row.0.is_none());
        assert_eq!(row.1, 0);
        assert_eq!(row.2, 0);
    }

    #[test]
    fn short_lived_owner_lease_releases_on_panic_unwind() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _lease = start_short_lived_owner_lease(&conn, collection_id).unwrap();
            panic!("boom");
        }));
        assert!(result.is_err());

        let row: (Option<String>, i64, i64) = conn
            .query_row(
                "SELECT active_lease_session_id,
                        (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1),
                        (SELECT COUNT(*) FROM serve_sessions)
                 FROM collections
                 WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(row.0.is_none());
        assert_eq!(row.1, 0);
        assert_eq!(row.2, 0);
    }

    #[test]
    fn short_lived_owner_lease_for_root_path_requires_existing_collection_rows() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let root_path = temp.path().display().to_string();

        match start_short_lived_owner_lease_for_root_path(&conn, &root_path) {
            Err(VaultSyncError::InvariantViolation { message }) => {
                assert!(message.contains("missing collection rows for short-lived owner lease"));
                assert!(message.contains(&root_path));
            }
            Ok(_) => panic!("expected InvariantViolationError"),
            Err(other) => panic!("expected InvariantViolationError, got {other:?}"),
        }
    }

    #[test]
    fn short_lived_owner_lease_for_root_path_claims_same_root_aliases_and_cleans_up() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let temp = tempfile::TempDir::new().unwrap();
        let root_path = temp.path().display().to_string();
        let work_id = insert_collection(&conn, "work", temp.path());
        let alias_id = insert_collection(&conn, "alias", temp.path());

        let lease = start_short_lived_owner_lease_for_root_path(&conn, &root_path).unwrap();
        let claimed_rows: Vec<(i64, Option<String>)> = conn
            .prepare(
                "SELECT id, active_lease_session_id
                 FROM collections
                 WHERE root_path = ?1
                 ORDER BY id",
            )
            .unwrap()
            .query_map([root_path.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            claimed_rows,
            vec![
                (work_id, Some(lease.session_id.clone())),
                (alias_id, Some(lease.session_id.clone()))
            ]
        );

        drop(lease);

        let row: (i64, i64, i64) = conn
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM collection_owners),
                     (SELECT COUNT(*) FROM serve_sessions),
                     (SELECT COUNT(*) FROM collections
                      WHERE root_path = ?1 AND active_lease_session_id IS NULL)",
                [root_path.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, 0);
        assert_eq!(row.1, 0);
        assert_eq!(row.2, 2);
    }

    #[test]
    fn serve_session_can_steal_cli_short_lived_lease() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let temp = tempfile::TempDir::new().unwrap();
        let root_path = temp.path().display().to_string();
        let collection_id = insert_collection(&conn, "work", temp.path());

        // CLI offline lease takes ownership.
        let _lease = start_short_lived_owner_lease_for_root_path(&conn, &root_path).unwrap();
        let cli_session_id = owner_session_id(&conn, collection_id).unwrap().unwrap();
        let cli_type: String = conn
            .query_row(
                "SELECT session_type FROM serve_sessions WHERE session_id = ?1",
                [cli_session_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cli_type, "cli");

        // A serve-type session can steal ownership; CLI sessions do not block serves.
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host) VALUES ('serve-take', 2, 'host')",
            [],
        )
        .unwrap();
        acquire_owner_lease(&conn, collection_id, "serve-take").unwrap();

        assert_eq!(
            owner_session_id(&conn, collection_id).unwrap().as_deref(),
            Some("serve-take")
        );
    }

    #[test]
    fn ensure_no_live_serve_owner_for_root_path_reports_same_root_alias_owner() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        insert_collection(&conn, "alias", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
             VALUES ('serve-live', 77, 'batch3-host', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-live')",
            [collection_id],
        )
        .unwrap();

        let error =
            ensure_no_live_serve_owner_for_root_path(&conn, &temp.path().display().to_string())
                .unwrap_err();
        let text = error.to_string();
        assert!(text.contains("ServeOwnsCollectionError"));
        assert!(text.contains("collection=work"));
        assert!(text.contains("owner_pid=77"));
        assert!(text.contains("owner_host=batch3-host"));
    }

    #[test]
    fn ensure_no_live_serve_owner_for_root_path_allows_stale_same_root_owner_residue() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        insert_collection(&conn, "work", temp.path());
        let alias_id = insert_collection(&conn, "alias", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
             VALUES ('serve-stale', 77, 'batch3-host', datetime('now', '-120 seconds'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-stale')",
            [alias_id],
        )
        .unwrap();

        ensure_no_live_serve_owner_for_root_path(&conn, &temp.path().display().to_string())
            .unwrap();
    }

    #[test]
    fn ensure_no_live_serve_owner_for_root_path_ignores_cli_session() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at, session_type)
             VALUES ('cli-lease', 99, 'cli-host', datetime('now'), 'cli')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'cli-lease')",
            [collection_id],
        )
        .unwrap();

        // A live CLI-type lease must not trigger a ServeOwnsCollectionError.
        ensure_no_live_serve_owner_for_root_path(&conn, &temp.path().display().to_string())
            .unwrap();
    }

    /// Regression: during the shutdown race window the serve session's `ipc_path`
    /// column may be cleared (by `cleanup_published_ipc_socket`) before the session
    /// row is unregistered.  `live_serve_endpoint_for_root_path` must treat a NULL
    /// `ipc_path` as "no live endpoint" (`Ok(None)`) rather than panicking with an
    /// `InvariantViolation`, so that `quaid put` falls back to the direct-write path
    /// and is still guarded by the owner-lease checks there.
    #[cfg(unix)]
    #[test]
    fn live_serve_endpoint_for_root_path_returns_ok_none_when_ipc_path_is_null() {
        let conn = open_test_db();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "shutdown-race", temp.path());
        // Insert a serve session that is still alive (fresh heartbeat) but whose
        // ipc_path has already been cleared by cleanup_published_ipc_socket.
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at, ipc_path)
             VALUES ('race-session', 1234, 'test-host', datetime('now'), NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'race-session')",
            [collection_id],
        )
        .unwrap();

        let result = live_serve_endpoint_for_root_path(&conn, &temp.path().display().to_string());

        assert!(
            matches!(result, Ok(None)),
            "expected Ok(None) for NULL ipc_path during shutdown race, got: {result:?}"
        );
    }

    /// Regression: `accept_ipc_clients` must offload each accepted connection to a
    /// dedicated thread via `thread::spawn` so that blocking IPC I/O (up to 5 s per
    /// read/write timeout) cannot stall the main serve loop and trigger a false-dead
    /// live-owner verdict.  We verify the structural invariant via source inspection.
    #[test]
    fn accept_ipc_clients_offloads_each_client_to_its_own_thread() {
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("core")
                .join("vault_sync.rs"),
        )
        .unwrap();
        let fn_start = source
            .find("fn accept_ipc_clients(")
            .expect("accept_ipc_clients fn present");
        // Boundary: the next fn after accept_ipc_clients is handle_ipc_client.
        let fn_end = source[fn_start..]
            .find("fn handle_ipc_client(")
            .map(|offset| fn_start + offset)
            .expect("handle_ipc_client fn follows accept_ipc_clients");
        let fn_body = &source[fn_start..fn_end];

        // The function must not call handle_ipc_client directly on the main loop thread.
        assert!(
            !fn_body.contains("handle_ipc_client(stream,"),
            "accept_ipc_clients must not call handle_ipc_client inline on the main thread"
        );
        // The function must use thread::spawn to offload.
        assert!(
            fn_body.contains("thread::spawn("),
            "accept_ipc_clients must offload each client via thread::spawn"
        );
        // The spawn closure must contain the handle_ipc_client call.
        let spawn_idx = fn_body.find("thread::spawn(").unwrap();
        let spawn_region = &fn_body[spawn_idx..];
        assert!(
            spawn_region.contains("handle_ipc_client("),
            "handle_ipc_client must be called inside the thread::spawn closure"
        );
        // The function must enforce the in-flight cap before spawning.
        assert!(
            fn_body.contains("IPC_HANDLER_LIMIT"),
            "accept_ipc_clients must reference IPC_HANDLER_LIMIT"
        );
        assert!(
            fn_body.contains("fetch_add("),
            "accept_ipc_clients must use fetch_add for the in-flight counter"
        );
        // Rollback path: fetch_sub must appear for the saturation branch.
        assert!(
            fn_body.contains("fetch_sub("),
            "accept_ipc_clients must roll back via fetch_sub when saturated"
        );
        // Guard must be created before the spawn closure.
        assert!(
            fn_body.contains("IpcHandlerGuard("),
            "accept_ipc_clients must construct an IpcHandlerGuard before spawning"
        );
    }

    /// Unit test: `IpcHandlerGuard` must decrement the counter when dropped,
    /// including via normal scope exit.
    #[cfg(unix)]
    #[test]
    fn ipc_handler_guard_decrements_on_drop() {
        let counter = Arc::new(AtomicUsize::new(1));
        {
            let _guard = IpcHandlerGuard(Arc::clone(&counter));
            // Counter unchanged while guard is alive.
            assert_eq!(counter.load(Ordering::SeqCst), 1);
        }
        // Guard dropped — counter must be back to 0.
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "IpcHandlerGuard must decrement the counter exactly once on drop"
        );
    }

    /// Structural: `IPC_HANDLER_LIMIT` must be a small positive value so the
    /// cap is intentional and not effectively infinite.
    ///
    /// These are compile-time assertions (`const _: () = assert!(...)`) rather
    /// than runtime `assert!` calls to avoid the `clippy::assertions_on_constants`
    /// lint — the values are constants, so the checks belong at compile time.
    #[cfg(unix)]
    #[test]
    fn ipc_handler_limit_is_small_and_positive() {
        const _: () = assert!(IPC_HANDLER_LIMIT > 0, "IPC_HANDLER_LIMIT must be > 0");
        const _: () = assert!(
            IPC_HANDLER_LIMIT <= 64,
            "IPC_HANDLER_LIMIT should be small (<=64); current value looks unintentionally large",
        );
    }

    #[cfg(unix)]
    #[test]
    fn plain_sync_reconciles_active_root_and_clears_needs_full_sync() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET needs_full_sync = 1 WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        fs::write(
            root.path().join("note.md"),
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nA body long enough to reconcile through the active-root path.\n",
        )
        .unwrap();

        let stats = sync_collection(&conn, "work").unwrap();
        let row: (String, i64, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT state, needs_full_sync, last_sync_at, active_lease_session_id
                 FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        let page_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE collection_id = ?1",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stats.walked, 1);
        assert_eq!(row.0, "active");
        assert_eq!(row.1, 0);
        assert!(row.2.is_some());
        assert!(row.3.is_none());
        assert_eq!(page_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn plain_sync_turns_duplicate_uuid_into_terminal_reconcile_halt() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let uuid = "01969f11-9448-7d79-8d3f-c68f54768888";
        let note_a = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nThis body is comfortably above the minimum size for rename inference.\n"
        );
        let note_b = format!(
            "---\nmemory_id: {uuid}\nslug: notes/b\ntitle: B\ntype: concept\n---\nThis second body is also comfortably above the minimum size for rename inference.\n"
        );
        fs::write(root.path().join("a.md"), note_a).unwrap();
        fs::write(root.path().join("b.md"), note_b).unwrap();

        let error = sync_collection(&conn, "work").unwrap_err().to_string();
        let row: (Option<String>, Option<String>, i64) = conn
            .query_row(
                "SELECT reconcile_halted_at, reconcile_halt_reason,
                        (SELECT COUNT(*) FROM pages WHERE collection_id = ?1)
                 FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert!(error.contains("ReconcileHaltedError"));
        assert!(error.contains("DuplicateUuidError"));
        assert!(row.0.is_some());
        assert_eq!(row.1.as_deref(), Some("duplicate_uuid"));
        assert_eq!(row.2, 0);
    }

    #[cfg(unix)]
    #[test]
    fn plain_sync_turns_trivial_hash_ambiguity_into_terminal_reconcile_halt() {
        let (_dir, _db_path, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        let content = concat!(
            "---\n",
            "slug: notes/template\n",
            "title: Template Note\n",
            "type: concept\n",
            "meta: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            "---\n",
            "Hi\n",
        );
        fs::write(root.path().join("template.md"), content).unwrap();
        let stat = crate::core::file_state::stat_file(&root.path().join("template.md")).unwrap();
        let sha256 = crate::core::file_state::hash_file(&root.path().join("template.md")).unwrap();
        conn.execute(
            "INSERT INTO pages (collection_id, slug, uuid, type, title, compiled_truth, timeline)
             VALUES (?1, 'notes/template', ?2, 'concept', 'Template', 'Hi', '')",
            params![collection_id, "01969f11-9448-7d79-8d3f-c68f54767777"],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO file_state
                 (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256)
             VALUES (?1, 'old-template.md', ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                collection_id,
                page_id,
                stat.mtime_ns,
                stat.ctime_ns,
                stat.size_bytes,
                stat.inode,
                sha256
            ],
        )
        .unwrap();
        crate::core::raw_imports::rotate_active_raw_import(
            &conn,
            page_id,
            "old-template.md",
            content.as_bytes(),
        )
        .unwrap();

        let error = sync_collection(&conn, "work").unwrap_err().to_string();
        let row: (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT reconcile_halted_at, reconcile_halt_reason
                 FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert!(error.contains("ReconcileHaltedError"));
        assert!(error.contains("UnresolvableTrivialContentError"));
        assert!(row.0.is_some());
        assert_eq!(row.1.as_deref(), Some("unresolvable_trivial_content"));
    }
}
