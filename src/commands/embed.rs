#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

use anyhow::Result;
use rusqlite::Connection;

use crate::commands::get::get_page_by_key;
use crate::core::chunking::chunk_page;
use crate::core::collections::OpKind;
use crate::core::inference::{embed, embedding_to_blob, EMBEDDER_VERSION};
use crate::core::vault_sync;

const DEFAULT_BATCH_SIZE: usize = 32;

/// `quaid_config` key recording the [`EMBEDDER_VERSION`] the store was last
/// fully re-embedded under. A missing key marks a legacy store (version 1).
const EMBEDDER_VERSION_KEY: &str = "embedder_version";

#[derive(Debug, Clone)]
struct PageRef {
    id: i64,
    collection_id: i64,
    slug: String,
}

pub fn run(db: &Connection, slug: Option<String>, all: bool, stale: bool) -> Result<()> {
    run_with_batch(db, slug, all, stale, None)
}

pub fn run_with_batch(
    db: &Connection,
    slug: Option<String>,
    all: bool,
    stale: bool,
    batch_size: Option<usize>,
) -> Result<()> {
    // Modes are mutually exclusive: exactly one of <SLUG>, --all, or --stale.
    if slug.is_some() && (all || stale) {
        anyhow::bail!(
            "embed modes are mutually exclusive: provide a <SLUG>, --all, or --stale — not a combination"
        );
    }
    if all && stale {
        anyhow::bail!("--all and --stale are mutually exclusive");
    }
    let batch_size = batch_size.unwrap_or(DEFAULT_BATCH_SIZE);
    anyhow::ensure!(batch_size > 0, "--batch-size must be greater than 0");

    let (model_name, vec_table) = active_model(db)?;
    anyhow::ensure!(
        is_safe_identifier(&vec_table),
        "unsafe vec table name: {vec_table}"
    );

    let (embedded_pages, embedded_chunks) = if let Some(slug) = slug.as_deref() {
        let (page, page_id) = resolve_single_page(db, slug)?;
        let chunks = chunk_page(&page);

        if chunks.is_empty() {
            (0, 0)
        } else {
            replace_page_embeddings(db, page_id, &model_name, &vec_table, &chunks)?;
            (1, chunks.len())
        }
    } else {
        let embedder_version_mismatch = stored_embedder_version(db)? != Some(EMBEDDER_VERSION);
        let mut embedded_pages = 0_usize;
        let mut embedded_chunks = 0_usize;
        let mut failed_pages = 0_usize;
        let mut last_page_id = 0_i64;

        loop {
            let page_refs = page_refs_after(db, last_page_id, batch_size)?;
            if page_refs.is_empty() {
                break;
            }

            for page_ref in page_refs {
                last_page_id = page_ref.id;
                let page = match get_page_by_key(db, page_ref.collection_id, &page_ref.slug) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!(
                            "{}",
                            format_embed_warning(
                                &page_ref.slug,
                                lookup_source_path_by_page_id(db, page_ref.id).as_deref(),
                                &e
                            )
                        );
                        failed_pages += 1;
                        continue;
                    }
                };
                let chunks = chunk_page(&page);

                if chunks.is_empty() {
                    continue;
                }

                if !page_needs_refresh(
                    db,
                    page_ref.id,
                    &model_name,
                    &chunks,
                    embedder_version_mismatch,
                )? {
                    continue;
                }

                if let Err(e) =
                    replace_page_embeddings(db, page_ref.id, &model_name, &vec_table, &chunks)
                {
                    eprintln!(
                        "{}",
                        format_embed_warning(
                            &page_ref.slug,
                            lookup_source_path_by_page_id(db, page_ref.id).as_deref(),
                            &e
                        )
                    );
                    failed_pages += 1;
                    continue;
                }
                embedded_pages += 1;
                embedded_chunks += chunks.len();
            }
        }

        // A clean full pass means every page's embeddings are now current, so
        // record the pipeline version; any failure keeps the old version so
        // the next pass retries the skipped pages.
        if failed_pages == 0 {
            record_embedder_version(db)?;
        }

        (embedded_pages, embedded_chunks)
    };

    println!("Embedded {embedded_chunks} chunks across {embedded_pages} page(s).");
    Ok(())
}

fn resolve_single_page(db: &Connection, slug: &str) -> Result<(crate::core::types::Page, i64)> {
    let resolved = vault_sync::resolve_slug_for_op(db, slug, OpKind::Read)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let page = get_page_by_key(db, resolved.collection_id, &resolved.slug)?;
    let page_id = page_id_by_key(db, resolved.collection_id, &resolved.slug)?;
    Ok((page, page_id))
}

fn active_model(db: &Connection) -> Result<(String, String)> {
    db.query_row(
        "SELECT name, vec_table FROM embedding_models WHERE active = 1 LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(Into::into)
}

fn page_refs_after(db: &Connection, last_page_id: i64, batch_size: usize) -> Result<Vec<PageRef>> {
    let limit = i64::try_from(batch_size)?;
    let mut stmt =
        db.prepare("SELECT id, collection_id, slug FROM pages WHERE id > ?1 ORDER BY id LIMIT ?2")?;
    let rows = stmt.query_map(rusqlite::params![last_page_id, limit], |row| {
        Ok(PageRef {
            id: row.get(0)?,
            collection_id: row.get(1)?,
            slug: row.get(2)?,
        })
    })?;

    let mut page_refs = Vec::new();
    for row in rows {
        page_refs.push(row?);
    }
    Ok(page_refs)
}

fn page_id_by_key(db: &Connection, collection_id: i64, slug: &str) -> Result<i64> {
    db.query_row(
        "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
        rusqlite::params![collection_id, slug],
        |row| row.get(0),
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("page not found: {slug}"),
        other => other.into(),
    })
}

/// Reads the embedder version recorded by the last clean full embed pass.
/// `None` marks a legacy store last embedded before version tracking.
fn stored_embedder_version(db: &Connection) -> Result<Option<i64>> {
    let value: Option<String> = db
        .query_row(
            "SELECT value FROM quaid_config WHERE key = ?1",
            [EMBEDDER_VERSION_KEY],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;

    match value {
        Some(raw) => {
            let parsed = raw.parse::<i64>().map_err(|_| {
                anyhow::anyhow!("quaid_config.{EMBEDDER_VERSION_KEY} must be an integer: {raw}")
            })?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

fn record_embedder_version(db: &Connection) -> Result<()> {
    db.execute(
        "INSERT INTO quaid_config (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![EMBEDDER_VERSION_KEY, EMBEDDER_VERSION.to_string()],
    )?;
    Ok(())
}

fn page_needs_refresh(
    db: &Connection,
    page_id: i64,
    model_name: &str,
    chunks: &[crate::core::types::Chunk],
    embedder_version_mismatch: bool,
) -> Result<bool> {
    if embedder_version_mismatch {
        return Ok(true);
    }

    let mut stmt = db.prepare(
        "SELECT chunk_type, content_hash, heading_path \
         FROM page_embeddings \
         WHERE page_id = ?1 AND model = ?2 \
         ORDER BY chunk_index",
    )?;
    let rows = stmt.query_map(rusqlite::params![page_id, model_name], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut existing = Vec::new();
    for row in rows {
        existing.push(row?);
    }

    if existing.len() != chunks.len() {
        return Ok(true);
    }

    Ok(existing
        .iter()
        .zip(chunks)
        .any(|((chunk_type, content_hash, heading_path), chunk)| {
            chunk_type != &chunk.chunk_type
                || content_hash != &chunk.content_hash
                || heading_path != &chunk.heading_path
        }))
}

fn lookup_source_path_by_page_id(db: &Connection, page_id: i64) -> Option<String> {
    db.query_row(
        "SELECT file_path
         FROM raw_imports
         WHERE page_id = ?1 AND is_active = 1
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
        [page_id],
        |row| row.get(0),
    )
    .ok()
}

/// Format a per-page embed warning with optional source file path.
fn format_embed_warning(
    slug: &str,
    source_path: Option<&str>,
    error: &dyn std::fmt::Display,
) -> String {
    match source_path {
        Some(path) => format!("warning: embedding skipped '{slug}' (source: {path}): {error}"),
        None => format!("warning: embedding skipped '{slug}': {error}"),
    }
}

/// Atomically replace all embeddings for a page. Uses a transaction so that
/// a failure mid-way (e.g. inference error on a later chunk) does not leave
/// the page with partially updated embeddings.
fn replace_page_embeddings(
    db: &Connection,
    page_id: i64,
    model_name: &str,
    vec_table: &str,
    chunks: &[crate::core::types::Chunk],
) -> Result<()> {
    let tx = db.unchecked_transaction()?;

    let existing_rowids = existing_vec_rowids(&tx, page_id, model_name)?;
    let delete_vec_sql = format!("DELETE FROM {vec_table} WHERE rowid = ?1");

    for vec_rowid in existing_rowids {
        tx.execute(&delete_vec_sql, [vec_rowid])?;
    }

    tx.execute(
        "DELETE FROM page_embeddings WHERE page_id = ?1 AND model = ?2",
        rusqlite::params![page_id, model_name],
    )?;

    let insert_vec_sql = format!("INSERT INTO {vec_table}(embedding) VALUES (?1)");
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        let embedding = embed(&chunk.content)?;
        let embedding_blob = embedding_to_blob(&embedding);

        tx.execute(&insert_vec_sql, rusqlite::params![embedding_blob])?;
        let vec_rowid = tx.last_insert_rowid();

        tx.execute(
            "INSERT INTO page_embeddings \
                 (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, \
                  content_hash, token_count, heading_path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                page_id,
                model_name,
                vec_rowid,
                chunk.chunk_type,
                chunk_index as i64,
                chunk.content,
                chunk.content_hash,
                chunk.token_count as i64,
                chunk.heading_path,
            ],
        )?;
    }

    tx.commit()?;
    Ok(())
}

fn existing_vec_rowids(db: &Connection, page_id: i64, model_name: &str) -> Result<Vec<i64>> {
    let mut stmt = db.prepare(
        "SELECT vec_rowid FROM page_embeddings WHERE page_id = ?1 AND model = ?2 ORDER BY chunk_index",
    )?;
    let rows = stmt.query_map(rusqlite::params![page_id, model_name], |row| row.get(0))?;

    let mut rowids = Vec::new();
    for row in rows {
        rowids.push(row?);
    }
    Ok(rowids)
}

fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test_memory.db");
        let conn = db::open(db_path.to_str().expect("utf8 path")).expect("open db");
        std::mem::forget(dir);
        conn
    }

    fn test_uuid(slug: &str) -> String {
        let mut hex = String::new();
        for byte in slug.as_bytes() {
            hex.push_str(&format!("{byte:02x}"));
            if hex.len() >= 32 {
                break;
            }
        }
        while hex.len() < 32 {
            hex.push('0');
        }

        format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
    }

    fn insert_test_page(conn: &Connection, slug: &str, compiled_truth: &str, timeline: &str) {
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'person', 'Alice', 'Founder', ?3, ?4, '{}', 'people', '', 1)",
            rusqlite::params![slug, test_uuid(slug), compiled_truth, timeline],
        )
        .expect("insert page");
    }

    fn insert_collection(conn: &Connection, name: &str, root_path: &std::path::Path) -> i64 {
        conn.execute(
            "INSERT INTO collections (name, root_path, state, writable, is_write_target)
             VALUES (?1, ?2, 'active', 1, 0)",
            rusqlite::params![name, root_path.display().to_string()],
        )
        .expect("insert collection");
        conn.last_insert_rowid()
    }

    fn insert_test_page_in_collection(
        conn: &Connection,
        collection_id: i64,
        slug: &str,
        compiled_truth: &str,
        timeline: &str,
    ) {
        conn.execute(
            "INSERT INTO pages (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, ?3, 'person', 'Alice', 'Founder', ?4, ?5, '{}', 'people', '', 1)",
            rusqlite::params![
                collection_id,
                slug,
                test_uuid(&format!("{collection_id}:{slug}")),
                compiled_truth,
                timeline
            ],
        )
        .expect("insert page");
    }

    fn lookup_source_path_for_slug(conn: &Connection, slug: &str) -> Option<String> {
        let page_id = conn
            .query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
                row.get(0)
            })
            .ok()?;
        lookup_source_path_by_page_id(conn, page_id)
    }

    #[test]
    fn run_embeds_chunks_for_all_pages() {
        let conn = open_test_db();
        insert_test_page(
            &conn,
            "people/alice",
            "## State\nAlice is investing.\n## Assessment\nStrong operator.",
            "2024-01-01 Joined Acme\n---\n2024-02-01 Raised seed",
        );

        run(&conn, None, true, false).expect("embed all");

        let metadata_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
            .expect("count metadata rows");
        let vec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM page_embeddings_vec_384", [], |row| {
                row.get(0)
            })
            .expect("count vec rows");

        assert_eq!(metadata_count, 4);
        assert_eq!(vec_count, 4);
    }

    #[test]
    fn run_with_stale_only_skips_unchanged_pages() {
        let conn = open_test_db();
        insert_test_page(
            &conn,
            "people/alice",
            "## State\nAlice is investing.",
            "2024-01-01 Joined Acme",
        );

        run(&conn, None, true, false).expect("initial embed");
        let first_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
            .expect("initial metadata count");

        run(&conn, None, false, true).expect("stale embed");
        let second_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
            .expect("second metadata count");

        assert_eq!(first_count, second_count);
    }

    #[test]
    fn run_with_all_skips_unchanged_pages() {
        let conn = open_test_db();
        insert_test_page(
            &conn,
            "people/alice",
            "## State\nAlice is investing.",
            "2024-01-01 Joined Acme",
        );

        run(&conn, None, true, false).expect("initial embed");
        let first_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
            .expect("initial metadata count");

        // --all on unchanged content must skip (spec: skip if content_hash unchanged)
        run(&conn, None, true, false).expect("second all embed");
        let second_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
            .expect("second metadata count");

        assert_eq!(first_count, second_count);
    }

    #[test]
    fn run_rejects_slug_with_all_flag() {
        let conn = open_test_db();
        insert_test_page(&conn, "people/alice", "## State\nAlice is investing.", "");

        let result = run(&conn, Some("people/alice".to_owned()), true, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("mutually exclusive"));
    }

    #[test]
    fn run_rejects_slug_with_stale_flag() {
        let conn = open_test_db();
        insert_test_page(&conn, "people/alice", "## State\nAlice is investing.", "");

        let result = run(&conn, Some("people/alice".to_owned()), false, true);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("mutually exclusive"));
    }

    #[test]
    fn run_rejects_all_with_stale() {
        let conn = open_test_db();
        let result = run(&conn, None, true, true);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("mutually exclusive"));
    }

    #[test]
    fn run_embeds_single_page_by_slug() {
        let conn = open_test_db();
        insert_test_page(&conn, "people/alice", "## State\nAlice is investing.", "");
        insert_test_page(&conn, "people/bob", "## State\nBob is building.", "");

        run(&conn, Some("people/alice".to_owned()), false, false).expect("embed single slug");

        let alice_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM page_embeddings pe \
                 JOIN pages p ON p.id = pe.page_id WHERE p.slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .expect("alice embedding count");
        let bob_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM page_embeddings pe \
                 JOIN pages p ON p.id = pe.page_id WHERE p.slug = 'people/bob'",
                [],
                |row| row.get(0),
            )
            .expect("bob embedding count");

        assert_eq!(alice_count, 1);
        assert_eq!(bob_count, 0);
    }

    #[test]
    fn run_with_slug_re_embeds_even_when_unchanged() {
        let conn = open_test_db();
        insert_test_page(&conn, "people/alice", "## State\nAlice is investing.", "");

        run(&conn, Some("people/alice".to_owned()), false, false).expect("first embed");
        // Re-embed same slug — should succeed (no stale skip for explicit slug)
        run(&conn, Some("people/alice".to_owned()), false, false).expect("re-embed");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn run_embeds_single_page_by_explicit_collection_slug() {
        let conn = open_test_db();
        let roots = tempfile::TempDir::new().expect("create roots dir");
        let work_root = roots.path().join("work");
        let memory_root = roots.path().join("memory");
        std::fs::create_dir_all(&work_root).expect("create work root");
        std::fs::create_dir_all(&memory_root).expect("create memory root");
        let work_id = insert_collection(&conn, "work", &work_root);
        let memory_id = insert_collection(&conn, "memory", &memory_root);
        insert_test_page_in_collection(
            &conn,
            work_id,
            "people/alice",
            "## State\nWork Alice is investing.",
            "",
        );
        insert_test_page_in_collection(
            &conn,
            memory_id,
            "people/alice",
            "## State\nMemory Alice is reflecting.",
            "",
        );

        run(&conn, Some("work::people/alice".to_owned()), false, false)
            .expect("embed explicit collection slug");

        let work_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM page_embeddings pe
                 JOIN pages p ON p.id = pe.page_id
                 WHERE p.collection_id = ?1 AND p.slug = 'people/alice'",
                [work_id],
                |row| row.get(0),
            )
            .expect("work embedding count");
        let memory_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM page_embeddings pe
                 JOIN pages p ON p.id = pe.page_id
                 WHERE p.collection_id = ?1 AND p.slug = 'people/alice'",
                [memory_id],
                |row| row.get(0),
            )
            .expect("memory embedding count");

        assert_eq!(work_count, 1);
        assert_eq!(memory_count, 0);
    }

    /// Batch embed (`--all`) must return Ok even when individual pages fail
    /// to embed. We sabotage the vec table so `replace_page_embeddings` errors
    /// on every page, proving the loop continues past failures.
    #[test]
    fn batch_embed_continues_past_per_page_failure() {
        let conn = open_test_db();
        insert_test_page(&conn, "people/alice", "## State\nAlice is investing.", "");
        insert_test_page(&conn, "people/bob", "## State\nBob is building.", "");

        // Drop the vec table to force replace_page_embeddings to fail.
        conn.execute_batch("DROP TABLE IF EXISTS page_embeddings_vec_384")
            .expect("drop vec table");

        // Batch embed must return Ok despite per-page failures.
        run(&conn, None, true, false)
            .expect("batch embed should succeed despite vec table missing");
    }

    /// Single-slug embed must propagate errors (not downgrade to warning).
    #[test]
    fn single_slug_embed_propagates_error_on_failure() {
        let conn = open_test_db();
        insert_test_page(&conn, "people/alice", "## State\nAlice is investing.", "");

        // Drop the vec table so embedding fails.
        conn.execute_batch("DROP TABLE IF EXISTS page_embeddings_vec_384")
            .expect("drop vec table");

        let result = run(&conn, Some("people/alice".to_owned()), false, false);
        assert!(
            result.is_err(),
            "single-slug embed must return Err, not swallow the failure"
        );
    }

    // ── Deterministic output format tests ─────────────────────────────────

    #[test]
    fn format_warning_with_source_path() {
        let msg = format_embed_warning(
            "people/alice",
            Some("/docs/people/alice.md"),
            &"input text is empty",
        );
        assert_eq!(
            msg,
            "warning: embedding skipped 'people/alice' (source: /docs/people/alice.md): input text is empty"
        );
    }

    #[test]
    fn format_warning_without_source_path() {
        let msg = format_embed_warning("people/alice", None, &"input text is empty");
        assert_eq!(
            msg,
            "warning: embedding skipped 'people/alice': input text is empty"
        );
    }

    #[test]
    fn format_warning_with_generic_error() {
        let msg = format_embed_warning(
            "companies/acme",
            Some("/import/companies/acme.md"),
            &"page not found: companies/acme",
        );
        assert_eq!(
            msg,
            "warning: embedding skipped 'companies/acme' (source: /import/companies/acme.md): page not found: companies/acme"
        );
    }

    /// When a page's frontmatter slug differs from its filename (e.g. file is
    /// `notes/2024-01-meeting.md` but `slug: people/alice`), lookup must follow
    /// the active raw_imports row rather than guessing from the filename.
    #[test]
    fn lookup_source_path_works_when_frontmatter_slug_differs_from_filename() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES ('people/alice', '01969f11-9448-7d79-8d3f-c68f54761234', 'person', 'Alice', '', '', '', '{}', 'people', '', 1)",
            [],
        )
        .expect("insert page");
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .expect("page id");
        conn.execute(
            "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
             VALUES (?1, '01969f11-9448-7d79-8d3f-c68f54769999', 1, x'74657374', '/notes/2024-01-meeting.md')",
            [page_id],
        )
        .expect("insert raw import row");

        let result = lookup_source_path_for_slug(&conn, "people/alice");
        assert_eq!(
            result.as_deref(),
            Some("/notes/2024-01-meeting.md"),
            "should find file_path for frontmatter slug that differs from filename"
        );

        let miss = lookup_source_path_for_slug(&conn, "notes/2024-01-meeting");
        assert_eq!(
            miss, None,
            "filename-derived slug should not match when only the resolved page slug exists"
        );
    }

    #[test]
    fn lookup_source_path_works_after_single_file_ingest_with_frontmatter_slug_override() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let file_path = dir.path().join("2024-01-meeting.md");
        std::fs::write(
            &file_path,
            "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n",
        )
        .expect("write markdown fixture");

        crate::commands::ingest::run(&conn, file_path.to_str().expect("utf8 path"), false)
            .expect("ingest file");

        let result = lookup_source_path_for_slug(&conn, "people/alice");
        assert_eq!(
            result.as_deref(),
            file_path.to_str(),
            "single-file ingest should preserve slug→file-path mapping for later warnings"
        );
    }

    #[test]
    fn lookup_source_path_recovers_after_duplicate_skip_refreshes_active_raw_import_path() {
        let conn = open_test_db();

        let first_dir = tempfile::TempDir::new().expect("create first dir");
        let first_path = first_dir.path().join("note.md");
        std::fs::write(
            &first_path,
            "---\ntitle: Note\ntype: concept\n---\nStable content.\n",
        )
        .expect("write first fixture");

        crate::commands::ingest::run(&conn, first_path.to_str().expect("utf8 path"), false)
            .expect("first ingest");

        let second_dir = tempfile::TempDir::new().expect("create second dir");
        let second_path = second_dir.path().join("note.md");
        std::fs::write(
            &second_path,
            "---\ntitle: Note\ntype: concept\n---\nStable content.\n",
        )
        .expect("write second fixture");

        crate::commands::ingest::run(&conn, second_path.to_str().expect("utf8 path"), false)
            .expect("second ingest");

        assert_eq!(
            lookup_source_path_for_slug(&conn, "note").as_deref(),
            second_path.to_str()
        );
    }

    #[test]
    fn lookup_source_path_updates_after_force_reingest_from_moved_path() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().expect("create temp dir");

        let original_path = dir.path().join("note.md");
        std::fs::write(
            &original_path,
            "---\ntitle: Note\ntype: concept\n---\nStable content.\n",
        )
        .expect("write original fixture");

        crate::commands::ingest::run(&conn, original_path.to_str().expect("utf8 path"), false)
            .expect("initial ingest");

        std::fs::create_dir_all(dir.path().join("sub")).expect("create subdir");
        let moved_path = dir.path().join("sub").join("note.md");
        std::fs::rename(&original_path, &moved_path).expect("move file");

        crate::commands::ingest::run(&conn, moved_path.to_str().expect("utf8 path"), true)
            .expect("reingest moved file");

        assert_eq!(
            lookup_source_path_for_slug(&conn, "note").as_deref(),
            moved_path.to_str()
        );
    }
}
