//! Namespace-aware page identity resolution.
//!
//! The schema keys pages on `UNIQUE(collection_id, namespace, slug)`, but most
//! call sites historically resolved pages by `(collection_id, slug)` alone,
//! binding to an arbitrary row whenever the same slug existed in more than one
//! namespace (issue #212). This module is the single place where namespace
//! resolution semantics are decided; every production lookup that turns a
//! `(collection, slug)` pair into a page row id MUST go through
//! [`resolve`](crate::core::pages::resolve) or
//! [`resolve_optional`](crate::core::pages::resolve_optional).
//! A source-audit test (`tests/namespace_source_audit.rs`) enforces this.
//!
//! Resolution semantics, ported from the documented global-fallback behaviour
//! of `commands::get::get_page_by_key_with_namespace`:
//!
//! - `namespace: Some("")` — match the global (empty) namespace only.
//! - `namespace: Some(ns)` — prefer the page in `ns`, fall back to the global
//!   (`''`) namespace; pages in *other* namespaces never match.
//! - `namespace: None` — match any namespace, preferring the global namespace,
//!   then the lexicographically smallest namespace. This keeps legacy
//!   slug-only callers working while making multi-namespace resolution
//!   deterministic instead of row-order dependent.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use super::namespace;

/// Identity key for a page: collection, optional namespace filter, and slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageKey<'a> {
    /// Collection the page belongs to.
    pub collection_id: i64,
    /// Namespace filter. See the module docs for the exact semantics of
    /// `None`, `Some("")`, and `Some(ns)`.
    pub namespace: Option<&'a str>,
    /// Page slug within the collection.
    pub slug: &'a str,
}

/// Resolve a [`PageKey`] to its page row id.
///
/// Returns [`rusqlite::Error::QueryReturnedNoRows`] when no page matches, so
/// existing call sites can keep their `QueryReturnedNoRows` → "page not
/// found" mappings unchanged.
pub fn resolve(conn: &Connection, key: &PageKey) -> Result<i64, rusqlite::Error> {
    resolve_optional(conn, key)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

/// Resolve a [`PageKey`] to its page row id, returning `Ok(None)` when no
/// page matches.
pub fn resolve_optional(conn: &Connection, key: &PageKey) -> Result<Option<i64>, rusqlite::Error> {
    match key.namespace {
        Some("") => conn
            .query_row(
                "SELECT id FROM pages \
                 WHERE collection_id = ?1 AND slug = ?2 AND namespace = '' \
                 LIMIT 1",
                rusqlite::params![key.collection_id, key.slug],
                |row| row.get(0),
            )
            .optional(),
        Some(ns) => conn
            .query_row(
                "SELECT id FROM pages \
                 WHERE collection_id = ?1 AND slug = ?2 \
                   AND (namespace = ?3 OR namespace = '') \
                 ORDER BY CASE WHEN namespace = ?3 THEN 0 ELSE 1 END \
                 LIMIT 1",
                rusqlite::params![key.collection_id, key.slug, ns],
                |row| row.get(0),
            )
            .optional(),
        None => conn
            .query_row(
                "SELECT id FROM pages \
                 WHERE collection_id = ?1 AND slug = ?2 \
                 ORDER BY CASE WHEN namespace = '' THEN 0 ELSE 1 END, namespace \
                 LIMIT 1",
                rusqlite::params![key.collection_id, key.slug],
                |row| row.get(0),
            )
            .optional(),
    }
}

/// Look up the namespace of an already-resolved page row.
///
/// Used by derived-edge sync and other id-keyed paths that need to thread the
/// source page's namespace into target resolution.
pub fn page_namespace(conn: &Connection, page_id: i64) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT namespace FROM pages WHERE id = ?1",
        [page_id],
        |row| row.get(0),
    )
    .optional()
}

/// Derive the namespace encoded in a vault-relative file path.
///
/// The only established namespace-carrying path layout is the extraction
/// tree: `extracted/<kind>/...` lives in the global namespace while
/// `<ns>/extracted/<kind>/...` lives in namespace `<ns>` (see
/// `conversation::supersede::relative_fact_path` and
/// `conversation::file_edit::is_extracted_path`). Every other path shape maps
/// to the global (empty) namespace. Prefixes that fail namespace-id
/// validation are treated as plain directories, i.e. global namespace.
pub fn derive_namespace_from_relative_path(relative_path: &Path) -> String {
    let parts: Vec<String> = relative_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    match parts.as_slice() {
        [ns, second, ..]
            if second == "extracted" && namespace::validate_namespace_id(ns).is_ok() =>
        {
            ns.clone()
        }
        _ => String::new(),
    }
}
