use std::sync::{Arc, Mutex};

use rmcp::model::*;
use rmcp::schemars;
use rmcp::tool;
use rmcp::{ServerHandler, ServiceExt};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::commands::{check, get, link, put};

use crate::core::collections::{self, OpKind, SlugResolution};
use crate::core::conversation::{
    correction, extractor::SlmClient, queue as conversation_queue, slm::LazySlmRunner, turn_writer,
};
use crate::core::fts::sanitize_fts_query;
use crate::core::gaps;
#[cfg(test)]
use crate::core::graph::GraphError;
use crate::core::graph::{self, TemporalFilter};
use crate::core::namespace;
use crate::core::progressive::progressive_retrieve_with_namespace;
use crate::core::search::{hybrid_search, HybridSearch};
use crate::core::supersede;
use crate::core::types::{ExtractionTriggerKind, TurnRole};
use crate::core::vault_sync;
use crate::mcp::errors::{
    ambiguous_slug_error, invalid_params, map_anyhow_error, map_close_action_put_error,
    map_collection_error, map_config_error, map_correction_error, map_db_error,
    map_extraction_queue_error, map_gaps_error, map_graph_error, map_namespace_error,
    map_search_error, map_serialize_error, map_turn_write_error, map_vault_sync_error,
    serialize_response,
};
use crate::mcp::validation::{
    parse_temporal_filter, validate_close_action_status, validate_content, validate_relationship,
    validate_slug, validate_tag_list, validate_temporal_value, validate_turn_timestamp,
    MAX_GAP_CONTEXT_LEN, MAX_LIMIT, MAX_RAW_DATA_LEN,
};
#[cfg(test)]
use crate::mcp::validation::{MAX_SLUG_LEN, MAX_TAGS_PER_REQUEST};

type DbRef = Arc<Mutex<Connection>>;
type SlmRef = Arc<dyn SlmClient + Send + Sync>;

fn canonical_slug(collection_name: &str, slug: &str) -> String {
    format!("{collection_name}::{slug}")
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
    crate::core::page_uuid::canonicalize_frontmatter_uuid(
        &mut rendered.frontmatter,
        &rendered.uuid,
    );
    rendered.slug = canonical_slug(&resolved.collection_name, &resolved.slug);
    crate::core::types::frontmatter_insert_string(
        &mut rendered.frontmatter,
        "slug",
        canonical_slug(&resolved.collection_name, &resolved.slug),
    );
    rendered
}

fn append_note(body: &mut String, note: &str) {
    if note.trim().is_empty() {
        return;
    }
    if body.is_empty() {
        body.push_str(note);
        return;
    }
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push('\n');
    body.push_str(note);
}


fn extraction_enabled(db: &Connection) -> Result<bool, rmcp::Error> {
    let raw = crate::core::db::read_config_value_or(db, "extraction.enabled", "false")
        .map_err(map_config_error)?;
    match raw.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(map_config_error(format!(
            "invalid extraction.enabled value: {other}"
        ))),
    }
}

fn extraction_debounce_ms(db: &Connection) -> Result<i64, rmcp::Error> {
    let raw = crate::core::db::read_config_value_or(db, "extraction.debounce_ms", "5000")
        .map_err(map_config_error)?;
    raw.parse::<i64>().map_err(|_| {
        map_config_error(format!("invalid extraction.debounce_ms value: {raw}"))
    })
}

#[derive(Clone)]
pub struct QuaidServer {
    db: DbRef,
    slm: SlmRef,
}

impl QuaidServer {
    pub fn new(conn: Connection) -> Self {
        Self::new_with_slm(conn, Arc::new(LazySlmRunner::new()))
    }

    pub fn new_with_slm<S>(conn: Connection, slm: Arc<S>) -> Self
    where
        S: SlmClient + Send + Sync + 'static,
    {
        Self {
            db: Arc::new(Mutex::new(conn)),
            slm,
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
    /// Optional namespace to write into; omitted writes global memory
    pub namespace: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryAddTurnInput {
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub namespace: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryCloseSessionInput {
    pub session_id: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryCloseActionInput {
    pub slug: String,
    pub status: String,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryCorrectInput {
    pub fact_slug: String,
    pub correction: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryCorrectContinueInput {
    pub correction_id: String,
    pub response: Option<String>,
    pub abandon: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryQueryInput {
    /// Search query string
    pub query: String,
    /// Optional collection filter
    pub collection: Option<String>,
    /// Optional namespace filter
    pub namespace: Option<String>,
    /// Optional wing filter
    pub wing: Option<String>,
    /// Maximum results to return
    pub limit: Option<u32>,
    /// Retrieval depth: "auto" for progressive expansion, absent/empty for direct results only
    pub depth: Option<String>,
    /// Include superseded historical pages in results
    pub include_superseded: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemorySearchInput {
    /// FTS5 search query string
    pub query: String,
    /// Optional collection filter
    pub collection: Option<String>,
    /// Optional namespace filter
    pub namespace: Option<String>,
    /// Optional wing filter
    pub wing: Option<String>,
    /// Maximum results to return
    pub limit: Option<u32>,
    /// Include superseded historical pages in results
    pub include_superseded: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryListInput {
    /// Optional collection filter
    pub collection: Option<String>,
    /// Optional namespace filter
    pub namespace: Option<String>,
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
pub struct MemoryNamespaceCreateInput {
    /// Namespace ID to create
    pub id: String,
    /// Optional TTL in hours
    pub ttl_hours: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryNamespaceDestroyInput {
    /// Namespace ID to destroy
    pub id: String,
}

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
        let canonical_page = canonicalize_page_for_mcp(&page, &resolved);
        let successor_slug = supersede::successor_slug_by_id(&db, canonical_page.superseded_by)
            .map_err(map_db_error)?;
        let supersedes = canonical_page
            .frontmatter
            .get("supersedes")
            .and_then(serde_json::Value::as_str)
            .map(|slug| canonical_slug(&resolved.collection_name, slug));

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "slug": canonical_page.slug,
            "uuid": canonical_page.uuid,
            "type": canonical_page.page_type,
            "title": canonical_page.title,
            "summary": canonical_page.summary,
            "compiled_truth": canonical_page.compiled_truth,
            "timeline": canonical_page.timeline,
            "frontmatter": canonical_page.frontmatter,
            "wing": canonical_page.wing,
            "room": canonical_page.room,
            "version": canonical_page.version,
            "created_at": canonical_page.created_at,
            "updated_at": canonical_page.updated_at,
            "truth_updated_at": canonical_page.truth_updated_at,
            "timeline_updated_at": canonical_page.timeline_updated_at,
            "supersedes": supersedes,
            "superseded_by": successor_slug,
        }))
        .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Append a turn to a conversation session")]
    pub fn memory_add_turn(
        &self,
        #[tool(aggr)] input: MemoryAddTurnInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_content(&input.content)?;
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        if let Some(metadata) = input.metadata.as_ref() {
            if !metadata.is_object() {
                return Err(invalid_params("metadata must be a JSON object"));
            }
        }

        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let role = input.role.parse::<TurnRole>().map_err(invalid_params)?;
        let timestamp = match input.timestamp.as_deref() {
            Some(timestamp) => {
                validate_turn_timestamp(timestamp)?;
                timestamp.to_owned()
            }
            None => {
                conversation_queue::current_timestamp(&db).map_err(map_extraction_queue_error)?
            }
        };

        let write_result = turn_writer::append_turn(
            &db,
            &input.session_id,
            role,
            &input.content,
            &timestamp,
            input.metadata,
            input.namespace.as_deref(),
        )
        .map_err(map_turn_write_error)?;

        let extraction_scheduled_at = if extraction_enabled(&db)? {
            let scheduled_for =
                conversation_queue::scheduled_timestamp_after_ms(&db, extraction_debounce_ms(&db)?)
                    .map_err(map_extraction_queue_error)?;
            let queue_session_id = conversation_queue::session_queue_key(
                input.namespace.as_deref(),
                &input.session_id,
            );
            conversation_queue::enqueue(
                &db,
                &queue_session_id,
                &write_result.conversation_path,
                ExtractionTriggerKind::Debounce,
                &scheduled_for,
            )
            .map_err(map_extraction_queue_error)?;
            Some(scheduled_for)
        } else {
            None
        };

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "turn_id": write_result.turn_id,
            "conversation_path": write_result.conversation_path,
            "extraction_scheduled_at": extraction_scheduled_at,
        }))
        .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Close a conversation session and trigger extraction")]
    pub fn memory_close_session(
        &self,
        #[tool(aggr)] input: MemoryCloseSessionInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let close_result =
            turn_writer::close_session(&db, &input.session_id, input.namespace.as_deref())
                .map_err(map_turn_write_error)?;

        let queue_session_id =
            conversation_queue::session_queue_key(input.namespace.as_deref(), &input.session_id);
        let (extraction_triggered, queue_position) = if close_result.newly_closed {
            let scheduled_for =
                conversation_queue::current_timestamp(&db).map_err(map_extraction_queue_error)?;
            conversation_queue::enqueue(
                &db,
                &queue_session_id,
                &close_result.conversation_path,
                ExtractionTriggerKind::SessionClose,
                &scheduled_for,
            )
            .map_err(map_extraction_queue_error)?;
            let position = conversation_queue::pending_queue_position(&db, &queue_session_id)
                .map_err(map_extraction_queue_error)?
                .unwrap_or(0);
            (true, position)
        } else {
            let position = conversation_queue::pending_queue_position(&db, &queue_session_id)
                .map_err(map_extraction_queue_error)?
                .unwrap_or(0);
            (position > 0, position)
        };

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "closed_at": close_result.closed_at,
            "extraction_triggered": extraction_triggered,
            "queue_position": queue_position,
        }))
        .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Close an action item in place")]
    pub fn memory_close_action(
        &self,
        #[tool(aggr)] input: MemoryCloseActionInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        self.memory_close_action_impl(input, |_, _, _| Ok(()))
    }

    fn memory_close_action_impl<F>(
        &self,
        input: MemoryCloseActionInput,
        before_write: F,
    ) -> Result<CallToolResult, rmcp::Error>
    where
        F: FnOnce(
            &Connection,
            &vault_sync::ResolvedSlug,
            &crate::core::types::Page,
        ) -> Result<(), rmcp::Error>,
    {
        validate_slug(&input.slug)?;
        validate_close_action_status(&input.status)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::WriteUpdate)?;
        vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)
            .map_err(map_vault_sync_error)?;
        let page = get::get_page_by_key(&db, resolved.collection_id, &resolved.slug)
            .map_err(map_anyhow_error)?;
        if page.page_type != "action_item" {
            return Err(rmcp::Error::new(
                ErrorCode(-32002),
                format!(
                    "KindError: page `{}` is `{}` not `action_item`",
                    canonical_slug(&resolved.collection_name, &resolved.slug),
                    page.page_type
                ),
                None,
            ));
        }

        let mut updated_page = page.clone();
        crate::core::types::frontmatter_insert_string(
            &mut updated_page.frontmatter,
            "status",
            input.status.clone(),
        );
        if let Some(note) = input.note.as_deref() {
            append_note(&mut updated_page.compiled_truth, note);
        }

        before_write(&db, &resolved, &updated_page)?;

        let content = crate::core::markdown::render_page(&updated_page);
        put::put_from_string_quiet(
            &db,
            &canonical_slug(&resolved.collection_name, &resolved.slug),
            &content,
            Some(page.version),
        )
        .map_err(|error| map_close_action_put_error(&db, &resolved, error))?;

        let (updated_at, version): (String, i64) = db
            .query_row(
                "SELECT updated_at, version
                 FROM pages
                 WHERE collection_id = ?1 AND slug = ?2",
                rusqlite::params![resolved.collection_id, &resolved.slug],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(map_db_error)?;

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "updated_at": updated_at,
            "version": version,
        }))
        .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Start a correction dialogue for an extracted fact")]
    pub fn memory_correct(
        &self,
        #[tool(aggr)] input: MemoryCorrectInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.fact_slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let step = correction::start_correction(
            &db,
            self.slm.as_ref(),
            &input.fact_slug,
            &input.correction,
        )
        .map_err(map_correction_error)?;
        let json = serde_json::to_string_pretty(&step)
            .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Continue or abandon an open fact correction dialogue")]
    pub fn memory_correct_continue(
        &self,
        #[tool(aggr)] input: MemoryCorrectContinueInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let step = correction::continue_correction(
            &db,
            self.slm.as_ref(),
            &input.correction_id,
            input.response.as_deref(),
            input.abandon.unwrap_or(false),
        )
        .map_err(map_correction_error)?;
        let json = serde_json::to_string_pretty(&step)
            .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Write or update a page")]
    pub fn memory_put(
        &self,
        #[tool(aggr)] input: MemoryPutInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        validate_content(&input.content)?;
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let namespace_filter = input.namespace.as_deref().unwrap_or("");
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
                "SELECT version
                 FROM pages
                 WHERE collection_id = ?1 AND namespace = ?2 AND slug = ?3",
                rusqlite::params![resolved.collection_id, namespace_filter, &resolved.slug],
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
        crate::commands::put::put_from_string_quiet_with_namespace(
            &db,
            &canonical_slug(&resolved.collection_name, &resolved.slug),
            &input.content,
            Some(namespace_filter),
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
                "SELECT version
                 FROM pages
                 WHERE collection_id = ?1 AND namespace = ?2 AND slug = ?3",
                rusqlite::params![resolved.collection_id, namespace_filter, &resolved.slug],
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
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let namespace_filter = input.namespace.as_deref().or(Some(""));
        let collection_filter =
            resolve_read_collection_filter_for_mcp(&db, input.collection.as_deref())?;
        let include_superseded = input.include_superseded.unwrap_or(false);

        let limit = input.limit.unwrap_or(10).min(MAX_LIMIT) as usize;
        let results = hybrid_search(
            &db,
            HybridSearch {
                query: &input.query,
                wing: input.wing.as_deref(),
                collection: collection_filter.as_ref().map(|collection| collection.id),
                namespace: namespace_filter,
                include_superseded,
                canonical: true,
                limit,
            },
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
                progressive_retrieve_with_namespace(
                    results.clone(),
                    budget,
                    3,
                    collection_filter.as_ref().map(|c| c.id),
                    namespace_filter,
                    include_superseded,
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
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let namespace_filter = input.namespace.as_deref().or(Some(""));
        let collection_filter =
            resolve_read_collection_filter_for_mcp(&db, input.collection.as_deref())?;
        let include_superseded = input.include_superseded.unwrap_or(false);

        let limit = input.limit.unwrap_or(50).min(MAX_LIMIT) as usize;
        let safe_query = sanitize_fts_query(&input.query);
        let results = crate::core::fts::search_fts(
            &db,
            crate::core::fts::FtsQuery {
                query: &safe_query,
                wing: input.wing.as_deref(),
                collection: collection_filter.as_ref().map(|collection| collection.id),
                namespace: namespace_filter,
                include_superseded,
                canonical: true,
                limit,
            },
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
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let namespace_filter = input.namespace.as_deref().or(Some(""));
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
        if let Some(namespace) = namespace_filter {
            if namespace.is_empty() {
                sql.push_str(" AND p.namespace = ?");
                params.push(Box::new(String::new()));
            } else {
                sql.push_str(" AND (p.namespace = ? OR p.namespace = '')");
                params.push(Box::new(namespace.to_owned()));
            }
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
            .map_err(map_serialize_error)?;
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
            .map_err(map_serialize_error)?;
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
            .map_err(map_serialize_error)?;
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
            .map_err(map_serialize_error)?;
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
            .map_err(map_serialize_error)?;
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
        .map_err(map_gaps_error)?;

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
            serialize_response(&result)?,
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

        let gap_list = gaps::list_gaps(resolved, limit, &db).map_err(map_gaps_error)?;

        let json = serde_json::to_string_pretty(&gap_list)
            .map_err(map_serialize_error)?;
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
            serialize_response(&result)?,
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
            serialize_response(&collections)?,
        )]))
    }

    #[tool(description = "Create namespace metadata")]
    pub fn memory_namespace_create(
        &self,
        #[tool(aggr)] input: MemoryNamespaceCreateInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let namespace = namespace::create_namespace(&db, &input.id, input.ttl_hours)
            .map_err(map_namespace_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&namespace)
                .map_err(map_serialize_error)?,
        )]))
    }

    #[tool(description = "Destroy a namespace and all pages assigned to it")]
    pub fn memory_namespace_destroy(
        &self,
        #[tool(aggr)] input: MemoryNamespaceDestroyInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let deleted_pages =
            namespace::destroy_namespace(&db, &input.id).map_err(map_namespace_error)?;
        let result = serde_json::json!({
            "status": "ok",
            "namespace": input.id,
            "deleted_pages": deleted_pages,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result)
                .map_err(map_serialize_error)?,
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
            .map_err(map_serialize_error)?;
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
            serialize_response(&result)?,
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

// reason: white-box; needs `extraction_enabled`, `extraction_debounce_ms`,
// `serialize_response`, `validate_slug`, `validate_relationship`,
// `validate_tag_list`, `validate_temporal_value`, `parse_temporal_filter`,
// `map_graph_error`, `memory_close_action_impl`, `canonical_slug`,
// `map_anyhow_error`, `MAX_SLUG_LEN`, `MAX_TAGS_PER_REQUEST`, `TemporalFilter`,
// `vault_sync::set_collection_recovery_in_progress_for_test`,
// and the private `QuaidServer::db` field for state-verification queries.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::core::db;
    use serde_json::json;
    use std::fs;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};

    #[test]
    fn serialize_response_returns_rmcp_error_on_unrepresentable_input() {
        // Spec note: feeding `f64::NAN` through `json!` collapses to `null`
        // (Value cannot hold NaN), so use a custom Serialize that always
        // errors. That exercises the same error path: `serialize_response`
        // must return a structured `rmcp::Error` rather than panicking.
        struct AlwaysFails;
        impl serde::Serialize for AlwaysFails {
            fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(serde::ser::Error::custom("intentional"))
            }
        }
        let result = serialize_response(&AlwaysFails);
        assert!(result.is_err());
    }

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
    fn memory_add_turn_skips_enqueue_when_extraction_is_disabled() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let result = server
            .memory_add_turn(MemoryAddTurnInput {
                session_id: "session-disabled".to_string(),
                role: "user".to_string(),
                content: "hello".to_string(),
                timestamp: Some("2026-05-03T09:14:22Z".to_string()),
                metadata: None,
                namespace: None,
            })
            .unwrap();

        let payload: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(payload["turn_id"], "session-disabled:1");
        assert!(payload["extraction_scheduled_at"].is_null());
        let db = server.db.lock().unwrap();
        let queue_count: i64 = db
            .query_row("SELECT COUNT(*) FROM extraction_queue", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(queue_count, 0);
    }

    #[test]
    fn memory_add_turn_enqueues_namespaced_session_when_extraction_is_enabled() {
        let (_dir, conn) = open_test_db();
        conn.execute(
            "INSERT OR REPLACE INTO config(key, value) VALUES ('extraction.enabled', 'true')",
            [],
        )
        .unwrap();
        let server = QuaidServer::new(conn);

        let result = server
            .memory_add_turn(MemoryAddTurnInput {
                session_id: "session-enabled".to_string(),
                role: "user".to_string(),
                content: "hello".to_string(),
                timestamp: Some("2026-05-03T09:14:22Z".to_string()),
                metadata: None,
                namespace: Some("alpha".to_string()),
            })
            .unwrap();

        let payload: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(
            payload["conversation_path"],
            "alpha/conversations/2026-05-03/session-enabled.md"
        );
        assert!(payload["extraction_scheduled_at"]
            .as_str()
            .unwrap()
            .ends_with('Z'));

        let db = server.db.lock().unwrap();
        let queue_row: (String, String, String, String) = db
            .query_row(
                "SELECT session_id, trigger_kind, conversation_path, status
                 FROM extraction_queue
                 WHERE session_id = 'alpha::session-enabled'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(queue_row.0, "alpha::session-enabled");
        assert_eq!(queue_row.1, "debounce");
        assert_eq!(
            queue_row.2,
            "alpha/conversations/2026-05-03/session-enabled.md"
        );
        assert_eq!(queue_row.3, "pending");
    }

    #[test]
    fn memory_close_session_reports_queue_position_for_first_and_repeat_close() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        server
            .memory_add_turn(MemoryAddTurnInput {
                session_id: "session-close".to_string(),
                role: "user".to_string(),
                content: "wrap up".to_string(),
                timestamp: Some("2026-05-03T09:14:22Z".to_string()),
                metadata: None,
                namespace: None,
            })
            .unwrap();

        let first = server
            .memory_close_session(MemoryCloseSessionInput {
                session_id: "session-close".to_string(),
                namespace: None,
            })
            .unwrap();
        let second = server
            .memory_close_session(MemoryCloseSessionInput {
                session_id: "session-close".to_string(),
                namespace: None,
            })
            .unwrap();

        let first_payload: serde_json::Value = serde_json::from_str(&extract_text(&first)).unwrap();
        let second_payload: serde_json::Value =
            serde_json::from_str(&extract_text(&second)).unwrap();
        assert_eq!(first_payload["extraction_triggered"], true);
        assert_eq!(first_payload["queue_position"], 1);
        assert_eq!(second_payload["extraction_triggered"], true);
        assert_eq!(second_payload["queue_position"], 1);
        assert_eq!(first_payload["closed_at"], second_payload["closed_at"]);

        let db = server.db.lock().unwrap();
        let queue_rows: (i64, String, String) = db
            .query_row(
                "SELECT COUNT(*), trigger_kind, status
                 FROM extraction_queue
                 WHERE session_id = 'session-close'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(queue_rows.0, 1);
        assert_eq!(queue_rows.1, "session_close");
        assert_eq!(queue_rows.2, "pending");
    }

    #[test]
    fn memory_close_action_updates_status_appends_note_and_bumps_version() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "actions/ship-phase5",
            concat!(
                "---\n",
                "title: Ship phase 5\n",
                "type: action_item\n",
                "status: open\n",
                "---\n",
                "# Action: Ship phase 5\n\n",
                "> Team to ship phase 5.\n",
            ),
        );

        let result = server
            .memory_close_action(MemoryCloseActionInput {
                slug: "actions/ship-phase5".to_string(),
                status: "done".to_string(),
                note: Some("Completed after final coverage pass.".to_string()),
            })
            .unwrap();

        let payload: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(payload["version"], 2);
        assert!(payload["updated_at"].as_str().unwrap().ends_with('Z'));

        let db = server.db.lock().unwrap();
        let page = get::get_page(&db, "actions/ship-phase5").unwrap();
        assert_eq!(page.version, 2);
        assert_eq!(
            page.frontmatter.get("status"),
            Some(&serde_json::json!("done"))
        );
        assert!(page
            .compiled_truth
            .contains("Completed after final coverage pass."));
    }

    #[test]
    fn memory_close_action_rejects_non_action_item_pages_with_kind_error() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "preferences/editor",
            concat!(
                "---\n",
                "title: Editor preference\n",
                "type: preference\n",
                "status: open\n",
                "---\n",
                "Prefer modal editing.\n",
            ),
        );

        let error = server
            .memory_close_action(MemoryCloseActionInput {
                slug: "preferences/editor".to_string(),
                status: "done".to_string(),
                note: Some("should not land".to_string()),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(error.message.contains("KindError"));

        let db = server.db.lock().unwrap();
        let page = get::get_page(&db, "preferences/editor").unwrap();
        assert_eq!(page.version, 1);
        assert_eq!(page.page_type, "preference");
        assert!(!page.compiled_truth.contains("should not land"));
    }

    #[test]
    fn memory_close_action_returns_conflict_error_when_page_changes_after_read() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);
        create_page(
            &server,
            "actions/race-close",
            concat!(
                "---\n",
                "title: Race close\n",
                "type: action_item\n",
                "status: open\n",
                "---\n",
                "Original action body.\n",
            ),
        );

        let error = server
            .memory_close_action_impl(
                MemoryCloseActionInput {
                    slug: "actions/race-close".to_string(),
                    status: "done".to_string(),
                    note: Some("close note".to_string()),
                },
                |db, resolved, page| {
                    let mut concurrent_page = page.clone();
                    concurrent_page.compiled_truth = "Concurrent writer landed first.".to_string();
                    crate::core::types::frontmatter_insert_string(
                        &mut concurrent_page.frontmatter,
                        "status",
                        "open",
                    );
                    let content = crate::core::markdown::render_page(&concurrent_page);
                    put::put_from_string_quiet(
                        db,
                        &canonical_slug(&resolved.collection_name, &resolved.slug),
                        &content,
                        Some(page.version),
                    )
                    .map_err(map_anyhow_error)
                },
            )
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32009));
        assert!(error.message.contains("ConflictError"));
        assert_eq!(error.data, Some(json!({ "current_version": 2 })));

        let db = server.db.lock().unwrap();
        let page = get::get_page(&db, "actions/race-close").unwrap();
        assert_eq!(page.version, 2);
        assert_eq!(
            page.frontmatter.get("status"),
            Some(&serde_json::json!("open"))
        );
        assert_eq!(page.compiled_truth, "Concurrent writer landed first.");
        assert!(!page.compiled_truth.contains("close note"));
    }

    #[test]
    fn extraction_enabled_rejects_invalid_config_value() {
        let (_dir, conn) = open_test_db();
        conn.execute(
            "INSERT OR REPLACE INTO config(key, value) VALUES ('extraction.enabled', 'maybe')",
            [],
        )
        .unwrap();

        let error = extraction_enabled(&conn).unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(error.message.contains("invalid extraction.enabled"));
    }

    #[test]
    fn extraction_debounce_ms_rejects_invalid_config_value() {
        let (_dir, conn) = open_test_db();
        conn.execute(
            "INSERT OR REPLACE INTO config(key, value) VALUES ('extraction.debounce_ms', 'later')",
            [],
        )
        .unwrap();

        let error = extraction_debounce_ms(&conn).unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(error.message.contains("invalid extraction.debounce_ms"));
    }

    #[test]
    fn memory_put_update_with_expected_version_returns_updated_status_and_persists_body() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let created = server
            .memory_put(MemoryPutInput {
                slug: "notes/occ-happy".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nInitial body\n".to_string(),
                expected_version: None,
                namespace: None,
            })
            .unwrap();
        assert!(extract_text(&created).contains("Created"));

        let updated = server
            .memory_put(MemoryPutInput {
                slug: "notes/occ-happy".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nUpdated body\n".to_string(),
                expected_version: Some(1),
                namespace: None,
            })
            .unwrap();

        let text = extract_text(&updated);
        assert!(text.contains("Updated"));
        assert!(text.contains("notes/occ-happy"));
        assert!(text.contains("(version 2)"));

        let db = server.db.lock().unwrap();
        let row: (i64, String) = db
            .query_row(
                "SELECT version, compiled_truth FROM pages WHERE slug = ?1",
                ["notes/occ-happy"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, 2);
        assert_eq!(row.1, "Updated body");
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
                namespace: None,
            })
            .unwrap();

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/existing".to_string(),
                content: "---\ntitle: Existing\ntype: note\n---\nUnexpected overwrite\n"
                    .to_string(),
                expected_version: None,
                namespace: None,
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
                namespace: None,
            })
            .unwrap();

        let error = server
            .memory_put(MemoryPutInput {
                slug: "notes/stale".to_string(),
                content: "---\ntitle: Stale\ntype: note\n---\nStale overwrite\n".to_string(),
                expected_version: Some(0),
                namespace: None,
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
                namespace: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32002));
        assert!(error.message.contains("CollectionRestoringError"));
    }

    #[test]
    fn memory_put_write_stampede_keeps_fts_fresh_and_drains_embedding_queue() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        server
            .memory_put(MemoryPutInput {
                slug: "notes/stampede".to_string(),
                content: "---\ntitle: Stampede\ntype: note\n---\nOriginal alpha body\n".to_string(),
                expected_version: None,
                namespace: None,
            })
            .unwrap();
        server
            .memory_put(MemoryPutInput {
                slug: "notes/stampede".to_string(),
                content: "---\ntitle: Stampede\ntype: note\n---\nUpdated omega body\n".to_string(),
                expected_version: Some(1),
                namespace: None,
            })
            .unwrap();

        let db = server.db.lock().unwrap();
        let page_id: i64 = db
            .query_row(
                "SELECT id FROM pages WHERE slug = 'notes/stampede'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let queued_jobs: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM embedding_jobs WHERE page_id = ?1 AND job_state = 'pending'",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(queued_jobs, 1);

        let fts_results = crate::core::fts::search_fts(
            &db,
            crate::core::fts::FtsQuery {
                query: "omega",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(fts_results
            .iter()
            .any(|result| result.slug == "notes/stampede"));

        vault_sync::drain_embedding_queue(&db).unwrap();

        let remaining_jobs: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM embedding_jobs WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_jobs, 0);

        let vec_results = crate::core::inference::search_vec("omega", 10, None, None, &db).unwrap();
        assert!(vec_results
            .iter()
            .any(|result| result.slug == "notes/stampede"));
    }

    #[test]
    fn memory_query_logs_gap_for_weak_results() {
        let (_dir, conn) = open_test_db();
        let server = QuaidServer::new(conn);

        let result = server
            .memory_query(MemoryQueryInput {
                query: "who runs the moon colony".to_string(),
                collection: None,
                namespace: None,
                wing: None,
                limit: None,
                depth: None,
                include_superseded: None,
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

    // 13.5 contract: depth="auto" must NOT expand across collection boundaries

    // D.6 — memory_search with natural-language '?' query returns valid JSON-RPC response

    // ── Phase 2 MCP tests ────────────────────────────────────

    fn create_page(server: &QuaidServer, slug: &str, content: &str) {
        server
            .memory_put(MemoryPutInput {
                slug: slug.to_string(),
                content: content.to_string(),
                expected_version: None,
                namespace: None,
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
                namespace: None,
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

    // ── memory_link_close ─────────────────────────────────────

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

    // ── memory_backlinks ──────────────────────────────────────

    // ── memory_graph ──────────────────────────────────────────

    // ── memory_check ──────────────────────────────────────────

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

    // ── memory_timeline ───────────────────────────────────────

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

    // ── memory_tags ───────────────────────────────────────────

    // ── Phase 3 MCP tests ────────────────────────────────────

    // ── memory_gap ────────────────────────────────────────────

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

    // ── memory_gaps ───────────────────────────────────────────

    // ── memory_stats ──────────────────────────────────────────

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
        create_page(
            &server,
            "notes/default-failed",
            "---\ntitle: Default Failed\ntype: note\n---\nFailed\n",
        );

        let db = server.db.lock().unwrap();
        let failed_page_id: i64 = db
            .query_row(
                "SELECT id FROM pages WHERE slug = 'notes/default-failed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        db.execute(
            "UPDATE embedding_jobs
             SET job_state = 'failed',
                 attempt_count = 5,
                 last_error = 'stuck forever',
                 started_at = NULL
             WHERE page_id = ?1",
            [failed_page_id],
        )
        .unwrap();
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
        assert_eq!(default["page_count"], 2);
        assert!(default["last_sync_at"].is_null());
        assert!(default["embedding_queue_depth"].as_i64().is_some());
        assert!(default.get("failing_jobs").is_none());
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
                namespace: None,
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
                namespace: None,
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
                namespace: None,
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
                namespace: None,
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
                namespace: None,
            })
            .unwrap();
        server
            .memory_put(MemoryPutInput {
                slug: "notes/happy".to_string(),
                content: updated.to_string(),
                expected_version: Some(1),
                namespace: None,
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
