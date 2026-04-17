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
    /// Files skipped because they were already ingested (same SHA-256).
    pub skipped_already_ingested: usize,
    /// Files skipped because they are not Markdown (`.md`).
    pub skipped_non_markdown: usize,
}

impl ImportStats {
    pub fn total_skipped(&self) -> usize {
        self.skipped_already_ingested + self.skipped_non_markdown
    }
}

/// Import all `.md` files from a directory into the brain database.
///
/// SHA-256 of raw file bytes is the idempotency key. Files already ingested
/// (by hash) are skipped. When `validate_only` is true, files are parsed but
/// no database writes are performed.
pub fn import_dir(db: &Connection, dir: &Path, validate_only: bool) -> Result<ImportStats> {
    let (md_files, non_markdown_count) = collect_files(dir)?;

    if md_files.is_empty() {
        return Ok(ImportStats {
            imported: 0,
            skipped_already_ingested: 0,
            skipped_non_markdown: non_markdown_count,
        });
    }

    // Parse all files and collect errors
    let mut parsed = Vec::new();
    let mut errors = Vec::new();

    for file_path in &md_files {
        let raw_bytes = fs::read(file_path)?;
        let hash = sha256_hex(&raw_bytes);
        let raw = String::from_utf8_lossy(&raw_bytes).into_owned();

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
            skipped_already_ingested: 0,
            skipped_non_markdown: non_markdown_count,
        });
    }

    // Check which hashes are already ingested
    let mut imported = 0;
    let mut skipped_already_ingested = 0;

    let tx = db.unchecked_transaction()?;

    for (file_path, hash, entry) in &parsed {
        if is_already_ingested(&tx, hash)? {
            skipped_already_ingested += 1;
            continue;
        }

        insert_page(&tx, entry)?;
        record_ingest(&tx, hash, &file_path.to_string_lossy())?;
        imported += 1;
    }

    tx.commit()?;

    // Embed pages after commit. In batch mode embed::run warns per-page on
    // failure and returns Ok — only setup-level errors (e.g. missing active
    // model) propagate here.
    if imported > 0 {
        if let Err(e) = crate::commands::embed::run(db, None, true, false) {
            eprintln!("warning: embedding failed after import: {e}");
        }
    }

    Ok(ImportStats {
        imported,
        skipped_already_ingested,
        skipped_non_markdown: non_markdown_count,
    })
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

fn is_already_ingested(db: &Connection, hash: &str) -> Result<bool> {
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM ingest_log WHERE ingest_key = ?1",
        [hash],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn record_ingest(db: &Connection, hash: &str, path: &str) -> Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO ingest_log (ingest_key, source_type, source_ref) \
         VALUES (?1, 'file', ?2)",
        rusqlite::params![hash, path],
    )?;
    Ok(())
}

/// Returns `(md_files, non_markdown_count)`.
fn collect_files(dir: &Path) -> Result<(Vec<std::path::PathBuf>, usize)> {
    if !dir.exists() {
        bail!("directory not found: {}", dir.display());
    }
    let mut md_files = Vec::new();
    let mut non_markdown_count = 0usize;
    collect_files_recursive(dir, &mut md_files, &mut non_markdown_count)?;
    md_files.sort();
    Ok((md_files, non_markdown_count))
}

fn collect_files_recursive(
    dir: &Path,
    md_files: &mut Vec<std::path::PathBuf>,
    non_markdown_count: &mut usize,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, md_files, non_markdown_count)?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            md_files.push(path);
        } else {
            *non_markdown_count += 1;
        }
    }
    Ok(())
}

// Keep the old name available for internal test helpers that call it directly.
#[cfg(test)]
fn collect_md_files(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    collect_files(dir).map(|(files, _)| files)
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
    use std::path::Path;

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
        assert_eq!(stats.skipped_already_ingested, 0);
        assert_eq!(stats.skipped_non_markdown, 0);
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
        assert_eq!(stats.skipped_already_ingested, 1);
        assert_eq!(stats.skipped_non_markdown, 0);
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

    #[test]
    fn import_dir_reimports_only_fixture_with_new_sha_after_content_change() {
        fn copy_fixture_tree(from: &Path, to: &Path) {
            for file in collect_md_files(from).unwrap() {
                let relative = file.strip_prefix(from).unwrap();
                let destination = to.join(relative);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::copy(&file, destination).unwrap();
            }
        }

        let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let fixture_count = collect_md_files(&fixtures_dir).unwrap().len();
        let corpus_dir = tempfile::TempDir::new().unwrap();
        copy_fixture_tree(&fixtures_dir, corpus_dir.path());

        let db_dir = tempfile::TempDir::new().unwrap();
        let db_path = db_dir.path().join("test_brain.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();

        let initial_stats = import_dir(&conn, corpus_dir.path(), false).unwrap();
        assert_eq!(initial_stats.imported, fixture_count);
        assert_eq!(initial_stats.skipped_already_ingested, 0);
        assert_eq!(initial_stats.skipped_non_markdown, 0);

        let modified_fixture = corpus_dir.path().join("person.md");
        let original = fs::read_to_string(&modified_fixture).unwrap();
        let original_hash = sha256_hex(original.as_bytes());
        let updated = original.replace(
            "Brazilian entrepreneur who moved to the US to build at scale.",
            "Brazilian entrepreneur who moved to the US to build fintech infrastructure at scale.",
        );
        let updated_hash = sha256_hex(updated.as_bytes());
        assert_ne!(original_hash, updated_hash);
        fs::write(&modified_fixture, updated).unwrap();

        let reimport_stats = import_dir(&conn, corpus_dir.path(), false).unwrap();
        assert_eq!(reimport_stats.imported, 1);
        assert_eq!(reimport_stats.skipped_already_ingested, fixture_count - 1);
        assert_eq!(reimport_stats.skipped_non_markdown, 0);
    }

    #[test]
    fn import_dir_counts_non_markdown_files_as_skipped_non_markdown() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        fs::write(
            dir.path().join("note.md"),
            "---\ntitle: Note\ntype: concept\n---\nContent.\n",
        )
        .unwrap();
        fs::write(dir.path().join("config.json"), r#"{"key":"value"}"#).unwrap();
        fs::write(dir.path().join("README.txt"), "Plain text readme").unwrap();

        let stats = import_dir(&conn, dir.path(), false).unwrap();

        assert_eq!(stats.imported, 1);
        assert_eq!(stats.skipped_already_ingested, 0);
        assert_eq!(stats.skipped_non_markdown, 2);
        assert_eq!(stats.total_skipped(), 2);
    }

    #[test]
    fn import_dir_mixed_skips_show_both_reason_counts() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        fs::write(
            dir.path().join("a.md"),
            "---\ntitle: A\ntype: concept\n---\nContent A.\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.md"),
            "---\ntitle: B\ntype: concept\n---\nContent B.\n",
        )
        .unwrap();
        fs::write(dir.path().join("config.json"), r#"{"x":1}"#).unwrap();

        // First pass: both .md files imported, json counted as non-markdown
        let first = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(first.imported, 2);
        assert_eq!(first.skipped_already_ingested, 0);
        assert_eq!(first.skipped_non_markdown, 1);

        // Second pass: both .md already ingested, json still non-markdown
        let second = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(second.imported, 0);
        assert_eq!(second.skipped_already_ingested, 2);
        assert_eq!(second.skipped_non_markdown, 1);
        assert_eq!(second.total_skipped(), 3);
    }
}
