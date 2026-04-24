use anyhow::{bail, Result};
use rusqlite::Connection;
use serde::Serialize;

use crate::core::collections::OpKind;
use crate::core::types::Link;
use crate::core::vault_sync;

// ── slug → page_id resolution ────────────────────────────────

struct ResolvedPage {
    resolved: vault_sync::ResolvedSlug,
    page_id: i64,
}

/// Resolve a slug to its integer page ID. Returns an error if the page doesn't exist.
fn resolve_page(db: &Connection, slug: &str, op_kind: OpKind) -> Result<ResolvedPage> {
    let resolved = vault_sync::resolve_slug_for_op(db, slug, op_kind)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let page_id = db
        .query_row(
            "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
            rusqlite::params![resolved.collection_id, resolved.slug],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("page not found: {slug}"),
            other => anyhow::anyhow!(other),
        })?;
    Ok(ResolvedPage { resolved, page_id })
}

// ── gbrain link ──────────────────────────────────────────────

/// Create a typed temporal link between two pages, or close an existing one.
///
/// If a link already exists matching (from, to, relationship) and `valid_until`
/// is provided, the existing link's `valid_until` is updated (close scenario).
/// Otherwise a new link row is inserted.
pub fn run(
    db: &Connection,
    from: &str,
    to: &str,
    relationship: &str,
    valid_from: Option<String>,
    valid_until: Option<String>,
) -> Result<()> {
    let from_page = resolve_page(db, from, OpKind::WriteUpdate)?;
    let to_page = resolve_page(db, to, OpKind::WriteUpdate)?;
    let closed = run_resolved(
        db,
        &from_page,
        &to_page,
        relationship,
        valid_from,
        valid_until.clone(),
    )?;
    let from_slug = from_page.resolved.canonical_slug();
    let to_slug = to_page.resolved.canonical_slug();
    if closed {
        println!(
            "Closed link {from} → {to} ({relationship}) valid_until={valid_until}",
            from = from_slug,
            to = to_slug,
            valid_until = valid_until.unwrap(),
        );
    } else {
        println!("Linked {from_slug} → {to_slug} ({relationship})");
    }
    Ok(())
}

/// Create or close a link without printing to stdout. Safe to call from MCP handlers.
/// Returns `true` if an existing link was closed, `false` if a new link was created.
pub fn run_silent(
    db: &Connection,
    from: &str,
    to: &str,
    relationship: &str,
    valid_from: Option<String>,
    valid_until: Option<String>,
) -> Result<bool> {
    let from_page = resolve_page(db, from, OpKind::WriteUpdate)?;
    let to_page = resolve_page(db, to, OpKind::WriteUpdate)?;
    run_resolved(
        db,
        &from_page,
        &to_page,
        relationship,
        valid_from,
        valid_until,
    )
}

fn run_resolved(
    db: &Connection,
    from_page: &ResolvedPage,
    to_page: &ResolvedPage,
    relationship: &str,
    valid_from: Option<String>,
    valid_until: Option<String>,
) -> Result<bool> {
    vault_sync::ensure_collection_write_allowed(db, from_page.resolved.collection_id)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    vault_sync::ensure_collection_write_allowed(db, to_page.resolved.collection_id)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    // Close scenario: existing link + valid_until supplied → update.
    if let Some(ref until) = valid_until {
        let rows = db.execute(
            "UPDATE links SET valid_until = ?1 \
             WHERE from_page_id = ?2 AND to_page_id = ?3 AND relationship = ?4 \
               AND valid_until IS NULL",
            rusqlite::params![until, from_page.page_id, to_page.page_id, relationship],
        )?;

        if rows > 0 {
            return Ok(true);
        }
    }

    // Create scenario: insert a new link row.
    db.execute(
        "INSERT INTO links (
            from_page_id, to_page_id, relationship, source_kind, valid_from, valid_until
         ) VALUES (?1, ?2, ?3, 'programmatic', ?4, ?5)",
        rusqlite::params![
            from_page.page_id,
            to_page.page_id,
            relationship,
            valid_from,
            valid_until
        ],
    )?;

    Ok(false)
}

// ── gbrain link-close ────────────────────────────────────────

/// Close a temporal link interval by its database ID.
pub fn close(db: &Connection, link_id: u64, valid_until: &str) -> Result<()> {
    close_silent(db, link_id, valid_until)?;
    println!("Closed link {link_id} valid_until={valid_until}");
    Ok(())
}

/// Close a temporal link by ID without printing to stdout. Safe to call from MCP handlers.
pub fn close_silent(db: &Connection, link_id: u64, valid_until: &str) -> Result<()> {
    let collection_id: i64 = db
        .query_row(
            "SELECT p.collection_id
             FROM links l
             JOIN pages p ON p.id = l.from_page_id
             WHERE l.id = ?1",
            [link_id as i64],
            |row| row.get(0),
        )
        .map_err(|_| anyhow::anyhow!("link not found: id {link_id}"))?;
    vault_sync::ensure_collection_write_allowed(db, collection_id)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let rows = db.execute(
        "UPDATE links SET valid_until = ?1 WHERE id = ?2",
        rusqlite::params![valid_until, link_id],
    )?;

    if rows == 0 {
        bail!("link not found: id {link_id}");
    }

    Ok(())
}

// ── gbrain links ─────────────────────────────────────────────

/// Serialisable link row for JSON output.
#[derive(Debug, Serialize)]
struct LinkRow {
    id: i64,
    to_slug: String,
    relationship: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
}

#[derive(Debug, Serialize)]
struct BacklinkRow {
    id: i64,
    from_slug: String,
    relationship: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
}

/// List all outbound links for a page.
pub fn links(db: &Connection, slug: &str, _temporal: Option<String>, json: bool) -> Result<()> {
    let resolved = resolve_page(db, slug, OpKind::Read)?;

    let mut stmt = db.prepare(
        "SELECT l.id, c.name || '::' || p.slug, l.relationship, l.valid_from, l.valid_until \
         FROM links l \
         JOIN pages p ON l.to_page_id = p.id \
         JOIN collections c ON c.id = p.collection_id \
         WHERE l.from_page_id = ?1 \
         ORDER BY l.created_at DESC",
    )?;

    let rows: Vec<LinkRow> = stmt
        .query_map([resolved.page_id], |row| {
            Ok(LinkRow {
                id: row.get(0)?,
                to_slug: row.get(1)?,
                relationship: row.get(2)?,
                valid_from: row.get(3)?,
                valid_until: row.get(4)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        if rows.is_empty() {
            println!(
                "No outbound links for {}",
                resolved.resolved.canonical_slug()
            );
        }
        for r in &rows {
            let validity = format_validity(&r.valid_from, &r.valid_until);
            println!(
                "[{}] → {} ({}){}",
                r.id, r.to_slug, r.relationship, validity
            );
        }
    }

    Ok(())
}

// ── gbrain unlink ────────────────────────────────────────────

/// Remove a cross-reference entirely.
pub fn unlink(db: &Connection, from: &str, to: &str, relationship: Option<String>) -> Result<()> {
    let from_page = resolve_page(db, from, OpKind::WriteUpdate)?;
    let to_page = resolve_page(db, to, OpKind::WriteUpdate)?;
    vault_sync::ensure_collection_write_allowed(db, from_page.resolved.collection_id)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    vault_sync::ensure_collection_write_allowed(db, to_page.resolved.collection_id)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    let rows = if let Some(ref rel) = relationship {
        db.execute(
            "DELETE FROM links WHERE from_page_id = ?1 AND to_page_id = ?2 AND relationship = ?3",
            rusqlite::params![from_page.page_id, to_page.page_id, rel],
        )?
    } else {
        db.execute(
            "DELETE FROM links WHERE from_page_id = ?1 AND to_page_id = ?2",
            rusqlite::params![from_page.page_id, to_page.page_id],
        )?
    };

    if rows == 0 {
        bail!(
            "no matching link found between {} and {}",
            from_page.resolved.canonical_slug(),
            to_page.resolved.canonical_slug()
        );
    }

    println!(
        "Removed {rows} link(s) {} → {}",
        from_page.resolved.canonical_slug(),
        to_page.resolved.canonical_slug()
    );
    Ok(())
}

// ── gbrain backlinks ─────────────────────────────────────────

/// List backlinks (inbound links) for a page.
pub fn backlinks(db: &Connection, slug: &str, _temporal: Option<String>, json: bool) -> Result<()> {
    let resolved = resolve_page(db, slug, OpKind::Read)?;

    let mut stmt = db.prepare(
        "SELECT l.id, c.name || '::' || p.slug, l.relationship, l.valid_from, l.valid_until \
         FROM links l \
         JOIN pages p ON l.from_page_id = p.id \
         JOIN collections c ON c.id = p.collection_id \
         WHERE l.to_page_id = ?1 \
         ORDER BY l.created_at DESC",
    )?;

    let rows: Vec<BacklinkRow> = stmt
        .query_map([resolved.page_id], |row| {
            Ok(BacklinkRow {
                id: row.get(0)?,
                from_slug: row.get(1)?,
                relationship: row.get(2)?,
                valid_from: row.get(3)?,
                valid_until: row.get(4)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        if rows.is_empty() {
            println!("No backlinks for {}", resolved.resolved.canonical_slug());
        }
        for r in &rows {
            let validity = format_validity(&r.valid_from, &r.valid_until);
            println!(
                "[{}] ← {} ({}){}",
                r.id, r.from_slug, r.relationship, validity
            );
        }
    }

    Ok(())
}

/// Format validity range for display.
fn format_validity(from: &Option<String>, until: &Option<String>) -> String {
    match (from, until) {
        (Some(f), Some(u)) => format!(" [{f}..{u}]"),
        (Some(f), None) => format!(" [{f}..]"),
        (None, Some(u)) => format!(" [..{u}]"),
        (None, None) => String::new(),
    }
}

// ── helper: read a Link struct back from DB ──────────────────

/// Read a link by its database ID, resolving page IDs back to slugs.
#[allow(dead_code)]
pub fn get_link(db: &Connection, link_id: i64) -> Result<Link> {
    let link = db.query_row(
        "SELECT l.id, cf.name || '::' || pf.slug, ct.name || '::' || pt.slug, l.relationship, l.context, \
                l.valid_from, l.valid_until, l.created_at \
          FROM links l \
          JOIN pages pf ON l.from_page_id = pf.id \
          JOIN collections cf ON cf.id = pf.collection_id \
          JOIN pages pt ON l.to_page_id = pt.id \
          JOIN collections ct ON ct.id = pt.collection_id \
          WHERE l.id = ?1",
        [link_id],
        |row| {
            Ok(Link {
                id: Some(row.get(0)?),
                from_slug: row.get(1)?,
                to_slug: row.get(2)?,
                relationship: row.get(3)?,
                context: row.get(4)?,
                valid_from: row.get(5)?,
                valid_until: row.get(6)?,
                created_at: row.get(7)?,
            })
        },
    )?;
    Ok(link)
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

    fn insert_page(conn: &Connection, slug: &str, page_type: &str) {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, ?2, ?3, '', '', '', '{}', '', '', 1)",
            rusqlite::params![slug, page_type, slug],
        )
        .unwrap();
    }

    // ── create link ──────────────────────────────────────────

    #[test]
    fn create_link_inserts_row_into_links_table() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person");
        insert_page(&conn, "companies/acme", "company");

        run(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            Some("2024-01".to_string()),
            None,
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM links", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let link = get_link(&conn, 1).unwrap();
        assert_eq!(link.from_slug, "default::people/alice");
        assert_eq!(link.to_slug, "default::companies/acme");
        assert_eq!(link.relationship, "works_at");
        assert_eq!(link.valid_from.as_deref(), Some("2024-01"));
        assert!(link.valid_until.is_none());
    }

    #[test]
    fn create_link_marks_row_as_programmatic() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person");
        insert_page(&conn, "companies/acme", "company");

        run(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            None,
            None,
        )
        .unwrap();

        let source_kind: String = conn
            .query_row("SELECT source_kind FROM links WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(source_kind, "programmatic");
    }

    // ── close link ───────────────────────────────────────────

    #[test]
    fn close_link_sets_valid_until_on_existing_link() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person");
        insert_page(&conn, "companies/acme", "company");

        // Create
        run(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            Some("2024-01".to_string()),
            None,
        )
        .unwrap();

        // Close via the same command with --valid-until
        run(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            None,
            Some("2025-06".to_string()),
        )
        .unwrap();

        let link = get_link(&conn, 1).unwrap();
        assert_eq!(link.valid_until.as_deref(), Some("2025-06"));

        // Only one link row — close updated in place, didn't create a second
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM links", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    // ── link-close by ID ─────────────────────────────────────

    #[test]
    fn link_close_by_id_sets_valid_until() {
        let conn = open_test_db();
        insert_page(&conn, "people/bob", "person");
        insert_page(&conn, "companies/beta", "company");

        run(&conn, "people/bob", "companies/beta", "advises", None, None).unwrap();

        close(&conn, 1, "2025-12").unwrap();

        let link = get_link(&conn, 1).unwrap();
        assert_eq!(link.valid_until.as_deref(), Some("2025-12"));
    }

    #[test]
    fn link_close_returns_error_for_nonexistent_id() {
        let conn = open_test_db();
        let result = close(&conn, 999, "2025-01");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("link not found"));
    }

    // ── page not found ───────────────────────────────────────

    #[test]
    fn link_fails_when_from_page_does_not_exist() {
        let conn = open_test_db();
        insert_page(&conn, "companies/acme", "company");

        let result = run(
            &conn,
            "people/ghost",
            "companies/acme",
            "works_at",
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("page not found"));
    }

    #[test]
    fn link_fails_when_to_page_does_not_exist() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person");

        let result = run(
            &conn,
            "people/alice",
            "companies/ghost",
            "works_at",
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("page not found"));
    }

    #[test]
    fn create_link_refuses_when_collection_needs_full_sync_even_if_not_restoring() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person");
        insert_page(&conn, "companies/acme", "company");
        conn.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = 1",
            [],
        )
        .unwrap();

        let error = run(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            None,
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("CollectionRestoringError"));
    }

    // ── unlink ───────────────────────────────────────────────

    #[test]
    fn unlink_removes_link_row() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person");
        insert_page(&conn, "companies/acme", "company");

        run(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM links", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );

        unlink(
            &conn,
            "people/alice",
            "companies/acme",
            Some("works_at".to_string()),
        )
        .unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM links", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    // ── links / backlinks ────────────────────────────────────

    #[test]
    fn links_lists_outbound_and_backlinks_lists_inbound() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person");
        insert_page(&conn, "companies/acme", "company");

        run(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            None,
            None,
        )
        .unwrap();

        // links (outbound from alice) — should succeed without panic
        links(&conn, "people/alice", None, false).unwrap();
        // backlinks (inbound to acme) — should succeed without panic
        backlinks(&conn, "companies/acme", None, false).unwrap();
    }
}
