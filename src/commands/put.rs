use std::io::{self, Read};

use anyhow::{bail, Result};
use rusqlite::Connection;

use crate::core::{markdown, palace};

/// Read markdown from stdin, parse it, and insert or update a page.
///
/// OCC contract:
/// - New page (no row for `slug`): INSERT with `version = 1`.
/// - Existing page + `--expected-version N`: compare-and-swap UPDATE.
///   If stored version ≠ N → print conflict with current version, exit 1.
/// - Existing page without `--expected-version`: unconditional upsert
///   (bump version, overwrite content).
pub fn run(db: &Connection, slug: &str, expected_version: Option<i64>) -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    let (frontmatter, body) = markdown::parse_frontmatter(&input);
    let (compiled_truth, timeline) = markdown::split_content(&body);
    let summary = markdown::extract_summary(&compiled_truth);
    let wing = palace::derive_wing(slug);
    let room = palace::derive_room(&compiled_truth);

    let title = frontmatter
        .get("title")
        .cloned()
        .unwrap_or_else(|| slug.to_string());
    let page_type = frontmatter
        .get("type")
        .cloned()
        .unwrap_or_else(|| "concept".to_string());

    let frontmatter_json = serde_json::to_string(&frontmatter)?;

    let now = now_iso_from(db);
    let existing_version: Option<i64> = db
        .prepare("SELECT version FROM pages WHERE slug = ?1")?
        .query_row([slug], |row| row.get(0))
        .ok();

    match existing_version {
        None => {
            // New page — INSERT with version 1.
            db.execute(
                "INSERT INTO pages \
                     (slug, type, title, summary, compiled_truth, timeline, \
                      frontmatter, wing, room, version, \
                      created_at, updated_at, truth_updated_at, timeline_updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10, ?10, ?10)",
                rusqlite::params![
                    slug,
                    page_type,
                    title,
                    summary,
                    compiled_truth,
                    timeline,
                    frontmatter_json,
                    wing,
                    room,
                    now,
                ],
            )?;
            println!("Created {slug} (version 1)");
        }
        Some(current) => {
            if let Some(expected) = expected_version {
                // OCC compare-and-swap: only update if version matches.
                let rows = db.execute(
                    "UPDATE pages SET \
                         type = ?1, title = ?2, summary = ?3, \
                         compiled_truth = ?4, timeline = ?5, \
                         frontmatter = ?6, wing = ?7, room = ?8, \
                         version = version + 1, \
                         updated_at = ?9, truth_updated_at = ?9, timeline_updated_at = ?9 \
                     WHERE slug = ?10 AND version = ?11",
                    rusqlite::params![
                        page_type,
                        title,
                        summary,
                        compiled_truth,
                        timeline,
                        frontmatter_json,
                        wing,
                        room,
                        now,
                        slug,
                        expected,
                    ],
                )?;

                if rows == 0 {
                    bail!("Conflict: page updated elsewhere (current version: {current})");
                }

                println!("Updated {slug} (version {})", expected + 1);
            } else {
                // Unconditional upsert — no OCC check.
                db.execute(
                    "UPDATE pages SET \
                         type = ?1, title = ?2, summary = ?3, \
                         compiled_truth = ?4, timeline = ?5, \
                         frontmatter = ?6, wing = ?7, room = ?8, \
                         version = version + 1, \
                         updated_at = ?9, truth_updated_at = ?9, timeline_updated_at = ?9 \
                     WHERE slug = ?10",
                    rusqlite::params![
                        page_type,
                        title,
                        summary,
                        compiled_truth,
                        timeline,
                        frontmatter_json,
                        wing,
                        room,
                        now,
                        slug,
                    ],
                )?;

                println!("Updated {slug} (version {})", current + 1);
            }
        }
    }

    Ok(())
}

/// Get current UTC timestamp in ISO 8601 format from SQLite.
/// Keeps us dependency-free (no chrono) and consistent with schema defaults.
fn now_iso_from(db: &Connection) -> String {
    db.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })
    .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;
    use crate::core::markdown;
    use std::collections::HashMap;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    /// Helper: read a page back from the database.
    fn read_page(conn: &Connection, slug: &str) -> Option<(i64, String, String, String, String)> {
        conn.prepare(
            "SELECT version, type, title, compiled_truth, timeline FROM pages WHERE slug = ?1",
        )
        .unwrap()
        .query_row([slug], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .ok()
    }

    /// Helper: simulate `run` without reading from real stdin.
    fn put_from_string(
        db: &Connection,
        slug: &str,
        content: &str,
        expected_version: Option<i64>,
    ) -> Result<()> {
        let (frontmatter, body) = markdown::parse_frontmatter(content);
        let (compiled_truth, timeline) = markdown::split_content(&body);
        let summary = markdown::extract_summary(&compiled_truth);
        let wing = palace::derive_wing(slug);
        let room = palace::derive_room(&compiled_truth);

        let title = frontmatter
            .get("title")
            .cloned()
            .unwrap_or_else(|| slug.to_string());
        let page_type = frontmatter
            .get("type")
            .cloned()
            .unwrap_or_else(|| "concept".to_string());

        let frontmatter_json = serde_json::to_string(&frontmatter)?;
        let now = now_iso_from(db);

        let existing_version: Option<i64> = db
            .prepare("SELECT version FROM pages WHERE slug = ?1")?
            .query_row([slug], |row| row.get(0))
            .ok();

        match existing_version {
            None => {
                db.execute(
                    "INSERT INTO pages \
                         (slug, type, title, summary, compiled_truth, timeline, \
                          frontmatter, wing, room, version, \
                          created_at, updated_at, truth_updated_at, timeline_updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10, ?10, ?10)",
                    rusqlite::params![
                        slug,
                        page_type,
                        title,
                        summary,
                        compiled_truth,
                        timeline,
                        frontmatter_json,
                        wing,
                        room,
                        now,
                    ],
                )?;
            }
            Some(current) => {
                if let Some(expected) = expected_version {
                    let rows = db.execute(
                        "UPDATE pages SET \
                             type = ?1, title = ?2, summary = ?3, \
                             compiled_truth = ?4, timeline = ?5, \
                             frontmatter = ?6, wing = ?7, room = ?8, \
                             version = version + 1, \
                             updated_at = ?9, truth_updated_at = ?9, timeline_updated_at = ?9 \
                         WHERE slug = ?10 AND version = ?11",
                        rusqlite::params![
                            page_type,
                            title,
                            summary,
                            compiled_truth,
                            timeline,
                            frontmatter_json,
                            wing,
                            room,
                            now,
                            slug,
                            expected,
                        ],
                    )?;

                    if rows == 0 {
                        bail!("Conflict: page updated elsewhere (current version: {current})");
                    }
                } else {
                    db.execute(
                        "UPDATE pages SET \
                             type = ?1, title = ?2, summary = ?3, \
                             compiled_truth = ?4, timeline = ?5, \
                             frontmatter = ?6, wing = ?7, room = ?8, \
                             version = version + 1, \
                             updated_at = ?9, truth_updated_at = ?9, timeline_updated_at = ?9 \
                         WHERE slug = ?10",
                        rusqlite::params![
                            page_type,
                            title,
                            summary,
                            compiled_truth,
                            timeline,
                            frontmatter_json,
                            wing,
                            room,
                            now,
                            slug,
                        ],
                    )?;
                }
            }
        }

        Ok(())
    }

    // ── create ─────────────────────────────────────────────────

    #[test]
    fn create_page_sets_version_to_1() {
        let conn = open_test_db();
        let md = "---\ntitle: Alice\ntype: person\n---\n# Alice\n\nAlice is an operator.\n---\n2024-01-01: Joined Acme.\n";

        put_from_string(&conn, "people/alice", md, None).unwrap();

        let (version, page_type, title, truth, timeline) =
            read_page(&conn, "people/alice").unwrap();
        assert_eq!(version, 1);
        assert_eq!(page_type, "person");
        assert_eq!(title, "Alice");
        assert!(truth.contains("Alice is an operator"));
        assert!(timeline.contains("Joined Acme"));
    }

    #[test]
    fn create_page_derives_wing_from_slug() {
        let conn = open_test_db();
        let md = "---\ntitle: Alice\ntype: person\n---\nContent.\n";

        put_from_string(&conn, "people/alice-jones", md, None).unwrap();

        let wing: String = conn
            .query_row(
                "SELECT wing FROM pages WHERE slug = ?1",
                ["people/alice-jones"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(wing, "people");
    }

    #[test]
    fn create_page_defaults_type_to_concept_when_missing() {
        let conn = open_test_db();
        let md = "---\ntitle: Readme\n---\nJust a concept.\n";

        put_from_string(&conn, "readme", md, None).unwrap();

        let (_, page_type, _, _, _) = read_page(&conn, "readme").unwrap();
        assert_eq!(page_type, "concept");
    }

    // ── update with OCC ───────────────────────────────────────

    #[test]
    fn update_with_correct_expected_version_bumps_version() {
        let conn = open_test_db();
        let md1 = "---\ntitle: Alice\ntype: person\n---\nOriginal.\n";
        put_from_string(&conn, "people/alice", md1, None).unwrap();

        let md2 = "---\ntitle: Alice\ntype: person\n---\nUpdated.\n";
        put_from_string(&conn, "people/alice", md2, Some(1)).unwrap();

        let (version, _, _, truth, _) = read_page(&conn, "people/alice").unwrap();
        assert_eq!(version, 2);
        assert!(truth.contains("Updated"));
    }

    #[test]
    fn update_with_stale_expected_version_returns_conflict_error() {
        let conn = open_test_db();
        let md = "---\ntitle: Alice\ntype: person\n---\nContent.\n";
        put_from_string(&conn, "people/alice", md, None).unwrap();

        // Simulate a concurrent update by bumping version directly.
        conn.execute(
            "UPDATE pages SET version = 2, updated_at = '2099-01-01T00:00:00Z' WHERE slug = 'people/alice'",
            [],
        )
        .unwrap();

        let md2 = "---\ntitle: Alice\ntype: person\n---\nStale update.\n";
        let result = put_from_string(&conn, "people/alice", md2, Some(1));

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Conflict"));
        assert!(err.contains("current version: 2"));
    }

    // ── unconditional upsert ──────────────────────────────────

    #[test]
    fn update_without_expected_version_upserts_unconditionally() {
        let conn = open_test_db();
        let md1 = "---\ntitle: Bob\ntype: person\n---\nOriginal.\n";
        put_from_string(&conn, "people/bob", md1, None).unwrap();

        let md2 = "---\ntitle: Bob\ntype: person\n---\nOverwritten.\n";
        put_from_string(&conn, "people/bob", md2, None).unwrap();

        let (version, _, _, truth, _) = read_page(&conn, "people/bob").unwrap();
        assert_eq!(version, 2);
        assert!(truth.contains("Overwritten"));
    }

    // ── round-trip fidelity ───────────────────────────────────

    #[test]
    fn put_then_get_roundtrips_through_render() {
        let conn = open_test_db();
        let md = "---\ntitle: Carol\ntype: person\n---\n# Carol\n\nCarol builds things.\n---\n2024-06-01: Shipped v1.\n";

        put_from_string(&conn, "people/carol", md, None).unwrap();

        // Read back through get path
        let page = crate::commands::get::get_page(&conn, "people/carol").unwrap();
        let rendered = markdown::render_page(&page);

        assert_eq!(rendered, md);
    }

    // ── frontmatter stored as JSON ────────────────────────────

    #[test]
    fn frontmatter_is_stored_as_json_and_recoverable() {
        let conn = open_test_db();
        let md = "---\nsource: manual\ntitle: Data\ntype: concept\n---\nContent.\n";

        put_from_string(&conn, "data/test", md, None).unwrap();

        let fm_json: String = conn
            .query_row(
                "SELECT frontmatter FROM pages WHERE slug = ?1",
                ["data/test"],
                |row| row.get(0),
            )
            .unwrap();
        let fm: HashMap<String, String> = serde_json::from_str(&fm_json).unwrap();
        assert_eq!(fm.get("source").unwrap(), "manual");
        assert_eq!(fm.get("title").unwrap(), "Data");
        assert_eq!(fm.get("type").unwrap(), "concept");
    }

    // ── FTS5 trigger fires ────────────────────────────────────

    #[test]
    fn insert_triggers_fts5_indexing() {
        let conn = open_test_db();
        let md = "---\ntitle: Searchable\ntype: concept\n---\n# Searchable\n\nUnique searchable keyword xylophone.\n";

        put_from_string(&conn, "test/searchable", md, None).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM page_fts WHERE page_fts MATCH 'xylophone'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
