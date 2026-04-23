use anyhow::{anyhow, Result};
use rusqlite::Connection;

use crate::commands::get::get_page;
use crate::core::assertions::{self, Contradiction};
use crate::core::collections::OpKind;
use crate::core::vault_sync;

#[derive(Debug)]
pub struct CheckReport {
    contradictions: Vec<Contradiction>,
    processed_pages: usize,
}

pub fn run(
    db: &Connection,
    slug: Option<String>,
    all: bool,
    check_type: Option<String>,
    json: bool,
) -> Result<()> {
    let report = execute_check(db, slug.as_deref(), all, check_type.as_deref())?;
    let output = render_output(&report, json)?;

    println!("{output}");

    Ok(())
}

/// Run structured assertion extraction and contradiction detection without printing. Safe to call from MCP.
pub fn execute_check(
    db: &Connection,
    slug: Option<&str>,
    all: bool,
    check_type: Option<&str>,
) -> Result<CheckReport> {
    if let Some(page_slug) = slug {
        let resolved = vault_sync::resolve_slug_for_op(db, page_slug, OpKind::WriteUpdate)
            .map_err(|err| anyhow!(err.to_string()))?;
        vault_sync::ensure_collection_write_allowed(db, resolved.collection_id)
            .map_err(|err| anyhow!(err.to_string()))?;
    } else if all {
        vault_sync::ensure_all_collections_write_allowed(db)
            .map_err(|err| anyhow!(err.to_string()))?;
    }
    let slugs = resolve_targets(db, slug, all)?;

    for page_slug in &slugs {
        let page = get_page(db, page_slug)?;
        assertions::extract_assertions(&page, db)?;
        assertions::check_assertions(page_slug, db)?;
    }

    let contradictions = fetch_unresolved_contradictions(db, slug, all, check_type)?;

    Ok(CheckReport {
        contradictions,
        processed_pages: slugs.len(),
    })
}

fn resolve_targets(db: &Connection, slug: Option<&str>, all: bool) -> Result<Vec<String>> {
    if all {
        let mut statement = db.prepare("SELECT slug FROM pages ORDER BY slug")?;
        let slugs = statement
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        return Ok(slugs);
    }

    match slug {
        Some(page_slug) => {
            get_page(db, page_slug)?;
            Ok(vec![page_slug.to_string()])
        }
        None => Err(anyhow!("provide a slug or pass --all")),
    }
}

fn fetch_unresolved_contradictions(
    db: &Connection,
    slug: Option<&str>,
    all: bool,
    check_type: Option<&str>,
) -> Result<Vec<Contradiction>> {
    let base_sql = "SELECT p.slug,
                           COALESCE(other.slug, p.slug),
                           c.type,
                           c.description,
                           c.detected_at
                    FROM contradictions c
                    JOIN pages p ON p.id = c.page_id
                    LEFT JOIN pages other ON other.id = c.other_page_id
                    WHERE c.resolved_at IS NULL";

    let contradictions = match (all, slug, check_type) {
        (true, _, Some(check_kind)) => query_contradictions(
            db,
            format!("{base_sql} AND c.type = ?1 ORDER BY c.detected_at, p.slug, other.slug"),
            rusqlite::params![check_kind],
        )?,
        (true, _, None) => query_contradictions(
            db,
            format!("{base_sql} ORDER BY c.detected_at, p.slug, other.slug"),
            [],
        )?,
        (false, Some(page_slug), Some(check_kind)) => query_contradictions(
            db,
            format!(
                "{base_sql} AND (p.slug = ?1 OR other.slug = ?1) AND c.type = ?2 \
                 ORDER BY c.detected_at, p.slug, other.slug"
            ),
            rusqlite::params![page_slug, check_kind],
        )?,
        (false, Some(page_slug), None) => query_contradictions(
            db,
            format!(
                "{base_sql} AND (p.slug = ?1 OR other.slug = ?1) \
                 ORDER BY c.detected_at, p.slug, other.slug"
            ),
            [page_slug],
        )?,
        (false, None, _) => Vec::new(),
    };

    Ok(contradictions)
}

fn query_contradictions<P>(db: &Connection, sql: String, params: P) -> Result<Vec<Contradiction>>
where
    P: rusqlite::Params,
{
    let mut statement = db.prepare(&sql)?;
    let contradictions = statement
        .query_map(params, |row| {
            Ok(Contradiction {
                page_slug: row.get(0)?,
                other_page_slug: row.get(1)?,
                r#type: row.get(2)?,
                description: row.get(3)?,
                detected_at: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(contradictions)
}

fn render_output(report: &CheckReport, json: bool) -> Result<String> {
    if json {
        return Ok(serde_json::to_string_pretty(&report.contradictions)?);
    }

    let mut lines = Vec::new();

    if report.contradictions.is_empty() {
        lines.push("No contradictions found.".to_string());
    } else {
        lines.extend(report.contradictions.iter().map(|contradiction| {
            format!(
                "[{}] ↔ [{}]: {}",
                contradiction.page_slug, contradiction.other_page_slug, contradiction.description
            )
        }));
    }

    if report.processed_pages > 1 {
        lines.push(format!(
            "{} contradiction(s) found across {} pages.",
            report.contradictions.len(),
            report.processed_pages
        ));
    } else {
        lines.push(format!(
            "{} contradiction(s) found.",
            report.contradictions.len()
        ));
    }

    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        db::open(":memory:").unwrap()
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

    fn insert_page(conn: &Connection, slug: &str, truth: &str) {
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'person', ?3, '', ?4, '', '{}', 'people', '', 1)",
            rusqlite::params![slug, test_uuid(slug), slug, truth],
        )
        .unwrap();
    }

    fn insert_manual_assertion(
        conn: &Connection,
        slug: &str,
        subject: &str,
        predicate: &str,
        object: &str,
    ) {
        let page_id: i64 = conn
            .query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
                row.get(0)
            })
            .unwrap();

        conn.execute(
            "INSERT INTO assertions (
                page_id, subject, predicate, object, valid_from, valid_until,
                confidence, asserted_by, source_ref, evidence_text
             ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, 1.0, 'manual', '', '')",
            rusqlite::params![page_id, subject, predicate, object],
        )
        .unwrap();
    }

    #[test]
    fn single_page_check_returns_existing_unresolved_contradiction() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "Alice biography.");
        insert_manual_assertion(&conn, "people/alice", "Alice", "employer", "Acme");
        insert_manual_assertion(&conn, "people/alice", "Alice", "employer", "Beta");
        assertions::check_assertions("people/alice", &conn).unwrap();

        let report = execute_check(&conn, Some("people/alice"), false, None).unwrap();

        assert_eq!(report.processed_pages, 1);
        assert_eq!(report.contradictions.len(), 1);
        assert_eq!(report.contradictions[0].page_slug, "people/alice");
    }

    #[test]
    fn all_mode_processes_multiple_pages() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "## Assertions\nAlice works at Acme Corp.\n",
        );
        insert_page(
            &conn,
            "sources/alice-profile",
            "## Assertions\nAlice works at Beta Corp.\n",
        );

        let report = execute_check(&conn, None, true, None).unwrap();

        assert_eq!(report.processed_pages, 2);
        assert_eq!(report.contradictions.len(), 1);
        assert_eq!(
            report.contradictions[0].other_page_slug,
            "sources/alice-profile"
        );
    }

    #[test]
    fn json_output_is_valid() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "## Assertions\nAlice works at Acme Corp.\n",
        );
        insert_page(
            &conn,
            "sources/alice-profile",
            "## Assertions\nAlice works at Beta Corp.\n",
        );
        let report = execute_check(&conn, None, true, None).unwrap();

        let json = render_output(&report, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(value.is_array());
        assert_eq!(value[0]["page_slug"], "people/alice");
        assert_eq!(value[0]["other_page_slug"], "sources/alice-profile");
        assert_eq!(value[0]["type"], "assertion_conflict");
    }

    #[test]
    fn missing_slug_returns_error() {
        let conn = open_test_db();

        let error = execute_check(&conn, Some("people/missing"), false, None).unwrap_err();

        assert!(error.to_string().contains("page not found"));
    }

    #[test]
    fn human_output_includes_contradiction_summary_for_all_mode() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "## Assertions\nAlice works at Acme Corp.\n",
        );
        insert_page(
            &conn,
            "sources/alice-profile",
            "## Assertions\nAlice works at Beta Corp.\n",
        );
        let report = execute_check(&conn, None, true, None).unwrap();

        let output = render_output(&report, false).unwrap();

        assert!(output.contains("[people/alice] ↔ [sources/alice-profile]"));
        assert!(output.contains("1 contradiction(s) found across 2 pages."));
    }

    #[test]
    fn all_mode_refuses_when_collection_needs_full_sync_even_if_not_restoring() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "## Assertions\nAlice works at Acme Corp.\n",
        );
        conn.execute(
            "UPDATE collections SET state = 'active', needs_full_sync = 1 WHERE id = 1",
            [],
        )
        .unwrap();

        let error = execute_check(&conn, None, true, None).unwrap_err();

        assert!(error.to_string().contains("CollectionRestoringError"));
    }
}
