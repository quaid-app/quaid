use anyhow::{bail, Result};
use rusqlite::Connection;

use crate::core::types::{frontmatter_insert_string, Frontmatter, Page};
use crate::core::{markdown, page_uuid, vault_sync};

/// Read a page by slug and print it to stdout.
pub fn run(db: &Connection, slug: &str, namespace: Option<&str>, json: bool) -> Result<()> {
    crate::core::namespace::validate_optional_namespace(namespace)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let namespace = namespace.or(Some(""));
    let resolved = vault_sync::resolve_page_for_read(db, slug)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let page = canonicalize_page_for_output(
        get_page_by_key_with_namespace(db, resolved.collection_id, &resolved.slug, namespace)?,
        &resolved,
    );

    if json {
        println!("{}", serde_json::to_string_pretty(&page)?);
    } else {
        print!("{}", markdown::render_page(&page));
    }

    Ok(())
}

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

fn canonicalize_page_for_output(page: Page, resolved: &vault_sync::ResolvedSlug) -> Page {
    let mut page = page;
    page_uuid::canonicalize_frontmatter_uuid(&mut page.frontmatter, &page.uuid);
    page.slug = resolved.canonical_slug();
    frontmatter_insert_string(&mut page.frontmatter, "slug", resolved.canonical_slug());
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;
    use crate::core::page_uuid;

    /// Insert a test page directly into the database.
    fn insert_test_page(conn: &Connection, slug: &str, title: &str, truth: &str, timeline: &str) {
        let frontmatter = serde_json::json!({
            "title": title,
            "type": "person"
        });
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                slug,
                page_uuid::generate_uuid_v7(),
                "person",
                title,
                "Test summary",
                truth,
                timeline,
                frontmatter.to_string(),
                "people",
                "",
                1,
            ],
        )
        .unwrap();
    }

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        // Leak the TempDir so it lives long enough (test-only)
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    #[test]
    fn get_page_returns_page_matching_inserted_data() {
        let conn = open_test_db();
        insert_test_page(
            &conn,
            "people/alice",
            "Alice",
            "# Alice\n\nAlice is an operator.",
            "2024-01-01: Joined Acme.",
        );

        let page = get_page(&conn, "people/alice").unwrap();

        assert_eq!(page.slug, "people/alice");
        assert_eq!(page.title, "Alice");
        assert_eq!(page.page_type, "person");
        assert_eq!(page.compiled_truth, "# Alice\n\nAlice is an operator.");
        assert_eq!(page.timeline, "2024-01-01: Joined Acme.");
        assert_eq!(page.wing, "people");
        assert_eq!(page.version, 1);
    }

    #[test]
    fn get_page_renders_back_to_matching_markdown() {
        let conn = open_test_db();
        insert_test_page(
            &conn,
            "people/bob",
            "Bob",
            "# Bob\n\nBob builds things.",
            "2024-06-01: Shipped v1.",
        );

        let page = get_page(&conn, "people/bob").unwrap();
        let rendered = markdown::render_page(&page);

        // Rendered output should contain the frontmatter, truth, and timeline
        assert!(rendered.contains("title: Bob"));
        assert!(rendered.contains("type: person"));
        assert!(rendered.contains("# Bob\n\nBob builds things."));
        assert!(rendered.contains("2024-06-01: Shipped v1."));
    }

    #[test]
    fn get_page_returns_error_for_nonexistent_slug() {
        let conn = open_test_db();

        let result = get_page(&conn, "people/nobody");
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("page not found"));
        assert!(err_msg.contains("nobody"));
    }

    #[test]
    fn get_page_deserializes_frontmatter_from_json() {
        let conn = open_test_db();
        insert_test_page(&conn, "people/carol", "Carol", "Content.", "");

        let page = get_page(&conn, "people/carol").unwrap();

        assert_eq!(
            page.frontmatter.get("title"),
            Some(&serde_json::json!("Carol"))
        );
        assert_eq!(
            page.frontmatter.get("type"),
            Some(&serde_json::json!("person"))
        );
    }

    #[test]
    fn canonicalize_page_for_output_restores_quaid_id_frontmatter() {
        let page = Page {
            slug: "people/alice".to_string(),
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234".to_string(),
            page_type: "person".to_string(),
            superseded_by: None,
            title: "Alice".to_string(),
            summary: "summary".to_string(),
            compiled_truth: "truth".to_string(),
            timeline: String::new(),
            frontmatter: crate::core::types::string_frontmatter([
                ("memory_id".to_string(), "legacy".to_string()),
                ("title".to_string(), "Alice".to_string()),
            ]),
            wing: "people".to_string(),
            room: String::new(),
            version: 1,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            truth_updated_at: "2026-01-01T00:00:00Z".to_string(),
            timeline_updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let resolved = vault_sync::ResolvedSlug {
            collection_id: 1,
            collection_name: "default".to_string(),
            slug: "people/alice".to_string(),
        };

        let page = canonicalize_page_for_output(page, &resolved);

        assert_eq!(
            page.frontmatter.get("quaid_id"),
            Some(&serde_json::json!("01969f11-9448-7d79-8d3f-c68f54761234"))
        );
        assert!(!page.frontmatter.contains_key("memory_id"));
        assert_eq!(
            page.frontmatter.get("slug"),
            Some(&serde_json::json!("default::people/alice"))
        );
    }
}
