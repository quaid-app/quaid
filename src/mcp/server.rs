use std::sync::{Arc, Mutex};

use rmcp::model::*;
use rmcp::schemars;
use rmcp::tool;
use rmcp::{ServerHandler, ServiceExt};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::commands::get::get_page;
use crate::core::fts::search_fts;
use crate::core::markdown;
use crate::core::palace;
use crate::core::search::hybrid_search;

type DbRef = Arc<Mutex<Connection>>;

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
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainSearchInput {
    /// FTS5 search query string
    pub query: String,
    /// Optional wing filter
    pub wing: Option<String>,
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

#[tool(tool_box)]
impl GigaBrainServer {
    #[tool(description = "Get a page by slug")]
    fn brain_get(&self, #[tool(aggr)] input: BrainGetInput) -> Result<CallToolResult, rmcp::Error> {
        let db = self
            .db
            .lock()
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
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
        let db = self
            .db
            .lock()
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

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

        let existing_version: Option<i64> = db
            .prepare("SELECT version FROM pages WHERE slug = ?1")
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?
            .query_row([&input.slug], |row| row.get(0))
            .ok();

        match existing_version {
            None => {
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
                .map_err(|e| {
                    rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None)
                })?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Created {} (version 1)",
                    input.slug
                ))]))
            }
            Some(current) => {
                if let Some(expected) = input.expected_version {
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
                        .map_err(|e| {
                            rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None)
                        })?;

                    if rows == 0 {
                        return Err(rmcp::Error::new(
                            rmcp::model::ErrorCode(-32009),
                            format!(
                                "Conflict: page updated elsewhere (current version: {current})"
                            ),
                            Some(serde_json::json!({ "current_version": current })),
                        ));
                    }

                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "Updated {} (version {})",
                        input.slug,
                        expected + 1
                    ))]))
                } else {
                    db.execute(
                        "UPDATE pages SET \
                             type = ?1, title = ?2, summary = ?3, \
                             compiled_truth = ?4, timeline = ?5, \
                             frontmatter = ?6, wing = ?7, room = ?8, \
                             version = version + 1, \
                             updated_at = ?9, truth_updated_at = ?9, timeline_updated_at = ?9 \
                         WHERE slug = ?10",
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
                        ],
                    )
                    .map_err(|e| {
                        rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None)
                    })?;

                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "Updated {} (version {})",
                        input.slug,
                        current + 1
                    ))]))
                }
            }
        }
    }

    #[tool(description = "Hybrid semantic + FTS5 query")]
    fn brain_query(
        &self,
        #[tool(aggr)] input: BrainQueryInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self
            .db
            .lock()
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let results = hybrid_search(&input.query, input.wing.as_deref(), &db)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "FTS5 full-text search")]
    fn brain_search(
        &self,
        #[tool(aggr)] input: BrainSearchInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self
            .db
            .lock()
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let results = search_fts(&input.query, input.wing.as_deref(), &db)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List pages with optional filters")]
    fn brain_list(
        &self,
        #[tool(aggr)] input: BrainListInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self
            .db
            .lock()
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let limit = input.limit.unwrap_or(50);
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
        let mut stmt = db
            .prepare(&sql)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;

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
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(|e| {
                rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None)
            })?);
        }

        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| rmcp::Error::new(rmcp::model::ErrorCode(-32003), e.to_string(), None))?;
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
