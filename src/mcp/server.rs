use std::sync::{Arc, Mutex};

use rmcp::model::*;
use rmcp::schemars;
use rmcp::tool;
use rmcp::{ServerHandler, ServiceExt};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::commands::get::get_page;
use crate::commands::{check, link};

use crate::core::fts::search_fts;
use crate::core::gaps;
use crate::core::graph::{self, GraphError, TemporalFilter};
use crate::core::markdown;
use crate::core::palace;
use crate::core::progressive::progressive_retrieve;
use crate::core::search::hybrid_search;
use crate::core::types::SearchError;

type DbRef = Arc<Mutex<Connection>>;

const MAX_SLUG_LEN: usize = 512;
const MAX_CONTENT_LEN: usize = 1_048_576; // 1 MB
const MAX_LIMIT: u32 = 1000;
const MAX_RELATIONSHIP_LEN: usize = 64;
const MAX_TAG_LEN: usize = 64;
const MAX_TAGS_PER_REQUEST: usize = 100;

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
    if !slug.bytes().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'/' || b == b'_' || b == b'-'
    }) {
        return Err(invalid_params(
            "invalid slug: allowed characters are [a-z0-9/_-]",
        ));
    }
    Ok(())
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
        SearchError::Internal { message } => {
            rmcp::Error::new(ErrorCode(-32003), format!("search error: {message}"), None)
        }
    }
}

fn map_anyhow_error(e: anyhow::Error) -> rmcp::Error {
    let msg = e.to_string();
    if msg.contains("page not found") || msg.contains("link not found") {
        rmcp::Error::new(ErrorCode(-32001), msg, None)
    } else {
        rmcp::Error::new(ErrorCode(-32003), msg, None)
    }
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
pub struct GigaBrainServer {
    db: DbRef,
}

impl GigaBrainServer {
    pub fn new(conn: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(conn)),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainGetInput {
    /// Page slug to retrieve
    pub slug: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainPutInput {
    /// Page slug to create or update
    pub slug: String,
    /// Markdown content of the page
    pub content: String,
    /// Expected current version for optimistic concurrency control
    pub expected_version: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainQueryInput {
    /// Search query string
    pub query: String,
    /// Optional wing filter
    pub wing: Option<String>,
    /// Maximum results to return
    pub limit: Option<u32>,
    /// Retrieval depth: "auto" for progressive expansion, absent/empty for direct results only
    pub depth: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainSearchInput {
    /// FTS5 search query string
    pub query: String,
    /// Optional wing filter
    pub wing: Option<String>,
    /// Maximum results to return
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainListInput {
    /// Optional wing filter
    pub wing: Option<String>,
    /// Optional type filter
    pub page_type: Option<String>,
    /// Maximum results to return
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainLinkInput {
    pub from_slug: String,
    pub to_slug: String,
    pub relationship: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainLinkCloseInput {
    pub link_id: u64,
    pub valid_until: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainBacklinksInput {
    pub slug: String,
    pub limit: Option<u32>,
    pub temporal: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainGraphInput {
    pub slug: String,
    pub depth: Option<u32>,
    pub temporal: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainCheckInput {
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainTimelineInput {
    pub slug: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainTagsInput {
    pub slug: String,
    pub add: Option<Vec<String>>,
    pub remove: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainGapInput {
    /// Query string to log as a knowledge gap
    pub query: String,
    /// Optional context about the gap
    pub context: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainGapsInput {
    /// Include resolved gaps (default: false)
    pub resolved: Option<bool>,
    /// Maximum number of gaps to return (default: 20, max: 1000)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainStatsInput {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainRawInput {
    /// Page slug to attach raw data to
    pub slug: String,
    /// Source identifier (e.g. "crustdata", "exa", "meeting")
    pub source: String,
    /// Arbitrary JSON data to store
    pub data: serde_json::Value,
}

#[tool(tool_box)]
impl GigaBrainServer {
    #[tool(description = "Get a page by slug")]
    pub fn brain_get(
        &self,
        #[tool(aggr)] input: BrainGetInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        match get_page(&db, &input.slug) {
            Ok(page) => {
                let rendered = markdown::render_page(&page);
                Ok(CallToolResult::success(vec![Content::text(rendered)]))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("page not found") {
                    Err(rmcp::Error::new(rmcp::model::ErrorCode(-32001), msg, None))
                } else {
                    Err(rmcp::Error::new(rmcp::model::ErrorCode(-32003), msg, None))
                }
            }
        }
    }

    #[tool(description = "Write or update a page")]
    pub fn brain_put(
        &self,
        #[tool(aggr)] input: BrainPutInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        validate_content(&input.content)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let (frontmatter, body) = markdown::parse_frontmatter(&input.content);
        let (compiled_truth, timeline) = markdown::split_content(&body);
        let summary = markdown::extract_summary(&compiled_truth);
        let wing = palace::derive_wing(&input.slug);
        let room = palace::derive_room(&compiled_truth);
        let title = frontmatter
            .get("title")
            .cloned()
            .unwrap_or_else(|| input.slug.clone());
        let page_type = frontmatter
            .get("type")
            .cloned()
            .unwrap_or_else(|| "concept".to_string());
        let frontmatter_json = serde_json::to_string(&frontmatter).map_err(|e| {
            rmcp::Error::new(
                rmcp::model::ErrorCode(-32002),
                format!("parse error: {e}"),
                None,
            )
        })?;

        let now: String = db
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
                row.get(0)
            })
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

        let existing_version: Option<i64> = match db
            .prepare("SELECT version FROM pages WHERE slug = ?1")
            .map_err(map_db_error)?
            .query_row([&input.slug], |row| row.get(0))
        {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(map_db_error(e)),
        };

        match existing_version {
            None => {
                // OCC: a client supplying expected_version on a non-existent page has stale
                // state — the page never existed at that version. Reject as a conflict.
                if let Some(n) = input.expected_version {
                    return Err(rmcp::Error::new(
                        rmcp::model::ErrorCode(-32009),
                        format!("conflict: page does not exist at version {n}"),
                        Some(serde_json::json!({ "current_version": null })),
                    ));
                }
                db.execute(
                    "INSERT INTO pages \
                         (slug, type, title, summary, compiled_truth, timeline, \
                          frontmatter, wing, room, version, \
                          created_at, updated_at, truth_updated_at, timeline_updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10, ?10, ?10)",
                    rusqlite::params![
                        input.slug,
                        page_type,
                        title,
                        summary,
                        compiled_truth,
                        timeline,
                        frontmatter_json,
                        wing,
                        room,
                        now,
                    ],
                )
                .map_err(map_db_error)?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Created {} (version 1)",
                    input.slug
                ))]))
            }
            Some(current) => {
                let expected = input.expected_version.ok_or_else(|| {
                    rmcp::Error::new(
                        rmcp::model::ErrorCode(-32009),
                        format!(
                            "conflict: page already exists (current version: {current}). \
                             Provide expected_version to update."
                        ),
                        Some(serde_json::json!({ "current_version": current })),
                    )
                })?;

                let rows = db
                    .execute(
                        "UPDATE pages SET \
                             type = ?1, title = ?2, summary = ?3, \
                             compiled_truth = ?4, timeline = ?5, \
                             frontmatter = ?6, wing = ?7, room = ?8, \
                             version = version + 1, \
                             updated_at = ?9, truth_updated_at = ?9, timeline_updated_at = ?9 \
                         WHERE slug = ?10 AND version = ?11",
                        rusqlite::params![
                            page_type,
                            title,
                            summary,
                            compiled_truth,
                            timeline,
                            frontmatter_json,
                            wing,
                            room,
                            now,
                            input.slug,
                            expected,
                        ],
                    )
                    .map_err(map_db_error)?;

                if rows == 0 {
                    return Err(rmcp::Error::new(
                        rmcp::model::ErrorCode(-32009),
                        format!("conflict: page updated elsewhere (current version: {current})"),
                        Some(serde_json::json!({ "current_version": current })),
                    ));
                }

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Updated {} (version {})",
                    input.slug,
                    expected + 1
                ))]))
            }
        }
    }

    #[tool(description = "Hybrid semantic + FTS5 query")]
    pub fn brain_query(
        &self,
        #[tool(aggr)] input: BrainQueryInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let limit = input.limit.unwrap_or(10).min(MAX_LIMIT) as usize;
        let results = hybrid_search(&input.query, input.wing.as_deref(), &db, limit)
            .map_err(map_search_error)?;

        // Auto-log knowledge gap on weak results
        if results.len() < 2 || results.iter().all(|r| r.score < 0.3) {
            let _ = gaps::log_gap(&input.query, "", results.first().map(|r| r.score), &db);
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
                progressive_retrieve(results.clone(), budget, 3, &db).unwrap_or(results)
            }
            _ => results,
        };

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "FTS5 full-text search")]
    pub fn brain_search(
        &self,
        #[tool(aggr)] input: BrainSearchInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let limit = input.limit.unwrap_or(50).min(MAX_LIMIT) as usize;
        let results = search_fts(&input.query, input.wing.as_deref(), &db, limit)
            .map_err(map_search_error)?;

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List pages with optional filters")]
    pub fn brain_list(
        &self,
        #[tool(aggr)] input: BrainListInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let limit = input.limit.unwrap_or(50).min(MAX_LIMIT);
        let mut sql = String::from("SELECT slug, type, summary FROM pages WHERE 1=1");
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref w) = input.wing {
            sql.push_str(" AND wing = ?");
            params.push(Box::new(w.clone()));
        }
        if let Some(ref t) = input.page_type {
            sql.push_str(" AND type = ?");
            params.push(Box::new(t.clone()));
        }
        sql.push_str(" ORDER BY updated_at DESC LIMIT ?");
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
    pub fn brain_link(
        &self,
        #[tool(aggr)] input: BrainLinkInput,
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

        link::run_silent(
            &db,
            &input.from_slug,
            &input.to_slug,
            &input.relationship,
            input.valid_from,
            input.valid_until,
        )
        .map_err(map_anyhow_error)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked {} → {} ({})",
            input.from_slug, input.to_slug, input.relationship
        ))]))
    }

    #[tool(description = "Close a temporal link by its database ID")]
    pub fn brain_link_close(
        &self,
        #[tool(aggr)] input: BrainLinkCloseInput,
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
    pub fn brain_backlinks(
        &self,
        #[tool(aggr)] input: BrainBacklinksInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let filter = parse_temporal_filter(input.temporal.as_deref())?;
        let limit = input.limit.unwrap_or(100).min(MAX_LIMIT);
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let to_id: i64 = db
            .query_row(
                "SELECT id FROM pages WHERE slug = ?1",
                [&input.slug],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => rmcp::Error::new(
                    ErrorCode(-32001),
                    format!("page not found: {}", input.slug),
                    None,
                ),
                other => map_db_error(other),
            })?;

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
            "SELECT l.id, p.slug, l.relationship, l.valid_from, l.valid_until \
             FROM links l JOIN pages p ON l.from_page_id = p.id \
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
    pub fn brain_graph(
        &self,
        #[tool(aggr)] input: BrainGraphInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let depth = input.depth.unwrap_or(1).min(graph::MAX_DEPTH);
        let filter = parse_temporal_filter(input.temporal.as_deref())?;

        let result =
            graph::neighborhood_graph(&input.slug, depth, filter, &db).map_err(map_graph_error)?;

        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Run contradiction detection on a page or all pages")]
    pub fn brain_check(
        &self,
        #[tool(aggr)] input: BrainCheckInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        if let Some(slug) = input.slug.as_deref() {
            validate_slug(slug)?;
        }
        let slug_filter = input.slug.clone();
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let all = slug_filter.is_none();
        check::execute_check(&db, input.slug.as_deref(), all, None).map_err(map_anyhow_error)?;

        // Fetch unresolved contradictions as JSON
        use crate::core::assertions::Contradiction;
        let contradictions: Vec<Contradiction> = if let Some(slug) = slug_filter.as_deref() {
            let mut stmt = db
                .prepare(
                    "SELECT p.slug, COALESCE(other.slug, p.slug), c.type, c.description, c.detected_at \
                     FROM contradictions c \
                     JOIN pages p ON p.id = c.page_id \
                     LEFT JOIN pages other ON other.id = c.other_page_id \
                     WHERE c.resolved_at IS NULL AND (p.slug = ?1 OR other.slug = ?1) \
                     ORDER BY c.detected_at, p.slug",
                )
                .map_err(map_db_error)?;

            let rows = stmt
                .query_map([slug], |row| {
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
                    "SELECT p.slug, COALESCE(other.slug, p.slug), c.type, c.description, c.detected_at \
                     FROM contradictions c \
                     JOIN pages p ON p.id = c.page_id \
                     LEFT JOIN pages other ON other.id = c.other_page_id \
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
    pub fn brain_timeline(
        &self,
        #[tool(aggr)] input: BrainTimelineInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let limit = input.limit.unwrap_or(20).min(MAX_LIMIT);

        // Verify page exists
        let page = get_page(&db, &input.slug).map_err(map_anyhow_error)?;

        let page_id: i64 = db
            .query_row(
                "SELECT id FROM pages WHERE slug = ?1",
                [&input.slug],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => rmcp::Error::new(
                    ErrorCode(-32001),
                    format!("page not found: {}", input.slug),
                    None,
                ),
                other => map_db_error(other),
            })?;

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
            slug: input.slug,
            entries,
        };

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List, add, or remove tags on a page")]
    pub fn brain_tags(
        &self,
        #[tool(aggr)] input: BrainTagsInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let page_id: i64 = db
            .query_row(
                "SELECT id FROM pages WHERE slug = ?1",
                [&input.slug],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => rmcp::Error::new(
                    ErrorCode(-32001),
                    format!("page not found: {}", input.slug),
                    None,
                ),
                other => map_db_error(other),
            })?;

        let add = input.add.unwrap_or_default();
        let remove = input.remove.unwrap_or_default();
        validate_tag_list(&add, "add")?;
        validate_tag_list(&remove, "remove")?;

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
    pub fn brain_gap(
        &self,
        #[tool(aggr)] input: BrainGapInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        if input.query.trim().is_empty() {
            return Err(invalid_params("query must not be empty"));
        }
        let context = input.context.unwrap_or_default();
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let query_hash = {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(input.query.as_bytes());
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };

        gaps::log_gap(&input.query, &context, None, &db).map_err(|e| {
            rmcp::Error::new(ErrorCode(-32003), format!("database error: {e}"), None)
        })?;

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
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap(),
        )]))
    }

    #[tool(description = "List knowledge gaps")]
    pub fn brain_gaps(
        &self,
        #[tool(aggr)] input: BrainGapsInput,
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
    pub fn brain_stats(
        &self,
        #[tool(aggr)] _input: BrainStatsInput,
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

    #[tool(description = "Store raw structured data (API responses, JSON) for a page")]
    pub fn brain_raw(
        &self,
        #[tool(aggr)] input: BrainRawInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        if input.source.is_empty() {
            return Err(invalid_params("source must not be empty"));
        }
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let page_id: i64 = db
            .query_row(
                "SELECT id FROM pages WHERE slug = ?1",
                [&input.slug],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => rmcp::Error::new(
                    ErrorCode(-32001),
                    format!("page not found: {}", input.slug),
                    None,
                ),
                other => map_db_error(other),
            })?;

        let data_json = serde_json::to_string(&input.data)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;

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
impl ServerHandler for GigaBrainServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("GigaBrain personal knowledge brain".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

/// Run the MCP stdio server with the given database connection.
pub async fn run(conn: Connection) -> anyhow::Result<()> {
    let server = GigaBrainServer::new(conn);
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

    fn open_test_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("server.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        (dir, conn)
    }

    #[test]
    fn get_info_enables_tools_capability_and_exposes_core_tool_methods() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        let info = <GigaBrainServer as ServerHandler>::get_info(&server);

        let _tool_methods = (
            GigaBrainServer::brain_get
                as fn(&GigaBrainServer, BrainGetInput) -> Result<CallToolResult, rmcp::Error>,
            GigaBrainServer::brain_put
                as fn(&GigaBrainServer, BrainPutInput) -> Result<CallToolResult, rmcp::Error>,
            GigaBrainServer::brain_query
                as fn(&GigaBrainServer, BrainQueryInput) -> Result<CallToolResult, rmcp::Error>,
            GigaBrainServer::brain_search
                as fn(&GigaBrainServer, BrainSearchInput) -> Result<CallToolResult, rmcp::Error>,
            GigaBrainServer::brain_list
                as fn(&GigaBrainServer, BrainListInput) -> Result<CallToolResult, rmcp::Error>,
        );

        assert!(info.capabilities.tools.is_some());
    }

    #[test]
    fn brain_get_returns_not_found_error_code_for_missing_slug() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_get(BrainGetInput {
                slug: "definitely-does-not-exist".to_string(),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn brain_put_returns_occ_conflict_error_with_current_version_for_stale_write() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        server
            .brain_put(BrainPutInput {
                slug: "notes/test".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nInitial content\n".to_string(),
                expected_version: None,
            })
            .unwrap();

        let error = server
            .brain_put(BrainPutInput {
                slug: "notes/test".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nUpdated content\n".to_string(),
                expected_version: Some(0),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32009));
        assert_eq!(error.data, Some(json!({ "current_version": 1 })));
    }

    #[test]
    fn brain_put_rejects_update_without_expected_version() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        server
            .brain_put(BrainPutInput {
                slug: "notes/occ".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nInitial\n".to_string(),
                expected_version: None,
            })
            .unwrap();

        let error = server
            .brain_put(BrainPutInput {
                slug: "notes/occ".to_string(),
                content: "---\ntitle: Test\ntype: note\n---\nSneaky overwrite\n".to_string(),
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32009));
        assert_eq!(error.data, Some(json!({ "current_version": 1 })));
    }

    #[test]
    fn brain_get_rejects_invalid_slug() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_get(BrainGetInput {
                slug: "Invalid/SLUG!".to_string(),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn brain_put_rejects_oversized_content() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let large_content = "x".repeat(1_048_577);
        let error = server
            .brain_put(BrainPutInput {
                slug: "test/large".to_string(),
                content: large_content,
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn brain_put_rejects_empty_slug() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_put(BrainPutInput {
                slug: "".to_string(),
                content: "content".to_string(),
                expected_version: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn brain_put_rejects_create_with_expected_version_when_page_does_not_exist() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        // Page does not exist; supplying expected_version is a client bug — reject as OCC conflict.
        let error = server
            .brain_put(BrainPutInput {
                slug: "notes/ghost".to_string(),
                content: "---\ntitle: Ghost\ntype: note\n---\nContent\n".to_string(),
                expected_version: Some(3),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32009));
        assert_eq!(error.data, Some(json!({ "current_version": null })));
    }

    #[test]
    fn brain_query_logs_gap_for_weak_results() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let result = server
            .brain_query(BrainQueryInput {
                query: "who runs the moon colony".to_string(),
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
    fn brain_query_auto_depth_expands_linked_results() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_link(BrainLinkInput {
                from_slug: "concepts/root".to_string(),
                to_slug: "concepts/child".to_string(),
                relationship: "related".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap();

        let result = server
            .brain_query(BrainQueryInput {
                query: "alpha".to_string(),
                wing: None,
                limit: Some(1),
                depth: Some("auto".to_string()),
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(rows.iter().any(|row| row["slug"] == "concepts/child"));
    }

    #[test]
    fn brain_search_returns_matching_pages() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme builds fundraising software.\n",
        );

        let result = server
            .brain_search(BrainSearchInput {
                query: "fundraising".to_string(),
                wing: None,
                limit: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows[0]["slug"], "companies/acme");
    }

    #[test]
    fn brain_list_applies_wing_and_type_filters() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_list(BrainListInput {
                wing: Some("people".to_string()),
                page_type: Some("person".to_string()),
                limit: None,
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
    }

    // ── Phase 2 MCP tests ────────────────────────────────────

    fn create_page(server: &GigaBrainServer, slug: &str, content: &str) {
        server
            .brain_put(BrainPutInput {
                slug: slug.to_string(),
                content: content.to_string(),
                expected_version: None,
            })
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

    // ── brain_link ───────────────────────────────────────────

    #[test]
    fn brain_link_with_unknown_from_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "companies/acme",
            "---\ntitle: Acme\ntype: company\n---\nAcme Corp\n",
        );

        let error = server
            .brain_link(BrainLinkInput {
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
    fn brain_link_creates_link_between_existing_pages() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_link(BrainLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: Some("2024-01".to_string()),
                valid_until: None,
            })
            .unwrap();

        let text = extract_text(&result);
        assert!(text.contains("Linked"));
        assert!(text.contains("people/alice"));
    }

    #[test]
    fn brain_link_rejects_invalid_relationship() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_link(BrainLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    // ── brain_link_close ─────────────────────────────────────

    #[test]
    fn brain_link_close_with_unknown_id_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_link_close(BrainLinkCloseInput {
                link_id: 99999,
                valid_until: "2025-06".to_string(),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn brain_link_close_sets_valid_until_on_existing_link() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_link(BrainLinkInput {
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
            .brain_link_close(BrainLinkCloseInput {
                link_id: link_id as u64,
                valid_until: "2025-06".to_string(),
            })
            .unwrap();

        let text = extract_text(&result);
        assert!(text.contains(&format!("Closed link {link_id}")));
    }

    #[test]
    fn brain_link_close_rejects_invalid_temporal_value() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_link_close(BrainLinkCloseInput {
                link_id: 1,
                valid_until: "not-a-date".to_string(),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    // ── brain_backlinks ──────────────────────────────────────

    #[test]
    fn brain_backlinks_returns_link_array() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_link(BrainLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap();

        let result = server
            .brain_backlinks(BrainBacklinksInput {
                slug: "companies/acme".to_string(),
                limit: None,
                temporal: None,
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["from_slug"], "people/alice");
        assert_eq!(arr[0]["relationship"], "works_at");
    }

    #[test]
    fn brain_backlinks_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_backlinks(BrainBacklinksInput {
                slug: "nobody/ghost".to_string(),
                limit: None,
                temporal: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn brain_backlinks_applies_limit() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
                .brain_link(BrainLinkInput {
                    from_slug: slug.to_string(),
                    to_slug: "companies/acme".to_string(),
                    relationship: "works_at".to_string(),
                    valid_from: None,
                    valid_until: None,
                })
                .unwrap();
        }

        let result = server
            .brain_backlinks(BrainBacklinksInput {
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
    fn brain_backlinks_temporal_all_includes_closed_links() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_link(BrainLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: Some("2020-01-01".to_string()),
                valid_until: Some("2020-12-31".to_string()),
            })
            .unwrap();

        let result = server
            .brain_backlinks(BrainBacklinksInput {
                slug: "companies/acme".to_string(),
                limit: None,
                temporal: Some("all".to_string()),
            })
            .unwrap();

        let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn brain_backlinks_rejects_invalid_temporal_filter() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_backlinks(BrainBacklinksInput {
                slug: "people/alice".to_string(),
                limit: None,
                temporal: Some("future".to_string()),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    // ── brain_graph ──────────────────────────────────────────

    #[test]
    fn brain_graph_returns_nodes_and_edges_json() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_link(BrainLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: None,
                valid_until: None,
            })
            .unwrap();

        let result = server
            .brain_graph(BrainGraphInput {
                slug: "people/alice".to_string(),
                depth: Some(2),
                temporal: None,
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(parsed["nodes"].as_array().unwrap().len() >= 2);
        assert!(!parsed["edges"].as_array().unwrap().is_empty());
    }

    #[test]
    fn brain_graph_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_graph(BrainGraphInput {
                slug: "people/ghost".to_string(),
                depth: None,
                temporal: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn brain_graph_temporal_all_includes_closed_links() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_link(BrainLinkInput {
                from_slug: "people/alice".to_string(),
                to_slug: "companies/acme".to_string(),
                relationship: "works_at".to_string(),
                valid_from: Some("2020-01-01".to_string()),
                valid_until: Some("2020-12-31".to_string()),
            })
            .unwrap();

        let result = server
            .brain_graph(BrainGraphInput {
                slug: "people/alice".to_string(),
                depth: None,
                temporal: Some("all".to_string()),
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed["edges"].as_array().unwrap().len(), 1);
    }

    // ── brain_check ──────────────────────────────────────────

    #[test]
    fn brain_check_on_clean_page_returns_empty_array() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice is a person.\n",
        );

        let result = server
            .brain_check(BrainCheckInput {
                slug: Some("people/alice".to_string()),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 0);
    }

    #[test]
    fn brain_check_detects_contradiction_on_page() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice works at Acme. Alice works at Beta.\n",
        );

        let result = server
            .brain_check(BrainCheckInput {
                slug: Some("people/alice".to_string()),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(!parsed.as_array().unwrap().is_empty());
    }

    #[test]
    fn brain_check_filters_output_to_requested_slug() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice works at Acme. Alice works at Beta.\n",
        );
        create_page(
            &server,
            "people/bob",
            "---\ntitle: Bob\ntype: person\n---\nBob works at Gamma. Bob works at Delta.\n",
        );

        server
            .brain_check(BrainCheckInput {
                slug: Some("people/bob".to_string()),
            })
            .unwrap();

        let result = server
            .brain_check(BrainCheckInput {
                slug: Some("people/alice".to_string()),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        assert!(!parsed.is_empty());
        assert!(parsed.iter().all(|row| {
            row["page_slug"] == "people/alice" || row["other_page_slug"] == "people/alice"
        }));
    }

    #[test]
    fn brain_check_without_slug_returns_all_unresolved_contradictions() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice works at Acme. Alice works at Beta.\n",
        );
        create_page(
            &server,
            "people/bob",
            "---\ntitle: Bob\ntype: person\n---\nBob works at Gamma. Bob works at Delta.\n",
        );

        let result = server.brain_check(BrainCheckInput { slug: None }).unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    // ── brain_timeline ───────────────────────────────────────

    #[test]
    fn brain_timeline_on_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_timeline(BrainTimelineInput {
                slug: "nobody/ghost".to_string(),
                limit: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn brain_timeline_returns_entries_for_page_with_timeline() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice bio\n\n## Timeline\n\n2024-01: Joined Acme\n---\n2024-06: Promoted\n",
        );

        let result = server
            .brain_timeline(BrainTimelineInput {
                slug: "people/alice".to_string(),
                limit: Some(10),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["slug"], "people/alice");
    }

    #[test]
    fn brain_timeline_prefers_structured_entries_and_applies_limit() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
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
            .brain_timeline(BrainTimelineInput {
                slug: "people/alice".to_string(),
                limit: Some(1),
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn brain_timeline_returns_empty_entries_for_page_without_timeline_data() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice bio\n",
        );

        let result = server
            .brain_timeline(BrainTimelineInput {
                slug: "people/alice".to_string(),
                limit: None,
            })
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
        assert!(parsed["entries"].as_array().unwrap().is_empty());
    }

    // ── brain_tags ───────────────────────────────────────────

    #[test]
    fn brain_tags_list_add_remove_round_trip() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        // List tags — should be empty
        let result = server
            .brain_tags(BrainTagsInput {
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
            .brain_tags(BrainTagsInput {
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
            .brain_tags(BrainTagsInput {
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
    fn brain_tags_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_tags(BrainTagsInput {
                slug: "nobody/ghost".to_string(),
                add: Some(vec!["tag".to_string()]),
                remove: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn brain_tags_rejects_invalid_tag_values() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        let error = server
            .brain_tags(BrainTagsInput {
                slug: "people/alice".to_string(),
                add: Some(vec!["bad tag".to_string()]),
                remove: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    // ── Phase 3 MCP tests ────────────────────────────────────

    // ── brain_gap ────────────────────────────────────────────

    #[test]
    fn brain_gap_with_empty_query_returns_invalid_params() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_gap(BrainGapInput {
                query: "".to_string(),
                context: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn brain_gap_stores_gap_with_null_query_text_and_internal_sensitivity() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let result = server
            .brain_gap(BrainGapInput {
                query: "who invented quantum socks".to_string(),
                context: Some("test context".to_string()),
            })
            .unwrap();

        let text = extract_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(parsed["id"].as_i64().is_some());
        assert!(parsed["query_hash"].as_str().is_some());

        // Verify stored with NULL query_text and internal sensitivity
        let db = server.db.lock().unwrap();
        let (query_text, sensitivity): (Option<String>, String) = db
            .query_row(
                "SELECT query_text, sensitivity FROM knowledge_gaps WHERE id = ?1",
                [parsed["id"].as_i64().unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(query_text.is_none());
        assert_eq!(sensitivity, "internal");
    }

    #[test]
    fn brain_gap_duplicate_is_idempotent() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let r1 = server
            .brain_gap(BrainGapInput {
                query: "same query".to_string(),
                context: None,
            })
            .unwrap();
        let r2 = server
            .brain_gap(BrainGapInput {
                query: "same query".to_string(),
                context: None,
            })
            .unwrap();

        let id1: serde_json::Value = serde_json::from_str(&extract_text(&r1)).unwrap();
        let id2: serde_json::Value = serde_json::from_str(&extract_text(&r2)).unwrap();
        assert_eq!(id1["id"], id2["id"]);
    }

    // ── brain_gaps ───────────────────────────────────────────

    #[test]
    fn brain_gaps_returns_array_with_limit() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        for i in 0..5 {
            server
                .brain_gap(BrainGapInput {
                    query: format!("gap query {i}"),
                    context: None,
                })
                .unwrap();
        }

        let result = server
            .brain_gaps(BrainGapsInput {
                resolved: None,
                limit: Some(3),
            })
            .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn brain_gaps_defaults_to_unresolved() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        server
            .brain_gap(BrainGapInput {
                query: "unresolved gap".to_string(),
                context: None,
            })
            .unwrap();

        let result = server
            .brain_gaps(BrainGapsInput {
                resolved: None,
                limit: None,
            })
            .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0]["resolved_at"].is_null());
    }

    // ── brain_stats ──────────────────────────────────────────

    #[test]
    fn brain_stats_returns_all_expected_fields() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        let result = server.brain_stats(BrainStatsInput {}).unwrap();

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

    // ── brain_raw ────────────────────────────────────────────

    #[test]
    fn brain_raw_with_unknown_slug_returns_not_found() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_raw(BrainRawInput {
                slug: "nobody/ghost".to_string(),
                source: "test".to_string(),
                data: json!({"key": "value"}),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
    }

    #[test]
    fn brain_raw_with_valid_slug_stores_row() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        let result = server
            .brain_raw(BrainRawInput {
                slug: "people/alice".to_string(),
                source: "crustdata".to_string(),
                data: json!({"funding": "$10M", "headcount": 50}),
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
    fn brain_raw_rejects_empty_source() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);
        create_page(
            &server,
            "people/alice",
            "---\ntitle: Alice\ntype: person\n---\nAlice\n",
        );

        let error = server
            .brain_raw(BrainRawInput {
                slug: "people/alice".to_string(),
                source: "".to_string(),
                data: json!({}),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
    }

    #[test]
    fn brain_raw_rejects_invalid_slug() {
        let (_dir, conn) = open_test_db();
        let server = GigaBrainServer::new(conn);

        let error = server
            .brain_raw(BrainRawInput {
                slug: "Invalid/SLUG!".to_string(),
                source: "test".to_string(),
                data: json!({}),
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32602));
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
