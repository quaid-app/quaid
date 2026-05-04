use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::core::markdown;
use crate::core::page_uuid;

/// Export all pages as markdown files to the given output directory.
pub fn export_dir(db: &Connection, output_path: &Path) -> Result<usize> {
    let pages = all_pages(db)?;

    for page in &pages {
        let rendered = markdown::render_page(page);
        let file_path = output_path.join(format!("{}.md", page.slug));

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, &rendered)?;
    }

    Ok(pages.len())
}

/// Validate round-trip fidelity: export then re-parse and compare.
/// Used only in tests.
#[cfg(test)]
fn validate_roundtrip(db: &Connection, output_path: &Path) -> Result<()> {
    let pages = all_pages(db)?;

    for page in &pages {
        let rendered = markdown::render_page(page);
        let file_path = output_path.join(format!("{}.md", page.slug));

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, &rendered)?;

        let raw = fs::read_to_string(&file_path)?;
        let (frontmatter, body) = markdown::parse_frontmatter(&raw);
        let (truth, timeline) = markdown::split_content(&body);

        assert_eq!(
            truth, page.compiled_truth,
            "truth mismatch for {}",
            page.slug
        );
        assert_eq!(
            timeline, page.timeline,
            "timeline mismatch for {}",
            page.slug
        );

        for (key, value) in &frontmatter {
            if let Some(original) = page.frontmatter.get(key) {
                assert_eq!(
                    value, original,
                    "frontmatter key '{key}' mismatch for {}",
                    page.slug
                );
            }
        }
    }

    Ok(())
}

fn all_pages(db: &Connection) -> Result<Vec<crate::core::types::Page>> {
    let mut stmt = db.prepare(
        "SELECT slug, type, title, summary, compiled_truth, timeline, \
                uuid, frontmatter, wing, room, superseded_by, version, created_at, updated_at, \
                truth_updated_at, timeline_updated_at, id \
         FROM pages ORDER BY slug",
    )?;

    let rows = stmt.query_map([], |row| {
        let frontmatter_json: String = row.get(7)?;
        let frontmatter: HashMap<String, String> =
            serde_json::from_str(&frontmatter_json).unwrap_or_default();

        Ok((
            row.get::<_, i64>(16)?,
            crate::core::types::Page {
                slug: row.get(0)?,
                uuid: row.get::<_, Option<String>>(6)?.ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Null,
                        Box::new(page_uuid::PageUuidError::EmptyFrontmatterUuid),
                    )
                })?,
                page_type: row.get(1)?,
                superseded_by: row.get(10)?,
                title: row.get(2)?,
                summary: row.get(3)?,
                compiled_truth: row.get(4)?,
                timeline: row.get(5)?,
                frontmatter,
                wing: row.get(8)?,
                room: row.get(9)?,
                version: row.get(11)?,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
                truth_updated_at: row.get(14)?,
                timeline_updated_at: row.get(15)?,
            },
        ))
    })?;

    let mut pages = Vec::new();
    for row in rows {
        let (page_id, mut page) = row?;
        if !page.frontmatter.contains_key("supersedes") {
            if let Ok(predecessor_slug) = db.query_row(
                "SELECT slug FROM pages WHERE superseded_by = ?1 LIMIT 1",
                [page_id],
                |row| row.get(0),
            ) {
                page.frontmatter
                    .insert("supersedes".to_string(), predecessor_slug);
            }
        }
        pages.push(page);
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ingest;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    #[test]
    fn export_dir_creates_md_files() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES ('test/page', '01969f11-9448-7d79-8d3f-c68f54761234', 'concept', 'Test', '', 'Content.', '', \
                     '{\"title\":\"Test\",\"type\":\"concept\"}', 'test', '', 1)",
            [],
        )
        .unwrap();

        let dir = tempfile::TempDir::new().unwrap();
        let count = export_dir(&conn, dir.path()).unwrap();

        assert_eq!(count, 1);
        assert!(dir.path().join("test/page.md").exists());
    }

    #[test]
    fn validate_roundtrip_preserves_rendered_page_content() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES ('people/alice', '01969f11-9448-7d79-8d3f-c68f54761234', 'person', 'Alice', 'Summary', \
                     'Alice is an operator.', '- **2026-05** | note — Added fixture.', \
                     '{\"title\":\"Alice\",\"type\":\"person\",\"slug\":\"people/alice\"}', 'people', '', 1)",
            [],
        )
        .unwrap();

        let dir = tempfile::TempDir::new().unwrap();
        validate_roundtrip(&conn, dir.path()).unwrap();
    }

    #[test]
    fn export_roundtrip_preserves_supersede_frontmatter_and_chain() {
        let source = open_test_db();
        source
            .execute(
                "INSERT INTO pages
                     (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
                 VALUES
                     ('facts/a', '01969f11-9448-7d79-8d3f-c68f54761234', 'fact', 'A', '', 'A', '', '{\"title\":\"A\",\"type\":\"fact\",\"slug\":\"facts/a\"}', 'facts', '', 1),
                     ('facts/b', '01969f11-9448-7d79-8d3f-c68f54761235', 'fact', 'B', '', 'B', '', '{\"title\":\"B\",\"type\":\"fact\",\"slug\":\"facts/b\",\"supersedes\":\"facts/a\"}', 'facts', '', 1)",
                [],
            )
            .unwrap();
        let b_id: i64 = source
            .query_row("SELECT id FROM pages WHERE slug = 'facts/b'", [], |row| {
                row.get(0)
            })
            .unwrap();
        source
            .execute(
                "UPDATE pages SET superseded_by = ?1 WHERE slug = 'facts/a'",
                [b_id],
            )
            .unwrap();

        let export_root = tempfile::TempDir::new().unwrap();
        export_dir(&source, export_root.path()).unwrap();

        let exported =
            std::fs::read_to_string(export_root.path().join("facts").join("b.md")).unwrap();
        assert!(exported.contains("supersedes: facts/a"));

        let roundtrip = open_test_db();
        ingest::run(
            &roundtrip,
            export_root
                .path()
                .join("facts")
                .join("a.md")
                .to_str()
                .unwrap(),
            false,
        )
        .unwrap();
        ingest::run(
            &roundtrip,
            export_root
                .path()
                .join("facts")
                .join("b.md")
                .to_str()
                .unwrap(),
            false,
        )
        .unwrap();

        let roundtrip_b_id: i64 = roundtrip
            .query_row("SELECT id FROM pages WHERE slug = 'facts/b'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let superseded_by: Option<i64> = roundtrip
            .query_row(
                "SELECT superseded_by FROM pages WHERE slug = 'facts/a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(superseded_by, Some(roundtrip_b_id));
    }
}
