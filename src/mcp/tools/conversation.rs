//! Conversation-lifecycle tool bodies: `memory_add_turn` (append a turn
//! to an open conversation, scheduling debounced extraction when enabled),
//! `memory_close_session` (mark the session closed and trigger extraction),
//! `memory_close_action` (mark an `action_item` page done or cancelled),
//! `memory_correct` (start a correction dialogue), and
//! `memory_correct_continue` (continue or abandon one). The private
//! `memory_close_action_impl` helper sits alongside its sole caller per
//! design D6 / R3.

use rmcp::model::{CallToolResult, Content};
use rmcp::tool;
use rusqlite::Connection;

use crate::commands::{get, put};
use crate::core::collections::OpKind;
use crate::core::conversation::{correction, queue as conversation_queue, turn_writer};
use crate::core::namespace;
use crate::core::types::{ExtractionTriggerKind, TurnRole};
use crate::core::vault_sync;
use crate::mcp::errors::{
    invalid_params, kind_error, map_anyhow_error, map_close_action_put_error, map_correction_error,
    map_db_error, map_extraction_queue_error, map_namespace_error, map_serialize_error,
    map_turn_write_error, map_vault_sync_error,
};
use crate::mcp::server::{
    append_note, canonical_slug, extraction_debounce_ms, extraction_enabled, resolve_slug_for_mcp,
    MemoryAddTurnInput, MemoryCloseActionInput, MemoryCloseSessionInput,
    MemoryCorrectContinueInput, MemoryCorrectInput, QuaidServer,
};
use crate::mcp::validation::{
    validate_close_action_status, validate_content, validate_slug, validate_turn_timestamp,
};

impl QuaidServer {
    /// `memory_add_turn` MCP tool: append a turn to an open conversation
    /// session, enqueueing a debounced extraction job when extraction is
    /// enabled in config.
    #[tool(description = "Append a turn to a conversation session")]
    /// `memory_add_turn` MCP tool: append a turn to an open conversation
    /// session, enqueueing a debounced extraction job when extraction is
    /// enabled in config.
    pub fn memory_add_turn(
        &self,
        #[tool(aggr)] input: MemoryAddTurnInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_content(&input.content)?;
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        if let Some(metadata) = input.metadata.as_ref() {
            if !metadata.is_object() {
                return Err(invalid_params("metadata must be a JSON object"));
            }
        }

        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let role = input.role.parse::<TurnRole>().map_err(invalid_params)?;
        let timestamp = match input.timestamp.as_deref() {
            Some(timestamp) => {
                validate_turn_timestamp(timestamp)?;
                timestamp.to_owned()
            }
            None => {
                conversation_queue::current_timestamp(&db).map_err(map_extraction_queue_error)?
            }
        };

        let write_result = turn_writer::append_turn(
            &db,
            &input.session_id,
            role,
            &input.content,
            &timestamp,
            input.metadata,
            input.namespace.as_deref(),
        )
        .map_err(map_turn_write_error)?;

        let extraction_scheduled_at = if extraction_enabled(&db)? {
            let scheduled_for =
                conversation_queue::scheduled_timestamp_after_ms(&db, extraction_debounce_ms(&db)?)
                    .map_err(map_extraction_queue_error)?;
            let queue_session_id = conversation_queue::session_queue_key(
                input.namespace.as_deref(),
                &input.session_id,
            );
            conversation_queue::enqueue(
                &db,
                &queue_session_id,
                &write_result.conversation_path,
                ExtractionTriggerKind::Debounce,
                &scheduled_for,
            )
            .map_err(map_extraction_queue_error)?;
            Some(scheduled_for)
        } else {
            None
        };

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "turn_id": write_result.turn_id,
            "conversation_path": write_result.conversation_path,
            "extraction_scheduled_at": extraction_scheduled_at,
        }))
        .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// `memory_close_session` MCP tool: mark a conversation session
    /// closed, enqueue a session-close extraction job, and report the
    /// caller's position in the extraction queue.
    #[tool(description = "Close a conversation session and trigger extraction")]
    /// `memory_close_session` MCP tool: mark a conversation session
    /// closed, enqueue a session-close extraction job, and report the
    /// caller's position in the extraction queue.
    pub fn memory_close_session(
        &self,
        #[tool(aggr)] input: MemoryCloseSessionInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        namespace::validate_optional_namespace(input.namespace.as_deref())
            .map_err(map_namespace_error)?;
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let close_result =
            turn_writer::close_session(&db, &input.session_id, input.namespace.as_deref())
                .map_err(map_turn_write_error)?;

        let queue_session_id =
            conversation_queue::session_queue_key(input.namespace.as_deref(), &input.session_id);
        let (extraction_triggered, queue_position) = if close_result.newly_closed {
            let scheduled_for =
                conversation_queue::current_timestamp(&db).map_err(map_extraction_queue_error)?;
            conversation_queue::enqueue(
                &db,
                &queue_session_id,
                &close_result.conversation_path,
                ExtractionTriggerKind::SessionClose,
                &scheduled_for,
            )
            .map_err(map_extraction_queue_error)?;
            let position = conversation_queue::pending_queue_position(&db, &queue_session_id)
                .map_err(map_extraction_queue_error)?
                .unwrap_or(0);
            (true, position)
        } else {
            let position = conversation_queue::pending_queue_position(&db, &queue_session_id)
                .map_err(map_extraction_queue_error)?
                .unwrap_or(0);
            (position > 0, position)
        };

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "closed_at": close_result.closed_at,
            "extraction_triggered": extraction_triggered,
            "queue_position": queue_position,
        }))
        .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// `memory_close_action` MCP tool: mark an `action_item` page `done`
    /// or `cancelled` in place, appending an optional note and bumping the
    /// page version through the same `put` flow used for direct edits.
    #[tool(description = "Close an action item in place")]
    /// `memory_close_action` MCP tool: mark an `action_item` page `done`
    /// or `cancelled` in place, appending an optional note and bumping the
    /// page version through the same `put` flow used for direct edits.
    pub fn memory_close_action(
        &self,
        #[tool(aggr)] input: MemoryCloseActionInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        self.memory_close_action_impl(input, |_, _, _| Ok(()))
    }

    pub(crate) fn memory_close_action_impl<F>(
        &self,
        input: MemoryCloseActionInput,
        before_write: F,
    ) -> Result<CallToolResult, rmcp::Error>
    where
        F: FnOnce(
            &Connection,
            &vault_sync::ResolvedSlug,
            &crate::core::types::Page,
        ) -> Result<(), rmcp::Error>,
    {
        validate_slug(&input.slug)?;
        validate_close_action_status(&input.status)?;
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let resolved = resolve_slug_for_mcp(&db, &input.slug, OpKind::WriteUpdate)?;
        vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)
            .map_err(map_vault_sync_error)?;
        let page = get::get_page_by_key(&db, resolved.collection_id, &resolved.slug)
            .map_err(map_anyhow_error)?;
        if page.page_type != "action_item" {
            return Err(kind_error(format!(
                "KindError: page `{}` is `{}` not `action_item`",
                canonical_slug(&resolved.collection_name, &resolved.slug),
                page.page_type
            )));
        }

        let mut updated_page = page.clone();
        crate::core::types::frontmatter_insert_string(
            &mut updated_page.frontmatter,
            "status",
            input.status.clone(),
        );
        if let Some(note) = input.note.as_deref() {
            append_note(&mut updated_page.compiled_truth, note);
        }

        before_write(&db, &resolved, &updated_page)?;

        let content = crate::core::markdown::render_page(&updated_page);
        put::put_from_string_quiet(
            &db,
            &canonical_slug(&resolved.collection_name, &resolved.slug),
            &content,
            Some(page.version),
        )
        .map_err(|error| map_close_action_put_error(&db, &resolved, error))?;

        // The put above wrote the global namespace; read the version back
        // from exactly that row.
        let page_id = crate::core::pages::resolve(
            &db,
            &crate::core::pages::PageKey {
                collection_id: resolved.collection_id,
                namespace: Some(""),
                slug: &resolved.slug,
            },
        )
        .map_err(map_db_error)?;
        let (updated_at, version): (String, i64) = db
            .query_row(
                "SELECT updated_at, version FROM pages WHERE id = ?1",
                [page_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(map_db_error)?;

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "updated_at": updated_at,
            "version": version,
        }))
        .map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// `memory_correct` MCP tool: open a multi-step correction dialogue
    /// against an extracted-fact page, returning the first SLM-mediated
    /// step.
    #[tool(description = "Start a correction dialogue for an extracted fact")]
    /// `memory_correct` MCP tool: open a multi-step correction dialogue
    /// against an extracted-fact page, returning the first SLM-mediated
    /// step.
    pub fn memory_correct(
        &self,
        #[tool(aggr)] input: MemoryCorrectInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        validate_slug(&input.fact_slug)?;
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let step = correction::start_correction(
            &db,
            self.slm().as_ref(),
            &input.fact_slug,
            &input.correction,
        )
        .map_err(map_correction_error)?;
        let json = serde_json::to_string_pretty(&step).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// `memory_correct_continue` MCP tool: advance an open correction
    /// dialogue with a follow-up response, or abandon it without
    /// committing changes.
    #[tool(description = "Continue or abandon an open fact correction dialogue")]
    /// `memory_correct_continue` MCP tool: advance an open correction
    /// dialogue with a follow-up response, or abandon it without
    /// committing changes.
    pub fn memory_correct_continue(
        &self,
        #[tool(aggr)] input: MemoryCorrectContinueInput,
    ) -> Result<CallToolResult, rmcp::Error> {
        let db = self.db().lock().unwrap_or_else(|e| e.into_inner());
        let step = correction::continue_correction(
            &db,
            self.slm().as_ref(),
            &input.correction_id,
            input.response.as_deref(),
            input.abandon.unwrap_or(false),
        )
        .map_err(map_correction_error)?;
        let json = serde_json::to_string_pretty(&step).map_err(map_serialize_error)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
