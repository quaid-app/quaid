//! Vault remap operator entry: rewire a collection from its old
//! `root_path` to a new directory tree on disk without losing
//! identity.
//!
//! [`remap_collection`] is the operator entry point. It validates
//! the new root via [`verify_remap_root`] (page-by-page identity
//! resolution + tree-fence stability check), runs the same safety
//! pipeline as begin_restore (so any pre-existing drift on the
//! *old* root surfaces before the swap), and then flips
//! `collections.root_path` to the new directory plus
//! `state = 'restoring'` + `needs_full_sync = 1` so the watcher
//! reconciles the new root from scratch on the next supervisor
//! tick.
//!
//! Online vs offline: the online path uses the live serve session's
//! handshake (mark + ack) to acquire the active lease; the offline
//! path acquires a short-lived CLI lease and runs the post-reconcile
//! attach inline before returning. The behaviour for `state` /
//! `needs_full_sync` / `file_state reset` is identical between
//! the two branches.
//!
//! `verify_remap_root` is the safety check that prevents a
//! mistargeted remap. It walks the new root, hashes each markdown
//! file, runs the same `resolve_page_identity` machinery the
//! reconciler uses, and only returns `Ok` when every page maps to
//! a file at the same sha256 and every file maps to a page. The
//! tree-fence assertion (`take_tree_fence` before/after) catches a
//! racing third-party writer that might mutate the new root while
//! we're inspecting it.
//!
//! The leaf helpers (`load_remap_page_rows`, `load_new_root_files`,
//! `build_new_root_ignore_globset`, `resolve_page_matches`,
//! `compare_manifest`, the manifest-comparison enum, the page-match
//! resolution struct, `walk_tree`, `take_tree_fence`,
//! `metadata_fence_tuple`, `metadata_timestamp_ns`, `path_string`,
//! `push_sample`, `samples_to_paths`) all live here because they
//! are private to this remap/verification flow.
//!
//! `walk_tree`, `path_string`, and `compare_manifest` are exposed
//! at `pub(super)` because the manifest-finalize path in
//! `vault_sync::mod` (`build_restore_manifest_for_directory` and
//! `finalize_pending_restore`) reuses them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(not(unix))]
use std::time::UNIX_EPOCH;

use rusqlite::{params, Connection};

use crate::core::collections::{self, Collection, CollectionState};
use crate::core::ignore_patterns;
use crate::core::markdown;
use crate::core::page_uuid;
use crate::core::reconciler::{
    is_markdown_file, resolve_page_identity, run_restore_remap_safety_pipeline_without_mount_check,
    CanonicalIdentityRecord, FullHashReconcileAuthorization, PageIdentityResolution,
    RestoreRemapOperation, RestoreRemapSafetyRequest,
};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use super::recovery::{bootstrap_recovery_directories, recovery_root_for_db_path};
use super::restore::{ensure_restore_not_blocked, AttachReason, RestoreError, RestoreManifest};
use super::{
    complete_attach, convert_reconcile_error, database_path,
    mark_collection_restoring_for_handshake, sha256_hex, start_short_lived_owner_lease,
    wait_for_exact_ack, RemapVerificationSummary, VaultSyncError,
};

/// Maximum number of sample paths reported in
/// `RestoreError::NewRootVerificationFailed` for each of the
/// missing / mismatched / extra buckets. Keeps the human-readable
/// error short enough to fit in a CLI line; the typed counts carry
/// the full size.
const SAMPLE_LIMIT: usize = 5;

/// Rewires a collection from its existing `root_path` to `new_root`
/// without losing page identity, verifying the new tree is a
/// bit-identical mirror before flipping `collections.root_path` and
/// re-arming the watcher for a fresh reconcile.
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
                lease_session_id: expected_session_id,
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

/// Validates that `new_root` is a faithful copy of `collection` by
/// hashing every markdown file, resolving page identity, and asserting
/// no third-party writer has mutated the tree mid-check via a
/// before/after tree-fence comparison.
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
        return Err(VaultSyncError::Restore(RestoreError::NewRootUnstable {
            collection_name: collection.name.clone(),
        }));
    }
    let missing = page_matches.missing_count;
    let mismatched = page_matches.mismatched_pages;
    let extra = page_matches.extra_count;
    if missing != 0 || mismatched != 0 || extra != 0 {
        return Err(VaultSyncError::Restore(
            RestoreError::NewRootVerificationFailed {
                collection_name: collection.name.clone(),
                missing,
                mismatched,
                extra,
                missing_samples: samples_to_paths(&page_matches.missing_pages),
                mismatched_samples: samples_to_paths(&page_matches.mismatched_samples),
                extra_samples: samples_to_paths(&page_matches.extra_files),
            },
        ));
    }
    Ok(RemapVerificationSummary {
        resolved_pages: page_matches.resolved_page_ids.len(),
        missing_pages: missing,
        mismatched_pages: mismatched,
        extra_files: extra,
    })
}

pub(super) enum ManifestComparison {
    Matches,
    MissingFiles,
    Mismatch,
}

pub(super) fn compare_manifest(
    path: &Path,
    manifest: &RestoreManifest,
) -> Result<ManifestComparison, VaultSyncError> {
    let mut actual = super::build_restore_manifest_for_directory(path)?;
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
pub(super) struct TreeFenceEntry {
    pub(super) mtime_ns: i64,
    pub(super) ctime_ns: i64,
    pub(super) size_bytes: u64,
    pub(super) inode: u64,
    pub(super) quaidignore_sha256: Option<String>,
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
            ignore_patterns::ParseResult::Valid(patterns) => {
                Some(serde_json::to_string(&patterns).map_err(|e| {
                    VaultSyncError::InvariantViolation {
                        message: format!(
                            "failed to serialize validated .quaidignore patterns: {e}"
                        ),
                    }
                })?)
            }
            ignore_patterns::ParseResult::Invalid(errors) => {
                let message = match errors.first() {
                    Some(first) => format!(
                        "invalid .quaidignore in remap root at line {}: {}",
                        first.line, first.message
                    ),
                    None => "invalid .quaidignore in remap root (no error details)".to_owned(),
                };
                return Err(VaultSyncError::InvariantViolation { message });
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
        .take(SAMPLE_LIMIT)
        .collect::<Vec<_>>();
    let extra_count = files
        .iter()
        .filter(|file| !accounted_file_paths.contains(&file.relative_path))
        .count();
    let extra_files = files
        .iter()
        .filter(|file| !accounted_file_paths.contains(&file.relative_path))
        .map(|file| path_string(&file.relative_path))
        .take(SAMPLE_LIMIT)
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

pub(super) fn walk_tree(root: &Path) -> Result<BTreeMap<PathBuf, fs::Metadata>, VaultSyncError> {
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

pub(super) fn path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn push_sample(samples: &mut Vec<String>, sample: String) {
    if samples.len() < SAMPLE_LIMIT {
        samples.push(sample);
    }
}

fn samples_to_paths(samples: &[String]) -> Vec<PathBuf> {
    samples.iter().map(PathBuf::from).collect()
}

/// `restore_reset` and `reconcile_reset` live here because they are
/// the operator escape-hatches that complement `remap_collection`
/// (clear an integrity-blocked restore / reconcile-halted state so
/// the next remap can proceed).
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
        return Err(VaultSyncError::Restore(RestoreError::RestoreResetBlocked {
            collection_name: collection.name,
            reason,
        }));
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

/// Clears the `reconcile_halted_at` / `reconcile_halt_reason` flags
/// on a collection so the watcher can resume automatic reconciliation
/// after an operator has resolved the underlying integrity issue.
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

#[cfg(test)]
pub(super) fn take_tree_fence_for_test(
    root: &Path,
) -> Result<BTreeMap<String, TreeFenceEntry>, VaultSyncError> {
    take_tree_fence(root)
}
