//! Assertion / contradiction-detection tool body: `memory_check`. Runs
//! `crate::core::assertions::extract_assertions` and `check_assertions*`
//! over a single resolved page (or every page when no slug is provided)
//! and returns the unresolved contradictions as JSON. Errors route through
//! `mcp::errors::map_*` helpers so JSON-RPC error codes stay consistent
//! with the rest of the MCP surface.

use rmcp::model::{CallToolResult, Content};
use rmcp::tool;

use crate::commands::{check, get};
use crate::core::collections::OpKind;
use crate::core::vault_sync;
use crate::mcp::errors::{
    map_anyhow_error, map_db_error, map_serialize_error, map_vault_sync_error,
};
use crate::mcp::server::{
    page_id_for_resolved, resolve_slug_for_mcp, MemoryCheckInput, QuaidServer,
};
use crate::mcp::validation::validate_slug;

impl QuaidServer {
    /// `memory_check` MCP tool: run heuristic contradiction detection
    /// over a single resolved page (when a slug is provided) or every
    /// page, returning the unresolved contradictions as JSON.
    #[tool(description = "Run contradiction detection on a page or all pages")]
    /// `memory_check` MCP tool: run heuristic contradiction detection
    /// over a single resolved page (when a slug is provided) or every
    /// page, returning the unresolved contradictions as JSON.
    pub fn memory_check(
        &self,
        #[tool(aggr)] input: MemoryCheckInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        if let Some(slug) = input.slug.as_deref() {
            validate_slug(slug)?;
        }
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
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

        let json = serde_json::to_string_pretty(&contradictions).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
