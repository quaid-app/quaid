use std::io::{self, Read};

#[cfg(unix)]
use std::fs::{self, File};
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::path::{Path, PathBuf};

use rusqlite::Connection;
#[cfg(unix)]
use rustix::{fd::AsFd, fs::fsync};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use uuid::Uuid;

use crate::core::{file_state, markdown, page_uuid, palace, raw_imports, vault_sync};

#[derive(Debug, Clone)]
struct PreparedPut {
    collection_id: i64,
    collection_name: String,
    slug: String,
    page_uuid: String,
    page_type: String,
    title: String,
    summary: String,
    compiled_truth: String,
    timeline: String,
    frontmatter_json: String,
    wing: String,
    room: String,
    now: String,
    current_version: Option<i64>,
    sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PutOutcome {
    created: bool,
    version: i64,
}

#[cfg(unix)]
#[derive(Debug, Clone, Default)]
struct PutTestHooks {
    fail_sentinel_create: bool,
    fail_before_rename: bool,
    fail_rename: bool,
    fail_parent_fsync: bool,
    block_inside_slug_lock: bool,
    post_rename_swap: Option<Vec<u8>>,
}

#[cfg(unix)]
impl PutTestHooks {
    fn before_rename(&self, _target_path: &Path) -> Result<(), vault_sync::VaultSyncError> {
        if self.fail_before_rename {
            Err(io::Error::other("injected pre-rename failure").into())
        } else {
            Ok(())
        }
    }

    fn after_rename(&self, target_path: &Path) -> Result<(), vault_sync::VaultSyncError> {
        let Some(replacement) = self.post_rename_swap.as_ref() else {
            return Ok(());
        };
        let foreign_temp = target_path.with_file_name(format!(".foreign-{}", Uuid::now_v7()));
        fs::write(&foreign_temp, replacement)?;
        fs::rename(&foreign_temp, target_path)?;
        Ok(())
    }
}

/// Read markdown from stdin, parse it, and insert or update a page.
///
/// OCC contract:
/// - New page (no row for `slug`): INSERT with `version = 1`.
/// - Existing page + `--expected-version N`: compare-and-swap UPDATE.
///   If stored version ≠ N → print conflict with current version, exit 1.
/// - On Unix vault write-through paths, existing pages MUST provide
///   `--expected-version`; omission fails closed before sentinel creation.
/// - Non-Unix paths fail closed with `UnsupportedPlatformError`.
pub fn run(db: &Connection, slug: &str, expected_version: Option<i64>) -> anyhow::Result<()> {
    vault_sync::ensure_unix_platform("gbrain put")
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    put_from_string(db, slug, &input, expected_version)?;
    Ok(())
}

/// Apply page content supplied by the caller.
pub fn put_from_string(
    db: &Connection,
    slug_input: &str,
    content: &str,
    expected_version: Option<i64>,
) -> anyhow::Result<()> {
    let (frontmatter, body) = markdown::parse_frontmatter(content);
    let (compiled_truth, timeline) = markdown::split_content(&body);
    let summary = markdown::extract_summary(&compiled_truth);
    let op_kind = if expected_version.is_some() {
        crate::core::collections::OpKind::WriteUpdate
    } else {
        crate::core::collections::OpKind::WriteCreate
    };
    let resolved = vault_sync::resolve_slug_for_op(db, slug_input, op_kind)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    vault_sync::ensure_collection_vault_write_allowed(db, resolved.collection_id)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let slug = resolved.slug.as_str();
    let wing = palace::derive_wing(slug);
    let room = palace::derive_room(&compiled_truth);

    let title = frontmatter
        .get("title")
        .cloned()
        .unwrap_or_else(|| slug.to_string());
    let page_type = frontmatter
        .get("type")
        .cloned()
        .unwrap_or_else(|| "concept".to_string());

    let relative_path = slug_to_relative_path(slug);
    let now = now_iso_from(db);
    let (prepared, outcome) = vault_sync::with_write_slug_lock(
        resolved.collection_id,
        &relative_path,
        || -> anyhow::Result<(PreparedPut, PutOutcome)> {
            maybe_block_inside_write_lock();
            let existing_row: Option<(i64, Option<String>)> = match db
                .prepare("SELECT version, uuid FROM pages WHERE collection_id = ?1 AND slug = ?2")?
                .query_row(rusqlite::params![resolved.collection_id, slug], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                }) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.into()),
            };
            let page_uuid = page_uuid::resolve_page_uuid(
                &frontmatter,
                existing_row.as_ref().and_then(|(_, uuid)| uuid.as_deref()),
            )?;
            let prepared = PreparedPut {
                collection_id: resolved.collection_id,
                collection_name: resolved.collection_name.clone(),
                slug: slug.to_owned(),
                page_uuid,
                page_type,
                title: title.clone(),
                summary: summary.clone(),
                compiled_truth: compiled_truth.clone(),
                timeline: timeline.clone(),
                frontmatter_json: serde_json::to_string(&frontmatter)?,
                wing: wing.clone(),
                room: room.clone(),
                now: now.clone(),
                current_version: existing_row.map(|(version, _)| version),
                sha256: sha256_hex(content.as_bytes()),
            };
            let outcome = persist_with_vault_write(
                db,
                &prepared,
                content.as_bytes(),
                &relative_path,
                expected_version,
            )
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
            Ok((prepared, outcome))
        },
    )
    .map_err(|err| anyhow::anyhow!(err.to_string()))??;

    let verb = if outcome.created {
        "Created"
    } else {
        "Updated"
    };
    println!(
        "{verb} {}::{} (version {})",
        prepared.collection_name, prepared.slug, outcome.version
    );

    Ok(())
}

/// Get current UTC timestamp in ISO 8601 format from SQLite.
/// Keeps us dependency-free (no chrono) and consistent with schema defaults.
fn now_iso_from(db: &Connection) -> String {
    db.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })
    .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn persist_page_record(
    db: &Connection,
    prepared: &PreparedPut,
    raw_bytes: &[u8],
    relative_path: &str,
    file_stat: Option<&file_state::FileStat>,
    expected_version: Option<i64>,
) -> Result<PutOutcome, rusqlite::Error> {
    let tx = db.unchecked_transaction()?;
    let (created, version) = match prepared.current_version {
        None => {
            tx.execute(
                "INSERT INTO pages \
                     (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, \
                      frontmatter, wing, room, version, \
                        created_at, updated_at, truth_updated_at, timeline_updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12, ?12, ?12)",
                rusqlite::params![
                    prepared.collection_id,
                    prepared.slug,
                    prepared.page_uuid,
                    prepared.page_type,
                    prepared.title,
                    prepared.summary,
                    prepared.compiled_truth,
                    prepared.timeline,
                    prepared.frontmatter_json,
                    prepared.wing,
                    prepared.room,
                    prepared.now,
                ],
            )?;
            (true, 1)
        }
        Some(current) => {
            let rows = if let Some(expected) = expected_version {
                tx.execute(
                    "UPDATE pages SET \
                         uuid = ?1, type = ?2, title = ?3, summary = ?4, \
                          compiled_truth = ?5, timeline = ?6, \
                          frontmatter = ?7, wing = ?8, room = ?9, \
                          version = version + 1, \
                          updated_at = ?10, truth_updated_at = ?10, timeline_updated_at = ?10 \
                     WHERE collection_id = ?11 AND slug = ?12 AND version = ?13",
                    rusqlite::params![
                        prepared.page_uuid,
                        prepared.page_type,
                        prepared.title,
                        prepared.summary,
                        prepared.compiled_truth,
                        prepared.timeline,
                        prepared.frontmatter_json,
                        prepared.wing,
                        prepared.room,
                        prepared.now,
                        prepared.collection_id,
                        prepared.slug,
                        expected,
                    ],
                )?
            } else {
                tx.execute(
                    "UPDATE pages SET \
                         uuid = ?1, type = ?2, title = ?3, summary = ?4, \
                          compiled_truth = ?5, timeline = ?6, \
                          frontmatter = ?7, wing = ?8, room = ?9, \
                          version = version + 1, \
                          updated_at = ?10, truth_updated_at = ?10, timeline_updated_at = ?10 \
                     WHERE collection_id = ?11 AND slug = ?12",
                    rusqlite::params![
                        prepared.page_uuid,
                        prepared.page_type,
                        prepared.title,
                        prepared.summary,
                        prepared.compiled_truth,
                        prepared.timeline,
                        prepared.frontmatter_json,
                        prepared.wing,
                        prepared.room,
                        prepared.now,
                        prepared.collection_id,
                        prepared.slug,
                    ],
                )?
            };

            if rows == 0 {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "Conflict: page updated elsewhere (current version: {current})"
                )));
            }

            (false, current + 1)
        }
    };

    let page_id: i64 = tx.query_row(
        "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
        rusqlite::params![prepared.collection_id, prepared.slug],
        |row| row.get(0),
    )?;

    if let Some(file_stat) = file_stat {
        file_state::upsert_file_state(
            &tx,
            prepared.collection_id,
            relative_path,
            page_id,
            file_stat,
            &prepared.sha256,
        )?;
    }
    raw_imports::rotate_active_raw_import(&tx, page_id, relative_path, raw_bytes)?;
    raw_imports::enqueue_embedding_job(&tx, page_id)?;
    tx.commit()?;

    Ok(PutOutcome { created, version })
}

#[cfg(not(unix))]
fn persist_with_vault_write(
    db: &Connection,
    prepared: &PreparedPut,
    raw_bytes: &[u8],
    relative_path: &str,
    expected_version: Option<i64>,
) -> Result<PutOutcome, vault_sync::VaultSyncError> {
    persist_page_record(
        db,
        prepared,
        raw_bytes,
        relative_path,
        None,
        expected_version,
    )
    .map_err(Into::into)
}

#[cfg(unix)]
fn persist_with_vault_write(
    db: &Connection,
    prepared: &PreparedPut,
    raw_bytes: &[u8],
    relative_path: &str,
    expected_version: Option<i64>,
) -> Result<PutOutcome, vault_sync::VaultSyncError> {
    let db_path = vault_sync::database_path(db)?;
    if db_path.is_empty() || db_path == ":memory:" {
        return persist_page_record(
            db,
            prepared,
            raw_bytes,
            relative_path,
            None,
            expected_version,
        )
        .map_err(Into::into);
    }

    let collection = vault_sync::load_collection_by_id(db, prepared.collection_id)?;
    let relative_path_buf = PathBuf::from(relative_path);
    let target_path = Path::new(&collection.root_path).join(&relative_path_buf);
    let write_id = Uuid::now_v7().to_string();
    let sentinel_name = format!("{write_id}.needs_full_sync");
    let recovery_dir = vault_sync::collection_recovery_dir(
        &vault_sync::recovery_root_for_db_path(Path::new(&db_path)),
        prepared.collection_id,
    );
    let sentinel_path = recovery_dir.join(&sentinel_name);
    let dedup_key = format!(
        "{}:{}:{}",
        prepared.collection_id, relative_path, prepared.sha256
    );
    vault_sync::check_update_expected_version(
        prepared.collection_id,
        relative_path,
        prepared.current_version,
        expected_version,
    )?;
    let _fs_precondition = vault_sync::check_fs_precondition_before_sentinel(
        db,
        prepared.collection_id,
        Path::new(&collection.root_path),
        &relative_path_buf,
    )?;

    create_recovery_sentinel(prepared, &recovery_dir, &sentinel_name)?;

    if let Some(parent) = target_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            let _ = remove_recovery_sentinel(&recovery_dir, &sentinel_name);
            return Err(error.into());
        }
    }

    let root_fd = match fs_safety::open_root_fd(Path::new(&collection.root_path)) {
        Ok(root_fd) => root_fd,
        Err(error) => {
            let _ = remove_recovery_sentinel(&recovery_dir, &sentinel_name);
            return Err(error.into());
        }
    };
    let parent_fd = match fs_safety::walk_to_parent(&root_fd, &relative_path_buf) {
        Ok(parent_fd) => parent_fd,
        Err(error) => {
            let _ = remove_recovery_sentinel(&recovery_dir, &sentinel_name);
            return Err(error.into());
        }
    };
    let target_name = match relative_path_buf.file_name() {
        Some(target_name) => target_name,
        None => {
            let _ = remove_recovery_sentinel(&recovery_dir, &sentinel_name);
            return Err(vault_sync::VaultSyncError::InvariantViolation {
                message: format!("slug={} produced no filename", prepared.slug),
            });
        }
    };
    let temp_name = PathBuf::from(format!(".gbrain-write-{write_id}.tmp"));
    let temp_file = match create_tempfile(&parent_fd, &temp_name, raw_bytes) {
        Ok(temp_file) => temp_file,
        Err(error) => {
            let _ = cleanup_pre_rename(
                &parent_fd,
                &temp_name,
                &target_path,
                &dedup_key,
                &recovery_dir,
                &sentinel_name,
            );
            return Err(error);
        }
    };
    let temp_identity = file_identity(&temp_file)?;
    let hooks = test_hooks_snapshot();

    if let Ok(existing) = fs_safety::stat_at_nofollow(&parent_fd, target_name) {
        if existing.is_symlink() {
            let _ = cleanup_pre_rename(
                &parent_fd,
                &temp_name,
                &target_path,
                &dedup_key,
                &recovery_dir,
                &sentinel_name,
            );
            return Err(io::Error::other("target path is a symlink").into());
        }
    }

    if let Some(hook) = hooks.as_ref() {
        if let Err(error) = hook.before_rename(&target_path) {
            let _ = cleanup_pre_rename(
                &parent_fd,
                &temp_name,
                &target_path,
                &dedup_key,
                &recovery_dir,
                &sentinel_name,
            );
            return Err(error);
        }
    }

    if let Err(error) = vault_sync::insert_write_dedup(&dedup_key) {
        let _ = vault_sync::forget_self_write_path(&target_path);
        let _ = cleanup_pre_rename(
            &parent_fd,
            &temp_name,
            &target_path,
            &dedup_key,
            &recovery_dir,
            &sentinel_name,
        );
        return Err(error);
    }
    if let Err(error) = vault_sync::remember_self_write_path(&target_path, &prepared.sha256) {
        let _ = cleanup_pre_rename(
            &parent_fd,
            &temp_name,
            &target_path,
            &dedup_key,
            &recovery_dir,
            &sentinel_name,
        );
        return Err(error);
    }

    if let Some(hook) = hooks.as_ref() {
        if hook.fail_rename {
            let error = io::Error::other("injected rename failure");
            let _ = cleanup_pre_rename(
                &parent_fd,
                &temp_name,
                &target_path,
                &dedup_key,
                &recovery_dir,
                &sentinel_name,
            );
            return Err(error.into());
        }
    }

    if let Err(error) = fs_safety::renameat_parent_fd(&parent_fd, &temp_name, target_name) {
        let _ = cleanup_pre_rename(
            &parent_fd,
            &temp_name,
            &target_path,
            &dedup_key,
            &recovery_dir,
            &sentinel_name,
        );
        return Err(error.into());
    }

    if let Some(hook) = hooks.as_ref() {
        if hook.fail_parent_fsync {
            return Err(handle_post_rename_failure(
                db,
                prepared,
                relative_path,
                &sentinel_path,
                &target_path,
                &dedup_key,
                "fsync-parent",
                io::Error::other("injected parent fsync failure").to_string(),
            ));
        }
    }
    if let Err(error) = sync_fd(&parent_fd) {
        return Err(handle_post_rename_failure(
            db,
            prepared,
            relative_path,
            &sentinel_path,
            &target_path,
            &dedup_key,
            "fsync-parent",
            error.to_string(),
        ));
    }

    if let Some(hook) = hooks.as_ref() {
        hook.after_rename(&target_path)?;
    }

    let post_rename_stat = match file_state::stat_file_fd(&parent_fd, target_name) {
        Ok(stat) => stat,
        Err(error) => {
            return Err(handle_post_rename_failure(
                db,
                prepared,
                relative_path,
                &sentinel_path,
                &target_path,
                &dedup_key,
                "post-rename-stat",
                error.to_string(),
            ));
        }
    };

    let final_hash = match file_state::hash_file(&target_path) {
        Ok(hash) => hash,
        Err(error) => {
            return Err(handle_post_rename_failure(
                db,
                prepared,
                relative_path,
                &sentinel_path,
                &target_path,
                &dedup_key,
                "post-rename-hash",
                error.to_string(),
            ));
        }
    };

    if post_rename_stat.size_bytes != temp_identity.size_bytes
        || post_rename_stat.inode != Some(temp_identity.inode)
        || final_hash != prepared.sha256
    {
        let _ = vault_sync::remove_write_dedup(&dedup_key);
        let _ = vault_sync::forget_self_write_path(&target_path);
        let _ = vault_sync::mark_collection_needs_full_sync_via_fresh_connection(
            db,
            prepared.collection_id,
        );
        return Err(vault_sync::VaultSyncError::ConcurrentRename {
            collection_id: prepared.collection_id,
            relative_path: relative_path.to_owned(),
            sentinel_path: sentinel_path.display().to_string(),
        });
    }

    let outcome = match persist_page_record(
        db,
        prepared,
        raw_bytes,
        relative_path,
        Some(&post_rename_stat),
        expected_version,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            return Err(handle_post_rename_failure(
                db,
                prepared,
                relative_path,
                &sentinel_path,
                &target_path,
                &dedup_key,
                "commit",
                error.to_string(),
            ));
        }
    };

    let _ = vault_sync::remove_write_dedup(&dedup_key);
    let _ = remove_recovery_sentinel(&recovery_dir, &sentinel_name);
    Ok(outcome)
}

fn slug_to_relative_path(slug: &str) -> String {
    format!("{slug}.md")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    size_bytes: i64,
    inode: i64,
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<FileIdentity, vault_sync::VaultSyncError> {
    let metadata = file.metadata()?;
    Ok(FileIdentity {
        size_bytes: metadata.len() as i64,
        inode: metadata.ino() as i64,
    })
}

#[cfg(unix)]
fn create_recovery_sentinel(
    prepared: &PreparedPut,
    recovery_dir: &Path,
    sentinel_name: &str,
) -> Result<(), vault_sync::VaultSyncError> {
    let sentinel_path = recovery_dir.join(sentinel_name);
    #[cfg(test)]
    if test_hooks_snapshot()
        .as_ref()
        .is_some_and(|hook| hook.fail_sentinel_create)
    {
        return Err(vault_sync::VaultSyncError::RecoverySentinel {
            collection_id: prepared.collection_id,
            relative_path: prepared.slug.clone(),
            sentinel_path: sentinel_path.display().to_string(),
            reason: "injected sentinel creation failure".to_string(),
        });
    }
    fs::create_dir_all(recovery_dir).map_err(|error| {
        vault_sync::VaultSyncError::RecoverySentinel {
            collection_id: prepared.collection_id,
            relative_path: prepared.slug.clone(),
            sentinel_path: sentinel_path.display().to_string(),
            reason: error.to_string(),
        }
    })?;
    let recovery_fd = fs_safety::open_root_fd(recovery_dir).map_err(|error| {
        vault_sync::VaultSyncError::RecoverySentinel {
            collection_id: prepared.collection_id,
            relative_path: prepared.slug.clone(),
            sentinel_path: sentinel_path.display().to_string(),
            reason: error.to_string(),
        }
    })?;
    let mut sentinel_file = File::from(
        fs_safety::openat_create_excl(&recovery_fd, Path::new(sentinel_name)).map_err(|error| {
            vault_sync::VaultSyncError::RecoverySentinel {
                collection_id: prepared.collection_id,
                relative_path: prepared.slug.clone(),
                sentinel_path: sentinel_path.display().to_string(),
                reason: error.to_string(),
            }
        })?,
    );
    if let Err(error) = sentinel_file
        .write_all(b"dirty\n")
        .and_then(|_| sentinel_file.sync_all())
        .and_then(|_| sync_fd(&recovery_fd))
    {
        let _ = fs_safety::unlinkat_parent_fd(&recovery_fd, Path::new(sentinel_name));
        let _ = sync_fd(&recovery_fd);
        return Err(vault_sync::VaultSyncError::RecoverySentinel {
            collection_id: prepared.collection_id,
            relative_path: prepared.slug.clone(),
            sentinel_path: sentinel_path.display().to_string(),
            reason: error.to_string(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn create_tempfile<Fd: AsFd>(
    parent_fd: Fd,
    temp_name: &Path,
    raw_bytes: &[u8],
) -> Result<File, vault_sync::VaultSyncError> {
    let mut temp_file = File::from(fs_safety::openat_create_excl(parent_fd, temp_name)?);
    temp_file.write_all(raw_bytes)?;
    temp_file.sync_all()?;
    Ok(temp_file)
}

#[cfg(unix)]
fn cleanup_pre_rename(
    parent_fd: &impl AsFd,
    temp_name: &Path,
    target_path: &Path,
    dedup_key: &str,
    recovery_dir: &Path,
    sentinel_name: &str,
) -> Result<(), vault_sync::VaultSyncError> {
    let _ = fs_safety::unlinkat_parent_fd(parent_fd, temp_name);
    let _ = vault_sync::remove_write_dedup(dedup_key);
    let _ = vault_sync::forget_self_write_path(target_path);
    let _ = remove_recovery_sentinel(recovery_dir, sentinel_name);
    Ok(())
}

#[cfg(unix)]
fn remove_recovery_sentinel(
    recovery_dir: &Path,
    sentinel_name: &str,
) -> Result<(), vault_sync::VaultSyncError> {
    let recovery_fd = fs_safety::open_root_fd(recovery_dir)?;
    let _ = fs_safety::unlinkat_parent_fd(&recovery_fd, Path::new(sentinel_name));
    let _ = sync_fd(&recovery_fd);
    Ok(())
}

#[cfg(unix)]
fn handle_post_rename_failure(
    db: &Connection,
    prepared: &PreparedPut,
    relative_path: &str,
    sentinel_path: &Path,
    target_path: &Path,
    dedup_key: &str,
    stage: &'static str,
    reason: String,
) -> vault_sync::VaultSyncError {
    let _ = vault_sync::remove_write_dedup(dedup_key);
    let _ = vault_sync::forget_self_write_path(target_path);
    let _ = vault_sync::mark_collection_needs_full_sync_via_fresh_connection(
        db,
        prepared.collection_id,
    );
    vault_sync::VaultSyncError::PostRenameRecoveryPending {
        collection_id: prepared.collection_id,
        relative_path: relative_path.to_owned(),
        sentinel_path: sentinel_path.display().to_string(),
        stage,
        reason,
    }
}

#[cfg(unix)]
fn sync_fd<Fd: AsFd>(fd: Fd) -> io::Result<()> {
    fsync(fd).map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))
}

#[cfg(not(test))]
#[cfg(unix)]
fn test_hooks_snapshot() -> Option<PutTestHooks> {
    None
}

#[cfg(all(test, unix))]
fn test_hooks_snapshot() -> Option<PutTestHooks> {
    put_test_hooks().lock().unwrap().clone()
}

#[cfg(all(test, unix))]
fn put_test_hooks() -> &'static std::sync::Mutex<PutTestHooks> {
    static HOOKS: std::sync::OnceLock<std::sync::Mutex<PutTestHooks>> = std::sync::OnceLock::new();
    HOOKS.get_or_init(|| std::sync::Mutex::new(PutTestHooks::default()))
}

#[cfg(not(all(test, unix)))]
fn maybe_block_inside_write_lock() {}

#[cfg(all(test, unix))]
#[derive(Debug, Default)]
struct WriteLockBlockState {
    blocked_once: bool,
    entered: bool,
    release: bool,
}

#[cfg(all(test, unix))]
fn write_lock_blocker() -> &'static (std::sync::Mutex<WriteLockBlockState>, std::sync::Condvar) {
    static BLOCKER: std::sync::OnceLock<(
        std::sync::Mutex<WriteLockBlockState>,
        std::sync::Condvar,
    )> = std::sync::OnceLock::new();
    BLOCKER.get_or_init(|| {
        (
            std::sync::Mutex::new(WriteLockBlockState::default()),
            std::sync::Condvar::new(),
        )
    })
}

#[cfg(all(test, unix))]
fn maybe_block_inside_write_lock() {
    let Some(hooks) = test_hooks_snapshot() else {
        return;
    };
    if !hooks.block_inside_slug_lock {
        return;
    }
    let (state_lock, wakeup) = write_lock_blocker();
    let mut state = state_lock.lock().unwrap();
    if state.blocked_once {
        return;
    }
    state.blocked_once = true;
    state.entered = true;
    wakeup.notify_all();
    while !state.release {
        state = wakeup.wait(state).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;
    use crate::core::markdown;
    use std::collections::HashMap;
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    #[cfg(unix)]
    fn open_test_db_with_vault() -> (tempfile::TempDir, String, Connection, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        conn.busy_timeout(Duration::from_millis(0)).unwrap();
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();
        conn.execute(
            "UPDATE collections
             SET root_path = ?1,
                 writable = 1,
                 is_write_target = 1,
                 state = 'active',
                 needs_full_sync = 0
             WHERE id = 1",
            [vault_root.display().to_string()],
        )
        .unwrap();
        (dir, db_path.display().to_string(), conn, vault_root)
    }

    #[cfg(unix)]
    fn recovery_sentinel_count(db_path: &str, collection_id: i64) -> usize {
        let recovery_root = vault_sync::recovery_root_for_db_path(Path::new(db_path));
        std::fs::read_dir(vault_sync::collection_recovery_dir(
            &recovery_root,
            collection_id,
        ))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".needs_full_sync")
                })
                .count()
        })
        .unwrap_or(0)
    }

    #[cfg(unix)]
    fn collection_needs_full_sync(conn: &Connection, collection_id: i64) -> i64 {
        conn.query_row(
            "SELECT needs_full_sync FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn page_count(conn: &Connection, slug: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM pages WHERE slug = ?1",
            [slug],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn hook_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[cfg(unix)]
    struct HookGuard {
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    #[cfg(unix)]
    impl HookGuard {
        fn install(hooks: PutTestHooks) -> Self {
            let guard = hook_test_lock().lock().unwrap();
            *put_test_hooks().lock().unwrap() = hooks;
            Self { _guard: guard }
        }
    }

    #[cfg(unix)]
    impl Drop for HookGuard {
        fn drop(&mut self) {
            *put_test_hooks().lock().unwrap() = PutTestHooks::default();
        }
    }

    #[cfg(unix)]
    fn reset_write_lock_blocker() {
        let (state_lock, _) = write_lock_blocker();
        *state_lock.lock().unwrap() = WriteLockBlockState::default();
    }

    #[cfg(unix)]
    fn wait_for_write_lock_entry() {
        let (state_lock, wakeup) = write_lock_blocker();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = state_lock.lock().unwrap();
        while !state.entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for first writer to enter slug lock"
            );
            let (next_state, timeout) = wakeup.wait_timeout(state, remaining).unwrap();
            state = next_state;
            assert!(
                !timeout.timed_out() || state.entered,
                "timed out waiting for first writer to enter slug lock"
            );
        }
    }

    #[cfg(unix)]
    fn release_write_lock_blocker() {
        let (state_lock, wakeup) = write_lock_blocker();
        let mut state = state_lock.lock().unwrap();
        state.release = true;
        wakeup.notify_all();
    }

    fn active_raw_import_count_for_slug(conn: &Connection, slug: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM raw_imports \
             WHERE page_id = (SELECT id FROM pages WHERE slug = ?1) AND is_active = 1",
            [slug],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn active_raw_import_bytes_for_slug(conn: &Connection, slug: &str) -> Vec<u8> {
        conn.query_row(
            "SELECT raw_bytes FROM raw_imports \
             WHERE page_id = (SELECT id FROM pages WHERE slug = ?1) AND is_active = 1",
            [slug],
            |row| row.get(0),
        )
        .unwrap()
    }

    /// Helper: read a page back from the database.
    fn read_page(conn: &Connection, slug: &str) -> Option<(i64, String, String, String, String)> {
        conn.prepare(
            "SELECT version, type, title, compiled_truth, timeline FROM pages WHERE slug = ?1",
        )
        .unwrap()
        .query_row([slug], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .ok()
    }

    // ── create ─────────────────────────────────────────────────

    #[test]
    fn create_page_sets_version_to_1() {
        let conn = open_test_db();
        let md = "---\ntitle: Alice\ntype: person\n---\n# Alice\n\nAlice is an operator.\n---\n2024-01-01: Joined Acme.\n";

        put_from_string(&conn, "people/alice", md, None).unwrap();

        let (version, page_type, title, truth, timeline) =
            read_page(&conn, "people/alice").unwrap();
        assert_eq!(version, 1);
        assert_eq!(page_type, "person");
        assert_eq!(title, "Alice");
        assert!(truth.contains("Alice is an operator"));
        assert!(timeline.contains("Joined Acme"));
    }

    #[test]
    fn create_page_derives_wing_from_slug() {
        let conn = open_test_db();
        let md = "---\ntitle: Alice\ntype: person\n---\nContent.\n";

        put_from_string(&conn, "people/alice-jones", md, None).unwrap();

        let wing: String = conn
            .query_row(
                "SELECT wing FROM pages WHERE slug = ?1",
                ["people/alice-jones"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(wing, "people");
    }

    #[test]
    fn create_page_defaults_type_to_concept_when_missing() {
        let conn = open_test_db();
        let md = "---\ntitle: Readme\n---\nJust a concept.\n";

        put_from_string(&conn, "readme", md, None).unwrap();

        let (_, page_type, _, _, _) = read_page(&conn, "readme").unwrap();
        assert_eq!(page_type, "concept");
    }

    // ── update with OCC ───────────────────────────────────────

    #[test]
    fn update_with_correct_expected_version_bumps_version() {
        let conn = open_test_db();
        let md1 = "---\ntitle: Alice\ntype: person\n---\nOriginal.\n";
        put_from_string(&conn, "people/alice", md1, None).unwrap();

        let md2 = "---\ntitle: Alice\ntype: person\n---\nUpdated.\n";
        put_from_string(&conn, "people/alice", md2, Some(1)).unwrap();

        let (version, _, _, truth, _) = read_page(&conn, "people/alice").unwrap();
        assert_eq!(version, 2);
        assert!(truth.contains("Updated"));
    }

    #[test]
    fn update_without_gbrain_id_frontmatter_keeps_existing_page_uuid() {
        let conn = open_test_db();
        let original = "---\ngbrain_id: 01969f11-9448-7d79-8d3f-c68f54761234\ntitle: Alice\ntype: person\n---\nOriginal.\n";
        put_from_string(&conn, "people/alice", original, None).unwrap();

        let updated = "---\ntitle: Alice\ntype: person\n---\nUpdated.\n";
        put_from_string(&conn, "people/alice", updated, Some(1)).unwrap();

        let stored_uuid: String = conn
            .query_row(
                "SELECT uuid FROM pages WHERE slug = ?1",
                ["people/alice"],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stored_uuid, "01969f11-9448-7d79-8d3f-c68f54761234");
    }

    #[test]
    fn update_with_stale_expected_version_returns_conflict_error() {
        let conn = open_test_db();
        let md = "---\ntitle: Alice\ntype: person\n---\nContent.\n";
        put_from_string(&conn, "people/alice", md, None).unwrap();

        // Simulate a concurrent update by bumping version directly.
        conn.execute(
            "UPDATE pages SET version = 2, updated_at = '2099-01-01T00:00:00Z' WHERE slug = 'people/alice'",
            [],
        )
        .unwrap();

        let md2 = "---\ntitle: Alice\ntype: person\n---\nStale update.\n";
        let result = put_from_string(&conn, "people/alice", md2, Some(1));

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Conflict"));
        assert!(err.contains("current version: 2"));
    }

    #[test]
    fn update_with_stale_expected_version_leaves_existing_page_body_unchanged() {
        let conn = open_test_db();
        let original = "---\ntitle: Alice\ntype: person\n---\nOriginal body.\n";
        put_from_string(&conn, "people/alice", original, None).unwrap();

        conn.execute(
            "UPDATE pages SET version = 2, compiled_truth = 'Concurrent body' WHERE slug = 'people/alice'",
            [],
        )
        .unwrap();

        let stale = "---\ntitle: Alice\ntype: person\n---\nStale body.\n";
        let result = put_from_string(&conn, "people/alice", stale, Some(1));
        assert!(result.is_err());

        let (version, _, _, truth, _) = read_page(&conn, "people/alice").unwrap();
        assert_eq!(version, 2);
        assert_eq!(truth, "Concurrent body");
    }

    #[test]
    fn put_occ_update_keeps_exactly_one_active_raw_import_row_for_latest_bytes() {
        let conn = open_test_db();
        let original = "---\ntitle: Alice\ntype: person\n---\nOriginal body.\n";
        put_from_string(&conn, "people/alice", original, None).unwrap();

        let updated = "---\ntitle: Alice\ntype: person\n---\nUpdated body.\n";
        put_from_string(&conn, "people/alice", updated, Some(1)).unwrap();

        assert_eq!(active_raw_import_count_for_slug(&conn, "people/alice"), 1);
        assert_eq!(
            active_raw_import_bytes_for_slug(&conn, "people/alice"),
            updated.as_bytes()
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_update_without_expected_version_conflicts_before_sentinel_creation() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        put_from_string(
            &conn,
            "notes/missing-expected",
            "---\ntitle: Missing Expected\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();

        let error = put_from_string(
            &conn,
            "notes/missing-expected",
            "---\ntitle: Missing Expected\ntype: note\n---\nNew body\n",
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConflictError"));
        assert!(error.to_string().contains("MissingExpectedVersion"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            std::fs::read_to_string(vault_root.join("notes").join("missing-expected.md")).unwrap(),
            "---\ntitle: Missing Expected\ntype: note\n---\nOld body\n"
        );
        assert_eq!(
            read_page(&conn, "notes/missing-expected").unwrap().3,
            "Old body"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_stale_expected_version_conflicts_before_sentinel_creation() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        put_from_string(
            &conn,
            "notes/stale-expected",
            "---\ntitle: Stale Expected\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();

        let error = put_from_string(
            &conn,
            "notes/stale-expected",
            "---\ntitle: Stale Expected\ntype: note\n---\nNew body\n",
            Some(0),
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConflictError"));
        assert!(error.to_string().contains("StaleExpectedVersion"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            std::fs::read_to_string(vault_root.join("notes").join("stale-expected.md")).unwrap(),
            "---\ntitle: Stale Expected\ntype: note\n---\nOld body\n"
        );
        assert_eq!(read_page(&conn, "notes/stale-expected").unwrap().0, 1);
    }

    #[cfg(unix)]
    #[test]
    fn unix_external_delete_conflicts_before_sentinel_creation() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        put_from_string(
            &conn,
            "notes/external-delete",
            "---\ntitle: External Delete\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        std::fs::remove_file(vault_root.join("notes").join("external-delete.md")).unwrap();

        let error = put_from_string(
            &conn,
            "notes/external-delete",
            "---\ntitle: External Delete\ntype: note\n---\nNew body\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConflictError"));
        assert!(error.to_string().contains("ExternalDelete"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(page_count(&conn, "notes/external-delete"), 1);
        assert!(!vault_root.join("notes").join("external-delete.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn unix_external_create_conflicts_before_sentinel_creation() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        let target = vault_root.join("notes").join("external-create.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"external bytes").unwrap();

        let error = put_from_string(
            &conn,
            "notes/external-create",
            "---\ntitle: External Create\ntype: note\n---\nBody\n",
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConflictError"));
        assert!(error.to_string().contains("ExternalCreate"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(page_count(&conn, "notes/external-create"), 0);
        assert_eq!(std::fs::read(&target).unwrap(), b"external bytes");
    }

    #[cfg(unix)]
    #[test]
    fn unix_fresh_create_succeeds_without_existing_file_state() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        let body = "---\ntitle: Fresh Create\ntype: note\n---\nBody\n";

        put_from_string(&conn, "notes/fresh-create", body, None).unwrap();

        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            std::fs::read_to_string(vault_root.join("notes").join("fresh-create.md")).unwrap(),
            body
        );
        assert_eq!(read_page(&conn, "notes/fresh-create").unwrap().0, 1);
    }

    #[cfg(unix)]
    #[test]
    fn same_slug_writes_wait_for_per_slug_mutex() {
        let (_dir, db_path, conn, _vault_root) = open_test_db_with_vault();
        put_from_string(
            &conn,
            "notes/serialized",
            "---\ntitle: Serialized\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        reset_write_lock_blocker();
        let _guard = HookGuard::install(PutTestHooks {
            block_inside_slug_lock: true,
            ..PutTestHooks::default()
        });

        let first_db_path = db_path.clone();
        let first = std::thread::spawn(move || {
            let conn = Connection::open(&first_db_path).unwrap();
            conn.busy_timeout(Duration::from_millis(0)).unwrap();
            put_from_string(
                &conn,
                "notes/serialized",
                "---\ntitle: Serialized\ntype: note\n---\nFirst writer body\n",
                Some(1),
            )
        });
        wait_for_write_lock_entry();

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let second_db_path = db_path.clone();
        let second = std::thread::spawn(move || {
            let conn = Connection::open(&second_db_path).unwrap();
            conn.busy_timeout(Duration::from_millis(0)).unwrap();
            let result = put_from_string(
                &conn,
                "notes/serialized",
                "---\ntitle: Serialized\ntype: note\n---\nSecond writer body\n",
                Some(1),
            );
            done_tx.send(result.is_ok()).unwrap();
            result
        });

        assert!(
            done_rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "same-slug second writer should stay blocked until the first writer releases the slug mutex"
        );

        release_write_lock_blocker();

        first.join().unwrap().unwrap();
        let error = second.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("StaleExpectedVersion"));
        let (version, _, _, truth, _) = read_page(&conn, "notes/serialized").unwrap();
        assert_eq!(version, 2);
        assert_eq!(truth, "First writer body");
    }

    #[cfg(unix)]
    #[test]
    fn different_slug_writes_do_not_share_per_slug_mutex() {
        let (_dir, db_path, conn, _vault_root) = open_test_db_with_vault();
        reset_write_lock_blocker();
        let _guard = HookGuard::install(PutTestHooks {
            block_inside_slug_lock: true,
            ..PutTestHooks::default()
        });

        let blocked_db_path = db_path.clone();
        let blocked = std::thread::spawn(move || {
            let conn = Connection::open(&blocked_db_path).unwrap();
            conn.busy_timeout(Duration::from_millis(0)).unwrap();
            put_from_string(
                &conn,
                "notes/alpha",
                "---\ntitle: Alpha\ntype: note\n---\nAlpha body\n",
                None,
            )
        });
        wait_for_write_lock_entry();

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let free_db_path = db_path.clone();
        let free = std::thread::spawn(move || {
            let conn = Connection::open(&free_db_path).unwrap();
            conn.busy_timeout(Duration::from_millis(0)).unwrap();
            let result = put_from_string(
                &conn,
                "notes/beta",
                "---\ntitle: Beta\ntype: note\n---\nBeta body\n",
                None,
            );
            done_tx.send(result.is_ok()).unwrap();
            result
        });

        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            true,
            "different-slug writer should not wait on the blocked slug mutex"
        );

        release_write_lock_blocker();

        blocked.join().unwrap().unwrap();
        free.join().unwrap().unwrap();
        assert_eq!(read_page(&conn, "notes/alpha").unwrap().3, "Alpha body");
        assert_eq!(read_page(&conn, "notes/beta").unwrap().3, "Beta body");
    }

    #[cfg(unix)]
    #[test]
    fn sentinel_creation_failure_returns_recovery_sentinel_error_without_db_or_vault_mutation() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        let _guard = HookGuard::install(PutTestHooks {
            fail_sentinel_create: true,
            ..PutTestHooks::default()
        });

        let error = put_from_string(
            &conn,
            "notes/sentinel-failure",
            "---\ntitle: Sentinel Failure\ntype: note\n---\nBody\n",
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("RecoverySentinelError"));
        assert_eq!(page_count(&conn, "notes/sentinel-failure"), 0);
        assert!(!vault_root
            .join("notes")
            .join("sentinel-failure.md")
            .exists());
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert!(!vault_sync::has_write_dedup(&format!(
            "1:{}:{}",
            "notes/sentinel-failure.md",
            sha256_hex(b"---\ntitle: Sentinel Failure\ntype: note\n---\nBody\n")
        ))
        .unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn pre_rename_failure_cleans_tempfile_sentinel_and_dedup() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        let _guard = HookGuard::install(PutTestHooks {
            fail_before_rename: true,
            ..PutTestHooks::default()
        });
        let body = "---\ntitle: Pre Rename\ntype: note\n---\nBody\n";
        let dedup_key = format!(
            "1:{}:{}",
            "notes/pre-rename.md",
            sha256_hex(body.as_bytes())
        );

        let error = put_from_string(&conn, "notes/pre-rename", body, None).unwrap_err();

        assert!(error.to_string().contains("injected pre-rename failure"));
        assert_eq!(page_count(&conn, "notes/pre-rename"), 0);
        assert!(!vault_root.join("notes").join("pre-rename.md").exists());
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert!(!vault_sync::has_write_dedup(&dedup_key).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn rename_failure_cleans_tempfile_sentinel_and_dedup() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        let _guard = HookGuard::install(PutTestHooks {
            fail_rename: true,
            ..PutTestHooks::default()
        });
        let body = "---\ntitle: Rename Failure\ntype: note\n---\nBody\n";
        let dedup_key = format!(
            "1:{}:{}",
            "notes/rename-failure.md",
            sha256_hex(body.as_bytes())
        );

        let error = put_from_string(&conn, "notes/rename-failure", body, None).unwrap_err();

        assert!(error.to_string().contains("injected rename failure"));
        assert_eq!(page_count(&conn, "notes/rename-failure"), 0);
        assert!(!vault_root.join("notes").join("rename-failure.md").exists());
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert!(!vault_sync::has_write_dedup(&dedup_key).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn parent_fsync_failure_refuses_db_commit_and_retains_sentinel() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        put_from_string(
            &conn,
            "notes/fsync-parent",
            "---\ntitle: Parent Fsync\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        let _guard = HookGuard::install(PutTestHooks {
            fail_parent_fsync: true,
            ..PutTestHooks::default()
        });

        let error = put_from_string(
            &conn,
            "notes/fsync-parent",
            "---\ntitle: Parent Fsync\ntype: note\n---\nNew body\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("PostRenameRecoveryPendingError"));
        assert!(error.to_string().contains("stage=fsync-parent"));
        assert_eq!(collection_needs_full_sync(&conn, 1), 1);
        assert_eq!(
            read_page(&conn, "notes/fsync-parent").unwrap().3,
            "Old body"
        );
        assert_eq!(recovery_sentinel_count(&db_path, 1), 1);
        assert!(vault_root.join("notes").join("fsync-parent.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn foreign_rename_returns_concurrent_rename_and_retains_sentinel() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        put_from_string(
            &conn,
            "notes/concurrent",
            "---\ntitle: Concurrent\ntype: note\n---\nOriginal body\n",
            None,
        )
        .unwrap();
        let _guard = HookGuard::install(PutTestHooks {
            post_rename_swap: Some(
                b"---\ntitle: Concurrent\ntype: note\n---\nForeign body\n".to_vec(),
            ),
            ..PutTestHooks::default()
        });

        let error = put_from_string(
            &conn,
            "notes/concurrent",
            "---\ntitle: Concurrent\ntype: note\n---\nLocal body\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConcurrentRenameError"));
        assert_eq!(collection_needs_full_sync(&conn, 1), 1);
        assert_eq!(recovery_sentinel_count(&db_path, 1), 1);
        assert!(vault_root.join("notes").join("concurrent.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn commit_busy_retains_sentinel_until_startup_recovery_reconciles() {
        let (_dir, db_path, conn, _vault_root) = open_test_db_with_vault();
        put_from_string(
            &conn,
            "notes/busy",
            "---\ntitle: Busy\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        let blocker = Connection::open(&db_path).unwrap();
        blocker.busy_timeout(Duration::from_millis(0)).unwrap();
        blocker
            .execute_batch(
                "BEGIN EXCLUSIVE; UPDATE collections SET updated_at = updated_at WHERE id = 1;",
            )
            .unwrap();

        let error = put_from_string(
            &conn,
            "notes/busy",
            "---\ntitle: Busy\ntype: note\n---\nNew body on disk\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("PostRenameRecoveryPendingError"));
        assert!(error.to_string().contains("stage=commit"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 1);
        drop(blocker);

        let runtime = vault_sync::start_serve_runtime(db_path.clone()).unwrap();
        let recovered = wait_for_recovered_truth(&db_path, "notes/busy", "New body on disk");
        assert!(recovered);
        drop(runtime);

        let verify = Connection::open(&db_path).unwrap();
        assert_eq!(collection_needs_full_sync(&verify, 1), 0);
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
    }

    #[cfg(unix)]
    #[test]
    fn foreign_rename_with_busy_falls_back_to_startup_sentinel_recovery() {
        let (_dir, db_path, conn, _vault_root) = open_test_db_with_vault();
        put_from_string(
            &conn,
            "notes/foreign-busy",
            "---\ntitle: Foreign Busy\ntype: note\n---\nOld body\n",
            None,
        )
        .unwrap();
        let blocker = Connection::open(&db_path).unwrap();
        blocker.busy_timeout(Duration::from_millis(0)).unwrap();
        blocker
            .execute_batch(
                "BEGIN EXCLUSIVE; UPDATE collections SET updated_at = updated_at WHERE id = 1;",
            )
            .unwrap();
        let _guard = HookGuard::install(PutTestHooks {
            post_rename_swap: Some(
                b"---\ntitle: Foreign Busy\ntype: note\n---\nForeign winner body\n".to_vec(),
            ),
            ..PutTestHooks::default()
        });

        let error = put_from_string(
            &conn,
            "notes/foreign-busy",
            "---\ntitle: Foreign Busy\ntype: note\n---\nLocal loser body\n",
            Some(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("ConcurrentRenameError"));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 1);
        drop(blocker);
        drop(_guard);

        let verify_before_runtime = Connection::open(&db_path).unwrap();
        assert_eq!(collection_needs_full_sync(&verify_before_runtime, 1), 0);
        drop(verify_before_runtime);

        let runtime = vault_sync::start_serve_runtime(db_path.clone()).unwrap();
        let recovered =
            wait_for_recovered_truth(&db_path, "notes/foreign-busy", "Foreign winner body");
        assert!(recovered);
        drop(runtime);

        let verify = Connection::open(&db_path).unwrap();
        assert_eq!(collection_needs_full_sync(&verify, 1), 0);
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
    }

    #[cfg(unix)]
    fn wait_for_recovered_truth(db_path: &str, slug: &str, expected: &str) -> bool {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            let verify = Connection::open(db_path).unwrap();
            let outcome: Option<(String, i64)> = verify
                .query_row(
                    "SELECT compiled_truth, version FROM pages WHERE slug = ?1",
                    [slug],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();
            if let Some((truth, version)) = outcome {
                if truth.contains(expected) && version >= 2 {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    // ── unconditional upsert ──────────────────────────────────

    #[cfg(not(unix))]
    #[test]
    fn update_without_expected_version_upserts_unconditionally() {
        let conn = open_test_db();
        let md1 = "---\ntitle: Bob\ntype: person\n---\nOriginal.\n";
        put_from_string(&conn, "people/bob", md1, None).unwrap();

        let md2 = "---\ntitle: Bob\ntype: person\n---\nOverwritten.\n";
        put_from_string(&conn, "people/bob", md2, None).unwrap();

        let (version, _, _, truth, _) = read_page(&conn, "people/bob").unwrap();
        assert_eq!(version, 2);
        assert!(truth.contains("Overwritten"));
    }

    // ── round-trip fidelity ───────────────────────────────────

    #[test]
    fn put_then_get_roundtrips_through_render() {
        let conn = open_test_db();
        let md = "---\ntitle: Carol\ntype: person\n---\n# Carol\n\nCarol builds things.\n---\n2024-06-01: Shipped v1.\n";

        put_from_string(&conn, "people/carol", md, None).unwrap();

        // Read back through get path
        let page = crate::commands::get::get_page(&conn, "people/carol").unwrap();
        let rendered = markdown::render_page(&page);
        assert!(rendered.contains("gbrain_id: "));
        assert!(rendered.contains("title: Carol"));
        assert!(rendered.contains("type: person"));
        assert!(rendered.contains("# Carol\n\nCarol builds things."));
        assert!(rendered.contains("2024-06-01: Shipped v1."));
    }

    #[test]
    fn put_render_cannot_strip_existing_gbrain_id_when_update_omits_it() {
        let conn = open_test_db();
        let original = "---\ngbrain_id: 01969f11-9448-7d79-8d3f-c68f54761234\ntitle: Carol\ntype: person\n---\n# Carol\n\nOriginal.\n";
        put_from_string(&conn, "people/carol", original, None).unwrap();

        let updated = "---\ntitle: Carol\ntype: person\n---\n# Carol\n\nUpdated.\n";
        put_from_string(&conn, "people/carol", updated, Some(1)).unwrap();

        let page = crate::commands::get::get_page(&conn, "people/carol").unwrap();
        let rendered = markdown::render_page(&page);

        assert!(
            rendered.contains("gbrain_id: 01969f11-9448-7d79-8d3f-c68f54761234"),
            "brain_put must not let a UUID-bearing page render back out without gbrain_id"
        );
    }

    // ── frontmatter stored as JSON ────────────────────────────

    #[test]
    fn frontmatter_is_stored_as_json_and_recoverable() {
        let conn = open_test_db();
        let md = "---\nsource: manual\ntitle: Data\ntype: concept\n---\nContent.\n";

        put_from_string(&conn, "data/test", md, None).unwrap();

        let fm_json: String = conn
            .query_row(
                "SELECT frontmatter FROM pages WHERE slug = ?1",
                ["data/test"],
                |row| row.get(0),
            )
            .unwrap();
        let fm: HashMap<String, String> = serde_json::from_str(&fm_json).unwrap();
        assert_eq!(fm.get("source").unwrap(), "manual");
        assert_eq!(fm.get("title").unwrap(), "Data");
        assert_eq!(fm.get("type").unwrap(), "concept");
    }

    // ── FTS5 trigger fires ────────────────────────────────────

    #[test]
    fn insert_triggers_fts5_indexing() {
        let conn = open_test_db();
        let md = "---\ntitle: Searchable\ntype: concept\n---\n# Searchable\n\nUnique searchable keyword xylophone.\n";

        put_from_string(&conn, "test/searchable", md, None).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM page_fts WHERE page_fts MATCH 'xylophone'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
