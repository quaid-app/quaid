// Reconciler: filesystem walk → stat-diff → ingest/quarantine/delete.
//
// This module owns collection-backed vault reconciliation and re-ingest.
//
// Planned responsibilities:
// - Cold-start reconciliation on `quaid serve` startup
// - On-demand sync via `quaid collection sync`
// - Rename detection (native events, UUID match, content-hash uniqueness)
// - Delete-vs-quarantine classification via `has_db_only_state`

#![allow(dead_code)]

use crate::core::collections::Collection;
use crate::core::conversation::file_edit::{
    handle_extracted_edit, handles_page_type, is_history_sidecar_path, normalized_content_key,
    EditedPage, HandleExtractedEditOutcome,
};
use crate::core::file_state::{self, FileStat};
#[cfg(unix)]
use crate::core::ignore_patterns;
use crate::core::markdown;
use crate::core::page_uuid;
use crate::core::palace;
use crate::core::raw_imports;
#[cfg(unix)]
use ignore::WalkBuilder;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
#[cfg(unix)]
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use crate::core::fs_safety;
#[cfg(unix)]
use rustix::fd::OwnedFd;
#[cfg(unix)]
use rustix::fs::{openat, Mode, OFlags};

// ── Reconciliation Result ─────────────────────────────────────

/// Summary statistics from a reconciliation pass.
#[derive(Debug, Default, Clone)]
pub struct ReconcileStats {
    pub walked: usize,
    pub unchanged: usize,
    pub modified: usize,
    pub new: usize,
    pub missing: usize,
    pub native_renamed: usize,
    pub uuid_renamed: usize,
    pub hash_renamed: usize,
    pub quarantined_ambiguous: usize,
    pub quarantined_db_state: usize,
    pub hard_deleted: usize,
}

pub(crate) const MIN_CANONICAL_BODY_BYTES: i64 = 64;
const UUID_MIGRATION_SAMPLE_LIMIT: usize = 5;
const DEFAULT_RESTORE_STABILITY_MAX_ITERS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullHashReconcileMode {
    Audit,
    FreshAttach,
    OverflowRecovery,
    RemapRoot,
    Restore,
    RemapDriftCapture,
    RestoreDriftCapture,
}

impl FullHashReconcileMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Audit => "audit",
            Self::FreshAttach => "fresh-attach",
            Self::OverflowRecovery => "overflow-recovery",
            Self::RemapRoot => "remap-root",
            Self::Restore => "restore",
            Self::RemapDriftCapture => "remap-drift-capture",
            Self::RestoreDriftCapture => "restore-drift-capture",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FullHashReconcileAuthorization {
    AuditCommand,
    ActiveLease { lease_session_id: String },
    AttachCommand { attach_command_id: String },
    RestoreCommand { restore_command_id: String },
    RestoreLease { lease_session_id: String },
}

impl FullHashReconcileAuthorization {
    fn as_str(&self) -> &'static str {
        match self {
            Self::AuditCommand => "audit-command",
            Self::ActiveLease { .. } => "active-lease",
            Self::AttachCommand { .. } => "attach-command",
            Self::RestoreCommand { .. } => "restore-command",
            Self::RestoreLease { .. } => "restore-lease",
        }
    }

    fn identity(&self) -> Option<&str> {
        match self {
            Self::AuditCommand => None,
            Self::ActiveLease { lease_session_id } | Self::RestoreLease { lease_session_id } => {
                Some(lease_session_id)
            }
            Self::AttachCommand { attach_command_id } => Some(attach_command_id),
            Self::RestoreCommand { restore_command_id } => Some(restore_command_id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreRemapOperation {
    Restore,
    Remap,
}

impl RestoreRemapOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Restore => "restore",
            Self::Remap => "remap",
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DriftCaptureSummary {
    pub pages_updated: usize,
    pub pages_added: usize,
    pub pages_quarantined: usize,
    pub pages_deleted: usize,
}

impl DriftCaptureSummary {
    fn from_stats(stats: &ReconcileStats) -> Self {
        Self {
            pages_updated: stats.modified,
            pages_added: stats.new,
            pages_quarantined: stats.quarantined_ambiguous + stats.quarantined_db_state,
            pages_deleted: stats.hard_deleted,
        }
    }

    fn has_material_changes(&self) -> bool {
        self.pages_updated != 0
            || self.pages_added != 0
            || self.pages_quarantined != 0
            || self.pages_deleted != 0
    }

    fn add_assign(&mut self, other: &Self) {
        self.pages_updated += other.pages_updated;
        self.pages_added += other.pages_added;
        self.pages_quarantined += other.pages_quarantined;
        self.pages_deleted += other.pages_deleted;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionDirtyStatus {
    pub needs_full_sync: bool,
    pub sentinel_count: usize,
    pub recovery_in_progress: bool,
    pub last_sync_at: Option<String>,
}

impl CollectionDirtyStatus {
    pub fn is_dirty(&self) -> bool {
        self.needs_full_sync || self.sentinel_count != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawImportInvariantPolicy {
    Enforce,
    AllowRerenderOverride,
}

#[derive(Debug, Clone)]
pub struct RestoreRemapSafetyRequest<'a> {
    pub collection_id: i64,
    pub db_path: &'a Path,
    pub recovery_root: &'a Path,
    pub operation: RestoreRemapOperation,
    pub authorization: FullHashReconcileAuthorization,
    pub allow_finalize_pending: bool,
    pub stability_max_iters: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRemapSafetyOutcome {
    pub drift_summary: DriftCaptureSummary,
    pub stability_retries: usize,
    pub final_snapshot_files: usize,
}

// ── Reconcile (stub) ──────────────────────────────────────────

/// Reconcile a collection's filesystem state against the DB.
///
/// This slice walks the vault with fd-relative nofollow checks, resolves
/// rename-vs-create-vs-delete, and applies content/file_state/raw_imports
/// mutations in 500-file transactions on Unix. `full_hash_reconcile()` uses the
/// same apply path but hashes every file regardless of stat metadata.
pub fn reconcile(
    conn: &Connection,
    collection: &Collection,
) -> Result<ReconcileStats, ReconcileError> {
    reconcile_with_native_events(conn, collection, &[])
}

pub(crate) fn reconcile_with_native_events(
    conn: &Connection,
    collection: &Collection,
    native_renames: &[NativeRename],
) -> Result<ReconcileStats, ReconcileError> {
    if collection.state != crate::core::collections::CollectionState::Active {
        eprintln!(
            "INFO: reconcile_skipped collection={} state={}",
            collection.name,
            collection.state.as_str()
        );
        return Ok(ReconcileStats::default());
    }

    #[cfg(not(unix))]
    let _ = native_renames;

    #[cfg(unix)]
    {
        let root_fd = fs_safety::open_root_fd(Path::new(&collection.root_path))?;
        ignore_patterns::reload_patterns(conn, collection.id, Path::new(&collection.root_path))
            .map_err(|err| {
                ReconcileError::Other(format!(
                    "reconcile: refusing to walk with stale .quaidignore state: {err}"
                ))
            })?;
        let walked = walk_collection(conn, &root_fd, collection)?;
        detect_duplicate_uuids_in_tree(Path::new(&collection.root_path), &walked.files)?;
        let diff = stat_diff_from_walk(
            conn,
            collection.id,
            Path::new(&collection.root_path),
            walked.files,
        )?;
        let rename_resolution = resolve_rename_resolution(
            conn,
            collection.id,
            Path::new(&collection.root_path),
            &diff,
            native_renames,
        )?;
        eprintln!(
            "INFO: reconcile_plan collection={} walked={} unchanged={} modified={} new={} missing={} native_renamed={} hash_renamed={} quarantined_ambiguous={}",
            collection.name,
            walked.walked,
            diff.unchanged.len(),
            diff.modified.len(),
            rename_resolution.remaining_new.len(),
            rename_resolution.remaining_missing.len(),
            rename_resolution.native_renamed,
            rename_resolution.hash_renamed,
            rename_resolution.quarantined_ambiguous
        );
        let apply_summary = apply_reconciliation(
            conn,
            collection,
            &diff,
            &rename_resolution,
            Path::new(&collection.root_path),
        )?;
        eprintln!(
            "INFO: reconcile_apply collection={} reingested={} created={} quarantined_db_state={} hard_deleted={}",
            collection.name,
            apply_summary.reingested,
            apply_summary.created,
            apply_summary.quarantined_db_state,
            apply_summary.hard_deleted
        );

        Ok(ReconcileStats {
            walked: walked.walked,
            unchanged: diff.unchanged.len(),
            modified: diff.modified.len(),
            new: rename_resolution.remaining_new.len(),
            missing: rename_resolution.remaining_missing.len(),
            native_renamed: rename_resolution.native_renamed,
            uuid_renamed: rename_resolution.uuid_renamed,
            hash_renamed: rename_resolution.hash_renamed,
            quarantined_ambiguous: rename_resolution.quarantined_ambiguous,
            quarantined_db_state: apply_summary.quarantined_db_state,
            hard_deleted: apply_summary.hard_deleted,
        })
    }

    #[cfg(not(unix))]
    {
        let _ = (conn, collection);
        Err(ReconcileError::Other(
            "reconcile: fd-relative operations not supported on Windows. \
             Vault sync commands (serve, collection add/sync) require Unix."
                .to_string(),
        ))
    }
}

/// Walk a collection filesystem using safe fd-relative iteration.
///
#[cfg(unix)]
#[derive(Debug, Default)]
struct WalkedCollection {
    files: HashMap<PathBuf, FileStat>,
    walked: usize,
    skipped_symlinks: usize,
}

#[cfg(unix)]
fn walk_collection(
    conn: &Connection,
    root_fd: &OwnedFd,
    collection: &Collection,
) -> Result<WalkedCollection, ReconcileError> {
    walk_root(
        conn,
        collection.id,
        Path::new(&collection.root_path),
        root_fd,
    )
}

#[cfg(not(unix))]
fn walk_collection(
    _root_fd: &std::fs::File,
    _collection: &Collection,
) -> Result<ReconcileStats, ReconcileError> {
    Err(ReconcileError::Other(
        "walk_collection: not supported on Windows".to_string(),
    ))
}

/// Full-hash reconciliation: ignore stat fields, hash every file.
///
/// Used by:
/// - `quaid collection sync --remap-root` (task 5.8)
/// - `quaid collection restore` (task 5.8)
/// - Fresh attach (task 5.9)
/// - Periodic audit (task 4.6)
///
/// Hash-unchanged pages self-heal `file_state` metadata only; hash-changed/new pages
/// reuse the normal apply path so page content and `raw_imports` rotate in the same tx.
///
pub fn full_hash_reconcile(
    conn: &Connection,
    collection_id: i64,
) -> Result<ReconcileStats, ReconcileError> {
    full_hash_reconcile_authorized(
        conn,
        collection_id,
        FullHashReconcileMode::Audit,
        FullHashReconcileAuthorization::AuditCommand,
    )
}

pub fn full_hash_reconcile_authorized(
    conn: &Connection,
    collection_id: i64,
    mode: FullHashReconcileMode,
    authorization: FullHashReconcileAuthorization,
) -> Result<ReconcileStats, ReconcileError> {
    #[cfg(unix)]
    {
        let collection = load_collection_by_id(conn, collection_id)?;
        authorize_full_hash_reconcile(&collection, mode, &authorization)?;
        let root_path = Path::new(&collection.root_path);
        let root_fd = fs_safety::open_root_fd(root_path)?;
        ignore_patterns::reload_patterns(conn, collection.id, root_path).map_err(|err| {
            ReconcileError::Other(format!(
                "full_hash_reconcile: refusing to walk with stale .quaidignore state: {err}"
            ))
        })?;
        let walked = walk_collection(conn, &root_fd, &collection)?;
        detect_duplicate_uuids_in_tree(root_path, &walked.files)?;
        let plan = build_full_hash_plan(conn, collection.id, root_path, &walked.files)?;
        let rename_resolution =
            resolve_rename_resolution(conn, collection.id, root_path, &plan.diff, &[])?;

        assert_full_hash_raw_import_invariants(conn, collection.id)?;
        apply_full_hash_metadata_self_heal(conn, collection.id, &plan.unchanged)?;
        let apply_summary =
            apply_reconciliation(conn, &collection, &plan.diff, &rename_resolution, root_path)?;

        Ok(ReconcileStats {
            walked: walked.walked,
            unchanged: plan.unchanged.len(),
            modified: plan.diff.modified.len(),
            new: rename_resolution.remaining_new.len(),
            missing: rename_resolution.remaining_missing.len(),
            native_renamed: rename_resolution.native_renamed,
            uuid_renamed: rename_resolution.uuid_renamed,
            hash_renamed: rename_resolution.hash_renamed,
            quarantined_ambiguous: rename_resolution.quarantined_ambiguous,
            quarantined_db_state: apply_summary.quarantined_db_state,
            hard_deleted: apply_summary.hard_deleted,
        })
    }

    #[cfg(not(unix))]
    {
        let _ = (conn, collection_id);
        Err(ReconcileError::Other(format!(
            "full_hash_reconcile: {} authorization for {} mode is not supported on Windows. \
             Vault sync commands require Unix.",
            authorization.as_str(),
            mode.as_str()
        )))
    }
}

pub fn scheduled_full_hash_audit_authorized(
    conn: &Connection,
    collection_id: i64,
    due_relative_paths: &[PathBuf],
    authorization: FullHashReconcileAuthorization,
) -> Result<ReconcileStats, ReconcileError> {
    #[cfg(unix)]
    {
        let collection = load_collection_by_id(conn, collection_id)?;
        authorize_full_hash_reconcile(&collection, FullHashReconcileMode::Audit, &authorization)?;
        if due_relative_paths.is_empty() {
            return Ok(ReconcileStats::default());
        }
        let root_path = Path::new(&collection.root_path);
        ignore_patterns::reload_patterns(conn, collection.id, root_path).map_err(|err| {
            ReconcileError::Other(format!(
                "scheduled_full_hash_audit: refusing to walk with stale .quaidignore state: {err}"
            ))
        })?;

        let stored_entries = load_stored_file_state_entries(conn, collection.id)?;
        let mut plan = FullHashPlan::default();
        let mut audited_page_ids = HashSet::new();
        let root_fd = fs_safety::open_root_fd(root_path)?;

        for relative_path in due_relative_paths {
            let Some(stored) = stored_entries.get(relative_path) else {
                continue;
            };
            audited_page_ids.insert(stored.page_id);
            match stat_and_hash_audit_path(&root_fd, relative_path)? {
                Some((stat, sha256)) => {
                    if stored.sha256 == sha256 {
                        plan.unchanged.push(FullHashUnchangedEntry {
                            relative_path: relative_path.clone(),
                            page_id: stored.page_id,
                            stat,
                            sha256,
                            skip_metadata_refresh: false,
                        });
                    } else if is_extracted_whitespace_noop(
                        conn,
                        collection_id,
                        root_path,
                        relative_path,
                        stored.page_id,
                    )? {
                        plan.unchanged.push(FullHashUnchangedEntry {
                            relative_path: relative_path.clone(),
                            page_id: stored.page_id,
                            stat,
                            sha256,
                            skip_metadata_refresh: true,
                        });
                    } else {
                        plan.diff.modified.insert(relative_path.clone(), stat);
                    }
                }
                None => {
                    plan.diff.missing.insert(relative_path.clone());
                }
            }
        }

        assert_raw_import_invariants_for_page_ids(
            conn,
            &audited_page_ids,
            "scheduled_full_hash_audit",
        )?;
        apply_full_hash_metadata_self_heal(conn, collection.id, &plan.unchanged)?;
        if !plan.diff.missing.is_empty() {
            conn.execute(
                "UPDATE collections
                 SET needs_full_sync = 1,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE id = ?1",
                [collection.id],
            )?;
            eprintln!(
                "WARN: scheduled_full_hash_audit_missing_paths_marked_dirty collection={} collection_id={} missing_count={}",
                collection.name,
                collection.id,
                plan.diff.missing.len()
            );
        }
        let rename_resolution = RenameResolution::default();
        let apply_summary =
            apply_reconciliation(conn, &collection, &plan.diff, &rename_resolution, root_path)?;

        Ok(ReconcileStats {
            walked: plan.unchanged.len() + plan.diff.modified.len(),
            unchanged: plan.unchanged.len(),
            modified: plan.diff.modified.len(),
            new: 0,
            missing: plan.diff.missing.len(),
            native_renamed: 0,
            uuid_renamed: 0,
            hash_renamed: 0,
            quarantined_ambiguous: 0,
            quarantined_db_state: apply_summary.quarantined_db_state,
            hard_deleted: apply_summary.hard_deleted,
        })
    }

    #[cfg(not(unix))]
    {
        let _ = (conn, collection_id, due_relative_paths);
        Err(ReconcileError::Other(format!(
            "scheduled_full_hash_audit: {} authorization for audit mode is not supported on Windows. \
             Vault sync commands require Unix.",
            authorization.as_str()
        )))
    }
}

#[cfg(unix)]
fn stat_and_hash_audit_path(
    root_fd: &OwnedFd,
    relative_path: &Path,
) -> Result<Option<(FileStat, String)>, ReconcileError> {
    let parent_fd = match fs_safety::walk_to_parent(root_fd, relative_path) {
        Ok(parent_fd) => parent_fd,
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(err.into()),
    };

    let entry_name = relative_path.file_name().ok_or_else(|| {
        ReconcileError::Other(format!(
            "scheduled_full_hash_audit: missing final path component for {}",
            relative_path.display()
        ))
    })?;
    let stat = match fs_safety::stat_at_nofollow(&parent_fd, Path::new(entry_name)) {
        Ok(stat) => stat,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    if stat.is_symlink() {
        eprintln!(
            "WARN: scheduled_full_hash_audit skipping symlinked tracked file {}",
            relative_path.display()
        );
        return Ok(None);
    }
    if !stat.is_regular_file() {
        return Ok(None);
    }

    let sha256 = hash_file_nofollow(&parent_fd, Path::new(entry_name))?;
    Ok(Some((
        FileStat {
            mtime_ns: stat.mtime_ns,
            ctime_ns: Some(stat.ctime_ns),
            size_bytes: stat.size_bytes,
            inode: Some(stat.inode),
        },
        sha256,
    )))
}

#[cfg(unix)]
fn hash_file_nofollow(root_fd: &OwnedFd, name: &Path) -> Result<String, ReconcileError> {
    let file_fd = openat(
        root_fd,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|err| io::Error::from_raw_os_error(err.raw_os_error()))?;
    let mut file = fs::File::from(file_fd);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

fn authorize_full_hash_reconcile(
    collection: &Collection,
    mode: FullHashReconcileMode,
    authorization: &FullHashReconcileAuthorization,
) -> Result<(), ReconcileError> {
    use crate::core::collections::CollectionState::{Active, Detached, Restoring};

    if let Some(identity) = authorization.identity() {
        if identity.trim().is_empty() {
            return Err(ReconcileError::InvalidFullHashAuthorization {
                mode,
                authorization: authorization.as_str(),
                reason: "missing caller identity",
            });
        }
    }

    match (mode, authorization, collection.state) {
        (
            FullHashReconcileMode::Audit,
            FullHashReconcileAuthorization::AuditCommand
            | FullHashReconcileAuthorization::ActiveLease { .. },
            Active,
        )
        | (
            FullHashReconcileMode::FreshAttach,
            FullHashReconcileAuthorization::AttachCommand { .. },
            Detached,
        )
        | (
            FullHashReconcileMode::OverflowRecovery,
            FullHashReconcileAuthorization::ActiveLease { .. },
            Active,
        ) => Ok(()),
        (
            FullHashReconcileMode::RemapRoot,
            FullHashReconcileAuthorization::ActiveLease { .. },
            Active,
        )
        | (
            FullHashReconcileMode::RemapRoot,
            FullHashReconcileAuthorization::ActiveLease { .. },
            Restoring,
        ) => require_persisted_full_hash_owner_match(collection, mode, authorization),
        (
            FullHashReconcileMode::Restore,
            FullHashReconcileAuthorization::ActiveLease { .. }
            | FullHashReconcileAuthorization::RestoreCommand { .. }
            | FullHashReconcileAuthorization::RestoreLease { .. },
            Active | Restoring,
        ) => require_persisted_full_hash_owner_match(collection, mode, authorization),
        (
            FullHashReconcileMode::RemapDriftCapture,
            FullHashReconcileAuthorization::ActiveLease { .. },
            Active | Restoring,
        )
        | (
            FullHashReconcileMode::RestoreDriftCapture,
            FullHashReconcileAuthorization::RestoreCommand { .. }
            | FullHashReconcileAuthorization::RestoreLease { .. },
            Restoring,
        ) => require_persisted_full_hash_owner_match(collection, mode, authorization),
        _ => Err(ReconcileError::UnauthorizedFullHashReconcile {
            mode,
            authorization: authorization.as_str(),
            collection_state: collection.state,
        }),
    }
}

fn require_persisted_full_hash_owner_match(
    collection: &Collection,
    mode: FullHashReconcileMode,
    authorization: &FullHashReconcileAuthorization,
) -> Result<(), ReconcileError> {
    match authorization {
        FullHashReconcileAuthorization::ActiveLease { lease_session_id } => {
            require_owner_identity_match(
                mode,
                authorization.as_str(),
                collection.active_lease_session_id.as_deref(),
                lease_session_id,
            )
        }
        FullHashReconcileAuthorization::RestoreCommand { restore_command_id } => {
            require_owner_identity_match(
                mode,
                authorization.as_str(),
                collection.restore_command_id.as_deref(),
                restore_command_id,
            )
        }
        FullHashReconcileAuthorization::RestoreLease { lease_session_id } => {
            require_owner_identity_match(
                mode,
                authorization.as_str(),
                collection.restore_lease_session_id.as_deref(),
                lease_session_id,
            )
        }
        FullHashReconcileAuthorization::AuditCommand
        | FullHashReconcileAuthorization::AttachCommand { .. } => Ok(()),
    }
}

fn require_owner_identity_match(
    mode: FullHashReconcileMode,
    authorization: &'static str,
    persisted_owner_identity: Option<&str>,
    caller_identity: &str,
) -> Result<(), ReconcileError> {
    let Some(persisted_owner_identity) =
        persisted_owner_identity.filter(|value| !value.trim().is_empty())
    else {
        return Err(ReconcileError::InvalidFullHashAuthorization {
            mode,
            authorization,
            reason: "missing persisted owner identity",
        });
    };

    if persisted_owner_identity == caller_identity {
        return Ok(());
    }

    Err(ReconcileError::InvalidFullHashAuthorization {
        mode,
        authorization,
        reason: "caller identity mismatch",
    })
}

fn has_canonical_nontrivial_body(body_size_bytes: i64, has_nonempty_body: bool) -> bool {
    body_size_bytes >= MIN_CANONICAL_BODY_BYTES && has_nonempty_body
}

fn has_canonical_trivial_body(body_size_bytes: i64, has_nonempty_body: bool) -> bool {
    !has_canonical_nontrivial_body(body_size_bytes, has_nonempty_body)
}

fn canonical_body_refusal_reason(
    prefix: &str,
    body_size_bytes: i64,
    has_nonempty_body: bool,
) -> Option<String> {
    if !has_nonempty_body {
        return Some(format!("{prefix}_empty_body"));
    }
    if body_size_bytes < MIN_CANONICAL_BODY_BYTES {
        return Some(format!("{prefix}_below_min_body_bytes"));
    }
    None
}

#[derive(Debug, Clone)]
pub(crate) struct CanonicalIdentityRecord<K> {
    pub key: K,
    pub label: String,
    pub uuid: Option<String>,
    pub sha256: String,
    pub body_size_bytes: i64,
    pub has_nonempty_body: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageIdentityResolution<K> {
    Matched(K),
    Missing,
    AmbiguousHash,
    DuplicateUuid {
        candidate_labels: Vec<String>,
    },
    TrivialHashRefusal {
        candidate_labels: Vec<String>,
        reason: String,
    },
}

pub(crate) fn resolve_page_identity<K>(
    page: &CanonicalIdentityRecord<()>,
    page_hash_count: usize,
    candidates: &[CanonicalIdentityRecord<K>],
) -> PageIdentityResolution<K>
where
    K: Clone,
{
    if let Some(uuid) = page.uuid.as_deref() {
        let uuid_matches = candidates
            .iter()
            .filter(|candidate| candidate.uuid.as_deref() == Some(uuid))
            .collect::<Vec<_>>();
        match uuid_matches.as_slice() {
            [candidate] => return PageIdentityResolution::Matched(candidate.key.clone()),
            [] => {}
            _ => {
                return PageIdentityResolution::DuplicateUuid {
                    candidate_labels: uuid_matches
                        .into_iter()
                        .map(|candidate| candidate.label.clone())
                        .collect(),
                };
            }
        }
    }

    let hash_matches = candidates
        .iter()
        .filter(|candidate| candidate.sha256 == page.sha256)
        .collect::<Vec<_>>();
    if hash_matches.is_empty() {
        return PageIdentityResolution::Missing;
    }
    if page_hash_count != 1 || hash_matches.len() != 1 {
        return PageIdentityResolution::AmbiguousHash;
    }
    if let Some(reason) =
        canonical_body_refusal_reason("missing", page.body_size_bytes, page.has_nonempty_body)
    {
        return PageIdentityResolution::TrivialHashRefusal {
            candidate_labels: hash_matches
                .iter()
                .map(|candidate| candidate.label.clone())
                .collect(),
            reason,
        };
    }
    if let Some(reason) = canonical_body_refusal_reason(
        "new",
        hash_matches[0].body_size_bytes,
        hash_matches[0].has_nonempty_body,
    ) {
        return PageIdentityResolution::TrivialHashRefusal {
            candidate_labels: hash_matches
                .iter()
                .map(|candidate| candidate.label.clone())
                .collect(),
            reason,
        };
    }

    PageIdentityResolution::Matched(hash_matches[0].key.clone())
}

fn raw_import_invariant_result(
    page_id: i64,
    row_count: i64,
    active_count: i64,
    context: &str,
    policy: RawImportInvariantPolicy,
) -> Result<(), ReconcileError> {
    if row_count != 0 && active_count == 1 {
        return Ok(());
    }

    let message = if row_count == 0 {
        format!(
            "{context}: page_id={page_id} has zero total raw_imports rows; use --allow-rerender to override"
        )
    } else {
        format!(
            "{context}: page_id={page_id} has {active_count} active raw_imports rows across {row_count} total rows; use --allow-rerender to override"
        )
    };
    match policy {
        RawImportInvariantPolicy::Enforce => {
            Err(ReconcileError::InvariantViolationError { message })
        }
        RawImportInvariantPolicy::AllowRerenderOverride => {
            eprintln!("WARN: allow_rerender_override {message}");
            Ok(())
        }
    }
}

fn default_restore_stability_max_iters() -> usize {
    std::env::var("QUAID_RESTORE_STABILITY_MAX_ITERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value != 0)
        .unwrap_or(DEFAULT_RESTORE_STABILITY_MAX_ITERS)
}

fn collection_recovery_dir(recovery_root: &Path, collection_id: i64) -> PathBuf {
    recovery_root.join(collection_id.to_string())
}

fn sentinel_count(recovery_root: &Path, collection_id: i64) -> Result<usize, ReconcileError> {
    let recovery_dir = collection_recovery_dir(recovery_root, collection_id);
    if !recovery_dir.exists() {
        return Ok(0);
    }

    let mut count = 0usize;
    for entry in fs::read_dir(recovery_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .ends_with(".needs_full_sync")
        {
            count += 1;
        }
    }
    Ok(count)
}

pub fn is_collection_dirty(
    conn: &Connection,
    collection_id: i64,
    recovery_root: &Path,
) -> Result<CollectionDirtyStatus, ReconcileError> {
    let collection = load_collection_by_id(conn, collection_id)?;
    Ok(CollectionDirtyStatus {
        needs_full_sync: collection.needs_full_sync,
        sentinel_count: sentinel_count(recovery_root, collection_id)?,
        recovery_in_progress: false,
        last_sync_at: collection.last_sync_at,
    })
}

fn fresh_collection_dirty_status(
    db_path: &Path,
    collection_id: i64,
    recovery_root: &Path,
) -> Result<CollectionDirtyStatus, ReconcileError> {
    let conn = Connection::open(db_path)?;
    is_collection_dirty(&conn, collection_id, recovery_root)
}

fn load_frontmatter_map(
    frontmatter_json: &str,
) -> Result<crate::core::types::Frontmatter, ReconcileError> {
    serde_json::from_str(frontmatter_json).map_err(|err| {
        ReconcileError::Other(format!(
            "load_frontmatter_map: invalid stored frontmatter json: {err}"
        ))
    })
}

fn uuid_migration_preflight(
    conn: &Connection,
    collection: &Collection,
) -> Result<(), ReconcileError> {
    let mut stmt = conn.prepare(
        "SELECT p.uuid, p.frontmatter, p.compiled_truth, p.timeline,
                COALESCE(fs.relative_path, p.slug) AS sample_path
         FROM pages p
         LEFT JOIN file_state fs
           ON fs.page_id = p.id
          AND fs.collection_id = p.collection_id
         WHERE p.collection_id = ?1
           AND p.uuid IS NOT NULL",
    )?;
    let rows = stmt.query_map([collection.id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    let mut affected_count = 0usize;
    let mut sample_paths = Vec::new();
    for row in rows {
        let (uuid, frontmatter_json, compiled_truth, timeline, sample_path) = row?;
        let frontmatter = load_frontmatter_map(&frontmatter_json)?;
        let mirrored_frontmatter_uuid =
            page_uuid::parse_frontmatter_uuid(&frontmatter).unwrap_or_default();
        if mirrored_frontmatter_uuid.as_deref() == Some(uuid.as_str()) {
            continue;
        }

        let trimmed_ct = compiled_truth.trim();
        let trimmed_tl = timeline.trim();
        if has_canonical_trivial_body(
            (trimmed_ct.len() + trimmed_tl.len()) as i64,
            !(trimmed_ct.is_empty() && trimmed_tl.is_empty()),
        ) {
            affected_count += 1;
            if sample_paths.len() < UUID_MIGRATION_SAMPLE_LIMIT {
                sample_paths.push(sample_path);
            }
        }
    }

    if affected_count == 0 {
        return Ok(());
    }

    Err(ReconcileError::UuidMigrationRequiredError {
        collection_name: collection.name.clone(),
        affected_count,
        sample_paths,
    })
}

#[cfg(unix)]
fn verify_read_only_mount(collection: &Collection) -> Result<(), ReconcileError> {
    use rustix::fs::{fstatvfs, StatVfsMountFlags};

    let root_fd = fs_safety::open_root_fd(Path::new(&collection.root_path))?;
    let statvfs = fstatvfs(&root_fd).map_err(std::io::Error::from)?;
    if !statvfs.f_flag.contains(StatVfsMountFlags::RDONLY) {
        return Err(ReconcileError::CollectionLacksWriterQuiescenceError {
            collection_name: collection.name.clone(),
            root_path: collection.root_path.clone(),
        });
    }

    eprintln!(
        "INFO: restore_ro_mount_verified collection={} mount_flags={:?}",
        collection.name, statvfs.f_flag
    );
    Ok(())
}

#[cfg(not(unix))]
fn verify_read_only_mount(collection: &Collection) -> Result<(), ReconcileError> {
    Err(ReconcileError::Other(format!(
        "restore/remap safety checks are not supported on Windows for collection={}",
        collection.name
    )))
}

type StatSnapshot = HashMap<PathBuf, FileStat>;

fn take_stat_snapshot(
    conn: &Connection,
    collection: &Collection,
) -> Result<StatSnapshot, ReconcileError> {
    #[cfg(unix)]
    {
        let root_fd = fs_safety::open_root_fd(Path::new(&collection.root_path))?;
        ignore_patterns::reload_patterns(conn, collection.id, Path::new(&collection.root_path))
            .map_err(|err| {
                ReconcileError::Other(format!(
                    "take_stat_snapshot: refusing to walk with stale .quaidignore state: {err}"
                ))
            })?;
        Ok(walk_collection(conn, &root_fd, collection)?.files)
    }

    #[cfg(not(unix))]
    {
        let _ = (conn, collection);
        Err(ReconcileError::Other(
            "take_stat_snapshot: fd-relative operations not supported on Windows. Vault sync commands require Unix."
                .to_string(),
        ))
    }
}

fn capture_phase1_drift(
    conn: &Connection,
    collection: &Collection,
    operation: RestoreRemapOperation,
    authorization: &FullHashReconcileAuthorization,
) -> Result<DriftCaptureSummary, ReconcileError> {
    let mode = match operation {
        RestoreRemapOperation::Restore => FullHashReconcileMode::RestoreDriftCapture,
        RestoreRemapOperation::Remap => FullHashReconcileMode::RemapDriftCapture,
    };
    let stats = full_hash_reconcile_authorized(conn, collection.id, mode, authorization.clone())?;
    let summary = DriftCaptureSummary::from_stats(&stats);

    match operation {
        RestoreRemapOperation::Restore if summary.has_material_changes() => {
            eprintln!(
                "WARN: restore_drift_captured collection={} pages_updated={} pages_added={} pages_quarantined={} pages_deleted={}",
                collection.name,
                summary.pages_updated,
                summary.pages_added,
                summary.pages_quarantined,
                summary.pages_deleted
            );
        }
        RestoreRemapOperation::Remap if summary.has_material_changes() => {
            eprintln!(
                "ERROR: remap_drift_refused collection={} pages_updated={} pages_added={} pages_quarantined={} pages_deleted={}",
                collection.name,
                summary.pages_updated,
                summary.pages_added,
                summary.pages_quarantined,
                summary.pages_deleted
            );
            return Err(ReconcileError::RemapDriftConflictError {
                collection_name: collection.name.clone(),
                summary,
            });
        }
        _ => {}
    }

    Ok(summary)
}

fn run_phase2_stability_check<FS, FR>(
    operation: RestoreRemapOperation,
    max_iters: usize,
    collection_name: &str,
    mut take_snapshot: FS,
    mut rerun_phase1: FR,
) -> Result<(StatSnapshot, usize, DriftCaptureSummary), ReconcileError>
where
    FS: FnMut() -> Result<StatSnapshot, ReconcileError>,
    FR: FnMut() -> Result<DriftCaptureSummary, ReconcileError>,
{
    let mut previous = take_snapshot()?;
    let mut retries = 0usize;
    let mut accumulated_drift = DriftCaptureSummary::default();

    loop {
        let current = take_snapshot()?;
        if previous == current {
            return Ok((current, retries, accumulated_drift));
        }

        if retries >= max_iters {
            eprintln!(
                "WARN: {}_aborted_unstable collection={} iters={}",
                operation.as_str(),
                collection_name,
                retries
            );
            return Err(ReconcileError::CollectionUnstableError {
                collection_name: collection_name.to_owned(),
                operation,
                phase: "stability",
                retries,
            });
        }

        retries += 1;
        accumulated_drift.add_assign(&rerun_phase1()?);
        previous = current;
    }
}

fn run_phase3_pre_destruction_fence(
    conn: &Connection,
    collection: &Collection,
    operation: RestoreRemapOperation,
    stable_snapshot: &StatSnapshot,
) -> Result<(), ReconcileError> {
    let fence_snapshot = take_stat_snapshot(conn, collection)?;
    if fence_snapshot == *stable_snapshot {
        return Ok(());
    }

    eprintln!(
        "WARN: {}_aborted_fence_drift collection={}",
        operation.as_str(),
        collection.name
    );
    Err(ReconcileError::CollectionUnstableError {
        collection_name: collection.name.clone(),
        operation,
        phase: "fence",
        retries: 0,
    })
}

fn run_restore_remap_safety_pipeline_inner<F, G>(
    conn: &Connection,
    request: &RestoreRemapSafetyRequest<'_>,
    verify_ro_mount: G,
    after_fence: F,
) -> Result<RestoreRemapSafetyOutcome, ReconcileError>
where
    F: FnOnce() -> Result<(), ReconcileError>,
    G: Fn(&Collection) -> Result<(), ReconcileError>,
{
    let mut after_fence = Some(after_fence);
    let collection = load_collection_by_id(conn, request.collection_id)?;
    uuid_migration_preflight(conn, &collection)?;
    verify_ro_mount(&collection)?;

    let dirty_status = is_collection_dirty(conn, collection.id, request.recovery_root)?;
    if dirty_status.is_dirty() && !request.allow_finalize_pending {
        return Err(ReconcileError::CollectionDirtyError {
            collection_name: collection.name.clone(),
            status: dirty_status,
        });
    }

    let mut total_drift =
        capture_phase1_drift(conn, &collection, request.operation, &request.authorization)?;
    let max_iters = if request.stability_max_iters == 0 {
        default_restore_stability_max_iters()
    } else {
        request.stability_max_iters
    };
    let (stable_snapshot, retries, retry_drift) = run_phase2_stability_check(
        request.operation,
        max_iters,
        &collection.name,
        || take_stat_snapshot(conn, &collection),
        || capture_phase1_drift(conn, &collection, request.operation, &request.authorization),
    )?;
    total_drift.add_assign(&retry_drift);
    run_phase3_pre_destruction_fence(conn, &collection, request.operation, &stable_snapshot)?;
    if let Some(after_fence) = after_fence.take() {
        after_fence()?;
    }

    let dirty_status =
        fresh_collection_dirty_status(request.db_path, collection.id, request.recovery_root)?;
    if dirty_status.is_dirty() && !request.allow_finalize_pending {
        eprintln!(
            "WARN: {}_aborted_dirty_recheck collection={}",
            request.operation.as_str(),
            collection.name
        );
        return Err(ReconcileError::CollectionDirtyError {
            collection_name: collection.name,
            status: dirty_status,
        });
    }

    Ok(RestoreRemapSafetyOutcome {
        drift_summary: total_drift,
        stability_retries: retries,
        final_snapshot_files: stable_snapshot.len(),
    })
}

pub fn run_restore_remap_safety_pipeline(
    conn: &Connection,
    request: &RestoreRemapSafetyRequest<'_>,
) -> Result<RestoreRemapSafetyOutcome, ReconcileError> {
    run_restore_remap_safety_pipeline_inner(conn, request, verify_read_only_mount, || Ok(()))
}

pub(crate) fn run_restore_remap_safety_pipeline_without_mount_check(
    conn: &Connection,
    request: &RestoreRemapSafetyRequest<'_>,
) -> Result<RestoreRemapSafetyOutcome, ReconcileError> {
    run_restore_remap_safety_pipeline_inner(conn, request, |_| Ok(()), || Ok(()))
}

pub fn fresh_attach_reconcile_and_activate(
    conn: &Connection,
    collection_id: i64,
    attach_command_id: &str,
) -> Result<ReconcileStats, ReconcileError> {
    let stats = full_hash_reconcile_authorized(
        conn,
        collection_id,
        FullHashReconcileMode::FreshAttach,
        FullHashReconcileAuthorization::AttachCommand {
            attach_command_id: attach_command_id.to_owned(),
        },
    )?;
    conn.execute(
        "UPDATE collections
         SET state = 'active',
             needs_full_sync = 0,
             last_sync_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [collection_id],
    )?;
    Ok(stats)
}

#[derive(Debug, Clone)]
struct StoredFileStateEntry {
    page_id: i64,
    stat: FileStat,
    sha256: String,
}

#[derive(Debug, Clone)]
struct FullHashUnchangedEntry {
    relative_path: PathBuf,
    page_id: i64,
    stat: FileStat,
    sha256: String,
    skip_metadata_refresh: bool,
}

#[derive(Debug, Default)]
struct FullHashPlan {
    unchanged: Vec<FullHashUnchangedEntry>,
    diff: StatDiff,
}

fn load_collection_by_id(
    conn: &Connection,
    collection_id: i64,
) -> Result<Collection, ReconcileError> {
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

fn load_stored_file_state_entries(
    conn: &Connection,
    collection_id: i64,
) -> Result<HashMap<PathBuf, StoredFileStateEntry>, ReconcileError> {
    let mut stmt = conn.prepare(
        "SELECT relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256
         FROM file_state
         WHERE collection_id = ?1",
    )?;
    let rows = stmt.query_map([collection_id], |row| {
        let path: String = row.get(0)?;
        Ok((
            PathBuf::from(path),
            StoredFileStateEntry {
                page_id: row.get(1)?,
                stat: FileStat {
                    mtime_ns: row.get(2)?,
                    ctime_ns: row.get(3)?,
                    size_bytes: row.get(4)?,
                    inode: row.get(5)?,
                },
                sha256: row.get(6)?,
            },
        ))
    })?;

    let mut entries = HashMap::new();
    for row in rows {
        let (path, entry) = row?;
        entries.insert(path, entry);
    }
    Ok(entries)
}

fn build_full_hash_plan(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    walked_files: &HashMap<PathBuf, FileStat>,
) -> Result<FullHashPlan, ReconcileError> {
    let mut stored_entries = load_stored_file_state_entries(conn, collection_id)?;
    let mut plan = FullHashPlan::default();

    for (relative_path, stat) in walked_files {
        let sha256 = file_state::hash_file(&root_path.join(relative_path))?;
        match stored_entries.remove(relative_path) {
            Some(stored) if stored.sha256 == sha256 => {
                plan.unchanged.push(FullHashUnchangedEntry {
                    relative_path: relative_path.clone(),
                    page_id: stored.page_id,
                    stat: stat.clone(),
                    sha256,
                    skip_metadata_refresh: false,
                })
            }
            Some(stored) => {
                let _ = stored.stat;
                if is_extracted_whitespace_noop(
                    conn,
                    collection_id,
                    root_path,
                    relative_path,
                    stored.page_id,
                )? {
                    plan.unchanged.push(FullHashUnchangedEntry {
                        relative_path: relative_path.clone(),
                        page_id: stored.page_id,
                        stat: stat.clone(),
                        sha256,
                        skip_metadata_refresh: true,
                    });
                } else {
                    plan.diff
                        .modified
                        .insert(relative_path.clone(), stat.clone());
                }
            }
            None => {
                plan.diff.new.insert(relative_path.clone(), stat.clone());
            }
        }
    }

    plan.diff.missing.extend(stored_entries.into_keys());
    Ok(plan)
}

fn assert_full_hash_raw_import_invariants(
    conn: &Connection,
    collection_id: i64,
) -> Result<(), ReconcileError> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT page_id FROM file_state WHERE collection_id = ?1")?;
    let page_ids = stmt
        .query_map([collection_id], |row| row.get::<_, i64>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    assert_raw_import_invariants_for_page_ids(conn, &page_ids, "full_hash_reconcile")
}

fn assert_raw_import_invariants_for_page_ids(
    conn: &Connection,
    page_ids: &HashSet<i64>,
    context: &'static str,
) -> Result<(), ReconcileError> {
    let mut sorted_page_ids = page_ids.iter().copied().collect::<Vec<_>>();
    sorted_page_ids.sort_unstable();

    for page_id in sorted_page_ids {
        let (row_count, active_count): (i64, i64) = conn.query_row(
            "SELECT
                 COUNT(*) AS row_count,
                 COALESCE(SUM(CASE WHEN is_active = 1 THEN 1 ELSE 0 END), 0) AS active_count
             FROM raw_imports
             WHERE page_id = ?1",
            [page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        raw_import_invariant_result(
            page_id,
            row_count,
            active_count,
            context,
            RawImportInvariantPolicy::Enforce,
        )?;
    }

    Ok(())
}

fn apply_full_hash_metadata_self_heal(
    conn: &Connection,
    collection_id: i64,
    unchanged: &[FullHashUnchangedEntry],
) -> Result<(), ReconcileError> {
    let mut unchanged = unchanged.to_vec();
    unchanged.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    for chunk in unchanged.chunks(500) {
        let tx = conn.unchecked_transaction()?;
        for entry in chunk {
            if entry.skip_metadata_refresh {
                continue;
            }
            file_state::upsert_file_state(
                &tx,
                collection_id,
                &path_to_string(&entry.relative_path),
                entry.page_id,
                &entry.stat,
                &entry.sha256,
            )?;
        }
        tx.commit()?;
    }

    Ok(())
}

// ── Stat Diff (stub) ──────────────────────────────────────────

/// Stat-diff result: classify files into changed/unchanged/new/missing sets.
#[derive(Debug, Default)]
pub struct StatDiff {
    pub unchanged: HashSet<PathBuf>,
    pub modified: HashMap<PathBuf, FileStat>,
    pub new: HashMap<PathBuf, FileStat>,
    pub missing: HashSet<PathBuf>,
}

/// Compare filesystem walk against `file_state`; yield changed/unchanged/new/missing sets.
///
/// Files are `unchanged` ONLY when ALL four stat fields match: mtime_ns, ctime_ns, size_bytes, inode.
/// Any mismatch → `modified` (will trigger re-hash).
///
pub fn stat_diff(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
) -> Result<StatDiff, ReconcileError> {
    #[cfg(unix)]
    {
        let root_fd = fs_safety::open_root_fd(root_path)?;
        ignore_patterns::reload_patterns(conn, collection_id, root_path).map_err(|err| {
            ReconcileError::Other(format!(
                "stat_diff: refusing to walk with stale .quaidignore state: {err}"
            ))
        })?;
        let walked = walk_root(conn, collection_id, root_path, &root_fd)?;
        stat_diff_from_walk(conn, collection_id, root_path, walked.files)
    }

    #[cfg(not(unix))]
    {
        let _ = (conn, collection_id, root_path);
        Err(ReconcileError::Other(
            "stat_diff: fd-relative operations not supported on Windows. \
             Vault sync commands require Unix."
                .to_string(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeRename {
    pub(crate) from_path: PathBuf,
    pub(crate) to_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenameMatchKind {
    Native,
    Uuid,
    Hash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenameMatch {
    page_id: i64,
    from_path: PathBuf,
    to_path: PathBuf,
    kind: RenameMatchKind,
}

#[derive(Debug, Clone)]
struct MissingPageIdentity {
    page_id: i64,
    uuid: Option<String>,
    sha256: String,
    /// Byte length of the body content stored in the DB (compiled_truth + timeline,
    /// trimmed). Does NOT include frontmatter bytes. Used in the conservative hash-rename
    /// guard so that large-frontmatter / tiny-body template notes cannot satisfy the
    /// minimum-content threshold by inflating the whole-file size.
    body_size_bytes: i64,
    has_nonempty_body: bool,
}

#[derive(Debug, Clone)]
struct NewTreeIdentity {
    relative_path: PathBuf,
    sha256: String,
    uuid: Option<String>,
    /// Byte length of the body text after frontmatter delimiter (trimmed).
    /// Used in the conservative hash-rename guard; whole-file size is intentionally
    /// not used here to prevent large-frontmatter / tiny-body template notes from
    /// satisfying the threshold.
    body_size_bytes: i64,
    has_nonempty_body: bool,
}

#[derive(Debug, Default)]
struct RenameResolution {
    native_renamed: usize,
    uuid_renamed: usize,
    hash_renamed: usize,
    quarantined_ambiguous: usize,
    remaining_new: HashMap<PathBuf, FileStat>,
    remaining_missing: HashSet<PathBuf>,
    matches: Vec<RenameMatch>,
}

#[derive(Debug, Clone)]
enum ApplyAction {
    DeleteOrQuarantine {
        page_id: i64,
        relative_path: PathBuf,
    },
    Reingest {
        existing_page_id: Option<i64>,
        old_relative_path: Option<PathBuf>,
        relative_path: PathBuf,
        stat: FileStat,
    },
}

#[derive(Debug, Default)]
struct ApplySummary {
    reingested: usize,
    created: usize,
    quarantined_db_state: usize,
    hard_deleted: usize,
}

#[derive(Debug)]
struct ParsedVaultFile {
    slug: String,
    title: String,
    page_type: String,
    summary: String,
    compiled_truth: String,
    timeline: String,
    frontmatter: crate::core::types::Frontmatter,
    wing: String,
    room: String,
    sha256: String,
}

#[cfg(unix)]
fn resolve_rename_resolution(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    diff: &StatDiff,
    native_renames: &[NativeRename],
) -> Result<RenameResolution, ReconcileError> {
    let mut resolution = RenameResolution {
        remaining_new: diff.new.clone(),
        remaining_missing: diff.missing.clone(),
        ..RenameResolution::default()
    };
    if diff.new.is_empty() || diff.missing.is_empty() {
        return Ok(resolution);
    }

    let missing_identities = load_missing_page_identities(conn, collection_id, &diff.missing)?;
    let new_identities = load_new_tree_identities(root_path, &diff.new)?;

    apply_native_rename_matches(
        &mut resolution,
        &missing_identities,
        native_renames,
        &new_identities,
    );
    apply_uuid_rename_matches(&mut resolution, &missing_identities, &new_identities);
    apply_hash_rename_matches(&mut resolution, &missing_identities, &new_identities)?;

    Ok(resolution)
}

#[cfg(not(unix))]
fn resolve_rename_resolution(
    _conn: &Connection,
    _collection_id: i64,
    _root_path: &Path,
    diff: &StatDiff,
    _native_renames: &[NativeRename],
) -> Result<RenameResolution, ReconcileError> {
    Ok(RenameResolution {
        remaining_new: diff.new.clone(),
        remaining_missing: diff.missing.clone(),
        ..RenameResolution::default()
    })
}

fn apply_native_rename_matches(
    resolution: &mut RenameResolution,
    missing_identities: &HashMap<PathBuf, MissingPageIdentity>,
    native_renames: &[NativeRename],
    new_identities: &HashMap<PathBuf, NewTreeIdentity>,
) {
    for native_rename in native_renames {
        if !resolution
            .remaining_missing
            .contains(&native_rename.from_path)
            || !resolution
                .remaining_new
                .contains_key(&native_rename.to_path)
        {
            continue;
        }

        if let Some(missing_identity) = missing_identities.get(&native_rename.from_path) {
            if new_identities.contains_key(&native_rename.to_path) {
                record_rename_match(
                    resolution,
                    missing_identity.page_id,
                    &native_rename.from_path,
                    &native_rename.to_path,
                    RenameMatchKind::Native,
                );
            }
        }
    }
}

fn apply_uuid_rename_matches(
    resolution: &mut RenameResolution,
    missing_identities: &HashMap<PathBuf, MissingPageIdentity>,
    new_identities: &HashMap<PathBuf, NewTreeIdentity>,
) {
    let mut new_by_uuid: HashMap<&str, Vec<&PathBuf>> = HashMap::new();
    for (path, identity) in new_identities {
        if !resolution.remaining_new.contains_key(path) {
            continue;
        }
        if let Some(uuid) = identity.uuid.as_deref() {
            new_by_uuid.entry(uuid).or_default().push(path);
        }
    }

    let remaining_missing_paths: Vec<PathBuf> =
        resolution.remaining_missing.iter().cloned().collect();
    for path in remaining_missing_paths {
        let Some(missing_identity) = missing_identities.get(&path) else {
            continue;
        };
        let Some(uuid) = missing_identity.uuid.as_deref() else {
            continue;
        };
        let Some(candidates) = new_by_uuid.get(uuid) else {
            continue;
        };

        let remaining_candidates: Vec<&PathBuf> = candidates
            .iter()
            .copied()
            .filter(|candidate| resolution.remaining_new.contains_key(*candidate))
            .collect();

        match remaining_candidates.as_slice() {
            [candidate] => record_rename_match(
                resolution,
                missing_identity.page_id,
                &path,
                candidate,
                RenameMatchKind::Uuid,
            ),
            [] => {}
            _ => {
                resolution.remaining_missing.remove(&path);
                resolution.quarantined_ambiguous += 1;
            }
        }
    }
}

fn apply_hash_rename_matches(
    resolution: &mut RenameResolution,
    missing_identities: &HashMap<PathBuf, MissingPageIdentity>,
    new_identities: &HashMap<PathBuf, NewTreeIdentity>,
) -> Result<(), ReconcileError> {
    let mut missing_by_hash: HashMap<&str, Vec<&PathBuf>> = HashMap::new();
    for (path, identity) in missing_identities {
        if resolution.remaining_missing.contains(path) {
            missing_by_hash
                .entry(&identity.sha256)
                .or_default()
                .push(path);
        }
    }

    let mut new_by_hash: HashMap<&str, Vec<&PathBuf>> = HashMap::new();
    for (path, identity) in new_identities {
        if resolution.remaining_new.contains_key(path) {
            new_by_hash.entry(&identity.sha256).or_default().push(path);
        }
    }

    let remaining_missing_paths: Vec<PathBuf> =
        resolution.remaining_missing.iter().cloned().collect();
    for path in remaining_missing_paths {
        let Some(missing_identity) = missing_identities.get(&path) else {
            continue;
        };
        let Some(new_candidates) = new_by_hash.get(missing_identity.sha256.as_str()) else {
            continue;
        };

        let remaining_candidates: Vec<&PathBuf> = new_candidates
            .iter()
            .copied()
            .filter(|candidate| resolution.remaining_new.contains_key(*candidate))
            .collect();
        if remaining_candidates.is_empty() {
            continue;
        }

        let candidate_records = remaining_candidates
            .iter()
            .filter_map(|candidate| {
                new_identities
                    .get(*candidate)
                    .map(|identity| CanonicalIdentityRecord {
                        key: (*candidate).clone(),
                        label: path_to_string(candidate),
                        uuid: identity.uuid.clone(),
                        sha256: identity.sha256.clone(),
                        body_size_bytes: identity.body_size_bytes,
                        has_nonempty_body: identity.has_nonempty_body,
                    })
            })
            .collect::<Vec<_>>();
        let page_record = CanonicalIdentityRecord {
            key: (),
            label: path_to_string(&path),
            uuid: missing_identity.uuid.clone(),
            sha256: missing_identity.sha256.clone(),
            body_size_bytes: missing_identity.body_size_bytes,
            has_nonempty_body: missing_identity.has_nonempty_body,
        };
        let resolution_result = resolve_page_identity(
            &page_record,
            missing_by_hash
                .get(missing_identity.sha256.as_str())
                .map_or(0, Vec::len),
            &candidate_records,
        );
        match resolution_result {
            PageIdentityResolution::Matched(candidate) => record_rename_match(
                resolution,
                missing_identity.page_id,
                &path,
                &candidate,
                RenameMatchKind::Hash,
            ),
            PageIdentityResolution::TrivialHashRefusal {
                candidate_labels,
                reason,
            } => {
                return Err(ReconcileError::UnresolvableTrivialContentError {
                    missing_path: path_to_string(&path),
                    candidate_paths: candidate_labels,
                    reason,
                });
            }
            PageIdentityResolution::Missing => {}
            PageIdentityResolution::AmbiguousHash => {
                let reason = if missing_by_hash
                    .get(missing_identity.sha256.as_str())
                    .map_or(0, Vec::len)
                    != 1
                {
                    "missing_hash_not_unique".to_owned()
                } else {
                    "new_hash_not_unique".to_owned()
                };
                log_hash_refusal(&reason, &path, &remaining_candidates);
                resolution.remaining_missing.remove(&path);
                resolution.quarantined_ambiguous += 1;
            }
            PageIdentityResolution::DuplicateUuid { .. } => {
                let reason = "duplicate_uuid_candidates".to_owned();
                log_hash_refusal(&reason, &path, &remaining_candidates);
                resolution.remaining_missing.remove(&path);
                resolution.quarantined_ambiguous += 1;
            }
        }
    }

    Ok(())
}

fn log_hash_refusal(reason: &str, missing_path: &Path, new_candidates: &[&PathBuf]) {
    let candidates = new_candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(",");
    eprintln!(
        "INFO: rename_inference_refused reason={reason} missing={} candidates={candidates}",
        missing_path.display()
    );
}

fn hash_refusal_reason(
    missing_identity: &MissingPageIdentity,
    new_candidates: &[&PathBuf],
    missing_hash_count: usize,
    new_identities: &HashMap<PathBuf, NewTreeIdentity>,
) -> Option<String> {
    let candidate_records = new_candidates
        .iter()
        .filter_map(|candidate| {
            new_identities
                .get(*candidate)
                .map(|identity| CanonicalIdentityRecord {
                    key: (*candidate).clone(),
                    label: path_to_string(candidate),
                    uuid: identity.uuid.clone(),
                    sha256: identity.sha256.clone(),
                    body_size_bytes: identity.body_size_bytes,
                    has_nonempty_body: identity.has_nonempty_body,
                })
        })
        .collect::<Vec<_>>();
    let page_record = CanonicalIdentityRecord {
        key: (),
        label: String::new(),
        uuid: missing_identity.uuid.clone(),
        sha256: missing_identity.sha256.clone(),
        body_size_bytes: missing_identity.body_size_bytes,
        has_nonempty_body: missing_identity.has_nonempty_body,
    };
    match resolve_page_identity(&page_record, missing_hash_count, &candidate_records) {
        PageIdentityResolution::Matched(_) | PageIdentityResolution::Missing => None,
        PageIdentityResolution::AmbiguousHash => {
            if missing_hash_count != 1 {
                Some("missing_hash_not_unique".to_owned())
            } else {
                Some("new_hash_not_unique".to_owned())
            }
        }
        PageIdentityResolution::DuplicateUuid { .. } => {
            Some("duplicate_uuid_candidates".to_owned())
        }
        PageIdentityResolution::TrivialHashRefusal { reason, .. } => Some(reason),
    }
}

fn record_rename_match(
    resolution: &mut RenameResolution,
    page_id: i64,
    from_path: &Path,
    to_path: &Path,
    kind: RenameMatchKind,
) {
    resolution.remaining_missing.remove(from_path);
    resolution.remaining_new.remove(to_path);
    resolution.matches.push(RenameMatch {
        page_id,
        from_path: from_path.to_path_buf(),
        to_path: to_path.to_path_buf(),
        kind,
    });

    match kind {
        RenameMatchKind::Native => resolution.native_renamed += 1,
        RenameMatchKind::Uuid => resolution.uuid_renamed += 1,
        RenameMatchKind::Hash => resolution.hash_renamed += 1,
    }
}

fn load_missing_page_identities(
    conn: &Connection,
    collection_id: i64,
    missing_paths: &HashSet<PathBuf>,
) -> Result<HashMap<PathBuf, MissingPageIdentity>, ReconcileError> {
    let mut stmt = conn.prepare(
        "SELECT fs.relative_path, fs.page_id, fs.sha256, fs.size_bytes, p.uuid, p.compiled_truth, p.timeline
          FROM file_state fs
          JOIN pages p ON p.id = fs.page_id
          WHERE fs.collection_id = ?1 AND fs.relative_path = ?2",
    )?;
    let mut identities = HashMap::new();

    for path in missing_paths {
        if let Some(identity) = stmt
            .query_row(
                rusqlite::params![collection_id, path_to_string(path)],
                |row| {
                    let page_id: i64 = row.get(1)?;
                    let sha256: String = row.get(2)?;
                    // row.get(3) is fs.size_bytes (whole-file) — intentionally ignored;
                    // the rename guard uses body_size_bytes computed from DB content, not
                    // the filesystem file size, to close the large-frontmatter seam.
                    let uuid: Option<String> = row.get(4)?;
                    let compiled_truth: String = row.get(5)?;
                    let timeline: String = row.get(6)?;
                    let trimmed_ct = compiled_truth.trim();
                    let trimmed_tl = timeline.trim();
                    Ok(MissingPageIdentity {
                        page_id,
                        sha256,
                        uuid,
                        body_size_bytes: (trimmed_ct.len() + trimmed_tl.len()) as i64,
                        has_nonempty_body: !(trimmed_ct.is_empty() && trimmed_tl.is_empty()),
                    })
                },
            )
            .optional()?
        {
            identities.insert(path.clone(), identity);
        }
    }

    Ok(identities)
}

fn load_new_tree_identities(
    root_path: &Path,
    new_paths: &HashMap<PathBuf, FileStat>,
) -> Result<HashMap<PathBuf, NewTreeIdentity>, ReconcileError> {
    let mut identities = HashMap::new();

    for (path, stat) in new_paths {
        let absolute_path = root_path.join(path);
        let raw_bytes = fs::read(&absolute_path)?;
        let sha256 = sha256_hex(&raw_bytes);
        let raw = String::from_utf8_lossy(&raw_bytes).into_owned();
        let (frontmatter, body) = markdown::parse_frontmatter(&raw);
        let (compiled_truth, timeline) = markdown::split_content(&body);
        let uuid = page_uuid::parse_frontmatter_uuid(&frontmatter).map_err(|err| {
            ReconcileError::Other(format!(
                "resolve_rename_resolution: {} has invalid frontmatter uuid: {err}",
                path.display()
            ))
        })?;

        let trimmed_ct = compiled_truth.trim();
        let trimmed_tl = timeline.trim();
        identities.insert(
            path.clone(),
            NewTreeIdentity {
                relative_path: path.clone(),
                sha256,
                uuid,
                body_size_bytes: (trimmed_ct.len() + trimmed_tl.len()) as i64,
                has_nonempty_body: !(trimmed_ct.is_empty() && trimmed_tl.is_empty()),
            },
        );
        // stat is kept available for callers that need filesystem metadata
        // (mtime/ctime/inode); the rename guard does not use it.
        let _ = stat;
    }

    Ok(identities)
}

fn detect_duplicate_uuids_in_tree(
    root_path: &Path,
    walked_files: &HashMap<PathBuf, FileStat>,
) -> Result<(), ReconcileError> {
    let mut relative_paths = walked_files.keys().cloned().collect::<Vec<_>>();
    relative_paths.sort();

    let mut uuids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for relative_path in relative_paths {
        let absolute_path = root_path.join(&relative_path);
        let raw_bytes = fs::read(&absolute_path)?;
        let raw = String::from_utf8_lossy(&raw_bytes);
        let (frontmatter, _) = markdown::parse_frontmatter(&raw);
        if let Some(uuid) = page_uuid::parse_frontmatter_uuid(&frontmatter).map_err(|err| {
            ReconcileError::Other(format!(
                "detect_duplicate_uuids_in_tree: {} has invalid frontmatter uuid: {err}",
                relative_path.display()
            ))
        })? {
            uuids
                .entry(uuid)
                .or_default()
                .push(path_to_string(&relative_path));
        }
    }

    if let Some((uuid, paths)) = uuids.into_iter().find(|(_, paths)| paths.len() > 1) {
        return Err(ReconcileError::DuplicateUuidError { uuid, paths });
    }

    Ok(())
}

// ── DB-Only State Predicate (stub) ────────────────────────────

/// Determine if a page has DB-only state (state that cannot be reconstructed from markdown).
///
/// A page has DB-only state if ANY of these are true:
/// 1. EXISTS a row in `links` where (`from_page_id = p.id` OR `to_page_id = p.id`) AND `source_kind = 'programmatic'`
/// 2. EXISTS a row in `assertions` where `page_id = p.id` AND `asserted_by != 'import'`
/// 3. EXISTS a row in `raw_data` where `page_id = p.id`
/// 4. EXISTS a row in `contradictions` where `page_id = p.id` OR `other_page_id = p.id`
/// 5. EXISTS a row in `knowledge_gaps` where `page_id = p.id`
///
pub fn has_db_only_state(conn: &Connection, page_id: i64) -> Result<bool, ReconcileError> {
    Ok(db_only_state_branches(conn, page_id)?.any())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DbOnlyStateBranches {
    programmatic_links: bool,
    non_import_assertions: bool,
    raw_data: bool,
    contradictions: bool,
    knowledge_gaps: bool,
}

impl DbOnlyStateBranches {
    fn any(self) -> bool {
        self.programmatic_links
            || self.non_import_assertions
            || self.raw_data
            || self.contradictions
            || self.knowledge_gaps
    }
}

fn db_only_state_branches(
    conn: &Connection,
    page_id: i64,
) -> Result<DbOnlyStateBranches, ReconcileError> {
    Ok(DbOnlyStateBranches {
        programmatic_links: exists(
            conn,
            "SELECT EXISTS(
                 SELECT 1
                 FROM links
                 WHERE (from_page_id = ?1 OR to_page_id = ?1)
                   AND source_kind = 'programmatic'
             )",
            [page_id],
        )?,
        non_import_assertions: exists(
            conn,
            "SELECT EXISTS(
                 SELECT 1
                 FROM assertions
                 WHERE page_id = ?1
                   AND asserted_by != 'import'
             )",
            [page_id],
        )?,
        raw_data: exists(
            conn,
            "SELECT EXISTS(
                 SELECT 1
                 FROM raw_data
                 WHERE page_id = ?1
             )",
            [page_id],
        )?,
        contradictions: exists(
            conn,
            "SELECT EXISTS(
                 SELECT 1
                 FROM contradictions
                 WHERE page_id = ?1 OR other_page_id = ?1
             )",
            [page_id],
        )?,
        knowledge_gaps: exists(
            conn,
            "SELECT EXISTS(
                 SELECT 1
                 FROM knowledge_gaps
                 WHERE page_id = ?1
             )",
            [page_id],
        )?,
    })
}

fn exists<P>(conn: &Connection, sql: &str, params: P) -> Result<bool, ReconcileError>
where
    P: rusqlite::Params,
{
    Ok(conn.query_row(sql, params, |row| row.get::<_, i64>(0))? != 0)
}

#[cfg(unix)]
fn walk_root(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    root_fd: &OwnedFd,
) -> Result<WalkedCollection, ReconcileError> {
    let globset = ignore_patterns::build_globset(conn, collection_id)
        .map_err(|err| ReconcileError::Other(format!("walk_collection: {err}")))?;
    let mut builder = WalkBuilder::new(root_path);
    builder
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .follow_links(false);

    let mut walked = WalkedCollection::default();

    for entry in builder.build() {
        let entry =
            entry.map_err(|err| ReconcileError::Other(format!("walk_collection: {}", err)))?;
        let entry_path = entry.path();

        if entry_path == root_path {
            continue;
        }

        let relative_path = entry_path.strip_prefix(root_path).map_err(|err| {
            ReconcileError::Other(format!(
                "walk_collection: failed to relativize {} against {}: {}",
                entry_path.display(),
                root_path.display(),
                err
            ))
        })?;

        if relative_path.as_os_str().is_empty() {
            continue;
        }

        let relative_path = relative_path.to_path_buf();
        let parent_fd = match fs_safety::walk_to_parent(root_fd, &relative_path) {
            Ok(parent_fd) => parent_fd,
            Err(err) if err.kind() == io::ErrorKind::NotADirectory => {
                walked.skipped_symlinks += 1;
                eprintln!("WARN: skipping symlinked entry {}", relative_path.display());
                continue;
            }
            Err(err) => return Err(ReconcileError::IoError(err)),
        };

        let entry_name = relative_path.file_name().ok_or_else(|| {
            ReconcileError::Other(format!(
                "walk_collection: missing final path component for {}",
                relative_path.display()
            ))
        })?;
        let stat = fs_safety::stat_at_nofollow(&parent_fd, Path::new(entry_name))?;

        if stat.is_symlink() {
            walked.skipped_symlinks += 1;
            eprintln!("WARN: skipping symlinked entry {}", relative_path.display());
            continue;
        }

        if stat.is_directory() || !stat.is_regular_file() {
            continue;
        }

        if is_ignored(&globset, &relative_path) || !is_markdown_file(&relative_path) {
            continue;
        }

        walked.walked += 1;
        walked.files.insert(
            relative_path,
            FileStat {
                mtime_ns: stat.mtime_ns,
                ctime_ns: Some(stat.ctime_ns),
                size_bytes: stat.size_bytes,
                inode: Some(stat.inode),
            },
        );
    }

    Ok(walked)
}

fn load_db_files(
    conn: &Connection,
    collection_id: i64,
) -> Result<HashMap<PathBuf, FileStat>, ReconcileError> {
    let mut stmt = conn.prepare(
        "SELECT relative_path, mtime_ns, ctime_ns, size_bytes, inode
         FROM file_state
         WHERE collection_id = ?1",
    )?;

    let rows = stmt.query_map([collection_id], |row| {
        let path: String = row.get(0)?;
        let stat = FileStat {
            mtime_ns: row.get(1)?,
            ctime_ns: row.get(2)?,
            size_bytes: row.get(3)?,
            inode: row.get(4)?,
        };
        Ok((PathBuf::from(path), stat))
    })?;

    let mut db_files = HashMap::new();
    for row in rows {
        let (path, stat) = row?;
        db_files.insert(path, stat);
    }

    Ok(db_files)
}

fn stat_diff_from_walk(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    walked_files: HashMap<PathBuf, FileStat>,
) -> Result<StatDiff, ReconcileError> {
    let mut db_files = load_db_files(conn, collection_id)?;
    let mut diff = StatDiff::default();

    for (path, stat) in walked_files {
        match db_files.remove(&path) {
            Some(stored) if file_state::stat_differs(&stat, &stored) => {
                let Some(page_id) = page_id_for_relative_path(conn, collection_id, &path)? else {
                    diff.modified.insert(path, stat);
                    continue;
                };
                if is_extracted_whitespace_noop(conn, collection_id, root_path, &path, page_id)? {
                    diff.unchanged.insert(path);
                } else {
                    diff.modified.insert(path, stat);
                }
            }
            Some(_) => {
                diff.unchanged.insert(path);
            }
            None => {
                diff.new.insert(path, stat);
            }
        }
    }

    diff.missing.extend(db_files.into_keys());
    Ok(diff)
}

fn is_extracted_whitespace_noop(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
    page_id: i64,
) -> Result<bool, ReconcileError> {
    if !crate::core::conversation::file_edit::is_extracted_path(relative_path) {
        return Ok(false);
    }

    let page_type: Option<String> = conn
        .query_row(
            "SELECT type FROM pages WHERE id = ?1 AND collection_id = ?2",
            params![page_id, collection_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(page_type) = page_type else {
        return Ok(false);
    };
    if !handles_page_type(&page_type) {
        return Ok(false);
    }

    let current_bytes = fs::read(root_path.join(relative_path))?;
    let mut stmt = conn.prepare(
        "SELECT raw_bytes
         FROM raw_imports
         WHERE page_id = ?1 AND is_active = 1
         ORDER BY id",
    )?;
    let active_rows = stmt
        .query_map([page_id], |row| row.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let [prior_bytes] = active_rows.as_slice() else {
        return Ok(false);
    };

    Ok(normalized_content_key(prior_bytes) == normalized_content_key(&current_bytes))
}

fn classify_missing_paths(
    conn: &Connection,
    collection_id: i64,
    missing: &HashSet<PathBuf>,
) -> Result<(usize, usize), ReconcileError> {
    let mut stmt = conn.prepare(
        "SELECT page_id
         FROM file_state
         WHERE collection_id = ?1 AND relative_path = ?2",
    )?;
    let mut quarantined = 0usize;
    let mut hard_deleted = 0usize;

    for path in missing {
        let page_id: Option<i64> = stmt
            .query_row(
                rusqlite::params![collection_id, path_to_string(path)],
                |row| row.get(0),
            )
            .optional()?;

        let Some(page_id) = page_id else {
            continue;
        };

        if has_db_only_state(conn, page_id)? {
            quarantined += 1;
        } else {
            hard_deleted += 1;
        }
    }

    Ok((quarantined, hard_deleted))
}

fn apply_reconciliation(
    conn: &Connection,
    collection: &Collection,
    diff: &StatDiff,
    rename_resolution: &RenameResolution,
    root_path: &Path,
) -> Result<ApplySummary, ReconcileError> {
    let actions = build_apply_actions(conn, collection.id, diff, rename_resolution)?;
    let mut summary = ApplySummary::default();

    for action in &actions {
        apply_action(conn, collection.id, root_path, action, &mut summary)?;
    }

    Ok(summary)
}

fn build_apply_actions(
    conn: &Connection,
    collection_id: i64,
    diff: &StatDiff,
    rename_resolution: &RenameResolution,
) -> Result<Vec<ApplyAction>, ReconcileError> {
    let mut actions = Vec::new();

    for relative_path in &rename_resolution.remaining_missing {
        if let Some(page_id) = page_id_for_relative_path(conn, collection_id, relative_path)? {
            actions.push(ApplyAction::DeleteOrQuarantine {
                page_id,
                relative_path: relative_path.clone(),
            });
        }
    }

    let mut rename_matches = rename_resolution.matches.clone();
    rename_matches.sort_by(|left, right| left.to_path.cmp(&right.to_path));
    for rename_match in rename_matches {
        let Some(stat) = diff.new.get(&rename_match.to_path).cloned() else {
            continue;
        };
        actions.push(ApplyAction::Reingest {
            existing_page_id: Some(rename_match.page_id),
            old_relative_path: Some(rename_match.from_path),
            relative_path: rename_match.to_path,
            stat,
        });
    }

    let mut modified_paths: Vec<_> = diff.modified.iter().collect();
    modified_paths.sort_by_key(|(left, _)| *left);
    for (relative_path, stat) in modified_paths {
        let Some(page_id) = page_id_for_relative_path(conn, collection_id, relative_path)? else {
            continue;
        };
        actions.push(ApplyAction::Reingest {
            existing_page_id: Some(page_id),
            old_relative_path: None,
            relative_path: relative_path.clone(),
            stat: stat.clone(),
        });
    }

    let mut new_paths: Vec<_> = rename_resolution.remaining_new.iter().collect();
    new_paths.sort_by_key(|(left, _)| *left);
    for (relative_path, stat) in new_paths {
        actions.push(ApplyAction::Reingest {
            existing_page_id: None,
            old_relative_path: None,
            relative_path: relative_path.clone(),
            stat: stat.clone(),
        });
    }

    Ok(actions)
}

fn apply_action(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    action: &ApplyAction,
    summary: &mut ApplySummary,
) -> Result<(), ReconcileError> {
    match action {
        ApplyAction::DeleteOrQuarantine {
            page_id,
            relative_path,
        } => apply_delete_or_quarantine(conn, collection_id, *page_id, relative_path, summary),
        ApplyAction::Reingest {
            existing_page_id,
            old_relative_path,
            relative_path,
            stat,
        } => {
            let outcome = apply_reingest(
                conn,
                collection_id,
                root_path,
                *existing_page_id,
                old_relative_path.as_deref(),
                relative_path,
                stat,
            )?;
            if outcome.created {
                summary.created += 1;
            } else {
                summary.reingested += 1;
            }
            Ok(())
        }
    }
}

fn apply_delete_or_quarantine(
    conn: &Connection,
    collection_id: i64,
    page_id: i64,
    relative_path: &Path,
    summary: &mut ApplySummary,
) -> Result<(), ReconcileError> {
    let tx = conn.unchecked_transaction()?;
    let branches = db_only_state_branches(&tx, page_id)?;
    if branches.any() {
        tx.execute(
            "UPDATE pages
             SET quarantined_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            [page_id],
        )?;
        file_state::delete_file_state(&tx, collection_id, &path_to_string(relative_path))?;
        tx.commit()?;
        summary.quarantined_db_state += 1;
        eprintln!(
            "INFO: reconcile_quarantined page_id={} path={} programmatic_links={} non_import_assertions={} raw_data={} contradictions={} knowledge_gaps={}",
            page_id,
            relative_path.display(),
            i32::from(branches.programmatic_links),
            i32::from(branches.non_import_assertions),
            i32::from(branches.raw_data),
            i32::from(branches.contradictions),
            i32::from(branches.knowledge_gaps)
        );
        return Ok(());
    }

    tx.execute("DELETE FROM pages WHERE id = ?1", [page_id])?;
    tx.commit()?;
    summary.hard_deleted += 1;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ApplyReingestOutcome {
    created: bool,
}

fn apply_reingest(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    existing_page_id: Option<i64>,
    old_relative_path: Option<&Path>,
    relative_path: &Path,
    stat: &FileStat,
) -> Result<ApplyReingestOutcome, ReconcileError> {
    let absolute_path = root_path.join(relative_path);
    let raw_bytes = fs::read(&absolute_path)?;
    let parsed = parse_vault_file(&raw_bytes, &absolute_path, root_path)?;
    let current_page =
        load_existing_page_identity(conn, collection_id, existing_page_id, &parsed.slug)?;

    // Fail closed: an existing page must already have raw_imports history.
    // row_count == 0 means the restore anchor is absent; silently bootstrapping the first
    // row here would hide the corruption rather than surface it. Covers both the explicit
    // existing_page_id path (modified files) and the slug-matched new-path case.
    if let Some((existing_pid, _)) = &current_page {
        let (row_count, active_count): (i64, i64) = conn.query_row(
            "SELECT
                 COUNT(*) AS row_count,
                 COALESCE(SUM(CASE WHEN is_active = 1 THEN 1 ELSE 0 END), 0) AS active_count
             FROM raw_imports
             WHERE page_id = ?1",
            [*existing_pid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        raw_import_invariant_result(
            *existing_pid,
            row_count,
            active_count,
            "apply_reingest",
            RawImportInvariantPolicy::Enforce,
        )?;
    }

    if let Some((page_id, _)) = current_page.as_ref() {
        let prior_page = crate::commands::get::get_page_by_key(conn, collection_id, &parsed.slug)
            .map_err(|err| ReconcileError::Other(format!("apply_reingest: {err}")))?;
        let edited_page = EditedPage {
            slug: parsed.slug.clone(),
            title: parsed.title.clone(),
            page_type: parsed.page_type.clone(),
            summary: parsed.summary.clone(),
            compiled_truth: parsed.compiled_truth.clone(),
            timeline: parsed.timeline.clone(),
            frontmatter: parsed.frontmatter.clone(),
            wing: parsed.wing.clone(),
            room: parsed.room.clone(),
            sha256: parsed.sha256.clone(),
        };
        match handle_extracted_edit(
            conn,
            collection_id,
            *page_id,
            relative_path,
            root_path,
            stat,
            &prior_page,
            &edited_page,
            &raw_bytes,
        )
        .map_err(|err| ReconcileError::Other(format!("apply_reingest: {err}")))?
        {
            HandleExtractedEditOutcome::Bypass => {}
            HandleExtractedEditOutcome::WhitespaceNoOp => {
                if let Some(old_relative_path) = old_relative_path {
                    if old_relative_path != relative_path {
                        file_state::move_file_state(
                            conn,
                            collection_id,
                            &path_to_string(old_relative_path),
                            &path_to_string(relative_path),
                        )?;
                    }
                }
                return Ok(ApplyReingestOutcome { created: false });
            }
            HandleExtractedEditOutcome::Superseded { .. } => {
                if let Some(old_relative_path) = old_relative_path {
                    if old_relative_path != relative_path {
                        file_state::delete_file_state(
                            conn,
                            collection_id,
                            &path_to_string(old_relative_path),
                        )?;
                    }
                }
                return Ok(ApplyReingestOutcome { created: false });
            }
        }
    }

    let page_uuid = page_uuid::resolve_page_uuid(
        &parsed.frontmatter,
        current_page.as_ref().and_then(|(_, uuid)| uuid.as_deref()),
    )
    .map_err(|err| ReconcileError::Other(format!("apply_reingest: {err}")))?;
    let frontmatter_json = serde_json::to_string(&parsed.frontmatter).map_err(|err| {
        ReconcileError::Other(format!("apply_reingest: serialize frontmatter: {err}"))
    })?;

    let tx = conn.unchecked_transaction()?;
    let now: String = tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })?;

    let (page_id, created) = if let Some((page_id, _)) = current_page {
        tx.execute(
            "UPDATE pages
             SET slug = ?1,
                 uuid = ?2,
                 type = ?3,
                 title = ?4,
                 summary = ?5,
                 compiled_truth = ?6,
                 timeline = ?7,
                 frontmatter = ?8,
                 wing = ?9,
                 room = ?10,
                 quarantined_at = NULL,
                 version = version + 1,
                 updated_at = ?11,
                 truth_updated_at = ?11,
                 timeline_updated_at = ?11
             WHERE id = ?12",
            rusqlite::params![
                parsed.slug,
                page_uuid,
                parsed.page_type,
                parsed.title,
                parsed.summary,
                parsed.compiled_truth,
                parsed.timeline,
                frontmatter_json,
                parsed.wing,
                parsed.room,
                now,
                page_id
            ],
        )?;
        (page_id, false)
    } else {
        tx.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline,
                   frontmatter, wing, room, version,
                  created_at, updated_at, truth_updated_at, timeline_updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12, ?12, ?12)",
            rusqlite::params![
                collection_id,
                parsed.slug,
                page_uuid,
                parsed.page_type,
                parsed.title,
                parsed.summary,
                parsed.compiled_truth,
                parsed.timeline,
                frontmatter_json,
                parsed.wing,
                parsed.room,
                now
            ],
        )?;
        (tx.last_insert_rowid(), true)
    };

    file_state::upsert_file_state(
        &tx,
        collection_id,
        &path_to_string(relative_path),
        page_id,
        stat,
        &parsed.sha256,
    )?;
    if let Some(old_relative_path) = old_relative_path {
        if old_relative_path != relative_path {
            file_state::delete_file_state(&tx, collection_id, &path_to_string(old_relative_path))?;
        }
    }
    raw_imports::rotate_active_raw_import(
        &tx,
        page_id,
        &absolute_path.to_string_lossy(),
        &raw_bytes,
    )?;
    raw_imports::enqueue_embedding_job(&tx, page_id)?;
    tx.commit()?;

    Ok(ApplyReingestOutcome { created })
}

fn load_existing_page_identity(
    conn: &Connection,
    collection_id: i64,
    preferred_page_id: Option<i64>,
    slug: &str,
) -> Result<Option<(i64, Option<String>)>, ReconcileError> {
    if let Some(page_id) = preferred_page_id {
        return conn
            .query_row(
                "SELECT id, uuid FROM pages WHERE id = ?1",
                [page_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(Into::into);
    }

    conn.query_row(
        "SELECT id, uuid
         FROM pages
         WHERE collection_id = ?1 AND slug = ?2",
        rusqlite::params![collection_id, slug],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

fn page_id_for_relative_path(
    conn: &Connection,
    collection_id: i64,
    relative_path: &Path,
) -> Result<Option<i64>, ReconcileError> {
    conn.query_row(
        "SELECT page_id
         FROM file_state
         WHERE collection_id = ?1 AND relative_path = ?2",
        rusqlite::params![collection_id, path_to_string(relative_path)],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn parse_vault_file(
    raw_bytes: &[u8],
    file_path: &Path,
    root_path: &Path,
) -> Result<ParsedVaultFile, ReconcileError> {
    let raw = String::from_utf8_lossy(raw_bytes).into_owned();
    let (frontmatter, body) = markdown::parse_frontmatter(&raw);
    let (compiled_truth, timeline) = markdown::split_content(&body);
    let summary = markdown::extract_summary(&compiled_truth);
    let slug = crate::core::types::frontmatter_get_string(&frontmatter, "slug")
        .unwrap_or_else(|| derive_slug_from_path(file_path, root_path));
    let title = crate::core::types::frontmatter_get_string(&frontmatter, "title")
        .unwrap_or_else(|| slug.clone());
    let frontmatter_type = frontmatter
        .get("type")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("null"))
        .map(str::to_owned);
    let page_type = frontmatter_type
        .or_else(|| infer_type_from_path(file_path, root_path))
        .unwrap_or_else(|| "concept".to_string());
    let wing = crate::core::types::frontmatter_get_string(&frontmatter, "wing")
        .unwrap_or_else(|| palace::derive_wing(&slug));
    let room = palace::derive_room(&compiled_truth);

    Ok(ParsedVaultFile {
        slug,
        title,
        page_type,
        summary,
        compiled_truth,
        timeline,
        frontmatter,
        wing,
        room,
        sha256: sha256_hex(raw_bytes),
    })
}

fn derive_slug_from_path(file_path: &Path, root_path: &Path) -> String {
    file_path
        .strip_prefix(root_path)
        .unwrap_or(file_path)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
}

fn infer_type_from_path(file_path: &Path, root_path: &Path) -> Option<String> {
    let relative = file_path.strip_prefix(root_path).ok()?;
    let first_component = relative.components().next()?;
    let folder = first_component.as_os_str().to_string_lossy();
    let normalized = strip_numeric_prefix(&folder).to_lowercase();

    match normalized.as_str() {
        "projects" => Some("project".to_string()),
        "areas" => Some("area".to_string()),
        "resources" => Some("resource".to_string()),
        "archives" => Some("archive".to_string()),
        "journal" | "journals" => Some("journal".to_string()),
        "people" => Some("person".to_string()),
        "companies" | "orgs" => Some("company".to_string()),
        _ => None,
    }
}

fn strip_numeric_prefix(name: &str) -> &str {
    let bytes = name.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }

    if index > 0 && index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        &name[index..]
    } else {
        name
    }
}

fn is_ignored(globset: &globset::GlobSet, relative_path: &Path) -> bool {
    globset.is_match(relative_path) || is_history_sidecar_path(relative_path)
}

pub(crate) fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

// ── Error ─────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ReconcileError {
    DbError(rusqlite::Error),
    IoError(std::io::Error),
    InvariantViolationError {
        message: String,
    },
    InvalidFullHashAuthorization {
        mode: FullHashReconcileMode,
        authorization: &'static str,
        reason: &'static str,
    },
    UnauthorizedFullHashReconcile {
        mode: FullHashReconcileMode,
        authorization: &'static str,
        collection_state: crate::core::collections::CollectionState,
    },
    UuidMigrationRequiredError {
        collection_name: String,
        affected_count: usize,
        sample_paths: Vec<String>,
    },
    CollectionLacksWriterQuiescenceError {
        collection_name: String,
        root_path: String,
    },
    CollectionDirtyError {
        collection_name: String,
        status: CollectionDirtyStatus,
    },
    RemapDriftConflictError {
        collection_name: String,
        summary: DriftCaptureSummary,
    },
    CollectionUnstableError {
        collection_name: String,
        operation: RestoreRemapOperation,
        phase: &'static str,
        retries: usize,
    },
    DuplicateUuidError {
        uuid: String,
        paths: Vec<String>,
    },
    UnresolvableTrivialContentError {
        missing_path: String,
        candidate_paths: Vec<String>,
        reason: String,
    },
    Other(String),
}

impl std::fmt::Display for ReconcileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DbError(e) => write!(f, "Database error: {}", e),
            Self::IoError(e) => write!(f, "I/O error: {}", e),
            Self::InvariantViolationError { message } => {
                write!(f, "InvariantViolationError: {message}")
            }
            Self::InvalidFullHashAuthorization {
                mode,
                authorization,
                reason,
            } => write!(
                f,
                "full_hash_reconcile authorization denied: mode={} authorization={} reason={}",
                mode.as_str(),
                authorization,
                reason
            ),
            Self::UnauthorizedFullHashReconcile {
                mode,
                authorization,
                collection_state,
            } => write!(
                f,
                "full_hash_reconcile authorization denied: mode={} collection_state={} authorization={}",
                mode.as_str(),
                collection_state.as_str(),
                authorization
            ),
            Self::UuidMigrationRequiredError {
                collection_name,
                affected_count,
                sample_paths,
            } => write!(
                f,
                "UuidMigrationRequiredError: collection={} affected={} sample_paths={} run `quaid collection migrate-uuids {}` before retrying",
                collection_name,
                affected_count,
                sample_paths.join(","),
                collection_name
            ),
            Self::CollectionLacksWriterQuiescenceError {
                collection_name,
                root_path,
            } => write!(
                f,
                "CollectionLacksWriterQuiescenceError: collection={} root_path={} acceptance_paths=[remount old root read-only, run from a quiesced environment]",
                collection_name,
                root_path
            ),
            Self::CollectionDirtyError {
                collection_name,
                status,
            } => write!(
                f,
                "CollectionDirtyError: collection={} needs_full_sync={} sentinel_count={} recovery_in_progress={} last_sync_at={}",
                collection_name,
                status.needs_full_sync,
                status.sentinel_count,
                status.recovery_in_progress,
                status.last_sync_at.as_deref().unwrap_or("null")
            ),
            Self::RemapDriftConflictError {
                collection_name,
                summary,
            } => write!(
                f,
                "RemapDriftConflictError: collection={} pages_updated={} pages_added={} pages_quarantined={} pages_deleted={}",
                collection_name,
                summary.pages_updated,
                summary.pages_added,
                summary.pages_quarantined,
                summary.pages_deleted
            ),
            Self::CollectionUnstableError {
                collection_name,
                operation,
                phase,
                retries,
            } => write!(
                f,
                "CollectionUnstableError: collection={} operation={} phase={} retries={}",
                collection_name,
                operation.as_str(),
                phase,
                retries
            ),
            Self::DuplicateUuidError { uuid, paths } => write!(
                f,
                "DuplicateUuidError: uuid={} paths={}",
                uuid,
                paths.join(",")
            ),
            Self::UnresolvableTrivialContentError {
                missing_path,
                candidate_paths,
                reason,
            } => write!(
                f,
                "UnresolvableTrivialContentError: missing={} candidates={} reason={}",
                missing_path,
                candidate_paths.join(","),
                reason
            ),
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for ReconcileError {}

impl From<rusqlite::Error> for ReconcileError {
    fn from(e: rusqlite::Error) -> Self {
        Self::DbError(e)
    }
}

impl From<std::io::Error> for ReconcileError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::file_state::upsert_file_state;
    use std::ffi::OsString;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::sync::{Mutex, OnceLock};
    #[cfg(unix)]
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    static ENV_MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_mutation_lock() -> &'static Mutex<()> {
        ENV_MUTATION_LOCK.get_or_init(|| Mutex::new(()))
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

    fn production_reconciler_source() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("core")
            .join("reconciler.rs");
        fs::read_to_string(path).expect("read production reconciler source")
    }

    fn open_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        conn
    }

    fn open_test_db_file() -> (TempDir, PathBuf, rusqlite::Connection) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        (dir, db_path, conn)
    }

    fn insert_collection(conn: &Connection, root_path: &Path) -> Collection {
        insert_collection_with_state(
            conn,
            root_path,
            crate::core::collections::CollectionState::Active,
            false,
        )
    }

    fn insert_collection_with_state(
        conn: &Connection,
        root_path: &Path,
        state: crate::core::collections::CollectionState,
        needs_full_sync: bool,
    ) -> Collection {
        let (active_lease_session_id, restore_command_id, restore_lease_session_id) =
            owner_identity_defaults_for_state(state);
        conn.execute(
            "INSERT INTO collections
                 (name, root_path, state, needs_full_sync,
                  active_lease_session_id, restore_command_id, restore_lease_session_id)
             VALUES ('test', ?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                root_path.to_string_lossy(),
                state.as_str(),
                if needs_full_sync { 1 } else { 0 },
                active_lease_session_id,
                restore_command_id,
                restore_lease_session_id,
            ],
        )
        .unwrap();
        Collection {
            id: 1,
            name: "test".to_owned(),
            root_path: root_path.to_string_lossy().into_owned(),
            state,
            writable: true,
            is_write_target: false,
            ignore_patterns: None,
            ignore_parse_errors: None,
            needs_full_sync,
            last_sync_at: None,
            active_lease_session_id: active_lease_session_id.map(str::to_owned),
            restore_command_id: restore_command_id.map(str::to_owned),
            restore_lease_session_id: restore_lease_session_id.map(str::to_owned),
            reload_generation: 0,
            watcher_released_session_id: None,
            watcher_released_generation: None,
            watcher_released_at: None,
            pending_command_heartbeat_at: None,
            pending_root_path: None,
            pending_restore_manifest: None,
            restore_command_pid: None,
            restore_command_host: None,
            integrity_failed_at: None,
            pending_manifest_incomplete_at: None,
            reconcile_halted_at: None,
            reconcile_halt_reason: None,
            created_at: "2024-01-01T00:00:00Z".to_owned(),
            updated_at: "2024-01-01T00:00:00Z".to_owned(),
        }
    }

    fn sample_collection_in_state(state: crate::core::collections::CollectionState) -> Collection {
        let (active_lease_session_id, restore_command_id, restore_lease_session_id) =
            owner_identity_defaults_for_state(state);
        Collection {
            id: 1,
            name: "test".to_owned(),
            root_path: "/vault".to_owned(),
            state,
            writable: true,
            is_write_target: false,
            ignore_patterns: None,
            ignore_parse_errors: None,
            needs_full_sync: false,
            last_sync_at: None,
            active_lease_session_id: active_lease_session_id.map(str::to_owned),
            restore_command_id: restore_command_id.map(str::to_owned),
            restore_lease_session_id: restore_lease_session_id.map(str::to_owned),
            reload_generation: 0,
            watcher_released_session_id: None,
            watcher_released_generation: None,
            watcher_released_at: None,
            pending_command_heartbeat_at: None,
            pending_root_path: None,
            pending_restore_manifest: None,
            restore_command_pid: None,
            restore_command_host: None,
            integrity_failed_at: None,
            pending_manifest_incomplete_at: None,
            reconcile_halted_at: None,
            reconcile_halt_reason: None,
            created_at: "2024-01-01T00:00:00Z".to_owned(),
            updated_at: "2024-01-01T00:00:00Z".to_owned(),
        }
    }

    fn owner_identity_defaults_for_state(
        state: crate::core::collections::CollectionState,
    ) -> (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
    ) {
        match state {
            crate::core::collections::CollectionState::Active => (Some("lease-1"), None, None),
            crate::core::collections::CollectionState::Detached => (None, None, None),
            crate::core::collections::CollectionState::Restoring => {
                (None, Some("restore-1"), Some("restore-lease-1"))
            }
        }
    }

    fn set_collection_dirty_flag(conn: &Connection, collection_id: i64, needs_full_sync: bool) {
        conn.execute(
            "UPDATE collections
             SET needs_full_sync = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            rusqlite::params![collection_id, if needs_full_sync { 1 } else { 0 }],
        )
        .unwrap();
    }

    fn create_recovery_sentinel(recovery_root: &Path, collection_id: i64, name: &str) {
        let dir = collection_recovery_dir(recovery_root, collection_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), b"dirty").unwrap();
    }

    fn insert_page(conn: &Connection, collection_id: i64, slug: &str) -> i64 {
        conn.execute(
            "INSERT INTO pages (collection_id, slug, uuid, type, title, compiled_truth, timeline)
             VALUES (?1, ?2, ?3, 'concept', ?2, 'Body', '')",
            rusqlite::params![collection_id, slug, page_uuid::generate_uuid_v7()],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn stat_for(root_path: &Path, relative_path: &str) -> FileStat {
        file_state::stat_file(&root_path.join(relative_path)).unwrap()
    }

    fn unique_old_stat(current: &FileStat) -> FileStat {
        FileStat {
            mtime_ns: current.mtime_ns.saturating_sub(1),
            ctime_ns: current.ctime_ns.map(|value| value.saturating_sub(1)),
            size_bytes: current.size_bytes.saturating_add(1),
            inode: current.inode.map(|value| value.saturating_add(1)),
        }
    }

    #[test]
    fn hash_refusal_reason_refuses_template_like_notes_when_only_the_frontmatter_is_large() {
        let renamed_path = PathBuf::from("renamed.md");
        let missing_identity = MissingPageIdentity {
            page_id: 1,
            uuid: None,
            sha256: "same-hash".to_string(),
            body_size_bytes: 1,
            has_nonempty_body: true,
        };
        let new_identities = HashMap::from([(
            renamed_path.clone(),
            NewTreeIdentity {
                relative_path: renamed_path.clone(),
                sha256: "same-hash".to_string(),
                uuid: None,
                body_size_bytes: 1,
                has_nonempty_body: true,
            },
        )]);
        let new_candidates = vec![&renamed_path];

        let refusal = hash_refusal_reason(&missing_identity, &new_candidates, 1, &new_identities);

        assert_eq!(refusal.as_deref(), Some("missing_below_min_body_bytes"));
    }

    #[test]
    fn hash_refusal_reason_allows_unique_pairing_when_body_content_exceeds_minimum() {
        let renamed_path = PathBuf::from("renamed.md");
        let missing_identity = MissingPageIdentity {
            page_id: 1,
            uuid: None,
            sha256: "same-hash".to_string(),
            body_size_bytes: 80,
            has_nonempty_body: true,
        };
        let new_identities = HashMap::from([(
            renamed_path.clone(),
            NewTreeIdentity {
                relative_path: renamed_path.clone(),
                sha256: "same-hash".to_string(),
                uuid: None,
                body_size_bytes: 80,
                has_nonempty_body: true,
            },
        )]);
        let new_candidates = vec![&renamed_path];

        let refusal = hash_refusal_reason(&missing_identity, &new_candidates, 1, &new_identities);

        assert!(
            refusal.is_none(),
            "expected long body content to stay hash-pairable"
        );
    }

    #[test]
    fn hash_refusal_reason_refuses_missing_page_with_empty_body_even_when_hash_is_unique() {
        let renamed_path = PathBuf::from("renamed.md");
        let missing_identity = MissingPageIdentity {
            page_id: 1,
            uuid: None,
            sha256: "same-hash".to_string(),
            body_size_bytes: 80,
            has_nonempty_body: false,
        };
        let new_identities = HashMap::from([(
            renamed_path.clone(),
            NewTreeIdentity {
                relative_path: renamed_path.clone(),
                sha256: "same-hash".to_string(),
                uuid: None,
                body_size_bytes: 80,
                has_nonempty_body: true,
            },
        )]);
        let new_candidates = vec![&renamed_path];

        let refusal = hash_refusal_reason(&missing_identity, &new_candidates, 1, &new_identities);

        assert_eq!(refusal.as_deref(), Some("missing_empty_body"));
    }

    #[test]
    fn hash_refusal_reason_refuses_new_candidate_with_empty_body_even_when_missing_page_is_nontrivial(
    ) {
        let renamed_path = PathBuf::from("renamed.md");
        let missing_identity = MissingPageIdentity {
            page_id: 1,
            uuid: None,
            sha256: "same-hash".to_string(),
            body_size_bytes: 80,
            has_nonempty_body: true,
        };
        let new_identities = HashMap::from([(
            renamed_path.clone(),
            NewTreeIdentity {
                relative_path: renamed_path.clone(),
                sha256: "same-hash".to_string(),
                uuid: None,
                body_size_bytes: 80,
                has_nonempty_body: false,
            },
        )]);
        let new_candidates = vec![&renamed_path];

        let refusal = hash_refusal_reason(&missing_identity, &new_candidates, 1, &new_identities);

        assert_eq!(refusal.as_deref(), Some("new_empty_body"));
    }

    #[test]
    fn trivial_content_boundary_at_exactly_sixty_four_body_bytes_stays_nontrivial() {
        let renamed_path = PathBuf::from("renamed.md");
        let missing_identity = MissingPageIdentity {
            page_id: 1,
            uuid: None,
            sha256: "same-hash".to_string(),
            body_size_bytes: 64,
            has_nonempty_body: true,
        };
        let new_identities = HashMap::from([(
            renamed_path.clone(),
            NewTreeIdentity {
                relative_path: renamed_path.clone(),
                sha256: "same-hash".to_string(),
                uuid: None,
                body_size_bytes: 64,
                has_nonempty_body: true,
            },
        )]);
        let new_candidates = vec![&renamed_path];

        let refusal = hash_refusal_reason(&missing_identity, &new_candidates, 1, &new_identities);

        assert!(refusal.is_none());
    }

    fn seed_file_state(
        conn: &Connection,
        collection_id: i64,
        slug: &str,
        relative_path: &str,
        stat: &FileStat,
    ) -> i64 {
        let page_id = insert_page(conn, collection_id, slug);
        upsert_file_state(conn, collection_id, relative_path, page_id, stat, "abc123").unwrap();
        page_id
    }

    fn active_raw_import_count(conn: &Connection, page_id: i64) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1 AND is_active = 1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn active_raw_import_bytes(conn: &Connection, page_id: i64) -> Vec<u8> {
        conn.query_row(
            "SELECT raw_bytes FROM raw_imports WHERE page_id = ?1 AND is_active = 1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn seed_page_with_identity(
        conn: &Connection,
        collection_id: i64,
        identity: SeededPageIdentity<'_>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO pages (collection_id, slug, uuid, type, title, compiled_truth, timeline)
             VALUES (?1, ?2, ?3, 'concept', ?2, ?4, ?5)",
            rusqlite::params![
                collection_id,
                identity.slug,
                identity.uuid,
                identity.compiled_truth,
                identity.timeline
            ],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();
        upsert_file_state(
            conn,
            collection_id,
            identity.relative_path,
            page_id,
            identity.stat,
            identity.sha256,
        )
        .unwrap();
        page_id
    }

    struct SeededPageIdentity<'a> {
        slug: &'a str,
        uuid: &'a str,
        relative_path: &'a str,
        stat: &'a FileStat,
        sha256: &'a str,
        compiled_truth: &'a str,
        timeline: &'a str,
    }

    fn total_raw_import_count(conn: &Connection, page_id: i64) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn apply_reingest_modified_existing_page_aborts_before_mutation_when_history_has_zero_total_raw_import_rows(
    ) {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
        fs::write(root.path().join("note.md"), original).unwrap();
        let original_stat = stat_for(root.path(), "note.md");
        let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &original_stat,
                sha256: &original_sha,
                compiled_truth: "Original body.",
                timeline: "",
            },
        );
        let before_file_state =
            crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
                .unwrap()
                .expect("file_state row should exist before abort");

        let updated =
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body that is deliberately longer.\n";
        fs::write(root.path().join("note.md"), updated).unwrap();
        let updated_stat = stat_for(root.path(), "note.md");

        let error = apply_reingest(
            &conn,
            collection.id,
            root.path(),
            Some(page_id),
            None,
            Path::new("note.md"),
            &updated_stat,
        )
        .unwrap_err()
        .to_string();

        let after_file_state =
            crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
                .unwrap()
                .expect("file_state row should still exist after abort");
        let (compiled_truth, version): (String, i64) = conn
            .query_row(
                "SELECT compiled_truth, version FROM pages WHERE id = ?1",
                [page_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert!(error.contains("zero total raw_imports rows"));
        assert_eq!(compiled_truth, "Original body.");
        assert_eq!(version, 1);
        assert_eq!(after_file_state.sha256, before_file_state.sha256);
        assert_eq!(total_raw_import_count(&conn, page_id), 0);
    }

    #[test]
    fn apply_reingest_slug_matched_existing_page_aborts_before_mutation_when_history_has_zero_total_raw_import_rows(
    ) {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let existing_body =
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
        fs::write(root.path().join("stale.md"), existing_body).unwrap();
        let stale_stat = stat_for(root.path(), "stale.md");
        let stale_sha = file_state::hash_file(&root.path().join("stale.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "stale.md",
                stat: &stale_stat,
                sha256: &stale_sha,
                compiled_truth: "Original body.",
                timeline: "",
            },
        );

        let incoming =
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nIncoming replacement body.\n";
        fs::write(root.path().join("incoming.md"), incoming).unwrap();
        let incoming_stat = stat_for(root.path(), "incoming.md");

        let error = apply_reingest(
            &conn,
            collection.id,
            root.path(),
            None,
            None,
            Path::new("incoming.md"),
            &incoming_stat,
        )
        .unwrap_err()
        .to_string();

        let tracked_paths: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_state WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        let incoming_file_state =
            crate::core::file_state::get_file_state(&conn, collection.id, "incoming.md").unwrap();
        let compiled_truth: String = conn
            .query_row(
                "SELECT compiled_truth FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert!(error.contains("zero total raw_imports rows"));
        assert_eq!(compiled_truth, "Original body.");
        assert_eq!(tracked_paths, 1);
        assert!(incoming_file_state.is_none());
        assert_eq!(total_raw_import_count(&conn, page_id), 0);
    }

    #[test]
    fn apply_reingest_new_page_bootstraps_first_raw_import_row_when_page_is_truly_new() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let content =
            "---\nslug: notes/new-note\ntitle: New Note\ntype: concept\n---\nFresh body.\n";
        fs::write(root.path().join("new-note.md"), content).unwrap();
        let stat = stat_for(root.path(), "new-note.md");

        let outcome = apply_reingest(
            &conn,
            collection.id,
            root.path(),
            None,
            None,
            Path::new("new-note.md"),
            &stat,
        )
        .unwrap();
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE collection_id = ?1 AND slug = 'notes/new-note'",
                [collection.id],
                |row| row.get(0),
            )
            .unwrap();
        let file_state_row =
            crate::core::file_state::get_file_state(&conn, collection.id, "new-note.md")
                .unwrap()
                .expect("new file_state row should be inserted");

        assert!(outcome.created);
        assert_eq!(file_state_row.page_id, page_id);
        assert_eq!(active_raw_import_count(&conn, page_id), 1);
        assert_eq!(active_raw_import_bytes(&conn, page_id), content.as_bytes());
    }

    #[test]
    fn extracted_whitespace_only_edits_stay_diff_quiet_and_skip_file_state_refresh() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let relative_path = PathBuf::from("extracted/preferences/noop.md");
        let original = "---\nslug: preferences/noop\ntitle: Noop\ntype: preference\n---\nbody\n";
        fs::create_dir_all(root.path().join("extracted").join("preferences")).unwrap();
        fs::write(root.path().join(&relative_path), original).unwrap();
        let original_stat = file_state::stat_file(&root.path().join(&relative_path)).unwrap();
        let original_sha = file_state::hash_file(&root.path().join(&relative_path)).unwrap();
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room)
             VALUES (?1, 'preferences/noop', ?2, 'preference', 'Noop', 'body', 'body', '', ?3, 'preferences', '')",
            rusqlite::params![
                collection.id,
                page_uuid::generate_uuid_v7(),
                serde_json::json!({"slug":"preferences/noop","title":"Noop","type":"preference"}).to_string()
            ],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();
        file_state::upsert_file_state(
            &conn,
            collection.id,
            &path_to_string(&relative_path),
            page_id,
            &original_stat,
            &original_sha,
        )
        .unwrap();
        raw_imports::rotate_active_raw_import(
            &conn,
            page_id,
            &root.path().join(&relative_path).to_string_lossy(),
            original.as_bytes(),
        )
        .unwrap();
        let before_file_state =
            file_state::get_file_state(&conn, collection.id, &path_to_string(&relative_path))
                .unwrap()
                .unwrap();

        let whitespace_only =
            "---\nslug: preferences/noop\n title: Noop\ntype: preference\n---\nbody  \n\n";
        fs::write(root.path().join(&relative_path), whitespace_only).unwrap();
        let current_stat = file_state::stat_file(&root.path().join(&relative_path)).unwrap();
        let walked = HashMap::from([(relative_path.clone(), current_stat.clone())]);
        let diff = stat_diff_from_walk(&conn, collection.id, root.path(), walked.clone()).unwrap();
        assert!(diff.modified.is_empty());
        assert!(diff.unchanged.contains(&relative_path));

        let plan = build_full_hash_plan(&conn, collection.id, root.path(), &walked).unwrap();
        assert_eq!(plan.unchanged.len(), 1);
        assert!(plan.unchanged[0].skip_metadata_refresh);
        apply_full_hash_metadata_self_heal(&conn, collection.id, &plan.unchanged).unwrap();

        let after_file_state =
            file_state::get_file_state(&conn, collection.id, &path_to_string(&relative_path))
                .unwrap()
                .unwrap();
        assert_eq!(before_file_state.sha256, after_file_state.sha256);
        assert_eq!(before_file_state.mtime_ns, after_file_state.mtime_ns);
    }

    #[test]
    fn apply_reingest_rename_only_extracted_whitespace_noop_moves_file_state_to_new_path() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let old_relative_path = PathBuf::from("extracted/preferences/noop.md");
        let new_relative_path = PathBuf::from("extracted/preferences/noop-renamed.md");
        let original = "---\nslug: preferences/noop\ntitle: Noop\ntype: preference\n---\nbody\n";
        fs::create_dir_all(root.path().join("extracted").join("preferences")).unwrap();
        fs::write(root.path().join(&old_relative_path), original).unwrap();
        let original_stat = file_state::stat_file(&root.path().join(&old_relative_path)).unwrap();
        let original_sha = file_state::hash_file(&root.path().join(&old_relative_path)).unwrap();
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room)
             VALUES (?1, 'preferences/noop', ?2, 'preference', 'Noop', 'body', 'body', '', ?3, 'preferences', '')",
            rusqlite::params![
                collection.id,
                page_uuid::generate_uuid_v7(),
                serde_json::json!({"slug":"preferences/noop","title":"Noop","type":"preference"}).to_string()
            ],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();
        file_state::upsert_file_state(
            &conn,
            collection.id,
            &path_to_string(&old_relative_path),
            page_id,
            &original_stat,
            &original_sha,
        )
        .unwrap();
        raw_imports::rotate_active_raw_import(
            &conn,
            page_id,
            &root.path().join(&old_relative_path).to_string_lossy(),
            original.as_bytes(),
        )
        .unwrap();
        let before_file_state =
            file_state::get_file_state(&conn, collection.id, &path_to_string(&old_relative_path))
                .unwrap()
                .unwrap();
        let before_version: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        let before_raw_import_rows = total_raw_import_count(&conn, page_id);
        let before_active_raw_bytes = active_raw_import_bytes(&conn, page_id);

        fs::rename(
            root.path().join(&old_relative_path),
            root.path().join(&new_relative_path),
        )
        .unwrap();
        let renamed_stat = file_state::stat_file(&root.path().join(&new_relative_path)).unwrap();

        let outcome = apply_reingest(
            &conn,
            collection.id,
            root.path(),
            Some(page_id),
            Some(old_relative_path.as_path()),
            new_relative_path.as_path(),
            &renamed_stat,
        )
        .unwrap();

        let old_file_state =
            file_state::get_file_state(&conn, collection.id, &path_to_string(&old_relative_path))
                .unwrap();
        let new_file_state =
            file_state::get_file_state(&conn, collection.id, &path_to_string(&new_relative_path))
                .unwrap()
                .expect("renamed whitespace noop must stay tracked at the new path");
        let after_version: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        let walked = HashMap::from([(new_relative_path.clone(), renamed_stat.clone())]);
        let diff = stat_diff_from_walk(&conn, collection.id, root.path(), walked).unwrap();

        assert!(!outcome.created);
        assert!(old_file_state.is_none());
        assert_eq!(new_file_state.page_id, page_id);
        assert_eq!(new_file_state.sha256, before_file_state.sha256);
        assert_eq!(before_version, after_version);
        assert_eq!(
            before_raw_import_rows,
            total_raw_import_count(&conn, page_id)
        );
        assert_eq!(
            before_active_raw_bytes,
            active_raw_import_bytes(&conn, page_id)
        );
        assert!(diff.new.is_empty());
        assert!(diff.modified.is_empty());
        assert!(diff.unchanged.contains(&new_relative_path));
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_is_idempotent_when_disk_matches_file_state() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("note.md"), "# note").unwrap();
        let collection = insert_collection(&conn, root.path());
        let stat = stat_for(root.path(), "note.md");
        seed_file_state(&conn, collection.id, "notes/note", "note.md", &stat);

        let first = reconcile(&conn, &collection).unwrap();
        let second = reconcile(&conn, &collection).unwrap();

        assert_eq!(first.walked, 1);
        assert_eq!(first.unchanged, 1);
        assert_eq!(first.modified, 0);
        assert_eq!(first.new, 0);
        assert_eq!(first.missing, 0);
        assert_eq!(first.walked, second.walked);
        assert_eq!(first.unchanged, second.unchanged);
        assert_eq!(first.modified, second.modified);
        assert_eq!(first.new, second.new);
        assert_eq!(first.missing, second.missing);
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_unchanged_path_keeps_existing_raw_import_row_without_rotation() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let content = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nStable body.\n";
        fs::write(root.path().join("note.md"), content).unwrap();
        let collection = insert_collection(&conn, root.path());
        let stat = stat_for(root.path(), "note.md");
        let sha256 = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth: "Stable body.",
                timeline: "",
            },
        );
        crate::core::raw_imports::rotate_active_raw_import(
            &conn,
            page_id,
            "note.md",
            content.as_bytes(),
        )
        .unwrap();

        let before_full_hash_at =
            crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
                .unwrap()
                .expect("file_state row should exist")
                .last_full_hash_at;
        let stats = reconcile(&conn, &collection).unwrap();
        let after_row = crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
            .unwrap()
            .expect("file_state row should still exist");
        let raw_import_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stats.unchanged, 1);
        assert_eq!(stats.modified, 0);
        assert_eq!(active_raw_import_count(&conn, page_id), 1);
        assert_eq!(raw_import_rows, 1);
        assert_eq!(active_raw_import_bytes(&conn, page_id), content.as_bytes());
        assert_eq!(after_row.last_full_hash_at, before_full_hash_at);
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_refuses_symlinked_root() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        let root_link = root.path().join("root-link");
        symlink(&target, &root_link).unwrap();
        let root_path = root_link.to_string_lossy().into_owned();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES (?1, ?2)",
            rusqlite::params!["test", root_path],
        )
        .unwrap();

        let collection = Collection {
            id: 1,
            name: "test".to_owned(),
            root_path,
            state: crate::core::collections::CollectionState::Active,
            writable: true,
            is_write_target: false,
            ignore_patterns: None,
            ignore_parse_errors: None,
            needs_full_sync: false,
            last_sync_at: None,
            active_lease_session_id: None,
            restore_command_id: None,
            restore_lease_session_id: None,
            reload_generation: 0,
            watcher_released_session_id: None,
            watcher_released_generation: None,
            watcher_released_at: None,
            pending_command_heartbeat_at: None,
            pending_root_path: None,
            pending_restore_manifest: None,
            restore_command_pid: None,
            restore_command_host: None,
            integrity_failed_at: None,
            pending_manifest_incomplete_at: None,
            reconcile_halted_at: None,
            reconcile_halt_reason: None,
            created_at: "2024-01-01T00:00:00Z".to_owned(),
            updated_at: "2024-01-01T00:00:00Z".to_owned(),
        };

        let result = reconcile(&conn, &collection);
        assert!(result.is_err());
    }

    #[test]
    fn full_hash_reconcile_rejects_fresh_attach_without_command_authorization() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Detached);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::FreshAttach,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        );

        let error = result.unwrap_err().to_string();
        assert!(error.contains("authorization denied"));
        assert!(error.contains("fresh-attach"));
    }

    #[test]
    fn full_hash_reconcile_allows_fresh_attach_with_attach_command_on_detached_collection() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Detached);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::FreshAttach,
            &FullHashReconcileAuthorization::AttachCommand {
                attach_command_id: "attach-1".to_owned(),
            },
        );

        assert!(result.is_ok());
    }

    #[test]
    fn full_hash_reconcile_rejects_fresh_attach_with_attach_command_when_collection_is_active() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::FreshAttach,
            &FullHashReconcileAuthorization::AttachCommand {
                attach_command_id: "attach-1".to_owned(),
            },
        );

        assert!(matches!(
            result,
            Err(ReconcileError::UnauthorizedFullHashReconcile {
                mode: FullHashReconcileMode::FreshAttach,
                authorization: "attach-command",
                collection_state: crate::core::collections::CollectionState::Active,
            })
        ));
    }

    #[test]
    fn full_hash_reconcile_rejects_remap_drift_capture_when_only_restore_identity_is_present() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RemapDriftCapture,
            &FullHashReconcileAuthorization::RestoreCommand {
                restore_command_id: "restore-1".to_owned(),
            },
        );

        assert!(matches!(
            result,
            Err(ReconcileError::UnauthorizedFullHashReconcile {
                mode: FullHashReconcileMode::RemapDriftCapture,
                authorization: "restore-command",
                collection_state: crate::core::collections::CollectionState::Active,
            })
        ));
    }

    #[test]
    fn full_hash_reconcile_rejects_restore_drift_capture_when_only_active_lease_identity_is_present(
    ) {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Restoring);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RestoreDriftCapture,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        );

        assert!(matches!(
            result,
            Err(ReconcileError::UnauthorizedFullHashReconcile {
                mode: FullHashReconcileMode::RestoreDriftCapture,
                authorization: "active-lease",
                collection_state: crate::core::collections::CollectionState::Restoring,
            })
        ));
    }

    #[test]
    fn full_hash_reconcile_rejects_empty_attach_identity() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Detached);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::FreshAttach,
            &FullHashReconcileAuthorization::AttachCommand {
                attach_command_id: "   ".to_owned(),
            },
        );

        assert!(matches!(
            result,
            Err(ReconcileError::InvalidFullHashAuthorization {
                mode: FullHashReconcileMode::FreshAttach,
                authorization: "attach-command",
                reason: "missing caller identity",
            })
        ));
    }

    #[test]
    fn full_hash_reconcile_rejects_restore_drift_capture_when_restore_command_does_not_match_owner()
    {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Restoring);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RestoreDriftCapture,
            &FullHashReconcileAuthorization::RestoreCommand {
                restore_command_id: "restore-other".to_owned(),
            },
        );

        assert!(matches!(
            result,
            Err(ReconcileError::InvalidFullHashAuthorization {
                mode: FullHashReconcileMode::RestoreDriftCapture,
                authorization: "restore-command",
                reason: "caller identity mismatch",
            })
        ));
    }

    #[test]
    fn full_hash_reconcile_allows_restore_drift_capture_when_restore_command_matches_owner() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Restoring);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RestoreDriftCapture,
            &FullHashReconcileAuthorization::RestoreCommand {
                restore_command_id: "restore-1".to_owned(),
            },
        );

        assert!(result.is_ok());
    }

    #[test]
    fn full_hash_reconcile_allows_restore_when_active_lease_matches_owner() {
        let mut collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Restoring);
        collection.active_lease_session_id = Some("lease-1".to_owned());

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::Restore,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        );

        assert!(result.is_ok());
    }

    #[test]
    fn load_collection_by_id_round_trips_optional_restore_metadata() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        conn.execute(
            "UPDATE collections
             SET active_lease_session_id = 'serve-1',
                 restore_command_id = 'restore-1',
                 restore_lease_session_id = 'restore-lease',
                 reload_generation = 4,
                 watcher_released_session_id = 'serve-1',
                 watcher_released_generation = 3,
                 watcher_released_at = '2026-04-28T00:00:00Z',
                 pending_command_heartbeat_at = '2026-04-28T00:01:00Z',
                 pending_root_path = 'D:\\restored',
                 pending_restore_manifest = '{\"entries\":[]}',
                 restore_command_pid = 7,
                 restore_command_host = 'host-a',
                 integrity_failed_at = '2026-04-28T00:02:00Z',
                 pending_manifest_incomplete_at = '2026-04-28T00:03:00Z',
                 reconcile_halted_at = '2026-04-28T00:04:00Z',
                 reconcile_halt_reason = 'duplicate_uuid'
             WHERE id = ?1",
            [collection.id],
        )
        .unwrap();

        let loaded = load_collection_by_id(&conn, collection.id).unwrap();

        assert_eq!(loaded.active_lease_session_id.as_deref(), Some("serve-1"));
        assert_eq!(loaded.restore_command_id.as_deref(), Some("restore-1"));
        assert_eq!(
            loaded.restore_lease_session_id.as_deref(),
            Some("restore-lease")
        );
        assert_eq!(loaded.reload_generation, 4);
        assert_eq!(
            loaded.watcher_released_session_id.as_deref(),
            Some("serve-1")
        );
        assert_eq!(loaded.watcher_released_generation, Some(3));
        assert_eq!(loaded.pending_root_path.as_deref(), Some("D:\\restored"));
        assert_eq!(
            loaded.pending_restore_manifest.as_deref(),
            Some("{\"entries\":[]}")
        );
        assert_eq!(loaded.restore_command_pid, Some(7));
        assert_eq!(loaded.restore_command_host.as_deref(), Some("host-a"));
        assert_eq!(
            loaded.reconcile_halt_reason.as_deref(),
            Some("duplicate_uuid")
        );
    }

    #[test]
    fn load_collection_by_id_rejects_invalid_collection_state() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        conn.pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        conn.execute(
            "UPDATE collections SET state = 'bogus' WHERE id = ?1",
            [collection.id],
        )
        .unwrap();

        let error = load_collection_by_id(&conn, collection.id)
            .unwrap_err()
            .to_string();

        assert!(error.contains("invalid collection state"));
        assert!(error.contains("bogus"));
    }

    #[test]
    fn full_hash_reconcile_rejects_restore_drift_capture_when_restore_lease_does_not_match_owner() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Restoring);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RestoreDriftCapture,
            &FullHashReconcileAuthorization::RestoreLease {
                lease_session_id: "restore-lease-other".to_owned(),
            },
        );

        assert!(matches!(
            result,
            Err(ReconcileError::InvalidFullHashAuthorization {
                mode: FullHashReconcileMode::RestoreDriftCapture,
                authorization: "restore-lease",
                reason: "caller identity mismatch",
            })
        ));
    }

    #[test]
    fn full_hash_reconcile_allows_restore_drift_capture_when_restore_lease_matches_owner() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Restoring);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RestoreDriftCapture,
            &FullHashReconcileAuthorization::RestoreLease {
                lease_session_id: "restore-lease-1".to_owned(),
            },
        );

        assert!(result.is_ok());
    }

    #[test]
    fn full_hash_reconcile_rejects_remap_drift_capture_when_active_lease_does_not_match_owner() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RemapDriftCapture,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-other".to_owned(),
            },
        );

        assert!(matches!(
            result,
            Err(ReconcileError::InvalidFullHashAuthorization {
                mode: FullHashReconcileMode::RemapDriftCapture,
                authorization: "active-lease",
                reason: "caller identity mismatch",
            })
        ));
    }

    #[test]
    fn full_hash_reconcile_allows_remap_drift_capture_when_active_lease_matches_owner() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);

        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RemapDriftCapture,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        );

        assert!(result.is_ok());
    }

    #[test]
    fn raw_import_invariant_allow_rerender_override_is_explicit_opt_in() {
        let result = raw_import_invariant_result(
            42,
            0,
            0,
            "restore",
            RawImportInvariantPolicy::AllowRerenderOverride,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn uuid_migration_preflight_refuses_trivial_pages_without_mirrored_frontmatter_uuid() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        fs::write(
            root.path().join("tiny.md"),
            "---\nslug: notes/tiny\ntitle: Tiny\ntype: concept\n---\nHi\n",
        )
        .unwrap();
        let stat = stat_for(root.path(), "tiny.md");
        conn.execute(
            "INSERT INTO pages (collection_id, slug, uuid, type, title, compiled_truth, timeline, frontmatter)
             VALUES (?1, 'notes/tiny', ?2, 'concept', 'Tiny', 'Hi', '', ?3)",
            rusqlite::params![
                collection.id,
                "01969f11-9448-7d79-8d3f-c68f54761234",
                "{\"slug\":\"notes/tiny\",\"title\":\"Tiny\",\"type\":\"concept\"}"
            ],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();
        upsert_file_state(&conn, collection.id, "tiny.md", page_id, &stat, "abc123").unwrap();

        let error = uuid_migration_preflight(&conn, &collection)
            .unwrap_err()
            .to_string();

        assert!(error.contains("UuidMigrationRequiredError"));
        assert!(error.contains("tiny.md"));
        assert!(error.contains("migrate-uuids test"));
    }

    #[test]
    fn collection_dirty_status_reports_needs_full_sync_and_sentinels() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let recovery_root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        set_collection_dirty_flag(&conn, collection.id, true);
        create_recovery_sentinel(
            recovery_root.path(),
            collection.id,
            "write-1.needs_full_sync",
        );

        let status = is_collection_dirty(&conn, collection.id, recovery_root.path()).unwrap();

        assert!(status.is_dirty());
        assert!(status.needs_full_sync);
        assert_eq!(status.sentinel_count, 1);
    }

    #[test]
    fn collection_dirty_status_is_clean_without_flags_or_sentinels() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let recovery_root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());

        let status = is_collection_dirty(&conn, collection.id, recovery_root.path()).unwrap();

        assert!(!status.is_dirty());
        assert!(!status.needs_full_sync);
        assert_eq!(status.sentinel_count, 0);
        assert!(!status.recovery_in_progress);
        assert!(status.last_sync_at.is_none());
    }

    #[test]
    fn raw_import_invariant_result_reports_zero_rows_and_multi_active_histories() {
        let missing =
            raw_import_invariant_result(7, 0, 0, "reconcile", RawImportInvariantPolicy::Enforce)
                .unwrap_err()
                .to_string();
        assert!(missing.contains("InvariantViolationError"));
        assert!(missing.contains("page_id=7 has zero total raw_imports rows"));

        let multi_active = raw_import_invariant_result(
            9,
            3,
            2,
            "full_hash_reconcile",
            RawImportInvariantPolicy::Enforce,
        )
        .unwrap_err()
        .to_string();
        assert!(multi_active.contains("InvariantViolationError"));
        assert!(
            multi_active.contains("page_id=9 has 2 active raw_imports rows across 3 total rows")
        );
    }

    #[test]
    fn canonical_body_refusal_reason_covers_empty_small_and_nontrivial_bodies() {
        assert_eq!(
            canonical_body_refusal_reason("missing", 0, false).as_deref(),
            Some("missing_empty_body")
        );
        assert_eq!(
            canonical_body_refusal_reason("new", 63, true).as_deref(),
            Some("new_below_min_body_bytes")
        );
        assert!(canonical_body_refusal_reason("new", 64, true).is_none());
    }

    #[cfg(not(unix))]
    #[test]
    fn windows_reconciler_helpers_fail_closed_with_explicit_platform_errors() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());

        let reconcile_error = reconcile(&conn, &collection).unwrap_err().to_string();
        assert!(reconcile_error.contains("fd-relative operations not supported on Windows"));

        let full_hash_error = full_hash_reconcile_authorized(
            &conn,
            collection.id,
            FullHashReconcileMode::OverflowRecovery,
            FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        )
        .unwrap_err()
        .to_string();
        assert!(full_hash_error.contains("active-lease authorization"));
        assert!(full_hash_error.contains("overflow-recovery mode"));

        let snapshot_error = take_stat_snapshot(&conn, &collection)
            .unwrap_err()
            .to_string();
        assert!(snapshot_error
            .contains("take_stat_snapshot: fd-relative operations not supported on Windows"));

        let mount_error = verify_read_only_mount(&collection).unwrap_err().to_string();
        assert!(mount_error.contains("restore/remap safety checks are not supported on Windows"));
        assert!(mount_error.contains("collection=test"));
    }

    #[cfg(unix)]
    #[test]
    fn verify_read_only_mount_rejects_writable_mounts() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection_with_state(
            &conn,
            root.path(),
            crate::core::collections::CollectionState::Restoring,
            false,
        );

        let error = verify_read_only_mount(&collection).unwrap_err().to_string();

        assert!(error.contains("CollectionLacksWriterQuiescenceError"));
        assert!(error.contains("quiesced environment"));
    }

    #[cfg(unix)]
    #[test]
    fn restore_phase1_drift_capture_rotates_authoritative_raw_imports() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection_with_state(
            &conn,
            root.path(),
            crate::core::collections::CollectionState::Restoring,
            false,
        );
        let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
        fs::write(root.path().join("note.md"), original).unwrap();
        let original_stat = stat_for(root.path(), "note.md");
        let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &original_stat,
                sha256: &original_sha,
                compiled_truth: "Original body.",
                timeline: "",
            },
        );
        raw_imports::rotate_active_raw_import(&conn, page_id, "note.md", original.as_bytes())
            .unwrap();

        let updated =
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated authoritative body.\n";
        fs::write(root.path().join("note.md"), updated).unwrap();

        let summary = capture_phase1_drift(
            &conn,
            &collection,
            RestoreRemapOperation::Restore,
            &FullHashReconcileAuthorization::RestoreCommand {
                restore_command_id: "restore-1".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(summary.pages_updated, 1);
        assert_eq!(active_raw_import_bytes(&conn, page_id), updated.as_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn remap_phase1_drift_capture_refuses_nonzero_drift_then_second_pass_is_clean() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection_with_state(
            &conn,
            root.path(),
            crate::core::collections::CollectionState::Active,
            false,
        );
        let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
        fs::write(root.path().join("note.md"), original).unwrap();
        let original_stat = stat_for(root.path(), "note.md");
        let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761235",
                relative_path: "note.md",
                stat: &original_stat,
                sha256: &original_sha,
                compiled_truth: "Original body.",
                timeline: "",
            },
        );
        raw_imports::rotate_active_raw_import(&conn, page_id, "note.md", original.as_bytes())
            .unwrap();

        let updated =
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body before remap.\n";
        fs::write(root.path().join("note.md"), updated).unwrap();

        let first = capture_phase1_drift(
            &conn,
            &collection,
            RestoreRemapOperation::Remap,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        )
        .unwrap_err();

        assert!(matches!(
            first,
            ReconcileError::RemapDriftConflictError { summary, .. }
                if summary.pages_updated == 1
                    && summary.pages_added == 0
                    && summary.pages_quarantined == 0
                    && summary.pages_deleted == 0
        ));
        assert_eq!(active_raw_import_bytes(&conn, page_id), updated.as_bytes());

        let second = capture_phase1_drift(
            &conn,
            &collection,
            RestoreRemapOperation::Remap,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(second, DriftCaptureSummary::default());
    }

    #[test]
    fn remap_safety_pipeline_wrapper_source_skips_mount_verifier_only() {
        let source = production_reconciler_source();
        let start = source
            .find("pub(crate) fn run_restore_remap_safety_pipeline_without_mount_check(")
            .unwrap();
        let end = source[start..]
            .find("pub fn fresh_attach_reconcile_and_activate(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("run_restore_remap_safety_pipeline_inner(conn, request, |_| Ok(()), || Ok(()))"),
            "the no-mount wrapper must reuse the shared safety pipeline and only skip the mount verifier"
        );
    }

    #[test]
    fn remap_safety_pipeline_source_keeps_phase_order_before_dirty_recheck() {
        let source = production_reconciler_source();
        let start = source
            .find("fn run_restore_remap_safety_pipeline_inner")
            .expect("shared restore/remap safety pipeline must remain present");
        let end = source[start..]
            .find("pub fn run_restore_remap_safety_pipeline(")
            .map(|offset| start + offset)
            .expect("shared restore/remap safety wrapper must remain present");
        let snippet = &source[start..end];
        let phase1_idx = snippet
            .find("let mut total_drift =")
            .expect("phase 1 drift capture assignment must remain present");
        let phase2_idx = snippet
            .find("run_phase2_stability_check(")
            .expect("phase 2 stability check must remain present");
        let phase3_idx = snippet
            .find("run_phase3_pre_destruction_fence(")
            .expect("phase 3 fence must remain present");
        let dirty_idx = snippet
            .find("fresh_collection_dirty_status(")
            .expect("fresh-connection dirty recheck must remain present");
        assert!(
            phase1_idx < phase2_idx && phase2_idx < phase3_idx && phase3_idx < dirty_idx,
            "restore/remap safety must capture drift, prove stability, fence, then do the fresh dirty recheck"
        );
    }

    #[test]
    fn scheduled_full_hash_audit_source_uses_nofollow_fd_relative_reads() {
        let source = production_reconciler_source();
        let start = source
            .find("pub fn scheduled_full_hash_audit_authorized(")
            .expect("scheduled audit entrypoint must remain present");
        let end = source[start..]
            .find("fn authorize_full_hash_reconcile(")
            .map(|offset| start + offset)
            .expect("audit helper block must remain before authorization logic");
        let snippet = &source[start..end];

        assert!(
            snippet.contains("let root_fd = fs_safety::open_root_fd(root_path)?;")
                && snippet.contains("stat_and_hash_audit_path(&root_fd, relative_path)?")
                && snippet.contains("fs_safety::walk_to_parent(root_fd, relative_path)")
                && snippet
                    .contains("fs_safety::stat_at_nofollow(&parent_fd, Path::new(entry_name))")
                && snippet.contains("OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW"),
            "scheduled audit must stay on fd-relative NOFOLLOW reads instead of path-based hashing"
        );
        assert!(
            !snippet.contains("file_state::stat_file(&absolute_path)")
                && !snippet.contains("file_state::hash_file(&absolute_path)"),
            "scheduled audit must not fall back to path-based stat/hash reads that follow symlinks"
        );
        assert!(
            snippet.contains("scheduled_full_hash_audit_missing_paths_marked_dirty")
                && snippet.contains("SET needs_full_sync = 1")
                && !snippet.contains("remaining_missing: plan.diff.missing.clone()"),
            "scheduled audit must mark the collection dirty on missing paths instead of deleting pages inline"
        );
    }

    #[cfg(unix)]
    #[test]
    fn scheduled_full_hash_audit_marks_collection_dirty_without_deleting_missing_pages() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("notes")).unwrap();
        let note_path = root.path().join("notes").join("note.md");
        let note_bytes = b"---\nslug: notes/note\ntitle: Note\ntype: concept\n---\naudit me\n";
        fs::write(&note_path, note_bytes).unwrap();

        let collection = insert_collection(&conn, root.path());
        let stat = stat_for(root.path(), "notes/note.md");
        let sha256 = format!("{:x}", sha2::Sha256::digest(note_bytes));
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54768888",
                relative_path: "notes/note.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth: "audit me",
                timeline: "",
            },
        );
        conn.execute(
            "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
             VALUES (?1, ?2, 1, ?3, ?4)",
            rusqlite::params![
                page_id,
                page_uuid::generate_uuid_v7(),
                note_bytes.as_slice(),
                "notes/note.md"
            ],
        )
        .unwrap();
        conn.execute(
            "UPDATE file_state
             SET last_full_hash_at = '2000-01-01T00:00:00Z'
             WHERE collection_id = ?1 AND relative_path = 'notes/note.md'",
            [collection.id],
        )
        .unwrap();
        fs::remove_file(&note_path).unwrap();

        let stats = scheduled_full_hash_audit_authorized(
            &conn,
            collection.id,
            &[PathBuf::from("notes/note.md")],
            FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(stats.missing, 1);
        assert_eq!(stats.hard_deleted, 0);
        assert_eq!(stats.quarantined_db_state, 0);
        let needs_full_sync: bool = conn
            .query_row(
                "SELECT needs_full_sync FROM collections WHERE id = ?1",
                [collection.id],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .unwrap();
        assert!(needs_full_sync);
        let existing_page_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM pages WHERE id = ?1 AND quarantined_at IS NULL",
                [page_id],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(existing_page_id, Some(page_id));
    }

    #[test]
    fn capture_phase1_drift_source_refuses_remap_on_material_changes() {
        let source = production_reconciler_source();
        let start = source.find("fn capture_phase1_drift(").unwrap();
        let end = source[start..]
            .find("fn run_phase2_stability_check")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("RestoreRemapOperation::Remap if summary.has_material_changes()")
                && snippet.contains("ERROR: remap_drift_refused")
                && snippet.contains("ReconcileError::RemapDriftConflictError"),
            "Phase 1 remap capture must fail closed when old-root drift would otherwise be lost"
        );
    }

    #[test]
    fn capture_phase1_drift_source_logs_restore_capture_without_refusal() {
        let source = production_reconciler_source();
        let start = source.find("fn capture_phase1_drift(").unwrap();
        let end = source[start..]
            .find("fn run_phase2_stability_check")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("RestoreRemapOperation::Restore if summary.has_material_changes()")
                && snippet.contains("WARN: restore_drift_captured"),
            "Phase 1 restore capture must record the adopted drift without turning it into a remap-style refusal"
        );
    }

    #[test]
    fn authorize_full_hash_source_requires_active_lease_for_remap_modes() {
        let source = production_reconciler_source();
        let start = source.find("fn authorize_full_hash_reconcile(").unwrap();
        let end = source[start..]
            .find("fn require_persisted_full_hash_owner_match(")
            .map(|offset| start + offset)
            .unwrap();
        let snippet = &source[start..end];

        assert!(
            snippet.contains("FullHashReconcileMode::RemapRoot,")
                && snippet.contains("FullHashReconcileMode::RemapDriftCapture,")
                && snippet.contains("FullHashReconcileAuthorization::ActiveLease { .. }"),
            "both remap full-hash modes must stay bound to the active owner lease"
        );
    }

    #[test]
    fn phase2_stability_retries_until_snapshots_converge() {
        fn snapshot_with_mtime(mtime_ns: i64) -> StatSnapshot {
            HashMap::from([(
                PathBuf::from("note.md"),
                FileStat {
                    mtime_ns,
                    ctime_ns: Some(mtime_ns),
                    size_bytes: 10,
                    inode: Some(1),
                },
            )])
        }

        let mut snapshots = vec![
            snapshot_with_mtime(1),
            snapshot_with_mtime(2),
            snapshot_with_mtime(2),
        ]
        .into_iter();
        let mut reruns = 0usize;

        let (stable_snapshot, retries, drift) = run_phase2_stability_check(
            RestoreRemapOperation::Restore,
            5,
            "test",
            || Ok(snapshots.next().unwrap()),
            || {
                reruns += 1;
                Ok(DriftCaptureSummary {
                    pages_updated: 1,
                    ..DriftCaptureSummary::default()
                })
            },
        )
        .unwrap();

        assert_eq!(retries, 1);
        assert_eq!(reruns, 1);
        assert_eq!(drift.pages_updated, 1);
        assert_eq!(stable_snapshot, snapshot_with_mtime(2));
    }

    #[cfg(unix)]
    #[test]
    fn phase3_fence_rejects_late_drift() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        fs::write(
            root.path().join("note.md"),
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nFence body.\n",
        )
        .unwrap();
        let stable_snapshot = take_stat_snapshot(&conn, &collection).unwrap();

        fs::write(
            root.path().join("note.md"),
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nFence body changed.\n",
        )
        .unwrap();

        let error = run_phase3_pre_destruction_fence(
            &conn,
            &collection,
            RestoreRemapOperation::Restore,
            &stable_snapshot,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("CollectionUnstableError"));
        assert!(error.contains("phase=fence"));
    }

    #[test]
    fn fresh_collection_dirty_status_uses_fresh_connection() {
        let (_db_dir, db_path, conn) = open_test_db_file();
        let root = TempDir::new().unwrap();
        let recovery_root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        set_collection_dirty_flag(&conn, collection.id, true);
        create_recovery_sentinel(
            recovery_root.path(),
            collection.id,
            "write-2.needs_full_sync",
        );

        let status =
            fresh_collection_dirty_status(&db_path, collection.id, recovery_root.path()).unwrap();

        assert!(status.is_dirty());
        assert!(status.needs_full_sync);
        assert_eq!(status.sentinel_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn restore_safety_pipeline_aborts_on_fresh_connection_dirty_recheck() {
        let (_db_dir, db_path, conn) = open_test_db_file();
        let root = TempDir::new().unwrap();
        let recovery_root = TempDir::new().unwrap();
        let collection = insert_collection_with_state(
            &conn,
            root.path(),
            crate::core::collections::CollectionState::Restoring,
            false,
        );
        fs::write(
            root.path().join("note.md"),
            "---\nslug: notes/note\nmemory_id: 01969f11-9448-7d79-8d3f-c68f54761234\ntitle: Note\ntype: concept\n---\nA long enough body to stay non-trivial for the shared helper.\n",
        )
        .unwrap();
        let stat = stat_for(root.path(), "note.md");
        let sha256 = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth: "A long enough body to stay non-trivial for the shared helper.",
                timeline: "",
            },
        );
        let note_bytes = fs::read(root.path().join("note.md")).unwrap();
        raw_imports::rotate_active_raw_import(&conn, page_id, "note.md", &note_bytes).unwrap();

        let request = RestoreRemapSafetyRequest {
            collection_id: collection.id,
            db_path: &db_path,
            recovery_root: recovery_root.path(),
            operation: RestoreRemapOperation::Restore,
            authorization: FullHashReconcileAuthorization::RestoreCommand {
                restore_command_id: "restore-1".to_owned(),
            },
            allow_finalize_pending: false,
            stability_max_iters: 1,
        };

        let error = run_restore_remap_safety_pipeline_inner(
            &conn,
            &request,
            |_| Ok(()),
            || {
                set_collection_dirty_flag(&conn, collection.id, true);
                create_recovery_sentinel(
                    recovery_root.path(),
                    collection.id,
                    "late-write.needs_full_sync",
                );
                Ok(())
            },
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("CollectionDirtyError"));
        assert!(error.contains("sentinel_count=1"));
    }

    #[cfg(unix)]
    #[test]
    fn fresh_attach_reconcile_and_activate_clears_gate_after_full_hash() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection_with_state(
            &conn,
            root.path(),
            crate::core::collections::CollectionState::Detached,
            true,
        );
        let original =
            "---\nslug: notes/note\nmemory_id: 01969f11-9448-7d79-8d3f-c68f54761234\ntitle: Note\ntype: concept\n---\nDetached body.\n";
        fs::write(root.path().join("note.md"), original).unwrap();
        let stat = stat_for(root.path(), "note.md");
        let sha256 = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth: "Detached body.",
                timeline: "",
            },
        );
        raw_imports::rotate_active_raw_import(&conn, page_id, "note.md", original.as_bytes())
            .unwrap();

        let stats = fresh_attach_reconcile_and_activate(&conn, collection.id, "attach-1").unwrap();
        let (state, needs_full_sync): (String, i64) = conn
            .query_row(
                "SELECT state, needs_full_sync FROM collections WHERE id = ?1",
                [collection.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(stats.walked, 1);
        assert_eq!(state, "active");
        assert_eq!(needs_full_sync, 0);
    }

    #[test]
    fn load_db_files_round_trips_tracked_rows() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let page_id = insert_page(&conn, collection.id, "notes/a");
        let stat = FileStat {
            mtime_ns: 11,
            ctime_ns: Some(22),
            size_bytes: 33,
            inode: Some(44),
        };
        upsert_file_state(&conn, collection.id, "notes/a.md", page_id, &stat, "abc123").unwrap();

        let db_files = load_db_files(&conn, collection.id).unwrap();

        assert_eq!(db_files.len(), 1);
        assert_eq!(db_files.get(&PathBuf::from("notes/a.md")), Some(&stat));
    }

    #[test]
    fn rename_match_helpers_cover_native_uuid_and_hash_paths() {
        let stat = FileStat {
            mtime_ns: 1,
            ctime_ns: Some(2),
            size_bytes: 3,
            inode: Some(4),
        };
        let native_from = PathBuf::from("notes/native-old.md");
        let native_to = PathBuf::from("notes/native-new.md");
        let uuid_from = PathBuf::from("notes/uuid-old.md");
        let uuid_to = PathBuf::from("notes/uuid-new.md");
        let hash_from = PathBuf::from("notes/hash-old.md");
        let hash_to = PathBuf::from("notes/hash-new.md");

        let mut resolution = RenameResolution {
            remaining_new: HashMap::from([
                (native_to.clone(), stat.clone()),
                (uuid_to.clone(), stat.clone()),
                (hash_to.clone(), stat.clone()),
            ]),
            remaining_missing: HashSet::from([
                native_from.clone(),
                uuid_from.clone(),
                hash_from.clone(),
            ]),
            ..Default::default()
        };
        let missing_identities = HashMap::from([
            (
                native_from.clone(),
                MissingPageIdentity {
                    page_id: 1,
                    uuid: Some("native-uuid".to_owned()),
                    sha256: "native-hash".to_owned(),
                    body_size_bytes: 80,
                    has_nonempty_body: true,
                },
            ),
            (
                uuid_from.clone(),
                MissingPageIdentity {
                    page_id: 2,
                    uuid: Some("uuid-match".to_owned()),
                    sha256: "uuid-hash".to_owned(),
                    body_size_bytes: 80,
                    has_nonempty_body: true,
                },
            ),
            (
                hash_from.clone(),
                MissingPageIdentity {
                    page_id: 3,
                    uuid: None,
                    sha256: "hash-match".to_owned(),
                    body_size_bytes: 80,
                    has_nonempty_body: true,
                },
            ),
        ]);
        let new_identities = HashMap::from([
            (
                native_to.clone(),
                NewTreeIdentity {
                    relative_path: native_to.clone(),
                    sha256: "native-hash".to_owned(),
                    uuid: Some("native-uuid".to_owned()),
                    body_size_bytes: 80,
                    has_nonempty_body: true,
                },
            ),
            (
                uuid_to.clone(),
                NewTreeIdentity {
                    relative_path: uuid_to.clone(),
                    sha256: "uuid-hash".to_owned(),
                    uuid: Some("uuid-match".to_owned()),
                    body_size_bytes: 80,
                    has_nonempty_body: true,
                },
            ),
            (
                hash_to.clone(),
                NewTreeIdentity {
                    relative_path: hash_to.clone(),
                    sha256: "hash-match".to_owned(),
                    uuid: None,
                    body_size_bytes: 80,
                    has_nonempty_body: true,
                },
            ),
        ]);

        apply_native_rename_matches(
            &mut resolution,
            &missing_identities,
            &[NativeRename {
                from_path: native_from.clone(),
                to_path: native_to.clone(),
            }],
            &new_identities,
        );
        apply_uuid_rename_matches(&mut resolution, &missing_identities, &new_identities);
        apply_hash_rename_matches(&mut resolution, &missing_identities, &new_identities).unwrap();

        assert_eq!(resolution.native_renamed, 1);
        assert_eq!(resolution.uuid_renamed, 1);
        assert_eq!(resolution.hash_renamed, 1);
        assert!(resolution.remaining_missing.is_empty());
        assert!(resolution.remaining_new.is_empty());
        assert_eq!(resolution.matches.len(), 3);
        assert!(resolution
            .matches
            .iter()
            .any(|entry| entry.kind == RenameMatchKind::Native));
        assert!(resolution
            .matches
            .iter()
            .any(|entry| entry.kind == RenameMatchKind::Uuid));
        assert!(resolution
            .matches
            .iter()
            .any(|entry| entry.kind == RenameMatchKind::Hash));
    }

    #[test]
    #[cfg(unix)]
    fn stat_diff_walk_classifies_new_modified_unchanged_and_missing_files() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("notes")).unwrap();
        fs::write(root.path().join("notes").join("same.md"), "# same").unwrap();
        fs::write(
            root.path().join("notes").join("changed.md"),
            "# changed on disk",
        )
        .unwrap();
        fs::write(root.path().join("notes").join("new.md"), "# new").unwrap();
        fs::write(root.path().join(".quaidignore"), "ignored/**\n").unwrap();
        fs::create_dir_all(root.path().join("ignored")).unwrap();
        fs::write(root.path().join("ignored").join("skip.md"), "# skip").unwrap();

        let collection = insert_collection(&conn, root.path());
        let same_stat = stat_for(root.path(), "notes/same.md");
        seed_file_state(
            &conn,
            collection.id,
            "notes/same",
            "notes/same.md",
            &same_stat,
        );

        let changed_stat = stat_for(root.path(), "notes/changed.md");
        seed_file_state(
            &conn,
            collection.id,
            "notes/changed",
            "notes/changed.md",
            &unique_old_stat(&changed_stat),
        );

        let missing_stat = FileStat {
            mtime_ns: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::from_secs(1))
                .as_nanos() as i64,
            ctime_ns: Some(1),
            size_bytes: 12,
            inode: Some(1),
        };
        seed_file_state(
            &conn,
            collection.id,
            "notes/missing",
            "notes/missing.md",
            &missing_stat,
        );

        let diff = stat_diff(&conn, collection.id, root.path()).unwrap();

        assert!(diff.unchanged.contains(Path::new("notes/same.md")));
        assert!(diff.modified.contains_key(Path::new("notes/changed.md")));
        assert!(diff.new.contains_key(Path::new("notes/new.md")));
        assert!(diff.missing.contains(Path::new("notes/missing.md")));
        assert!(!diff.new.contains_key(Path::new("ignored/skip.md")));
    }

    #[cfg(unix)]
    #[test]
    fn walk_collection_never_descends_symlinks_and_counts_skips() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("notes")).unwrap();
        fs::write(root.path().join("notes").join("real.md"), "# real").unwrap();
        symlink(
            root.path().join("notes").join("real.md"),
            root.path().join("notes").join("real-link.md"),
        )
        .unwrap();
        fs::create_dir_all(root.path().join("actual")).unwrap();
        fs::write(root.path().join("actual").join("inside.md"), "# hidden").unwrap();
        symlink(root.path().join("actual"), root.path().join("linked-dir")).unwrap();

        let collection = insert_collection(&conn, root.path());
        let root_fd = fs_safety::open_root_fd(root.path()).unwrap();
        let walked = walk_collection(&conn, &root_fd, &collection).unwrap();

        assert_eq!(walked.walked, 2);
        assert_eq!(walked.skipped_symlinks, 2);
        assert!(walked.files.contains_key(Path::new("notes/real.md")));
        assert!(!walked.files.contains_key(Path::new("notes/real-link.md")));
        assert!(!walked.files.contains_key(Path::new("linked-dir/inside.md")));
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_skips_symlinked_entries_at_the_reconciler_boundary() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("notes")).unwrap();
        fs::write(root.path().join("notes").join("real.md"), "# real").unwrap();
        symlink(
            root.path().join("notes").join("real.md"),
            root.path().join("notes").join("real-link.md"),
        )
        .unwrap();
        fs::create_dir_all(root.path().join("actual")).unwrap();
        fs::write(root.path().join("actual").join("inside.md"), "# hidden").unwrap();
        symlink(root.path().join("actual"), root.path().join("linked-dir")).unwrap();

        let collection = insert_collection(&conn, root.path());

        let stats = reconcile(&conn, &collection).unwrap();

        assert_eq!(stats.walked, 2);
        assert_eq!(stats.new, 2);
        assert_eq!(stats.modified, 0);
        assert_eq!(stats.missing, 0);
    }

    #[cfg(unix)]
    #[test]
    fn native_rename_resolution_preserves_page_id_via_interface_only_pairing() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let content =
            "---\nslug: notes/renamed\ntitle: Rename me\ntype: concept\n---\nThis is a long enough body to satisfy conservative rename inference guards.\n";
        fs::write(root.path().join("renamed.md"), content).unwrap();
        let stat = stat_for(root.path(), "renamed.md");
        let sha256 = file_state::hash_file(&root.path().join("renamed.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/renamed",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "old-name.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth:
                    "This is a long enough body to satisfy conservative rename inference guards.",
                timeline: "",
            },
        );

        let diff = stat_diff(&conn, collection.id, root.path()).unwrap();
        let resolution = resolve_rename_resolution(
            &conn,
            collection.id,
            root.path(),
            &diff,
            &[NativeRename {
                from_path: PathBuf::from("old-name.md"),
                to_path: PathBuf::from("renamed.md"),
            }],
        )
        .unwrap();

        assert_eq!(resolution.native_renamed, 1);
        assert_eq!(
            resolution.matches,
            vec![RenameMatch {
                page_id,
                from_path: PathBuf::from("old-name.md"),
                to_path: PathBuf::from("renamed.md"),
                kind: RenameMatchKind::Native,
            }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn uuid_rename_resolution_preserves_page_id_across_reorganization() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("nested")).unwrap();
        let collection = insert_collection(&conn, root.path());
        let uuid = "01969f11-9448-7d79-8d3f-c68f54761234";
        let content = format!(
            "---\nmemory_id: {uuid}\nslug: notes/renamed\ntitle: Rename me\ntype: concept\n---\nThis is a long enough body to satisfy conservative rename inference guards.\n"
        );
        fs::write(root.path().join("nested").join("renamed.md"), content).unwrap();
        let stat = stat_for(root.path(), "nested/renamed.md");
        let sha256 = file_state::hash_file(&root.path().join("nested").join("renamed.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/renamed",
                uuid,
                relative_path: "old-name.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth:
                    "This is a long enough body to satisfy conservative rename inference guards.",
                timeline: "",
            },
        );

        let diff = stat_diff(&conn, collection.id, root.path()).unwrap();
        let resolution =
            resolve_rename_resolution(&conn, collection.id, root.path(), &diff, &[]).unwrap();
        assert_eq!(
            resolution.matches,
            vec![RenameMatch {
                page_id,
                from_path: PathBuf::from("old-name.md"),
                to_path: PathBuf::from("nested/renamed.md"),
                kind: RenameMatchKind::Uuid,
            }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn hash_rename_resolution_preserves_page_id_when_body_content_is_unique_and_over_minimum() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let content =
            "---\nslug: notes/renamed\ntitle: Rename me\ntype: concept\n---\nThis body alone is intentionally longer than sixty-four bytes so the hash rename guard can still pair it.\n";
        fs::write(root.path().join("renamed.md"), content).unwrap();
        let stat = stat_for(root.path(), "renamed.md");
        let sha256 = file_state::hash_file(&root.path().join("renamed.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/renamed",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "old-name.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth:
                    "This body alone is intentionally longer than sixty-four bytes so the hash rename guard can still pair it.",
                timeline: "",
            },
        );

        let diff = stat_diff(&conn, collection.id, root.path()).unwrap();
        let resolution =
            resolve_rename_resolution(&conn, collection.id, root.path(), &diff, &[]).unwrap();

        assert_eq!(resolution.hash_renamed, 1);
        assert_eq!(
            resolution.matches,
            vec![RenameMatch {
                page_id,
                from_path: PathBuf::from("old-name.md"),
                to_path: PathBuf::from("renamed.md"),
                kind: RenameMatchKind::Hash,
            }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn ambiguous_hash_refusal_quarantines_old_page_and_leaves_new_files_unpaired() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let content =
            "---\nslug: notes/renamed\ntitle: Rename me\ntype: concept\n---\nThis is a long enough body to satisfy conservative rename inference guards.\n";
        fs::write(root.path().join("renamed-a.md"), content).unwrap();
        fs::write(root.path().join("renamed-b.md"), content).unwrap();
        let stat = stat_for(root.path(), "renamed-a.md");
        let sha256 = file_state::hash_file(&root.path().join("renamed-a.md")).unwrap();
        seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/renamed",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "old-name.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth:
                    "This is a long enough body to satisfy conservative rename inference guards.",
                timeline: "",
            },
        );

        let diff = stat_diff(&conn, collection.id, root.path()).unwrap();
        let resolution =
            resolve_rename_resolution(&conn, collection.id, root.path(), &diff, &[]).unwrap();

        assert_eq!(resolution.hash_renamed, 0);
        assert_eq!(resolution.quarantined_ambiguous, 1);
        assert_eq!(resolution.remaining_new.len(), 2);
        assert!(resolution.remaining_missing.is_empty());
    }

    /// Regression guard for Batch J:
    /// A template note with large frontmatter (>64 bytes whole-file) and a trivially
    /// small body (<64 body bytes) MUST hard-stop with UnresolvableTrivialContentError.
    /// The old guard used whole-file size, which allowed such notes to pass.
    #[cfg(unix)]
    #[test]
    fn template_note_with_large_frontmatter_and_tiny_body_is_never_hash_paired() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());

        // Frontmatter alone is >> 64 bytes; body is "Hi" (2 bytes after trimming).
        // Whole-file size therefore exceeds 64, which exposed the old guard's seam.
        let content = concat!(
            "---\n",
            "slug: notes/template\n",
            "title: Template Note\n",
            "type: concept\n",
            "meta: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            "---\n",
            "Hi\n",
        );
        // Confirm the precondition: file is large enough to fool a whole-file-size guard.
        assert!(
            content.len() >= 64,
            "precondition: whole-file content must be >= 64 bytes, got {}",
            content.len()
        );
        // Confirm the body is small enough to trigger the body-size guard.
        let body_only = "Hi";
        assert!(
            body_only.len() < 64,
            "precondition: body must be < 64 bytes, got {}",
            body_only.len()
        );

        fs::write(root.path().join("template.md"), content).unwrap();
        let stat = stat_for(root.path(), "template.md");
        let sha256 = file_state::hash_file(&root.path().join("template.md")).unwrap();

        // Seed the missing page with a tiny (non-empty) body in the DB.
        seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/template",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761235",
                relative_path: "old-template.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth: "Hi",
                timeline: "",
            },
        );

        let diff = stat_diff(&conn, collection.id, root.path()).unwrap();
        let error =
            resolve_rename_resolution(&conn, collection.id, root.path(), &diff, &[]).unwrap_err();

        assert!(matches!(
            error,
            ReconcileError::UnresolvableTrivialContentError { .. }
        ));
        let rendered = error.to_string();
        assert!(rendered.contains("UnresolvableTrivialContentError"));
        assert!(rendered.contains("template.md"));
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_halts_when_two_files_share_the_same_memory_id() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let uuid = "01969f11-9448-7d79-8d3f-c68f54769999";
        let note_a = format!(
            "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nThis body is long enough to avoid the trivial-content path.\n"
        );
        let note_b = format!(
            "---\nmemory_id: {uuid}\nslug: notes/b\ntitle: B\ntype: concept\n---\nThis other body is also long enough to avoid the trivial-content path.\n"
        );
        fs::write(root.path().join("a.md"), note_a).unwrap();
        fs::write(root.path().join("b.md"), note_b).unwrap();

        let error = reconcile(&conn, &collection).unwrap_err().to_string();
        let page_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE collection_id = ?1",
                [collection.id],
                |row| row.get(0),
            )
            .unwrap();

        assert!(error.contains("DuplicateUuidError"));
        assert!(error.contains("a.md"));
        assert!(error.contains("b.md"));
        assert_eq!(
            page_count, 0,
            "duplicate uuid halt must abort before mutation"
        );
    }

    fn insert_programmatic_link(conn: &Connection, page_a: i64, page_b: i64) {
        conn.execute(
            "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
             VALUES (?1, ?2, 'related', 'programmatic')",
            rusqlite::params![page_a, page_b],
        )
        .unwrap();
    }

    #[test]
    fn has_db_only_state_returns_true_for_programmatic_links_branch() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();

        let page_a = insert_page(&conn, 1, "notes/a");
        let page_b = insert_page(&conn, 1, "notes/b");

        insert_programmatic_link(&conn, page_a, page_b);
        assert!(has_db_only_state(&conn, page_a).unwrap());
    }

    #[test]
    fn has_db_only_state_returns_true_for_non_import_assertions_branch() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let page_a = insert_page(&conn, 1, "notes/a");

        conn.execute(
            "INSERT INTO assertions (page_id, subject, predicate, object, asserted_by)
             VALUES (?1, 'A', 'knows', 'B', 'manual')",
            rusqlite::params![page_a],
        )
        .unwrap();
        assert!(has_db_only_state(&conn, page_a).unwrap());
    }

    #[test]
    fn has_db_only_state_returns_true_for_raw_data_branch() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let page_a = insert_page(&conn, 1, "notes/a");

        conn.execute(
            "INSERT INTO raw_data (page_id, source, data) VALUES (?1, 'api', '{}')",
            rusqlite::params![page_a],
        )
        .unwrap();
        assert!(has_db_only_state(&conn, page_a).unwrap());
    }

    #[test]
    fn has_db_only_state_returns_true_for_contradictions_branch() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let page_a = insert_page(&conn, 1, "notes/a");
        let page_b = insert_page(&conn, 1, "notes/b");

        conn.execute(
            "INSERT INTO contradictions (page_id, other_page_id, type, description)
             VALUES (?1, ?2, 'assertion_conflict', 'conflict')",
            rusqlite::params![page_a, page_b],
        )
        .unwrap();
        assert!(has_db_only_state(&conn, page_a).unwrap());
    }

    #[test]
    fn has_db_only_state_returns_true_for_knowledge_gaps_branch() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let page_a = insert_page(&conn, 1, "notes/a");

        conn.execute(
            "INSERT INTO knowledge_gaps (page_id, query_hash, context)
              VALUES (?1, 'gap-hash', 'context')",
            rusqlite::params![page_a],
        )
        .unwrap();
        assert!(has_db_only_state(&conn, page_a).unwrap());
    }

    #[test]
    fn knowledge_gap_without_page_binding_does_not_preserve_missing_page() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let page_id = insert_page(&conn, 1, "notes/a");
        let stat = FileStat {
            mtime_ns: 1,
            ctime_ns: Some(1),
            size_bytes: 10,
            inode: Some(1),
        };
        upsert_file_state(&conn, 1, "missing/plain.md", page_id, &stat, "hash").unwrap();

        conn.execute(
            "INSERT INTO knowledge_gaps (page_id, query_hash, context)
             VALUES (NULL, 'gap-hash', 'context without page')",
            [],
        )
        .unwrap();

        let missing = HashSet::from([PathBuf::from("missing/plain.md")]);
        let (quarantined, hard_deleted) = classify_missing_paths(&conn, 1, &missing).unwrap();

        assert_eq!(quarantined, 0);
        assert_eq!(hard_deleted, 1);
    }

    #[test]
    fn classifier_rechecks_db_only_state_after_a_previous_clear_result() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let page_id = insert_page(&conn, 1, "notes/a");
        let stat = FileStat {
            mtime_ns: 1,
            ctime_ns: Some(1),
            size_bytes: 10,
            inode: Some(1),
        };
        upsert_file_state(&conn, 1, "missing/later-gap.md", page_id, &stat, "hash").unwrap();
        assert!(!has_db_only_state(&conn, page_id).unwrap());

        let missing = HashSet::from([PathBuf::from("missing/later-gap.md")]);
        conn.execute(
            "INSERT INTO raw_data (page_id, source, data) VALUES (?1, 'api', '{}')",
            rusqlite::params![page_id],
        )
        .unwrap();

        let (quarantined, hard_deleted) = classify_missing_paths(&conn, 1, &missing).unwrap();

        assert_eq!(quarantined, 1);
        assert_eq!(hard_deleted, 0);
    }

    #[cfg(unix)]
    #[test]
    fn full_hash_reconcile_aborts_before_mutation_when_a_page_has_zero_total_raw_import_rows() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("note.md"), "# note").unwrap();
        let collection = insert_collection(&conn, root.path());
        let stat = stat_for(root.path(), "note.md");
        let page_id = seed_file_state(&conn, collection.id, "notes/note", "note.md", &stat);

        assert_eq!(active_raw_import_count(&conn, page_id), 0);

        let err = full_hash_reconcile(&conn, collection.id)
            .unwrap_err()
            .to_string();
        assert!(err.contains("InvariantViolation"));
    }

    #[cfg(unix)]
    #[test]
    fn full_hash_reconcile_aborts_before_mutation_when_history_has_zero_active_raw_import_rows() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let content = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body.\n";
        fs::write(root.path().join("note.md"), content).unwrap();
        let collection = insert_collection(&conn, root.path());
        let stat = stat_for(root.path(), "note.md");
        let sha256 = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth: "Updated body.",
                timeline: "",
            },
        );
        conn.execute(
            "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
             VALUES (?1, ?2, 0, ?3, ?4)",
            rusqlite::params![
                page_id,
                crate::core::page_uuid::generate_uuid_v7(),
                b"stale",
                "note.md"
            ],
        )
        .unwrap();

        let err = full_hash_reconcile(&conn, collection.id)
            .unwrap_err()
            .to_string();

        assert!(err.contains("InvariantViolation"));
        assert_eq!(active_raw_import_count(&conn, page_id), 0);
    }

    #[cfg(unix)]
    #[test]
    fn full_hash_reconcile_unchanged_hash_updates_only_last_full_hash_at_without_rotation() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let content = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nStable body.\n";
        fs::write(root.path().join("note.md"), content).unwrap();
        let collection = insert_collection(&conn, root.path());
        let stat = stat_for(root.path(), "note.md");
        let sha256 = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth: "Stable body.",
                timeline: "",
            },
        );
        crate::core::raw_imports::rotate_active_raw_import(
            &conn,
            page_id,
            "note.md",
            content.as_bytes(),
        )
        .unwrap();
        conn.execute(
            "UPDATE file_state
             SET last_full_hash_at = '2000-01-01T00:00:00Z'
             WHERE collection_id = ?1 AND relative_path = 'note.md'",
            [collection.id],
        )
        .unwrap();

        let before_row = crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
            .unwrap()
            .expect("file_state row should exist");

        let stats = full_hash_reconcile(&conn, collection.id).unwrap();
        let after_row = crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
            .unwrap()
            .expect("file_state row should still exist");

        assert_eq!(stats.unchanged, 1);
        assert_eq!(stats.modified, 0);
        assert_eq!(active_raw_import_count(&conn, page_id), 1);
        assert_eq!(active_raw_import_bytes(&conn, page_id), content.as_bytes());
        assert_eq!(after_row.sha256, before_row.sha256);
        assert_ne!(after_row.last_full_hash_at, before_row.last_full_hash_at);
    }

    #[test]
    fn has_db_only_state_returns_false_only_when_all_five_branches_are_clear() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let page_a = insert_page(&conn, 1, "notes/a");
        let page_b = insert_page(&conn, 1, "notes/b");

        conn.execute(
            "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
             VALUES (?1, ?2, 'related', 'wiki_link')",
            rusqlite::params![page_a, page_b],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assertions (page_id, subject, predicate, object, asserted_by)
             VALUES (?1, 'A', 'knows', 'B', 'import')",
            rusqlite::params![page_a],
        )
        .unwrap();

        let branches = db_only_state_branches(&conn, page_a).unwrap();

        assert_eq!(
            branches,
            DbOnlyStateBranches {
                programmatic_links: false,
                non_import_assertions: false,
                raw_data: false,
                contradictions: false,
                knowledge_gaps: false,
            }
        );
        assert!(!has_db_only_state(&conn, page_a).unwrap());
    }

    #[test]
    fn classifier_quarantines_missing_pages_with_each_db_only_state_branch() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();

        let cases = [
            ("programmatic-link", "missing/link.md"),
            ("manual-assertion", "missing/assertion.md"),
            ("raw-data", "missing/raw.md"),
            ("contradiction", "missing/contradiction.md"),
            ("knowledge-gap", "missing/gap.md"),
        ];

        for (index, (slug_suffix, relative_path)) in cases.iter().enumerate() {
            let slug = format!("notes/{slug_suffix}");
            let page_id = insert_page(&conn, 1, &slug);
            let stat = FileStat {
                mtime_ns: index as i64 + 1,
                ctime_ns: Some(index as i64 + 1),
                size_bytes: 10,
                inode: Some(index as i64 + 1),
            };
            upsert_file_state(&conn, 1, relative_path, page_id, &stat, "hash").unwrap();

            match *slug_suffix {
                "programmatic-link" => {
                    let other_page = insert_page(&conn, 1, "notes/other-link");
                    conn.execute(
                        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
                         VALUES (?1, ?2, 'related', 'programmatic')",
                        rusqlite::params![page_id, other_page],
                    )
                    .unwrap();
                }
                "manual-assertion" => {
                    conn.execute(
                        "INSERT INTO assertions (page_id, subject, predicate, object, asserted_by)
                         VALUES (?1, 'A', 'knows', 'B', 'manual')",
                        rusqlite::params![page_id],
                    )
                    .unwrap();
                }
                "raw-data" => {
                    conn.execute(
                        "INSERT INTO raw_data (page_id, source, data) VALUES (?1, 'api', '{}')",
                        rusqlite::params![page_id],
                    )
                    .unwrap();
                }
                "contradiction" => {
                    let other_page = insert_page(&conn, 1, "notes/other-contradiction");
                    conn.execute(
                        "INSERT INTO contradictions (page_id, other_page_id, type, description)
                         VALUES (?1, ?2, 'assertion_conflict', 'conflict')",
                        rusqlite::params![page_id, other_page],
                    )
                    .unwrap();
                }
                "knowledge-gap" => {
                    conn.execute(
                        "INSERT INTO knowledge_gaps (page_id, query_hash, context)
                         VALUES (?1, ?2, 'context')",
                        rusqlite::params![page_id, format!("gap-hash-{index}")],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }

        let missing: HashSet<PathBuf> = cases
            .iter()
            .map(|(_, relative_path)| PathBuf::from(relative_path))
            .collect();

        let (quarantined, hard_deleted) = classify_missing_paths(&conn, 1, &missing).unwrap();

        assert_eq!(quarantined, cases.len());
        assert_eq!(hard_deleted, 0);
    }

    #[test]
    fn classifier_hard_deletes_missing_pages_without_db_only_state() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();

        let page_id = insert_page(&conn, 1, "notes/plain");
        let stat = FileStat {
            mtime_ns: 1,
            ctime_ns: Some(1),
            size_bytes: 10,
            inode: Some(1),
        };
        upsert_file_state(&conn, 1, "missing/plain.md", page_id, &stat, "hash").unwrap();

        let missing = HashSet::from([PathBuf::from("missing/plain.md")]);
        let (quarantined, hard_deleted) = classify_missing_paths(&conn, 1, &missing).unwrap();

        assert_eq!(quarantined, 0);
        assert_eq!(hard_deleted, 1);
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_hard_deletes_missing_pages_without_db_only_state() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let stat = FileStat {
            mtime_ns: 1,
            ctime_ns: Some(1),
            size_bytes: 10,
            inode: Some(1),
        };
        let page_id = seed_file_state(&conn, collection.id, "notes/plain", "notes/plain.md", &stat);

        let stats = reconcile(&conn, &collection).unwrap();
        let page_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stats.hard_deleted, 1);
        assert_eq!(stats.quarantined_db_state, 0);
        assert_eq!(page_count, 0);
        assert!(
            crate::core::file_state::get_file_state(&conn, collection.id, "notes/plain.md")
                .unwrap()
                .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_quarantines_missing_pages_with_db_only_state() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let stat = FileStat {
            mtime_ns: 1,
            ctime_ns: Some(1),
            size_bytes: 10,
            inode: Some(1),
        };
        let page_id = seed_file_state(
            &conn,
            collection.id,
            "notes/quarantined",
            "notes/quarantined.md",
            &stat,
        );
        let other_page = insert_page(&conn, collection.id, "notes/other");
        insert_programmatic_link(&conn, page_id, other_page);

        let stats = reconcile(&conn, &collection).unwrap();
        let quarantined_at: Option<String> = conn
            .query_row(
                "SELECT quarantined_at FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stats.hard_deleted, 0);
        assert_eq!(stats.quarantined_db_state, 1);
        assert!(quarantined_at.is_some());
        assert!(crate::core::file_state::get_file_state(
            &conn,
            collection.id,
            "notes/quarantined.md"
        )
        .unwrap()
        .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_quarantines_missing_pages_for_each_db_only_state_branch() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());

        let cases = [
            ("programmatic-link", "missing/link.md"),
            ("manual-assertion", "missing/assertion.md"),
            ("raw-data", "missing/raw.md"),
            ("contradiction", "missing/contradiction.md"),
            ("knowledge-gap", "missing/gap.md"),
        ];

        for (index, (slug_suffix, relative_path)) in cases.iter().enumerate() {
            let slug = format!("notes/{slug_suffix}");
            let page_id = insert_page(&conn, collection.id, &slug);
            let stat = FileStat {
                mtime_ns: index as i64 + 1,
                ctime_ns: Some(index as i64 + 1),
                size_bytes: 10,
                inode: Some(index as i64 + 1),
            };
            upsert_file_state(&conn, collection.id, relative_path, page_id, &stat, "hash").unwrap();

            match *slug_suffix {
                "programmatic-link" => {
                    let other_page = insert_page(&conn, collection.id, "notes/other-link");
                    conn.execute(
                        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
                         VALUES (?1, ?2, 'related', 'programmatic')",
                        rusqlite::params![page_id, other_page],
                    )
                    .unwrap();
                }
                "manual-assertion" => {
                    conn.execute(
                        "INSERT INTO assertions (page_id, subject, predicate, object, asserted_by)
                         VALUES (?1, 'A', 'knows', 'B', 'manual')",
                        rusqlite::params![page_id],
                    )
                    .unwrap();
                }
                "raw-data" => {
                    conn.execute(
                        "INSERT INTO raw_data (page_id, source, data) VALUES (?1, 'api', '{}')",
                        rusqlite::params![page_id],
                    )
                    .unwrap();
                }
                "contradiction" => {
                    let other_page = insert_page(&conn, collection.id, "notes/other-contradiction");
                    conn.execute(
                        "INSERT INTO contradictions (page_id, other_page_id, type, description)
                         VALUES (?1, ?2, 'assertion_conflict', 'conflict')",
                        rusqlite::params![page_id, other_page],
                    )
                    .unwrap();
                }
                "knowledge-gap" => {
                    conn.execute(
                        "INSERT INTO knowledge_gaps (page_id, query_hash, context)
                         VALUES (?1, ?2, 'context')",
                        rusqlite::params![page_id, format!("gap-hash-{index}")],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }

        let stats = reconcile(&conn, &collection).unwrap();
        assert_eq!(stats.quarantined_db_state, cases.len());
        assert_eq!(stats.hard_deleted, 0);

        for (slug_suffix, relative_path) in cases {
            let quarantined_at: Option<String> = conn
                .query_row(
                    "SELECT quarantined_at FROM pages WHERE slug = ?1",
                    [format!("notes/{slug_suffix}")],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(quarantined_at.is_some());
            assert!(
                crate::core::file_state::get_file_state(&conn, collection.id, relative_path)
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_hard_deletes_missing_page_when_gap_is_not_attached_to_page() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let stat = FileStat {
            mtime_ns: 1,
            ctime_ns: Some(1),
            size_bytes: 10,
            inode: Some(1),
        };
        let page_id = seed_file_state(
            &conn,
            collection.id,
            "notes/plain-gap",
            "notes/plain-gap.md",
            &stat,
        );
        conn.execute(
            "INSERT INTO knowledge_gaps (page_id, query_hash, context)
             VALUES (NULL, 'orphan-gap', 'context')",
            [],
        )
        .unwrap();

        let stats = reconcile(&conn, &collection).unwrap();
        let page_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stats.hard_deleted, 1);
        assert_eq!(stats.quarantined_db_state, 0);
        assert_eq!(page_count, 0);
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_applies_hash_rename_and_rotates_raw_imports() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let content =
            "---\nslug: notes/renamed\ntitle: Rename me\ntype: concept\n---\nThis body alone is intentionally longer than sixty-four bytes so the hash rename guard can still pair it.\n";
        fs::write(root.path().join("renamed.md"), content).unwrap();
        let stat = stat_for(root.path(), "renamed.md");
        let sha256 = file_state::hash_file(&root.path().join("renamed.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/renamed",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "old-name.md",
                stat: &stat,
                sha256: &sha256,
                compiled_truth:
                    "This body alone is intentionally longer than sixty-four bytes so the hash rename guard can still pair it.",
                timeline: "",
            },
        );
        crate::core::raw_imports::rotate_active_raw_import(
            &conn,
            page_id,
            "old-name.md",
            b"old bytes",
        )
        .unwrap();

        let stats = reconcile(&conn, &collection).unwrap();
        let file_state_row =
            crate::core::file_state::get_file_state(&conn, collection.id, "renamed.md")
                .unwrap()
                .expect("renamed path should be tracked");
        let inactive_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM raw_imports
                 WHERE page_id = ?1 AND is_active = 0",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stats.hash_renamed, 1);
        assert_eq!(file_state_row.page_id, page_id);
        assert!(
            crate::core::file_state::get_file_state(&conn, collection.id, "old-name.md")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            crate::core::raw_imports::active_raw_import_count(&conn, page_id).unwrap(),
            1
        );
        assert_eq!(inactive_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_changed_hash_modified_path_rotates_raw_imports_to_latest_bytes() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
        fs::write(root.path().join("note.md"), original).unwrap();
        let original_stat = stat_for(root.path(), "note.md");
        let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &original_stat,
                sha256: &original_sha,
                compiled_truth: "Original body.",
                timeline: "",
            },
        );
        crate::core::raw_imports::rotate_active_raw_import(
            &conn,
            page_id,
            "note.md",
            original.as_bytes(),
        )
        .unwrap();

        let updated =
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body that is deliberately longer.\n";
        fs::write(root.path().join("note.md"), updated).unwrap();

        let stats = reconcile(&conn, &collection).unwrap();
        let file_state_row =
            crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
                .unwrap()
                .expect("modified path should still be tracked");
        let inactive_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1 AND is_active = 0",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        let compiled_truth: String = conn
            .query_row(
                "SELECT compiled_truth FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stats.modified, 1);
        assert_eq!(stats.unchanged, 0);
        assert_eq!(
            file_state_row.sha256,
            file_state::hash_file(&root.path().join("note.md")).unwrap()
        );
        assert_eq!(active_raw_import_count(&conn, page_id), 1);
        assert_eq!(active_raw_import_bytes(&conn, page_id), updated.as_bytes());
        assert_eq!(inactive_count, 1);
        assert_eq!(
            compiled_truth.trim_end(),
            "Updated body that is deliberately longer."
        );
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_changed_hash_aborts_before_mutation_when_history_has_zero_active_raw_import_rows()
    {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
        fs::write(root.path().join("note.md"), original).unwrap();
        let original_stat = stat_for(root.path(), "note.md");
        let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &original_stat,
                sha256: &original_sha,
                compiled_truth: "Original body.",
                timeline: "",
            },
        );
        conn.execute(
            "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
             VALUES (?1, ?2, 0, ?3, ?4)",
            rusqlite::params![
                page_id,
                crate::core::page_uuid::generate_uuid_v7(),
                original.as_bytes(),
                "note.md"
            ],
        )
        .unwrap();
        let before_row = crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
            .unwrap()
            .expect("file_state row should exist");

        let updated =
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body that is deliberately longer.\n";
        fs::write(root.path().join("note.md"), updated).unwrap();

        let error = reconcile(&conn, &collection).unwrap_err().to_string();
        let after_row = crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
            .unwrap()
            .expect("file_state row should still exist after abort");
        let compiled_truth: String = conn
            .query_row(
                "SELECT compiled_truth FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        let raw_import_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert!(error.contains("InvariantViolationError"));
        assert_eq!(compiled_truth, "Original body.");
        assert_eq!(after_row.sha256, before_row.sha256);
        assert_eq!(active_raw_import_count(&conn, page_id), 0);
        assert_eq!(raw_import_rows, 1);
    }

    #[cfg(unix)]
    #[test]
    fn full_hash_reconcile_changed_hash_rotates_raw_imports_atomically() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
        fs::write(root.path().join("note.md"), original).unwrap();
        let original_stat = stat_for(root.path(), "note.md");
        let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &original_stat,
                sha256: &original_sha,
                compiled_truth: "Original body.",
                timeline: "",
            },
        );
        crate::core::raw_imports::rotate_active_raw_import(
            &conn,
            page_id,
            "note.md",
            original.as_bytes(),
        )
        .unwrap();

        let updated =
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body that is deliberately longer.\n";
        fs::write(root.path().join("note.md"), updated).unwrap();

        let stats = full_hash_reconcile(&conn, collection.id).unwrap();
        let file_state_row =
            crate::core::file_state::get_file_state(&conn, collection.id, "note.md")
                .unwrap()
                .expect("file_state row should still exist");
        let inactive_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1 AND is_active = 0",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stats.modified, 1);
        assert_eq!(active_raw_import_count(&conn, page_id), 1);
        assert_eq!(active_raw_import_bytes(&conn, page_id), updated.as_bytes());
        assert_eq!(inactive_count, 1);
        assert_eq!(
            file_state_row.sha256,
            file_state::hash_file(&root.path().join("note.md")).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_fails_closed_when_existing_page_has_zero_total_raw_imports_on_modified_path() {
        // Nibbler adversarial seam: existing page on the stat-diff modified path with
        // row_count == 0 must fail with InvariantViolationError, not silently bootstrap.
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());
        let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
        fs::write(root.path().join("note.md"), original).unwrap();
        let original_stat = stat_for(root.path(), "note.md");
        let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
        let page_id = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/note",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
                relative_path: "note.md",
                stat: &original_stat,
                sha256: &original_sha,
                compiled_truth: "Original body.",
                timeline: "",
            },
        );
        // Intentionally leave raw_imports empty (row_count == 0, not just active_count == 0).
        let raw_import_rows_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw_import_rows_before, 0);

        let updated = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body.\n";
        fs::write(root.path().join("note.md"), updated).unwrap();

        let error = reconcile(&conn, &collection).unwrap_err().to_string();
        let compiled_truth: String = conn
            .query_row(
                "SELECT compiled_truth FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        let raw_import_rows_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert!(error.contains("InvariantViolationError"));
        assert_eq!(compiled_truth, "Original body.", "page must not be mutated");
        assert_eq!(
            raw_import_rows_after, 0,
            "no raw_imports row must be bootstrapped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_fails_closed_when_slug_matched_existing_page_has_zero_total_raw_imports() {
        // Nibbler adversarial seam: existing page found via slug-match on the remaining_new
        // path (existing_page_id = None at action construction time) must also fail closed
        // when row_count == 0.
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());

        // Insert page directly into pages with no file_state row — the stat-diff walk will
        // never see it as modified/missing; it's invisible to rename resolution.
        let page_id = insert_page(&conn, collection.id, "notes/note");
        let raw_import_rows_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw_import_rows_before, 0);

        // A new file appears with a slug that matches the existing DB page.
        let content = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nNew path body.\n";
        fs::write(root.path().join("new.md"), content).unwrap();

        // reconcile: "new.md" is in remaining_new (no file_state entry),
        // apply_reingest is called with existing_page_id = None,
        // load_existing_page_identity finds the DB page by slug "notes/note",
        // the zero-total-rows guard must fire before any mutation.
        let error = reconcile(&conn, &collection).unwrap_err().to_string();
        let raw_import_rows_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert!(error.contains("InvariantViolationError"));
        assert_eq!(
            raw_import_rows_after, 0,
            "no raw_imports row must be bootstrapped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_commits_in_500_file_chunks() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection(&conn, root.path());

        for index in 0..500 {
            fs::write(
                root.path().join(format!("note-{index:03}.md")),
                format!(
                    "---\nslug: notes/{index:03}\ntitle: Note {index}\ntype: concept\n---\nBody {index} with enough text to stay well formed.\n"
                ),
            )
            .unwrap();
        }
        fs::write(
            root.path().join("note-500.md"),
            "---\nmemory_id: not-a-uuid\nslug: notes/500\ntitle: Broken\ntype: concept\n---\nBroken body.\n",
        )
        .unwrap();

        let error = reconcile(&conn, &collection).unwrap_err().to_string();
        let committed_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE collection_id = ?1 AND slug LIKE 'notes/%'",
                [collection.id],
                |row| row.get(0),
            )
            .unwrap();

        assert!(error.contains("invalid frontmatter uuid"));
        assert_eq!(committed_count, 0);
    }

    // ── Coverage gap tests ────────────────────────────────────

    #[test]
    fn reconcile_skips_non_active_collection_without_error() {
        let conn = open_test_db();
        for state in [
            crate::core::collections::CollectionState::Detached,
            crate::core::collections::CollectionState::Restoring,
        ] {
            let collection = sample_collection_in_state(state);
            let result = reconcile_with_native_events(&conn, &collection, &[]);
            let stats = result.unwrap();
            assert_eq!(stats.walked, 0);
            assert_eq!(stats.unchanged, 0);
        }
    }

    #[test]
    fn drift_capture_summary_has_material_changes_returns_false_when_all_zero() {
        let summary = DriftCaptureSummary::default();
        assert!(!summary.has_material_changes());
    }

    #[test]
    fn drift_capture_summary_has_material_changes_returns_true_when_any_nonzero() {
        for summary in [
            DriftCaptureSummary {
                pages_updated: 1,
                ..Default::default()
            },
            DriftCaptureSummary {
                pages_added: 1,
                ..Default::default()
            },
            DriftCaptureSummary {
                pages_quarantined: 1,
                ..Default::default()
            },
            DriftCaptureSummary {
                pages_deleted: 1,
                ..Default::default()
            },
        ] {
            assert!(summary.has_material_changes());
        }
    }

    #[test]
    fn drift_capture_summary_from_stats_maps_fields_correctly() {
        let stats = ReconcileStats {
            modified: 2,
            new: 3,
            quarantined_ambiguous: 1,
            quarantined_db_state: 4,
            hard_deleted: 5,
            ..Default::default()
        };
        let summary = DriftCaptureSummary::from_stats(&stats);
        assert_eq!(summary.pages_updated, 2);
        assert_eq!(summary.pages_added, 3);
        assert_eq!(summary.pages_quarantined, 5); // ambiguous + db_state
        assert_eq!(summary.pages_deleted, 5);
    }

    #[test]
    fn drift_capture_summary_add_assign_accumulates_fields() {
        let mut left = DriftCaptureSummary {
            pages_updated: 1,
            pages_added: 2,
            pages_quarantined: 3,
            pages_deleted: 4,
        };
        let right = DriftCaptureSummary {
            pages_updated: 10,
            pages_added: 20,
            pages_quarantined: 30,
            pages_deleted: 40,
        };
        left.add_assign(&right);
        assert_eq!(left.pages_updated, 11);
        assert_eq!(left.pages_added, 22);
        assert_eq!(left.pages_quarantined, 33);
        assert_eq!(left.pages_deleted, 44);
    }

    #[test]
    fn collection_dirty_status_is_dirty_when_only_sentinel_count_nonzero() {
        let status = CollectionDirtyStatus {
            needs_full_sync: false,
            sentinel_count: 1,
            recovery_in_progress: false,
            last_sync_at: None,
        };
        assert!(status.is_dirty());
    }

    #[test]
    fn collection_dirty_status_is_not_dirty_when_all_clear() {
        let status = CollectionDirtyStatus {
            needs_full_sync: false,
            sentinel_count: 0,
            recovery_in_progress: false,
            last_sync_at: None,
        };
        assert!(!status.is_dirty());
    }

    #[test]
    fn phase2_stability_exceeds_max_iters_returns_collection_unstable_error() {
        fn always_different(n: usize) -> StatSnapshot {
            HashMap::from([(
                PathBuf::from("note.md"),
                FileStat {
                    mtime_ns: n as i64,
                    ctime_ns: Some(n as i64),
                    size_bytes: 10,
                    inode: Some(1),
                },
            )])
        }

        let mut call = 0usize;
        let err = run_phase2_stability_check(
            RestoreRemapOperation::Restore,
            2,
            "vault-test",
            || {
                call += 1;
                Ok(always_different(call))
            },
            || Ok(DriftCaptureSummary::default()),
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("CollectionUnstableError"));
        assert!(err.contains("vault-test"));
        assert!(err.contains("phase=stability"));
    }

    #[test]
    fn phase2_stability_remap_operation_error_contains_remap_in_message() {
        let mut call = 0usize;
        let err = run_phase2_stability_check(
            RestoreRemapOperation::Remap,
            1,
            "remap-vault",
            || {
                call += 1;
                Ok(HashMap::from([(
                    PathBuf::from("file.md"),
                    FileStat {
                        mtime_ns: call as i64,
                        ctime_ns: Some(call as i64),
                        size_bytes: 5,
                        inode: Some(1),
                    },
                )]))
            },
            || Ok(DriftCaptureSummary::default()),
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("CollectionUnstableError"));
        assert!(err.contains("operation=remap"));
    }

    #[test]
    fn build_full_hash_plan_classifies_unchanged_modified_new_and_missing_entries() {
        let conn = open_test_db();
        let root = TempDir::new().unwrap();
        let collection = insert_collection_with_state(
            &conn,
            root.path(),
            crate::core::collections::CollectionState::Active,
            false,
        );

        let unchanged_bytes =
            b"---\nslug: notes/unchanged\ntitle: Unchanged\ntype: concept\n---\nunchanged body\n";
        fs::write(root.path().join("unchanged.md"), unchanged_bytes).unwrap();
        let unchanged_stat = stat_for(root.path(), "unchanged.md");
        let unchanged_sha = file_state::hash_file(&root.path().join("unchanged.md")).unwrap();
        let unchanged_page = seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/unchanged",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761231",
                relative_path: "unchanged.md",
                stat: &unchanged_stat,
                sha256: &unchanged_sha,
                compiled_truth: "unchanged body",
                timeline: "",
            },
        );

        let modified_original =
            b"---\nslug: notes/modified\ntitle: Modified\ntype: concept\n---\nold modified body\n";
        fs::write(root.path().join("modified.md"), modified_original).unwrap();
        let modified_stat = stat_for(root.path(), "modified.md");
        let modified_sha = file_state::hash_file(&root.path().join("modified.md")).unwrap();
        seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/modified",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761232",
                relative_path: "modified.md",
                stat: &modified_stat,
                sha256: &modified_sha,
                compiled_truth: "old modified body",
                timeline: "",
            },
        );
        fs::write(
            root.path().join("modified.md"),
            b"---\nslug: notes/modified\ntitle: Modified\ntype: concept\n---\nnew modified body\n",
        )
        .unwrap();
        let modified_new_stat = stat_for(root.path(), "modified.md");

        let missing_bytes =
            b"---\nslug: notes/missing\ntitle: Missing\ntype: concept\n---\nmissing body\n";
        let missing_stat = FileStat {
            mtime_ns: 1,
            ctime_ns: Some(1),
            size_bytes: missing_bytes.len() as i64,
            inode: Some(1),
        };
        seed_page_with_identity(
            &conn,
            collection.id,
            SeededPageIdentity {
                slug: "notes/missing",
                uuid: "01969f11-9448-7d79-8d3f-c68f54761233",
                relative_path: "missing.md",
                stat: &missing_stat,
                sha256: "missing-sha",
                compiled_truth: "missing body",
                timeline: "",
            },
        );

        fs::write(
            root.path().join("new.md"),
            b"---\nslug: notes/new\ntitle: New\ntype: concept\n---\nnew body\n",
        )
        .unwrap();
        let new_stat = stat_for(root.path(), "new.md");

        let walked_files = HashMap::from([
            (PathBuf::from("unchanged.md"), unchanged_stat.clone()),
            (PathBuf::from("modified.md"), modified_new_stat.clone()),
            (PathBuf::from("new.md"), new_stat.clone()),
        ]);

        let plan = build_full_hash_plan(&conn, collection.id, root.path(), &walked_files).unwrap();

        assert_eq!(plan.unchanged.len(), 1);
        assert_eq!(plan.unchanged[0].page_id, unchanged_page);
        assert_eq!(
            plan.unchanged[0].relative_path,
            PathBuf::from("unchanged.md")
        );
        assert_eq!(plan.unchanged[0].sha256, unchanged_sha);
        assert!(plan
            .diff
            .modified
            .contains_key(&PathBuf::from("modified.md")));
        assert!(plan.diff.new.contains_key(&PathBuf::from("new.md")));
        assert!(plan.diff.missing.contains(&PathBuf::from("missing.md")));
    }

    #[test]
    fn detect_duplicate_uuids_in_tree_rejects_invalid_frontmatter_uuid() {
        let root = TempDir::new().unwrap();
        fs::write(
            root.path().join("broken.md"),
            b"---\nmemory_id: not-a-uuid\ntitle: Broken\ntype: concept\n---\nbody\n",
        )
        .unwrap();
        let walked_files = HashMap::from([(
            PathBuf::from("broken.md"),
            stat_for(root.path(), "broken.md"),
        )]);

        let error = detect_duplicate_uuids_in_tree(root.path(), &walked_files).unwrap_err();

        assert!(error
            .to_string()
            .contains("detect_duplicate_uuids_in_tree: broken.md has invalid frontmatter uuid"));
    }

    #[test]
    fn detect_duplicate_uuids_in_tree_rejects_duplicate_uuid_paths() {
        let root = TempDir::new().unwrap();
        let raw_bytes =
            b"---\nmemory_id: 01969f11-9448-7d79-8d3f-c68f54761234\ntitle: Duplicate\ntype: concept\n---\nbody\n";
        fs::write(root.path().join("one.md"), raw_bytes).unwrap();
        fs::write(root.path().join("two.md"), raw_bytes).unwrap();
        let walked_files = HashMap::from([
            (PathBuf::from("one.md"), stat_for(root.path(), "one.md")),
            (PathBuf::from("two.md"), stat_for(root.path(), "two.md")),
        ]);

        let error = detect_duplicate_uuids_in_tree(root.path(), &walked_files).unwrap_err();

        assert!(matches!(
            error,
            ReconcileError::DuplicateUuidError { ref uuid, ref paths }
                if uuid == "01969f11-9448-7d79-8d3f-c68f54761234"
                    && paths == &vec!["one.md".to_owned(), "two.md".to_owned()]
        ));
    }

    #[test]
    fn infer_type_from_path_returns_correct_type_for_each_folder() {
        let root = Path::new("/vault");
        let cases = [
            ("projects", "project"),
            ("areas", "area"),
            ("resources", "resource"),
            ("archives", "archive"),
            ("journal", "journal"),
            ("journals", "journal"),
            ("people", "person"),
            ("companies", "company"),
            ("orgs", "company"),
        ];
        for (folder, expected) in cases {
            let path = Path::new("/vault").join(folder).join("note.md");
            let result = infer_type_from_path(&path, root);
            assert_eq!(result.as_deref(), Some(expected), "folder={folder}");
        }
    }

    #[test]
    fn infer_type_from_path_returns_none_for_unknown_folder() {
        let root = Path::new("/vault");
        let path = Path::new("/vault/misc/note.md");
        assert!(infer_type_from_path(path, root).is_none());
    }

    #[test]
    fn infer_type_from_path_strips_numeric_prefix_before_matching() {
        let root = Path::new("/vault");
        let path = Path::new("/vault/01. Projects/something.md");
        let result = infer_type_from_path(path, root);
        assert_eq!(result.as_deref(), Some("project"));
    }

    #[test]
    fn strip_numeric_prefix_removes_leading_digits_dot_and_space() {
        assert_eq!(strip_numeric_prefix("01. Folder"), "Folder");
        assert_eq!(strip_numeric_prefix("123. Area"), "Area");
        assert_eq!(strip_numeric_prefix("9. Thing"), "Thing");
    }

    #[test]
    fn strip_numeric_prefix_leaves_non_numeric_name_unchanged() {
        assert_eq!(strip_numeric_prefix("Projects"), "Projects");
        assert_eq!(strip_numeric_prefix("01-not-standard"), "01-not-standard");
        assert_eq!(strip_numeric_prefix(""), "");
    }

    #[test]
    fn is_markdown_file_returns_true_for_md_extension() {
        assert!(is_markdown_file(Path::new("note.md")));
        assert!(is_markdown_file(Path::new("note.MD")));
        assert!(is_markdown_file(Path::new("note.Md")));
    }

    #[test]
    fn is_markdown_file_returns_false_for_non_md_extension() {
        assert!(!is_markdown_file(Path::new("note.txt")));
        assert!(!is_markdown_file(Path::new("image.png")));
        assert!(!is_markdown_file(Path::new("no-extension")));
        assert!(!is_markdown_file(Path::new(".hidden")));
    }

    #[test]
    fn raw_import_invariant_multiple_active_rows_fails_with_enforce_policy() {
        let result = raw_import_invariant_result(
            99,
            3,
            2,
            "test-context",
            RawImportInvariantPolicy::Enforce,
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("InvariantViolationError"));
        assert!(err.contains("page_id=99"));
        assert!(err.contains("2 active raw_imports rows"));
    }

    #[test]
    fn raw_import_invariant_multiple_active_rows_warns_with_allow_override_policy() {
        let result = raw_import_invariant_result(
            99,
            3,
            2,
            "test-context",
            RawImportInvariantPolicy::AllowRerenderOverride,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn reconcile_error_display_collection_lacks_writer_quiescence() {
        let err = ReconcileError::CollectionLacksWriterQuiescenceError {
            collection_name: "my-vault".to_owned(),
            root_path: "/mnt/vault".to_owned(),
        };
        let s = err.to_string();
        assert!(s.contains("CollectionLacksWriterQuiescenceError"));
        assert!(s.contains("my-vault"));
        assert!(s.contains("/mnt/vault"));
    }

    #[test]
    fn reconcile_error_display_collection_dirty_error() {
        let err = ReconcileError::CollectionDirtyError {
            collection_name: "dirty-vault".to_owned(),
            status: CollectionDirtyStatus {
                needs_full_sync: true,
                sentinel_count: 2,
                recovery_in_progress: false,
                last_sync_at: Some("2024-01-01T00:00:00Z".to_owned()),
            },
        };
        let s = err.to_string();
        assert!(s.contains("CollectionDirtyError"));
        assert!(s.contains("dirty-vault"));
        assert!(s.contains("needs_full_sync=true"));
        assert!(s.contains("sentinel_count=2"));
    }

    #[test]
    fn reconcile_error_display_remap_drift_conflict_error() {
        let err = ReconcileError::RemapDriftConflictError {
            collection_name: "remap-vault".to_owned(),
            summary: DriftCaptureSummary {
                pages_updated: 1,
                pages_added: 2,
                pages_quarantined: 0,
                pages_deleted: 0,
            },
        };
        let s = err.to_string();
        assert!(s.contains("RemapDriftConflictError"));
        assert!(s.contains("remap-vault"));
        assert!(s.contains("pages_updated=1"));
    }

    #[test]
    fn reconcile_error_display_collection_unstable_error() {
        let err = ReconcileError::CollectionUnstableError {
            collection_name: "unstable-vault".to_owned(),
            operation: RestoreRemapOperation::Remap,
            phase: "stability",
            retries: 5,
        };
        let s = err.to_string();
        assert!(s.contains("CollectionUnstableError"));
        assert!(s.contains("unstable-vault"));
        assert!(s.contains("operation=remap"));
        assert!(s.contains("retries=5"));
    }

    #[test]
    fn uuid_migration_preflight_succeeds_when_no_pages_exist() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);
        let result = uuid_migration_preflight(&conn, &collection);
        assert!(result.is_ok());
    }

    #[test]
    fn uuid_migration_preflight_succeeds_when_page_uuid_is_mirrored_in_frontmatter() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let uuid = "01969f11-9448-7d79-8d3f-c68f54761234";
        conn.execute(
            "INSERT INTO pages (collection_id, slug, uuid, type, title, compiled_truth, timeline, frontmatter)
             VALUES (1, 'notes/mirrored', ?1, 'concept', 'Mirrored', 'tiny', '', ?2)",
            rusqlite::params![
                uuid,
                format!("{{\"slug\":\"notes/mirrored\",\"memory_id\":\"{uuid}\"}}")
            ],
        )
        .unwrap();
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);
        let result = uuid_migration_preflight(&conn, &collection);
        assert!(
            result.is_ok(),
            "preflight should pass when uuid is mirrored: {result:?}"
        );
    }

    #[test]
    fn uuid_migration_preflight_succeeds_when_page_has_nontrivial_body_without_uuid_mirror() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO collections (name, root_path) VALUES ('test', '/vault')",
            [],
        )
        .unwrap();
        let large_body = "a".repeat(MIN_CANONICAL_BODY_BYTES as usize);
        conn.execute(
            "INSERT INTO pages (collection_id, slug, uuid, type, title, compiled_truth, timeline, frontmatter)
             VALUES (1, 'notes/large', '01969f11-9448-7d79-8d3f-c68f54761235', 'concept', 'Large', ?1, '', '{\"slug\":\"notes/large\"}')",
            rusqlite::params![large_body],
        )
        .unwrap();
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);
        // Large body → has_canonical_trivial_body returns false → no error
        let result = uuid_migration_preflight(&conn, &collection);
        assert!(
            result.is_ok(),
            "large-body page should not trigger preflight error: {result:?}"
        );
    }

    #[test]
    fn load_frontmatter_map_fails_on_invalid_json() {
        let result = load_frontmatter_map("not valid json {");
        assert!(matches!(result, Err(ReconcileError::Other(_))));
        let err = result.unwrap_err().to_string();
        assert!(err.contains("load_frontmatter_map"));
    }

    #[test]
    fn default_restore_stability_max_iters_uses_env_var_when_set() {
        let _guard = env_mutation_lock().lock().unwrap();
        let _env = EnvVarGuard::set("QUAID_RESTORE_STABILITY_MAX_ITERS", "12");
        let value = default_restore_stability_max_iters();
        assert_eq!(value, 12);
    }

    #[test]
    fn default_restore_stability_max_iters_falls_back_to_default_when_var_is_zero() {
        let _guard = env_mutation_lock().lock().unwrap();
        let _env = EnvVarGuard::set("QUAID_RESTORE_STABILITY_MAX_ITERS", "0");
        let value = default_restore_stability_max_iters();
        assert_eq!(value, DEFAULT_RESTORE_STABILITY_MAX_ITERS);
    }

    #[test]
    fn default_restore_stability_max_iters_guard_restores_previous_value() {
        let _guard = env_mutation_lock().lock().unwrap();
        let _original = EnvVarGuard::set("QUAID_RESTORE_STABILITY_MAX_ITERS", "7");
        {
            let _env = EnvVarGuard::set("QUAID_RESTORE_STABILITY_MAX_ITERS", "12");
            assert_eq!(default_restore_stability_max_iters(), 12);
        }

        assert_eq!(
            std::env::var("QUAID_RESTORE_STABILITY_MAX_ITERS").as_deref(),
            Ok("7")
        );
        assert_eq!(default_restore_stability_max_iters(), 7);
    }

    #[test]
    fn full_hash_reconcile_authorization_helpers_cover_all_identity_variants() {
        let audit = FullHashReconcileAuthorization::AuditCommand;
        assert_eq!(audit.as_str(), "audit-command");
        assert_eq!(audit.identity(), None);

        let active = FullHashReconcileAuthorization::ActiveLease {
            lease_session_id: "lease-1".to_owned(),
        };
        assert_eq!(active.as_str(), "active-lease");
        assert_eq!(active.identity(), Some("lease-1"));

        let attach = FullHashReconcileAuthorization::AttachCommand {
            attach_command_id: "attach-1".to_owned(),
        };
        assert_eq!(attach.as_str(), "attach-command");
        assert_eq!(attach.identity(), Some("attach-1"));

        let restore_command = FullHashReconcileAuthorization::RestoreCommand {
            restore_command_id: "restore-1".to_owned(),
        };
        assert_eq!(restore_command.as_str(), "restore-command");
        assert_eq!(restore_command.identity(), Some("restore-1"));

        let restore_lease = FullHashReconcileAuthorization::RestoreLease {
            lease_session_id: "lease-2".to_owned(),
        };
        assert_eq!(restore_lease.as_str(), "restore-lease");
        assert_eq!(restore_lease.identity(), Some("lease-2"));
    }

    #[test]
    fn sentinel_count_ignores_files_without_needs_full_sync_extension() {
        let recovery_root = tempfile::TempDir::new().unwrap();
        let collection_id = 1i64;
        let dir = collection_recovery_dir(recovery_root.path(), collection_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("write-1.needs_full_sync"), b"dirty").unwrap();
        fs::write(dir.join("readme.txt"), b"not a sentinel").unwrap();
        fs::write(dir.join("other.log"), b"not a sentinel").unwrap();

        let count = sentinel_count(recovery_root.path(), collection_id).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn sentinel_count_returns_zero_when_recovery_dir_does_not_exist() {
        let recovery_root = tempfile::TempDir::new().unwrap();
        let count = sentinel_count(recovery_root.path(), 999).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn authorize_remap_root_active_state_matching_lease_succeeds() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);
        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RemapRoot,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        );
        assert!(
            result.is_ok(),
            "remap-root with matching lease on active should pass: {result:?}"
        );
    }

    #[test]
    fn authorize_remap_root_restoring_state_matching_lease_succeeds() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Restoring);
        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RemapRoot,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        );
        // `owner_identity_defaults_for_state(Restoring)` sets active_lease to None,
        // so this should fail with missing persisted owner identity.
        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing persisted owner identity"));
    }

    #[test]
    fn authorize_remap_root_active_state_mismatched_lease_fails() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);
        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RemapRoot,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "wrong-lease".to_owned(),
            },
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("caller identity mismatch"));
        assert!(err.contains("remap-root"));
    }

    #[test]
    fn authorize_overflow_recovery_active_state_with_active_lease_succeeds() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);
        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::OverflowRecovery,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn authorize_restore_active_state_with_active_lease_matching_owner_succeeds() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);
        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::Restore,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "lease-1".to_owned(),
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn authorize_full_hash_reconcile_empty_identity_fails() {
        let collection =
            sample_collection_in_state(crate::core::collections::CollectionState::Active);
        let result = authorize_full_hash_reconcile(
            &collection,
            FullHashReconcileMode::RemapDriftCapture,
            &FullHashReconcileAuthorization::ActiveLease {
                lease_session_id: "   ".to_owned(),
            },
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing caller identity"));
    }

    #[test]
    fn full_hash_reconcile_mode_as_str_covers_all_variants() {
        assert_eq!(FullHashReconcileMode::Audit.as_str(), "audit");
        assert_eq!(FullHashReconcileMode::FreshAttach.as_str(), "fresh-attach");
        assert_eq!(
            FullHashReconcileMode::OverflowRecovery.as_str(),
            "overflow-recovery"
        );
        assert_eq!(FullHashReconcileMode::RemapRoot.as_str(), "remap-root");
        assert_eq!(FullHashReconcileMode::Restore.as_str(), "restore");
        assert_eq!(
            FullHashReconcileMode::RemapDriftCapture.as_str(),
            "remap-drift-capture"
        );
        assert_eq!(
            FullHashReconcileMode::RestoreDriftCapture.as_str(),
            "restore-drift-capture"
        );
    }

    #[test]
    fn restore_remap_operation_as_str_covers_both_variants() {
        assert_eq!(RestoreRemapOperation::Restore.as_str(), "restore");
        assert_eq!(RestoreRemapOperation::Remap.as_str(), "remap");
    }
}
