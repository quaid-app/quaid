use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

/// A single row in the list output.
#[derive(Debug, Serialize)]
struct ListEntry {
    slug: String,
    #[serde(rename = "type")]
    page_type: String,
    summary: String,
}

/// List pages with optional wing/type filters, ordered by updated_at DESC.
pub fn run(
    db: &Connection,
    wing: Option<String>,
    page_type: Option<String>,
    namespace: Option<&str>,
    limit: u32,
    json: bool,
) -> Result<()> {
    crate::core::namespace::validate_optional_namespace(namespace)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let entries = list_pages(db, wing.as_deref(), page_type.as_deref(), namespace, limit)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        if entries.is_empty() {
            println!("No pages found.");
        } else {
            for e in &entries {
                println!("{}\t{}\t{}", e.slug, e.page_type, e.summary);
            }
        }
    }

    Ok(())
}

/// Query the pages table with optional filters.
fn list_pages(
    db: &Connection,
    wing: Option<&str>,
    page_type: Option<&str>,
    namespace: Option<&str>,
    limit: u32,
) -> Result<Vec<ListEntry>> {
    let mut sql = String::from(
        "SELECT c.name || '::' || p.slug, p.type, p.summary
         FROM pages p
         JOIN collections c ON c.id = p.collection_id
         WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(w) = wing {
        sql.push_str(" AND p.wing = ?");
        params.push(Box::new(w.to_owned()));
    }
    if let Some(t) = page_type {
        sql.push_str(" AND p.type = ?");
        params.push(Box::new(t.to_owned()));
    }
    if let Some(ns) = namespace {
        if ns.is_empty() {
            sql.push_str(" AND p.namespace = ?");
            params.push(Box::new(String::new()));
        } else {
            sql.push_str(" AND (p.namespace = ? OR p.namespace = '')");
            params.push(Box::new(ns.to_owned()));
        }
    }

    sql.push_str(" ORDER BY p.updated_at DESC LIMIT ?");
    params.push(Box::new(limit));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(ListEntry {
            slug: row.get(0)?,
            page_type: row.get(1)?,
            summary: row.get(2)?,
        })
    })?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    fn insert_page(conn: &Connection, slug: &str, page_type: &str, wing: &str, summary: &str) {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, ?2, ?3, ?4, '', '', '{}', ?5, '', 1)",
            rusqlite::params![slug, page_type, slug, summary, wing],
        )
        .unwrap();
    }

    #[test]
    fn list_returns_all_pages_when_no_filters() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "people", "Alice summary");
        insert_page(
            &conn,
            "companies/acme",
            "company",
            "companies",
            "Acme summary",
        );

        let entries = list_pages(&conn, None, None, None, 50).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn list_filters_by_wing() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "people", "Alice summary");
        insert_page(
            &conn,
            "companies/acme",
            "company",
            "companies",
            "Acme summary",
        );

        let entries = list_pages(&conn, Some("people"), None, None, 50).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].slug, "default::people/alice");
    }

    #[test]
    fn list_filters_by_type() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "people", "Alice summary");
        insert_page(
            &conn,
            "concepts/rust",
            "concept",
            "concepts",
            "Rust summary",
        );

        let entries = list_pages(&conn, None, Some("concept"), None, 50).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].slug, "default::concepts/rust");
    }

    #[test]
    fn list_respects_limit() {
        let conn = open_test_db();
        for i in 0..5 {
            insert_page(
                &conn,
                &format!("test/page-{i}"),
                "concept",
                "test",
                &format!("Page {i}"),
            );
        }

        let entries = list_pages(&conn, None, None, None, 3).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn list_combines_wing_and_type_filters() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "people", "Alice");
        insert_page(&conn, "people/meeting", "concept", "people", "Meeting");
        insert_page(&conn, "companies/acme", "company", "companies", "Acme");

        let entries = list_pages(&conn, Some("people"), Some("person"), None, 50).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].slug, "default::people/alice");
    }

    #[test]
    fn list_returns_empty_vec_on_empty_database() {
        let conn = open_test_db();
        let entries = list_pages(&conn, None, None, None, 50).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn list_orders_by_updated_at_descending() {
        let conn = open_test_db();
        // Insert with explicit updated_at to control order
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version, updated_at) \
             VALUES ('test/old', 'concept', 'Old', 'Old page', '', '', '{}', 'test', '', 1, '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version, updated_at) \
             VALUES ('test/new', 'concept', 'New', 'New page', '', '', '{}', 'test', '', 1, '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let entries = list_pages(&conn, None, None, None, 50).unwrap();
        assert_eq!(entries[0].slug, "default::test/new");
        assert_eq!(entries[1].slug, "default::test/old");
    }
}
