use std::sync::{Arc, Mutex};

use rmcp::model::*;
use rmcp::schemars;
use rmcp::tool;
use rmcp::{ServerHandler, ServiceExt};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::commands::{check, get, link};

use crate::core::collections::{self, CollectionError, OpKind, SlugResolution};
use crate::core::fts::sanitize_fts_query;
use crate::core::gaps;
use crate::core::graph::{self, GraphError, TemporalFilter};
use crate::core::markdown;
use crate::core::progressive::progressive_retrieve;
use crate::core::search::hybrid_search_canonical;
use crate::core::types::SearchError;
use crate::core::vault_sync;

type DbRef = Arc<Mutex<Connection>>;

const MAX_SLUG_LEN: usize = 512;
const MAX_CONTENT_LEN: usize = 1_048_576; // 1 MB
const MAX_LIMIT: u32 = 1000;
const MAX_RELATIONSHIP_LEN: usize = 64;
const MAX_TAG_LEN: usize = 64;
const MAX_TAGS_PER_REQUEST: usize = 100;
const MAX_GAP_CONTEXT_LEN: usize = 500;
const MAX_RAW_DATA_LEN: usize = 1_048_576; // 1 MB

fn invalid_params(message: impl Into<String>) -> rmcp::Error {
    rmcp::Error::new(ErrorCode(-32602), message.into(), None)
}

fn validate_slug(slug: &str) -> Result<(), rmcp::Error> {
    if slug.is_empty() {
        return Err(invalid_params("invalid slug: must not be empty"));
    }
    if slug.len() > MAX_SLUG_LEN {
        return Err(invalid_params(format!(
            "invalid slug: exceeds maximum length of {MAX_SLUG_LEN} characters"
        )));
    }
    vault_sync::parse_slug_input(slug).map_err(|err| invalid_params(err.to_string()))
}

fn canonical_slug(collection_name: &str, slug: &str) -> String {
    format!("{collection_name}::{slug}")
}

fn ambiguous_slug_error(slug: &str, candidates: Vec<String>) -> rmcp::Error {
    rmcp::Error::new(
        ErrorCode(-32002),
        format!("AmbiguityError: slug `{slug}` matches multiple collections"),
        Some(serde_json::json!({
            "code": "ambiguous_slug",
            "candidates": candidates,
        })),
    )
}

fn map_collection_error(error: CollectionError) -> rmcp::Error {
    match error {
        CollectionError::NotFound { name } => rmcp::Error::new(
            ErrorCode(-32001),
            format!("collection not found: {name}"),
            None,
        ),
        CollectionError::Ambiguous { slug, candidates } => ambiguous_slug_error(
            &slug,
            candidates
                .split(", ")
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        CollectionError::Sqlite(sqlite_err) => map_db_error(sqlite_err),
        other => invalid_params(other.to_string()),
    }
}

fn resolve_slug_for_mcp(
    db: &Connection,
    input: &str,
    op_kind: OpKind,
) -> Result<vault_sync::ResolvedSlug, rmcp::Error> {
    match collections::parse_slug(db, input, op_kind).map_err(map_collection_error)? {
        SlugResolution::Resolved {
            collection_id,
            collection_name,
            slug,
        } => Ok(vault_sync::ResolvedSlug {
            collection_id,
            collection_name,
            slug,
        }),
        SlugResolution::NotFound { slug } => Err(rmcp::Error::new(
            ErrorCode(-32001),
            format!("page not found: {slug}"),
            None,
        )),
        SlugResolution::Ambiguous { slug, candidates } => Err(ambiguous_slug_error(
            &slug,
            candidates
                .into_iter()
                .map(|candidate| candidate.full_address)
                .collect(),
        )),
    }
}

fn resolve_read_collection_filter_for_mcp(
    db: &Connection,
    collection_name: Option<&str>,
) -> Result<Option<collections::Collection>, rmcp::Error> {
    collections::resolve_read_collection_filter(db, collection_name).map_err(map_collection_error)
}

fn page_id_for_resolved(
    db: &Connection,
    resolved: &vault_sync::ResolvedSlug,
) -> Result<i64, rmcp::Error> {
    db.query_row(
        "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
        rusqlite::params![resolved.collection_id, &resolved.slug],
        |row| row.get(0),
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => rmcp::Error::new(
            ErrorCode(-32001),
            format!(
                "page not found: {}",
                canonical_slug(&resolved.collection_name, &resolved.slug)
            ),
            None,
        ),
        other => map_db_error(other),
    })
}

fn canonicalize_page_for_mcp(
    page: &crate::core::types::Page,
    resolved: &vault_sync::ResolvedSlug,
) -> crate::core::types::Page {
    let mut rendered = page.clone();
    rendered.frontmatter.insert(
        "slug".to_string(),
        canonical_slug(&resolved.collection_name, &resolved.slug),
    );
    rendered
}

fn validate_content(content: &str) -> Result<(), rmcp::Error> {
    if content.len() > MAX_CONTENT_LEN {
        return Err(invalid_params(format!(
            "content too large: {} bytes exceeds maximum of {MAX_CONTENT_LEN} bytes",
            content.len()
        )));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    field: &str,
    max_len: usize,
    allowed: fn(u8) -> bool,
    allowed_hint: &str,
) -> Result<(), rmcp::Error> {
    if value.is_empty() {
        return Err(invalid_params(format!(
            "invalid {field}: must not be empty"
        )));
    }
    if value.len() > max_len {
        return Err(invalid_params(format!(
            "invalid {field}: exceeds maximum length of {max_len} characters"
        )));
    }
    if !value.bytes().all(allowed) {
        return Err(invalid_params(format!(
            "invalid {field}: allowed characters are {allowed_hint}"
        )));
    }
    Ok(())
}

fn is_tag_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
}

fn validate_relationship(relationship: &str) -> Result<(), rmcp::Error> {
    validate_token(
        relationship,
        "relationship",
        MAX_RELATIONSHIP_LEN,
        is_tag_byte,
        "[a-z0-9_-]",
    )
}

fn validate_tag_list(tags: &[String], field: &str) -> Result<(), rmcp::Error> {
    if tags.len() > MAX_TAGS_PER_REQUEST {
        return Err(invalid_params(format!(
            "invalid {field}: exceeds maximum of {MAX_TAGS_PER_REQUEST} tags"
        )));
    }
    for tag in tags {
        validate_token(tag, "tag", MAX_TAG_LEN, is_tag_byte, "[a-z0-9_-]")?;
    }
    Ok(())
}

fn parse_component(value: &str, start: usize, len: usize) -> Option<u32> {
    value.get(start..start + len)?.parse().ok()
}

fn is_valid_temporal_value(value: &str) -> bool {
    match value.len() {
        7 => matches!(
            (parse_component(value, 0, 4), value.as_bytes().get(4), parse_component(value, 5, 2)),
            (Some(_year), Some(b'-'), Some(month)) if (1..=12).contains(&month)
        ),
        10 => matches!(
            (
                parse_component(value, 0, 4),
                value.as_bytes().get(4),
                parse_component(value, 5, 2),
                value.as_bytes().get(7),
                parse_component(value, 8, 2)
            ),
            (Some(_year), Some(b'-'), Some(month), Some(b'-'), Some(day))
                if (1..=12).contains(&month) && (1..=31).contains(&day)
        ),
        20 => matches!(
            (
                parse_component(value, 0, 4),
                value.as_bytes().get(4),
                parse_component(value, 5, 2),
                value.as_bytes().get(7),
                parse_component(value, 8, 2),
                value.as_bytes().get(10),
                parse_component(value, 11, 2),
                value.as_bytes().get(13),
                parse_component(value, 14, 2),
                value.as_bytes().get(16),
                parse_component(value, 17, 2),
                value.as_bytes().get(19)
            ),
            (
                Some(_year),
                Some(b'-'),
                Some(month),
                Some(b'-'),
                Some(day),
                Some(b'T'),
                Some(hour),
                Some(b':'),
                Some(minute),
                Some(b':'),
                Some(second),
                Some(b'Z')
            ) if (1..=12).contains(&month)
                && (1..=31).contains(&day)
                && hour <= 23
                && minute <= 59
                && second <= 59
        ),
        _ => false,
    }
}

fn validate_temporal_value(value: &str, field: &str) -> Result<(), rmcp::Error> {
    if is_valid_temporal_value(value) {
        Ok(())
    } else {
        Err(invalid_params(format!(
            "invalid {field}: expected YYYY-MM, YYYY-MM-DD, or YYYY-MM-DDTHH:MM:SSZ"
        )))
    }
}

fn map_db_error(e: rusqlite::Error) -> rmcp::Error {
    if let rusqlite::Error::SqliteFailure(ref err, ref msg) = e {
        // SQLITE_CONSTRAINT_UNIQUE (extended code 2067)
        if err.extended_code == 2067 {
            return rmcp::Error::new(
                ErrorCode(-32009),
                format!(
                    "conflict: {}",
                    msg.as_deref().unwrap_or("unique constraint violation")
                ),
                None,
            );
        }
        // FTS5 parse/syntax errors surface as SQLITE_ERROR with "fts5" in message
        if let Some(ref msg_str) = msg {
            if msg_str.contains("fts5") {
                return rmcp::Error::new(
                    ErrorCode(-32602),
                    format!("invalid search query: {msg_str}"),
                    None,
                );
            }
        }
    }
    rmcp::Error::new(ErrorCode(-32003), format!("database error: {e}"), None)
}

fn map_search_error(e: SearchError) -> rmcp::Error {
    match e {
        SearchError::Sqlite(sqlite_err) => map_db_error(sqlite_err),
        SearchError::Ambiguous { slug, candidates } => ambiguous_slug_error(
            &slug,
            candidates
                .split(", ")
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        SearchError::Internal { message } => {
            rmcp::Error::new(ErrorCode(-32003), format!("search error: {message}"), None)
        }
    }
}

fn map_anyhow_error(e: anyhow::Error) -> rmcp::Error {
    let msg = e.to_string();
    if msg.contains("ConflictError") || msg.contains("ConcurrentRenameError") {
        rmcp::Error::new(ErrorCode(-32009), msg, None)
    } else if msg.contains("page not found") || msg.contains("link not found") {
        rmcp::Error::new(ErrorCode(-32001), msg, None)
    } else if msg.contains("CollectionRestoringError")
        || msg.contains("ServeOwnsCollectionError")
        || msg.contains("Restore")
        || msg.contains("NewRoot")
        || msg.contains("ambiguous slug")
    {
        rmcp::Error::new(ErrorCode(-32002), msg, None)
    } else {
        rmcp::Error::new(ErrorCode(-32003), msg, None)
    }
}

fn map_vault_sync_error(e: vault_sync::VaultSyncError) -> rmcp::Error {
    if let vault_sync::VaultSyncError::AmbiguousSlug { slug, candidates } = &e {
        return ambiguous_slug_error(
            slug,
            candidates
                .split(", ")
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
    }
    let code = match e {
        vault_sync::VaultSyncError::PageNotFound { .. } => ErrorCode(-32001),
        vault_sync::VaultSyncError::AmbiguousSlug { .. }
        | vault_sync::VaultSyncError::CollectionRestoring { .. }
        | vault_sync::VaultSyncError::ServeOwnsCollectionError { .. }
        | vault_sync::VaultSyncError::RestoreInProgress { .. }
        | vault_sync::VaultSyncError::RestorePendingFinalize { .. }
        | vault_sync::VaultSyncError::RestoreIntegrityBlocked { .. }
        | vault_sync::VaultSyncError::RestoreNonEmptyTarget { .. }
        | vault_sync::VaultSyncError::ServeDiedDuringHandshake { .. }
        | vault_sync::VaultSyncError::HandshakeTimeout { .. }
        | vault_sync::VaultSyncError::NewRootVerificationFailed { .. }
        | vault_sync::VaultSyncError::NewRootUnstable { .. }
        | vault_sync::VaultSyncError::ReconcileHalted { .. } => ErrorCode(-32002),
        #[cfg(unix)]
        vault_sync::VaultSyncError::MissingExpectedVersion { .. }
        | vault_sync::VaultSyncError::StaleExpectedVersion { .. }
        | vault_sync::VaultSyncError::ExternalDelete { .. }
        | vault_sync::VaultSyncError::ExternalCreate { .. }
        | vault_sync::VaultSyncError::HashMismatch { .. }
        | vault_sync::VaultSyncError::ConcurrentRename { .. } => ErrorCode(-32009),
        _ => ErrorCode(-32003),
    };
    rmcp::Error::new(code, e.to_string(), None)
}

fn map_graph_error(e: GraphError) -> rmcp::Error {
    match e {
        GraphError::PageNotFound { slug } => {
            rmcp::Error::new(ErrorCode(-32001), format!("page not found: {slug}"), None)
        }
        GraphError::Sqlite(sqlite_err) => map_db_error(sqlite_err),
    }
}

fn parse_temporal_filter(temporal: Option<&str>) -> Result<TemporalFilter, rmcp::Error> {
    match temporal.unwrap_or("active") {
        "active" | "current" => Ok(TemporalFilter::Active),
        "all" | "history" => Ok(TemporalFilter::All),
        other => Err(rmcp::Error::new(
            ErrorCode(-32602),
            format!("invalid temporal filter: {other}"),
            None,
        )),
    }
}

#[derive(Clone)]
pub struct QuaidServer {
    db: DbRef,
}

impl QuaidServer {
    pub fn new(conn: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(conn)),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryGetInput {
    /// Page slug to retrieve
    pub slug: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryPutInput {
    /// Page slug to create or update
    pub slug: String,
    /// Markdown content of the page
    pub content: String,
    /// Expected current version for optimistic concurrency control
    pub expected_version: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryQueryInput {
    /// Search query string
    pub query: String,
    /// Optional collection filter
    pub collection: Option<String>,
    /// Optional wing filter
    pub wing: Option<String>,
    /// Maximum results to return
    pub limit: Option<u32>,
    /// Retrieval depth: "auto" for progressive expansion, absent/empty for direct results only
    pub depth: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemorySearchInput {
    /// FTS5 search query string
    pub query: String,
    /// Optional collection filter
    pub collection: Option<String>,
    /// Optional wing filter
    pub wing: Option<String>,
    /// Maximum results to return
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryListInput {
    /// Optional collection filter
    pub collection: Option<String>,
    /// Optional wing filter
    pub wing: Option<String>,
    /// Optional type filter
    pub page_type: Option<String>,
    /// Maximum results to return
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryLinkInput {
    pub from_slug: String,
    pub to_slug: String,
    pub relationship: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryLinkCloseInput {
    pub link_id: u64,
    pub valid_until: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryBacklinksInput {
    pub slug: String,
    pub limit: Option<u32>,
    pub temporal: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryGraphInput {
    pub slug: String,
    pub depth: Option<u32>,
    pub temporal: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryCheckInput {
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryTimelineInput {
    pub slug: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryTagsInput {
    pub slug: String,
    pub add: Option<Vec<String>>,
    pub remove: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryGapInput {
    /// Query string to log as a knowledge gap
    pub query: String,
    /// Optional page slug to bind the gap to
    pub slug: Option<String>,
    /// Optional context about the gap
    pub context: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryGapsInput {
    /// Include resolved gaps (default: false)
    pub resolved: Option<bool>,
    /// Maximum number of gaps to return (default: 20, max: 1000)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryStatsInput {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryCollectionsInput {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryRawInput {
    /// Page slug to attach raw data to
    pub slug: String,
    /// Source identifier (e.g. "crustdata", "exa", "meeting")
    pub source: String,
    /// Arbitrary JSON object to store (must be a JSON object, not array/scalar)
    pub data: serde_json::Value,
    /// Set to true to overwrite an existing row for (slug, source). Default: false.
    pub overwrite: Option<bool>,
}

#[tool(tool_box)]
impl QuaidServer {
    #[tool(description = "Get a page by slug")]
    pub fn memory_get(
        &self,
        #[tool(aggr)] input: MemoryGetInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::Read)?;
        let page = vault_sync::get_page_by_input(&db, &input.slug).map_err(map_vault_sync_error)?;
        let rendered = markdown::render_page(&canonicalize_page_for_mcp(&page, &resolved));
        Ok(CallToolResult::success(vec![Content::text(rendered)]))
    }

    #[tool(description = "Write or update a page")]
    pub fn memory_put(
        &self,
        #[tool(aggr)] input: MemoryPutInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        validate_content(&input.content)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(
            &db,
            &input.slug,
            if input.expected_version.is_some() {
                OpKind::WriteUpdate
            } else {
                OpKind::WriteCreate
            },
        )?;
        // Collection write-gate must run BEFORE any OCC/precondition prevalidation.
        // If the collection is restoring or needs_full_sync, CollectionRestoringError wins
        // over any version-conflict or existence-conflict that the prevalidation would surface.
        vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)
            .map_err(map_vault_sync_error)?;
        let existing_version: Option<i64> = db
            .query_row(
                "SELECT version FROM pages WHERE collection_id = ?1 AND slug = ?2",
                rusqlite::params![resolved.collection_id, &resolved.slug],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_db_error)?;
        match (existing_version, input.expected_version) {
            (None, Some(expected)) => {
                return Err(rmcp::Error::new(
                    ErrorCode(-32009),
                    format!("conflict: page does not exist at version {expected}"),
                    Some(serde_json::json!({ "current_version": null })),
                ));
            }
            (Some(current), None) => {
                return Err(rmcp::Error::new(
                    ErrorCode(-32009),
                    format!(
                        "conflict: page already exists (current version: {current}). Provide expected_version to update."
                    ),
                    Some(serde_json::json!({ "current_version": current })),
                ));
            }
            _ => {}
        }
        crate::commands::put::put_from_string(
            &db,
            &canonical_slug(&resolved.collection_name, &resolved.slug),
            &input.content,
            input.expected_version,
        )
        .map_err(|err| {
            let message = err.to_string();
            if message.contains("Conflict:") {
                rmcp::Error::new(
                    ErrorCode(-32009),
                    message.replace("Conflict: ", "conflict: "),
                    Some(serde_json::json!({ "current_version": existing_version })),
                )
            } else {
                map_anyhow_error(err)
            }
        })?;
        let version: i64 = db
            .query_row(
                "SELECT version FROM pages WHERE collection_id = ?1 AND slug = ?2",
                rusqlite::params![resolved.collection_id, &resolved.slug],
                |row| row.get(0),
            )
            .map_err(map_db_error)?;
        let verb = if input.expected_version.is_some() {
            "Updated"
        } else {
            "Created"
        };
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{verb} {}::{} (version {})",
            resolved.collection_name, resolved.slug, version
        ))]))
    }

    #[tool(description = "Hybrid semantic + FTS5 query")]
    pub fn memory_query(
        &self,
        #[tool(aggr)] input: MemoryQueryInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let collection_filter =
            resolve_read_collection_filter_for_mcp(&db, input.collection.as_deref())?;

        let limit = input.limit.unwrap_or(10).min(MAX_LIMIT) as usize;
        let results = hybrid_search_canonical(
            &input.query,
            input.wing.as_deref(),
            collection_filter.as_ref().map(|collection| collection.id),
            &db,
            limit,
        )
        .map_err(map_search_error)?;

        // Auto-log knowledge gap on weak results
        if results.len() < 2 || results.iter().all(|r| r.score < 0.3) {
            let _ = gaps::log_gap(
                None,
                &input.query,
                "",
                results.first().map(|r| r.score),
                &db,
            );
        }

        let depth_normalized = input.depth.as_deref().map(|d| d.trim().to_lowercase());
        let results = match depth_normalized.as_deref() {
            Some("auto") => {
                let budget: usize = db
                    .query_row(
                        "SELECT value FROM config WHERE key = 'default_token_budget'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(4000);
                progressive_retrieve(
                    results.clone(),
                    budget,
                    3,
                    collection_filter.as_ref().map(|c| c.id),
                    &db,
                )
                .unwrap_or(results)
            }
            _ => results,
        };

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "FTS5 full-text search")]
    pub fn memory_search(
        &self,
        #[tool(aggr)] input: MemorySearchInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let collection_filter =
            resolve_read_collection_filter_for_mcp(&db, input.collection.as_deref())?;

        let limit = input.limit.unwrap_or(50).min(MAX_LIMIT) as usize;
        let safe_query = sanitize_fts_query(&input.query);
        let results = crate::core::fts::search_fts_canonical(
            &safe_query,
            input.wing.as_deref(),
            collection_filter.as_ref().map(|collection| collection.id),
            &db,
            limit,
        )
        .map_err(map_search_error)?;

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List pages with optional filters")]
    pub fn memory_list(
        &self,
        #[tool(aggr)] input: MemoryListInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let collection_filter =
            resolve_read_collection_filter_for_mcp(&db, input.collection.as_deref())?;

        let limit = input.limit.unwrap_or(50).min(MAX_LIMIT);
        let mut sql = String::from(
            "SELECT c.name || '::' || p.slug, p.type, p.summary \
             FROM pages p \
             JOIN collections c ON c.id = p.collection_id \
             WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref w) = input.wing {
            sql.push_str(" AND p.wing = ?");
            params.push(Box::new(w.clone()));
        }
        if let Some(ref t) = input.page_type {
            sql.push_str(" AND p.type = ?");
            params.push(Box::new(t.clone()));
        }
        if let Some(collection) = collection_filter {
            sql.push_str(" AND p.collection_id = ?");
            params.push(Box::new(collection.id));
        }
        sql.push_str(" ORDER BY p.updated_at DESC LIMIT ?");
        params.push(Box::new(limit));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = db.prepare(&sql).map_err(map_db_error)?;

        #[derive(Serialize)]
        struct ListEntry {
            slug: String,
            #[serde(rename = "type")]
            page_type: String,
            summary: String,
        }

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(ListEntry {
                    slug: row.get(0)?,
                    page_type: row.get(1)?,
                    summary: row.get(2)?,
                })
            })
            .map_err(map_db_error)?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(map_db_error)?);
        }

        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Create a typed temporal link between two pages")]
    pub fn memory_link(
        &self,
        #[tool(aggr)] input: MemoryLinkInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.from_slug)?;
        validate_slug(&input.to_slug)?;
        validate_relationship(&input.relationship)?;
        if let Some(valid_from) = input.valid_from.as_deref() {
            validate_temporal_value(valid_from, "valid_from")?;
        }
        if let Some(valid_until) = input.valid_until.as_deref() {
            validate_temporal_value(valid_until, "valid_until")?;
        }
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let from = resolve_slug_for_mcp(&db, &input.from_slug, OpKind::WriteUpdate)?;
        let to = resolve_slug_for_mcp(&db, &input.to_slug, OpKind::WriteUpdate)?;
        let from_slug = canonical_slug(&from.collection_name, &from.slug);
        let to_slug = canonical_slug(&to.collection_name, &to.slug);

        link::run_silent(
            &db,
            &from_slug,
            &to_slug,
            &input.relationship,
            input.valid_from,
            input.valid_until,
        )
        .map_err(map_anyhow_error)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked {} → {} ({})",
            from_slug, to_slug, input.relationship
        ))]))
    }

    #[tool(description = "Close a temporal link by its database ID")]
    pub fn memory_link_close(
        &self,
        #[tool(aggr)] input: MemoryLinkCloseInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_temporal_value(&input.valid_until, "valid_until")?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        link::close_silent(&db, input.link_id, &input.valid_until).map_err(map_anyhow_error)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Closed link {} valid_until={}",
            input.link_id, input.valid_until
        ))]))
    }

    #[tool(description = "List inbound backlinks for a page")]
    pub fn memory_backlinks(
        &self,
        #[tool(aggr)] input: MemoryBacklinksInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let filter = parse_temporal_filter(input.temporal.as_deref())?;
        let limit = input.limit.unwrap_or(100).min(MAX_LIMIT);
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::Read)?;
        let to_id = page_id_for_resolved(&db, &resolved)?;

        #[derive(Serialize)]
        struct BacklinkRow {
            id: i64,
            from_slug: String,
            relationship: String,
            valid_from: Option<String>,
            valid_until: Option<String>,
        }

        let temporal_clause = match filter {
            TemporalFilter::Active => {
                " AND (l.valid_from IS NULL OR l.valid_from <= date('now'))\
                 AND (l.valid_until IS NULL OR l.valid_until >= date('now'))"
            }
            TemporalFilter::All => "",
        };

        let sql = format!(
            "SELECT l.id, c.name || '::' || p.slug, l.relationship, l.valid_from, l.valid_until \
             FROM links l \
             JOIN pages p ON l.from_page_id = p.id \
             JOIN collections c ON c.id = p.collection_id \
             WHERE l.to_page_id = ?1{temporal_clause} \
             ORDER BY l.created_at DESC \
             LIMIT ?2"
        );

        let mut stmt = db.prepare(&sql).map_err(map_db_error)?;

        let rows: Vec<BacklinkRow> = stmt
            .query_map(rusqlite::params![to_id, limit], |row| {
                Ok(BacklinkRow {
                    id: row.get(0)?,
                    from_slug: row.get(1)?,
                    relationship: row.get(2)?,
                    valid_from: row.get(3)?,
                    valid_until: row.get(4)?,
                })
            })
            .map_err(map_db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_db_error)?;

        let json = serde_json::to_string_pretty(&rows)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "N-hop neighbourhood graph from a page")]
    pub fn memory_graph(
        &self,
        #[tool(aggr)] input: MemoryGraphInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let depth = input.depth.unwrap_or(1).min(graph::MAX_DEPTH);
        let filter = parse_temporal_filter(input.temporal.as_deref())?;
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::Read)?;
        let page_id = page_id_for_resolved(&db, &resolved)?;
        let result = graph::neighborhood_graph_for_page(
            page_id,
            &resolved.collection_name,
            &resolved.slug,
            depth,
            filter,
            &db,
        )
        .map_err(map_graph_error)?;

        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Run contradiction detection on a page or all pages")]
    pub fn memory_check(
        &self,
        #[tool(aggr)] input: MemoryCheckInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        if let Some(slug) = input.slug.as_deref() {
            validate_slug(slug)?;
        }
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let slug_filter = input
            .slug
            .as_deref()
            .map(|slug| resolve_slug_for_mcp(&db, slug, OpKind::WriteUpdate))
            .transpose()?;

        let selected_page_id = if let Some(resolved) = slug_filter.as_ref() {
            vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)
                .map_err(map_vault_sync_error)?;
            let page_id = page_id_for_resolved(&db, resolved)?;
            let page = get::get_page_by_key(&db, resolved.collection_id, &resolved.slug)
                .map_err(map_anyhow_error)?;
            crate::core::assertions::extract_assertions(&page, &db)
                .map_err(|error| map_anyhow_error(error.into()))?;
            crate::core::assertions::check_assertions_for_page_id(page_id, &db)
                .map_err(|error| map_anyhow_error(error.into()))?;
            Some(page_id)
        } else {
            check::execute_check(&db, None, true, None).map_err(map_anyhow_error)?;
            None
        };

        // Fetch unresolved contradictions as JSON
        use crate::core::assertions::Contradiction;
        let contradictions: Vec<Contradiction> = if let Some(page_id) = selected_page_id {
            let mut stmt = db
                .prepare(
                    "SELECT cp.name || '::' || p.slug, \
                            COALESCE(co.name || '::' || other.slug, cp.name || '::' || p.slug), \
                            c.type, c.description, c.detected_at \
                      FROM contradictions c \
                      JOIN pages p ON p.id = c.page_id \
                      JOIN collections cp ON cp.id = p.collection_id \
                      LEFT JOIN pages other ON other.id = c.other_page_id \
                      LEFT JOIN collections co ON co.id = other.collection_id \
                      WHERE c.resolved_at IS NULL AND (c.page_id = ?1 OR c.other_page_id = ?1) \
                      ORDER BY c.detected_at, p.slug",
                )
                .map_err(map_db_error)?;

            let rows = stmt
                .query_map([page_id], |row| {
                    Ok(Contradiction {
                        page_slug: row.get(0)?,
                        other_page_slug: row.get(1)?,
                        r#type: row.get(2)?,
                        description: row.get(3)?,
                        detected_at: row.get(4)?,
                    })
                })
                .map_err(map_db_error)?;

            rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)?
        } else {
            let mut stmt = db
                .prepare(
                    "SELECT cp.name || '::' || p.slug, \
                            COALESCE(co.name || '::' || other.slug, cp.name || '::' || p.slug), \
                            c.type, c.description, c.detected_at \
                      FROM contradictions c \
                      JOIN pages p ON p.id = c.page_id \
                      JOIN collections cp ON cp.id = p.collection_id \
                      LEFT JOIN pages other ON other.id = c.other_page_id \
                      LEFT JOIN collections co ON co.id = other.collection_id \
                      WHERE c.resolved_at IS NULL \
                      ORDER BY c.detected_at, p.slug",
                )
                .map_err(map_db_error)?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(Contradiction {
                        page_slug: row.get(0)?,
                        other_page_slug: row.get(1)?,
                        r#type: row.get(2)?,
                        description: row.get(3)?,
                        detected_at: row.get(4)?,
                    })
                })
                .map_err(map_db_error)?;

            rows.collect::<Result<Vec<_>, _>>().map_err(map_db_error)?
        };

        let json = serde_json::to_string_pretty(&contradictions)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Show timeline entries for a page")]
    pub fn memory_timeline(
        &self,
        #[tool(aggr)] input: MemoryTimelineInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::Read)?;

        let limit = input.limit.unwrap_or(20).min(MAX_LIMIT);

        let page = get::get_page_by_key(&db, resolved.collection_id, &resolved.slug)
            .map_err(map_anyhow_error)?;

        let page_id = page_id_for_resolved(&db, &resolved)?;

        // Query structured timeline_entries table
        let mut stmt = db
            .prepare(
                "SELECT date, summary, source, detail FROM timeline_entries \
                 WHERE page_id = ?1 ORDER BY date DESC LIMIT ?2",
            )
            .map_err(map_db_error)?;

        let rows = stmt
            .query_map(rusqlite::params![page_id, limit], |row| {
                let date: String = row.get(0)?;
                let summary: String = row.get(1)?;
                let source: String = row.get(2)?;
                let detail: String = row.get(3)?;
                let mut entry = format!("{date}: {summary}");
                if !source.is_empty() {
                    entry.push_str(&format!(" [source: {source}]"));
                }
                if !detail.is_empty() {
                    entry.push_str(&format!("\n{detail}"));
                }
                Ok(entry)
            })
            .map_err(map_db_error)?;

        let mut entries: Vec<String> = Vec::new();
        for row in rows {
            entries.push(row.map_err(map_db_error)?);
        }

        // Fall back to legacy timeline markdown field
        if entries.is_empty() {
            let timeline = page.timeline.trim();
            if !timeline.is_empty() {
                entries = timeline
                    .split("\n---\n")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .take(limit as usize)
                    .collect();
            }
        }

        #[derive(Serialize)]
        struct TimelineOutput {
            slug: String,
            entries: Vec<String>,
        }

        let output = TimelineOutput {
            slug: canonical_slug(&resolved.collection_name, &resolved.slug),
            entries,
        };

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List, add, or remove tags on a page")]
    pub fn memory_tags(
        &self,
        #[tool(aggr)] input: MemoryTagsInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let add = input.add.unwrap_or_default();
        let remove = input.remove.unwrap_or_default();
        validate_tag_list(&add, "add")?;
        validate_tag_list(&remove, "remove")?;
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::WriteUpdate)?;
        if !add.is_empty() || !remove.is_empty() {
            vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)
                .map_err(map_vault_sync_error)?;
        }
        let page_id: i64 = db
            .query_row(
                "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
                rusqlite::params![resolved.collection_id, &resolved.slug],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => rmcp::Error::new(
                    ErrorCode(-32001),
                    format!("page not found: {}", input.slug),
                    None,
                ),
                other => map_db_error(other),
            })?;

        for tag in &add {
            db.execute(
                "INSERT OR IGNORE INTO tags (page_id, tag) VALUES (?1, ?2)",
                rusqlite::params![page_id, tag],
            )
            .map_err(map_db_error)?;
        }

        for tag in &remove {
            db.execute(
                "DELETE FROM tags WHERE page_id = ?1 AND tag = ?2",
                rusqlite::params![page_id, tag],
            )
            .map_err(map_db_error)?;
        }

        // Return current tags
        let mut stmt = db
            .prepare("SELECT tag FROM tags WHERE page_id = ?1 ORDER BY tag")
            .map_err(map_db_error)?;
        let tags: Vec<String> = stmt
            .query_map([page_id], |row| row.get(0))
            .map_err(map_db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_db_error)?;

        let json = serde_json::to_string_pretty(&tags)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Log a knowledge gap (privacy-safe: stores query_hash, not raw query)")]
    pub fn memory_gap(
        &self,
        #[tool(aggr)] input: MemoryGapInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        if input.query.trim().is_empty() {
            return Err(invalid_params("query must not be empty"));
        }
        let mut context = input.context.unwrap_or_default();
        if context.len() > MAX_GAP_CONTEXT_LEN {
            return Err(invalid_params(format!(
                "context exceeds maximum length of {MAX_GAP_CONTEXT_LEN} characters"
            )));
        }
        if !context.is_empty() {
            // Do not persist caller-provided context to avoid leaking sensitive query text.
            context.clear();
        }
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let page_id = if let Some(slug) = input.slug.as_deref() {
            validate_slug(slug)?;
            let resolved = resolve_slug_for_mcp(&db, slug, OpKind::WriteUpdate)?;
            vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)
                .map_err(map_vault_sync_error)?;
            Some(page_id_for_resolved(&db, &resolved)?)
        } else {
            None
        };

        let query_hash = {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(input.query.as_bytes());
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };

        match page_id {
            Some(page_id) => gaps::log_gap_for_page(page_id, &input.query, &context, None, &db),
            None => gaps::log_gap(None, &input.query, &context, None, &db),
        }
        .map_err(|e| rmcp::Error::new(ErrorCode(-32003), format!("database error: {e}"), None))?;

        // Retrieve the gap ID
        let gap_id: i64 = db
            .query_row(
                "SELECT id FROM knowledge_gaps WHERE query_hash = ?1",
                [&query_hash],
                |row| row.get(0),
            )
            .map_err(map_db_error)?;

        let result = serde_json::json!({
            "id": gap_id,
            "query_hash": query_hash,
            "page_id": page_id,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap(),
        )]))
    }

    #[tool(description = "List knowledge gaps")]
    pub fn memory_gaps(
        &self,
        #[tool(aggr)] input: MemoryGapsInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let resolved = input.resolved.unwrap_or(false);
        let limit = input.limit.unwrap_or(20).min(MAX_LIMIT) as usize;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let gap_list = gaps::list_gaps(resolved, limit, &db).map_err(|e| {
            rmcp::Error::new(ErrorCode(-32003), format!("database error: {e}"), None)
        })?;

        let json = serde_json::to_string_pretty(&gap_list)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Brain statistics (page count, link count, etc.)")]
    pub fn memory_stats(
        &self,
        #[tool(aggr)] _input: MemoryStatsInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let page_count: i64 = db
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .map_err(map_db_error)?;
        let link_count: i64 = db
            .query_row("SELECT COUNT(*) FROM links", [], |row| row.get(0))
            .map_err(map_db_error)?;
        let assertion_count: i64 = db
            .query_row("SELECT COUNT(*) FROM assertions", [], |row| row.get(0))
            .map_err(map_db_error)?;
        let contradiction_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM contradictions WHERE resolved_at IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(map_db_error)?;
        let gap_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM knowledge_gaps WHERE resolved_at IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(map_db_error)?;
        let embedding_count: i64 = db
            .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
            .map_err(map_db_error)?;

        let active_model: Option<String> = db
            .query_row(
                "SELECT name FROM embedding_models WHERE active = 1 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();

        let db_size_bytes: u64 = db
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name = 'main'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|m| m.len())
            .unwrap_or(0);

        let result = serde_json::json!({
            "page_count": page_count,
            "link_count": link_count,
            "assertion_count": assertion_count,
            "contradiction_count": contradiction_count,
            "gap_count": gap_count,
            "embedding_count": embedding_count,
            "active_model": active_model,
            "db_size_bytes": db_size_bytes,
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap(),
        )]))
    }

    #[tool(description = "List collection status for MCP clients")]
    pub fn memory_collections(
        &self,
        #[tool(aggr)] _input: MemoryCollectionsInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let collections = vault_sync::list_memory_collections(&db).map_err(map_vault_sync_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&collections).unwrap(),
        )]))
    }

    #[tool(description = "Store raw structured data (API responses, JSON) for a page")]
    pub fn memory_raw(
        &self,
        #[tool(aggr)] input: MemoryRawInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        if input.source.is_empty() {
            return Err(invalid_params("source must not be empty"));
        }
        if !input.data.is_object() {
            return Err(invalid_params(
                "data must be a JSON object, not an array or scalar",
            ));
        }
        let data_json = serde_json::to_string(&input.data)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        if data_json.len() > MAX_RAW_DATA_LEN {
            return Err(invalid_params(format!(
                "data exceeds maximum size of {MAX_RAW_DATA_LEN} bytes"
            )));
        }
        let overwrite = input.overwrite.unwrap_or(false);
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::WriteUpdate)?;
        vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)
            .map_err(map_vault_sync_error)?;

        let page_id = page_id_for_resolved(&db, &resolved)?;
        let canonical_page_slug = canonical_slug(&resolved.collection_name, &resolved.slug);

        // Guard against silent replacement of existing source data.
        let existing: Option<i64> = db
            .query_row(
                "SELECT id FROM raw_data WHERE page_id = ?1 AND source = ?2",
                rusqlite::params![page_id, &input.source],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_db_error)?;

        if existing.is_some() && !overwrite {
            return Err(rmcp::Error::new(
                ErrorCode(-32003),
                format!(
                    "raw data for source '{}' already exists on '{}'; set overwrite=true to replace",
                    input.source, canonical_page_slug
                ),
                None,
            ));
        }

        db.execute(
            "INSERT OR REPLACE INTO raw_data (page_id, source, data, fetched_at) \
             VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
            rusqlite::params![page_id, input.source, data_json],
        )
        .map_err(map_db_error)?;

        let row_id = db.last_insert_rowid();
        let result = serde_json::json!({ "id": row_id });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap(),
        )]))
    }
}

#[tool(tool_box)]
impl ServerHandler for QuaidServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Quaid personal memory".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

/// Run the MCP stdio server with the given database connection.
pub async fn run(conn: Connection) -> anyhow::Result<()> {
    let server = QuaidServer::new(conn);
    let transport = (tokio::io::stdin(), tokio::io::stdout());
    let _service = server.serve(transport).await?;
    _service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::core::db;
    use serde_json::json;
    use std::fs;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};

    fn open_test_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("server.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        let vault_root = dir.path().join("vault");
        fs::create_dir_all(&vault_root).unwrap();
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
        (dir, conn)
    }

    #[cfg(unix)]
    fn open_test_db_with_vault() -> (tempfile::TempDir, String, Connection, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("server.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        let vault_root = dir.path().join("vault");
        fs::create_dir_all(&vault_root).unwrap();
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
        fs::read_dir(vault_sync::collection_recovery_dir(
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
    fn active_raw_import_count(conn: &Connection, slug: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM raw_imports \
             WHERE page_id = (SELECT id FROM pages WHERE slug = ?1) AND is_active = 1",
            [slug],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn page_version(conn: &Connection, slug: &str) -> i64 {
        conn.query_row("SELECT version FROM pages WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn get_info_enables_tools_capability_and_exposes_core_tool_methods() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        let info = <QuaidServer as ServerHandler>::get_info(&server);

        let _tool_methods = (
            QuaidServer::memory_get
                as fn(&QuaidServer, MemoryGetInput) -> Result<CallToolResult, rmcp::Error>,
            QuaidServer::memory_put
                as fn(&QuaidServer, MemoryPutInput) -> Result<CallToolResult, rmcp::Error>,
            QuaidServer::memory_query
                as fn(&QuaidServer, MemoryQueryInput) -> Result<CallToolResult, rmcp::Error>,
            QuaidServer::memory_search
                as fn(&QuaidServer, MemorySearchInput) -> Result<CallToolResult, rmcp::Error>,
            QuaidServer::memory_list
                as fn(&QuaidServer, MemoryListInput) -> Result<CallToolResult, rmcp::Error>,
            QuaidServer::memory_collections
                as fn(&QuaidServer, MemoryCollectionsInput) -> Result<CallToolResult, rmcp::Error>,
        );

        assert!(info.capabilities.tools.is_some());
    }

    #[test]
    fn memory_get_returns_not_found_error_code_for_missing_slug() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_get(MemoryGetInput {
                slug: "definitely-does-not-exist".to_string(),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn memory_get_returns_structured_ambiguity_payload_for_colliding_bare_slug() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "memory", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nDefault Alice\n",
        );
        create_page_in_collection(
            &server,
            "memory",
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nMemory Alice\n",
        );

        let error = server
            .memory_get(MemoryGetInput {
                slug: "people/alice".to_string(),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        let data = error.data.unwrap();
        assert_eq!(data["code"], "ambiguous_slug");
        let mut candidates = data["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        candidates.sort();
        assert_eq!(
            candidates,
            vec![
                "default::people/alice".to_string(),
                "memory::people/alice".to_string()
            ]
        );
    }

    #[test]
    fn memory_get_explicit_collection_slug_reads_resolved_page_when_slug_collides() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "memory", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nDefault Alice\n",
        );
        create_page_in_collection(
            &server,
            "memory",
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nMemory Alice\n",
        );

        let result = server
            .memory_get(MemoryGetInput {
                slug: "memory::people/alice".to_string(),
            })
            .unwrap();

        let rendered = extract_text(&result);
        assert!(rendered.contains("slug: memory::people/alice"));
        assert!(rendered.contains("Memory Alice"));
        assert!(!rendered.contains("Default Alice"));
    }

    #[test]
    fn memory_put_returns_occ_conflict_error_with_current_version_for_stale_write() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        server
            .memory_put(MemoryPutInput {
                slug: "notes/test".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nInitial content\n".to_string(),
                expected_version: None,
            })
            .unwrap();

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/test".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nUpdated content\n".to_string(),
                expected_version: Some(0),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32009));
        assert_eq!(error.data, Some(json!({ "current_version": 1 })));
    }

    #[test]
    fn memory_put_rejects_update_without_expected_version() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        server
            .memory_put(MemoryPutInput {
                slug: "notes/occ".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nInitial\n".to_string(),
                expected_version: None,
            })
            .unwrap();

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/occ".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nSneaky overwrite\n".to_string(),
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32009));
        assert_eq!(error.data, Some(json!({ "current_version": 1 })));
    }

    #[cfg(unix)]
    #[test]
    fn memory_put_existing_page_without_expected_version_conflicts_before_vault_mutation() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        let server = QuaidServer::new(conn);
        let original = "---\ntitle: Existing\ntype: note\n---\nOriginal body\n";

        server
            .memory_put(MemoryPutInput {
                slug: "notes/existing".to_string(),
                content: original.to_string(),
                expected_version: None,
            })
            .unwrap();

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/existing".to_string(),
                content: "---\ntitle: Existing\ntype: note\n---\nUnexpected overwrite\n"
                    .to_string(),
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32009));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            fs::read_to_string(vault_root.join("notes").join("existing.md")).unwrap(),
            original
        );
        let db = server.db.lock().unwrap();
        assert_eq!(page_version(&db, "notes/existing"), 1);
    }

    #[cfg(unix)]
    #[test]
    fn memory_put_stale_expected_version_conflicts_before_vault_mutation() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        let server = QuaidServer::new(conn);
        let original = "---\ntitle: Stale\ntype: note\n---\nOriginal body\n";

        server
            .memory_put(MemoryPutInput {
                slug: "notes/stale".to_string(),
                content: original.to_string(),
                expected_version: None,
            })
            .unwrap();

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/stale".to_string(),
                content: "---\ntitle: Stale\ntype: note\n---\nStale overwrite\n".to_string(),
                expected_version: Some(0),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32009));
        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            fs::read_to_string(vault_root.join("notes").join("stale.md")).unwrap(),
            original
        );
        let db = server.db.lock().unwrap();
        assert_eq!(page_version(&db, "notes/stale"), 1);
    }

    #[test]
    fn memory_put_refuses_when_collection_needs_full_sync_even_if_not_restoring() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/blocked".to_string(),
                content: "---\ntitle: Blocked\ntype: note\n---\nBlocked\n".to_string(),
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(error.message.contains("CollectionRestoringError"));
    }

    #[test]
    fn memory_get_rejects_invalid_slug() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_get(MemoryGetInput {
                slug: "Invalid/SLUG!".to_string(),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn memory_put_rejects_oversized_content() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let large_content = "x".repeat(1_048_577);
        let error = server
            .memory_put(MemoryPutInput {
                slug: "test/large".to_string(),
                content: large_content,
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn memory_put_rejects_empty_slug() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_put(MemoryPutInput {
                slug: "".to_string(),
                content: "content".to_string(),
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn memory_put_rejects_create_with_expected_version_when_page_does_not_exist() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        // Page does not exist; supplying expected_version is a client bug — reject as OCC conflict.
        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/ghost".to_string(),
                content: "---\ntitle: Ghost\ntype: note\n---\nContent\n".to_string(),
                expected_version: Some(3),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32009));
        assert_eq!(error.data, Some(json!({ "current_version": null })));
    }

    #[test]
    fn memory_get_renders_persisted_memory_id_after_update_omits_frontmatter_uuid() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        server
            .memory_put(MemoryPutInput {
                slug: "notes/uuid".to_string(),
                content: "---\nmemory_id: 01969f11-9448-7d79-8d3f-c68f54761234\ntitle: UUID\ntype: note\n---\nOriginal\n".to_string(),
                expected_version: None,
            })
            .unwrap();
        server
            .memory_put(MemoryPutInput {
                slug: "notes/uuid".to_string(),
                content: "---\ntitle: UUID\ntype: note\n---\nUpdated\n".to_string(),
                expected_version: Some(1),
            })
            .unwrap();

        let result = server
            .memory_get(MemoryGetInput {
                slug: "notes/uuid".to_string(),
            })
            .unwrap();
        let rendered = extract_text(&result);

        assert!(rendered.contains("memory_id: 01969f11-9448-7d79-8d3f-c68f54761234"));
        assert!(rendered.contains("slug: default::notes/uuid"));
        assert!(rendered.contains("Updated"));
    }

    #[test]
    fn memory_query_logs_gap_for_weak_results() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let result = server
            .memory_query(MemoryQueryInput {
                query: "who runs the moon colony".to_string(),
                collection: None,
                wing: None,
                limit: None,
                depth: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        let gap_count: i64 = server
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM knowledge_gaps", [], |row| row.get(0))
            .unwrap();
        assert!(rows.is_empty() && gap_count == 1);
    }

    #[test]
    fn memory_query_auto_depth_expands_linked_results() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "concepts/root",
            "---\ntitle: Root\ntype: concept\n---\nalpha anchor\n",
        );
        create_page(
            &server,
            "concepts/child",
            "---\ntitle: Child\ntype: concept\n---\nlinked expansion result\n",
        );
        server
            .memory_link(MemoryLinkInput {
                from_slug: "concepts/root".to_string(),
                to_slug: "concepts/child".to_string(),
                relationship: "related".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap();

        let result = server
            .memory_query(MemoryQueryInput {
                query: "alpha".to_string(),
                collection: None,
                wing: None,
                limit: Some(1),
                depth: Some("auto".to_string()),
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(rows
            .iter()
            .any(|row| row["slug"] == "default::concepts/child"));
    }

    // 13.5 contract: depth="auto" must NOT expand across collection boundaries
    #[test]
    fn memory_query_auto_depth_does_not_expand_across_collections() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "work", false);
        let server = QuaidServer::new(conn);

        // Anchor page in "default" — will match the query
        create_page(
            &server,
            "concepts/anchor",
            "---\ntitle: Anchor\ntype: concept\n---\ncross collection fence anchor\n",
        );
        // Outside page in "work" — linked from anchor but must NOT appear
        create_page_in_collection(
            &server,
            "work",
            "concepts/outside",
            "---\ntitle: Outside\ntype: concept\n---\nshould not appear in filtered results\n",
        );
        // Cross-collection link: default::concepts/anchor -> work::concepts/outside
        server
            .memory_link(MemoryLinkInput {
                from_slug: "default::concepts/anchor".to_string(),
                to_slug: "work::concepts/outside".to_string(),
                relationship: "related".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap();

        let result = server
            .memory_query(MemoryQueryInput {
                query: "cross collection fence anchor".to_string(),
                collection: Some("default".to_string()),
                wing: None,
                limit: Some(5),
                depth: Some("auto".to_string()),
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(
            !rows
                .iter()
                .any(|row| row["slug"] == "work::concepts/outside"),
            "depth=auto expansion must not cross into a different collection: got {rows:?}"
        );
    }

    #[test]
    fn memory_search_returns_matching_pages() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme builds fundraising software.\n",
        );

        let result = server
            .memory_search(MemorySearchInput {
                query: "fundraising".to_string(),
                collection: None,
                wing: None,
                limit: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows[0]["slug"], "default::companies/acme");
    }

    // D.6 — memory_search with natural-language '?' query returns valid JSON-RPC response
    #[test]
    fn memory_search_natural_language_question_mark_returns_valid_response() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        // '?' would be invalid FTS5 syntax if passed raw — memory_search must sanitize.
        let result = server.memory_search(MemorySearchInput {
            query: "what is CLARITY?".to_string(),
            collection: None,
            wing: None,
            limit: None,
        });

        assert!(
            result.is_ok(),
            "memory_search with '?' must not return an MCP error: {result:?}"
        );
        // The response content must parse as a JSON array (empty or populated).
        let text = extract_text(&result.unwrap());
        let parsed: serde_json::Value =
            serde_json::from_str(&text).expect("memory_search response must be valid JSON");
        assert!(
            parsed.is_array(),
            "memory_search response must be a JSON array"
        );
    }

    #[test]
    fn memory_list_applies_wing_and_type_filters() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );

        let result = server
            .memory_list(MemoryListInput {
                collection: None,
                wing: Some("people".to_string()),
                page_type: Some("person".to_string()),
                limit: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["slug"], "default::people/alice");
    }

    #[test]
    fn memory_search_explicit_collection_filter_returns_only_named_collection() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "work", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/default-hit",
            "---\ntitle: Default Hit\ntype: note\n---\nshared needle\n",
        );
        create_page_in_collection(
            &server,
            "work",
            "notes/work-hit",
            "---\ntitle: Work Hit\ntype: note\n---\nshared needle\n",
        );

        let result = server
            .memory_search(MemorySearchInput {
                query: "shared".to_string(),
                collection: Some("work".to_string()),
                wing: None,
                limit: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["slug"], "work::notes/work-hit");
    }

    #[test]
    fn memory_query_explicit_collection_filter_returns_only_named_collection() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "work", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/default-hit",
            "---\ntitle: Default Hit\ntype: note\n---\nsemantic overlap on robotics leadership\n",
        );
        create_page_in_collection(
            &server,
            "work",
            "notes/work-hit",
            "---\ntitle: Work Hit\ntype: note\n---\nsemantic overlap on robotics leadership\n",
        );

        let result = server
            .memory_query(MemoryQueryInput {
                query: "robotics leadership".to_string(),
                collection: Some("work".to_string()),
                wing: None,
                limit: None,
                depth: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["slug"], "work::notes/work-hit");
    }

    #[test]
    fn memory_list_explicit_collection_filter_returns_only_named_collection() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "work", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice Default\ntype: person\n---\nDefault Alice\n",
        );
        create_page_in_collection(
            &server,
            "work",
            "people/alice",
            "---\ntitle: Alice Work\ntype: person\n---\nWork Alice\n",
        );

        let result = server
            .memory_list(MemoryListInput {
                collection: Some("work".to_string()),
                wing: Some("people".to_string()),
                page_type: Some("person".to_string()),
                limit: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["slug"], "work::people/alice");
    }

    #[test]
    fn read_tools_unknown_collection_filter_errors_clearly() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let query_error = server
            .memory_query(MemoryQueryInput {
                query: "anything".to_string(),
                collection: Some("missing".to_string()),
                wing: None,
                limit: None,
                depth: None,
            })
            .unwrap_err();
        assert_eq!(query_error.code, ErrorCode(-32001));
        assert!(query_error
            .message
            .contains("collection not found: missing"));

        let search_error = server
            .memory_search(MemorySearchInput {
                query: "anything".to_string(),
                collection: Some("missing".to_string()),
                wing: None,
                limit: None,
            })
            .unwrap_err();
        assert_eq!(search_error.code, ErrorCode(-32001));
        assert!(search_error
            .message
            .contains("collection not found: missing"));

        let list_error = server
            .memory_list(MemoryListInput {
                collection: Some("missing".to_string()),
                wing: None,
                page_type: None,
                limit: None,
            })
            .unwrap_err();
        assert_eq!(list_error.code, ErrorCode(-32001));
        assert!(list_error.message.contains("collection not found: missing"));
    }

    #[test]
    fn memory_search_defaults_to_single_active_collection() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "work", false);
        set_collection_state(&conn, "default", "detached");
        let server = QuaidServer::new(conn);
        create_page_in_collection(
            &server,
            "work",
            "notes/only-active",
            "---\ntitle: Only Active\ntype: note\n---\nsole active marker\n",
        );

        let result = server
            .memory_search(MemorySearchInput {
                query: "sole".to_string(),
                collection: None,
                wing: None,
                limit: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["slug"], "work::notes/only-active");
    }

    #[test]
    fn memory_query_defaults_to_write_target_when_multiple_collections_are_active() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "work", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/default-target",
            "---\ntitle: Default Target\ntype: note\n---\nshared semantic marker\n",
        );
        create_page_in_collection(
            &server,
            "work",
            "notes/work-target",
            "---\ntitle: Work Target\ntype: note\n---\nshared semantic marker\n",
        );

        let result = server
            .memory_query(MemoryQueryInput {
                query: "shared semantic marker".to_string(),
                collection: None,
                wing: None,
                limit: None,
                depth: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["slug"], "default::notes/default-target");
    }

    #[test]
    fn memory_list_defaults_to_write_target_when_multiple_collections_are_active() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "work", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/default-target",
            "---\ntitle: Default Target\ntype: note\n---\ndefault target body\n",
        );
        create_page_in_collection(
            &server,
            "work",
            "notes/work-target",
            "---\ntitle: Work Target\ntype: note\n---\nwork target body\n",
        );

        let result = server
            .memory_list(MemoryListInput {
                collection: None,
                wing: Some("notes".to_string()),
                page_type: Some("note".to_string()),
                limit: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["slug"], "default::notes/default-target");
    }

    // ── Phase 2 MCP tests ────────────────────────────────────

    fn create_page(server: &QuaidServer, slug: &str, content: &str) {
        server
            .memory_put(MemoryPutInput {
                slug: slug.to_string(),
                content: content.to_string(),
                expected_version: None,
            })
            .unwrap();
    }

    fn create_page_in_collection(
        server: &QuaidServer,
        collection_name: &str,
        slug: &str,
        content: &str,
    ) {
        server
            .memory_put(MemoryPutInput {
                slug: format!("{collection_name}::{slug}"),
                content: content.to_string(),
                expected_version: None,
            })
            .unwrap();
    }

    fn insert_collection(conn: &Connection, id: i64, name: &str, is_write_target: bool) {
        let root_path = std::env::temp_dir()
            .join(format!(
                "quaid-mcp-{id}-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .display()
            .to_string();
        fs::create_dir_all(&root_path).unwrap();
        conn.execute(
            "INSERT INTO collections (id, name, root_path, state, writable, is_write_target) \
             VALUES (?1, ?2, ?3, 'active', 1, ?4)",
            rusqlite::params![id, name, root_path, if is_write_target { 1 } else { 0 }],
        )
        .unwrap();
    }

    fn set_collection_state(conn: &Connection, name: &str, state: &str) {
        conn.execute(
            "UPDATE collections SET state = ?1 WHERE name = ?2",
            rusqlite::params![state, name],
        )
        .unwrap();
    }

    fn page_id(conn: &Connection, slug: &str) -> i64 {
        conn.query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn insert_timeline_entry(
        conn: &Connection,
        slug: &str,
        date: &str,
        summary: &str,
        source: &str,
        detail: &str,
    ) {
        conn.execute(
            "INSERT INTO timeline_entries (page_id, date, source, summary, summary_hash, detail) \
             VALUES (?1, ?2, ?3, ?4, 'hash', ?5)",
            rusqlite::params![page_id(conn, slug), date, source, summary, detail],
        )
        .unwrap();
    }

    mod validate_slug {
        use super::*;

        #[test]
        fn accepts_lowercase_slug_tokens() {
            assert!(validate_slug("people/alice_1").is_ok());
        }

        #[test]
        fn rejects_slug_with_invalid_characters() {
            assert_eq!(
                validate_slug("People/Alice!").unwrap_err().code,
                ErrorCode(-32602)
            );
        }

        #[test]
        fn rejects_slug_longer_than_limit() {
            let slug = format!("people/{}", "a".repeat(MAX_SLUG_LEN));
            assert_eq!(validate_slug(&slug).unwrap_err().code, ErrorCode(-32602));
        }
    }

    mod validate_relationship {
        use super::*;

        #[test]
        fn accepts_snake_case_relationship() {
            assert!(validate_relationship("works_at").is_ok());
        }

        #[test]
        fn rejects_relationship_with_spaces() {
            assert_eq!(
                validate_relationship("works at").unwrap_err().code,
                ErrorCode(-32602)
            );
        }
    }

    mod validate_tag_list {
        use super::*;

        #[test]
        fn rejects_more_than_maximum_tags() {
            let tags = vec!["tag".to_string(); MAX_TAGS_PER_REQUEST + 1];
            assert_eq!(
                validate_tag_list(&tags, "add").unwrap_err().code,
                ErrorCode(-32602)
            );
        }
    }

    mod validate_temporal_value {
        use super::*;

        #[test]
        fn accepts_supported_temporal_formats() {
            assert!(validate_temporal_value("2024-06", "valid_from").is_ok());
            assert!(validate_temporal_value("2024-06-30", "valid_from").is_ok());
            assert!(validate_temporal_value("2024-06-30T12:34:56Z", "valid_from").is_ok());
        }

        #[test]
        fn rejects_invalid_temporal_values() {
            assert_eq!(
                validate_temporal_value("2024-13", "valid_from")
                    .unwrap_err()
                    .code,
                ErrorCode(-32602)
            );
        }
    }

    mod parse_temporal_filter {
        use super::*;

        #[test]
        fn defaults_to_active_when_absent() {
            assert_eq!(
                super::parse_temporal_filter(None).unwrap(),
                TemporalFilter::Active
            );
        }

        #[test]
        fn accepts_all_filter() {
            assert_eq!(
                super::parse_temporal_filter(Some("all")).unwrap(),
                TemporalFilter::All
            );
        }

        #[test]
        fn rejects_unknown_filter() {
            assert_eq!(
                super::parse_temporal_filter(Some("future"))
                    .unwrap_err()
                    .code,
                ErrorCode(-32602)
            );
        }

        #[test]
        fn accepts_current_as_synonym_for_active() {
            assert_eq!(
                super::parse_temporal_filter(Some("current")).unwrap(),
                TemporalFilter::Active
            );
        }

        #[test]
        fn accepts_history_as_synonym_for_all() {
            assert_eq!(
                super::parse_temporal_filter(Some("history")).unwrap(),
                TemporalFilter::All
            );
        }
    }

    mod map_graph_error {
        use super::*;

        #[test]
        fn maps_page_not_found_to_not_found_code() {
            assert_eq!(
                super::map_graph_error(GraphError::PageNotFound {
                    slug: "people/ghost".to_string()
                })
                .code,
                ErrorCode(-32001)
            );
        }
    }

    // ── memory_link ───────────────────────────────────────────

    #[test]
    fn memory_link_with_unknown_from_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme Corp\n",
        );

        let error = server
            .memory_link(MemoryLinkInput {
                from_slug: "people/ghost".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn memory_link_creates_link_between_existing_pages() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );

        let result = server
            .memory_link(MemoryLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: Some("2024-01".to_string()),
                valid_until: None,
            })
            .unwrap();

        let text = extract_text(&result);
        assert!(text.contains("Linked"));
        assert!(text.contains("default::people/alice"));
        assert!(text.contains("default::companies/acme"));
    }

    #[test]
    fn memory_link_rejects_invalid_relationship() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );

        let error = server
            .memory_link(MemoryLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    // ── memory_link_close ─────────────────────────────────────

    #[test]
    fn memory_link_close_with_unknown_id_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_link_close(MemoryLinkCloseInput {
                link_id: 99999,
                valid_until: "2025-06".to_string(),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn memory_link_close_sets_valid_until_on_existing_link() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );

        server
            .memory_link(MemoryLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: Some("2024-01".to_string()),
                valid_until: None,
            })
            .unwrap();

        let link_id: i64 = {
            let db = server.db.lock().unwrap();
            db.query_row("SELECT id FROM links ORDER BY id DESC LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap()
        };

        let result = server
            .memory_link_close(MemoryLinkCloseInput {
                link_id: link_id as u64,
                valid_until: "2025-06".to_string(),
            })
            .unwrap();

        let text = extract_text(&result);
        assert!(text.contains(&format!("Closed link {link_id}")));
    }

    #[test]
    fn memory_link_close_rejects_invalid_temporal_value() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_link_close(MemoryLinkCloseInput {
                link_id: 1,
                valid_until: "not-a-date".to_string(),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    // ── memory_backlinks ──────────────────────────────────────

    #[test]
    fn memory_backlinks_returns_link_array() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );

        server
            .memory_link(MemoryLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap();

        let result = server
            .memory_backlinks(MemoryBacklinksInput {
                slug: "companies/acme".to_string(),
                limit: None,
                temporal: None,
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["from_slug"], "default::people/alice");
        assert_eq!(arr[0]["relationship"], "works_at");
    }

    #[test]
    fn memory_backlinks_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_backlinks(MemoryBacklinksInput {
                slug: "nobody/ghost".to_string(),
                limit: None,
                temporal: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn memory_backlinks_applies_limit() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );

        for slug in ["people/alice", "people/bob", "people/carla"] {
            create_page(
                &server,
                slug,
                &format!("---\ntitle: {slug}\ntype: person\n---\n{slug}\n"),
            );
            server
                .memory_link(MemoryLinkInput {
                    from_slug: slug.to_string(),
                    to_slug: "companies/acme".to_string(),
                    relationship: "works_at".to_string(),
                    valid_from: None,
                    valid_until: None,
                })
                .unwrap();
        }

        let result = server
            .memory_backlinks(MemoryBacklinksInput {
                slug: "companies/acme".to_string(),
                limit: Some(2),
                temporal: None,
            })
            .unwrap();

        let text = extract_text(&result);
        let arr: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn memory_backlinks_temporal_all_includes_closed_links() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );

        server
            .memory_link(MemoryLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: Some("2020-01-01".to_string()),
                valid_until: Some("2020-12-31".to_string()),
            })
            .unwrap();

        let result = server
            .memory_backlinks(MemoryBacklinksInput {
                slug: "companies/acme".to_string(),
                limit: None,
                temporal: Some("all".to_string()),
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn memory_backlinks_rejects_invalid_temporal_filter() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_backlinks(MemoryBacklinksInput {
                slug: "people/alice".to_string(),
                limit: None,
                temporal: Some("future".to_string()),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    // ── memory_graph ──────────────────────────────────────────

    #[test]
    fn memory_graph_returns_nodes_and_edges_json() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );

        server
            .memory_link(MemoryLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap();

        let result = server
            .memory_graph(MemoryGraphInput {
                slug: "people/alice".to_string(),
                depth: Some(2),
                temporal: None,
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let node_slugs = parsed["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["slug"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(node_slugs.contains(&"default::people/alice"));
        assert!(node_slugs.contains(&"default::companies/acme"));
        assert!(parsed["edges"].as_array().unwrap().iter().any(|edge| {
            edge["from"] == "default::people/alice" && edge["to"] == "default::companies/acme"
        }));
    }

    #[test]
    fn memory_graph_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_graph(MemoryGraphInput {
                slug: "people/ghost".to_string(),
                depth: None,
                temporal: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn memory_graph_temporal_all_includes_closed_links() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );

        server
            .memory_link(MemoryLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: Some("2020-01-01".to_string()),
                valid_until: Some("2020-12-31".to_string()),
            })
            .unwrap();

        let result = server
            .memory_graph(MemoryGraphInput {
                slug: "people/alice".to_string(),
                depth: None,
                temporal: Some("all".to_string()),
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed["edges"].as_array().unwrap().len(), 1);
    }

    // ── memory_check ──────────────────────────────────────────

    #[test]
    fn memory_check_on_clean_page_returns_empty_array() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice is a person.\n",
        );

        let result = server
            .memory_check(MemoryCheckInput {
                slug: Some("people/alice".to_string()),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 0);
    }

    #[test]
    fn memory_check_detects_contradiction_on_page() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\n## Assertions\nAlice works at Acme Corp.\nAlice works at Beta Corp.\n",
        );

        let result = server
            .memory_check(MemoryCheckInput {
                slug: Some("people/alice".to_string()),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(!parsed.as_array().unwrap().is_empty());
    }

    #[test]
    fn memory_check_filters_output_to_requested_slug() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\n## Assertions\nAlice works at Acme Corp.\nAlice works at Beta Corp.\n",
        );
        create_page(
            &server,
            "people/bob",
            "---\ntitle: Bob\ntype: person\n---\n## Assertions\nBob works at Gamma LLC.\nBob works at Delta LLC.\n",
        );

        server
            .memory_check(MemoryCheckInput {
                slug: Some("people/bob".to_string()),
            })
            .unwrap();

        let result = server
            .memory_check(MemoryCheckInput {
                slug: Some("people/alice".to_string()),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        assert!(!parsed.is_empty());
        assert!(parsed.iter().all(|row| {
            row["page_slug"] == "default::people/alice"
                || row["other_page_slug"] == "default::people/alice"
        }));
    }

    #[test]
    fn memory_check_explicit_collection_slug_filters_to_resolved_page_when_slug_collides() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "memory", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\n## Assertions\nAlice works at Acme Corp.\nAlice works at Beta Corp.\n",
        );
        create_page_in_collection(
            &server,
            "memory",
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nMemory Alice is a person.\n",
        );

        server
            .memory_check(MemoryCheckInput {
                slug: Some("default::people/alice".to_string()),
            })
            .unwrap();

        let result = server
            .memory_check(MemoryCheckInput {
                slug: Some("memory::people/alice".to_string()),
            })
            .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn memory_tags_explicit_collection_slug_updates_only_resolved_page_when_slug_collides() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "memory", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nDefault Alice\n",
        );
        create_page_in_collection(
            &server,
            "memory",
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nMemory Alice\n",
        );

        server
            .memory_tags(MemoryTagsInput {
                slug: "memory::people/alice".to_string(),
                add: Some(vec!["memory".to_string()]),
                remove: None,
            })
            .unwrap();

        let db = server.db.lock().unwrap();
        let default_page_id: i64 = db
            .query_row(
                "SELECT id FROM pages WHERE collection_id = 1 AND slug = ?1",
                ["people/alice"],
                |row| row.get(0),
            )
            .unwrap();
        let memory_page_id: i64 = db
            .query_row(
                "SELECT id FROM pages WHERE collection_id = 2 AND slug = ?1",
                ["people/alice"],
                |row| row.get(0),
            )
            .unwrap();
        let default_tag_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM tags WHERE page_id = ?1",
                [default_page_id],
                |row| row.get(0),
            )
            .unwrap();
        let memory_tags: Vec<String> = db
            .prepare("SELECT tag FROM tags WHERE page_id = ?1 ORDER BY tag")
            .unwrap()
            .query_map([memory_page_id], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(default_tag_count, 0);
        assert_eq!(memory_tags, vec!["memory".to_string()]);
    }

    #[test]
    fn memory_check_without_slug_returns_all_unresolved_contradictions() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\n## Assertions\nAlice works at Acme Corp.\nAlice works at Beta Corp.\n",
        );
        create_page(
            &server,
            "people/bob",
            "---\ntitle: Bob\ntype: person\n---\n## Assertions\nBob works at Gamma LLC.\nBob works at Delta LLC.\n",
        );

        let result = server
            .memory_check(MemoryCheckInput { slug: None })
            .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    // ── memory_timeline ───────────────────────────────────────

    #[test]
    fn memory_timeline_on_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_timeline(MemoryTimelineInput {
                slug: "nobody/ghost".to_string(),
                limit: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn memory_timeline_returns_entries_for_page_with_timeline() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice bio\n\n## Timeline\n\n2024-01: Joined Acme\n---\n2024-06: Promoted\n",
        );

        let result = server
            .memory_timeline(MemoryTimelineInput {
                slug: "people/alice".to_string(),
                limit: Some(10),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["slug"], "default::people/alice");
    }

    #[test]
    fn memory_timeline_prefers_structured_entries_and_applies_limit() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice bio\n\n## Timeline\n\nlegacy entry\n",
        );
        {
            let db = server.db.lock().unwrap();
            insert_timeline_entry(
                &db,
                "people/alice",
                "2024-06-01",
                "Promoted",
                "memo",
                "to staff engineer",
            );
            insert_timeline_entry(&db, "people/alice", "2024-01-01", "Joined", "", "");
        }

        let result = server
            .memory_timeline(MemoryTimelineInput {
                slug: "people/alice".to_string(),
                limit: Some(1),
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn memory_timeline_returns_empty_entries_for_page_without_timeline_data() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice bio\n",
        );

        let result = server
            .memory_timeline(MemoryTimelineInput {
                slug: "people/alice".to_string(),
                limit: None,
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(parsed["entries"].as_array().unwrap().is_empty());
    }

    // ── memory_tags ───────────────────────────────────────────

    #[test]
    fn memory_tags_list_add_remove_round_trip() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        // List tags — should be empty
        let result = server
            .memory_tags(MemoryTagsInput {
                slug: "people/alice".to_string(),
                add: None,
                remove: None,
            })
            .unwrap();
        let text = extract_text(&result);
        let tags: Vec<String> = serde_json::from_str(&text).unwrap();
        assert!(tags.is_empty());

        // Add tags
        let result = server
            .memory_tags(MemoryTagsInput {
                slug: "people/alice".to_string(),
                add: Some(vec!["investor".to_string(), "founder".to_string()]),
                remove: None,
            })
            .unwrap();
        let text = extract_text(&result);
        let tags: Vec<String> = serde_json::from_str(&text).unwrap();
        assert_eq!(tags, vec!["founder", "investor"]);

        // Remove a tag
        let result = server
            .memory_tags(MemoryTagsInput {
                slug: "people/alice".to_string(),
                add: None,
                remove: Some(vec!["investor".to_string()]),
            })
            .unwrap();
        let text = extract_text(&result);
        let tags: Vec<String> = serde_json::from_str(&text).unwrap();
        assert_eq!(tags, vec!["founder"]);
    }

    #[test]
    fn memory_tags_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_tags(MemoryTagsInput {
                slug: "nobody/ghost".to_string(),
                add: Some(vec!["tag".to_string()]),
                remove: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn memory_tags_rejects_invalid_tag_values() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        let error = server
            .memory_tags(MemoryTagsInput {
                slug: "people/alice".to_string(),
                add: Some(vec!["bad tag".to_string()]),
                remove: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    // ── Phase 3 MCP tests ────────────────────────────────────

    // ── memory_gap ────────────────────────────────────────────

    #[test]
    fn memory_gap_with_empty_query_returns_invalid_params() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_gap(MemoryGapInput {
                query: "".to_string(),
                slug: None,
                context: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn memory_gap_stores_gap_with_null_query_text_and_internal_sensitivity() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let result = server
            .memory_gap(MemoryGapInput {
                query: "who invented quantum socks".to_string(),
                slug: None,
                context: Some("test context".to_string()),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(parsed["id"].as_i64().is_some());
        assert!(parsed["query_hash"].as_str().is_some());

        // Verify stored with NULL query_text and internal sensitivity
        let db = server.db.lock().unwrap();
        let (query_text, sensitivity, context): (Option<String>, String, String) = db
            .query_row(
                "SELECT query_text, sensitivity, context FROM knowledge_gaps WHERE id = ?1",
                [parsed["id"].as_i64().unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(query_text.is_none());
        assert_eq!(sensitivity, "internal");
        assert!(context.is_empty());
    }

    #[test]
    fn memory_gap_duplicate_is_idempotent() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let r1 = server
            .memory_gap(MemoryGapInput {
                query: "same query".to_string(),
                slug: None,
                context: None,
            })
            .unwrap();
        let r2 = server
            .memory_gap(MemoryGapInput {
                query: "same query".to_string(),
                slug: None,
                context: None,
            })
            .unwrap();

        let id1: serde_json::Value = serde_json::from_str(&extract_text(&r1)).unwrap();
        let id2: serde_json::Value = serde_json::from_str(&extract_text(&r2)).unwrap();
        assert_eq!(id1["id"], id2["id"]);
    }

    #[test]
    fn memory_gap_context_is_redacted_in_listings() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        server
            .memory_gap(MemoryGapInput {
                query: "sensitive query".to_string(),
                slug: None,
                context: Some("sensitive query with extra details".to_string()),
            })
            .unwrap();

        let result = server
            .memory_gaps(MemoryGapsInput {
                resolved: None,
                limit: None,
            })
            .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed.len(), 1);
        let context = parsed[0]["context"].as_str().unwrap_or_default();
        assert!(context.is_empty());
    }

    // ── memory_gaps ───────────────────────────────────────────

    #[test]
    fn memory_gaps_returns_array_with_limit() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        for i in 0..5 {
            server
                .memory_gap(MemoryGapInput {
                    query: format!("gap query {i}"),
                    slug: None,
                    context: None,
                })
                .unwrap();
        }

        let result = server
            .memory_gaps(MemoryGapsInput {
                resolved: None,
                limit: Some(3),
            })
            .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn memory_gaps_defaults_to_unresolved() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        server
            .memory_gap(MemoryGapInput {
                query: "unresolved gap".to_string(),
                slug: None,
                context: None,
            })
            .unwrap();

        let result = server
            .memory_gaps(MemoryGapsInput {
                resolved: None,
                limit: None,
            })
            .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0]["resolved_at"].is_null());
    }

    // ── memory_stats ──────────────────────────────────────────

    #[test]
    fn memory_stats_returns_all_expected_fields() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        let result = server.memory_stats(MemoryStatsInput {}).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed["page_count"], 1);
        assert!(parsed["link_count"].is_number());
        assert!(parsed["assertion_count"].is_number());
        assert!(parsed["contradiction_count"].is_number());
        assert!(parsed["gap_count"].is_number());
        assert!(parsed["embedding_count"].is_number());
        assert!(parsed["active_model"].is_string());
        assert!(parsed["db_size_bytes"].is_number());
    }

    #[test]
    fn memory_collections_is_read_only_and_returns_frozen_schema_fields() {
        let (_dir, conn) = open_test_db();
        insert_collection(&conn, 2, "archive", false);
        insert_collection(&conn, 3, "restore", false);
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/default-page",
            "---\ntitle: Default Page\ntype: note\n---\nDefault\n",
        );

        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections
             SET ignore_parse_errors = ?2
             WHERE id = ?1",
            rusqlite::params![
                1_i64,
                r#"[{"code":"parse_error","line":2,"raw":"**]","message":"invalid glob"}]"#
            ],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'detached',
                 root_path = 'C:\\vaults\\archive-detached'
             WHERE id = 2",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'restoring',
                 watcher_released_at = '2026-04-24T00:00:00Z',
                 restore_command_id = 'restore-1'
             WHERE id = 3",
            [],
        )
        .unwrap();
        let snapshot_before = db
            .prepare(
                "SELECT id, state, root_path, needs_full_sync, ignore_parse_errors, restore_command_id, watcher_released_at
                 FROM collections
                 ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(db);

        let result = server
            .memory_collections(MemoryCollectionsInput {})
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();

        let db = server.db.lock().unwrap();
        let snapshot_after = db
            .prepare(
                "SELECT id, state, root_path, needs_full_sync, ignore_parse_errors, restore_command_id, watcher_released_at
                 FROM collections
                 ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(snapshot_after, snapshot_before);
        drop(db);

        assert_eq!(rows.len(), 3);
        let expected_keys = [
            "name",
            "root_path",
            "state",
            "writable",
            "is_write_target",
            "page_count",
            "last_sync_at",
            "embedding_queue_depth",
            "ignore_parse_errors",
            "needs_full_sync",
            "recovery_in_progress",
            "integrity_blocked",
            "restore_in_progress",
        ];
        for row in &rows {
            let mut actual_keys = row.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
            actual_keys.sort();
            let mut expected = expected_keys
                .iter()
                .map(|key| key.to_string())
                .collect::<Vec<_>>();
            expected.sort();
            assert_eq!(actual_keys, expected);
        }

        let default = rows.iter().find(|row| row["name"] == "default").unwrap();
        assert!(default["root_path"].as_str().is_some());
        assert_eq!(default["state"], "active");
        assert!(default["writable"].as_bool().unwrap());
        assert!(default["is_write_target"].as_bool().unwrap());
        assert_eq!(default["page_count"], 1);
        assert!(default["last_sync_at"].is_null());
        assert!(default["embedding_queue_depth"].as_i64().is_some());
        assert!(!default["needs_full_sync"].as_bool().unwrap());
        assert!(!default["recovery_in_progress"].as_bool().unwrap());
        assert!(default["integrity_blocked"].is_null());
        assert!(!default["restore_in_progress"].as_bool().unwrap());

        let archive = rows.iter().find(|row| row["name"] == "archive").unwrap();
        assert!(archive["root_path"].is_null());
        assert_eq!(archive["state"], "detached");
        assert!(archive["ignore_parse_errors"].is_null());

        let parse_errors = default["ignore_parse_errors"].as_array().unwrap();
        assert_eq!(parse_errors.len(), 1);
        assert_eq!(parse_errors[0]["code"].as_str(), Some("parse_error"));
        assert_eq!(parse_errors[0]["line"].as_i64(), Some(2));
        assert_eq!(parse_errors[0]["raw"].as_str(), Some("**]"));
        assert_eq!(parse_errors[0]["message"].as_str(), Some("invalid glob"));

        let restore = rows.iter().find(|row| row["name"] == "restore").unwrap();
        assert!(restore["root_path"].is_null());
        assert_eq!(restore["state"], "restoring");
        assert!(restore["restore_in_progress"].as_bool().unwrap());
    }

    #[test]
    fn memory_collections_surfaces_status_flags_and_terminal_precedence() {
        const QUEUED_ID: i64 = 20_002;
        const RUNNING_ID: i64 = 20_003;
        const TAMPERED_ID: i64 = 20_004;
        const REASON_ONLY_ID: i64 = 20_005;
        const DUPLICATE_ID: i64 = 20_006;
        const TRIVIAL_ID: i64 = 20_007;
        const RESTORE_PENDING_ID: i64 = 20_008;
        const WITHIN_WINDOW_ID: i64 = 20_009;
        const ESCALATED_ID: i64 = 20_010;
        const PRECEDENCE_ID: i64 = 20_011;
        const ABSENT_ID: i64 = 20_012;

        let (_dir, conn) = open_test_db();
        insert_collection(&conn, QUEUED_ID, "queued", false);
        insert_collection(&conn, RUNNING_ID, "running", false);
        insert_collection(&conn, TAMPERED_ID, "tampered", false);
        insert_collection(&conn, REASON_ONLY_ID, "reason-only", false);
        insert_collection(&conn, DUPLICATE_ID, "duplicate", false);
        insert_collection(&conn, TRIVIAL_ID, "trivial", false);
        insert_collection(&conn, RESTORE_PENDING_ID, "restore-pending", false);
        insert_collection(&conn, WITHIN_WINDOW_ID, "within-window", false);
        insert_collection(&conn, ESCALATED_ID, "escalated", false);
        insert_collection(&conn, PRECEDENCE_ID, "precedence", false);
        insert_collection(&conn, ABSENT_ID, "absent", false);
        let server = QuaidServer::new(conn);

        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'active',
                  needs_full_sync = 1
              WHERE id = ?1",
            [QUEUED_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'active',
                  needs_full_sync = 1
             WHERE id = ?1",
            [RUNNING_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'restoring',
                  integrity_failed_at = '2026-04-24T00:00:00Z'
             WHERE id = ?1",
            [TAMPERED_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'active',
                  reconcile_halt_reason = 'duplicate_uuid',
                  reconcile_halted_at = '2026-04-24T00:00:00Z'
             WHERE id = ?1",
            [DUPLICATE_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'active',
                  reconcile_halt_reason = 'duplicate_uuid'
             WHERE id = ?1",
            [REASON_ONLY_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'active',
                  reconcile_halt_reason = 'unresolvable_trivial_content',
                  reconcile_halted_at = '2026-04-24T00:00:00Z'
             WHERE id = ?1",
            [TRIVIAL_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'restoring',
                  restore_command_id = 'restore-pending-1'
             WHERE id = ?1",
            [RESTORE_PENDING_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'restoring',
                  pending_manifest_incomplete_at = datetime('now', '-31 seconds')
             WHERE id = ?1",
            [WITHIN_WINDOW_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'restoring',
                  pending_manifest_incomplete_at = datetime('now', '-31 minutes'),
                  reconcile_halt_reason = 'duplicate_uuid',
                  reconcile_halted_at = '2026-04-24T00:00:00Z'
             WHERE id = ?1",
            [ESCALATED_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET state = 'restoring',
                  integrity_failed_at = '2026-04-24T00:00:00Z',
                  pending_manifest_incomplete_at = datetime('now', '-31 minutes'),
                  reconcile_halt_reason = 'unresolvable_trivial_content',
                  reconcile_halted_at = '2026-04-24T00:00:00Z'
             WHERE id = ?1",
            [PRECEDENCE_ID],
        )
        .unwrap();
        db.execute(
            "UPDATE collections
             SET ignore_parse_errors = ?2
             WHERE id = ?1",
            rusqlite::params![
                ABSENT_ID,
                r#"[{"code":"file_stably_absent_but_clear_not_confirmed","line":0,"raw":"","message":".quaidignore absent but prior mirror exists; use `quaid collection ignore clear <name> --confirm` to clear explicitly"}]"#
            ],
        )
        .unwrap();
        drop(db);

        vault_sync::set_collection_recovery_in_progress_for_test(RUNNING_ID, true);
        let result = server
            .memory_collections(MemoryCollectionsInput {})
            .unwrap();
        vault_sync::set_collection_recovery_in_progress_for_test(RUNNING_ID, false);

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        let queued = rows.iter().find(|row| row["name"] == "queued").unwrap();
        assert!(queued["needs_full_sync"].as_bool().unwrap());
        assert!(!queued["recovery_in_progress"].as_bool().unwrap());
        assert!(queued["integrity_blocked"].is_null());

        let running = rows.iter().find(|row| row["name"] == "running").unwrap();
        assert!(running["needs_full_sync"].as_bool().unwrap());
        assert!(running["recovery_in_progress"].as_bool().unwrap());
        assert!(running["integrity_blocked"].is_null());

        let tampered = rows.iter().find(|row| row["name"] == "tampered").unwrap();
        assert_eq!(
            tampered["integrity_blocked"].as_str(),
            Some("manifest_tampering")
        );

        let reason_only = rows
            .iter()
            .find(|row| row["name"] == "reason-only")
            .unwrap();
        assert!(reason_only["integrity_blocked"].is_null());

        let duplicate = rows.iter().find(|row| row["name"] == "duplicate").unwrap();
        assert_eq!(
            duplicate["integrity_blocked"].as_str(),
            Some("duplicate_uuid")
        );

        let trivial = rows.iter().find(|row| row["name"] == "trivial").unwrap();
        assert_eq!(
            trivial["integrity_blocked"].as_str(),
            Some("unresolvable_trivial_content")
        );

        let restore_pending = rows
            .iter()
            .find(|row| row["name"] == "restore-pending")
            .unwrap();
        assert_eq!(restore_pending["state"], "restoring");
        assert!(!restore_pending["restore_in_progress"].as_bool().unwrap());

        let within_window = rows
            .iter()
            .find(|row| row["name"] == "within-window")
            .unwrap();
        assert!(within_window["integrity_blocked"].is_null());

        let escalated = rows.iter().find(|row| row["name"] == "escalated").unwrap();
        assert_eq!(
            escalated["integrity_blocked"].as_str(),
            Some("manifest_incomplete_escalated")
        );

        let precedence = rows.iter().find(|row| row["name"] == "precedence").unwrap();
        assert_eq!(
            precedence["integrity_blocked"].as_str(),
            Some("manifest_tampering")
        );

        let absent = rows.iter().find(|row| row["name"] == "absent").unwrap();
        let absent_errors = absent["ignore_parse_errors"].as_array().unwrap();
        assert_eq!(absent_errors.len(), 1);
        assert_eq!(
            absent_errors[0]["code"].as_str(),
            Some("file_stably_absent_but_clear_not_confirmed")
        );
        assert!(absent_errors[0]["line"].is_null());
        assert!(absent_errors[0]["raw"].is_null());
        assert_eq!(
            absent_errors[0]["message"].as_str(),
            Some(".quaidignore absent but prior mirror exists; use `quaid collection ignore clear <name> --confirm` to clear explicitly")
        );
    }

    // ── memory_raw ────────────────────────────────────────────

    #[test]
    fn memory_raw_with_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_raw(MemoryRawInput {
                slug: "nobody/ghost".to_string(),
                source: "test".to_string(),
                data: json!({"key": "value"}),
                overwrite: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn memory_raw_with_valid_slug_stores_row() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        let result = server
            .memory_raw(MemoryRawInput {
                slug: "people/alice".to_string(),
                source: "crustdata".to_string(),
                data: json!({"funding": "$10M", "headcount": 50}),
                overwrite: None,
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(parsed["id"].as_i64().is_some());

        // Verify data was stored
        let db = server.db.lock().unwrap();
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM raw_data WHERE source = 'crustdata'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn memory_raw_rejects_empty_source() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        let error = server
            .memory_raw(MemoryRawInput {
                slug: "people/alice".to_string(),
                source: "".to_string(),
                data: json!({}),
                overwrite: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn memory_raw_rejects_invalid_slug() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let error = server
            .memory_raw(MemoryRawInput {
                slug: "Invalid/SLUG!".to_string(),
                source: "test".to_string(),
                data: json!({}),
                overwrite: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn memory_raw_rejects_array_payload() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        let error = server
            .memory_raw(MemoryRawInput {
                slug: "people/alice".to_string(),
                source: "crustdata".to_string(),
                data: json!([1, 2, 3]),
                overwrite: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
        assert!(error.message.contains("JSON object"));
    }

    #[test]
    fn memory_raw_rejects_scalar_payload() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        for bad in [json!("string"), json!(42), json!(true), json!(null)] {
            let error = server
                .memory_raw(MemoryRawInput {
                    slug: "people/alice".to_string(),
                    source: "crustdata".to_string(),
                    data: bad,
                    overwrite: None,
                })
                .unwrap_err();
            assert_eq!(error.code, ErrorCode(-32602));
        }
    }

    #[test]
    fn memory_raw_rejects_duplicate_source_without_overwrite() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        server
            .memory_raw(MemoryRawInput {
                slug: "people/alice".to_string(),
                source: "crustdata".to_string(),
                data: json!({"v": 1}),
                overwrite: None,
            })
            .unwrap();

        // Second write without overwrite must fail.
        let error = server
            .memory_raw(MemoryRawInput {
                slug: "people/alice".to_string(),
                source: "crustdata".to_string(),
                data: json!({"v": 2}),
                overwrite: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32003));
        assert!(error.message.contains("overwrite=true"));
    }

    #[test]
    fn memory_raw_overwrites_when_flag_is_true() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        server
            .memory_raw(MemoryRawInput {
                slug: "people/alice".to_string(),
                source: "crustdata".to_string(),
                data: json!({"v": 1}),
                overwrite: None,
            })
            .unwrap();

        // Explicit overwrite must succeed and persist new data.
        server
            .memory_raw(MemoryRawInput {
                slug: "people/alice".to_string(),
                source: "crustdata".to_string(),
                data: json!({"v": 2}),
                overwrite: Some(true),
            })
            .unwrap();

        let db = server.db.lock().unwrap();
        let stored: String = db
            .query_row(
                "SELECT data FROM raw_data WHERE source = 'crustdata'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(v["v"], 2);
    }

    #[test]
    fn memory_raw_refuses_when_collection_needs_full_sync_even_if_not_restoring() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_raw(MemoryRawInput {
                slug: "people/alice".to_string(),
                source: "crustdata".to_string(),
                data: json!({"v": 1}),
                overwrite: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(error.message.contains("CollectionRestoringError"));
    }

    #[test]
    fn memory_gap_rejects_oversized_context() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let big_context = "x".repeat(501);
        let error = server
            .memory_gap(MemoryGapInput {
                query: "who invented quantum socks".to_string(),
                slug: None,
                context: Some(big_context),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
        assert!(error.message.contains("context"));
    }

    #[test]
    fn memory_gap_accepts_context_at_max_length() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let exact_context = "y".repeat(500);
        server
            .memory_gap(MemoryGapInput {
                query: "boundary test query".to_string(),
                slug: None,
                context: Some(exact_context),
            })
            .unwrap();
    }

    #[test]
    fn memory_gap_without_slug_succeeds_while_collection_is_restoring() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let result = server
            .memory_gap(MemoryGapInput {
                query: "record this globally".to_string(),
                slug: None,
                context: None,
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(parsed["id"].as_i64().is_some());

        let db = server.db.lock().unwrap();
        let page_id: Option<i64> = db
            .query_row(
                "SELECT page_id FROM knowledge_gaps WHERE id = ?1",
                [parsed["id"].as_i64().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(page_id.is_none());
    }

    #[test]
    fn memory_gap_with_slug_refuses_while_collection_is_restoring() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/restore-gap",
            "---\ntitle: Restore Gap\ntype: note\n---\ncontent\n",
        );
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_gap(MemoryGapInput {
                query: "page-bound gap".to_string(),
                slug: Some("notes/restore-gap".to_string()),
                context: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(error.message.contains("CollectionRestoringError"));
    }

    #[test]
    fn memory_gap_with_slug_binds_gap_to_page_id() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/bound-gap",
            "---\ntitle: Bound Gap\ntype: note\n---\ncontent\n",
        );

        let result = server
            .memory_gap(MemoryGapInput {
                query: "page-bound gap".to_string(),
                slug: Some("notes/bound-gap".to_string()),
                context: None,
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        let db = server.db.lock().unwrap();
        let (page_id, stored_page_id): (i64, Option<i64>) = db
            .query_row(
                "SELECT p.id, g.page_id
                 FROM pages p
                 JOIN knowledge_gaps g ON g.id = ?1
                 WHERE p.slug = 'notes/bound-gap'",
                [parsed["id"].as_i64().unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(stored_page_id, Some(page_id));
    }

    #[test]
    fn memory_gap_with_slug_refuses_when_collection_needs_full_sync() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/needs-sync-gap",
            "---\ntitle: Needs Sync Gap\ntype: note\n---\ncontent\n",
        );
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_gap(MemoryGapInput {
                query: "page-bound gap".to_string(),
                slug: Some("notes/needs-sync-gap".to_string()),
                context: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(error.message.contains("CollectionRestoringError"));
        assert!(error.message.contains("needs_full_sync=true"));
    }

    // ── 17.5s2 write-interlock mutator matrix ────────────────
    // memory_put + state='restoring'
    #[test]
    fn memory_put_refuses_when_collection_is_restoring() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/blocked".to_string(),
                content: "---\ntitle: Blocked\ntype: note\n---\nBlocked\n".to_string(),
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(
            error.message.contains("CollectionRestoringError"),
            "memory_put must refuse with CollectionRestoringError when state=restoring: {error:?}"
        );
    }

    // ── 17.5s5 memory_link refused during restoring ───────────
    #[test]
    fn memory_link_refuses_when_collection_is_restoring() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme\n",
        );
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_link(MemoryLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(
            error.message.contains("CollectionRestoringError"),
            "memory_link must refuse with CollectionRestoringError when state=restoring: {error:?}"
        );
    }

    // memory_link + needs_full_sync=1
    #[test]
    fn memory_link_refuses_when_collection_needs_full_sync() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/bob",
            "---\ntitle: Bob\ntype: person\n---\nBob\n",
        );
        create_page(
            &server,
            "companies/initech",
            "---\ntitle: Initech\ntype: company\n---\nInitech\n",
        );
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_link(MemoryLinkInput {
                from_slug: "people/bob".to_string(),
                to_slug: "companies/initech".to_string(),
                relationship: "works_at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(
            error.message.contains("CollectionRestoringError"),
            "memory_link must refuse with CollectionRestoringError when needs_full_sync=1: {error:?}"
        );
        assert!(error.message.contains("needs_full_sync=true"));
    }

    // ── 17.5s5 memory_check refused during restoring ──────────
    #[test]
    fn memory_check_refuses_when_collection_is_restoring() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/check-restoring",
            "---\ntitle: Check Restoring\ntype: note\n---\ncontent\n",
        );
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_check(MemoryCheckInput {
                slug: Some("notes/check-restoring".to_string()),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(
            error.message.contains("CollectionRestoringError"),
            "memory_check must refuse with CollectionRestoringError when state=restoring: {error:?}"
        );
    }

    // memory_check + needs_full_sync=1
    #[test]
    fn memory_check_refuses_when_collection_needs_full_sync() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/check-needs-sync",
            "---\ntitle: Check Needs Sync\ntype: note\n---\ncontent\n",
        );
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_check(MemoryCheckInput {
                slug: Some("notes/check-needs-sync".to_string()),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(
            error.message.contains("CollectionRestoringError"),
            "memory_check must refuse with CollectionRestoringError when needs_full_sync=1: {error:?}"
        );
        assert!(error.message.contains("needs_full_sync=true"));
    }

    // ── 17.5s5 memory_raw refused during restoring ────────────
    #[test]
    fn memory_raw_refuses_when_collection_is_restoring() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "people/carol",
            "---\ntitle: Carol\ntype: person\n---\nCarol\n",
        );
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_raw(MemoryRawInput {
                slug: "people/carol".to_string(),
                source: "crustdata".to_string(),
                data: json!({"v": 1}),
                overwrite: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(
            error.message.contains("CollectionRestoringError"),
            "memory_raw must refuse with CollectionRestoringError when state=restoring: {error:?}"
        );
    }

    // ── M1b-ii ordering proofs: interlock wins over OCC ──────
    // Collection restoring + page EXISTS + no expected_version → CollectionRestoringError, not "already exists"
    #[test]
    fn memory_put_collection_interlock_wins_over_update_without_expected_version() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/interlock-exists",
            "---\ntitle: Interlock\ntype: note\n---\nexisting\n",
        );
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/interlock-exists".to_string(),
                content: "---\ntitle: Interlock\ntype: note\n---\novewrite attempt\n".to_string(),
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(
            error.message.contains("CollectionRestoringError"),
            "collection interlock must win over 'already exists' conflict: {error:?}"
        );
    }

    // Collection restoring + page ABSENT + expected_version supplied → CollectionRestoringError, not "does not exist at version N"
    #[test]
    fn memory_put_collection_interlock_wins_over_ghost_expected_version() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        let db = server.db.lock().unwrap();
        db.execute(
            "UPDATE collections SET state = 'restoring' WHERE id = 1",
            [],
        )
        .unwrap();
        drop(db);

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/ghost-version".to_string(),
                content: "---\ntitle: Ghost\ntype: note\n---\ncontent\n".to_string(),
                expected_version: Some(1),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(
            error.message.contains("CollectionRestoringError"),
            "collection interlock must win over ghost-version OCC conflict: {error:?}"
        );
    }

    // ── 17.5qq11 MCP path ────────────────────────────────────
    #[test]
    fn memory_put_refuses_when_collection_is_read_only() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        let db = server.db.lock().unwrap();
        db.execute("UPDATE collections SET writable = 0 WHERE id = 1", [])
            .unwrap();
        drop(db);

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/read-only-page".to_string(),
                content: "---\ntitle: Read Only\ntype: note\n---\nhello\n".to_string(),
                expected_version: None,
            })
            .unwrap_err();

        assert!(
            error.message.contains("CollectionReadOnlyError"),
            "memory_put must surface CollectionReadOnlyError when collection is read-only: {error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn memory_put_happy_path_updates_file_and_clears_mechanical_artifacts() {
        let (_dir, db_path, conn, vault_root) = open_test_db_with_vault();
        let server = QuaidServer::new(conn);
        let original = "---\ntitle: Happy\ntype: note\n---\nOriginal body\n";
        let updated = "---\ntitle: Happy\ntype: note\n---\nUpdated body\n";

        server
            .memory_put(MemoryPutInput {
                slug: "notes/happy".to_string(),
                content: original.to_string(),
                expected_version: None,
            })
            .unwrap();
        server
            .memory_put(MemoryPutInput {
                slug: "notes/happy".to_string(),
                content: updated.to_string(),
                expected_version: Some(1),
            })
            .unwrap();

        assert_eq!(recovery_sentinel_count(&db_path, 1), 0);
        assert_eq!(
            fs::read_to_string(vault_root.join("notes").join("happy.md")).unwrap(),
            updated
        );
        let db = server.db.lock().unwrap();
        assert_eq!(page_version(&db, "notes/happy"), 2);
        assert_eq!(active_raw_import_count(&db, "notes/happy"), 1);
    }

    // ── 1.1b response completeness ───────────────────────────
    #[test]
    fn memory_gap_with_slug_response_includes_page_id() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "notes/response-gap",
            "---\ntitle: Response Gap\ntype: note\n---\ncontent\n",
        );

        let result = server
            .memory_gap(MemoryGapInput {
                query: "gap with page bound".to_string(),
                slug: Some("notes/response-gap".to_string()),
                context: None,
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(
            parsed["page_id"].as_i64().is_some(),
            "memory_gap with slug must return page_id in response: {parsed}"
        );
    }

    #[test]
    fn memory_gap_without_slug_response_has_null_page_id() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let result = server
            .memory_gap(MemoryGapInput {
                query: "global gap no page".to_string(),
                slug: None,
                context: None,
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(
            parsed["page_id"].is_null(),
            "memory_gap without slug must return null page_id: {parsed}"
        );
    }

    fn extract_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| match &c.raw {
                RawContent::Text(tc) => Some(tc.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}
