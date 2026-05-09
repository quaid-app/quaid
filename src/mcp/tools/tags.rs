//! Tag and timeline tool bodies: `memory_tags` (list, add, or remove tags
//! attached to a page) and `memory_timeline` (the per-page timeline-entry
//! readout, with a fallback to the legacy markdown timeline field on the
//! page itself). Both tools resolve the slug via the shared collection
//! helpers in `mcp::server` and route every error through
//! `mcp::errors::map_*` helpers.

use rmcp::model::{CallToolResult, Content};
use rmcp::tool;
use serde::Serialize;

use crate::commands::get;
use crate::core::collections::OpKind;
use crate::core::vault_sync;
use crate::mcp::errors::{
    map_anyhow_error, map_db_error, map_serialize_error, map_vault_sync_error, page_not_found,
};
use crate::mcp::server::{
    canonical_slug, page_id_for_resolved, resolve_slug_for_mcp, MemoryTagsInput,
    MemoryTimelineInput, QuaidServer,
};
use crate::mcp::validation::{validate_slug, validate_tag_list, MAX_LIMIT};

impl QuaidServer {
    #[tool(description = "Show timeline entries for a page")]
    pub fn memory_timeline(
        &self,
        #[tool(aggr)] input: MemoryTimelineInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
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

        let json = serde_json::to_string_pretty(&output).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List, add, or remove tags on a page")]
    pub fn memory_tags(
        &self,
        #[tool(aggr)] input: MemoryTagsInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());

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
                rusqlite::Error::QueryReturnedNoRows => page_not_found(&input.slug),
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

        let json = serde_json::to_string_pretty(&tags).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
