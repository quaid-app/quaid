use anyhow::Result;
use rusqlite::Connection;

use crate::core::collections::OpKind;
use crate::core::vault_sync;

/// Manage tags on a page: list, add, or remove.
///
/// Tags live in the `tags` table — no OCC, no page version bump.
/// Without `--add` or `--remove`, prints current tags one per line.
pub fn run(db: &Connection, slug: &str, add: &[String], remove: &[String]) -> Result<()> {
    let resolved = vault_sync::resolve_slug_for_op(db, slug, OpKind::WriteUpdate)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    if !add.is_empty() || !remove.is_empty() {
        vault_sync::ensure_collection_write_allowed(db, resolved.collection_id)
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    }
    let page_id = resolve_page_id(db, resolved.collection_id, &resolved.slug)?;

    for tag in add {
        db.execute(
            "INSERT OR IGNORE INTO tags (page_id, tag) VALUES (?1, ?2)",
            rusqlite::params![page_id, tag],
        )?;
    }

    for tag in remove {
        db.execute(
            "DELETE FROM tags WHERE page_id = ?1 AND tag = ?2",
            rusqlite::params![page_id, tag],
        )?;
    }

    if add.is_empty() && remove.is_empty() {
        let tags = list_tags(db, page_id)?;
        for tag in &tags {
            println!("{tag}");
        }
    }

    Ok(())
}

fn resolve_page_id(db: &Connection, collection_id: i64, slug: &str) -> Result<i64> {
    db.query_row(
        "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
        rusqlite::params![collection_id, slug],
        |row| row.get(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("page not found: {slug}"),
        other => other.into(),
    })
}

fn list_tags(db: &Connection, page_id: i64) -> Result<Vec<String>> {
    let mut stmt = db.prepare("SELECT tag FROM tags WHERE page_id = ?1 ORDER BY tag")?;
    let tags = stmt
        .query_map([page_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(tags)
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

    fn insert_page(conn: &Connection, slug: &str) {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, 'person', ?1, '', '', '', '{}', 'people', '', 1)",
            [slug],
        )
        .unwrap();
    }

    #[test]
    fn list_tags_returns_empty_for_untagged_page() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice");
        let page_id = resolve_page_id(&conn, 1, "people/alice").unwrap();

        let tags = list_tags(&conn, page_id).unwrap();
        assert!(tags.is_empty());
    }

    #[test]
    fn add_then_list_returns_added_tags() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice");

        run(&conn, "people/alice", &["investor".into()], &[]).unwrap();

        let page_id = resolve_page_id(&conn, 1, "people/alice").unwrap();
        let tags = list_tags(&conn, page_id).unwrap();
        assert_eq!(tags, vec!["investor"]);
    }

    #[test]
    fn add_duplicate_tag_is_idempotent() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice");

        run(&conn, "people/alice", &["investor".into()], &[]).unwrap();
        run(&conn, "people/alice", &["investor".into()], &[]).unwrap();

        let page_id = resolve_page_id(&conn, 1, "people/alice").unwrap();
        let tags = list_tags(&conn, page_id).unwrap();
        assert_eq!(tags, vec!["investor"]);
    }

    #[test]
    fn remove_tag_deletes_it() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice");

        run(
            &conn,
            "people/alice",
            &["investor".into(), "founder".into()],
            &[],
        )
        .unwrap();
        run(&conn, "people/alice", &[], &["investor".into()]).unwrap();

        let page_id = resolve_page_id(&conn, 1, "people/alice").unwrap();
        let tags = list_tags(&conn, page_id).unwrap();
        assert_eq!(tags, vec!["founder"]);
    }

    #[test]
    fn remove_nonexistent_tag_is_noop() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice");

        let result = run(&conn, "people/alice", &[], &["ghost".into()]);
        assert!(result.is_ok());
    }

    #[test]
    fn tags_on_nonexistent_page_returns_error() {
        let conn = open_test_db();

        let result = run(&conn, "people/nobody", &["tag".into()], &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("page not found"));
    }

    #[test]
    fn tags_do_not_bump_page_version() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice");

        let version_before: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        run(&conn, "people/alice", &["investor".into()], &[]).unwrap();

        let version_after: i64 = conn
            .query_row(
                "SELECT version FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(version_before, version_after);
    }

    #[test]
    fn list_tags_returns_alphabetical_order() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice");

        run(
            &conn,
            "people/alice",
            &["zebra".into(), "alpha".into(), "mid".into()],
            &[],
        )
        .unwrap();

        let page_id = resolve_page_id(&conn, 1, "people/alice").unwrap();
        let tags = list_tags(&conn, page_id).unwrap();
        assert_eq!(tags, vec!["alpha", "mid", "zebra"]);
    }

    #[test]
    fn add_tags_refuses_when_collection_needs_full_sync_even_if_not_restoring() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice");
        conn.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = 1",
            [],
        )
        .unwrap();

        let error = run(&conn, "people/alice", &["investor".into()], &[]).unwrap_err();

        assert!(error.to_string().contains("CollectionRestoringError"));
    }
}
