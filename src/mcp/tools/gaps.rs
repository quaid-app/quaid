//! Knowledge-gap tool bodies: `memory_gap` (record an unanswered query as
//! a SHA-256 hash with optional page binding) and `memory_gaps` (paginated
//! list, optionally including resolved gaps). Errors from the
//! `crate::core::gaps` layer route through `mcp::errors::map_gaps_error`;
//! none of the bodies in this file construct `rmcp::Error` directly, in
//! line with the §2.4 audit convention.

use rmcp::model::{CallToolResult, Content};
use rmcp::tool;

use crate::core::collections::OpKind;
use crate::core::gaps;
use crate::core::vault_sync;
use crate::mcp::errors::{
    invalid_params, map_db_error, map_gaps_error, map_serialize_error, map_vault_sync_error,
    serialize_response,
};
use crate::mcp::server::{
    page_id_for_resolved, resolve_slug_for_mcp, MemoryGapInput, MemoryGapsInput, QuaidServer,
};
use crate::mcp::validation::{validate_slug, MAX_GAP_CONTEXT_LEN, MAX_LIMIT};

impl QuaidServer {
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
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
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
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());

        let gap_list = gaps::list_gaps(resolved, limit, &db).map_err(map_gaps_error)?;

        let json = serde_json::to_string_pretty(&gap_list).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
