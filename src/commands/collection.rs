use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use rusqlite::{params, Connection};
use serde::Serialize;
use uuid::Uuid;

use crate::core::collections;
#[cfg(unix)]
use crate::core::fs_safety;
use crate::core::ignore_patterns::{self, ParseResult};
use crate::core::quarantine;
use crate::core::vault_sync;

#[derive(Subcommand, Debug)]
pub enum CollectionAction {
    /// Attach a vault as a collection
    Add(CollectionAddArgs),
    /// List collections
    List,
    /// Show collection diagnostics
    Info { name: String },
    /// Manage .quaidignore patterns
    Ignore {
        #[command(subcommand)]
        action: CollectionIgnoreAction,
    },
    /// Manage quarantined pages
    Quarantine {
        #[command(subcommand)]
        action: CollectionQuarantineAction,
    },
    /// Reconcile a collection or adopt a new root
    Sync(CollectionSyncArgs),
    /// Restore a collection to a target path
    Restore(CollectionRestoreArgs),
    /// Clear restore-integrity blocked state
    #[command(name = "restore-reset")]
    RestoreReset {
        name: String,
        #[arg(long)]
        confirm: bool,
    },
    /// Clear reconcile-halted state after manual repair
    #[command(name = "reconcile-reset")]
    ReconcileReset {
        name: String,
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Args, Debug)]
pub struct CollectionAddArgs {
    pub name: String,
    pub path: PathBuf,
    #[arg(long, conflicts_with = "writable")]
    pub read_only: bool,
    #[arg(long, conflicts_with = "read_only")]
    pub writable: bool,
    #[arg(long)]
    pub write_memory_id: bool,
}

#[derive(Args, Debug)]
pub struct CollectionSyncArgs {
    pub name: String,
    #[arg(long)]
    pub remap_root: Option<PathBuf>,
    #[arg(long)]
    pub finalize_pending: bool,
    #[arg(long)]
    pub online: bool,
}

#[derive(Args, Debug)]
pub struct CollectionRestoreArgs {
    pub name: String,
    pub target: PathBuf,
    #[arg(long)]
    pub online: bool,
}

#[derive(Subcommand, Debug)]
pub enum CollectionIgnoreAction {
    /// Add a user-authored ignore pattern
    Add { name: String, pattern: String },
    /// Remove a user-authored ignore pattern
    Remove { name: String, pattern: String },
    /// List cached user-authored ignore patterns
    List { name: String },
    /// Explicitly clear the ignore file and cached mirror
    Clear {
        name: String,
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum CollectionQuarantineAction {
    /// List quarantined pages for a collection
    List { name: String },
    /// Export a quarantined page's preserved state as JSON
    Export { slug: String, output: PathBuf },
    /// Discard a quarantined page
    Discard {
        slug: String,
        #[arg(long)]
        force: bool,
    },
    /// Restore a quarantined page to a target markdown path (Unix-only)
    Restore { slug: String, relative_path: String },
}

#[derive(Debug, Serialize)]
struct CollectionInfoOutput {
    name: String,
    root_path: String,
    writable: bool,
    writable_mode: String,
    write_target: bool,
    state: String,
    needs_full_sync: bool,
    last_sync_at: Option<String>,
    page_count: i64,
    queue_depth: i64,
    quarantined_pages_awaiting_action: i64,
    ignore_parse_errors: Option<String>,
    pending_root_path: Option<String>,
    restore_command_id: Option<String>,
    restore_command_pid: Option<i64>,
    restore_command_host: Option<String>,
    pending_command_heartbeat_at: Option<String>,
    integrity_failed_at: Option<String>,
    pending_manifest_incomplete_at: Option<String>,
    reconcile_halted_at: Option<String>,
    reconcile_halt_reason: Option<String>,
    reload_generation: i64,
    watcher_released_session_id: Option<String>,
    watcher_released_generation: Option<i64>,
    watcher_mode: Option<String>,
    watcher_last_event_at: Option<String>,
    watcher_channel_depth: Option<i64>,
    blocked_state: String,
    integrity_blocked: Option<String>,
    suggested_command: Option<String>,
    status_message: String,
}

#[derive(Debug, Serialize)]
struct CollectionListRow {
    name: String,
    state: String,
    writable: String,
    write_target: bool,
    root_path: String,
    page_count: i64,
    last_sync_at: Option<String>,
    queue_depth: i64,
}

#[derive(Debug)]
struct CollectionStatusSummary {
    blocked_state: String,
    integrity_blocked: Option<String>,
    suggested_command: Option<String>,
    status_message: String,
}

#[derive(Debug)]
struct CollectionMetrics {
    page_count: i64,
    queue_depth: i64,
    quarantined_pages_awaiting_action: i64,
}

pub fn run(db: &Connection, action: CollectionAction, json: bool) -> Result<()> {
    match action {
        CollectionAction::Add(args) => {
            ensure_unix_collection_command("quaid collection add")?;
            add(db, args, json)
        }
        CollectionAction::List => list(db, json),
        CollectionAction::Info { name } => info(db, &name, json),
        CollectionAction::Ignore { action } => ignore(db, action, json),
        CollectionAction::Quarantine { action } => quarantine_action(db, action, json),
        CollectionAction::Sync(args) => {
            ensure_unix_collection_command("quaid collection sync")?;
            sync(db, args, json)
        }
        CollectionAction::Restore(args) => {
            ensure_unix_collection_command("quaid collection restore")?;
            restore(db, args, json)
        }
        CollectionAction::RestoreReset { name, confirm } => {
            if !confirm {
                bail!("restore-reset requires --confirm");
            }
            vault_sync::restore_reset(db, &name)?;
            render_success(
                json,
                serde_json::json!({ "status": "ok", "command": "restore-reset", "collection": name }),
            )
        }
        CollectionAction::ReconcileReset { name, confirm } => {
            if !confirm {
                bail!("reconcile-reset requires --confirm");
            }
            vault_sync::reconcile_reset(db, &name)?;
            render_success(
                json,
                serde_json::json!({ "status": "ok", "command": "reconcile-reset", "collection": name }),
            )
        }
    }
}

fn ensure_unix_collection_command(command: &'static str) -> Result<()> {
    vault_sync::ensure_unix_platform(command).map_err(|err| anyhow!(err.to_string()))
}

fn add(db: &Connection, args: CollectionAddArgs, json: bool) -> Result<()> {
    collections::validate_collection_name(&args.name).map_err(|err| anyhow!(err.to_string()))?;
    if collections::get_by_name(db, &args.name)?.is_some() {
        bail!("collection already exists: {}", args.name);
    }
    if args.write_memory_id {
        bail!(
            "--write-quaid-id is deferred in Batch K2; K1 only supports default read-only attach"
        );
    }

    let root_path = resolve_collection_root(&args.path)?;
    let ignore_patterns = read_initial_ignore_patterns(&root_path)?;
    let writable = if args.read_only {
        false
    } else {
        probe_root_writable(&root_path)?
    };

    if args.writable && !writable {
        return Err(anyhow!(vault_sync::VaultSyncError::CollectionReadOnly {
            collection_name: args.name.clone(),
        }
        .to_string()));
    }

    let collection_id = {
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO collections (
                 name,
                 root_path,
                 state,
                 writable,
                 is_write_target,
                 ignore_patterns,
                 ignore_parse_errors,
                 needs_full_sync
             )
             VALUES (?1, ?2, 'detached', ?3, 0, ?4, NULL, 1)",
            params![
                args.name,
                root_path.display().to_string(),
                i64::from(writable),
                ignore_patterns,
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        id
    };

    let attach_command_id = Uuid::now_v7().to_string();
    let stats = match vault_sync::fresh_attach_collection(db, collection_id, &attach_command_id) {
        Ok(stats) => stats,
        Err(err) => {
            let _ = db.execute("DELETE FROM collections WHERE id = ?1", [collection_id]);
            return Err(anyhow!(err.to_string()));
        }
    };

    let collection = collections::get_by_name(db, &args.name)?
        .ok_or_else(|| anyhow!("collection not found after attach: {}", args.name))?;
    let metrics = collection_metrics(db, collection.id)?;

    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "add",
            "collection": collection.name,
            "root_path": collection.root_path,
            "state": collection.state.as_str(),
            "writable": collection.writable,
            "writable_mode": writable_label(collection.writable),
            "write_target": collection.is_write_target,
            "page_count": metrics.page_count,
            "queue_depth": metrics.queue_depth,
            "last_sync_at": collection.last_sync_at,
            "attach_command_id": attach_command_id,
            "walked": stats.walked,
            "modified": stats.modified,
            "new": stats.new,
            "missing": stats.missing,
            "uuid_renamed": stats.uuid_renamed,
            "hash_renamed": stats.hash_renamed
        }),
    )
}

fn list(db: &Connection, json: bool) -> Result<()> {
    let mut stmt = db.prepare(
        "SELECT c.id,
                c.name,
                c.state,
                c.writable,
                c.is_write_target,
                c.root_path,
                COALESCE((
                    SELECT COUNT(*)
                    FROM pages p
                    WHERE p.collection_id = c.id AND p.quarantined_at IS NULL
                ), 0),
                c.last_sync_at,
                COALESCE((
                    SELECT COUNT(*)
                    FROM embedding_jobs ej
                    JOIN pages p ON p.id = ej.page_id
                    WHERE p.collection_id = c.id
                ), 0)
         FROM collections c
         WHERE c.root_path <> ''
         ORDER BY c.name",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CollectionListRow {
                name: row.get(1)?,
                state: row.get::<_, String>(2)?,
                writable: writable_label(row.get::<_, i64>(3)? != 0).to_owned(),
                write_target: row.get::<_, i64>(4)? != 0,
                root_path: row.get(5)?,
                page_count: row.get(6)?,
                last_sync_at: row.get(7)?,
                queue_depth: row.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!("name | state | writable | write_target | root_path | page_count | last_sync_at | queue_depth");
        for row in rows {
            println!(
                "{} | {} | {} | {} | {} | {} | {} | {}",
                row.name,
                row.state,
                row.writable,
                row.write_target,
                row.root_path,
                row.page_count,
                row.last_sync_at.as_deref().unwrap_or("-"),
                row.queue_depth
            );
        }
    }
    Ok(())
}

fn info(db: &Connection, name: &str, json: bool) -> Result<()> {
    let collection = load_collection_by_name(db, name)?;
    let output = build_collection_info_output(db, collection)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "collection={} state={} writable={} write_target={} root_path={}",
            output.name, output.state, output.writable_mode, output.write_target, output.root_path
        );
        println!(
            "page_count={} queue_depth={} quarantined_pages_awaiting_action={} needs_full_sync={} last_sync_at={}",
            output.page_count,
            output.queue_depth,
            output.quarantined_pages_awaiting_action,
            output.needs_full_sync,
            output.last_sync_at.as_deref().unwrap_or("null")
        );
        println!(
            "pending_root_path={} integrity_failed_at={} reconcile_halted_at={} ignore_parse_errors={}",
            output.pending_root_path.as_deref().unwrap_or("null"),
            output.integrity_failed_at.as_deref().unwrap_or("null"),
            output.reconcile_halted_at.as_deref().unwrap_or("null"),
            output.ignore_parse_errors.as_deref().unwrap_or("null")
        );
        println!(
            "watcher_mode={} watcher_last_event_at={} watcher_channel_depth={}",
            output.watcher_mode.as_deref().unwrap_or("null"),
            output.watcher_last_event_at.as_deref().unwrap_or("null"),
            output
                .watcher_channel_depth
                .map(|depth| depth.to_string())
                .as_deref()
                .unwrap_or("null")
        );
        println!(
            "blocked_state={} integrity_blocked={} suggested_command={} status_message=\"{}\"",
            output.blocked_state,
            output.integrity_blocked.as_deref().unwrap_or("null"),
            output.suggested_command.as_deref().unwrap_or("null"),
            output.status_message
        );
    }
    Ok(())
}

fn build_collection_info_output(
    db: &Connection,
    collection: collections::Collection,
) -> Result<CollectionInfoOutput> {
    let status = describe_collection_status(&collection);
    let metrics = collection_metrics(db, collection.id)?;
    let watcher_health = matches!(collection.state, collections::CollectionState::Active)
        .then(|| vault_sync::collection_watcher_health(collection.id))
        .flatten();
    Ok(CollectionInfoOutput {
        name: collection.name,
        root_path: collection.root_path,
        writable: collection.writable,
        writable_mode: writable_label(collection.writable).to_owned(),
        write_target: collection.is_write_target,
        state: collection.state.as_str().to_owned(),
        needs_full_sync: collection.needs_full_sync,
        last_sync_at: collection.last_sync_at,
        page_count: metrics.page_count,
        queue_depth: metrics.queue_depth,
        quarantined_pages_awaiting_action: metrics.quarantined_pages_awaiting_action,
        ignore_parse_errors: collection.ignore_parse_errors,
        pending_root_path: collection.pending_root_path,
        restore_command_id: collection.restore_command_id,
        restore_command_pid: collection.restore_command_pid,
        restore_command_host: collection.restore_command_host,
        pending_command_heartbeat_at: collection.pending_command_heartbeat_at,
        integrity_failed_at: collection.integrity_failed_at,
        pending_manifest_incomplete_at: collection.pending_manifest_incomplete_at,
        reconcile_halted_at: collection.reconcile_halted_at,
        reconcile_halt_reason: collection.reconcile_halt_reason,
        reload_generation: collection.reload_generation,
        watcher_released_session_id: collection.watcher_released_session_id,
        watcher_released_generation: collection.watcher_released_generation,
        watcher_mode: watcher_health.as_ref().map(|health| health.mode.clone()),
        watcher_last_event_at: watcher_health
            .as_ref()
            .and_then(|health| health.last_event_at.clone()),
        watcher_channel_depth: watcher_health.as_ref().map(|health| health.channel_depth),
        blocked_state: status.blocked_state,
        integrity_blocked: status.integrity_blocked,
        suggested_command: status.suggested_command,
        status_message: status.status_message,
    })
}

fn ignore(db: &Connection, action: CollectionIgnoreAction, json: bool) -> Result<()> {
    match action {
        CollectionIgnoreAction::Add { name, pattern } => {
            ensure_unix_collection_command("quaid collection ignore add")?;
            ignore_add(db, &name, &pattern, json)
        }
        CollectionIgnoreAction::Remove { name, pattern } => {
            ensure_unix_collection_command("quaid collection ignore remove")?;
            ignore_remove(db, &name, &pattern, json)
        }
        CollectionIgnoreAction::List { name } => ignore_list(db, &name, json),
        CollectionIgnoreAction::Clear { name, confirm } => {
            ensure_unix_collection_command("quaid collection ignore clear")?;
            if !confirm {
                bail!("ignore clear requires --confirm");
            }
            ignore_clear(db, &name, json)
        }
    }
}

fn quarantine_action(
    db: &Connection,
    action: CollectionQuarantineAction,
    json: bool,
) -> Result<()> {
    match action {
        CollectionQuarantineAction::List { name } => quarantine_list(db, &name, json),
        CollectionQuarantineAction::Export { slug, output } => {
            quarantine_export(db, &slug, &output, json)
        }
        CollectionQuarantineAction::Discard { slug, force } => {
            quarantine_discard(db, &slug, force, json)
        }
        CollectionQuarantineAction::Restore {
            slug,
            relative_path,
        } => {
            ensure_unix_collection_command("quaid collection quarantine restore")?;
            quarantine_restore(db, &slug, &relative_path, json)
        }
    }
}

fn load_collection_by_name(db: &Connection, name: &str) -> Result<collections::Collection> {
    collections::get_by_name(db, name)?.ok_or_else(|| anyhow!("collection not found: {name}"))
}

fn cached_user_ignore_patterns(collection: &collections::Collection) -> Result<Vec<String>> {
    match &collection.ignore_patterns {
        Some(json) => serde_json::from_str(json)
            .with_context(|| format!("invalid ignore pattern mirror for {}", collection.name)),
        None => Ok(Vec::new()),
    }
}

fn ignore_file_path(collection: &collections::Collection) -> PathBuf {
    Path::new(&collection.root_path).join(".quaidignore")
}

fn load_ignore_source(collection: &collections::Collection) -> Result<Option<String>> {
    let ignore_path = ignore_file_path(collection);
    match fs::read_to_string(&ignore_path) {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let patterns = cached_user_ignore_patterns(collection)?;
            if patterns.is_empty() {
                Ok(None)
            } else {
                Ok(Some(render_patterns_file(&patterns)))
            }
        }
        Err(err) => Err(anyhow!("failed to read {}: {}", ignore_path.display(), err)),
    }
}

fn render_patterns_file(patterns: &[String]) -> String {
    if patterns.is_empty() {
        String::new()
    } else {
        let mut rendered = patterns.join("\n");
        rendered.push('\n');
        rendered
    }
}

fn ignore_content_contains_pattern(content: &str, pattern: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed == pattern
    })
}

fn add_ignore_pattern_content(current: Option<String>, pattern: &str) -> String {
    let mut content = current.unwrap_or_default();
    if ignore_content_contains_pattern(&content, pattern) {
        return content;
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(pattern);
    content.push('\n');
    content
}

fn remove_ignore_pattern_content(current: Option<String>, pattern: &str) -> String {
    let Some(content) = current else {
        return String::new();
    };
    let had_trailing_newline = content.ends_with('\n');
    let remaining = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed.is_empty() || trimmed.starts_with('#') || trimmed != pattern
        })
        .collect::<Vec<_>>();
    if remaining.is_empty() {
        String::new()
    } else {
        let mut rendered = remaining.join("\n");
        if had_trailing_newline {
            rendered.push('\n');
        }
        rendered
    }
}

fn validate_cli_ignore_pattern(pattern: &str) -> Result<String> {
    let trimmed = pattern.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        bail!("ignore pattern must be a non-empty glob");
    }
    validate_proposed_ignore_content(&format!("{trimmed}\n"))?;
    Ok(trimmed.to_owned())
}

fn validate_proposed_ignore_content(content: &str) -> Result<()> {
    match ignore_patterns::parse_ignore_file(content) {
        ParseResult::Valid(_) => Ok(()),
        ParseResult::Invalid(errors) => bail!(format_ignore_parse_errors(&errors)),
    }
}

fn format_ignore_parse_errors(errors: &[ignore_patterns::IgnoreParseError]) -> String {
    errors
        .iter()
        .map(|error| {
            format!(
                "line {} raw={} error={}",
                error.line,
                serde_json::to_string(&error.raw).unwrap_or_else(|_| "\"\"".to_owned()),
                error.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_ignore_file_atomically(root_path: &Path, contents: Option<&str>) -> Result<PathBuf> {
    let ignore_path = root_path.join(".quaidignore");
    match contents {
        Some(contents) => {
            let temp_path = root_path.join(format!(".quaidignore.tmp-{}", Uuid::now_v7()));
            {
                let mut file = fs::File::create(&temp_path).with_context(|| {
                    format!(
                        "failed to create temporary ignore file at {}",
                        temp_path.display()
                    )
                })?;
                file.write_all(contents.as_bytes()).with_context(|| {
                    format!(
                        "failed to write temporary ignore file at {}",
                        temp_path.display()
                    )
                })?;
                file.sync_all().with_context(|| {
                    format!(
                        "failed to flush temporary ignore file at {}",
                        temp_path.display()
                    )
                })?;
            }
            fs::rename(&temp_path, &ignore_path).with_context(|| {
                let _ = fs::remove_file(&temp_path);
                format!("failed to move {} into place", temp_path.display())
            })?;
        }
        None => {
            if ignore_path.exists() {
                fs::remove_file(&ignore_path)
                    .with_context(|| format!("failed to remove {}", ignore_path.display()))?;
            }
        }
    }
    Ok(ignore_path)
}

fn refresh_ignore_mirror(
    db: &Connection,
    collection: &collections::Collection,
    explicit_clear: bool,
) -> Result<()> {
    let root_path = Path::new(&collection.root_path);
    let result = if explicit_clear {
        ignore_patterns::clear_patterns(db, collection.id, root_path)
    } else {
        ignore_patterns::reload_patterns(db, collection.id, root_path)
    };
    result.map_err(|err| anyhow!(err.to_string()))
}

fn reconcile_after_ignore_change(
    db: &Connection,
    collection: &collections::Collection,
) -> Result<crate::core::reconciler::ReconcileStats> {
    if collection.state == collections::CollectionState::Active {
        return vault_sync::sync_collection(db, &collection.name)
            .map_err(|err| anyhow!(err.to_string()));
    }
    Ok(crate::core::reconciler::ReconcileStats::default())
}

fn ignore_add(db: &Connection, name: &str, pattern: &str, json: bool) -> Result<()> {
    let pattern = validate_cli_ignore_pattern(pattern)?;
    let collection = load_collection_by_name(db, name)?;
    vault_sync::ensure_collection_write_allowed(db, collection.id)
        .map_err(|err| anyhow!(err.to_string()))?;
    let proposed = add_ignore_pattern_content(load_ignore_source(&collection)?, &pattern);
    validate_proposed_ignore_content(&proposed)?;
    let ignore_path =
        write_ignore_file_atomically(Path::new(&collection.root_path), Some(&proposed))?;
    refresh_ignore_mirror(db, &collection, false)?;
    let stats = reconcile_after_ignore_change(db, &collection)?;
    let updated = load_collection_by_name(db, name)?;
    let patterns = cached_user_ignore_patterns(&updated)?;

    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "ignore-add",
            "collection": name,
            "pattern": pattern,
            "file_path": ignore_path.display().to_string(),
            "patterns": patterns,
            "walked": stats.walked,
            "modified": stats.modified,
            "new": stats.new,
            "missing": stats.missing,
            "uuid_renamed": stats.uuid_renamed,
            "hash_renamed": stats.hash_renamed
        }),
    )
}

fn ignore_remove(db: &Connection, name: &str, pattern: &str, json: bool) -> Result<()> {
    let pattern = validate_cli_ignore_pattern(pattern)?;
    let collection = load_collection_by_name(db, name)?;
    vault_sync::ensure_collection_write_allowed(db, collection.id)
        .map_err(|err| anyhow!(err.to_string()))?;
    let proposed = remove_ignore_pattern_content(load_ignore_source(&collection)?, &pattern);
    validate_proposed_ignore_content(&proposed)?;
    let ignore_path =
        write_ignore_file_atomically(Path::new(&collection.root_path), Some(&proposed))?;
    refresh_ignore_mirror(db, &collection, false)?;
    let stats = reconcile_after_ignore_change(db, &collection)?;
    let updated = load_collection_by_name(db, name)?;
    let patterns = cached_user_ignore_patterns(&updated)?;

    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "ignore-remove",
            "collection": name,
            "pattern": pattern,
            "file_path": ignore_path.display().to_string(),
            "patterns": patterns,
            "walked": stats.walked,
            "modified": stats.modified,
            "new": stats.new,
            "missing": stats.missing,
            "uuid_renamed": stats.uuid_renamed,
            "hash_renamed": stats.hash_renamed
        }),
    )
}

fn ignore_list(db: &Connection, name: &str, json: bool) -> Result<()> {
    let collection = load_collection_by_name(db, name)?;
    let patterns = cached_user_ignore_patterns(&collection)?;
    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "ignore-list",
            "collection": name,
            "patterns": patterns,
            "file_present": ignore_file_path(&collection).exists()
        }),
    )
}

fn ignore_clear(db: &Connection, name: &str, json: bool) -> Result<()> {
    let collection = load_collection_by_name(db, name)?;
    vault_sync::ensure_collection_write_allowed(db, collection.id)
        .map_err(|err| anyhow!(err.to_string()))?;
    let ignore_path = write_ignore_file_atomically(Path::new(&collection.root_path), None)?;
    refresh_ignore_mirror(db, &collection, true)?;
    let stats = reconcile_after_ignore_change(db, &collection)?;
    let updated = load_collection_by_name(db, name)?;
    let patterns = cached_user_ignore_patterns(&updated)?;

    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "ignore-clear",
            "collection": name,
            "file_path": ignore_path.display().to_string(),
            "patterns": patterns,
            "walked": stats.walked,
            "modified": stats.modified,
            "new": stats.new,
            "missing": stats.missing,
            "uuid_renamed": stats.uuid_renamed,
            "hash_renamed": stats.hash_renamed
        }),
    )
}

fn quarantine_list(db: &Connection, name: &str, json: bool) -> Result<()> {
    let pages =
        quarantine::list_collection_quarantine(db, name).map_err(|err| anyhow!(err.to_string()))?;
    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "quarantine-list",
            "collection": name,
            "pages": pages
        }),
    )
}

fn quarantine_export(db: &Connection, slug: &str, output: &Path, json: bool) -> Result<()> {
    let receipt = quarantine::export_quarantined_page(db, slug, output)
        .map_err(|err| anyhow!(err.to_string()))?;
    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "quarantine-export",
            "collection": receipt.collection,
            "slug": receipt.slug,
            "quarantined_at": receipt.quarantined_at,
            "exported_at": receipt.exported_at,
            "output_path": receipt.output_path
        }),
    )
}

fn quarantine_discard(db: &Connection, slug: &str, force: bool, json: bool) -> Result<()> {
    let receipt = quarantine::discard_quarantined_page(db, slug, force)
        .map_err(|err| anyhow!(err.to_string()))?;
    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "quarantine-discard",
            "collection": receipt.collection,
            "slug": receipt.slug,
            "quarantined_at": receipt.quarantined_at,
            "force": receipt.forced,
            "exported_before_discard": receipt.exported_before_discard
        }),
    )
}

fn quarantine_restore(db: &Connection, slug: &str, relative_path: &str, json: bool) -> Result<()> {
    let receipt = quarantine::restore_quarantined_page(db, slug, relative_path)
        .map_err(|err| anyhow!(err.to_string()))?;
    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "quarantine-restore",
            "collection": receipt.collection,
            "slug": receipt.slug,
            "restored_slug": receipt.restored_slug,
            "restored_relative_path": receipt.restored_relative_path,
            "quarantined_at": receipt.quarantined_at
        }),
    )
}

fn collection_metrics(db: &Connection, collection_id: i64) -> Result<CollectionMetrics> {
    db.query_row(
        "SELECT COALESCE((
                 SELECT COUNT(*)
                 FROM pages
                 WHERE collection_id = ?1 AND quarantined_at IS NULL
             ), 0),
                COALESCE((
                    SELECT COUNT(*)
                    FROM pages
                    WHERE collection_id = ?1 AND quarantined_at IS NOT NULL
                ), 0),
                COALESCE((
                    SELECT COUNT(*)
                    FROM embedding_jobs ej
                    JOIN pages p ON p.id = ej.page_id
                    WHERE p.collection_id = ?1
                ), 0)",
        [collection_id],
        |row| {
            Ok(CollectionMetrics {
                page_count: row.get(0)?,
                quarantined_pages_awaiting_action: row.get(1)?,
                queue_depth: row.get(2)?,
            })
        },
    )
    .map_err(Into::into)
}

fn describe_collection_status(collection: &collections::Collection) -> CollectionStatusSummary {
    if collection.reconcile_halted_at.is_some() {
        let reason = collection
            .reconcile_halt_reason
            .as_deref()
            .unwrap_or("unknown");
        return CollectionStatusSummary {
            blocked_state: "reconcile_halted".to_owned(),
            integrity_blocked: Some(reason.to_owned()),
            suggested_command: Some(format!(
                "quaid collection reconcile-reset {} --confirm",
                collection.name
            )),
            status_message: match reason {
                "duplicate_uuid" => format!(
                    "reconcile is halted on duplicate memory_id values; repair the vault first, then run quaid collection reconcile-reset {} --confirm",
                    collection.name
                ),
                "unresolvable_trivial_content" => format!(
                    "reconcile is halted on trivial-content identity ambiguity; run quaid collection migrate-uuids {} or restore the vault, then run quaid collection reconcile-reset {} --confirm",
                    collection.name, collection.name
                ),
                _ => format!(
                    "reconcile is halted; repair the vault first, then run quaid collection reconcile-reset {} --confirm",
                    collection.name
                ),
            },
        };
    }
    if collection.integrity_failed_at.is_some() {
        return CollectionStatusSummary {
            blocked_state: "restore_integrity_blocked".to_owned(),
            integrity_blocked: Some("manifest_tampering".to_owned()),
            suggested_command: Some(format!(
                "quaid collection restore-reset {} --confirm",
                collection.name
            )),
            status_message: format!(
                "restore is terminally blocked by integrity failure; run quaid collection restore-reset {} --confirm after repair",
                collection.name
            ),
        };
    }
    if collection.pending_manifest_incomplete_at.is_some() {
        return CollectionStatusSummary {
            blocked_state: "pending_finalize".to_owned(),
            integrity_blocked: Some("manifest_incomplete_pending".to_owned()),
            suggested_command: Some(format!(
                "quaid collection sync {} --finalize-pending",
                collection.name
            )),
            status_message: format!(
                "restore manifest is still incomplete; collection remains blocked until the files reappear and quaid collection sync {} --finalize-pending succeeds",
                collection.name
            ),
        };
    }
    if collection.pending_root_path.is_some() {
        return CollectionStatusSummary {
            blocked_state: "pending_finalize".to_owned(),
            integrity_blocked: None,
            suggested_command: Some(format!(
                "quaid collection sync {} --finalize-pending",
                collection.name
            )),
            status_message: format!(
                "restore is waiting for finalize; plain sync stays closed until quaid collection sync {} --finalize-pending succeeds",
                collection.name
            ),
        };
    }
    if matches!(collection.state, collections::CollectionState::Restoring)
        && collection.needs_full_sync
    {
        return CollectionStatusSummary {
            blocked_state: "pending_attach".to_owned(),
            integrity_blocked: Some("post_tx_b_attach_pending".to_owned()),
            suggested_command: Some(format!(
                "quaid collection sync {} --finalize-pending",
                collection.name
            )),
            status_message: format!(
                "restore finalized its root switch but writes stay closed until quaid collection sync {} --finalize-pending completes attach",
                collection.name
            ),
        };
    }
    if matches!(collection.state, collections::CollectionState::Restoring) {
        return CollectionStatusSummary {
            blocked_state: "restoring".to_owned(),
            integrity_blocked: None,
            suggested_command: None,
            status_message:
                "restore is still in progress; plain sync will not reopen this collection"
                    .to_owned(),
        };
    }
    if collection.needs_full_sync {
        return CollectionStatusSummary {
            blocked_state: "active_reconcile_needed".to_owned(),
            integrity_blocked: None,
            suggested_command: Some(format!("quaid collection sync {}", collection.name)),
            status_message:
                "collection is active but needs a real reconcile before writes are considered fully healthy"
                    .to_owned(),
        };
    }

    CollectionStatusSummary {
        blocked_state: "active".to_owned(),
        integrity_blocked: None,
        suggested_command: None,
        status_message:
            "collection is active; plain sync only reports success after active-root reconcile completes"
                .to_owned(),
    }
}

fn sync(db: &Connection, args: CollectionSyncArgs, json: bool) -> Result<()> {
    if args.finalize_pending {
        let collection = collections::get_by_name(db, &args.name)?
            .ok_or_else(|| anyhow::anyhow!("collection not found: {}", args.name))?;
        let outcome = vault_sync::finalize_pending_restore_via_cli(db, collection.id)?;
        match &outcome {
            vault_sync::FinalizeCliOutcome::Attached
            | vault_sync::FinalizeCliOutcome::OrphanRecovered => {
                return render_success(
                    json,
                    serde_json::json!({
                        "status": "ok",
                        "command": "sync",
                        "collection": args.name,
                        "finalize_pending": format!("{outcome:?}")
                    }),
                );
            }
            blocked => {
                bail!(
                    "FinalizePendingBlockedError: collection={} outcome={blocked:?} collection remains blocked and was not finalized",
                    args.name
                );
            }
        }
    }
    if let Some(remap_root) = args.remap_root {
        let summary = vault_sync::remap_collection(db, &args.name, &remap_root, args.online)?;
        return render_success(
            json,
            serde_json::json!({
                "status": "ok",
                "command": "sync",
                "collection": args.name,
                "remap_root": remap_root.display().to_string(),
                "resolved_pages": summary.resolved_pages,
                "missing_pages": summary.missing_pages,
                "mismatched_pages": summary.mismatched_pages,
                "extra_files": summary.extra_files
            }),
        );
    }
    let stats = vault_sync::sync_collection(db, &args.name)?;
    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "sync",
            "collection": args.name,
            "active_root_reconciled": true,
            "status_message": "active root reconciled",
            "walked": stats.walked,
            "modified": stats.modified,
            "new": stats.new,
            "missing": stats.missing,
            "uuid_renamed": stats.uuid_renamed,
            "hash_renamed": stats.hash_renamed
        }),
    )
}

fn restore(db: &Connection, args: CollectionRestoreArgs, json: bool) -> Result<()> {
    let command_identity = vault_sync::begin_restore(db, &args.name, &args.target, args.online)?;
    let collection = collections::get_by_name(db, &args.name)?
        .ok_or_else(|| anyhow::anyhow!("collection not found: {}", args.name))?;
    let page_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM pages WHERE collection_id = ?1 AND quarantined_at IS NULL",
        [collection.id],
        |row| row.get(0),
    )?;
    render_success(
        json,
        serde_json::json!({
            "status": "ok",
            "command": "restore",
            "collection": args.name,
            "target": args.target.display().to_string(),
            "command_identity": command_identity,
            "restored": page_count,
            "byte_exact": page_count,
            "pending_finalize": false
        }),
    )
}

fn resolve_collection_root(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).with_context(|| {
        format!(
            "collection root does not exist or cannot be resolved: {}",
            path.display()
        )
    })?;
    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("failed to stat collection root: {}", canonical.display()))?;
    if !metadata.is_dir() {
        bail!(
            "collection root must be a directory: {}",
            canonical.display()
        );
    }
    #[cfg(unix)]
    {
        fs_safety::open_root_fd(&canonical).map_err(|err| {
            anyhow!(
                "collection root must be a real directory and not a symlink: {} ({err})",
                canonical.display()
            )
        })?;
    }
    Ok(canonical)
}

fn read_initial_ignore_patterns(root_path: &Path) -> Result<Option<String>> {
    let ignore_path = root_path.join(".quaidignore");
    if !ignore_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&ignore_path)
        .with_context(|| format!("failed to read {}", ignore_path.display()))?;
    match ignore_patterns::parse_ignore_file(&content) {
        ParseResult::Valid(patterns) => Ok(Some(serde_json::to_string(&patterns)?)),
        ParseResult::Invalid(errors) => {
            let details = errors
                .iter()
                .map(|error| {
                    format!(
                        "line {} raw={} error={}",
                        error.line,
                        serde_json::to_string(&error.raw).unwrap_or_else(|_| "\"\"".to_owned()),
                        error.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "invalid .quaidignore at {}\n{}\nFix .quaidignore and re-run quaid collection add.",
                ignore_path.display(),
                details
            );
        }
    }
}

fn writable_label(writable: bool) -> &'static str {
    if writable {
        "writable"
    } else {
        "read-only"
    }
}

fn probe_root_writable(root_path: &Path) -> Result<bool> {
    let probe_name = format!(".quaid-probe-{}", Uuid::now_v7());
    let probe_path = root_path.join(&probe_name);

    #[cfg(unix)]
    {
        let root_fd = fs_safety::open_root_fd(root_path).map_err(|err| {
            anyhow!(
                "failed to open collection root for capability probe: {}",
                err
            )
        })?;
        match fs_safety::openat_create_excl(&root_fd, Path::new(&probe_name)) {
            Ok(fd) => {
                drop(fd);
                fs_safety::unlinkat_parent_fd(&root_fd, Path::new(&probe_name)).map_err(|err| {
                    anyhow!(
                        "created capability probe file but failed to remove it: {} ({err})",
                        probe_path.display()
                    )
                })?;
                Ok(true)
            }
            Err(err) if is_read_only_probe_error(&err) => {
                let _ = fs::remove_file(&probe_path);
                eprintln!(
                    "WARN: collection root is read-only; attaching {} as read-only",
                    root_path.display()
                );
                Ok(false)
            }
            Err(err) => {
                let _ = fs::remove_file(&probe_path);
                Err(anyhow!(
                    "failed capability probe in {}: {}",
                    root_path.display(),
                    err
                ))
            }
        }
    }

    #[cfg(not(unix))]
    {
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&probe_path)
        {
            Ok(file) => {
                drop(file);
                fs::remove_file(&probe_path)?;
                Ok(true)
            }
            Err(err) if is_read_only_probe_error(&err) => {
                let _ = fs::remove_file(&probe_path);
                eprintln!(
                    "WARN: collection root is read-only; attaching {} as read-only",
                    root_path.display()
                );
                Ok(false)
            }
            Err(err) => Err(anyhow!(
                "failed capability probe in {}: {}",
                root_path.display(),
                err
            )),
        }
    }
}

fn is_read_only_probe_error(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::PermissionDenied || err.raw_os_error() == Some(30)
}

fn render_success(json: bool, value: serde_json::Value) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!(
            "{}",
            value
                .as_object()
                .into_iter()
                .flat_map(|object| object.iter())
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::put;
    use crate::core::{db, markdown};
    use sha2::{Digest, Sha256};
    use std::path::Path;
    use uuid::Uuid;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn open_test_db() -> Connection {
        db::open(":memory:").unwrap()
    }

    fn open_test_db_file_any() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        (dir, conn)
    }

    #[cfg(unix)]
    fn open_test_db_file() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        (dir, conn)
    }

    fn insert_collection(conn: &Connection, name: &str, root_path: &Path) -> i64 {
        conn.execute(
            "INSERT INTO collections (name, root_path, state, writable, is_write_target)
             VALUES (?1, ?2, 'active', 1, 0)",
            rusqlite::params![name, root_path.display().to_string()],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_page_with_raw_import(
        conn: &Connection,
        collection_id: i64,
        slug: &str,
        uuid: &str,
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
        let compiled_truth = std::str::from_utf8(raw_bytes)
            .ok()
            .map(|s| markdown::parse_frontmatter(s).1)
            .unwrap_or_default();
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES (?1, ?2, ?3, 'concept', ?2, '', ?4, '', ?5, '', '', 1)",
            rusqlite::params![collection_id, slug, uuid, compiled_truth, frontmatter_json],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
             VALUES (?1, ?2, 1, ?3, ?4)",
            rusqlite::params![
                page_id,
                Uuid::now_v7().to_string(),
                raw_bytes,
                relative_path
            ],
        )
        .unwrap();
        let hash = Sha256::digest(raw_bytes);
        let sha256 = hash
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        conn.execute(
            "INSERT INTO file_state (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256)
             VALUES (?1, ?2, ?3, 1, 1, ?4, 1, ?5)",
            rusqlite::params![collection_id, relative_path, page_id, raw_bytes.len() as i64, sha256],
        )
        .unwrap();
        page_id
    }

    fn page_id(conn: &Connection, collection_id: i64, slug: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
            rusqlite::params![collection_id, slug],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn quarantine_page(conn: &Connection, page_id: i64, quarantined_at: &str) {
        conn.execute(
            "UPDATE pages SET quarantined_at = ?2 WHERE id = ?1",
            rusqlite::params![page_id, quarantined_at],
        )
        .unwrap();
    }

    fn insert_knowledge_gap(conn: &Connection, page_id: i64, query_hash: &str) {
        conn.execute(
            "INSERT INTO knowledge_gaps (page_id, query_hash, context) VALUES (?1, ?2, 'context')",
            rusqlite::params![page_id, query_hash],
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn collection_page_count(conn: &Connection, name: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*)
               FROM pages p
               JOIN collections c ON c.id = p.collection_id
              WHERE c.name = ?1 AND p.quarantined_at IS NULL",
            [name],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn fetch_ignore_mirror(conn: &Connection, name: &str) -> Option<String> {
        conn.query_row(
            "SELECT ignore_patterns FROM collections WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn attach_collection(conn: &Connection, name: &str, root_path: &Path) {
        run(
            conn,
            CollectionAction::Add(CollectionAddArgs {
                name: name.to_owned(),
                path: root_path.to_path_buf(),
                read_only: false,
                writable: false,
                write_memory_id: false,
            }),
            true,
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn add_refuses_invalid_root_before_creating_collection_row() {
        let conn = open_test_db();
        let missing = PathBuf::from(r"D:\does-not-exist");

        let error = run(
            &conn,
            CollectionAction::Add(CollectionAddArgs {
                name: "work".to_owned(),
                path: missing,
                read_only: false,
                writable: false,
                write_memory_id: false,
            }),
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("collection root"));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM collections WHERE name = 'work'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[cfg(unix)]
    #[test]
    fn add_refuses_invalid_ignore_before_creating_collection_row() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        fs::write(root.path().join(".quaidignore"), "[broken\n").unwrap();

        let error = run(
            &conn,
            CollectionAction::Add(CollectionAddArgs {
                name: "work".to_owned(),
                path: root.path().to_path_buf(),
                read_only: false,
                writable: false,
                write_memory_id: false,
            }),
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("invalid .quaidignore"));
        assert!(error.to_string().contains("Fix .quaidignore and re-run"));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM collections WHERE name = 'work'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[cfg(unix)]
    #[test]
    fn add_attaches_collection_and_cleans_short_lived_lease_residue() {
        let (_dir, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        fs::write(
            root.path().join("note.md"),
            "---\ntitle: Note\ntype: note\n---\nhello\n",
        )
        .unwrap();

        run(
            &conn,
            CollectionAction::Add(CollectionAddArgs {
                name: "work".to_owned(),
                path: root.path().to_path_buf(),
                read_only: false,
                writable: false,
                write_memory_id: false,
            }),
            true,
        )
        .unwrap();

        let row: (String, i64, Option<String>, i64, i64) = conn
            .query_row(
                "SELECT state, writable, active_lease_session_id,
                        (SELECT COUNT(*) FROM collection_owners),
                        (SELECT COUNT(*) FROM serve_sessions)
                 FROM collections WHERE name = 'work'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, "active");
        assert_eq!(row.1, 1);
        assert!(row.2.is_none());
        assert_eq!(row.3, 0);
        assert_eq!(row.4, 0);

        let page_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages p JOIN collections c ON c.id = p.collection_id WHERE c.name = 'work'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(page_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn capability_probe_leaves_no_residue_on_success() {
        let root = tempfile::TempDir::new().unwrap();

        let writable = probe_root_writable(root.path()).unwrap();

        assert!(writable);
        let residue = fs::read_dir(root.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".quaid-probe-")
            });
        assert!(!residue);
    }

    #[cfg(unix)]
    #[test]
    fn add_marks_collection_read_only_when_probe_hits_permission_denied() {
        if rustix::process::geteuid().is_root() {
            return;
        }

        let (_dir, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        fs::write(
            root.path().join("note.md"),
            "---\ntitle: Note\ntype: note\n---\nhello\n",
        )
        .unwrap();
        let original_permissions = fs::metadata(root.path()).unwrap().permissions();
        let mut read_only_permissions = original_permissions.clone();
        read_only_permissions.set_mode(0o555);
        fs::set_permissions(root.path(), read_only_permissions).unwrap();

        let result = run(
            &conn,
            CollectionAction::Add(CollectionAddArgs {
                name: "ro-vault".to_owned(),
                path: root.path().to_path_buf(),
                read_only: false,
                writable: false,
                write_memory_id: false,
            }),
            true,
        );

        fs::set_permissions(root.path(), original_permissions).unwrap();
        result.unwrap();

        let writable: i64 = conn
            .query_row(
                "SELECT writable FROM collections WHERE name = 'ro-vault'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(writable, 0);
        let residue = fs::read_dir(root.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".quaid-probe-")
            });
        assert!(!residue);
    }

    #[cfg(unix)]
    #[test]
    fn info_and_list_surfaces_report_read_only_truthfully() {
        let (_dir, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        fs::write(
            root.path().join("note.md"),
            "---\ntitle: Note\ntype: note\n---\nhello\n",
        )
        .unwrap();

        run(
            &conn,
            CollectionAction::Add(CollectionAddArgs {
                name: "archive".to_owned(),
                path: root.path().to_path_buf(),
                read_only: true,
                writable: false,
                write_memory_id: false,
            }),
            true,
        )
        .unwrap();

        let collection = collections::get_by_name(&conn, "archive").unwrap().unwrap();
        assert!(!collection.writable);
        let status = describe_collection_status(&collection);
        assert_eq!(status.blocked_state, "active");

        let metrics = collection_metrics(&conn, collection.id).unwrap();
        assert_eq!(metrics.page_count, 1);
        let row = CollectionListRow {
            name: collection.name.clone(),
            state: collection.state.as_str().to_owned(),
            writable: writable_label(collection.writable).to_owned(),
            write_target: collection.is_write_target,
            root_path: collection.root_path.clone(),
            page_count: metrics.page_count,
            last_sync_at: collection.last_sync_at.clone(),
            queue_depth: metrics.queue_depth,
        };
        assert_eq!(row.writable, "read-only");

        let info = CollectionInfoOutput {
            name: collection.name,
            root_path: collection.root_path,
            writable: collection.writable,
            writable_mode: writable_label(collection.writable).to_owned(),
            write_target: collection.is_write_target,
            state: collection.state.as_str().to_owned(),
            needs_full_sync: collection.needs_full_sync,
            last_sync_at: collection.last_sync_at,
            page_count: metrics.page_count,
            queue_depth: metrics.queue_depth,
            quarantined_pages_awaiting_action: metrics.quarantined_pages_awaiting_action,
            ignore_parse_errors: collection.ignore_parse_errors,
            pending_root_path: collection.pending_root_path,
            restore_command_id: collection.restore_command_id,
            restore_command_pid: collection.restore_command_pid,
            restore_command_host: collection.restore_command_host,
            pending_command_heartbeat_at: collection.pending_command_heartbeat_at,
            integrity_failed_at: collection.integrity_failed_at,
            pending_manifest_incomplete_at: collection.pending_manifest_incomplete_at,
            reconcile_halted_at: collection.reconcile_halted_at,
            reconcile_halt_reason: collection.reconcile_halt_reason,
            reload_generation: collection.reload_generation,
            watcher_released_session_id: collection.watcher_released_session_id,
            watcher_released_generation: collection.watcher_released_generation,
            blocked_state: status.blocked_state,
            integrity_blocked: status.integrity_blocked,
            suggested_command: status.suggested_command,
            status_message: status.status_message,
        };
        assert_eq!(info.writable_mode, "read-only");
        assert!(!info.writable);
    }

    #[test]
    fn put_refuses_read_only_collection() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET writable = 0 WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let error = put::put_from_string(
            &conn,
            "work::notes/read-only",
            "---\ntitle: Read Only\ntype: note\n---\nhello\n",
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("CollectionReadOnlyError"));
    }

    #[test]
    fn restore_reset_requires_confirm() {
        let conn = db::open(":memory:").unwrap();
        let error = run(
            &conn,
            CollectionAction::RestoreReset {
                name: "work".to_owned(),
                confirm: false,
            },
            true,
        )
        .unwrap_err();
        assert!(error.to_string().contains("--confirm"));
    }

    #[test]
    fn reconcile_reset_requires_confirm() {
        let conn = db::open(":memory:").unwrap();
        let error = run(
            &conn,
            CollectionAction::ReconcileReset {
                name: "work".to_owned(),
                confirm: false,
            },
            true,
        )
        .unwrap_err();
        assert!(error.to_string().contains("--confirm"));
    }

    #[test]
    fn add_rejects_duplicate_collection_name_before_attach() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        insert_collection(&conn, "work", root.path());

        let error = add(
            &conn,
            CollectionAddArgs {
                name: "work".to_owned(),
                path: root.path().to_path_buf(),
                read_only: false,
                writable: false,
                write_memory_id: false,
            },
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("collection already exists"));
    }

    #[test]
    fn add_rejects_write_memory_id_before_creating_collection_row() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();

        let error = add(
            &conn,
            CollectionAddArgs {
                name: "work".to_owned(),
                path: root.path().to_path_buf(),
                read_only: false,
                writable: false,
                write_memory_id: true,
            },
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("--write-quaid-id is deferred"));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM collections WHERE name = 'work'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[cfg(unix)]
    #[test]
    fn ignore_add_updates_file_mirror_and_reconciles() {
        let (_db_dir, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        fs::write(
            root.path().join("note.md"),
            "---\ntitle: Note\ntype: note\n---\nhello\n",
        )
        .unwrap();
        attach_collection(&conn, "work", root.path());

        run(
            &conn,
            CollectionAction::Ignore {
                action: CollectionIgnoreAction::Add {
                    name: "work".to_owned(),
                    pattern: "note.md".to_owned(),
                },
            },
            true,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(root.path().join(".quaidignore")).unwrap(),
            "note.md\n"
        );
        let mirror: Vec<String> =
            serde_json::from_str(&fetch_ignore_mirror(&conn, "work").unwrap()).unwrap();
        assert_eq!(mirror, vec!["note.md"]);
        assert_eq!(collection_page_count(&conn, "work"), 0);
    }

    #[cfg(unix)]
    #[test]
    fn ignore_clear_removes_file_clears_mirror_and_reconciles() {
        let (_db_dir, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        fs::write(
            root.path().join("note.md"),
            "---\ntitle: Note\ntype: note\n---\nhello\n",
        )
        .unwrap();
        attach_collection(&conn, "work", root.path());
        run(
            &conn,
            CollectionAction::Ignore {
                action: CollectionIgnoreAction::Add {
                    name: "work".to_owned(),
                    pattern: "note.md".to_owned(),
                },
            },
            true,
        )
        .unwrap();

        run(
            &conn,
            CollectionAction::Ignore {
                action: CollectionIgnoreAction::Clear {
                    name: "work".to_owned(),
                    confirm: true,
                },
            },
            true,
        )
        .unwrap();

        assert!(!root.path().join(".quaidignore").exists());
        assert!(fetch_ignore_mirror(&conn, "work").is_none());
        assert_eq!(collection_page_count(&conn, "work"), 1);
    }

    #[cfg(unix)]
    #[test]
    fn ignore_add_invalid_glob_refuses_without_disk_or_db_mutation() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        insert_collection(&conn, "work", root.path());

        let error = run(
            &conn,
            CollectionAction::Ignore {
                action: CollectionIgnoreAction::Add {
                    name: "work".to_owned(),
                    pattern: "[broken".to_owned(),
                },
            },
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("Invalid glob pattern"));
        assert!(!root.path().join(".quaidignore").exists());
        assert!(fetch_ignore_mirror(&conn, "work").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn ignore_remove_updates_file_and_mirror() {
        let (_db_dir, conn) = open_test_db_file();
        let root = tempfile::TempDir::new().unwrap();
        fs::write(
            root.path().join("note.md"),
            "---\ntitle: Note\ntype: note\n---\nhello\n",
        )
        .unwrap();
        attach_collection(&conn, "work", root.path());
        run(
            &conn,
            CollectionAction::Ignore {
                action: CollectionIgnoreAction::Add {
                    name: "work".to_owned(),
                    pattern: "note.md".to_owned(),
                },
            },
            true,
        )
        .unwrap();
        run(
            &conn,
            CollectionAction::Ignore {
                action: CollectionIgnoreAction::Add {
                    name: "work".to_owned(),
                    pattern: "archive/**".to_owned(),
                },
            },
            true,
        )
        .unwrap();

        run(
            &conn,
            CollectionAction::Ignore {
                action: CollectionIgnoreAction::Remove {
                    name: "work".to_owned(),
                    pattern: "note.md".to_owned(),
                },
            },
            true,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(root.path().join(".quaidignore")).unwrap(),
            "archive/**\n"
        );
        let mirror: Vec<String> =
            serde_json::from_str(&fetch_ignore_mirror(&conn, "work").unwrap()).unwrap();
        assert_eq!(mirror, vec!["archive/**"]);
        assert_eq!(collection_page_count(&conn, "work"), 1);
    }

    #[cfg(unix)]
    #[test]
    fn ignore_mutations_refuse_while_collection_is_restoring() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let error = run(
            &conn,
            CollectionAction::Ignore {
                action: CollectionIgnoreAction::Add {
                    name: "work".to_owned(),
                    pattern: "private/**".to_owned(),
                },
            },
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("CollectionRestoringError"));
        assert!(!root.path().join(".quaidignore").exists());
    }

    #[test]
    fn ignore_list_reads_cached_user_patterns_only() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET ignore_patterns = ?2 WHERE id = ?1",
            rusqlite::params![
                collection_id,
                serde_json::to_string(&vec!["private/**", "*.bak"]).unwrap()
            ],
        )
        .unwrap();

        let collection = load_collection_by_name(&conn, "work").unwrap();
        let patterns = cached_user_ignore_patterns(&collection).unwrap();

        assert_eq!(patterns, vec!["private/**", "*.bak"]);
        let globset =
            ignore_patterns::build_globset_from_patterns(collection.ignore_patterns.as_deref())
                .unwrap();
        assert!(globset.is_match(".git/config"));
        assert!(globset.is_match("private/plan.md"));
    }

    #[test]
    fn ignore_cli_never_writes_ignore_mirror_directly() {
        let source = include_str!("collection.rs");
        let production_source = source.split("#[cfg(test)]").next().unwrap_or(source);

        assert!(!production_source.contains("SET ignore_patterns ="));
        assert!(production_source.contains("ignore_patterns::reload_patterns"));
        assert!(production_source.contains("ignore_patterns::clear_patterns"));
    }

    #[cfg(not(unix))]
    #[test]
    fn add_refuses_windows_platform() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();

        let error = run(
            &conn,
            CollectionAction::Add(CollectionAddArgs {
                name: "work".to_owned(),
                path: root.path().to_path_buf(),
                read_only: false,
                writable: false,
                write_memory_id: false,
            }),
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("UnsupportedPlatformError"));
    }

    #[cfg(not(unix))]
    #[test]
    fn sync_refuses_windows_platform() {
        let conn = open_test_db();

        let error = run(
            &conn,
            CollectionAction::Sync(CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: false,
                online: false,
            }),
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("UnsupportedPlatformError"));
    }

    #[cfg(not(unix))]
    #[test]
    fn restore_refuses_windows_platform() {
        let conn = open_test_db();
        let target = tempfile::TempDir::new().unwrap();

        let error = run(
            &conn,
            CollectionAction::Restore(CollectionRestoreArgs {
                name: "work".to_owned(),
                target: target.path().to_path_buf(),
                online: false,
            }),
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("UnsupportedPlatformError"));
    }

    #[cfg(unix)]
    #[test]
    fn sync_finalize_pending_uses_external_finalize_path() {
        let (_db_dir, conn) = open_test_db_file();
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
                 pending_command_heartbeat_at = datetime('now', '-120 seconds')
             WHERE id = ?1",
            rusqlite::params![collection_id, pending_root.display().to_string()],
        )
        .unwrap();

        run(
            &conn,
            CollectionAction::Sync(CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: true,
                online: false,
            }),
            true,
        )
        .unwrap();

        let row: (String, String, i64, Option<String>) = conn
            .query_row(
                "SELECT state, root_path, needs_full_sync, pending_root_path
                  FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "active");
        assert_eq!(row.1, pending_root.display().to_string());
        assert_eq!(row.2, 0);
        assert!(row.3.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn sync_without_flags_requires_active_root_collection() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections SET state = 'detached' WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let error = run(
            &conn,
            CollectionAction::Sync(CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: false,
                online: false,
            }),
            true,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("PlainSyncActiveRootRequiredError"));
    }

    #[cfg(unix)]
    #[test]
    fn sync_without_flags_refuses_restore_in_progress_state() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let error = run(
            &conn,
            CollectionAction::Sync(CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: false,
                online: false,
            }),
            true,
        )
        .unwrap_err();

        let state: String = conn
            .query_row(
                "SELECT state FROM collections WHERE id = ?1",
                [collection_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(error.to_string().contains("RestoreInProgressError"));
        assert_eq!(state, "restoring");
    }

    #[cfg(unix)]
    #[test]
    fn sync_without_flags_does_not_finalize_pending_restore_state() {
        let conn = db::open(":memory:").unwrap();
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
            rusqlite::params![collection_id, pending_root.display().to_string()],
        )
        .unwrap();

        let error = run(
            &conn,
            CollectionAction::Sync(CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: false,
                online: false,
            }),
            true,
        )
        .unwrap_err();

        let row: (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT state, pending_root_path, restore_command_id
                 FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(error.to_string().contains("RestorePendingFinalizeError"));
        assert_eq!(row.0, "restoring");
        assert_eq!(row.1.as_deref(), Some(pending_root.to_str().unwrap()));
        assert_eq!(row.2.as_deref(), Some("restore-1"));
    }

    #[cfg(unix)]
    #[test]
    fn sync_without_flags_refuses_restore_integrity_blocked_state() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 integrity_failed_at = '2026-04-23T00:00:00Z'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let error = run(
            &conn,
            CollectionAction::Sync(CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: false,
                online: false,
            }),
            true,
        )
        .unwrap_err();

        let row: (String, Option<String>) = conn
            .query_row(
                "SELECT state, integrity_failed_at FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(error.to_string().contains("RestoreIntegrityBlockedError"));
        assert_eq!(row.0, "restoring");
        assert_eq!(row.1.as_deref(), Some("2026-04-23T00:00:00Z"));
    }

    #[cfg(unix)]
    #[test]
    fn sync_without_flags_does_not_clear_integrity_or_reconcile_halt_markers() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET integrity_failed_at = '2026-04-23T00:00:00Z',
                 reconcile_halted_at = '2026-04-23T00:05:00Z',
                 reconcile_halt_reason = 'duplicate_uuid'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let error = run(
            &conn,
            CollectionAction::Sync(CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: false,
                online: false,
            }),
            true,
        )
        .unwrap_err();

        let row: (Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT integrity_failed_at, reconcile_halted_at, reconcile_halt_reason
                 FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(error.to_string().contains("ReconcileHaltedError"));
        assert_eq!(row.0.as_deref(), Some("2026-04-23T00:00:00Z"));
        assert_eq!(row.1.as_deref(), Some("2026-04-23T00:05:00Z"));
        assert_eq!(row.2.as_deref(), Some("duplicate_uuid"));
    }

    #[test]
    fn run_routes_ignore_list_action() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET ignore_patterns = ?2 WHERE id = ?1",
            rusqlite::params![
                collection_id,
                serde_json::to_string(&vec!["private/**"]).unwrap()
            ],
        )
        .unwrap();

        run(
            &conn,
            CollectionAction::Ignore {
                action: CollectionIgnoreAction::List {
                    name: "work".to_owned(),
                },
            },
            true,
        )
        .unwrap();
    }

    #[test]
    fn run_routes_quarantine_list_export_and_discard_actions() {
        let (_db_dir, conn) = open_test_db_file_any();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/quarantined",
            &uuid::Uuid::now_v7().to_string(),
            b"---\ntitle: Quarantined\ntype: note\n---\nbody\n",
            "notes/quarantined.md",
        );
        let page_id = page_id(&conn, collection_id, "notes/quarantined");
        quarantine_page(&conn, page_id, "2026-04-28T00:00:00Z");
        insert_knowledge_gap(&conn, page_id, "gap-1");

        run(
            &conn,
            CollectionAction::Quarantine {
                action: CollectionQuarantineAction::List {
                    name: "work".to_owned(),
                },
            },
            true,
        )
        .unwrap();

        let output_path = root.path().join("quarantine-export.json");
        quarantine_action(
            &conn,
            CollectionQuarantineAction::Export {
                slug: "work::notes/quarantined".to_owned(),
                output: output_path.clone(),
            },
            true,
        )
        .unwrap();
        assert!(output_path.exists());

        quarantine_action(
            &conn,
            CollectionQuarantineAction::Discard {
                slug: "work::notes/quarantined".to_owned(),
                force: false,
            },
            true,
        )
        .unwrap();
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn describe_collection_status_points_pending_finalize_to_finalize_command() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = 'D:\\vault\\restored'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let collection = collections::get_by_name(&conn, "work").unwrap().unwrap();

        let status = describe_collection_status(&collection);

        assert_eq!(status.blocked_state, "pending_finalize");
        assert_eq!(
            status.suggested_command.as_deref(),
            Some("quaid collection sync work --finalize-pending")
        );
    }

    #[test]
    fn describe_collection_status_points_retryable_manifest_gap_to_finalize_command() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = 'D:\\vault\\restored',
                 pending_manifest_incomplete_at = '2026-04-23T00:00:00Z'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let collection = collections::get_by_name(&conn, "work").unwrap().unwrap();

        let status = describe_collection_status(&collection);

        assert_eq!(status.blocked_state, "pending_finalize");
        assert_eq!(
            status.integrity_blocked.as_deref(),
            Some("manifest_incomplete_pending")
        );
        assert_eq!(
            status.suggested_command.as_deref(),
            Some("quaid collection sync work --finalize-pending")
        );
    }

    #[test]
    fn describe_collection_status_points_pending_attach_to_finalize_command() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 needs_full_sync = 1
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let collection = collections::get_by_name(&conn, "work").unwrap().unwrap();

        let status = describe_collection_status(&collection);

        assert_eq!(status.blocked_state, "pending_attach");
        assert_eq!(
            status.integrity_blocked.as_deref(),
            Some("post_tx_b_attach_pending")
        );
        assert_eq!(
            status.suggested_command.as_deref(),
            Some("quaid collection sync work --finalize-pending")
        );
    }

    #[test]
    fn describe_collection_status_reports_plain_restoring_without_finalize_hint() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 needs_full_sync = 0,
                 pending_root_path = NULL,
                 pending_manifest_incomplete_at = NULL
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let collection = collections::get_by_name(&conn, "work").unwrap().unwrap();

        let status = describe_collection_status(&collection);

        assert_eq!(status.blocked_state, "restoring");
        assert!(status.integrity_blocked.is_none());
        assert!(status.suggested_command.is_none());
        assert!(status.status_message.contains("still in progress"));
    }

    #[test]
    fn describe_collection_status_points_active_reconcile_needed_to_plain_sync() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET state = 'active',
                 needs_full_sync = 1
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let collection = collections::get_by_name(&conn, "work").unwrap().unwrap();

        let status = describe_collection_status(&collection);

        assert_eq!(status.blocked_state, "active_reconcile_needed");
        assert!(status.integrity_blocked.is_none());
        assert_eq!(
            status.suggested_command.as_deref(),
            Some("quaid collection sync work")
        );
        assert!(status.status_message.contains("needs a real reconcile"));
    }

    #[test]
    fn build_collection_info_output_surfaces_active_watcher_health_from_runtime_registry() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET reload_generation = 6,
                 watcher_released_session_id = 'serve-session',
                 watcher_released_generation = 5
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        vault_sync::set_collection_watcher_health_for_test(
            collection_id,
            "serve-session",
            3,
            Some(vault_sync::WatcherMode::Poll),
            Some("2026-04-28T01:02:03Z".to_owned()),
            7,
        );
        let collection = collections::get_by_name(&conn, "work").unwrap().unwrap();

        let output = build_collection_info_output(&conn, collection).unwrap();

        assert_eq!(output.watcher_mode.as_deref(), Some("poll"));
        assert_eq!(
            output.watcher_last_event_at.as_deref(),
            Some("2026-04-28T01:02:03Z")
        );
        assert_eq!(output.watcher_channel_depth, Some(7));
        assert_eq!(output.reload_generation, 6);
        assert_eq!(
            output.watcher_released_session_id.as_deref(),
            Some("serve-session")
        );
        assert_eq!(output.watcher_released_generation, Some(5));
        vault_sync::clear_collection_watcher_health_for_test(collection_id);
    }

    #[test]
    fn load_ignore_source_prefers_disk_and_falls_back_to_cached_patterns() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET ignore_patterns = ?2 WHERE id = ?1",
            rusqlite::params![
                collection_id,
                serde_json::to_string(&vec!["cached/**", "*.tmp"]).unwrap()
            ],
        )
        .unwrap();
        let collection = load_collection_by_name(&conn, "work").unwrap();

        assert_eq!(
            load_ignore_source(&collection).unwrap(),
            Some("cached/**\n*.tmp\n".to_owned())
        );

        fs::write(root.path().join(".quaidignore"), "disk-only\n").unwrap();
        assert_eq!(
            load_ignore_source(&collection).unwrap(),
            Some("disk-only\n".to_owned())
        );
    }

    #[test]
    fn ignore_pattern_content_helpers_preserve_comments_and_avoid_duplicates() {
        assert_eq!(
            add_ignore_pattern_content(Some("notes/**".to_owned()), "notes/**"),
            "notes/**"
        );
        assert_eq!(
            add_ignore_pattern_content(Some("# keep\nnotes/**".to_owned()), "*.tmp"),
            "# keep\nnotes/**\n*.tmp\n"
        );
        assert_eq!(
            remove_ignore_pattern_content(Some("# keep\nnotes/**\n*.tmp\n".to_owned()), "notes/**"),
            "# keep\n*.tmp\n"
        );
        assert_eq!(remove_ignore_pattern_content(None, "notes/**"), "");
        assert_eq!(
            remove_ignore_pattern_content(Some("notes/**\n*.tmp".to_owned()), "*.tmp"),
            "notes/**"
        );
    }

    #[test]
    fn validate_cli_ignore_pattern_rejects_blank_and_comment_inputs() {
        let blank = validate_cli_ignore_pattern("   ").unwrap_err();
        assert!(blank.to_string().contains("non-empty glob"));

        let comment = validate_cli_ignore_pattern("# comment").unwrap_err();
        assert!(comment.to_string().contains("non-empty glob"));

        assert_eq!(
            validate_cli_ignore_pattern("  notes/**  ").unwrap(),
            "notes/**"
        );
    }

    #[test]
    fn validate_proposed_ignore_content_formats_parse_errors() {
        let error = validate_proposed_ignore_content("[broken\n").unwrap_err();
        let text = error.to_string();

        assert!(text.contains("line 1"));
        assert!(text.contains("raw=\"[broken\""));
        assert!(text.contains("Invalid glob pattern"));
    }

    #[test]
    fn write_ignore_file_atomically_round_trips_contents() {
        let root = tempfile::TempDir::new().unwrap();

        let ignore_path = write_ignore_file_atomically(root.path(), Some("notes/**\n")).unwrap();
        assert_eq!(ignore_path, root.path().join(".quaidignore"));
        assert_eq!(fs::read_to_string(&ignore_path).unwrap(), "notes/**\n");

        let cleared_path = write_ignore_file_atomically(root.path(), None).unwrap();
        assert_eq!(cleared_path, ignore_path);
        assert!(!cleared_path.exists());
    }

    #[test]
    fn write_ignore_file_atomically_tolerates_absent_existing_file() {
        let root = tempfile::TempDir::new().unwrap();

        let ignore_path = write_ignore_file_atomically(root.path(), None).unwrap();

        assert_eq!(ignore_path, root.path().join(".quaidignore"));
        assert!(!ignore_path.exists());
    }

    #[test]
    fn read_initial_ignore_patterns_handles_missing_and_valid_files() {
        let root = tempfile::TempDir::new().unwrap();
        assert!(read_initial_ignore_patterns(root.path()).unwrap().is_none());

        fs::write(root.path().join(".quaidignore"), "notes/**\n*.tmp\n").unwrap();
        assert_eq!(
            read_initial_ignore_patterns(root.path()).unwrap(),
            Some(serde_json::to_string(&vec!["notes/**", "*.tmp"]).unwrap())
        );
    }

    #[test]
    fn load_ignore_source_reports_read_errors() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(root.path().join(".quaidignore")).unwrap();
        insert_collection(&conn, "work", root.path());
        let collection = load_collection_by_name(&conn, "work").unwrap();

        let error = load_ignore_source(&collection).unwrap_err();

        assert!(error.to_string().contains("failed to read"));
        assert!(error.to_string().contains(".quaidignore"));
    }

    #[test]
    fn cached_user_ignore_patterns_reports_invalid_mirror_json() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET ignore_patterns = ?2 WHERE id = ?1",
            rusqlite::params![collection_id, "not-json"],
        )
        .unwrap();
        let collection = load_collection_by_name(&conn, "work").unwrap();

        let error = cached_user_ignore_patterns(&collection).unwrap_err();

        assert!(error
            .to_string()
            .contains("invalid ignore pattern mirror for work"));
    }

    #[test]
    fn read_initial_ignore_patterns_reports_invalid_file_details() {
        let root = tempfile::TempDir::new().unwrap();
        fs::write(root.path().join(".quaidignore"), "[broken\n").unwrap();

        let error = read_initial_ignore_patterns(root.path()).unwrap_err();
        let text = error.to_string();

        assert!(text.contains("invalid .quaidignore"));
        assert!(text.contains("line 1 raw=\"[broken\" error=Invalid glob pattern"));
        assert!(text.contains("Fix .quaidignore and re-run quaid collection add."));
    }

    #[test]
    fn restore_reset_and_reconcile_reset_succeed_when_confirmed() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = 'D:\\vault\\restored',
                 integrity_failed_at = '2026-04-23T00:00:00Z',
                 reconcile_halted_at = '2026-04-23T00:05:00Z',
                 reconcile_halt_reason = 'duplicate_uuid'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        run(
            &conn,
            CollectionAction::RestoreReset {
                name: "work".to_owned(),
                confirm: true,
            },
            true,
        )
        .unwrap();
        run(
            &conn,
            CollectionAction::ReconcileReset {
                name: "work".to_owned(),
                confirm: true,
            },
            true,
        )
        .unwrap();

        let row: (String, Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT state, pending_root_path, integrity_failed_at, reconcile_halted_at
                 FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "active");
        assert!(row.1.is_none());
        assert!(row.2.is_none());
        assert!(row.3.is_none());
    }

    #[test]
    fn list_and_info_plain_text_helpers_return_ok_for_seeded_collection() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections
             SET ignore_parse_errors = 'line 2 raw=\"[broken\" error=Invalid glob pattern',
                 reload_generation = 2,
                 watcher_released_session_id = 'serve-1',
                 watcher_released_generation = 1,
                 needs_full_sync = 1
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        list(&conn, false).unwrap();
        info(&conn, "work", false).unwrap();
    }

    #[test]
    fn resolve_collection_root_rejects_non_directory_paths() {
        let root = tempfile::TempDir::new().unwrap();
        let file_path = root.path().join("note.md");
        fs::write(&file_path, "hello").unwrap();

        let error = resolve_collection_root(&file_path).unwrap_err();
        assert!(error.to_string().contains("must be a directory"));
        assert_eq!(writable_label(true), "writable");
        assert_eq!(writable_label(false), "read-only");
        assert!(is_read_only_probe_error(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(!is_read_only_probe_error(&io::Error::from(
            io::ErrorKind::Other
        )));
    }

    #[test]
    fn resolve_collection_root_reports_missing_paths_clearly() {
        let root = tempfile::TempDir::new().unwrap();
        let missing = root.path().join("missing");

        let error = resolve_collection_root(&missing).unwrap_err();

        assert!(error
            .to_string()
            .contains("collection root does not exist or cannot be resolved"));
    }

    #[cfg(not(unix))]
    #[test]
    fn add_helper_cleans_up_collection_row_when_fresh_attach_is_unsupported() {
        let (_db_dir, conn) = open_test_db_file_any();
        let root = tempfile::TempDir::new().unwrap();
        fs::write(
            root.path().join("note.md"),
            "---\ntitle: Note\ntype: note\n---\nhello\n",
        )
        .unwrap();

        let error = add(
            &conn,
            CollectionAddArgs {
                name: "work".to_owned(),
                path: root.path().to_path_buf(),
                read_only: false,
                writable: false,
                write_memory_id: false,
            },
            true,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Vault sync commands require Unix"));
        assert!(collections::get_by_name(&conn, "work").unwrap().is_none());
    }

    #[cfg(not(unix))]
    #[test]
    fn probe_root_writable_succeeds_without_residue_on_windows() {
        let root = tempfile::TempDir::new().unwrap();

        let writable = probe_root_writable(root.path()).unwrap();

        assert!(writable);
        let residue = fs::read_dir(root.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".quaid-probe-")
            });
        assert!(!residue);
    }

    #[cfg(not(unix))]
    #[test]
    fn probe_root_writable_reports_non_permission_failures_on_windows() {
        let root = tempfile::TempDir::new().unwrap();

        let error = probe_root_writable(&root.path().join("missing")).unwrap_err();

        assert!(error.to_string().contains("failed capability probe in"));
    }

    #[cfg(not(unix))]
    #[test]
    fn ignore_dispatch_requires_confirm_before_clearing() {
        let conn = open_test_db();
        let root = tempfile::TempDir::new().unwrap();
        insert_collection(&conn, "work", root.path());

        let error = ignore(
            &conn,
            CollectionIgnoreAction::Clear {
                name: "work".to_owned(),
                confirm: false,
            },
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("UnsupportedPlatformError"));
        assert!(!root.path().join(".quaidignore").exists());
    }

    #[cfg(not(unix))]
    #[test]
    fn quarantine_restore_dispatch_refuses_windows_platform() {
        let conn = open_test_db();

        let error = quarantine_action(
            &conn,
            CollectionQuarantineAction::Restore {
                slug: "work::notes/a".to_owned(),
                relative_path: "notes/a.md".to_owned(),
            },
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("UnsupportedPlatformError"));
    }

    #[test]
    fn describe_collection_status_reports_unresolvable_trivial_content_remediation() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET reconcile_halted_at = '2026-04-23T00:05:00Z',
                 reconcile_halt_reason = 'unresolvable_trivial_content'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let collection = collections::get_by_name(&conn, "work").unwrap().unwrap();

        let status = describe_collection_status(&collection);

        assert_eq!(status.blocked_state, "reconcile_halted");
        assert_eq!(
            status.integrity_blocked.as_deref(),
            Some("unresolvable_trivial_content")
        );
        assert!(status.status_message.contains("migrate-uuids work"));
    }

    #[test]
    fn describe_collection_status_reports_unknown_reconcile_halt_generically() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", temp.path());
        conn.execute(
            "UPDATE collections
             SET reconcile_halted_at = '2026-04-23T00:05:00Z',
                 reconcile_halt_reason = 'mystery'
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();
        let collection = collections::get_by_name(&conn, "work").unwrap().unwrap();

        let status = describe_collection_status(&collection);

        assert_eq!(status.blocked_state, "reconcile_halted");
        assert_eq!(status.integrity_blocked.as_deref(), Some("mystery"));
        assert!(status.status_message.contains("repair the vault first"));
    }

    #[test]
    fn describe_collection_status_reports_active_state_without_follow_up_command() {
        let conn = db::open(":memory:").unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        insert_collection(&conn, "work", temp.path());
        let collection = collections::get_by_name(&conn, "work").unwrap().unwrap();

        let status = describe_collection_status(&collection);

        assert_eq!(status.blocked_state, "active");
        assert!(status.integrity_blocked.is_none());
        assert!(status.suggested_command.is_none());
        assert!(status
            .status_message
            .contains("plain sync only reports success after active-root reconcile completes"));
    }

    #[cfg(not(unix))]
    #[test]
    fn sync_finalize_pending_fails_closed_when_attach_backend_is_unavailable() {
        let (_db_dir, conn) = open_test_db_file_any();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections
             SET state = 'active',
                 needs_full_sync = 1
             WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        sync(
            &conn,
            CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: true,
                online: false,
            },
            false,
        )
        .unwrap_err();

        let row: (String, i64) = conn
            .query_row(
                "SELECT state, needs_full_sync FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, "active");
        assert_eq!(row.1, 1);
    }

    #[cfg(not(unix))]
    #[test]
    fn sync_finalize_pending_reports_blocked_outcome_for_no_pending_work() {
        let (_db_dir, conn) = open_test_db_file_any();
        let root = tempfile::TempDir::new().unwrap();
        insert_collection(&conn, "work", root.path());

        let error = sync(
            &conn,
            CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: true,
                online: false,
            },
            true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("FinalizePendingBlockedError"));
        assert!(error.to_string().contains("NoPendingWork"));
    }

    #[test]
    fn sync_remap_root_updates_collection_and_reports_summary_in_plain_text_mode() {
        let (_db_dir, conn) = open_test_db_file_any();
        let source_root = tempfile::TempDir::new().unwrap();
        let remapped_root = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(remapped_root.path().join("notes")).unwrap();
        let collection_id = insert_collection(&conn, "work", source_root.path());
        let raw_bytes =
            b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\ntitle: Remapped Note\ntype: concept\n---\nhello from remap\n";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "11111111-1111-7111-8111-111111111111",
            raw_bytes,
            "notes/a.md",
        );
        fs::write(remapped_root.path().join("notes").join("a.md"), raw_bytes).unwrap();

        sync(
            &conn,
            CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: Some(remapped_root.path().to_path_buf()),
                finalize_pending: false,
                online: false,
            },
            false,
        )
        .unwrap();

        let row: (String, String, i64) = conn
            .query_row(
                "SELECT root_path, state, needs_full_sync FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, remapped_root.path().display().to_string());
        assert_eq!(row.1, "restoring");
        assert_eq!(row.2, 1);
    }

    #[test]
    fn restore_helper_materializes_pages_and_reports_counts_in_plain_text_mode() {
        let (_db_dir, conn) = open_test_db_file_any();
        let source_root = tempfile::TempDir::new().unwrap();
        let target_parent = tempfile::TempDir::new().unwrap();
        let target_root = target_parent.path().join("restored");
        let collection_id = insert_collection(&conn, "work", source_root.path());
        let raw_bytes =
            b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\ntitle: Restored Note\ntype: concept\n---\nhello from restore\n";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "notes/a",
            "22222222-2222-7222-8222-222222222222",
            raw_bytes,
            "notes/a.md",
        );

        restore(
            &conn,
            CollectionRestoreArgs {
                name: "work".to_owned(),
                target: target_root.clone(),
                online: false,
            },
            false,
        )
        .unwrap();

        assert_eq!(
            fs::read(target_root.join("notes").join("a.md")).unwrap(),
            raw_bytes
        );
        let row: (String, String, i64, Option<String>) = conn
            .query_row(
                "SELECT state, root_path, needs_full_sync, pending_root_path
                 FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "restoring");
        assert_eq!(row.1, target_root.display().to_string());
        assert_eq!(row.2, 1);
        assert!(row.3.is_none());
    }

    #[test]
    fn ignore_helper_round_trips_patterns_cross_platform() {
        let (_db_dir, conn) = open_test_db_file_any();
        let root = tempfile::TempDir::new().unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET state = 'detached' WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        ignore_add(&conn, "work", "note.md", true).unwrap();
        let collection = load_collection_by_name(&conn, "work").unwrap();
        assert_eq!(
            cached_user_ignore_patterns(&collection).unwrap(),
            vec!["note.md"]
        );

        ignore_list(&conn, "work", true).unwrap();
        ignore_remove(&conn, "work", "note.md", true).unwrap();
        let collection = load_collection_by_name(&conn, "work").unwrap();
        assert!(cached_user_ignore_patterns(&collection).unwrap().is_empty());

        ignore_clear(&conn, "work", true).unwrap();
        assert!(!root.path().join(".quaidignore").exists());
    }

    #[cfg(not(unix))]
    #[test]
    fn sync_helper_fails_closed_when_reconcile_backend_is_unavailable() {
        let (_db_dir, conn) = open_test_db_file_any();
        let root = tempfile::TempDir::new().unwrap();
        fs::write(
            root.path().join("note.md"),
            "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nreconcile me\n",
        )
        .unwrap();
        let collection_id = insert_collection(&conn, "work", root.path());
        conn.execute(
            "UPDATE collections SET needs_full_sync = 1 WHERE id = ?1",
            [collection_id],
        )
        .unwrap();

        let error = sync(
            &conn,
            CollectionSyncArgs {
                name: "work".to_owned(),
                remap_root: None,
                finalize_pending: false,
                online: false,
            },
            true,
        )
        .unwrap_err();

        let row: (String, i64) = conn
            .query_row(
                "SELECT state, needs_full_sync FROM collections WHERE id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(error
            .to_string()
            .contains("Vault sync commands (serve, collection add/sync) require Unix"));
        assert_eq!(row.0, "active");
        assert_eq!(row.1, 1);
    }
}
