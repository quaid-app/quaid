use std::fs;
use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::core::{markdown, novelty, page_uuid, palace, raw_imports, vault_sync};

pub fn run(db: &Connection, path: &str, force: bool) -> Result<()> {
    let file = Path::new(path);
    let raw_bytes = fs::read(file)?;
    let hash = sha256_hex(&raw_bytes);
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();

    vault_sync::ensure_all_collections_write_allowed(db)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

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
    let existing_uuid: Option<String> = db
        .query_row("SELECT uuid FROM pages WHERE slug = ?1", [&slug], |row| {
            row.get(0)
        })
        .optional()?;
    let page_uuid = page_uuid::resolve_page_uuid(&frontmatter, existing_uuid.as_deref())?;
    let frontmatter_json = serde_json::to_string(&frontmatter)?;

    let tx = db.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO pages \
             (slug, uuid, type, title, summary, compiled_truth, timeline, \
              frontmatter, wing, room, version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1) \
         ON CONFLICT(collection_id, slug) DO UPDATE SET \
             uuid = excluded.uuid, \
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
            page_uuid,
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
    let page_id: i64 = tx.query_row(
        "SELECT id FROM pages WHERE collection_id = 1 AND slug = ?1",
        [&slug],
        |row| row.get(0),
    )?;
    raw_imports::rotate_active_raw_import(&tx, page_id, path, &raw_bytes)?;
    record_ingest(&tx, &hash, path, &slug)?;
    tx.commit()?;
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
    use crate::core::raw_imports;
    use std::process::Command;

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
    fn ingest_skip_path_leaves_existing_page_version_unchanged() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        fs::write(
            &file_path,
            "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nContent.\n",
        )
        .unwrap();

        run(&conn, file_path.to_str().unwrap(), false).unwrap();
        let version_before: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        run(&conn, file_path.to_str().unwrap(), false).unwrap();
        let version_after: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(version_before, version_after);
        assert_eq!(version_after, 1);
    }

    #[test]
    #[ignore = "blocked on task 5.4d/5.4g: ingest write path does not rotate raw_imports yet"]
    fn ingest_force_reingest_keeps_exactly_one_active_raw_import_row_for_latest_bytes() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        let original = "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nContent.\n";
        fs::write(&file_path, original).unwrap();
        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        let updated =
            "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nUpdated content.\n";
        fs::write(&file_path, updated).unwrap();
        run(&conn, file_path.to_str().unwrap(), true).unwrap();

        assert_eq!(active_raw_import_count_for_slug(&conn, "people/alice"), 1);
        assert_eq!(
            active_raw_import_bytes_for_slug(&conn, "people/alice"),
            updated.as_bytes()
        );
    }

    #[test]
    fn ingest_refuses_when_any_collection_write_is_blocked() {
        let conn = open_test_db();
        conn.execute(
            "UPDATE collections SET needs_full_sync = 1 WHERE is_write_target = 1",
            [],
        )
        .unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("blocked.md");
        fs::write(&file_path, "---\nslug: blocked\n---\nblocked").unwrap();

        let error = run(&conn, file_path.to_str().unwrap(), false).unwrap_err();

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

    /// Force re-ingest of identical content from a NEW path must refresh the
    /// ingest_log source_ref so that `lookup_source_path` returns the new path.
    #[test]
    fn force_reingest_from_new_path_refreshes_source_mapping() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();

        let path_a = dir.path().join("old-location.md");
        fs::write(
            &path_a,
            "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n",
        )
        .unwrap();
        run(&conn, path_a.to_str().unwrap(), false).unwrap();

        // Verify initial source_ref.
        let initial_ref: String = conn
            .query_row(
                "SELECT source_ref FROM ingest_log WHERE EXISTS ( \
                     SELECT 1 FROM json_each(pages_updated) WHERE value = 'people/alice' \
                 )",
                [],
                |row| row.get(0),
            )
            .expect("initial ingest_log row");
        assert_eq!(initial_ref, path_a.to_str().unwrap());

        // Force re-ingest same content from a different path.
        let path_b = dir.path().join("new-location.md");
        fs::write(
            &path_b,
            "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n",
        )
        .unwrap();
        run(&conn, path_b.to_str().unwrap(), true).unwrap();

        // source_ref must now point to path_b.
        let updated_ref: String = conn
            .query_row(
                "SELECT source_ref FROM ingest_log WHERE EXISTS ( \
                     SELECT 1 FROM json_each(pages_updated) WHERE value = 'people/alice' \
                 )",
                [],
                |row| row.get(0),
            )
            .expect("updated ingest_log row");
        assert_eq!(
            updated_ref,
            path_b.to_str().unwrap(),
            "source_ref must update to the new path after force re-ingest"
        );
    }

    #[test]
    fn ingest_without_memory_id_keeps_source_file_bytes_unchanged() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("no-id.md");
        let original =
            b"---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n";
        fs::write(&file_path, original).unwrap();

        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        assert_eq!(fs::read(&file_path).unwrap(), original);
    }

    #[test]
    fn ingest_preserves_existing_memory_id_in_stored_frontmatter() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("with-id.md");
        fs::write(
            &file_path,
            "---\nquaid_id: 0195c7c0-2d06-7df0-bf59-acde48001122\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n",
        )
        .unwrap();

        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        let frontmatter_json: String = conn
            .query_row(
                "SELECT frontmatter FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let frontmatter: std::collections::HashMap<String, String> =
            serde_json::from_str(&frontmatter_json).unwrap();

        assert_eq!(
            frontmatter.get("quaid_id").map(String::as_str),
            Some("0195c7c0-2d06-7df0-bf59-acde48001122")
        );
    }

    #[test]
    fn ingest_adopts_frontmatter_memory_id_as_page_uuid() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("with-id.md");
        fs::write(
            &file_path,
            "---\nquaid_id: 0195c7c0-2d06-7df0-bf59-acde48001122\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n",
        )
        .unwrap();

        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        let uuid: String = conn
            .query_row(
                "SELECT uuid FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(uuid, "0195c7c0-2d06-7df0-bf59-acde48001122");
    }

    #[test]
    fn ingest_rotates_raw_imports_with_exactly_one_active_row() {
        let conn = open_test_db();
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("note.md");
        fs::write(
            &file_path,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nFirst body.\n",
        )
        .unwrap();

        run(&conn, file_path.to_str().unwrap(), false).unwrap();
        fs::write(
            &file_path,
            "---\nslug: notes/test\ntitle: Test\ntype: concept\n---\nSecond body with a changed revision.\n",
        )
        .unwrap();
        run(&conn, file_path.to_str().unwrap(), true).unwrap();

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

    #[test]
    fn ingest_without_memory_id_generates_uuid_and_keeps_git_clean() {
        let conn = open_test_db();
        let repo_dir = tempfile::TempDir::new().unwrap();
        let file_path = repo_dir.path().join("note.md");
        let original =
            b"---\nslug: people/alice\ntitle: Alice\ntype: person\n---\nAlice is a founder.\n";
        fs::write(&file_path, original).unwrap();

        let git_init = Command::new("git")
            .arg("init")
            .current_dir(repo_dir.path())
            .output()
            .expect("git init should run");
        assert!(
            git_init.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&git_init.stderr)
        );

        let git_add = Command::new("git")
            .args(["add", "."])
            .current_dir(repo_dir.path())
            .output()
            .expect("git add should run");
        assert!(
            git_add.status.success(),
            "git add failed: {}",
            String::from_utf8_lossy(&git_add.stderr)
        );

        let git_config_name = Command::new("git")
            .args(["config", "user.name", "Scruffy"])
            .current_dir(repo_dir.path())
            .output()
            .expect("git config user.name should run");
        assert!(
            git_config_name.status.success(),
            "git config user.name failed: {}",
            String::from_utf8_lossy(&git_config_name.stderr)
        );

        let git_config_email = Command::new("git")
            .args(["config", "user.email", "scruffy@example.com"])
            .current_dir(repo_dir.path())
            .output()
            .expect("git config user.email should run");
        assert!(
            git_config_email.status.success(),
            "git config user.email failed: {}",
            String::from_utf8_lossy(&git_config_email.stderr)
        );

        let git_commit = Command::new("git")
            .args(["commit", "-m", "baseline"])
            .current_dir(repo_dir.path())
            .output()
            .expect("git commit should run");
        assert!(
            git_commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&git_commit.stderr)
        );

        run(&conn, file_path.to_str().unwrap(), false).unwrap();

        let uuid: String = conn
            .query_row(
                "SELECT uuid FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let parsed_uuid = uuid::Uuid::parse_str(&uuid).expect("generated uuid should parse");

        assert_eq!(parsed_uuid.get_version_num(), 7);
        assert_eq!(fs::read(&file_path).unwrap(), original);

        let git_status = Command::new("git")
            .args(["status", "--short"])
            .current_dir(repo_dir.path())
            .output()
            .expect("git status should run");
        assert!(
            git_status.status.success(),
            "git status failed: {}",
            String::from_utf8_lossy(&git_status.stderr)
        );
        assert!(
            String::from_utf8_lossy(&git_status.stdout)
                .trim()
                .is_empty(),
            "ingest should not rewrite the source file or dirty git"
        );
    }
}
