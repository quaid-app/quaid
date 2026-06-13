//! Namespace identifiers and lifecycle. Namespaces let agents partition pages
//! by session (e.g. an ephemeral planning scratch-space) without leaking into
//! the global vault; this module validates ids, manages the `namespaces`
//! metadata table, and provides destructive cleanup that cascades to pages.
//!
//! See also: `db` for the schema that backs these rows, and `search` /
//! `fts` for the namespace-aware filtering applied at query time.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use thiserror::Error;

/// Maximum namespace identifier length accepted by CLI and MCP surfaces.
pub const MAX_NAMESPACE_ID_LEN: usize = 128;

/// A namespace row persisted in the memory database.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Namespace {
    /// Namespace identifier (matches `[A-Za-z0-9_.-]{1,128}`).
    pub id: String,
    /// Optional TTL in hours used by janitor sweeps; `None` means no expiry.
    pub ttl_hours: Option<f64>,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
}

/// Errors returned by namespace management operations.
#[derive(Debug, Error)]
pub enum NamespaceError {
    /// Identifier was an empty string.
    #[error("invalid namespace: must not be empty")]
    Empty,

    /// Identifier exceeded [`MAX_NAMESPACE_ID_LEN`] bytes.
    #[error("invalid namespace: exceeds maximum length of {MAX_NAMESPACE_ID_LEN} characters")]
    TooLong,

    /// Identifier contained characters outside the `[A-Za-z0-9_.-]` allowlist.
    #[error("invalid namespace: allowed characters are [A-Za-z0-9_.-]")]
    InvalidCharacters,

    /// Namespace row did not exist for the requested id.
    #[error("namespace not found: {id}")]
    NotFound {
        /// The id that was not found.
        id: String,
    },

    /// Underlying SQLite failure.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Validate a non-global namespace identifier.
pub fn validate_namespace_id(id: &str) -> Result<(), NamespaceError> {
    if id.is_empty() {
        return Err(NamespaceError::Empty);
    }
    if id.len() > MAX_NAMESPACE_ID_LEN {
        return Err(NamespaceError::TooLong);
    }
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(NamespaceError::InvalidCharacters);
    }
    Ok(())
}

/// Validate an optional namespace filter or write target.
pub fn validate_optional_namespace(namespace: Option<&str>) -> Result<(), NamespaceError> {
    if let Some(id) = namespace.filter(|id| !id.is_empty()) {
        validate_namespace_id(id)?;
    }
    Ok(())
}

/// Create or update a namespace metadata row.
pub fn create_namespace(
    conn: &Connection,
    id: &str,
    ttl_hours: Option<f64>,
) -> Result<Namespace, NamespaceError> {
    validate_namespace_id(id)?;
    conn.execute(
        "INSERT INTO namespaces (id, ttl_hours)
         VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET ttl_hours = excluded.ttl_hours",
        params![id, ttl_hours],
    )?;
    get_namespace(conn, id)?.ok_or_else(|| NamespaceError::NotFound { id: id.to_owned() })
}

/// List known namespace metadata rows ordered by creation time.
pub fn list_namespaces(conn: &Connection) -> Result<Vec<Namespace>, NamespaceError> {
    let mut stmt = conn.prepare(
        "SELECT id, ttl_hours, created_at
         FROM namespaces
         ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Namespace {
            id: row.get(0)?,
            ttl_hours: row.get(1)?,
            created_at: row.get(2)?,
        })
    })?;

    let mut namespaces = Vec::new();
    for row in rows {
        namespaces.push(row?);
    }
    Ok(namespaces)
}

/// Delete a namespace and all pages assigned to it.
pub fn destroy_namespace(conn: &Connection, id: &str) -> Result<usize, NamespaceError> {
    validate_namespace_id(id)?;
    let tx = conn.unchecked_transaction()?;
    // Drop the backing vec0 rows first: they do not cascade with the page
    // delete, so without this they orphan permanently (review item #10).
    let page_ids = page_ids_in_namespace(&tx, id)?;
    crate::core::inference::delete_page_vec_rows(&tx, &page_ids).map_err(|err| {
        NamespaceError::Sqlite(rusqlite::Error::InvalidParameterName(err.to_string()))
    })?;
    let deleted_pages = tx.execute("DELETE FROM pages WHERE namespace = ?1", [id])?;
    let deleted_namespaces = tx.execute("DELETE FROM namespaces WHERE id = ?1", [id])?;
    tx.commit()?;

    if deleted_pages == 0 && deleted_namespaces == 0 {
        return Err(NamespaceError::NotFound { id: id.to_owned() });
    }
    Ok(deleted_pages)
}

fn page_ids_in_namespace(conn: &Connection, id: &str) -> Result<Vec<i64>, NamespaceError> {
    let mut stmt = conn.prepare("SELECT id FROM pages WHERE namespace = ?1")?;
    let rows = stmt.query_map([id], |row| row.get::<_, i64>(0))?;
    let mut page_ids = Vec::new();
    for row in rows {
        page_ids.push(row?);
    }
    Ok(page_ids)
}

fn get_namespace(conn: &Connection, id: &str) -> Result<Option<Namespace>, NamespaceError> {
    conn.query_row(
        "SELECT id, ttl_hours, created_at FROM namespaces WHERE id = ?1",
        [id],
        |row| {
            Ok(Namespace {
                id: row.get(0)?,
                ttl_hours: row.get(1)?,
                created_at: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(NamespaceError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    #[test]
    fn namespace_create_list_destroy_round_trips() {
        let conn = db::open(":memory:").expect("open db");

        create_namespace(&conn, "test-ns", Some(2.5)).expect("create namespace");
        let namespaces = list_namespaces(&conn).expect("list namespaces");

        assert_eq!(namespaces.len(), 1);
        assert_eq!(namespaces[0].id, "test-ns");
        assert_eq!(namespaces[0].ttl_hours, Some(2.5));

        let deleted = destroy_namespace(&conn, "test-ns").expect("destroy namespace");
        assert_eq!(deleted, 0);
    }

    #[test]
    fn validate_optional_namespace_allows_global_empty_namespace() {
        assert!(validate_optional_namespace(Some("")).is_ok());
    }

    #[test]
    fn validate_namespace_id_rejects_empty() {
        assert!(matches!(
            validate_namespace_id(""),
            Err(NamespaceError::Empty)
        ));
    }

    #[test]
    fn validate_namespace_id_rejects_too_long() {
        let long_id = "a".repeat(MAX_NAMESPACE_ID_LEN + 1);
        assert!(matches!(
            validate_namespace_id(&long_id),
            Err(NamespaceError::TooLong)
        ));
    }

    #[test]
    fn validate_namespace_id_rejects_invalid_characters() {
        assert!(matches!(
            validate_namespace_id("invalid namespace!"),
            Err(NamespaceError::InvalidCharacters)
        ));
    }

    #[test]
    fn validate_namespace_id_accepts_valid_chars() {
        assert!(validate_namespace_id("session-abc123").is_ok());
        assert!(validate_namespace_id("user.v2").is_ok());
        assert!(validate_namespace_id("agent_1").is_ok());
    }

    #[test]
    fn destroy_namespace_returns_not_found_when_missing() {
        let conn = db::open(":memory:").expect("open db");
        let err = destroy_namespace(&conn, "nonexistent").unwrap_err();
        assert!(matches!(err, NamespaceError::NotFound { .. }));
    }

    #[test]
    fn validate_optional_namespace_propagates_invalid_id_error() {
        assert!(validate_optional_namespace(Some("bad namespace!")).is_err());
    }

    #[test]
    fn create_namespace_without_ttl() {
        let conn = db::open(":memory:").expect("open db");
        let ns = create_namespace(&conn, "no-ttl", None).expect("create");
        assert_eq!(ns.id, "no-ttl");
        assert!(ns.ttl_hours.is_none());
    }
}
