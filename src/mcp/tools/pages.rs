//! Page-CRUD tool bodies: `memory_get` (slug -> canonical page JSON),
//! `memory_put` (create/update with optimistic-concurrency `expected_version`
//! handling and write-gate enforcement), `memory_list` (filtered listing
//! across collections, namespaces, wings, and types), and `memory_raw`
//! (attach arbitrary JSON to a page under a named source). All errors
//! route through `mcp::errors::map_*` helpers; ad-hoc `rmcp::Error` is
//! forbidden in this file.

use rmcp::model::{CallToolResult, Content};
use rmcp::tool;
use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::core::collections::OpKind;
use crate::core::namespace;
use crate::core::supersede;
use crate::core::vault_sync;
use crate::mcp::errors::{
    conflict_error, invalid_params, map_anyhow_error, map_db_error, map_namespace_error,
    map_serialize_error, map_vault_sync_error, serialize_response, tool_error,
};
use crate::mcp::server::{
    canonical_slug, canonicalize_page_for_mcp, page_id_for_resolved,
    resolve_memory_collection_filter_for_mcp, resolve_slug_for_mcp, MemoryGetInput,
    MemoryListInput, MemoryPutInput, MemoryRawInput, QuaidServer,
};
use crate::mcp::validation::{validate_content, validate_slug, MAX_LIMIT, MAX_RAW_DATA_LEN};

impl QuaidServer {
    /// `memory_get` MCP tool: resolve a slug to its canonical page, render
    /// the truth body and timeline together, and surface supersede pointers
    /// alongside the standard page payload.
    #[tool(description = "Get a page by slug")]
    /// `memory_get` MCP tool: resolve a slug to its canonical page, render
    /// the truth body and timeline together, and surface supersede pointers
    /// alongside the standard page payload.
    pub fn memory_get(
        &self,
        #[tool(aggr)] input: MemoryGetInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::Read)?;
        let page = vault_sync::get_page_by_input(&db, &input.slug).map_err(map_vault_sync_error)?;
        let canonical_page = canonicalize_page_for_mcp(&page, &resolved);
        let successor_slug = supersede::successor_slug_by_id(&db, canonical_page.superseded_by)
            .map_err(map_db_error)?;
        let supersedes = canonical_page
            .frontmatter
            .get("supersedes")
            .and_then(serde_json::Value::as_str)
            .map(|slug| canonical_slug(&resolved.collection_name, slug));

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "slug": canonical_page.slug,
            "uuid": canonical_page.uuid,
            "type": canonical_page.page_type,
            "title": canonical_page.title,
            "summary": canonical_page.summary,
            "compiled_truth": canonical_page.compiled_truth,
            "timeline": canonical_page.timeline,
            "frontmatter": canonical_page.frontmatter,
            "wing": canonical_page.wing,
            "room": canonical_page.room,
            "version": canonical_page.version,
            "created_at": canonical_page.created_at,
            "updated_at": canonical_page.updated_at,
            "truth_updated_at": canonical_page.truth_updated_at,
            "timeline_updated_at": canonical_page.timeline_updated_at,
            "supersedes": supersedes,
            "superseded_by": successor_slug,
        }))
        .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// `memory_put` MCP tool: create or update a page under optimistic
    /// concurrency, honouring the collection write-gate and rejecting
    /// version mismatches with a structured `ConflictError`.
    #[tool(description = "Write or update a page")]
    /// `memory_put` MCP tool: create or update a page under optimistic
    /// concurrency, honouring the collection write-gate and rejecting
    /// version mismatches with a structured `ConflictError`.
    pub fn memory_put(
        &self,
        #[tool(aggr)] input: MemoryPutInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        validate_content(&input.content)?;
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let namespace_filter = input.namespace.as_deref().unwrap_or("");
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(
            &db,
            &input.slug,
            if input.expected_version.is_some() {
                OpKind::WriteUpdate
            } else {
                OpKind::WriteCreate
            },
        )?;
        // Collection write-gate must run BEFORE any OCC/precondition prevalidation.
        // If the collection is restoring or needs_full_sync, CollectionRestoringError wins
        // over any version-conflict or existence-conflict that the prevalidation would surface.
        vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)
            .map_err(map_vault_sync_error)?;
        let existing_version: Option<i64> = db
            .query_row(
                "SELECT version
                 FROM pages
                 WHERE collection_id = ?1 AND namespace = ?2 AND slug = ?3",
                rusqlite::params![resolved.collection_id, namespace_filter, &resolved.slug],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_db_error)?;
        match (existing_version, input.expected_version) {
            (None, Some(expected)) => {
                return Err(conflict_error(
                    format!("ConflictError: page does not exist at version {expected}"),
                    Some(serde_json::json!({ "current_version": null })),
                ));
            }
            (Some(current), None) => {
                return Err(conflict_error(
                    format!(
                        "ConflictError: page already exists (current version: {current}). Provide expected_version to update."
                    ),
                    Some(serde_json::json!({ "current_version": current })),
                ));
            }
            _ => {}
        }
        crate::commands::put::put_from_string_quiet_with_namespace(
            &db,
            &canonical_slug(&resolved.collection_name, &resolved.slug),
            &input.content,
            Some(namespace_filter),
            input.expected_version,
        )
        .map_err(|err| {
            let message = err.to_string();
            // Normalise the canonical (`ConflictError: `) and legacy
            // (`Conflict: `) spellings onto the single `ConflictError: `
            // prefix used for every -32009 response.
            if message.contains("ConflictError") || message.contains("Conflict:") {
                conflict_error(
                    message.replace("Conflict: ", "ConflictError: "),
                    Some(serde_json::json!({ "current_version": existing_version })),
                )
            } else {
                map_anyhow_error(err)
            }
        })?;
        let version: i64 = db
            .query_row(
                "SELECT version
                 FROM pages
                 WHERE collection_id = ?1 AND namespace = ?2 AND slug = ?3",
                rusqlite::params![resolved.collection_id, namespace_filter, &resolved.slug],
                |row| row.get(0),
            )
            .map_err(map_db_error)?;
        let verb = if input.expected_version.is_some() {
            "Updated"
        } else {
            "Created"
        };
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{verb} {}::{} (version {})",
            resolved.collection_name, resolved.slug, version
        ))]))
    }

    /// `memory_list` MCP tool: enumerate pages with optional collection,
    /// namespace, wing, and type filters, ordered by recency and clamped to
    /// `MAX_LIMIT`.
    #[tool(description = "List pages with optional filters")]
    /// `memory_list` MCP tool: enumerate pages with optional collection,
    /// namespace, wing, and type filters, ordered by recency and clamped to
    /// `MAX_LIMIT`.
    pub fn memory_list(
        &self,
        #[tool(aggr)] input: MemoryListInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let namespace_filter = input.namespace.as_deref().or(Some(""));
        let collection_filter =
            resolve_memory_collection_filter_for_mcp(&db, input.collection.as_deref())?;

        let limit = input.limit.unwrap_or(50).min(MAX_LIMIT);
        let mut sql = String::from(
            "SELECT c.name || '::' || p.slug, p.type, p.summary \
             FROM pages p \
             JOIN collections c ON c.id = p.collection_id \
             WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref w) = input.wing {
            sql.push_str(" AND p.wing = ?");
            params.push(Box::new(w.clone()));
        }
        if let Some(ref t) = input.page_type {
            sql.push_str(" AND p.type = ?");
            params.push(Box::new(t.clone()));
        }
        if let Some(collection) = collection_filter {
            sql.push_str(" AND p.collection_id = ?");
            params.push(Box::new(collection.id));
        }
        if let Some(namespace) = namespace_filter {
            if namespace.is_empty() {
                sql.push_str(" AND p.namespace = ?");
                params.push(Box::new(String::new()));
            } else {
                sql.push_str(" AND (p.namespace = ? OR p.namespace = '')");
                params.push(Box::new(namespace.to_owned()));
            }
        }
        sql.push_str(" ORDER BY p.updated_at DESC LIMIT ?");
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

        let json = serde_json::to_string_pretty(&entries).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// `memory_raw` MCP tool: attach an arbitrary JSON-object payload to a
    /// page under a named source identifier, guarded against silent
    /// replacement by the `overwrite` flag.
    #[tool(description = "Store raw structured data (API responses, JSON) for a page")]
    /// `memory_raw` MCP tool: attach an arbitrary JSON-object payload to a
    /// page under a named source identifier, guarded against silent
    /// replacement by the `overwrite` flag.
    pub fn memory_raw(
        &self,
        #[tool(aggr)] input: MemoryRawInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.slug)?;
        if input.source.is_empty() {
            return Err(invalid_params("source must not be empty"));
        }
        if !input.data.is_object() {
            return Err(invalid_params(
                "data must be a JSON object, not an array or scalar",
            ));
        }
        let data_json = serde_json::to_string(&input.data).map_err(map_serialize_error)?;
        if data_json.len() > MAX_RAW_DATA_LEN {
            return Err(invalid_params(format!(
                "data exceeds maximum size of {MAX_RAW_DATA_LEN} bytes"
            )));
        }
        let overwrite = input.overwrite.unwrap_or(false);
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::WriteUpdate)?;
        vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)
            .map_err(map_vault_sync_error)?;

        let page_id = page_id_for_resolved(&db, &resolved)?;
        let canonical_page_slug = canonical_slug(&resolved.collection_name, &resolved.slug);

        // Guard against silent replacement of existing source data.
        let existing: Option<i64> = db
            .query_row(
                "SELECT id FROM raw_data WHERE page_id = ?1 AND source = ?2",
                rusqlite::params![page_id, &input.source],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_db_error)?;

        if existing.is_some() && !overwrite {
            return Err(tool_error(format!(
                "raw data for source '{}' already exists on '{}'; set overwrite=true to replace",
                input.source, canonical_page_slug
            )));
        }

        db.execute(
            "INSERT OR REPLACE INTO raw_data (page_id, source, data, fetched_at) \
             VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
            rusqlite::params![page_id, input.source, data_json],
        )
        .map_err(map_db_error)?;

        let row_id = db.last_insert_rowid();
        let result = serde_json::json!({ "id": row_id });
        Ok(CallToolResult::success(vec![Content::text(
            serialize_response(&result)?,
        )]))
    }
}
