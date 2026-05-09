//! Admin tool bodies: `memory_stats` (page/link/assertion counts plus
//! database file size), `memory_collections` (the per-collection state
//! summary used by MCP clients), `memory_namespace_create`, and
//! `memory_namespace_destroy`. Each tool is registered through the same
//! `#[tool(tool_box)] impl QuaidServer { ... }` block; the macro merges
//! these registrations with those declared in sibling files into a single
//! `tools/list` response.

use rmcp::model::{CallToolResult, Content};
use rmcp::tool;

use crate::core::namespace;
use crate::core::vault_sync;
use crate::mcp::errors::{
    map_db_error, map_namespace_error, map_serialize_error, map_vault_sync_error,
    serialize_response,
};
use crate::mcp::server::{
    MemoryCollectionsInput, MemoryNamespaceCreateInput, MemoryNamespaceDestroyInput,
    MemoryStatsInput, QuaidServer,
};

impl QuaidServer {
    #[tool(description = "Brain statistics (page count, link count, etc.)")]
    pub fn memory_stats(
        &self,
        #[tool(aggr)] _input: MemoryStatsInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());

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
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
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
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let namespace = namespace::create_namespace(&db, &input.id, input.ttl_hours)
            .map_err(map_namespace_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&namespace).map_err(map_serialize_error)?,
        )]))
    }

    #[tool(description = "Destroy a namespace and all pages assigned to it")]
    pub fn memory_namespace_destroy(
        &self,
        #[tool(aggr)] input: MemoryNamespaceDestroyInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let deleted_pages =
            namespace::destroy_namespace(&db, &input.id).map_err(map_namespace_error)?;
        let result = serde_json::json!({
            "status": "ok",
            "namespace": input.id,
            "deleted_pages": deleted_pages,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).map_err(map_serialize_error)?,
        )]))
    }
}
