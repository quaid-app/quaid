use std::fs;
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::core::{markdown, palace};

pub fn run(db: &Connection, path: &str, force: bool) -> Result<()> {
    let file = Path::new(path);
    let raw = fs::read_to_string(file)?;
    let hash = sha256_hex(raw.as_bytes());

    // Check ingest_log for existing ingestion
    ensure_ingest_table(db)?;
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
        "INSERT OR REPLACE INTO pages \
             (slug, type, title, summary, compiled_truth, timeline, \
              frontmatter, wing, room, version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, \
                 COALESCE((SELECT version + 1 FROM pages WHERE slug = ?1), 1))",
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

    record_ingest(db, &hash, path)?;
    println!("Ingested {slug}");

    Ok(())
}

fn ensure_ingest_table(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS import_hashes (\
             source_hash TEXT PRIMARY KEY, \
             source_path TEXT NOT NULL, \
             ingested_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))\
         )",
    )?;
    Ok(())
}

fn is_already_ingested(db: &Connection, hash: &str) -> Result<bool> {
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM import_hashes WHERE source_hash = ?1",
        [hash],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn record_ingest(db: &Connection, hash: &str, path: &str) -> Result<()> {
    db.execute(
        "INSERT OR REPLACE INTO import_hashes (source_hash, source_path) VALUES (?1, ?2)",
        rusqlite::params![hash, path],
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
}
