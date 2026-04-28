use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::core::collections::OpKind;
use crate::core::vault_sync;

#[derive(Debug, Serialize)]
struct TimelineOutput {
    slug: String,
    entries: Vec<String>,
}

/// Show timeline entries for a page from the `timeline_entries` table,
/// with legacy fallback to the page's `timeline` markdown field.
pub fn run(db: &Connection, slug: &str, limit: u32, json: bool) -> Result<()> {
    let resolved = vault_sync::resolve_slug_for_op(db, slug, OpKind::Read)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let canonical_slug = resolved.canonical_slug();
    let page = crate::commands::get::get_page_by_key(db, resolved.collection_id, &resolved.slug)?;

    let page_id: i64 = db
        .query_row(
            "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
            rusqlite::params![resolved.collection_id, &resolved.slug],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("page not found: {slug}"),
            other => other.into(),
        })?;

    // Query structured timeline_entries table
    let mut stmt = db.prepare(
        "SELECT date, summary, source, detail FROM timeline_entries \
         WHERE page_id = ?1 ORDER BY date DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![page_id, limit], |row| {
        let date: String = row.get(0)?;
        let summary: String = row.get(1)?;
        let source: String = row.get(2)?;
        let detail: String = row.get(3)?;
        let mut entry = format!("{date}: {summary}");
        if !source.is_empty() {
            entry.push_str(&format!(" [source: {source}]"));
        }
        if !detail.is_empty() {
            entry.push_str(&format!("\n{detail}"));
        }
        Ok(entry)
    })?;

    let mut entries: Vec<String> = Vec::new();
    for row in rows {
        entries.push(row?);
    }

    // Fall back to legacy timeline markdown field if no structured entries exist
    if entries.is_empty() {
        let timeline = page.timeline.trim();
        if !timeline.is_empty() {
            entries = split_timeline(timeline)
                .into_iter()
                .take(limit as usize)
                .collect();
        }
    }

    if entries.is_empty() {
        if json {
            let output = TimelineOutput {
                slug: canonical_slug,
                entries: Vec::new(),
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("No timeline entries for {canonical_slug}");
        }
        return Ok(());
    }

    if json {
        let output = TimelineOutput {
            slug: canonical_slug,
            entries,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        for entry in &entries {
            println!("{entry}");
            println!();
        }
    }

    Ok(())
}

pub fn add(
    db: &Connection,
    slug: &str,
    date: &str,
    summary: &str,
    source: Option<String>,
    detail: Option<String>,
) -> Result<()> {
    let resolved = vault_sync::resolve_slug_for_op(db, slug, OpKind::WriteUpdate)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let canonical_slug = resolved.canonical_slug();
    vault_sync::ensure_collection_write_allowed(db, resolved.collection_id)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let page_id: i64 = db
        .query_row(
            "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
            rusqlite::params![resolved.collection_id, &resolved.slug],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("page not found: {slug}"),
            other => other.into(),
        })?;

    let summary_hash = {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(summary.as_bytes());
        format!("{digest:x}")
    };

    db.execute(
        "INSERT INTO timeline_entries (page_id, date, source, summary, summary_hash, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            page_id,
            date,
            source.as_deref().unwrap_or(""),
            summary,
            summary_hash,
            detail.as_deref().unwrap_or(""),
        ],
    )?;

    println!("Added timeline entry for {canonical_slug}");
    Ok(())
}

/// Split timeline markdown into individual entries separated by `---` lines.
fn split_timeline(timeline: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = Vec::new();

    for line in timeline.lines() {
        if line.trim() == "---" {
            if !current.is_empty() {
                entries.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line.to_string());
        }
    }

    if !current.is_empty() {
        entries.push(current.join("\n"));
    }

    entries
        .into_iter()
        .filter(|e| !e.trim().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;
    use rusqlite::Connection;

    fn open_test_db() -> Connection {
        db::open(":memory:").unwrap()
    }

    fn insert_page(conn: &Connection, slug: &str) {
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'note', ?1, '', '', '', '{}', '', '', 1)",
            rusqlite::params![slug, uuid::Uuid::now_v7().to_string()],
        )
        .unwrap();
    }

    fn activate_collection(conn: &Connection) {
        conn.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 0 WHERE id = 1",
            [],
        )
        .unwrap();
    }

    #[test]
    fn split_timeline_separates_on_bare_boundary() {
        let entries = split_timeline("entry one\n---\nentry two");
        assert_eq!(entries, vec!["entry one", "entry two"]);
    }

    #[test]
    fn split_timeline_single_entry_no_boundary() {
        let entries = split_timeline("just one entry");
        assert_eq!(entries, vec!["just one entry"]);
    }

    #[test]
    fn split_timeline_empty_returns_empty() {
        let entries = split_timeline("");
        assert!(entries.is_empty());
    }

    #[test]
    fn add_refuses_when_collection_needs_full_sync_even_if_not_restoring() {
        let conn = open_test_db();
        insert_page(&conn, "notes/alice");
        conn.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = 1",
            [],
        )
        .unwrap();

        let error = add(&conn, "notes/alice", "2026-04-22", "blocked", None, None).unwrap_err();

        assert!(error.to_string().contains("CollectionRestoringError"));
    }

    #[test]
    fn run_no_entries_text_output() {
        let conn = open_test_db();
        activate_collection(&conn);
        insert_page(&conn, "notes/empty");
        run(&conn, "notes/empty", 10, false).expect("run with no entries");
    }

    #[test]
    fn run_no_entries_json_output() {
        let conn = open_test_db();
        activate_collection(&conn);
        insert_page(&conn, "notes/empty-json");
        run(&conn, "notes/empty-json", 10, true).expect("run json with no entries");
    }

    #[test]
    fn run_with_structured_entry_text_output() {
        let conn = open_test_db();
        activate_collection(&conn);
        insert_page(&conn, "notes/structured");
        add(
            &conn,
            "notes/structured",
            "2026-04-01",
            "something happened",
            None,
            None,
        )
        .expect("add entry");
        run(&conn, "notes/structured", 10, false).expect("run with structured entry");
    }

    #[test]
    fn run_with_structured_entry_json_output() {
        let conn = open_test_db();
        activate_collection(&conn);
        insert_page(&conn, "notes/structured-json");
        add(
            &conn,
            "notes/structured-json",
            "2026-04-01",
            "event text",
            Some("src".to_owned()),
            Some("some detail".to_owned()),
        )
        .expect("add entry");
        run(&conn, "notes/structured-json", 10, true)
            .expect("run json with structured entry");
    }

    #[test]
    fn run_with_legacy_timeline_markdown() {
        let conn = open_test_db();
        activate_collection(&conn);
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES ('notes/legacy', ?1, 'note', 'legacy', '', '', 'event alpha\n---\nevent beta', '{}', '', '', 1)",
            [uuid::Uuid::now_v7().to_string()],
        )
        .unwrap();
        run(&conn, "notes/legacy", 10, false).expect("run with legacy timeline");
    }

    #[test]
    fn add_with_source_and_detail_inserts_entry() {
        let conn = open_test_db();
        activate_collection(&conn);
        insert_page(&conn, "notes/sourced");
        add(
            &conn,
            "notes/sourced",
            "2026-04-01",
            "something",
            Some("test-src".to_owned()),
            Some("some detail".to_owned()),
        )
        .expect("add with source and detail");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM timeline_entries WHERE source = 'test-src'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
