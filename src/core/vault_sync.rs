use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(unix)]
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
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
    RecommendedWatcher, RecursiveMode, Watcher,
};
#[cfg(all(test, unix))]
use rustix::fs::fsync;
#[cfg(all(test, unix))]
use std::io::Write;
#[cfg(all(test, unix))]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use tokio::sync::mpsc::{self, error::TryRecvError};

use crate::commands::get::get_page_by_key;
use crate::core::collections::{
    self, Collection, CollectionError, CollectionState, OpKind, SlugResolution,
};
#[cfg(all(test, unix))]
use crate::core::db;
#[cfg(unix)]
use crate::core::file_state;
#[cfg(unix)]
use crate::core::fs_safety;
use crate::core::markdown;
use crate::core::quarantine;
use crate::core::reconciler::{
    fresh_attach_reconcile_and_activate, full_hash_reconcile_authorized, reconcile,
    FullHashReconcileAuthorization, FullHashReconcileMode, ReconcileError, ReconcileStats,
};

const SESSION_LIVENESS_SECS: i64 = 15;
const HANDSHAKE_POLL_MS: u64 = 100;
const HANDSHAKE_TIMEOUT_SECS: u64 = 30;
const HEARTBEAT_INTERVAL_SECS: u64 = 5;
const DEFERRED_RETRY_SECS: u64 = 1;
const DEFAULT_MANIFEST_INCOMPLETE_ESCALATION_SECS: i64 = 1800;
const QUARANTINE_SWEEP_INTERVAL_SECS: u64 = 24 * 60 * 60;
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
}

#[cfg(unix)]
#[derive(Debug, Clone)]
struct SelfWriteDedupEntry {
    sha256: String,
    inserted_at: Instant,
}

#[cfg(unix)]
struct CollectionWatcherState {
    root_path: PathBuf,
    generation: i64,
    receiver: mpsc::Receiver<WatchEvent>,
    _watcher: RecommendedWatcher,
    buffer: WatchBatchBuffer,
}

#[cfg(unix)]
#[derive(Debug, Default)]
struct WatchBatchBuffer {
    dirty_paths: HashSet<PathBuf>,
    native_renames: Vec<crate::core::reconciler::NativeRename>,
    debounce_deadline: Option<Instant>,
}

#[cfg(unix)]
#[derive(Debug)]
enum WatchEvent {
    DirtyPath(PathBuf),
    NativeRename(crate::core::reconciler::NativeRename),
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

    #[error("ServeOwnsCollectionError: collection={collection_name} owner_session_id={owner_session_id}")]
    ServeOwnsCollectionError {
        collection_name: String,
        owner_session_id: String,
    },

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
        "HandshakeTimeoutError: collection={collection_name} expected_session_id={expected_session_id} reload_generation={reload_generation}"
    )]
    HandshakeTimeout {
        collection_name: String,
        expected_session_id: String,
        reload_generation: i64,
    },

    #[error(
        "NewRootVerificationFailedError: collection={collection_name} missing={missing} mismatched={mismatched} extra={extra}"
    )]
    NewRootVerificationFailed {
        collection_name: String,
        missing: usize,
        mismatched: usize,
        extra: usize,
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
fn inspect_fs_precondition(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
) -> Result<FsPreconditionInspection, VaultSyncError> {
    let relative_path_str = relative_path.to_string_lossy().into_owned();
    let stored_row = file_state::get_file_state(conn, collection_id, &relative_path_str)?;
    let root_fd = fs_safety::open_root_fd(root_path)?;
    let target_name =
        relative_path
            .file_name()
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: format!(
                    "relative path has no terminal component: {}",
                    relative_path.display()
                ),
            })?;
    let parent_fd = match fs_safety::walk_to_parent(&root_fd, relative_path) {
        Ok(parent_fd) => Some(parent_fd),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };

    let current_stat = match parent_fd.as_ref() {
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
pub(crate) fn check_fs_precondition_before_sentinel(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
) -> Result<FsPreconditionOutcome, VaultSyncError> {
    Ok(inspect_fs_precondition(conn, collection_id, root_path, relative_path)?.outcome)
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
             ), 0) AS embedding_queue_depth,
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
                row.get::<_, i64>(17)? != 0,
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
        if state == CollectionState::Active.as_str() && !needs_full_sync {
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
    registries
        .dedup
        .lock()
        .map_err(|_| VaultSyncError::RegistryPoisoned { registry: "dedup" })?
        .insert(key.to_owned());
    Ok(())
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
    let _ = quarantine::sweep_expired_quarantined_pages(conn);
    let _ = run_rcrt_pass(conn, session_id);
    sync_supervisor_handles(conn, session_id)?;
    Ok(())
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

pub fn session_is_live(conn: &Connection, session_id: &str) -> Result<bool, VaultSyncError> {
    let live = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM serve_sessions
             WHERE session_id = ?1
               AND heartbeat_at >= datetime('now', ?2)
         )",
        params![session_id, format!("-{SESSION_LIVENESS_SECS} seconds")],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(live != 0)
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
    let existing_owner = owner_session_id(conn, collection_id)?;
    match existing_owner {
        Some(owner) if owner != session_id && session_is_live(conn, &owner)? => {
            let collection = load_collection_by_id(conn, collection_id)?;
            return Err(VaultSyncError::ServeOwnsCollectionError {
                collection_name: collection.name,
                owner_session_id: owner,
            });
        }
        _ => {}
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
fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

#[cfg(unix)]
fn relative_markdown_path(root_path: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(root_path).ok()?;
    is_markdown_path(relative).then(|| relative.to_path_buf())
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
fn start_collection_watcher(
    collection_id: i64,
    root_path: &Path,
    db_path: &str,
) -> Result<CollectionWatcherState, VaultSyncError> {
    let (sender, receiver) = mpsc::channel(WATCH_CHANNEL_CAPACITY);
    let watch_root = root_path.to_path_buf();
    let callback_root = watch_root.clone();
    let db_path = db_path.to_owned();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<NotifyEvent>| {
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
    })
    .map_err(|error| VaultSyncError::InvariantViolation {
        message: format!("failed to create watcher for collection_id={collection_id}: {error}"),
    })?;
    watcher
        .configure(NotifyConfig::default())
        .map_err(|error| VaultSyncError::InvariantViolation {
            message: format!(
                "failed to configure watcher for collection_id={collection_id}: {error}"
            ),
        })?;
    watcher
        .watch(&watch_root, RecursiveMode::Recursive)
        .map_err(|error| VaultSyncError::InvariantViolation {
            message: format!(
                "failed to watch root {} for collection_id={collection_id}: {error}",
                watch_root.display()
            ),
        })?;
    Ok(CollectionWatcherState {
        root_path: watch_root,
        generation: 0,
        receiver,
        _watcher: watcher,
        buffer: WatchBatchBuffer::default(),
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
        let needs_replace = watchers
            .get(&collection_id)
            .map(|state| state.root_path != root_path || state.generation != generation)
            .unwrap_or(true);
        if !needs_replace {
            continue;
        }
        let mut state = start_collection_watcher(collection_id, &root_path, db_path)?;
        state.generation = generation;
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
    let debounce = watch_debounce_duration();
    loop {
        match state.receiver.try_recv() {
            Ok(WatchEvent::DirtyPath(path)) => {
                state.buffer.dirty_paths.insert(path);
                state.buffer.debounce_deadline = Some(Instant::now() + debounce);
            }
            Ok(WatchEvent::NativeRename(rename)) => {
                state.buffer.native_renames.push(rename);
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

    let Some(deadline) = state.buffer.debounce_deadline else {
        return Ok(());
    };
    if Instant::now() < deadline {
        return Ok(());
    }

    let native_renames = std::mem::take(&mut state.buffer.native_renames);
    state.buffer.dirty_paths.clear();
    state.buffer.debounce_deadline = None;
    run_watcher_reconcile(conn, collection_id, &native_renames)
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
    let expected_session_id = owner_session_id(conn, collection_id)?.ok_or_else(|| {
        VaultSyncError::ServeOwnsCollectionError {
            collection_name: collection.name.clone(),
            owner_session_id: "none".to_owned(),
        }
    })?;
    if !session_is_live(conn, &expected_session_id)? {
        return Err(VaultSyncError::ServeDiedDuringHandshake {
            collection_name: collection.name,
            expected_session_id,
        });
    }

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
        if owner_session_id(conn, collection_id)?.as_deref() != Some(expected_session_id) {
            return Err(VaultSyncError::ServeDiedDuringHandshake {
                collection_name: collection.name,
                expected_session_id: expected_session_id.to_owned(),
            });
        }
        if !session_is_live(conn, expected_session_id)? {
            return Err(VaultSyncError::ServeDiedDuringHandshake {
                collection_name: collection.name,
                expected_session_id: expected_session_id.to_owned(),
            });
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

pub fn start_serve_runtime(db_path: String) -> Result<ServeRuntime, VaultSyncError> {
    init_process_registries()?;
    let conn = Connection::open(&db_path)?;
    sweep_stale_sessions(&conn)?;
    let session_id = register_session(&conn)?;
    run_startup_sequence(&conn, Path::new(&db_path), &session_id)?;
    #[cfg(unix)]
    let mut watchers: HashMap<i64, CollectionWatcherState> = HashMap::new();
    #[cfg(unix)]
    sync_collection_watchers(&conn, &db_path, &mut watchers)?;
    let mut stmt = conn.prepare("SELECT id, reload_generation FROM collections")?;
    let initial_generations = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
        .collect::<Result<HashMap<_, _>, _>>()?;
    drop(stmt);
    drop(conn);

    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = Arc::clone(&stop);
    let session_id_for_thread = session_id.clone();
    let handle = thread::spawn(move || {
        let mut last_heartbeat = Instant::now();
        let mut last_quarantine_sweep = Instant::now();
        let mut last_generations = initial_generations;
        #[cfg(unix)]
        let mut watchers = watchers;
        #[cfg(unix)]
        let mut last_dedup_sweep = Instant::now();
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
                    let _ = sync_collection_watchers(&conn, &db_path, &mut watchers);
                    for (collection_id, state) in &mut watchers {
                        let _ = poll_collection_watcher(&conn, *collection_id, state);
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
                let _ = run_rcrt_pass(&conn, &session_id_for_thread);
                let _ = sync_supervisor_handles(&conn, &session_id_for_thread);
            }
            thread::sleep(Duration::from_millis(DEFERRED_RETRY_SECS * 200));
        }
        if let Ok(conn) = Connection::open(&db_path) {
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

    if online {
        let (_, expected_session_id, generation) =
            mark_collection_restoring_for_handshake(conn, collection.id)?;
        wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?;
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
        let session_id = register_session(conn)?;
        let command_id = Uuid::now_v7().to_string();
        acquire_owner_lease(conn, collection.id, &session_id)?;
        let _lease_guard = LeaseGuard {
            db_path: database_path(conn).ok(),
            collection_id: collection.id,
            session_id: session_id.clone(),
        };
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 restore_lease_session_id = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![collection.id, session_id],
        )?;
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
        unregister_session(conn, &session_id)?;
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
    let summary = verify_remap_root(conn, &collection, new_root)?;

    if online {
        let (_, expected_session_id, generation) =
            mark_collection_restoring_for_handshake(conn, collection.id)?;
        wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?;
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
    } else {
        let session_id = register_session(conn)?;
        acquire_owner_lease(conn, collection.id, &session_id)?;
        conn.execute(
            "UPDATE collections
             SET root_path = ?2,
                 state = 'restoring',
                 needs_full_sync = 1,
                 restore_lease_session_id = ?3,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![collection.id, new_root.display().to_string(), session_id],
        )?;
        conn.execute(
            "DELETE FROM file_state WHERE collection_id = ?1",
            [collection.id],
        )?;
        unregister_session(conn, &session_id)?;
    }

    Ok(summary)
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
    let missing = page_rows
        .len()
        .saturating_sub(page_matches.resolved_page_ids.len());
    let mismatched = page_matches.mismatched_pages;
    let extra = file_rows
        .len()
        .saturating_sub(page_matches.resolved_file_paths.len());
    if missing != 0 || mismatched != 0 || extra != 0 {
        return Err(VaultSyncError::NewRootVerificationFailed {
            collection_name: collection.name.clone(),
            missing,
            mismatched,
            extra,
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
    collection_id: i64,
    session_id: String,
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if let Some(db_path) = self.db_path.as_deref() {
            if let Ok(conn) = Connection::open(db_path) {
                let _ = release_owner_lease(&conn, self.collection_id, &self.session_id);
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

fn start_short_lived_owner_lease_with_interval(
    conn: &Connection,
    collection_id: i64,
    heartbeat_interval: Duration,
) -> Result<ShortLivedLease, VaultSyncError> {
    let db_path = database_path(conn)?;
    let session_id = register_session(conn)?;
    if let Err(err) = acquire_owner_lease(conn, collection_id, &session_id) {
        let _ = unregister_session(conn, &session_id);
        return Err(err);
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
            collection_id,
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
    body_size_bytes: usize,
    has_nonempty_body: bool,
}

#[derive(Debug, Clone)]
struct NewRootFileRow {
    relative_path: PathBuf,
    uuid: Option<String>,
    sha256: String,
    body_size_bytes: usize,
    has_nonempty_body: bool,
}

struct PageMatchResolution {
    resolved_page_ids: HashSet<i64>,
    resolved_file_paths: HashSet<PathBuf>,
    mismatched_pages: usize,
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
                body_size_bytes: body.trim().len(),
                has_nonempty_body: !body.trim().is_empty(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn load_new_root_files(root: &Path) -> Result<Vec<NewRootFileRow>, VaultSyncError> {
    let walked = walk_tree(root)?;
    let mut rows = Vec::new();
    for relative_path in walked.keys() {
        if relative_path == Path::new(".quaidignore") {
            continue;
        }
        let bytes = fs::read(root.join(relative_path))?;
        let text = String::from_utf8_lossy(&bytes);
        let (frontmatter, body) = markdown::parse_frontmatter(&text);
        rows.push(NewRootFileRow {
            relative_path: relative_path.clone(),
            uuid: frontmatter.get("memory_id").cloned(),
            sha256: sha256_hex(&bytes),
            body_size_bytes: body.trim().len(),
            has_nonempty_body: !body.trim().is_empty(),
        });
    }
    Ok(rows)
}

fn resolve_page_matches(pages: &[RemapPageRow], files: &[NewRootFileRow]) -> PageMatchResolution {
    let mut files_by_uuid: HashMap<&str, Vec<&NewRootFileRow>> = HashMap::new();
    let mut files_by_hash: HashMap<&str, Vec<&NewRootFileRow>> = HashMap::new();
    for file in files {
        if let Some(uuid) = file.uuid.as_deref() {
            files_by_uuid.entry(uuid).or_default().push(file);
        }
        files_by_hash
            .entry(file.sha256.as_str())
            .or_default()
            .push(file);
    }

    let mut resolved_page_ids = HashSet::new();
    let mut resolved_file_paths = HashSet::new();
    let mut mismatched_pages = 0;

    for page in pages {
        if let Some(uuid) = page.uuid.as_deref() {
            if let Some(matches) = files_by_uuid.get(uuid) {
                if matches.len() == 1 {
                    resolved_page_ids.insert(page.page_id);
                    resolved_file_paths.insert(matches[0].relative_path.clone());
                    continue;
                }
                mismatched_pages += 1;
                continue;
            }
        }
        let hash_matches = files_by_hash
            .get(page.sha256.as_str())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|file| {
                page.body_size_bytes >= 64
                    && page.has_nonempty_body
                    && file.body_size_bytes >= 64
                    && file.has_nonempty_body
            })
            .collect::<Vec<_>>();
        if hash_matches.len() == 1 {
            resolved_page_ids.insert(page.page_id);
            resolved_file_paths.insert(hash_matches[0].relative_path.clone());
        } else {
            let _ = &page.slug;
        }
    }

    PageMatchResolution {
        resolved_page_ids,
        resolved_file_paths,
        mismatched_pages,
    }
}

fn take_tree_fence(root: &Path) -> Result<BTreeMap<String, (u64, u128)>, VaultSyncError> {
    let mut fence = BTreeMap::new();
    for (relative_path, metadata) in walk_tree(root)? {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        fence.insert(path_string(&relative_path), (metadata.len(), modified));
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
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                walk_dir(root, &path, output)?;
            } else {
                let relative = path
                    .strip_prefix(root)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|_| path.clone());
                output.insert(relative, metadata);
            }
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
            _watcher: watcher,
            buffer: WatchBatchBuffer::default(),
        };

        let err = poll_collection_watcher(&conn, 42, &mut state).unwrap_err();

        assert!(matches!(err, VaultSyncError::InvariantViolation { .. }));
        assert!(err.to_string().contains("watch channel disconnected"));
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
            snippet.contains("state.root_path != root_path || state.generation != generation"),
            "watcher sync must replace watchers when root or reload_generation changes: {snippet}"
        );
        assert!(
            snippet.contains("state.generation = generation;"),
            "replacement watchers must inherit the new reload_generation: {snippet}"
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
        let conn = open_test_db();
        let source_root = tempfile::TempDir::new().unwrap();
        let target_parent = tempfile::TempDir::new().unwrap();
        let target_root = target_parent.path().join("restored");
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
                collection_id,
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

    #[test]
    fn offline_restore_keeps_write_gate_closed_until_rcrt_owns_attach() {
        let conn = open_test_db();
        let source_root = tempfile::TempDir::new().unwrap();
        let target_parent = tempfile::TempDir::new().unwrap();
        let target_root = target_parent.path().join("restored");
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

        begin_restore(&conn, "work", &target_root, false).unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(collection.state, CollectionState::Restoring);
        assert!(collection.needs_full_sync);
        assert!(collection.active_lease_session_id.is_none());
        assert!(collection.restore_lease_session_id.is_none());
        assert!(owner_session_id(&conn, collection_id).unwrap().is_none());
        let error = ensure_collection_write_allowed(&conn, collection_id).unwrap_err();
        assert!(error.to_string().contains("CollectionRestoringError"));
    }

    #[test]
    fn offline_remap_keeps_write_gate_closed_until_rcrt_owns_attach() {
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

        remap_collection(&conn, "work", new_root.path(), false).unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        assert_eq!(collection.root_path, new_root.path().display().to_string());
        assert_eq!(collection.state, CollectionState::Restoring);
        assert!(collection.needs_full_sync);
        assert!(collection.active_lease_session_id.is_none());
        assert!(collection.restore_lease_session_id.is_none());
        assert!(owner_session_id(&conn, collection_id).unwrap().is_none());
        let error = ensure_collection_write_allowed(&conn, collection_id).unwrap_err();
        assert!(error.to_string().contains("CollectionRestoringError"));
    }

    #[test]
    fn verify_remap_root_detects_missing_extra_and_mismatched_files() {
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
            new_root.path().join("notes").join("extra.md"),
            b"extra file",
        )
        .unwrap();

        let collection = load_collection_by_id(&conn, collection_id).unwrap();
        let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();
        assert!(error.to_string().contains("NewRootVerificationFailedError"));
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

        let row: (
            String,
            String,
            i64,
            Option<String>,
            i64,
            i64,
            i64,
            i64,
            Option<String>,
        ) = wait_for_collection_update(
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

        let verify = Connection::open(&db_path).unwrap();
        let row: (String, String, i64, Option<String>, Option<String>, i64) = verify
            .query_row(
                "SELECT state,
                        root_path,
                        needs_full_sync,
                        pending_root_path,
                        restore_command_id,
                        (SELECT COUNT(*) FROM collection_owners WHERE collection_id = ?1 AND session_id = ?2)
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

        drop(runtime);
    }

    #[cfg(unix)]
    #[test]
    fn start_serve_runtime_bootstraps_recovery_directories_for_existing_collections() {
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
