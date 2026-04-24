use std::collections::HashMap;
use std::fs;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use thiserror::Error;

use crate::core::collections::{self, OpKind};
use crate::core::markdown;
use crate::core::reconciler;
use crate::core::types::{Page, TimelineEntry};
use crate::core::vault_sync::{self, ResolvedSlug, VaultSyncError};

const DEFAULT_QUARANTINE_TTL_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct DbOnlyStateCounts {
    pub programmatic_links: i64,
    pub non_import_assertions: i64,
    pub raw_data: i64,
    pub contradictions: i64,
    pub knowledge_gaps: i64,
}

impl DbOnlyStateCounts {
    pub fn any(self) -> bool {
        self.programmatic_links != 0
            || self.non_import_assertions != 0
            || self.raw_data != 0
            || self.contradictions != 0
            || self.knowledge_gaps != 0
    }
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
    let counts = db_only_state_counts(conn, page.page_id)?;
    let exported_before_discard = has_current_export(conn, page.page_id, &page.quarantined_at)?;
    if counts.any() && !force && !exported_before_discard {
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
    fn list_collection_quarantine_returns_collection_specific_not_found_error() {
        let conn = open_test_db();

        let err = list_collection_quarantine(&conn, "missing").unwrap_err();

        assert!(matches!(
            err,
            QuarantineError::CollectionNotFound { collection } if collection == "missing"
        ));
    }
}
