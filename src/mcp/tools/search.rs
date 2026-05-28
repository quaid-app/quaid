//! Read-side query tool bodies: `memory_query` (the hybrid semantic +
//! FTS5 retrieval that drives most agent reads, with optional progressive
//! depth expansion) and `memory_search` (raw FTS5 search). Both honour
//! collection, namespace, wing, and superseded filters. The call sites
//! into `crate::core::search::*` and `crate::core::fts::*` are kept
//! minimal to make a future API rename in the search layer (see the
//! parallel `collapse-search-fn-variants` change) a clean rebase.

use rmcp::model::{CallToolResult, Content};
use rmcp::tool;

use crate::core::fts::{expand_numeric_fts_query, sanitize_fts_query, search_fts, FtsQuery};
use crate::core::gaps;
use crate::core::namespace;
use crate::core::progressive::progressive_retrieve_with_namespace;
use crate::core::search::{hybrid_search, HybridSearch};
use crate::mcp::errors::{map_namespace_error, map_search_error, map_serialize_error};
use crate::mcp::server::{
    resolve_memory_collection_filter_for_mcp, MemoryQueryInput, MemorySearchInput, QuaidServer,
};
use crate::mcp::validation::MAX_LIMIT;

impl QuaidServer {
    /// `memory_query` MCP tool: run hybrid semantic + FTS5 retrieval with
    /// optional progressive-depth expansion, auto-logging weak results as
    /// knowledge gaps so the brain remembers what it failed to answer.
    #[tool(description = "Hybrid semantic + FTS5 query")]
    /// `memory_query` MCP tool: run hybrid semantic + FTS5 retrieval with
    /// optional progressive-depth expansion, auto-logging weak results as
    /// knowledge gaps so the brain remembers what it failed to answer.
    pub fn memory_query(
        &self,
        #[tool(aggr)] input: MemoryQueryInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let namespace_filter = input.namespace.as_deref().or(Some(""));
        let collection_filter =
            resolve_memory_collection_filter_for_mcp(&db, input.collection.as_deref())?;
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
                hops: None,
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

        let json = serde_json::to_string_pretty(&results).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// `memory_search` MCP tool: run an FTS5 full-text search over the
    /// sanitised query, honouring collection, namespace, wing, and
    /// superseded filters and clamping the result count to `MAX_LIMIT`.
    #[tool(description = "FTS5 full-text search")]
    /// `memory_search` MCP tool: run an FTS5 full-text search over the
    /// sanitised query, honouring collection, namespace, wing, and
    /// superseded filters and clamping the result count to `MAX_LIMIT`.
    pub fn memory_search(
        &self,
        #[tool(aggr)] input: MemorySearchInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let namespace_filter = input.namespace.as_deref().or(Some(""));
        let collection_filter =
            resolve_memory_collection_filter_for_mcp(&db, input.collection.as_deref())?;
        let include_superseded = input.include_superseded.unwrap_or(false);

        let limit = input.limit.unwrap_or(50).min(MAX_LIMIT) as usize;
        let safe_query = expand_numeric_fts_query(&sanitize_fts_query(&input.query));
        let results = search_fts(
            &db,
            FtsQuery {
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

        let json = serde_json::to_string_pretty(&results).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
