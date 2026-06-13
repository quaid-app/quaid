//! Read-side query tool bodies: `memory_query` (the hybrid semantic +
//! FTS5 retrieval that drives most agent reads, with optional progressive
//! depth expansion) and `memory_search` (tiered FTS5 search — the same
//! sanitize + AND→OR-blended path as CLI `quaid search`). Both honour
//! collection, namespace, wing, and superseded filters. The call sites
//! into `crate::core::search::*` and `crate::core::fts::*` are kept
//! minimal to make a future API rename in the search layer (see the
//! parallel `collapse-search-fn-variants` change) a clean rebase.

use rmcp::model::{CallToolResult, Content};
use rmcp::tool;

use crate::core::fts::{sanitize_fts_query, search_fts_tiered, FtsQuery};
use crate::core::gaps;
use crate::core::namespace;
use crate::core::progressive::progressive_retrieve_with_namespace;
use crate::core::search::{
    configured_max_chunks_per_doc, configured_relevance_floor, dedup_chunks_per_page,
    filter_below_floor, hybrid_search, HybridSearch,
};
use crate::mcp::errors::{
    invalid_params, map_namespace_error, map_search_error, map_serialize_error,
};
use crate::mcp::server::{
    resolve_memory_collection_filter_for_mcp, MemoryQueryInput, MemorySearchInput, QuaidServer,
};
use crate::mcp::validation::MAX_LIMIT;

/// Reject a caller-supplied relevance floor outside `[0.0, 1.0]`.
fn validate_relevance_floor(floor: Option<f64>) -> Result<Option<f64>, rmcp::Error> {
    if let Some(value) = floor {
        if !(0.0..=1.0).contains(&value) {
            return Err(invalid_params(format!(
                "relevance_floor must be between 0.0 and 1.0, got {value}"
            )));
        }
    }
    Ok(floor)
}

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

        let relevance_floor = validate_relevance_floor(input.relevance_floor)?;

        let limit = input.limit.unwrap_or(10).min(MAX_LIMIT) as usize;
        let results = hybrid_search(
            &db,
            HybridSearch {
                query: &input.query,
                wing: input.wing.as_deref(),
                collection: collection_filter.as_ref().map(|collection| collection.id),
                namespace: namespace_filter,
                include_superseded,
                include_quarantined: false,
                canonical: true,
                limit,
                hops: input.hops,
                relevance_floor,
                max_chunks_per_doc: input.max_chunks_per_doc.map(|value| value as usize),
            },
        )
        .map_err(map_search_error)?;

        // Auto-log knowledge gap on weak results, recording query-free
        // diagnostics as the context (never the query text itself).
        if gaps::should_log_gap(&results) {
            let _ = gaps::log_gap(
                None,
                &input.query,
                &gaps::auto_gap_context("hybrid_search", &results),
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

    /// `memory_search` MCP tool: run a tiered FTS5 full-text search over the
    /// sanitised query — a precision-first implicit-AND pass with OR-recall
    /// hits blended in below the AND hits, the same path as CLI
    /// `quaid search` — honouring collection, namespace, wing, and
    /// superseded filters and clamping the result count to `MAX_LIMIT`.
    #[tool(description = "FTS5 full-text search")]
    /// `memory_search` MCP tool: run a tiered FTS5 full-text search over the
    /// sanitised query — a precision-first implicit-AND pass with OR-recall
    /// hits blended in below the AND hits, the same path as CLI
    /// `quaid search` — honouring collection, namespace, wing, and
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

        let relevance_floor = validate_relevance_floor(input.relevance_floor)?;

        let limit = input.limit.unwrap_or(50).min(MAX_LIMIT) as usize;
        // `search_fts_tiered` applies numeric-alias expansion in its AND
        // pass, so sanitization is the only preprocessing needed here.
        let safe_query = sanitize_fts_query(&input.query);
        let results = search_fts_tiered(
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

        // Post-retrieval quality passes (dedup → floor); identity no-ops at
        // the seeded config defaults. Parameter values, when given, override
        // the `search.*` config keys.
        let max_per_page = match input.max_chunks_per_doc {
            Some(value) => value as usize,
            None => configured_max_chunks_per_doc(&db).map_err(map_search_error)?,
        };
        let floor = match relevance_floor {
            Some(value) => value,
            None => configured_relevance_floor(&db).map_err(map_search_error)?,
        };
        let results = filter_below_floor(dedup_chunks_per_page(results, max_per_page), floor);

        let json = serde_json::to_string_pretty(&results).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
