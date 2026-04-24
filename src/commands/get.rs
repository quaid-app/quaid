use std::collections::HashMap;

use anyhow::{bail, Result};
use rusqlite::Connection;

use crate::core::types::Page;
use crate::core::{markdown, page_uuid, vault_sync};

/// Read a page by slug and print it to stdout.
pub fn run(db: &Connection, slug: &str, json: bool) -> Result<()> {
    let resolved = vault_sync::resolve_page_for_read(db, slug)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let page = canonicalize_page_for_output(
        get_page_by_key(db, resolved.collection_id, &resolved.slug)?,
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
    let resolved = vault_sync::resolve_page_for_read(db, slug)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    get_page_by_key(db, resolved.collection_id, &resolved.slug)
}

pub fn get_page_by_key(db: &Connection, collection_id: i64, slug: &str) -> Result<Page> {
    let mut stmt = db.prepare(
        "SELECT slug, uuid, type, title, summary, compiled_truth, timeline, \
                frontmatter, wing, room, version, created_at, updated_at, \
                truth_updated_at, timeline_updated_at \
           FROM pages WHERE collection_id = ?1 AND slug = ?2",
    )?;

    let page = stmt.query_row(rusqlite::params![collection_id, slug], |row| {
        let frontmatter_json: String = row.get(7)?;
        let frontmatter: HashMap<String, String> =
            serde_json::from_str(&frontmatter_json).unwrap_or_default();

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
            title: row.get(3)?,
            summary: row.get(4)?,
            compiled_truth: row.get(5)?,
            timeline: row.get(6)?,
            frontmatter,
            wing: row.get(8)?,
            room: row.get(9)?,
            version: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            truth_updated_at: row.get(13)?,
            timeline_updated_at: row.get(14)?,
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
    page.slug = resolved.canonical_slug();
    page.frontmatter
        .insert("slug".to_string(), resolved.canonical_slug());
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
        let db_path = dir.path().join("test_brain.db");
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

        assert_eq!(page.frontmatter.get("title").unwrap(), "Carol");
        assert_eq!(page.frontmatter.get("type").unwrap(), "person");
    }
}
