use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Result};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::core::markdown;
use crate::core::page_uuid;
use crate::core::palace;
use crate::core::raw_imports;
use crate::core::vault_sync;

#[derive(Debug)]
pub struct ImportStats {
    pub imported: usize,
    /// Files skipped because they were already ingested (same SHA-256).
    pub skipped_already_ingested: usize,
    /// Files skipped because they are not Markdown (`.md`).
    pub skipped_non_markdown: usize,
    pub type_inferred: usize,
}

impl ImportStats {
    pub fn total_skipped(&self) -> usize {
        self.skipped_already_ingested + self.skipped_non_markdown
    }
}

/// Import all `.md` files from a directory into the memory database.
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
            type_inferred: 0,
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
            Ok(entry) => parsed.push((file_path.clone(), raw_bytes, hash, entry)),
            Err(e) => errors.push(format!("{}: {e}", file_path.display())),
        }
    }

    if !errors.is_empty() && validate_only {
        bail!("Validation errors:\n{}", errors.join("\n"));
    }

    if validate_only {
        let type_inferred = parsed.iter().filter(|(_, _, _, e)| e.type_inferred).count();
        return Ok(ImportStats {
            imported: parsed.len(),
            skipped_already_ingested: 0,
            skipped_non_markdown: non_markdown_count,
            type_inferred,
        });
    }

    // Check which hashes are already ingested
    let mut imported = 0;
    let mut skipped_already_ingested = 0;
    let mut type_inferred_count = 0;

    vault_sync::ensure_all_collections_write_allowed(db)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    let tx = db.unchecked_transaction()?;

    for (file_path, raw_bytes, hash, entry) in &parsed {
        if is_already_ingested(&tx, hash)? {
            // Content unchanged — refresh the source path in case the file moved.
            refresh_ingest_source(&tx, hash, &file_path.to_string_lossy(), entry)?;
            skipped_already_ingested += 1;
            continue;
        }

        if entry.type_inferred {
            type_inferred_count += 1;
            eprintln!(
                "Imported {} (inferred type: {})",
                entry.slug, entry.page_type
            );
        }

        insert_page(&tx, entry)?;
        let page_id = page_id_for_slug(&tx, &entry.slug)?;
        raw_imports::rotate_active_raw_import(
            &tx,
            page_id,
            &file_path.to_string_lossy(),
            raw_bytes,
        )?;
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
        type_inferred: type_inferred_count,
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
    uuid: String,
    title: String,
    page_type: String,
    type_inferred: bool,
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

    // Three-tier type resolution:
    // Tier 1: explicit frontmatter type: field (non-blank, non-null wins)
    // Tier 2: infer from top-level PARA folder name
    // Tier 3: fallback to "concept"
    let frontmatter_type = frontmatter
        .get("type")
        .map(|t| t.trim())
        .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("null"))
        .map(|t| t.to_string());

    let (page_type, type_inferred) = if let Some(t) = frontmatter_type {
        (t, false)
    } else if let Some(t) = infer_type_from_path(file_path, root) {
        (t, true)
    } else {
        ("concept".to_string(), false)
    };

    let wing = frontmatter
        .get("wing")
        .cloned()
        .unwrap_or_else(|| palace::derive_wing(&slug));
    let room = palace::derive_room(&compiled_truth);
    let frontmatter_json = serde_json::to_string(&frontmatter)?;
    let uuid = page_uuid::resolve_page_uuid(&frontmatter, None)?;

    Ok(ParsedEntry {
        slug,
        uuid,
        title,
        page_type,
        type_inferred,
        summary,
        compiled_truth,
        timeline,
        frontmatter_json,
        wing,
        room,
    })
}

/// Infer page type from the top-level folder in a PARA-structured vault.
///
/// Strips leading numeric prefixes (e.g. `1. `, `02. `) that Obsidian users
/// commonly use for sort order, then matches case-insensitively against known
/// PARA folder names.
fn infer_type_from_path(file_path: &Path, root: &Path) -> Option<String> {
    let relative = file_path.strip_prefix(root).ok()?;
    let first_component = relative.components().next()?;
    let folder = first_component.as_os_str().to_string_lossy();

    // Strip leading numeric prefix: "1. Projects" → "Projects", "02. Areas" → "Areas"
    let stripped = strip_numeric_prefix(&folder);
    let normalized = stripped.to_lowercase();

    match normalized.as_str() {
        "projects" => Some("project".to_string()),
        "areas" => Some("area".to_string()),
        "resources" => Some("resource".to_string()),
        "archives" => Some("archive".to_string()),
        "journal" | "journals" => Some("journal".to_string()),
        "people" => Some("person".to_string()),
        "companies" | "orgs" => Some("company".to_string()),
        _ => None,
    }
}

/// Strip a leading numeric prefix like "1. ", "02. ", "3.  " from a folder name.
fn strip_numeric_prefix(name: &str) -> &str {
    let bytes = name.as_bytes();
    let mut i = 0;

    // Skip leading digits
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }

    // If we consumed at least one digit and next char is '.', skip the dot and any spaces
    if i > 0 && i < bytes.len() && bytes[i] == b'.' {
        i += 1; // skip '.'
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1; // skip trailing whitespace
        }
        &name[i..]
    } else {
        name
    }
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
             (slug, uuid, type, title, summary, compiled_truth, timeline, \
              frontmatter, wing, room, version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1) \
         ON CONFLICT(collection_id, slug) DO UPDATE SET \
             uuid = COALESCE(pages.uuid, excluded.uuid), \
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
            entry.uuid,
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

fn page_id_for_slug(db: &Connection, slug: &str) -> Result<i64> {
    db.query_row(
        "SELECT id FROM pages WHERE collection_id = 1 AND slug = ?1",
        [slug],
        |row| row.get(0),
    )
    .map_err(Into::into)
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
                uuid, frontmatter, wing, room, version, created_at, updated_at, \
                truth_updated_at, timeline_updated_at \
         FROM pages ORDER BY slug",
    )?;

    let rows = stmt.query_map([], |row| {
        let frontmatter_json: String = row.get(7)?;
        let frontmatter: HashMap<String, String> =
            serde_json::from_str(&frontmatter_json).unwrap_or_default();

        Ok(crate::core::types::Page {
            slug: row.get(0)?,
            uuid: row.get::<_, Option<String>>(6)?.ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Null,
                    Box::new(page_uuid::PageUuidError::EmptyFrontmatterUuid),
                )
            })?,
            page_type: row.get(1)?,
            title: row.get(2)?,
            summary: row.get(3)?,
            compiled_truth: row.get(4)?,
            timeline: row.get(5)?,
            frontmatter,
            wing: row.get(8)?,
            room: row.get(9)?,
            version: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            truth_updated_at: row.get(13)?,
            timeline_updated_at: row.get(14)?,
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
    use crate::core::raw_imports;
    use std::path::Path;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    fn active_raw_import_count_for_slug(conn: &Connection, slug: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM raw_imports \
             WHERE page_id = (SELECT id FROM pages WHERE slug = ?1) AND is_active = 1",
            [slug],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn active_raw_import_bytes_for_slug(conn: &Connection, slug: &str) -> Vec<u8> {
        conn.query_row(
            "SELECT raw_bytes FROM raw_imports \
             WHERE page_id = (SELECT id FROM pages WHERE slug = ?1) AND is_active = 1",
            [slug],
            |row| row.get(0),
        )
        .unwrap()
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
    fn import_dir_refuses_when_any_collection_write_is_blocked() {
        let conn = open_test_db();
        conn.execute(
            "UPDATE collections SET state = 'restoring' WHERE is_write_target = 1",
            [],
        )
        .unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("blocked.md");
        fs::write(&file_path, "---\nslug: blocked\n---\nblocked").unwrap();

        let error = import_dir(&conn, dir.path(), false).unwrap_err();

        assert!(error.to_string().contains("CollectionRestoringError"));
        let page_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE slug = 'blocked'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(page_count, 0);
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
    fn import_dir_second_pass_is_zero_change_for_existing_page_rows() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("note.md");
        fs::write(
            &file_path,
            "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is still here.\n",
        )
        .unwrap();

        let first = import_dir(&conn, dir.path(), false).unwrap();
        let version_before: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let page_count_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .unwrap();

        let second = import_dir(&conn, dir.path(), false).unwrap();
        let version_after: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let page_count_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .unwrap();

        assert_eq!(first.imported, 1);
        assert_eq!(second.imported, 0);
        assert_eq!(second.skipped_already_ingested, 1);
        assert_eq!(page_count_before, page_count_after);
        assert_eq!(version_before, version_after);
        assert_eq!(version_after, 1);
    }

    #[test]
    #[ignore = "blocked on task 5.4d/5.4g: import_dir does not rotate raw_imports yet"]
    fn import_dir_write_path_keeps_exactly_one_active_raw_import_row_for_latest_bytes() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("note.md");
        let original =
            "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice founded Acme.\n";
        fs::write(&file_path, original).unwrap();
        import_dir(&conn, dir.path(), false).unwrap();

        let updated = "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice founded Acme and now runs operations.\n";
        fs::write(&file_path, updated).unwrap();
        import_dir(&conn, dir.path(), false).unwrap();

        assert_eq!(active_raw_import_count_for_slug(&conn, "people/alice"), 1);
        assert_eq!(
            active_raw_import_bytes_for_slug(&conn, "people/alice"),
            updated.as_bytes()
        );
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
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES ('test/page', '01969f11-9448-7d79-8d3f-c68f54761234', 'concept', 'Test', 'Summary', 'Content.', 'Timeline.', \
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
        let db_path = db_dir.path().join("test_memory.db");
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

    #[test]
    fn import_dir_rotates_raw_imports_with_exactly_one_active_row() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("note.md");
        fs::write(
            &file_path,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nFirst version.\n",
        )
        .unwrap();

        import_dir(&conn, dir.path(), false).unwrap();
        fs::write(
            &file_path,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nSecond version with a changed hash.\n",
        )
        .unwrap();
        import_dir(&conn, dir.path(), false).unwrap();

        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'notes/test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let inactive_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM raw_imports
                 WHERE page_id = ?1 AND is_active = 0",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(
            raw_imports::active_raw_import_count(&conn, page_id).unwrap(),
            1
        );
        assert_eq!(inactive_count, 1);
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
        let moved_path = dir.path().join("sub").join("note.md");
        fs::rename(dir.path().join("note.md"), &moved_path).unwrap();

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
        let expected_suffix = Path::new("sub").join("note.md");
        assert!(
            Path::new(&updated_ref).ends_with(&expected_suffix),
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

    #[test]
    fn import_export_reimport_preserves_memory_id_frontmatter() {
        let source_db = open_test_db();
        let source_dir = tempfile::TempDir::new().unwrap();
        fs::write(
            source_dir.path().join("alice.md"),
            "---\nmemory_id: 0195c7c0-2d06-7df0-bf59-acde48001122\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n",
        )
        .unwrap();

        import_dir(&source_db, source_dir.path(), false).unwrap();

        let export_dir_path = tempfile::TempDir::new().unwrap();
        export_dir(&source_db, export_dir_path.path()).unwrap();

        let exported =
            fs::read_to_string(export_dir_path.path().join("people").join("alice.md")).unwrap();
        assert!(
            exported.contains("memory_id: 0195c7c0-2d06-7df0-bf59-acde48001122\n"),
            "exported markdown must keep memory_id frontmatter, got: {exported}"
        );

        let reimport_db = open_test_db();
        import_dir(&reimport_db, export_dir_path.path(), false).unwrap();

        let frontmatter_json: String = reimport_db
            .query_row(
                "SELECT frontmatter FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let frontmatter: HashMap<String, String> = serde_json::from_str(&frontmatter_json).unwrap();
        assert_eq!(
            frontmatter.get("memory_id").map(String::as_str),
            Some("0195c7c0-2d06-7df0-bf59-acde48001122")
        );
    }

    // ── infer_type_from_path tests ───────────────────────────────

    #[test]
    fn infer_type_numbered_projects() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/1. Projects/foo/bar.md");
        assert_eq!(
            infer_type_from_path(file, root),
            Some("project".to_string())
        );
    }

    #[test]
    fn infer_type_numbered_areas() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/2. Areas/health.md");
        assert_eq!(infer_type_from_path(file, root), Some("area".to_string()));
    }

    #[test]
    fn infer_type_plain_resources() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/Resources/book.md");
        assert_eq!(
            infer_type_from_path(file, root),
            Some("resource".to_string())
        );
    }

    #[test]
    fn infer_type_archives() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/4. Archives/old.md");
        assert_eq!(
            infer_type_from_path(file, root),
            Some("archive".to_string())
        );
    }

    #[test]
    fn infer_type_journal() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/Journal/2024-01-01.md");
        assert_eq!(
            infer_type_from_path(file, root),
            Some("journal".to_string())
        );
    }

    #[test]
    fn infer_type_journals_plural() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/Journals/entry.md");
        assert_eq!(
            infer_type_from_path(file, root),
            Some("journal".to_string())
        );
    }

    #[test]
    fn infer_type_people() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/people/alice.md");
        assert_eq!(infer_type_from_path(file, root), Some("person".to_string()));
    }

    #[test]
    fn infer_type_companies() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/Companies/acme.md");
        assert_eq!(
            infer_type_from_path(file, root),
            Some("company".to_string())
        );
    }

    #[test]
    fn infer_type_orgs() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/Orgs/nonprofit.md");
        assert_eq!(
            infer_type_from_path(file, root),
            Some("company".to_string())
        );
    }

    #[test]
    fn infer_type_unknown_folder() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/random/note.md");
        assert_eq!(infer_type_from_path(file, root), None);
    }

    #[test]
    fn infer_type_case_insensitive() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/PROJECTS/task.md");
        assert_eq!(
            infer_type_from_path(file, root),
            Some("project".to_string())
        );
    }

    #[test]
    fn infer_type_file_at_root_no_folder() {
        let root = Path::new("/vault");
        let file = Path::new("/vault/readme.md");
        // A file directly in root has its first component as the filename itself,
        // which won't match any PARA folder name.
        assert_eq!(infer_type_from_path(file, root), None);
    }

    // ── integration: PARA folder import ──────────────────────────

    #[test]
    fn import_para_vault_infers_types() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // Create a mini PARA vault with no type: frontmatter
        let folders = &[
            ("1. Projects", "project"),
            ("2. Areas", "area"),
            ("Resources", "resource"),
            ("Journal", "journal"),
            ("people", "person"),
        ];

        for (folder, expected_type) in folders {
            let folder_path = dir.path().join(folder);
            fs::create_dir_all(&folder_path).unwrap();
            fs::write(
                folder_path.join("note.md"),
                format!("---\ntitle: Note\n---\nContent about {expected_type}.\n"),
            )
            .unwrap();
        }

        let stats = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(stats.imported, 5);
        assert_eq!(stats.type_inferred, 5);

        // Verify each page has the correct inferred type
        for (_, expected_type) in folders {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pages WHERE type = ?1",
                    [expected_type],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                count >= 1,
                "expected at least 1 page of type '{expected_type}', got {count}"
            );
        }
    }

    #[test]
    fn frontmatter_type_overrides_folder_inference() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // File in Projects/ folder but with explicit type: concept
        let proj_dir = dir.path().join("Projects");
        fs::create_dir_all(&proj_dir).unwrap();
        fs::write(
            proj_dir.join("note.md"),
            "---\ntitle: Override\ntype: concept\n---\nContent.\n",
        )
        .unwrap();

        let stats = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(stats.imported, 1);
        assert_eq!(stats.type_inferred, 0);

        let page_type: String = conn
            .query_row(
                "SELECT type FROM pages WHERE title = 'Override'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(page_type, "concept");
    }

    #[test]
    fn blank_type_in_frontmatter_falls_back_to_folder_inference() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // File in Projects/ with `type:` explicitly set to blank — should infer from folder.
        let proj_dir = dir.path().join("Projects");
        fs::create_dir_all(&proj_dir).unwrap();
        fs::write(
            proj_dir.join("note.md"),
            "---\ntitle: BlankType\ntype: \n---\nContent.\n",
        )
        .unwrap();

        let stats = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(stats.imported, 1);
        assert_eq!(
            stats.type_inferred, 1,
            "blank type: should fall back to folder inference"
        );

        let page_type: String = conn
            .query_row(
                "SELECT type FROM pages WHERE title = 'BlankType'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(page_type, "project");
    }

    #[test]
    fn null_type_in_frontmatter_falls_back_to_folder_inference() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // File in Areas/ with `type: null` — should infer from folder.
        let area_dir = dir.path().join("Areas");
        fs::create_dir_all(&area_dir).unwrap();
        fs::write(
            area_dir.join("note.md"),
            "---\ntitle: NullType\ntype: null\n---\nContent.\n",
        )
        .unwrap();

        let stats = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(stats.imported, 1);
        assert_eq!(
            stats.type_inferred, 1,
            "null type: should fall back to folder inference"
        );

        let page_type: String = conn
            .query_row(
                "SELECT type FROM pages WHERE title = 'NullType'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(page_type, "area");
    }

    #[test]
    fn string_null_type_in_frontmatter_falls_back_to_folder_inference() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // File in Areas/ with `type: "null"` — should infer from folder.
        let area_dir = dir.path().join("Areas");
        fs::create_dir_all(&area_dir).unwrap();
        fs::write(
            area_dir.join("note.md"),
            "---\ntitle: StringNullType\ntype: \"null\"\n---\nContent.\n",
        )
        .unwrap();

        let stats = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(stats.imported, 1);
        assert_eq!(
            stats.type_inferred, 1,
            "string null type: should fall back to folder inference"
        );

        let page_type: String = conn
            .query_row(
                "SELECT type FROM pages WHERE title = 'StringNullType'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(page_type, "area");
    }

    #[test]
    fn frontmatter_type_is_trimmed_before_persist() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // File in Areas/ with a padded type should store the trimmed value.
        let area_dir = dir.path().join("Areas");
        fs::create_dir_all(&area_dir).unwrap();
        fs::write(
            area_dir.join("note.md"),
            "---\ntitle: TrimmedType\ntype: \"project \"\n---\nContent.\n",
        )
        .unwrap();

        let stats = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(stats.imported, 1);
        assert_eq!(stats.type_inferred, 0);

        let page_type: String = conn
            .query_row(
                "SELECT type FROM pages WHERE title = 'TrimmedType'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(page_type, "project");
    }

    // ── strip_numeric_prefix tests ───────────────────────────────

    #[test]
    fn strip_numeric_prefix_basic() {
        assert_eq!(strip_numeric_prefix("1. Projects"), "Projects");
        assert_eq!(strip_numeric_prefix("02. Areas"), "Areas");
        assert_eq!(strip_numeric_prefix("3.  Resources"), "Resources");
        assert_eq!(strip_numeric_prefix("Projects"), "Projects");
        assert_eq!(strip_numeric_prefix("10. Archives"), "Archives");
    }

    // ── fallback behavior tests ───────────────────────────────────

    #[test]
    fn unknown_folder_falls_back_to_concept() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // File in an unrecognised folder — should default to concept
        let misc_dir = dir.path().join("Miscellaneous");
        fs::create_dir_all(&misc_dir).unwrap();
        fs::write(
            misc_dir.join("note.md"),
            "---\ntitle: UnknownFolder\n---\nContent.\n",
        )
        .unwrap();

        let stats = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(stats.imported, 1);
        assert_eq!(stats.type_inferred, 0);

        let page_type: String = conn
            .query_row(
                "SELECT type FROM pages WHERE title = 'UnknownFolder'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(page_type, "concept");
    }

    #[test]
    fn root_level_file_no_type_falls_back_to_concept() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        // File directly at vault root (no sub-folder) — first component is the filename
        fs::write(
            dir.path().join("readme.md"),
            "---\ntitle: RootFile\n---\nRoot level content.\n",
        )
        .unwrap();

        let stats = import_dir(&conn, dir.path(), false).unwrap();
        assert_eq!(stats.imported, 1);
        assert_eq!(stats.type_inferred, 0);

        let page_type: String = conn
            .query_row(
                "SELECT type FROM pages WHERE title = 'RootFile'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(page_type, "concept");
    }
}
