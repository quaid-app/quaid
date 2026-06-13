//! Page-record read helpers shared by the CLI commands, the MCP tools, and the
//! core subsystems (reconciler, vault sync, embedding).
//!
//! These functions are pure SQLite readers: given a connection they resolve a
//! slug (honoring vault-sync redirects) and load the corresponding
//! [`crate::core::types::Page`]. They live in `core` so that library code never
//! has to reach up into `crate::commands` to read a page; `crate::commands::get`
//! re-exports them for the CLI surface and for backward compatibility.
//!
//! See also: `crate::core::vault_sync` for slug resolution and
//! `crate::core::types::Page` for the row shape.

use anyhow::{bail, Result};
use rusqlite::Connection;

use crate::core::page_uuid;
use crate::core::types::{Frontmatter, Page};
use crate::core::vault_sync;

/// Load a single page from the database by slug.
pub fn get_page(db: &Connection, slug: &str) -> Result<Page> {
    get_page_with_namespace(db, slug, None)
}

/// Load a single page from the database by slug with an optional namespace filter.
pub fn get_page_with_namespace(
    db: &Connection,
    slug: &str,
    namespace: Option<&str>,
) -> Result<Page> {
    let resolved = vault_sync::resolve_page_for_read(db, slug)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    get_page_by_key_with_namespace(db, resolved.collection_id, &resolved.slug, namespace)
}

/// Load a single page from the database by collection and slug.
pub fn get_page_by_key(db: &Connection, collection_id: i64, slug: &str) -> Result<Page> {
    get_page_by_key_with_namespace(db, collection_id, slug, None)
}

/// Load a single page from the database by collection, slug, and namespace filter.
pub fn get_page_by_key_with_namespace(
    db: &Connection,
    collection_id: i64,
    slug: &str,
    namespace: Option<&str>,
) -> Result<Page> {
    crate::core::namespace::validate_optional_namespace(namespace)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let mut sql = String::from(
        "SELECT slug, uuid, type, title, summary, compiled_truth, timeline, \
                frontmatter, wing, room, superseded_by, version, created_at, updated_at, \
                truth_updated_at, timeline_updated_at \
           FROM pages WHERE collection_id = ?1 AND slug = ?2",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(collection_id), Box::new(slug.to_owned())];

    if let Some(namespace) = namespace {
        if namespace.is_empty() {
            sql.push_str(" AND namespace = ?");
            sql.push_str(&(params.len() + 1).to_string());
            params.push(Box::new(String::new()));
        } else {
            sql.push_str(" AND (namespace = ?");
            sql.push_str(&(params.len() + 1).to_string());
            sql.push_str(" OR namespace = '')");
            params.push(Box::new(namespace.to_owned()));
        }
    }
    if let Some(namespace) = namespace.filter(|namespace| !namespace.is_empty()) {
        sql.push_str(" ORDER BY CASE WHEN namespace = ?");
        sql.push_str(&(params.len() + 1).to_string());
        sql.push_str(" THEN 0 ELSE 1 END");
        params.push(Box::new(namespace.to_owned()));
    }
    sql.push_str(" LIMIT 1");

    let mut stmt = db.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let page = stmt.query_row(param_refs.as_slice(), |row| {
        let frontmatter_json: String = row.get(7)?;
        let frontmatter: Frontmatter = serde_json::from_str(&frontmatter_json).unwrap_or_default();

        Ok(Page {
            slug: row.get(0)?,
            uuid: row.get::<_, Option<String>>(1)?.ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Null,
                    Box::new(page_uuid::PageUuidError::EmptyFrontmatterUuid),
                )
            })?,
            page_type: row.get(2)?,
            superseded_by: row.get(10)?,
            title: row.get(3)?,
            summary: row.get(4)?,
            compiled_truth: row.get(5)?,
            timeline: row.get(6)?,
            frontmatter,
            wing: row.get(8)?,
            room: row.get(9)?,
            version: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
            truth_updated_at: row.get(14)?,
            timeline_updated_at: row.get(15)?,
        })
    });

    match page {
        Ok(page) => Ok(page),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            bail!("page not found: {slug}")
        }
        Err(e) => Err(e.into()),
    }
}
