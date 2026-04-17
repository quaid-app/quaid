use std::fs;
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::core::{markdown, novelty, palace};

pub fn run(db: &Connection, path: &str, force: bool) -> Result<()> {
    let file = Path::new(path);
    let raw_bytes = fs::read(file)?;
    let hash = sha256_hex(&raw_bytes);
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();

    // Check ingest_log for existing ingestion (uses canonical ingest_log table from schema.sql)
    if !force && is_already_ingested(db, &hash)? {
        println!("Already ingested (SHA-256 match), use --force to re-ingest");
        return Ok(());
    }

    let (frontmatter, body) = markdown::parse_frontmatter(&raw);
    let (compiled_truth, timeline) = markdown::split_content(&body);
    let summary = markdown::extract_summary(&compiled_truth);
    let slug = frontmatter.get("slug").cloned().unwrap_or_else(|| {
        file.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    });

    // Novelty check: skip near-duplicate content unless --force
    if !force {
        if let Ok(existing_page) = crate::commands::get::get_page(db, &slug) {
            match novelty::check_novelty(&compiled_truth, &existing_page, db) {
                Ok(false) => {
                    eprintln!("Skipping ingest: content not novel (slug: {slug})");
                    return Ok(());
                }
                Ok(true) => {} // novel content, proceed
                Err(e) => {
                    eprintln!("Warning: novelty check failed ({e}), proceeding with ingest");
                }
            }
        }
    }

    let wing = frontmatter
        .get("wing")
        .cloned()
        .unwrap_or_else(|| palace::derive_wing(&slug));
    let room = palace::derive_room(&compiled_truth);
    let title = frontmatter
        .get("title")
        .cloned()
        .unwrap_or_else(|| slug.clone());
    let page_type = frontmatter
        .get("type")
        .cloned()
        .unwrap_or_else(|| "concept".to_string());
    let frontmatter_json = serde_json::to_string(&frontmatter)?;

    db.execute(
        "INSERT INTO pages \
             (slug, type, title, summary, compiled_truth, timeline, \
              frontmatter, wing, room, version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1) \
         ON CONFLICT(slug) DO UPDATE SET \
             type = excluded.type, \
             title = excluded.title, \
             summary = excluded.summary, \
             compiled_truth = excluded.compiled_truth, \
             timeline = excluded.timeline, \
             frontmatter = excluded.frontmatter, \
             wing = excluded.wing, \
             room = excluded.room, \
             version = pages.version + 1, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        rusqlite::params![
            slug,
            page_type,
            title,
            summary,
            compiled_truth,
            timeline,
            frontmatter_json,
            wing,
            room
        ],
    )?;

    record_ingest(db, &hash, path, &slug)?;
    println!("Ingested {slug}");

    Ok(())
}

fn is_already_ingested(db: &Connection, hash: &str) -> Result<bool> {
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM ingest_log WHERE ingest_key = ?1",
        [hash],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn record_ingest(db: &Connection, hash: &str, path: &str, slug: &str) -> Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO ingest_log (ingest_key, source_type, source_ref, pages_updated) \
         VALUES (?1, 'file', ?2, json_array(?3))",
        rusqlite::params![hash, path, slug],
    )?;
    Ok(())
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    #[test]
    fn ingest_same_file_twice_without_force_skips_second() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        fs::write(
            &file_path,
            "---\ntitle: Test\ntype: concept\n---\nContent.\n",
        )
        .unwrap();

        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        let count_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .unwrap();

        // Second ingest — should be skipped
        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        let count_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .unwrap();

        assert_eq!(count_before, count_after);
    }

    #[test]
    fn ingest_with_force_re_ingests() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        fs::write(
            &file_path,
            "---\ntitle: Test\ntype: concept\n---\nContent.\n",
        )
        .unwrap();

        run(&conn, file_path.to_str().unwrap(), false).unwrap();
        // Re-ingest with force
        run(&conn, file_path.to_str().unwrap(), true).unwrap();

        let version: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE title = 'Test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn near_duplicate_content_is_skipped_by_novelty_check() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // First ingest
        let file_path = dir.path().join("note.md");
        fs::write(
            &file_path,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nAlice works at Acme and invests in climate software.\n",
        )
        .unwrap();
        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        // Second ingest with near-identical content (different file bytes → new ingest_key)
        let file_path2 = dir.path().join("note2.md");
        fs::write(
            &file_path2,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nAlice works at Acme and invests in climate software.\n",
        )
        .unwrap();
        run(&conn, file_path2.to_str().unwrap(), false).unwrap();

        // Version should still be 1 — novelty check prevented the upsert
        let version: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE slug = 'notes/test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn distinct_content_proceeds_past_novelty_check() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        let file_path = dir.path().join("note.md");
        fs::write(
            &file_path,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nAlice works at Acme and invests in climate software.\n",
        )
        .unwrap();
        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        let file_path2 = dir.path().join("note2.md");
        fs::write(
            &file_path2,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nBob teaches medieval history and collects rare maps.\n",
        )
        .unwrap();
        run(&conn, file_path2.to_str().unwrap(), false).unwrap();

        let version: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE slug = 'notes/test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn force_bypasses_novelty_check() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        let file_path = dir.path().join("note.md");
        fs::write(
            &file_path,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nAlice works at Acme and invests in climate software.\n",
        )
        .unwrap();
        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        // Re-ingest same content with --force
        let file_path2 = dir.path().join("note2.md");
        fs::write(
            &file_path2,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nAlice works at Acme and invests in climate software.\n",
        )
        .unwrap();
        run(&conn, file_path2.to_str().unwrap(), true).unwrap();

        let version: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE slug = 'notes/test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn first_time_ingest_skips_novelty_check() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        let file_path = dir.path().join("brand-new.md");
        fs::write(
            &file_path,
            "---\nslug: notes/brand-new\ntitle: Brand New\ntype: concept\n---\nCompletely new content.\n",
        )
        .unwrap();
        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE slug = 'notes/brand-new'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn ingest_records_resolved_frontmatter_slug_in_pages_updated() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("2024-01-meeting.md");
        fs::write(
            &file_path,
            "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n",
        )
        .unwrap();

        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        let pages_updated: String = conn
            .query_row(
                "SELECT pages_updated FROM ingest_log WHERE source_ref = ?1",
                [file_path.to_str().unwrap()],
                |row| row.get(0),
            )
            .expect("ingest_log row should exist");

        assert!(
            pages_updated.contains("people/alice"),
            "pages_updated should contain the resolved slug, got: {pages_updated}"
        );
        assert!(
            !pages_updated.contains("2024-01-meeting"),
            "pages_updated should not contain the filename stem, got: {pages_updated}"
        );
    }
}
