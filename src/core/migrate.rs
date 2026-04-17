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
            // Content unchanged — refresh the source path in case the file moved.
            refresh_ingest_source(&tx, hash, &file_path.to_string_lossy(), entry)?;
            skipped_already_ingested += 1;
            continue;
        }

        insert_page(&tx, entry)?;
        record_ingest(&tx, hash, &file_path.to_string_lossy(), &entry.slug)?;
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

fn record_ingest(db: &Connection, hash: &str, path: &str, slug: &str) -> Result<()> {
    db.execute(
        "INSERT INTO ingest_log (ingest_key, source_type, source_ref, pages_updated) \
         VALUES (?1, 'file', ?2, json_array(?3)) \
         ON CONFLICT(ingest_key) DO UPDATE SET \
             source_ref = excluded.source_ref, \
             pages_updated = excluded.pages_updated, \
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        rusqlite::params![hash, path, slug],
    )?;
    Ok(())
}

/// Update the source path for an existing ingest_log row. Called when a
/// directory re-import encounters a file whose SHA-256 already exists but
/// whose path may have changed (e.g. the user moved or renamed the file).
fn refresh_ingest_source(
    db: &Connection,
    hash: &str,
    path: &str,
    entry: &ParsedEntry,
) -> Result<()> {
    let pages_updated = refreshed_pages_updated(db, hash, entry)?;
    db.execute(
        "UPDATE ingest_log SET source_ref = ?2, \
             pages_updated = ?3, \
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
         WHERE ingest_key = ?1",
        rusqlite::params![hash, path, pages_updated],
    )?;
    Ok(())
}

fn refreshed_pages_updated(db: &Connection, hash: &str, entry: &ParsedEntry) -> Result<String> {
    let mut slugs = existing_valid_pages_updated(db, hash)?;
    if slugs.is_empty() {
        slugs = matching_page_slugs(db, entry)?;
    }
    if slugs.is_empty() && page_exists(db, &entry.slug)? {
        slugs.push(entry.slug.clone());
    }
    slugs.sort();
    slugs.dedup();
    Ok(serde_json::to_string(&slugs)?)
}

fn existing_valid_pages_updated(db: &Connection, hash: &str) -> Result<Vec<String>> {
    let mut stmt = db.prepare(
        "SELECT je.value \
         FROM ingest_log il, json_each(il.pages_updated) je \
         WHERE il.ingest_key = ?1",
    )?;
    let rows = stmt.query_map([hash], |row| row.get::<_, String>(0))?;

    let mut slugs = Vec::new();
    for row in rows {
        let slug = row?;
        if page_exists(db, &slug)? {
            slugs.push(slug);
        }
    }
    Ok(slugs)
}

fn matching_page_slugs(db: &Connection, entry: &ParsedEntry) -> Result<Vec<String>> {
    let expected_frontmatter: HashMap<String, String> =
        serde_json::from_str(&entry.frontmatter_json).unwrap_or_default();
    let mut stmt = db.prepare(
        "SELECT slug, frontmatter FROM pages \
         WHERE compiled_truth = ?1 AND timeline = ?2 \
         ORDER BY slug",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![entry.compiled_truth, entry.timeline],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;

    let mut slugs = Vec::new();
    for row in rows {
        let (slug, frontmatter_json) = row?;
        let frontmatter: HashMap<String, String> =
            serde_json::from_str(&frontmatter_json).unwrap_or_default();
        if frontmatter == expected_frontmatter {
            slugs.push(slug);
        }
    }

    Ok(match slugs.as_slice() {
        [only] => vec![only.clone()],
        _ if slugs.iter().any(|slug| slug == &entry.slug) => vec![entry.slug.clone()],
        _ => Vec::new(),
    })
}

fn page_exists(db: &Connection, slug: &str) -> Result<bool> {
    let exists: i64 = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pages WHERE slug = ?1)",
        [slug],
        |row| row.get(0),
    )?;
    Ok(exists != 0)
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

    /// When a file uses a frontmatter `slug:` that differs from the filename,
    /// `record_ingest` must store the actual slug in `pages_updated` so that
    /// `lookup_source_path` can find the real file path later.
    #[test]
    fn record_ingest_stores_frontmatter_slug_in_pages_updated() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // Filename is `2024-01-meeting.md` but the frontmatter slug overrides it.
        let file_path = dir.path().join("2024-01-meeting.md");
        fs::write(
            &file_path,
            "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n",
        )
        .unwrap();

        import_dir(&conn, dir.path(), false).unwrap();

        // The ingest_log row must record the frontmatter slug, not the filename stem.
        let pages_updated: String = conn
            .query_row(
                "SELECT pages_updated FROM ingest_log WHERE source_type = 'file'",
                [],
                |row| row.get(0),
            )
            .expect("ingest_log row should exist");

        assert!(
            pages_updated.contains("people/alice"),
            "pages_updated should contain the frontmatter slug 'people/alice', got: {pages_updated}"
        );
        assert!(
            !pages_updated.contains("2024-01-meeting"),
            "pages_updated should not contain the filename stem, got: {pages_updated}"
        );
    }

    /// Re-importing the same content from a new directory must refresh the
    /// source_ref in ingest_log so that `lookup_source_path` returns the
    /// new path, not the stale original.
    #[test]
    fn reimport_same_content_from_new_directory_refreshes_source_ref() {
        let conn = open_test_db();

        // First import from directory A.
        let dir_a = tempfile::TempDir::new().unwrap();
        fs::write(
            dir_a.path().join("note.md"),
            "---\ntitle: Note\ntype: concept\n---\nSome real content here.\n",
        )
        .unwrap();
        let first = import_dir(&conn, dir_a.path(), false).unwrap();
        assert_eq!(first.imported, 1);

        let initial_ref: String = conn
            .query_row(
                "SELECT source_ref FROM ingest_log WHERE source_type = 'file'",
                [],
                |row| row.get(0),
            )
            .expect("initial ingest_log row");
        assert!(
            initial_ref.contains("note.md"),
            "initial source_ref should reference note.md"
        );

        // Copy exact same content into directory B and re-import.
        let dir_b = tempfile::TempDir::new().unwrap();
        fs::write(
            dir_b.path().join("note.md"),
            "---\ntitle: Note\ntype: concept\n---\nSome real content here.\n",
        )
        .unwrap();
        let second = import_dir(&conn, dir_b.path(), false).unwrap();
        assert_eq!(second.imported, 0);
        assert_eq!(second.skipped_already_ingested, 1);

        // source_ref must now point to directory B's path.
        let updated_ref: String = conn
            .query_row(
                "SELECT source_ref FROM ingest_log WHERE source_type = 'file'",
                [],
                |row| row.get(0),
            )
            .expect("updated ingest_log row");
        let dir_b_str = dir_b.path().to_string_lossy().to_string();
        assert!(
            updated_ref.starts_with(&dir_b_str),
            "source_ref should now point to dir_b ({}), got: {updated_ref}",
            dir_b_str
        );
    }

    /// Re-importing a directory where a file was moved to a subdirectory
    /// must refresh the source_ref to the new nested path.
    #[test]
    fn reimport_detects_path_change_within_same_directory() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // First: file at top level.
        fs::write(
            dir.path().join("note.md"),
            "---\ntitle: Note\ntype: concept\n---\nContent that stays the same.\n",
        )
        .unwrap();
        import_dir(&conn, dir.path(), false).unwrap();

        let initial_ref: String = conn
            .query_row(
                "SELECT source_ref FROM ingest_log WHERE source_type = 'file'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // Move the file into a subdirectory.
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::rename(dir.path().join("note.md"), dir.path().join("sub/note.md")).unwrap();

        import_dir(&conn, dir.path(), false).unwrap();

        let updated_ref: String = conn
            .query_row(
                "SELECT source_ref FROM ingest_log WHERE source_type = 'file'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(
            initial_ref, updated_ref,
            "source_ref must change when the file moves within the directory"
        );
        assert!(
            updated_ref.contains("sub/note.md"),
            "source_ref should reflect the new nested path, got: {updated_ref}"
        );
    }

    #[test]
    fn reimport_backfills_empty_pages_updated_with_existing_slug() {
        let conn = open_test_db();

        let dir_a = tempfile::TempDir::new().unwrap();
        fs::write(
            dir_a.path().join("note.md"),
            "---\ntitle: Note\ntype: concept\n---\nStable content.\n",
        )
        .unwrap();
        import_dir(&conn, dir_a.path(), false).unwrap();

        conn.execute(
            "UPDATE ingest_log \
             SET source_ref = 'old/path.md', pages_updated = '[]' \
             WHERE source_type = 'file'",
            [],
        )
        .unwrap();

        let dir_b = tempfile::TempDir::new().unwrap();
        fs::write(
            dir_b.path().join("note.md"),
            "---\ntitle: Note\ntype: concept\n---\nStable content.\n",
        )
        .unwrap();

        let stats = import_dir(&conn, dir_b.path(), false).unwrap();
        assert_eq!(stats.imported, 0);
        assert_eq!(stats.skipped_already_ingested, 1);

        let pages_updated: String = conn
            .query_row(
                "SELECT pages_updated FROM ingest_log WHERE source_type = 'file'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pages_updated, "[\"note\"]");
    }
}
