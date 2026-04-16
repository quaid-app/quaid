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

fn validate_slug(slug: &str) -> Result<(), rmcp::Error> {
    if slug.is_empty() {
        return Err(rmcp::Error::new(
            ErrorCode(-32602),
            "invalid slug: must not be empty".to_string(),
            None,
        ));
    }
    if slug.len() > MAX_SLUG_LEN {
        return Err(rmcp::Error::new(
            ErrorCode(-32602),
            format!("invalid slug: exceeds maximum length of {MAX_SLUG_LEN} characters"),
            None,
        ));
    }
    if !slug.bytes().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'/' || b == b'_' || b == b'-'
    }) {
        return Err(rmcp::Error::new(
            ErrorCode(-32602),
            "invalid slug: allowed characters are [a-z0-9/_-]".to_string(),
            None,
        ));
    }
    Ok(())
}

fn validate_content(content: &str) -> Result<(), rmcp::Error> {
    if content.len() > MAX_CONTENT_LEN {
        return Err(rmcp::Error::new(
            ErrorCode(-32602),
            format!(
                "content too large: {} bytes exceeds maximum of {MAX_CONTENT_LEN} bytes",
                content.len()
            ),
            None,
        ));
    }
    Ok(())
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
        "active" => Ok(TemporalFilter::Active),
        "all" => Ok(TemporalFilter::All),
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

#[tool(tool_box)]
impl GigaBrainServer {
    #[tool(description = "Get a page by slug")]
    fn brain_get(&self, #[tool(aggr)] input: BrainGetInput) -> Result<CallToolResult, rmcp::Error> {
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
    fn brain_put(&self, #[tool(aggr)] input: BrainPutInput) -> Result<CallToolResult, rmcp::Error> {
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
    fn brain_query(
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

        let results = match input.depth.as_deref() {
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
                progressive_retrieve(results, budget, 3, &db).map_err(map_search_error)?
            }
            _ => results,
        };

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "FTS5 full-text search")]
    fn brain_search(
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
    fn brain_list(
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
    fn brain_link(
        &self,
        #[tool(aggr)] input: BrainLinkInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.from_slug)?;
        validate_slug(&input.to_slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        link::run(
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
    fn brain_link_close(
        &self,
        #[tool(aggr)] input: BrainLinkCloseInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        link::close(&db, input.link_id, &input.valid_until).map_err(map_anyhow_error)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Closed link {} valid_until={}",
            input.link_id, input.valid_until
        ))]))
    }

    #[tool(description = "List inbound backlinks for a page")]
    fn brain_backlinks(
        &self,
        #[tool(aggr)] input: BrainBacklinksInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let filter = parse_temporal_filter(input.temporal.as_deref())?;
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
             ORDER BY l.created_at DESC"
        );

        let mut stmt = db.prepare(&sql).map_err(map_db_error)?;

        let rows: Vec<BacklinkRow> = stmt
            .query_map([to_id], |row| {
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
    fn brain_graph(
        &self,
        #[tool(aggr)] input: BrainGraphInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let depth = input.depth.unwrap_or(1).min(MAX_LIMIT);
        let filter = parse_temporal_filter(input.temporal.as_deref())?;

        let result =
            graph::neighborhood_graph(&input.slug, depth, filter, &db).map_err(map_graph_error)?;

        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Run contradiction detection on a page or all pages")]
    fn brain_check(
        &self,
        #[tool(aggr)] input: BrainCheckInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let all = input.slug.is_none();
        check::run(&db, input.slug, all, None, true).map_err(map_anyhow_error)?;

        // Fetch unresolved contradictions as JSON
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

        use crate::core::assertions::Contradiction;
        let contradictions: Vec<Contradiction> = stmt
            .query_map([], |row| {
                Ok(Contradiction {
                    page_slug: row.get(0)?,
                    other_page_slug: row.get(1)?,
                    r#type: row.get(2)?,
                    description: row.get(3)?,
                    detected_at: row.get(4)?,
                })
            })
            .map_err(map_db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_db_error)?;

        let json = serde_json::to_string_pretty(&contradictions)
            .map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Show timeline entries for a page")]
    fn brain_timeline(
        &self,
        #[tool(aggr)] input: BrainTimelineInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());

        let limit = input.limit.unwrap_or(50).min(MAX_LIMIT);

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
    fn brain_tags(
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

        let result = server
            .brain_link_close(BrainLinkCloseInput {
                link_id: 1,
                valid_until: "2025-06".to_string(),
            })
            .unwrap();

        let text = extract_text(&result);
        assert!(text.contains("Closed link 1"));
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
                temporal: None,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode(-32001));
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
