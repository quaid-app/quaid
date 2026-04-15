use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Result};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::core::markdown;
use crate::core::palace;

pub struct ImportStats {
    pub imported: usize,
    pub skipped: usize,
}

/// Import all `.md` files from a directory into the brain database.
///
/// SHA-256 of raw file bytes is the idempotency key. Files already ingested
/// (by hash) are skipped. When `validate_only` is true, files are parsed but
/// no database writes are performed.
pub fn import_dir(db: &Connection, dir: &Path, validate_only: bool) -> Result<ImportStats> {
    let md_files = collect_md_files(dir)?;

    if md_files.is_empty() {
        return Ok(ImportStats {
            imported: 0,
            skipped: 0,
        });
    }

    // Parse all files and collect errors
    let mut parsed = Vec::new();
    let mut errors = Vec::new();

    for file_path in &md_files {
        let raw = fs::read_to_string(file_path)?;
        let hash = sha256_hex(raw.as_bytes());

        match parse_file(&raw, file_path, dir) {
            Ok(entry) => parsed.push((file_path.clone(), hash, entry)),
            Err(e) => errors.push(format!("{}: {e}", file_path.display())),
        }
    }

    if !errors.is_empty() && validate_only {
        bail!("Validation errors:\n{}", errors.join("\n"));
    }

    if validate_only {
        return Ok(ImportStats {
            imported: parsed.len(),
            skipped: 0,
        });
    }

    // Check which hashes are already ingested
    ensure_ingest_log(db)?;
    let mut imported = 0;
    let mut skipped = 0;

    db.execute_batch("BEGIN")?;

    for (file_path, hash, entry) in &parsed {
        if is_already_ingested(db, hash)? {
            skipped += 1;
            continue;
        }

        insert_page(db, entry)?;
        record_ingest(db, hash, &file_path.to_string_lossy())?;
        imported += 1;
    }

    db.execute_batch("COMMIT")?;

    // Embed stale pages after commit
    if imported > 0 {
        let _ = crate::commands::embed::run(db, None, true, false);
    }

    Ok(ImportStats { imported, skipped })
}

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

/// Validate round-trip fidelity: export then re-import and compare.
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

        // Re-parse and compare
        let raw = fs::read_to_string(&file_path)?;
        let (fm, body) = markdown::parse_frontmatter(&raw);
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

        // Verify frontmatter round-trips
        for (k, v) in &fm {
            if let Some(original) = page.frontmatter.get(k) {
                assert_eq!(
                    v, original,
                    "frontmatter key '{k}' mismatch for {}",
                    page.slug
                );
            }
        }
    }

    Ok(())
}

// ── helpers ───────────────────────────────────────────────────

struct ParsedEntry {
    slug: String,
    title: String,
    page_type: String,
    summary: String,
    compiled_truth: String,
    timeline: String,
    frontmatter_json: String,
    wing: String,
    room: String,
}

fn parse_file(raw: &str, file_path: &Path, root: &Path) -> Result<ParsedEntry> {
    let (frontmatter, body) = markdown::parse_frontmatter(raw);
    let (compiled_truth, timeline) = markdown::split_content(&body);
    let summary = markdown::extract_summary(&compiled_truth);

    // Use frontmatter slug if present, else derive from path
    let slug = if let Some(s) = frontmatter.get("slug") {
        s.clone()
    } else {
        derive_slug_from_path(file_path, root)
    };

    let title = frontmatter
        .get("title")
        .cloned()
        .unwrap_or_else(|| slug.clone());
    let page_type = frontmatter
        .get("type")
        .cloned()
        .unwrap_or_else(|| "concept".to_string());
    let wing = frontmatter
        .get("wing")
        .cloned()
        .unwrap_or_else(|| palace::derive_wing(&slug));
    let room = palace::derive_room(&compiled_truth);
    let frontmatter_json = serde_json::to_string(&frontmatter)?;

    Ok(ParsedEntry {
        slug,
        title,
        page_type,
        summary,
        compiled_truth,
        timeline,
        frontmatter_json,
        wing,
        room,
    })
}

fn derive_slug_from_path(file_path: &Path, root: &Path) -> String {
    let relative = file_path.strip_prefix(root).unwrap_or(file_path);
    let slug = relative
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");
    slug
}

fn insert_page(db: &Connection, entry: &ParsedEntry) -> Result<()> {
    db.execute(
        "INSERT OR REPLACE INTO pages \
             (slug, type, title, summary, compiled_truth, timeline, \
              frontmatter, wing, room, version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, \
                 COALESCE((SELECT version + 1 FROM pages WHERE slug = ?1), 1))",
        rusqlite::params![
            entry.slug,
            entry.page_type,
            entry.title,
            entry.summary,
            entry.compiled_truth,
            entry.timeline,
            entry.frontmatter_json,
            entry.wing,
            entry.room,
        ],
    )?;
    Ok(())
}

fn ensure_ingest_log(db: &Connection) -> Result<()> {
    // The ingest_log table is defined in schema.sql, but we check with a different
    // structure here for the file-based import hash tracking.
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
        "INSERT OR IGNORE INTO import_hashes (source_hash, source_path) VALUES (?1, ?2)",
        rusqlite::params![hash, path],
    )?;
    Ok(())
}

fn collect_md_files(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    if !dir.exists() {
        bail!("directory not found: {}", dir.display());
    }
    let mut files = Vec::new();
    collect_md_recursive(dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_md_recursive(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_md_recursive(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            files.push(path);
        }
    }
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

fn all_pages(db: &Connection) -> Result<Vec<crate::core::types::Page>> {
    let mut stmt = db.prepare(
        "SELECT slug, type, title, summary, compiled_truth, timeline, \
                frontmatter, wing, room, version, created_at, updated_at, \
                truth_updated_at, timeline_updated_at \
         FROM pages ORDER BY slug",
    )?;

    let rows = stmt.query_map([], |row| {
        let frontmatter_json: String = row.get(6)?;
        let frontmatter: HashMap<String, String> =
            serde_json::from_str(&frontmatter_json).unwrap_or_default();

        Ok(crate::core::types::Page {
            slug: row.get(0)?,
            page_type: row.get(1)?,
            title: row.get(2)?,
            summary: row.get(3)?,
            compiled_truth: row.get(4)?,
            timeline: row.get(5)?,
            frontmatter,
            wing: row.get(7)?,
            room: row.get(8)?,
            version: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
            truth_updated_at: row.get(12)?,
            timeline_updated_at: row.get(13)?,
        })
    })?;

    let mut pages = Vec::new();
    for row in rows {
        pages.push(row?);
    }
    Ok(pages)
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
    fn import_dir_imports_md_files() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        fs::write(
            &file_path,
            "---\ntitle: Test\ntype: concept\n---\nTest content.\n",
        )
        .unwrap();

        let stats = import_dir(&conn, dir.path(), false).unwrap();

        assert_eq!(stats.imported, 1);
        assert_eq!(stats.skipped, 0);
    }

    #[test]
    fn import_dir_skips_already_imported_files() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        fs::write(
            &file_path,
            "---\ntitle: Test\ntype: concept\n---\nContent.\n",
        )
        .unwrap();

        import_dir(&conn, dir.path(), false).unwrap();
        let stats = import_dir(&conn, dir.path(), false).unwrap();

        assert_eq!(stats.imported, 0);
        assert_eq!(stats.skipped, 1);
    }

    #[test]
    fn export_dir_creates_md_files() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES ('test/page', 'concept', 'Test', '', 'Content.', '', \
                     '{\"title\":\"Test\",\"type\":\"concept\"}', 'test', '', 1)",
            [],
        )
        .unwrap();

        let dir = tempfile::TempDir::new().unwrap();
        let count = export_dir(&conn, dir.path()).unwrap();

        assert_eq!(count, 1);
        let exported = dir.path().join("test/page.md");
        assert!(exported.exists());
    }

    #[test]
    fn uses_frontmatter_slug_when_present() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        fs::write(
            &file_path,
            "---\nslug: custom/slug\ntitle: Custom\ntype: concept\n---\nContent.\n",
        )
        .unwrap();

        import_dir(&conn, dir.path(), false).unwrap();

        let slug: String = conn
            .query_row(
                "SELECT slug FROM pages WHERE slug = 'custom/slug'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(slug, "custom/slug");
    }

    #[test]
    fn export_then_reimport_roundtrips() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES ('test/page', 'concept', 'Test', 'Summary', 'Content.', 'Timeline.', \
                     '{\"title\":\"Test\",\"type\":\"concept\"}', 'test', '', 1)",
            [],
        )
        .unwrap();

        let dir = tempfile::TempDir::new().unwrap();
        export_dir(&conn, dir.path()).unwrap();
        validate_roundtrip(&conn, dir.path()).unwrap();
    }
}
