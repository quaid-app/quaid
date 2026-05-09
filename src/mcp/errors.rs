//! Error mapping helpers for the MCP surface. Every `#[tool]` body MUST route
//! its error paths through one of the `map_*_error` functions defined here so
//! that JSON-RPC error codes (-32001 not-found, -32002 config/ambiguity,
//! -32003 internal/server, -32009 conflict, -32602 invalid-params) stay
//! consistent across the API. Direct construction of `rmcp::Error` is
//! permitted only inside this module (the helper bodies themselves) and inside
//! `mcp::validation` (validators emit `-32602` invalid-params as a control-flow
//! primitive). The block was extracted verbatim from `server.rs:357–546`
//! during the `decompose-mcp-server-module` change.

use rmcp::model::ErrorCode;
use rusqlite::Error as SqliteError;

use crate::core::collections::CollectionError;
use crate::core::conversation::{correction, queue as conversation_queue, turn_writer};
use crate::core::gaps::GapsError;
use crate::core::graph::GraphError;
use crate::core::namespace;
use crate::core::types::SearchError;
use crate::core::vault_sync;

/// Construct a JSON-RPC `-32602` invalid-params error.
pub fn invalid_params(message: impl Into<String>) -> rmcp::Error {
    rmcp::Error::new(ErrorCode(-32602), message.into(), None)
}

/// Construct a JSON-RPC `-32002` ambiguous-slug error with structured
/// candidate metadata.
pub fn ambiguous_slug_error(slug: &str, candidates: Vec<String>) -> rmcp::Error {
    rmcp::Error::new(
        ErrorCode(-32002),
        format!("AmbiguityError: slug `{slug}` matches multiple collections"),
        Some(serde_json::json!({
            "code": "ambiguous_slug",
            "candidates": candidates,
        })),
    )
}

/// Map a [`CollectionError`] from the collections layer onto an
/// `rmcp::Error`.
pub fn map_collection_error(error: CollectionError) -> rmcp::Error {
    match error {
        CollectionError::NotFound { name } => rmcp::Error::new(
            ErrorCode(-32001),
            format!("collection not found: {name}"),
            None,
        ),
        CollectionError::Ambiguous { slug, candidates } => ambiguous_slug_error(
            &slug,
            candidates
                .split(", ")
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        CollectionError::Sqlite(sqlite_err) => map_db_error(sqlite_err),
        other => invalid_params(other.to_string()),
    }
}

/// Map a rusqlite [`SqliteError`] onto an `rmcp::Error`. UNIQUE constraint
/// violations surface as `-32009` conflicts; FTS5 syntax errors surface as
/// `-32602` invalid-params; everything else is a generic `-32003` database
/// error.
pub fn map_db_error(e: SqliteError) -> rmcp::Error {
    if let SqliteError::SqliteFailure(ref err, ref msg) = e {
        // SQLITE_CONSTRAINT_UNIQUE (extended code 2067)
        if err.extended_code == 2067 {
            return rmcp::Error::new(
                ErrorCode(-32009),
                format!(
                    "conflict: {}",
                    msg.as_deref().unwrap_or("unique constraint violation")
                ),
                None,
            );
        }
        // FTS5 parse/syntax errors surface as SQLITE_ERROR with "fts5" in message
        if let Some(ref msg_str) = msg {
            if msg_str.contains("fts5") {
                return rmcp::Error::new(
                    ErrorCode(-32602),
                    format!("invalid search query: {msg_str}"),
                    None,
                );
            }
        }
    }
    rmcp::Error::new(ErrorCode(-32003), format!("database error: {e}"), None)
}

/// Map a [`SearchError`] onto an `rmcp::Error`.
pub fn map_search_error(e: SearchError) -> rmcp::Error {
    match e {
        SearchError::Sqlite(sqlite_err) => map_db_error(sqlite_err),
        SearchError::Ambiguous { slug, candidates } => ambiguous_slug_error(
            &slug,
            candidates
                .split(", ")
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        SearchError::Internal { message } => {
            rmcp::Error::new(ErrorCode(-32003), format!("search error: {message}"), None)
        }
    }
}

/// Serialise a value as pretty-printed JSON, mapping `serde_json` failures
/// onto `rmcp::Error` via [`map_anyhow_error`]. Emits `-32003`.
pub fn serialize_response<T: serde::Serialize>(value: &T) -> Result<String, rmcp::Error> {
    serde_json::to_string_pretty(value).map_err(|e| map_anyhow_error(anyhow::Error::from(e)))
}

/// Map a `serde_json::Error` directly onto an `rmcp::Error` carrying
/// `-32003`. Use this from tool bodies that previously constructed
/// `rmcp::Error::new(ErrorCode(-32003), e.to_string(), None)` ad-hoc when
/// serialising a response.
pub fn map_serialize_error(e: serde_json::Error) -> rmcp::Error {
    rmcp::Error::new(ErrorCode(-32003), e.to_string(), None)
}

/// Map a config-layer error (string-displayable) onto an `rmcp::Error`
/// carrying `-32002` with the canonical `ConfigError: {error}` prefix. Use
/// from tool helpers that read values out of `quaid_config`.
pub fn map_config_error(error: impl std::fmt::Display) -> rmcp::Error {
    rmcp::Error::new(ErrorCode(-32002), format!("ConfigError: {error}"), None)
}

/// Map an [`anyhow::Error`] onto an `rmcp::Error` by inspecting its message
/// for known sentinel substrings. Used by tool bodies that propagate errors
/// from the commands layer.
pub fn map_anyhow_error(e: anyhow::Error) -> rmcp::Error {
    let msg = e.to_string();
    if msg.contains("ConflictError")
        || msg.contains("ConcurrentRenameError")
        || msg.contains("SupersedeConflictError")
    {
        rmcp::Error::new(ErrorCode(-32009), msg, None)
    } else if msg.contains("page not found") || msg.contains("link not found") {
        rmcp::Error::new(ErrorCode(-32001), msg, None)
    } else if msg.contains("CollectionRestoringError")
        || msg.contains("ServeOwnsCollectionError")
        || msg.contains("Restore")
        || msg.contains("NewRoot")
        || msg.contains("ambiguous slug")
    {
        rmcp::Error::new(ErrorCode(-32002), msg, None)
    } else {
        rmcp::Error::new(ErrorCode(-32003), msg, None)
    }
}

/// Map a [`vault_sync::VaultSyncError`] onto an `rmcp::Error`.
pub fn map_vault_sync_error(e: vault_sync::VaultSyncError) -> rmcp::Error {
    if let vault_sync::VaultSyncError::AmbiguousSlug { slug, candidates } = &e {
        return ambiguous_slug_error(
            slug,
            candidates
                .split(", ")
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
    }
    let code = match e {
        vault_sync::VaultSyncError::PageNotFound { .. } => ErrorCode(-32001),
        vault_sync::VaultSyncError::AmbiguousSlug { .. }
        | vault_sync::VaultSyncError::CollectionRestoring { .. }
        | vault_sync::VaultSyncError::ServeOwnsCollectionError { .. }
        | vault_sync::VaultSyncError::Restore(vault_sync::RestoreError::RestoreInProgress {
            ..
        })
        | vault_sync::VaultSyncError::Restore(vault_sync::RestoreError::RestorePendingFinalize {
            ..
        })
        | vault_sync::VaultSyncError::Restore(
            vault_sync::RestoreError::RestoreIntegrityBlocked { .. },
        )
        | vault_sync::VaultSyncError::Restore(vault_sync::RestoreError::RestoreNonEmptyTarget {
            ..
        })
        | vault_sync::VaultSyncError::Restore(
            vault_sync::RestoreError::ServeDiedDuringHandshake { .. },
        )
        | vault_sync::VaultSyncError::Restore(vault_sync::RestoreError::HandshakeTimeout {
            ..
        })
        | vault_sync::VaultSyncError::Restore(
            vault_sync::RestoreError::NewRootVerificationFailed { .. },
        )
        | vault_sync::VaultSyncError::Restore(vault_sync::RestoreError::NewRootUnstable {
            ..
        })
        | vault_sync::VaultSyncError::ReconcileHalted { .. } => ErrorCode(-32002),
        #[cfg(unix)]
        vault_sync::VaultSyncError::Conflict(
            vault_sync::ConflictError::MissingExpectedVersion { .. },
        )
        | vault_sync::VaultSyncError::Conflict(vault_sync::ConflictError::StaleExpectedVersion {
            ..
        })
        | vault_sync::VaultSyncError::Conflict(vault_sync::ConflictError::ExternalDelete {
            ..
        })
        | vault_sync::VaultSyncError::Conflict(vault_sync::ConflictError::ExternalCreate {
            ..
        })
        | vault_sync::VaultSyncError::Conflict(vault_sync::ConflictError::HashMismatch {
            ..
        })
        | vault_sync::VaultSyncError::Conflict(vault_sync::ConflictError::ConcurrentRename {
            ..
        }) => ErrorCode(-32009),
        _ => ErrorCode(-32003),
    };
    rmcp::Error::new(code, e.to_string(), None)
}

/// Map a [`GapsError`] from the knowledge-gaps layer onto an `rmcp::Error`.
pub fn map_gaps_error(e: GapsError) -> rmcp::Error {
    match e {
        GapsError::Sqlite(sqlite_err) => map_db_error(sqlite_err),
        GapsError::NotFound { id } => rmcp::Error::new(
            ErrorCode(-32001),
            format!("gap not found: id {id}"),
            None,
        ),
    }
}

/// Map a [`GraphError`] onto an `rmcp::Error`.
pub fn map_graph_error(e: GraphError) -> rmcp::Error {
    match e {
        GraphError::PageNotFound { slug } => {
            rmcp::Error::new(ErrorCode(-32001), format!("page not found: {slug}"), None)
        }
        GraphError::Sqlite(sqlite_err) => map_db_error(sqlite_err),
    }
}

/// Map a [`namespace::NamespaceError`] onto an `rmcp::Error`.
pub fn map_namespace_error(e: namespace::NamespaceError) -> rmcp::Error {
    match e {
        namespace::NamespaceError::NotFound { id } => rmcp::Error::new(
            ErrorCode(-32001),
            format!("namespace not found: {id}"),
            None,
        ),
        namespace::NamespaceError::Sqlite(sqlite_err) => map_db_error(sqlite_err),
        other => invalid_params(other.to_string()),
    }
}

/// Map a [`turn_writer::TurnWriteError`] onto an `rmcp::Error`.
pub fn map_turn_write_error(e: turn_writer::TurnWriteError) -> rmcp::Error {
    match e {
        turn_writer::TurnWriteError::InvalidSessionId { message } => invalid_params(message),
        turn_writer::TurnWriteError::SessionClosed { session_id } => rmcp::Error::new(
            ErrorCode(-32009),
            format!("ConflictError: session `{session_id}` is already closed"),
            None,
        ),
        turn_writer::TurnWriteError::SessionNotFound { session_id } => rmcp::Error::new(
            ErrorCode(-32001),
            format!("NotFoundError: session `{session_id}` not found"),
            None,
        ),
        turn_writer::TurnWriteError::Config { message } => {
            rmcp::Error::new(ErrorCode(-32002), format!("ConfigError: {message}"), None)
        }
        turn_writer::TurnWriteError::Io(error) => {
            rmcp::Error::new(ErrorCode(-32002), format!("ConfigError: {error}"), None)
        }
        turn_writer::TurnWriteError::Sqlite(error) => map_db_error(error),
        turn_writer::TurnWriteError::Format(error) => rmcp::Error::new(
            ErrorCode(-32003),
            format!("conversation error: {error}"),
            None,
        ),
    }
}

/// Map a [`conversation_queue::ExtractionQueueError`] onto an `rmcp::Error`.
pub fn map_extraction_queue_error(
    e: conversation_queue::ExtractionQueueError,
) -> rmcp::Error {
    match e {
        conversation_queue::ExtractionQueueError::Sqlite(error) => map_db_error(error),
        conversation_queue::ExtractionQueueError::Config { message } => {
            rmcp::Error::new(ErrorCode(-32002), format!("ConfigError: {message}"), None)
        }
        conversation_queue::ExtractionQueueError::StaleLease { job_id, attempts } => {
            rmcp::Error::new(
                ErrorCode(-32009),
                format!(
                    "ConflictError: stale extraction lease for job {job_id} attempt {attempts}"
                ),
                None,
            )
        }
    }
}

/// Map a [`correction::CorrectionError`] onto an `rmcp::Error`.
pub fn map_correction_error(e: correction::CorrectionError) -> rmcp::Error {
    match e {
        correction::CorrectionError::NotFound { .. } => {
            rmcp::Error::new(ErrorCode(-32001), e.to_string(), None)
        }
        correction::CorrectionError::Kind { .. }
        | correction::CorrectionError::InvalidRequest { .. }
        | correction::CorrectionError::Config { .. } => {
            rmcp::Error::new(ErrorCode(-32002), e.to_string(), None)
        }
        correction::CorrectionError::Conflict { message } => {
            rmcp::Error::new(ErrorCode(-32009), message, None)
        }
        correction::CorrectionError::Sqlite(error) => map_db_error(error),
        correction::CorrectionError::VaultSync(error) => map_vault_sync_error(error),
        correction::CorrectionError::Json(_)
        | correction::CorrectionError::Slm(_)
        | correction::CorrectionError::FactResolution(_)
        | correction::CorrectionError::Output { .. } => {
            rmcp::Error::new(ErrorCode(-32003), e.to_string(), None)
        }
    }
}

/// Map an `anyhow::Error` from `memory_close_action`'s `put` flow onto an
/// `rmcp::Error`, normalising legacy "Conflict:" prefixes into "ConflictError:"
/// and tagging `-32009` with the page's current version.
pub fn map_close_action_put_error(
    db: &rusqlite::Connection,
    resolved: &vault_sync::ResolvedSlug,
    error: anyhow::Error,
) -> rmcp::Error {
    let message = error.to_string();
    if message.contains("Conflict:") || message.contains("ConflictError") {
        let current_version = resolved_page_version(db, resolved).ok().flatten();
        let normalized = if message.contains("Conflict: ") {
            message.replace("Conflict: ", "ConflictError: ")
        } else {
            message
        };
        rmcp::Error::new(
            ErrorCode(-32009),
            normalized,
            Some(serde_json::json!({ "current_version": current_version })),
        )
    } else {
        map_anyhow_error(error)
    }
}

fn resolved_page_version(
    db: &rusqlite::Connection,
    resolved: &vault_sync::ResolvedSlug,
) -> Result<Option<i64>, rmcp::Error> {
    use rusqlite::OptionalExtension;
    db.query_row(
        "SELECT version
         FROM pages
         WHERE collection_id = ?1 AND slug = ?2",
        rusqlite::params![resolved.collection_id, &resolved.slug],
        |row| row.get(0),
    )
    .optional()
    .map_err(map_db_error)
}
