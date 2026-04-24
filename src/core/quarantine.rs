use std::collections::HashMap;
use std::fs;
#[cfg(unix)]
use std::io::Write;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
#[cfg(unix)]
use sha2::{Digest, Sha256};
use thiserror::Error;
#[cfg(unix)]
use uuid::Uuid;

use crate::core::collections::{self, OpKind};
#[cfg(unix)]
use crate::core::file_state;
#[cfg(unix)]
use crate::core::fs_safety;
use crate::core::markdown;
use crate::core::page_uuid;
#[cfg(unix)]
use crate::core::palace;
#[cfg(unix)]
use crate::core::raw_imports;
use crate::core::reconciler;
use crate::core::types::{Page, TimelineEntry};
use crate::core::vault_sync::{self, ResolvedSlug, VaultSyncError};

#[cfg(unix)]
use rustix::fd::AsFd;
#[cfg(unix)]
use rustix::fs::fsync;

const DEFAULT_QUARANTINE_TTL_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct DbOnlyStateCounts {
    pub programmatic_links: i64,
    pub non_import_assertions: i64,
    pub raw_data: i64,
    pub contradictions: i64,
    pub knowledge_gaps: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuarantinedPageView {
    pub collection: String,
    pub slug: String,
    pub address: String,
    pub title: String,
    pub quarantined_at: String,
    pub exported_at: Option<String>,
    pub db_only_state: DbOnlyStateCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuarantineExportReceipt {
    pub collection: String,
    pub slug: String,
    pub quarantined_at: String,
    pub exported_at: String,
    pub output_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuarantineDiscardReceipt {
    pub collection: String,
    pub slug: String,
    pub quarantined_at: String,
    pub forced: bool,
    pub exported_before_discard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuarantineRestoreReceipt {
    pub collection: String,
    pub slug: String,
    pub restored_slug: String,
    pub restored_relative_path: String,
    pub quarantined_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct QuarantineSweepSummary {
    pub discarded: usize,
    pub skipped_db_only_state: usize,
}

#[derive(Debug, Clone, Serialize)]
struct QuarantineExportPayload {
    page_id: i64,
    collection: String,
    slug: String,
    quarantined_at: String,
    exported_at: String,
    page: Page,
    rendered_markdown: String,
    active_raw_markdown: Option<String>,
    programmatic_links: Vec<ProgrammaticLinkExport>,
    non_import_assertions: Vec<AssertionExport>,
    raw_data_rows: Vec<RawDataExport>,
    contradictions: Vec<ContradictionExport>,
    knowledge_gaps: Vec<KnowledgeGapExport>,
    tags: Vec<String>,
    timeline_entries: Vec<TimelineEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticLinkExport {
    id: i64,
    from_slug: String,
    to_slug: String,
    relationship: String,
    context: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
    created_at: String,
}

#[derive(Debug, Clone, Serialize)]
struct AssertionExport {
    id: i64,
    subject: String,
    predicate: String,
    object: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
    supersedes_id: Option<i64>,
    asserted_by: String,
    source_ref: String,
    evidence_text: String,
    created_at: String,
}

#[derive(Debug, Clone, Serialize)]
struct RawDataExport {
    id: i64,
    source: String,
    data: String,
    fetched_at: String,
}

#[derive(Debug, Clone, Serialize)]
struct ContradictionExport {
    id: i64,
    page_slug: String,
    other_page_slug: Option<String>,
    r#type: String,
    description: String,
    detected_at: String,
    resolved_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct KnowledgeGapExport {
    id: i64,
    query_hash: String,
    query_text: Option<String>,
    context: String,
    confidence_score: Option<f64>,
    sensitivity: String,
    approved_by: Option<String>,
    approved_at: Option<String>,
    redacted_query: Option<String>,
    resolved_at: Option<String>,
    resolved_by_slug: Option<String>,
    detected_at: String,
}

#[derive(Debug, Clone)]
struct QuarantinedPageRecord {
    page_id: i64,
    collection_name: String,
    slug: String,
    title: String,
    uuid: String,
    page_type: String,
    summary: String,
    compiled_truth: String,
    timeline: String,
    frontmatter: HashMap<String, String>,
    wing: String,
    room: String,
    version: i64,
    created_at: String,
    updated_at: String,
    truth_updated_at: String,
    timeline_updated_at: String,
    quarantined_at: String,
}

impl QuarantinedPageRecord {
    fn page(&self) -> Page {
        Page {
            slug: self.slug.clone(),
            uuid: self.uuid.clone(),
            page_type: self.page_type.clone(),
            title: self.title.clone(),
            summary: self.summary.clone(),
            compiled_truth: self.compiled_truth.clone(),
            timeline: self.timeline.clone(),
            frontmatter: self.frontmatter.clone(),
            wing: self.wing.clone(),
            room: self.room.clone(),
            version: self.version,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            truth_updated_at: self.truth_updated_at.clone(),
            timeline_updated_at: self.timeline_updated_at.clone(),
        }
    }
}

#[derive(Debug, Error)]
pub enum QuarantineError {
    #[error("quarantine collection not found: {collection}")]
    CollectionNotFound { collection: String },

    #[error("page is not quarantined: {slug}")]
    NotQuarantined { slug: String },

    #[error(
        "QuarantineDiscardExportRequiredError: slug={slug} programmatic_links={} non_import_assertions={} raw_data={} contradictions={} knowledge_gaps={}",
        counts.programmatic_links,
        counts.non_import_assertions,
        counts.raw_data,
        counts.contradictions,
        counts.knowledge_gaps
    )]
    ExportRequired {
        slug: String,
        counts: DbOnlyStateCounts,
    },

    #[cfg(unix)]
    #[error("QuarantineRestoreTargetNotMarkdownError: target={target}")]
    RestoreTargetNotMarkdown { target: String },

    #[cfg(unix)]
    #[error("QuarantineRestoreTargetOccupiedError: target={target}")]
    RestoreTargetOccupied { target: String },

    #[cfg(unix)]
    #[error("QuarantineRestorePathOwnedError: target={target} owner_slug={owner_slug}")]
    RestorePathOwned { target: String, owner_slug: String },

    #[cfg(unix)]
    #[error("QuarantineRestoreHookError: {message}")]
    RestoreHook { message: String },

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Collections(#[from] collections::CollectionError),

    #[error(transparent)]
    Reconcile(#[from] reconciler::ReconcileError),

    #[error(transparent)]
    VaultSync(#[from] VaultSyncError),

    #[error(transparent)]
    PageUuid(#[from] page_uuid::PageUuidError),
}

pub fn quarantine_ttl_days() -> i64 {
    std::env::var("GBRAIN_QUARANTINE_TTL_DAYS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value >= 0)
        .unwrap_or(DEFAULT_QUARANTINE_TTL_DAYS)
}

pub fn list_collection_quarantine(
    conn: &Connection,
    collection_name: &str,
) -> Result<Vec<QuarantinedPageView>, QuarantineError> {
    let collection = collections::get_by_name(conn, collection_name)?.ok_or_else(|| {
        QuarantineError::CollectionNotFound {
            collection: collection_name.to_owned(),
        }
    })?;
    let mut stmt = conn.prepare(
        "SELECT p.id, p.slug, p.title, p.quarantined_at,
                qe.exported_at
         FROM pages p
         LEFT JOIN quarantine_exports qe
           ON qe.page_id = p.id
          AND qe.quarantined_at = p.quarantined_at
         WHERE p.collection_id = ?1
           AND p.quarantined_at IS NOT NULL
         ORDER BY p.quarantined_at, p.slug",
    )?;
    let rows = stmt
        .query_map([collection.id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(|(page_id, slug, title, quarantined_at, exported_at)| {
            Ok(QuarantinedPageView {
                collection: collection.name.clone(),
                address: format!("{}::{}", collection.name, slug),
                slug,
                title,
                quarantined_at,
                exported_at,
                db_only_state: db_only_state_counts(conn, page_id)?,
            })
        })
        .collect()
}

pub fn export_quarantined_page(
    conn: &Connection,
    slug_input: &str,
    output_path: &Path,
) -> Result<QuarantineExportReceipt, QuarantineError> {
    let resolved = vault_sync::resolve_slug_for_op(conn, slug_input, OpKind::Read)?;
    let page = load_quarantined_page(conn, &resolved)?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let exported_at = current_timestamp(conn)?;
    let payload = build_export_payload(conn, &page, &exported_at)?;
    fs::write(output_path, serde_json::to_vec_pretty(&payload)?)?;
    record_quarantine_export(
        conn,
        page.page_id,
        &page.quarantined_at,
        &exported_at,
        output_path.display().to_string(),
    )?;
    eprintln!(
        "INFO: quarantine_exported collection={} slug={} output_path={}",
        page.collection_name,
        page.slug,
        output_path.display()
    );
    Ok(QuarantineExportReceipt {
        collection: page.collection_name,
        slug: page.slug,
        quarantined_at: page.quarantined_at,
        exported_at,
        output_path: output_path.display().to_string(),
    })
}

pub fn discard_quarantined_page(
    conn: &Connection,
    slug_input: &str,
    force: bool,
) -> Result<QuarantineDiscardReceipt, QuarantineError> {
    let resolved = vault_sync::resolve_slug_for_op(conn, slug_input, OpKind::WriteUpdate)?;
    vault_sync::ensure_collection_write_allowed(conn, resolved.collection_id)?;
    let page = load_quarantined_page(conn, &resolved)?;
    let has_db_only_state = reconciler::has_db_only_state(conn, page.page_id)?;
    let exported_before_discard = has_current_export(conn, page.page_id, &page.quarantined_at)?;
    if has_db_only_state && !force && !exported_before_discard {
        let counts = db_only_state_counts(conn, page.page_id)?;
        return Err(QuarantineError::ExportRequired {
            slug: page.slug,
            counts,
        });
    }
    conn.execute("DELETE FROM pages WHERE id = ?1", [page.page_id])?;
    eprintln!(
        "INFO: quarantine_discarded collection={} slug={} forced={} exported_before_discard={}",
        page.collection_name, page.slug, force, exported_before_discard
    );
    Ok(QuarantineDiscardReceipt {
        collection: page.collection_name,
        slug: page.slug,
        quarantined_at: page.quarantined_at,
        forced: force,
        exported_before_discard,
    })
}

#[cfg(unix)]
pub fn restore_quarantined_page(
    conn: &Connection,
    slug_input: &str,
    relative_path_input: &str,
) -> Result<QuarantineRestoreReceipt, QuarantineError> {
    let resolved = vault_sync::resolve_slug_for_op(conn, slug_input, OpKind::WriteUpdate)?;
    vault_sync::ensure_collection_vault_write_allowed(conn, resolved.collection_id)?;
    let _lease = vault_sync::start_short_lived_owner_lease(conn, resolved.collection_id)?;
    let page = load_quarantined_page(conn, &resolved)?;
    let collection = collections::get_by_name(conn, &page.collection_name)?.ok_or_else(|| {
        QuarantineError::CollectionNotFound {
            collection: page.collection_name.clone(),
        }
    })?;
    let normalized_relative_path = normalize_restore_relative_path(relative_path_input)?;
    refuse_restore_path_owned_by_other_page(
        conn,
        resolved.collection_id,
        &normalized_relative_path,
        page.page_id,
    )?;

    let root_path = Path::new(&collection.root_path);
    let raw_bytes = active_raw_import_bytes(conn, page.page_id)?;
    let sha256 = sha256_hex(&raw_bytes);
    let root_fd = fs_safety::open_root_fd(root_path)?;
    let target_relative_path = Path::new(&normalized_relative_path);
    // Use walk_to_parent (not walk_to_parent_create_dirs): absent parent directories are refused
    // rather than silently created without a durable fsync chain. Callers must pre-create the
    // target directory structure before restoring.
    let parent_fd = fs_safety::walk_to_parent(&root_fd, target_relative_path)?;
    let target_name =
        target_relative_path
            .file_name()
            .ok_or_else(|| QuarantineError::RestoreHook {
                message: format!(
                    "relative path has no terminal component: {}",
                    target_relative_path.display()
                ),
            })?;
    let absolute_target_path = root_path.join(&normalized_relative_path);
    refuse_existing_target(&parent_fd, target_name, &absolute_target_path)?;
    maybe_pause_after_precheck()?;

    let temp_name = PathBuf::from(format!(".quarantine-restore-{}.tmp", Uuid::now_v7()));
    let mut temp_file = std::fs::File::from(fs_safety::openat_create_excl(&parent_fd, &temp_name)?);

    // Test hook: simulate failure immediately after tempfile creation (before write/sync).
    if should_fail_after_tempfile_create() {
        drop(temp_file);
        cleanup_tempfile(&parent_fd, &temp_name)?;
        return Err(QuarantineError::RestoreHook {
            message: "injected failure after tempfile create".to_owned(),
        });
    }

    // write_all / sync_all failure must clean up the tempfile before propagating the error,
    // otherwise a partial `.quarantine-restore-*.tmp` is left on disk across a crash restart.
    if let Err(err) = temp_file.write_all(&raw_bytes).and_then(|()| temp_file.sync_all()) {
        drop(temp_file);
        cleanup_tempfile(&parent_fd, &temp_name)?;
        return Err(err.into());
    }
    drop(temp_file);

    let install_result = install_tempfile_without_replace(&parent_fd, &temp_name, target_name);
    if let Err(err) = install_result {
        cleanup_tempfile(&parent_fd, &temp_name)?;
        return Err(match err.kind() {
            std::io::ErrorKind::AlreadyExists => QuarantineError::RestoreTargetOccupied {
                target: absolute_target_path.display().to_string(),
            },
            _ => err.into(),
        });
    }

    if let Err(err) = cleanup_tempfile(&parent_fd, &temp_name) {
        rollback_target_entry(&parent_fd, target_name)?;
        return Err(err);
    }

    if should_fail_after_install_before_db() {
        rollback_target_entry(&parent_fd, target_name)?;
        return Err(QuarantineError::RestoreHook {
            message: "injected failure after install and before DB reactivation".to_owned(),
        });
    }

    let target_stat = match file_state::stat_file_fd(&parent_fd, target_name) {
        Ok(stat) => stat,
        Err(err) => {
            rollback_target_entry(&parent_fd, target_name)?;
            return Err(err.into());
        }
    };
    // parse_restored_page can fail (UUID conflict, JSON serialization). On failure the
    // installed target must be rolled back before returning so the vault is not left with
    // an orphaned file while the page remains quarantined in the DB.
    let parsed = match parse_restored_page(&raw_bytes, &absolute_target_path, root_path, &page.uuid) {
        Ok(p) => p,
        Err(err) => {
            rollback_target_entry(&parent_fd, target_name)?;
            return Err(err);
        }
    };

    let tx = conn.unchecked_transaction()?;
    let now = current_timestamp(&tx)?;
    let update_result = restore_page_transaction(
        &tx,
        &page,
        resolved.collection_id,
        &normalized_relative_path,
        &parsed,
        &target_stat,
        &sha256,
        &now,
    );
    match update_result.and_then(|()| tx.commit().map_err(QuarantineError::from)) {
        Ok(()) => {
            eprintln!(
                "INFO: quarantine_restored collection={} slug={} restored_slug={} relative_path={}",
                page.collection_name, page.slug, parsed.slug, normalized_relative_path
            );
            Ok(QuarantineRestoreReceipt {
                collection: page.collection_name,
                slug: page.slug,
                restored_slug: parsed.slug,
                restored_relative_path: normalized_relative_path,
                quarantined_at: page.quarantined_at,
            })
        }
        Err(err) => {
            rollback_target_entry(&parent_fd, target_name)?;
            Err(err)
        }
    }
}

#[cfg(not(unix))]
pub fn restore_quarantined_page(
    _conn: &Connection,
    _slug_input: &str,
    _relative_path_input: &str,
) -> Result<QuarantineRestoreReceipt, QuarantineError> {
    Err(VaultSyncError::UnsupportedPlatform {
        command: "gbrain collection quarantine restore",
    }
    .into())
}

pub fn sweep_expired_quarantined_pages(
    conn: &Connection,
) -> Result<QuarantineSweepSummary, QuarantineError> {
    let ttl_days = quarantine_ttl_days();
    let mut stmt = conn.prepare(
        "SELECT id, collection_id, slug, quarantined_at
         FROM pages
         WHERE quarantined_at IS NOT NULL
           AND CAST(strftime('%s', 'now') - strftime('%s', quarantined_at) AS INTEGER) >= (?1 * 86400)
         ORDER BY quarantined_at, id",
    )?;
    let rows = stmt
        .query_map([ttl_days], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    let mut summary = QuarantineSweepSummary::default();
    for (page_id, collection_id, slug, quarantined_at) in rows {
        if reconciler::has_db_only_state(conn, page_id)? {
            summary.skipped_db_only_state += 1;
            let counts = db_only_state_counts(conn, page_id)?;
            eprintln!(
                "DEBUG: quarantine_ttl_skip collection_id={} slug={} quarantined_at={} programmatic_links={} non_import_assertions={} raw_data={} contradictions={} knowledge_gaps={}",
                collection_id,
                slug,
                quarantined_at,
                counts.programmatic_links,
                counts.non_import_assertions,
                counts.raw_data,
                counts.contradictions,
                counts.knowledge_gaps
            );
            continue;
        }
        conn.execute("DELETE FROM pages WHERE id = ?1", [page_id])?;
        summary.discarded += 1;
        eprintln!(
            "INFO: quarantine_ttl_discard collection_id={} slug={} quarantined_at={}",
            collection_id, slug, quarantined_at
        );
    }
    Ok(summary)
}

pub fn db_only_state_counts(
    conn: &Connection,
    page_id: i64,
) -> Result<DbOnlyStateCounts, QuarantineError> {
    Ok(DbOnlyStateCounts {
        programmatic_links: count(
            conn,
            "SELECT COUNT(*)
             FROM links
             WHERE (from_page_id = ?1 OR to_page_id = ?1)
               AND source_kind = 'programmatic'",
            [page_id],
        )?,
        non_import_assertions: count(
            conn,
            "SELECT COUNT(*)
             FROM assertions
             WHERE page_id = ?1
               AND asserted_by != 'import'",
            [page_id],
        )?,
        raw_data: count(
            conn,
            "SELECT COUNT(*)
             FROM raw_data
             WHERE page_id = ?1",
            [page_id],
        )?,
        contradictions: count(
            conn,
            "SELECT COUNT(*)
             FROM contradictions
             WHERE page_id = ?1 OR other_page_id = ?1",
            [page_id],
        )?,
        knowledge_gaps: count(
            conn,
            "SELECT COUNT(*)
             FROM knowledge_gaps
             WHERE page_id = ?1",
            [page_id],
        )?,
    })
}

fn build_export_payload(
    conn: &Connection,
    page: &QuarantinedPageRecord,
    exported_at: &str,
) -> Result<QuarantineExportPayload, QuarantineError> {
    Ok(QuarantineExportPayload {
        page_id: page.page_id,
        collection: page.collection_name.clone(),
        slug: page.slug.clone(),
        quarantined_at: page.quarantined_at.clone(),
        exported_at: exported_at.to_owned(),
        page: page.page(),
        rendered_markdown: markdown::render_page(&page.page()),
        active_raw_markdown: active_raw_import_markdown(conn, page.page_id)?,
        programmatic_links: load_programmatic_links(conn, page.page_id)?,
        non_import_assertions: load_non_import_assertions(conn, page.page_id)?,
        raw_data_rows: load_raw_data_rows(conn, page.page_id)?,
        contradictions: load_contradictions(conn, page.page_id)?,
        knowledge_gaps: load_knowledge_gaps(conn, page.page_id)?,
        tags: load_tags(conn, page.page_id)?,
        timeline_entries: load_timeline_entries(conn, page.page_id)?,
    })
}

fn load_quarantined_page(
    conn: &Connection,
    resolved: &ResolvedSlug,
) -> Result<QuarantinedPageRecord, QuarantineError> {
    conn.query_row(
        "SELECT p.id, p.collection_id, c.name, p.slug, p.title, COALESCE(p.uuid, ''),
                p.type, p.summary, p.compiled_truth, p.timeline, p.frontmatter,
                p.wing, p.room, p.version, p.created_at, p.updated_at,
                p.truth_updated_at, p.timeline_updated_at, p.quarantined_at
         FROM pages p
         JOIN collections c ON c.id = p.collection_id
         WHERE p.collection_id = ?1
           AND p.slug = ?2
           AND p.quarantined_at IS NOT NULL",
        params![resolved.collection_id, resolved.slug],
        |row| {
            let frontmatter_raw: String = row.get(10)?;
            let frontmatter =
                serde_json::from_str(&frontmatter_raw).unwrap_or_else(|_| HashMap::new());
            Ok(QuarantinedPageRecord {
                page_id: row.get(0)?,
                collection_name: row.get(2)?,
                slug: row.get(3)?,
                title: row.get(4)?,
                uuid: row.get(5)?,
                page_type: row.get(6)?,
                summary: row.get(7)?,
                compiled_truth: row.get(8)?,
                timeline: row.get(9)?,
                frontmatter,
                wing: row.get(11)?,
                room: row.get(12)?,
                version: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
                truth_updated_at: row.get(16)?,
                timeline_updated_at: row.get(17)?,
                quarantined_at: row.get(18)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| QuarantineError::NotQuarantined {
        slug: resolved.canonical_slug(),
    })
}

fn record_quarantine_export(
    conn: &Connection,
    page_id: i64,
    quarantined_at: &str,
    exported_at: &str,
    output_path: String,
) -> Result<(), QuarantineError> {
    conn.execute(
        "INSERT INTO quarantine_exports (page_id, quarantined_at, output_path, exported_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(page_id, quarantined_at) DO UPDATE SET
             exported_at = excluded.exported_at,
             output_path = excluded.output_path",
        params![page_id, quarantined_at, output_path, exported_at],
    )?;
    Ok(())
}

fn has_current_export(
    conn: &Connection,
    page_id: i64,
    quarantined_at: &str,
) -> Result<bool, QuarantineError> {
    Ok(conn.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM quarantine_exports
             WHERE page_id = ?1
               AND quarantined_at = ?2
         )",
        params![page_id, quarantined_at],
        |row| row.get::<_, i64>(0),
    )? != 0)
}

fn load_programmatic_links(
    conn: &Connection,
    page_id: i64,
) -> Result<Vec<ProgrammaticLinkExport>, QuarantineError> {
    let mut stmt = conn.prepare(
        "SELECT l.id, from_p.slug, to_p.slug, l.relationship, l.context,
                l.valid_from, l.valid_until, l.created_at
         FROM links l
         JOIN pages from_p ON from_p.id = l.from_page_id
         JOIN pages to_p ON to_p.id = l.to_page_id
         WHERE (l.from_page_id = ?1 OR l.to_page_id = ?1)
           AND l.source_kind = 'programmatic'
         ORDER BY l.id",
    )?;
    let rows = stmt.query_map([page_id], |row| {
        Ok(ProgrammaticLinkExport {
            id: row.get(0)?,
            from_slug: row.get(1)?,
            to_slug: row.get(2)?,
            relationship: row.get(3)?,
            context: row.get(4)?,
            valid_from: row.get(5)?,
            valid_until: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_non_import_assertions(
    conn: &Connection,
    page_id: i64,
) -> Result<Vec<AssertionExport>, QuarantineError> {
    let mut stmt = conn.prepare(
        "SELECT id, subject, predicate, object, valid_from, valid_until,
                supersedes_id, asserted_by, source_ref, evidence_text, created_at
         FROM assertions
         WHERE page_id = ?1
           AND asserted_by != 'import'
         ORDER BY id",
    )?;
    let rows = stmt.query_map([page_id], |row| {
        Ok(AssertionExport {
            id: row.get(0)?,
            subject: row.get(1)?,
            predicate: row.get(2)?,
            object: row.get(3)?,
            valid_from: row.get(4)?,
            valid_until: row.get(5)?,
            supersedes_id: row.get(6)?,
            asserted_by: row.get(7)?,
            source_ref: row.get(8)?,
            evidence_text: row.get(9)?,
            created_at: row.get(10)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_raw_data_rows(
    conn: &Connection,
    page_id: i64,
) -> Result<Vec<RawDataExport>, QuarantineError> {
    let mut stmt = conn.prepare(
        "SELECT id, source, data, fetched_at
         FROM raw_data
         WHERE page_id = ?1
         ORDER BY id",
    )?;
    let rows = stmt.query_map([page_id], |row| {
        Ok(RawDataExport {
            id: row.get(0)?,
            source: row.get(1)?,
            data: row.get(2)?,
            fetched_at: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_contradictions(
    conn: &Connection,
    page_id: i64,
) -> Result<Vec<ContradictionExport>, QuarantineError> {
    let mut stmt = conn.prepare(
        "SELECT c.id, p.slug, other_p.slug, c.type, c.description, c.detected_at, c.resolved_at
         FROM contradictions c
         JOIN pages p ON p.id = c.page_id
         LEFT JOIN pages other_p ON other_p.id = c.other_page_id
         WHERE c.page_id = ?1 OR c.other_page_id = ?1
         ORDER BY c.id",
    )?;
    let rows = stmt.query_map([page_id], |row| {
        Ok(ContradictionExport {
            id: row.get(0)?,
            page_slug: row.get(1)?,
            other_page_slug: row.get(2)?,
            r#type: row.get(3)?,
            description: row.get(4)?,
            detected_at: row.get(5)?,
            resolved_at: row.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_knowledge_gaps(
    conn: &Connection,
    page_id: i64,
) -> Result<Vec<KnowledgeGapExport>, QuarantineError> {
    let mut stmt = conn.prepare(
        "SELECT id, query_hash, query_text, context, confidence_score, sensitivity,
                approved_by, approved_at, redacted_query, resolved_at, resolved_by_slug, detected_at
         FROM knowledge_gaps
         WHERE page_id = ?1
         ORDER BY id",
    )?;
    let rows = stmt.query_map([page_id], |row| {
        Ok(KnowledgeGapExport {
            id: row.get(0)?,
            query_hash: row.get(1)?,
            query_text: row.get(2)?,
            context: row.get(3)?,
            confidence_score: row.get(4)?,
            sensitivity: row.get(5)?,
            approved_by: row.get(6)?,
            approved_at: row.get(7)?,
            redacted_query: row.get(8)?,
            resolved_at: row.get(9)?,
            resolved_by_slug: row.get(10)?,
            detected_at: row.get(11)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_tags(conn: &Connection, page_id: i64) -> Result<Vec<String>, QuarantineError> {
    let mut stmt = conn.prepare(
        "SELECT tag
         FROM tags
         WHERE page_id = ?1
         ORDER BY tag",
    )?;
    let rows = stmt.query_map([page_id], |row| row.get::<_, String>(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_timeline_entries(
    conn: &Connection,
    page_id: i64,
) -> Result<Vec<TimelineEntry>, QuarantineError> {
    let mut stmt = conn.prepare(
        "SELECT id, page_id, date, source, summary, detail, created_at
         FROM timeline_entries
         WHERE page_id = ?1
         ORDER BY date, id",
    )?;
    let rows = stmt.query_map([page_id], |row| {
        Ok(TimelineEntry {
            id: row.get(0)?,
            page_id: row.get(1)?,
            date: row.get(2)?,
            source: row.get(3)?,
            summary: row.get(4)?,
            detail: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn active_raw_import_markdown(
    conn: &Connection,
    page_id: i64,
) -> Result<Option<String>, QuarantineError> {
    conn.query_row(
        "SELECT raw_bytes
         FROM raw_imports
         WHERE page_id = ?1
           AND is_active = 1",
        [page_id],
        |row| row.get::<_, Vec<u8>>(0),
    )
    .optional()
    .map(|raw| raw.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
    .map_err(Into::into)
}

#[cfg(unix)]
fn active_raw_import_bytes(conn: &Connection, page_id: i64) -> Result<Vec<u8>, QuarantineError> {
    conn.query_row(
        "SELECT raw_bytes
         FROM raw_imports
         WHERE page_id = ?1
           AND is_active = 1",
        [page_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

#[cfg(unix)]
fn normalize_restore_relative_path(relative_path_input: &str) -> Result<String, QuarantineError> {
    let normalized = if relative_path_input.ends_with(".md") {
        relative_path_input.to_owned()
    } else if Path::new(relative_path_input).extension().is_some() {
        return Err(QuarantineError::RestoreTargetNotMarkdown {
            target: relative_path_input.to_owned(),
        });
    } else {
        format!("{relative_path_input}.md")
    };
    collections::validate_relative_path(&normalized)?;
    Ok(normalized)
}

#[cfg(unix)]
fn refuse_restore_path_owned_by_other_page(
    conn: &Connection,
    collection_id: i64,
    relative_path: &str,
    restoring_page_id: i64,
) -> Result<(), QuarantineError> {
    if let Some((owner_page_id, owner_slug)) = conn
        .query_row(
            "SELECT p.id, p.slug
             FROM file_state f
             JOIN pages p ON p.id = f.page_id
             WHERE f.collection_id = ?1
               AND f.relative_path = ?2",
            params![collection_id, relative_path],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    {
        if owner_page_id != restoring_page_id {
            return Err(QuarantineError::RestorePathOwned {
                target: relative_path.to_owned(),
                owner_slug,
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn refuse_existing_target<Fd: AsFd>(
    parent_fd: Fd,
    target_name: &Path,
    absolute_target_path: &Path,
) -> Result<(), QuarantineError> {
    match fs_safety::stat_at_nofollow(parent_fd, target_name) {
        Ok(_) => Err(QuarantineError::RestoreTargetOccupied {
            target: absolute_target_path.display().to_string(),
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(unix)]
fn install_tempfile_without_replace<Fd: AsFd>(
    parent_fd: Fd,
    temp_name: &Path,
    target_name: &Path,
) -> std::io::Result<()> {
    fs_safety::linkat_parent_fd(parent_fd, temp_name, target_name)
}

#[cfg(unix)]
fn cleanup_tempfile<Fd: AsFd>(parent_fd: Fd, temp_name: &Path) -> Result<(), QuarantineError> {
    remove_entry_and_sync_parent(parent_fd, temp_name, "temp")
}

#[cfg(unix)]
fn rollback_target_entry<Fd: AsFd>(
    parent_fd: Fd,
    target_name: &Path,
) -> Result<(), QuarantineError> {
    remove_entry_and_sync_parent(parent_fd, target_name, "target")
}

#[cfg(unix)]
fn remove_entry_and_sync_parent<Fd: AsFd>(
    parent_fd: Fd,
    name: &Path,
    trace_label: &'static str,
) -> Result<(), QuarantineError> {
    match fs_safety::unlinkat_parent_fd(&parent_fd, name) {
        Ok(()) => {
            append_restore_trace(&format!("unlink:{trace_label}"))?;
            sync_parent_fd(&parent_fd)?;
            append_restore_trace(&format!("fsync-after-unlink:{trace_label}"))?;
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(unix)]
fn sync_parent_fd<Fd: AsFd>(parent_fd: Fd) -> Result<(), QuarantineError> {
    fsync(parent_fd.as_fd())
        .map_err(|err| std::io::Error::from_raw_os_error(err.raw_os_error()).into())
}

#[cfg(unix)]
fn parse_restored_page(
    raw_bytes: &[u8],
    absolute_target_path: &Path,
    root_path: &Path,
    stored_uuid: &str,
) -> Result<ParsedRestoredPage, QuarantineError> {
    // Test hook: simulate a parse failure to exercise post-install rollback.
    if should_fail_in_parse() {
        return Err(QuarantineError::RestoreHook {
            message: "injected parse failure".to_owned(),
        });
    }
    let raw = String::from_utf8_lossy(raw_bytes).into_owned();
    let (frontmatter, body) = markdown::parse_frontmatter(&raw);
    let (compiled_truth, timeline) = markdown::split_content(&body);
    let summary = markdown::extract_summary(&compiled_truth);
    let slug = frontmatter
        .get("slug")
        .cloned()
        .unwrap_or_else(|| derive_slug_from_path(absolute_target_path, root_path));
    let title = frontmatter
        .get("title")
        .cloned()
        .unwrap_or_else(|| slug.clone());
    let page_type = frontmatter
        .get("type")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("null"))
        .map(str::to_owned)
        .unwrap_or_else(|| "concept".to_owned());
    let wing = frontmatter
        .get("wing")
        .cloned()
        .unwrap_or_else(|| palace::derive_wing(&slug));
    let room = palace::derive_room(&compiled_truth);
    let uuid = page_uuid::resolve_page_uuid(&frontmatter, Some(stored_uuid))?;
    Ok(ParsedRestoredPage {
        slug,
        uuid,
        title,
        page_type,
        summary,
        compiled_truth,
        timeline,
        frontmatter_json: serde_json::to_string(&frontmatter)?,
        wing,
        room,
    })
}

#[cfg(unix)]
struct ParsedRestoredPage {
    slug: String,
    uuid: String,
    title: String,
    page_type: String,
    summary: String,
    compiled_truth: String,
    timeline: String,
    frontmatter_json: String,
    wing: String,
    room: String,
}

#[cfg(unix)]
fn restore_page_transaction(
    conn: &Connection,
    page: &QuarantinedPageRecord,
    collection_id: i64,
    relative_path: &str,
    parsed: &ParsedRestoredPage,
    stat: &file_state::FileStat,
    sha256: &str,
    now: &str,
) -> Result<(), QuarantineError> {
    conn.execute(
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
        params![
            parsed.slug,
            parsed.uuid,
            parsed.page_type,
            parsed.title,
            parsed.summary,
            parsed.compiled_truth,
            parsed.timeline,
            parsed.frontmatter_json,
            parsed.wing,
            parsed.room,
            now,
            page.page_id
        ],
    )?;
    file_state::upsert_file_state(
        conn,
        collection_id,
        relative_path,
        page.page_id,
        stat,
        sha256,
    )?;
    conn.execute(
        "DELETE FROM file_state
         WHERE page_id = ?1
           AND relative_path != ?2",
        params![page.page_id, relative_path],
    )?;
    raw_imports::assert_exactly_one_active_row(conn, page.page_id)?;
    raw_imports::enqueue_embedding_job(conn, page.page_id)?;
    Ok(())
}

#[cfg(unix)]
fn derive_slug_from_path(file_path: &Path, root_path: &Path) -> String {
    file_path
        .strip_prefix(root_path)
        .unwrap_or(file_path)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(unix)]
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(unix)]
fn maybe_pause_after_precheck() -> Result<(), QuarantineError> {
    let Some(flag_path) = std::env::var_os("GBRAIN_TEST_QUARANTINE_RESTORE_PAUSE_FILE") else {
        return Ok(());
    };
    let flag_path = PathBuf::from(flag_path);
    fs::write(&flag_path, b"ready")?;
    while flag_path.exists() {
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[cfg(unix)]
fn should_fail_after_tempfile_create() -> bool {
    std::env::var("GBRAIN_TEST_QUARANTINE_RESTORE_FAIL_AFTER_TEMPFILE_CREATE")
        .ok()
        .as_deref()
        == Some("1")
}

#[cfg(unix)]
fn should_fail_after_install_before_db() -> bool {
    std::env::var("GBRAIN_TEST_QUARANTINE_RESTORE_FAIL_AFTER_INSTALL")
        .ok()
        .as_deref()
        == Some("1")
}

#[cfg(unix)]
fn should_fail_in_parse() -> bool {
    std::env::var("GBRAIN_TEST_QUARANTINE_RESTORE_FAIL_IN_PARSE")
        .ok()
        .as_deref()
        == Some("1")
}

#[cfg(unix)]
fn append_restore_trace(event: &str) -> Result<(), QuarantineError> {
    let Some(trace_path) = std::env::var_os("GBRAIN_TEST_QUARANTINE_RESTORE_TRACE_FILE") else {
        return Ok(());
    };
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace_path)?;
    writeln!(file, "{event}")?;
    Ok(())
}

fn count<P>(conn: &Connection, sql: &str, params: P) -> Result<i64, QuarantineError>
where
    P: rusqlite::Params,
{
    conn.query_row(sql, params, |row| row.get(0))
        .map_err(Into::into)
}

fn current_timestamp(conn: &Connection) -> Result<String, QuarantineError> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get::<_, String>(0)
    })
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        conn
    }

    fn insert_collection(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO collections (name, root_path, state, writable, is_write_target)
             VALUES ('work', '/vault', 'active', 1, 0)",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_quarantined_page(
        conn: &Connection,
        collection_id: i64,
        slug: &str,
        quarantined_at: &str,
    ) -> i64 {
        conn.execute(
             "INSERT INTO pages
                  (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version, quarantined_at)
             VALUES (?1, ?2, ?3, 'note', ?2, '', 'truth', '', '{}', 'notes', '', 1, ?4)",
            params![collection_id, slug, uuid::Uuid::now_v7().to_string(), quarantined_at],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn production_source_without_tests(path: &str) -> String {
        let source_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        let source = std::fs::read_to_string(source_path).unwrap();
        let test_module_start = source.rfind("#[cfg(test)]").unwrap();
        source[..test_module_start].to_owned()
    }

    #[test]
    fn sweep_discards_expired_clean_quarantined_pages_but_skips_db_only_state() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn);
        let clean_page =
            insert_quarantined_page(&conn, collection_id, "notes/clean", "2026-01-01T00:00:00Z");
        let kept_page =
            insert_quarantined_page(&conn, collection_id, "notes/kept", "2026-01-01T00:00:00Z");
        conn.execute(
            "INSERT INTO knowledge_gaps (page_id, query_hash, context)
             VALUES (?1, 'gap-1', 'context')",
            [kept_page],
        )
        .unwrap();

        let summary = sweep_expired_quarantined_pages(&conn).unwrap();

        assert_eq!(summary.discarded, 1);
        assert_eq!(summary.skipped_db_only_state, 1);
        let clean_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE id = ?1",
                [clean_page],
                |row| row.get(0),
            )
            .unwrap();
        let kept_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE id = ?1",
                [kept_page],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(clean_exists, 0);
        assert_eq!(kept_exists, 1);
    }

    #[test]
    fn current_export_only_matches_current_quarantine_epoch() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn);
        let page_id = insert_quarantined_page(
            &conn,
            collection_id,
            "notes/exported",
            "2026-01-01T00:00:00Z",
        );
        record_quarantine_export(
            &conn,
            page_id,
            "2026-01-01T00:00:00Z",
            "2026-01-03T04:05:06Z",
            "out.json".to_owned(),
        )
        .unwrap();

        assert!(has_current_export(&conn, page_id, "2026-01-01T00:00:00Z").unwrap());
        assert!(!has_current_export(&conn, page_id, "2026-01-02T00:00:00Z").unwrap());
        let exported_at: String = conn
            .query_row(
                "SELECT exported_at FROM quarantine_exports WHERE page_id = ?1 AND quarantined_at = ?2",
                params![page_id, "2026-01-01T00:00:00Z"],
                |row| row.get(0),
        )
        .unwrap();
        assert_eq!(exported_at, "2026-01-03T04:05:06Z");
    }

    #[test]
    fn list_collection_quarantine_ignores_export_receipts_from_prior_quarantine_epoch() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn);
        let page_id = insert_quarantined_page(
            &conn,
            collection_id,
            "notes/exported",
            "2026-01-02T00:00:00Z",
        );
        record_quarantine_export(
            &conn,
            page_id,
            "2026-01-01T00:00:00Z",
            "2026-01-03T04:05:06Z",
            "out.json".to_owned(),
        )
        .unwrap();

        let rows = list_collection_quarantine(&conn, "work").unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].address, "work::notes/exported");
        assert_eq!(rows[0].quarantined_at, "2026-01-02T00:00:00Z");
        assert_eq!(
            rows[0].exported_at, None,
            "stale export receipts must not surface on the current quarantine epoch"
        );
    }

    #[test]
    fn list_collection_quarantine_reports_db_only_counts_and_current_epoch_export() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn);
        let page_id = insert_quarantined_page(
            &conn,
            collection_id,
            "notes/exported",
            "2026-01-02T00:00:00Z",
        );
        let peer_id =
            insert_quarantined_page(&conn, collection_id, "notes/peer", "2026-01-02T00:00:00Z");
        conn.execute(
            "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
             VALUES (?1, ?2, 'related', 'programmatic')",
            params![page_id, peer_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO knowledge_gaps (page_id, query_hash, context)
             VALUES (?1, 'gap-1', 'context')",
            [page_id],
        )
        .unwrap();
        record_quarantine_export(
            &conn,
            page_id,
            "2026-01-02T00:00:00Z",
            "2026-01-03T04:05:06Z",
            "out.json".to_owned(),
        )
        .unwrap();

        let rows = list_collection_quarantine(&conn, "work").unwrap();
        let exported = rows
            .into_iter()
            .find(|row| row.slug == "notes/exported")
            .expect("exported row");

        assert_eq!(
            exported.exported_at.as_deref(),
            Some("2026-01-03T04:05:06Z")
        );
        assert_eq!(
            exported.db_only_state,
            DbOnlyStateCounts {
                programmatic_links: 1,
                non_import_assertions: 0,
                raw_data: 0,
                contradictions: 0,
                knowledge_gaps: 1,
            }
        );
    }

    #[test]
    fn discard_without_force_requires_current_epoch_export_when_db_only_state_exists() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn);
        let page_id = insert_quarantined_page(
            &conn,
            collection_id,
            "notes/exported",
            "2026-01-02T00:00:00Z",
        );
        conn.execute(
            "INSERT INTO knowledge_gaps (page_id, query_hash, context)
             VALUES (?1, 'gap-1', 'context')",
            [page_id],
        )
        .unwrap();
        record_quarantine_export(
            &conn,
            page_id,
            "2026-01-01T00:00:00Z",
            "2026-01-03T04:05:06Z",
            "out.json".to_owned(),
        )
        .unwrap();

        let err = discard_quarantined_page(&conn, "work::notes/exported", false).unwrap_err();

        assert!(matches!(
            err,
            QuarantineError::ExportRequired { slug, .. } if slug == "notes/exported"
        ));
        let still_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(still_exists, 1);
    }

    #[test]
    fn discard_without_force_succeeds_after_current_epoch_export_when_db_only_state_exists() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn);
        let page_id = insert_quarantined_page(
            &conn,
            collection_id,
            "notes/exported",
            "2026-01-02T00:00:00Z",
        );
        conn.execute(
            "INSERT INTO knowledge_gaps (page_id, query_hash, context)
             VALUES (?1, 'gap-1', 'context')",
            [page_id],
        )
        .unwrap();
        record_quarantine_export(
            &conn,
            page_id,
            "2026-01-02T00:00:00Z",
            "2026-01-03T04:05:06Z",
            "out.json".to_owned(),
        )
        .unwrap();

        let receipt = discard_quarantined_page(&conn, "work::notes/exported", false).unwrap();

        assert_eq!(
            receipt,
            QuarantineDiscardReceipt {
                collection: "work".to_owned(),
                slug: "notes/exported".to_owned(),
                quarantined_at: "2026-01-02T00:00:00Z".to_owned(),
                forced: false,
                exported_before_discard: true,
            }
        );
        let still_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(still_exists, 0);
    }

    #[test]
    fn discard_with_force_deletes_db_only_state_without_prior_export() {
        let conn = open_test_db();
        let collection_id = insert_collection(&conn);
        let page_id =
            insert_quarantined_page(&conn, collection_id, "notes/forced", "2026-01-02T00:00:00Z");
        conn.execute(
            "INSERT INTO knowledge_gaps (page_id, query_hash, context)
             VALUES (?1, 'gap-1', 'context')",
            [page_id],
        )
        .unwrap();

        let receipt = discard_quarantined_page(&conn, "work::notes/forced", true).unwrap();

        assert_eq!(
            receipt,
            QuarantineDiscardReceipt {
                collection: "work".to_owned(),
                slug: "notes/forced".to_owned(),
                quarantined_at: "2026-01-02T00:00:00Z".to_owned(),
                forced: true,
                exported_before_discard: false,
            }
        );
        let still_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(still_exists, 0);
    }

    #[test]
    fn hard_delete_paths_consult_db_only_state_predicate_before_delete() {
        let reconciler_source = production_source_without_tests(
            PathBuf::from("src")
                .join("core")
                .join("reconciler.rs")
                .to_str()
                .unwrap(),
        );
        let quarantine_source = production_source_without_tests(
            PathBuf::from("src")
                .join("core")
                .join("quarantine.rs")
                .to_str()
                .unwrap(),
        );

        assert!(
            reconciler_source.contains("if has_db_only_state(conn, page_id)? {"),
            "reconciler missing-file classification must consult has_db_only_state before choosing quarantine vs hard-delete"
        );
        assert!(
            quarantine_source.contains(
                "let has_db_only_state = reconciler::has_db_only_state(conn, page.page_id)?;"
            ),
            "quarantine discard must consult the shared has_db_only_state predicate before deleting a page"
        );
        assert!(
            quarantine_source.contains("if reconciler::has_db_only_state(conn, page_id)? {"),
            "quarantine TTL sweep must consult has_db_only_state before deleting a page"
        );
    }

    #[test]
    fn list_collection_quarantine_returns_collection_specific_not_found_error() {
        let conn = open_test_db();

        let err = list_collection_quarantine(&conn, "missing").unwrap_err();

        assert!(matches!(
            err,
            QuarantineError::CollectionNotFound { collection } if collection == "missing"
        ));
    }
}
