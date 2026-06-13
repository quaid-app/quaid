//! Cross-page linkage tool bodies: `memory_link` (open a typed temporal
//! link), `memory_link_close` (close one by id), `memory_backlinks` (the
//! inbound-edge listing with optional temporal filter), and `memory_graph`
//! (the bounded N-hop neighbourhood walk). All four resolve slugs through
//! the shared `resolve_slug_for_mcp` helper and route errors through
//! `mcp::errors::map_*`.

use rmcp::model::{CallToolResult, Content};
use rmcp::tool;
use serde::Serialize;

use crate::commands::link;
use crate::core::collections::OpKind;
use crate::core::graph::{self, TemporalFilter};
use crate::mcp::errors::{map_anyhow_error, map_db_error, map_graph_error, map_serialize_error};
use crate::mcp::server::{
    canonical_slug, page_id_for_resolved, resolve_slug_for_mcp, MemoryBacklinksInput,
    MemoryGraphInput, MemoryLinkCloseInput, MemoryLinkInput, QuaidServer,
};
use crate::mcp::validation::{
    parse_temporal_filter, validate_relationship, validate_slug, validate_temporal_value, MAX_LIMIT,
};

impl QuaidServer {
    /// `memory_link` MCP tool: create a typed, temporally-bounded
    /// directed link from one page to another, validating both endpoints
    /// and the optional `valid_from` / `valid_until` bounds.
    #[tool(description = "Create a typed temporal link between two pages")]
    /// `memory_link` MCP tool: create a typed, temporally-bounded
    /// directed link from one page to another, validating both endpoints
    /// and the optional `valid_from` / `valid_until` bounds.
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
        crate::core::namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(crate::mcp::errors::map_namespace_error)?;
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let from = resolve_slug_for_mcp(&db, &input.from_slug, OpKind::WriteUpdate)?;
        let to = resolve_slug_for_mcp(&db, &input.to_slug, OpKind::WriteUpdate)?;
        let from_slug = canonical_slug(&from.collection_name, &from.slug);
        let to_slug = canonical_slug(&to.collection_name, &to.slug);

        link::run_silent_with_namespace(
            &db,
            &from_slug,
            &to_slug,
            &input.relationship,
            input.valid_from,
            input.valid_until,
            input.namespace.as_deref(),
        )
        .map_err(map_anyhow_error)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked {} → {} ({})",
            from_slug, to_slug, input.relationship
        ))]))
    }

    /// `memory_link_close` MCP tool: close an existing link by row id,
    /// stamping a `valid_until` timestamp without otherwise touching the
    /// link's relationship or endpoints.
    #[tool(description = "Close a temporal link by its database ID")]
    /// `memory_link_close` MCP tool: close an existing link by row id,
    /// stamping a `valid_until` timestamp without otherwise touching the
    /// link's relationship or endpoints.
    pub fn memory_link_close(
        &self,
        #[tool(aggr)] input: MemoryLinkCloseInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_temporal_value(&input.valid_until, "valid_until")?;
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());

        link::close_silent(&db, input.link_id, &input.valid_until).map_err(map_anyhow_error)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Closed link {} valid_until={}",
            input.link_id, input.valid_until
        ))]))
    }

    /// `memory_backlinks` MCP tool: list inbound edges pointing at the
    /// resolved page, optionally restricted to currently-active links and
    /// clamped to `MAX_LIMIT`.
    #[tool(description = "List inbound backlinks for a page")]
    /// `memory_backlinks` MCP tool: list inbound edges pointing at the
    /// resolved page, optionally restricted to currently-active links and
    /// clamped to `MAX_LIMIT`.
    pub fn memory_backlinks(
        &self,
        #[tool(aggr)] input: MemoryBacklinksInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let filter = parse_temporal_filter(input.temporal.as_deref())?;
        let limit = input.limit.unwrap_or(100).min(MAX_LIMIT);
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        crate::core::namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(crate::mcp::errors::map_namespace_error)?;
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::Read)?;
        let to_id = page_id_for_resolved(&db, &resolved, input.namespace.as_deref())?;

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

        let json = serde_json::to_string_pretty(&rows).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// `memory_graph` MCP tool: build a bounded N-hop neighbourhood graph
    /// from the resolved page, with depth clamped to `graph::MAX_DEPTH`
    /// and edges filtered by the requested temporal mode.
    #[tool(description = "N-hop neighbourhood graph from a page")]
    /// `memory_graph` MCP tool: build a bounded N-hop neighbourhood graph
    /// from the resolved page, with depth clamped to `graph::MAX_DEPTH`
    /// and edges filtered by the requested temporal mode.
    pub fn memory_graph(
        &self,
        #[tool(aggr)] input: MemoryGraphInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());

        let depth = input.depth.unwrap_or(1).min(graph::MAX_DEPTH);
        let filter = parse_temporal_filter(input.temporal.as_deref())?;
        crate::core::namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(crate::mcp::errors::map_namespace_error)?;
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::Read)?;
        let page_id = page_id_for_resolved(&db, &resolved, input.namespace.as_deref())?;
        let result = graph::neighborhood_graph_for_page(
            page_id,
            &resolved.collection_name,
            &resolved.slug,
            depth,
            filter,
            &db,
        )
        .map_err(map_graph_error)?;

        let json = serde_json::to_string_pretty(&result).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
