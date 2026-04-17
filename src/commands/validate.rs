use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

/// A single integrity violation found during validation.
#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub check: String,
    #[serde(rename = "type")]
    pub violation_type: String,
    pub details: serde_json::Value,
}

/// Summary of a validation run.
#[derive(Debug, Serialize)]
pub struct ValidateReport {
    pub passed: bool,
    pub checks: Vec<String>,
    pub violations: Vec<Violation>,
}

/// Which individual checks to run.
pub struct CheckFlags {
    pub links: bool,
    pub assertions: bool,
    pub embeddings: bool,
}

impl CheckFlags {
    /// All checks enabled (default / --all).
    pub fn all() -> Self {
        Self {
            links: true,
            assertions: true,
            embeddings: true,
        }
    }
}

pub fn run(db: &Connection, flags: &CheckFlags, json: bool) -> Result<()> {
    let report = execute_validate(db, flags)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if report.passed {
        println!("All checks passed.");
    } else {
        for v in &report.violations {
            println!("[{}] {}: {}", v.check, v.violation_type, v.details);
        }
        println!(
            "{} violation(s) found across checks: {}.",
            report.violations.len(),
            report.checks.join(", ")
        );
    }

    if report.passed {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

/// Run selected integrity checks and return a report. Usable from MCP/call.
pub fn execute_validate(db: &Connection, flags: &CheckFlags) -> Result<ValidateReport> {
    let mut checks = Vec::new();
    let mut violations = Vec::new();

    if flags.links {
        checks.push("links".to_string());
        check_links(db, &mut violations)?;
    }
    if flags.assertions {
        checks.push("assertions".to_string());
        check_assertions(db, &mut violations)?;
    }
    if flags.embeddings {
        checks.push("embeddings".to_string());
        check_embeddings(db, &mut violations)?;
    }

    let passed = violations.is_empty();
    Ok(ValidateReport {
        passed,
        checks,
        violations,
    })
}

// ── Link checks ─────────────────────────────────────────────

fn check_links(db: &Connection, violations: &mut Vec<Violation>) -> Result<()> {
    // 1. Referential integrity: from_page_id exists
    let mut stmt = db.prepare(
        "SELECT l.id, l.from_page_id FROM links l \
         WHERE NOT EXISTS (SELECT 1 FROM pages p WHERE p.id = l.from_page_id)",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (link_id, page_id) = row?;
        violations.push(Violation {
            check: "links".into(),
            violation_type: "dangling_from_page".into(),
            details: serde_json::json!({
                "link_id": link_id,
                "from_page_id": page_id,
            }),
        });
    }

    // 2. Referential integrity: to_page_id exists
    let mut stmt = db.prepare(
        "SELECT l.id, l.to_page_id FROM links l \
         WHERE NOT EXISTS (SELECT 1 FROM pages p WHERE p.id = l.to_page_id)",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (link_id, page_id) = row?;
        violations.push(Violation {
            check: "links".into(),
            violation_type: "dangling_to_page".into(),
            details: serde_json::json!({
                "link_id": link_id,
                "to_page_id": page_id,
            }),
        });
    }

    // 3. Temporal ordering: valid_from <= valid_until
    // (Schema CHECK constraint covers this, but validate anyway for corrupted data)
    let mut stmt = db.prepare(
        "SELECT id, valid_from, valid_until FROM links \
         WHERE valid_from IS NOT NULL AND valid_until IS NOT NULL AND valid_from > valid_until",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (id, from, until) = row?;
        violations.push(Violation {
            check: "links".into(),
            violation_type: "invalid_temporal_order".into(),
            details: serde_json::json!({
                "link_id": id,
                "valid_from": from,
                "valid_until": until,
            }),
        });
    }

    // 4. Overlapping intervals for same from/to/relationship
    let mut stmt = db.prepare(
        "SELECT a.id, b.id, a.from_page_id, a.to_page_id, a.relationship \
         FROM links a JOIN links b ON \
           a.from_page_id = b.from_page_id AND a.to_page_id = b.to_page_id \
           AND a.relationship = b.relationship AND a.id < b.id \
         WHERE \
           (COALESCE(a.valid_from, '') < COALESCE(b.valid_until, '9999-12-31')) \
           AND (COALESCE(b.valid_from, '') < COALESCE(a.valid_until, '9999-12-31'))",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (id_a, id_b, from_page, to_page, rel) = row?;
        violations.push(Violation {
            check: "links".into(),
            violation_type: "overlapping_interval".into(),
            details: serde_json::json!({
                "link_id_a": id_a,
                "link_id_b": id_b,
                "from_page_id": from_page,
                "to_page_id": to_page,
                "relationship": rel,
            }),
        });
    }

    Ok(())
}

// ── Assertion checks ─────────────────────────────────────────

fn check_assertions(db: &Connection, violations: &mut Vec<Violation>) -> Result<()> {
    // 1. Dangling page_id
    let mut stmt = db.prepare(
        "SELECT a.id, a.page_id FROM assertions a \
         WHERE NOT EXISTS (SELECT 1 FROM pages p WHERE p.id = a.page_id)",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (id, page_id) = row?;
        violations.push(Violation {
            check: "assertions".into(),
            violation_type: "dangling_page".into(),
            details: serde_json::json!({
                "assertion_id": id,
                "page_id": page_id,
            }),
        });
    }

    // 2. Dangling supersedes_id
    let mut stmt = db.prepare(
        "SELECT a.id, a.supersedes_id FROM assertions a \
         WHERE a.supersedes_id IS NOT NULL \
         AND NOT EXISTS (SELECT 1 FROM assertions b WHERE b.id = a.supersedes_id)",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (id, supersedes_id) = row?;
        violations.push(Violation {
            check: "assertions".into(),
            violation_type: "dangling_supersedes".into(),
            details: serde_json::json!({
                "assertion_id": id,
                "supersedes_id": supersedes_id,
            }),
        });
    }

    // 3. Duplicate subject+predicate+object with overlapping validity
    let mut stmt = db.prepare(
        "SELECT a.id, b.id, a.subject, a.predicate, a.object \
         FROM assertions a JOIN assertions b ON \
           a.subject = b.subject AND a.predicate = b.predicate AND a.object = b.object \
           AND a.id < b.id \
         WHERE \
           (COALESCE(a.valid_from, '') < COALESCE(b.valid_until, '9999-12-31')) \
           AND (COALESCE(b.valid_from, '') < COALESCE(a.valid_until, '9999-12-31'))",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (id_a, id_b, subj, pred, obj) = row?;
        violations.push(Violation {
            check: "assertions".into(),
            violation_type: "duplicate_assertion".into(),
            details: serde_json::json!({
                "assertion_id_a": id_a,
                "assertion_id_b": id_b,
                "subject": subj,
                "predicate": pred,
                "object": obj,
            }),
        });
    }

    Ok(())
}

// ── Embedding checks ─────────────────────────────────────────

fn check_embeddings(db: &Connection, violations: &mut Vec<Violation>) -> Result<()> {
    // 1. Exactly one active model
    let active_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM embedding_models WHERE active = 1",
        [],
        |row| row.get(0),
    )?;
    if active_count != 1 {
        violations.push(Violation {
            check: "embeddings".into(),
            violation_type: "active_model_count".into(),
            details: serde_json::json!({
                "expected": 1,
                "actual": active_count,
            }),
        });
        // If no active model, can't check further
        if active_count == 0 {
            return Ok(());
        }
    }

    // 2. All page_embeddings reference the active model
    let (active_model, vec_table): (String, String) = db.query_row(
        "SELECT name, vec_table FROM embedding_models WHERE active = 1 LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let stale_model_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM page_embeddings WHERE model != ?1",
        [&active_model],
        |row| row.get(0),
    )?;
    if stale_model_count > 0 {
        violations.push(Violation {
            check: "embeddings".into(),
            violation_type: "stale_model_embeddings".into(),
            details: serde_json::json!({
                "active_model": active_model,
                "stale_embedding_count": stale_model_count,
            }),
        });
    }

    // 3. All page_embeddings.page_id reference existing pages
    let orphan_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM page_embeddings pe \
         WHERE NOT EXISTS (SELECT 1 FROM pages p WHERE p.id = pe.page_id)",
        [],
        |row| row.get(0),
    )?;
    if orphan_count > 0 {
        violations.push(Violation {
            check: "embeddings".into(),
            violation_type: "orphaned_embeddings".into(),
            details: serde_json::json!({
                "orphan_count": orphan_count,
            }),
        });
    }

    // 4. All vec_rowids resolve in the active model's vec table
    if !is_safe_identifier(&vec_table) {
        violations.push(Violation {
            check: "embeddings".into(),
            violation_type: "unsafe_vec_table".into(),
            details: serde_json::json!({
                "vec_table": vec_table.as_str(),
            }),
        });
        return Ok(());
    }

    let sql = format!(
        "SELECT pe.id, pe.vec_rowid FROM page_embeddings pe \
         LEFT JOIN {vec_table} v ON v.rowid = pe.vec_rowid \
         WHERE pe.model = ?1 AND v.rowid IS NULL"
    );
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map([&active_model], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (id, vec_rowid) = row?;
        violations.push(Violation {
            check: "embeddings".into(),
            violation_type: "stale_vec_rowid".into(),
            details: serde_json::json!({
                "page_embedding_id": id,
                "vec_rowid": vec_rowid,
                "vec_table": vec_table.as_str(),
            }),
        });
    }

    Ok(())
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
        db::open(":memory:").unwrap()
    }

    fn insert_page(conn: &Connection, slug: &str) {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, 'concept', ?1, '', '', '', '{}', '', '', 1)",
            rusqlite::params![slug],
        )
        .unwrap();
    }

    #[test]
    fn clean_brain_passes_all_checks() {
        let conn = open_test_db();
        insert_page(&conn, "test/one");
        let report = execute_validate(&conn, &CheckFlags::all()).unwrap();
        assert!(report.passed);
        assert!(report.violations.is_empty());
        assert_eq!(report.checks.len(), 3);
    }

    #[test]
    fn detects_dangling_assertion_page() {
        let conn = open_test_db();
        insert_page(&conn, "test/a");
        let page_id: i64 = conn
            .query_row("SELECT id FROM pages WHERE slug='test/a'", [], |r| r.get(0))
            .unwrap();
        // Insert assertion referencing valid page, then delete the page
        conn.execute(
            "INSERT INTO assertions (page_id, subject, predicate, object, asserted_by) \
             VALUES (?1, 'X', 'is', 'Y', 'manual')",
            [page_id],
        )
        .unwrap();
        // Disable FK enforcement temporarily to create dangling ref
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        conn.execute("DELETE FROM pages WHERE slug = 'test/a'", [])
            .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

        let report = execute_validate(
            &conn,
            &CheckFlags {
                links: false,
                assertions: true,
                embeddings: false,
            },
        )
        .unwrap();
        assert!(!report.passed);
        assert!(report
            .violations
            .iter()
            .any(|v| v.violation_type == "dangling_page"));
    }

    #[test]
    fn detects_dangling_supersedes() {
        let conn = open_test_db();
        insert_page(&conn, "test/a");
        let page_id: i64 = conn
            .query_row("SELECT id FROM pages WHERE slug='test/a'", [], |r| r.get(0))
            .unwrap();
        // Disable FK to insert assertion with dangling supersedes_id
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        conn.execute(
            "INSERT INTO assertions (page_id, subject, predicate, object, supersedes_id, asserted_by) \
             VALUES (?1, 'X', 'is', 'Y', 9999, 'manual')",
            [page_id],
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

        let report = execute_validate(
            &conn,
            &CheckFlags {
                links: false,
                assertions: true,
                embeddings: false,
            },
        )
        .unwrap();
        assert!(!report.passed);
        assert!(report
            .violations
            .iter()
            .any(|v| v.violation_type == "dangling_supersedes"));
    }

    #[test]
    fn detects_wrong_active_model_count() {
        let conn = open_test_db();
        // Deactivate the default model
        conn.execute("UPDATE embedding_models SET active = 0", [])
            .unwrap();
        let report = execute_validate(
            &conn,
            &CheckFlags {
                links: false,
                assertions: false,
                embeddings: true,
            },
        )
        .unwrap();
        assert!(!report.passed);
        assert!(report
            .violations
            .iter()
            .any(|v| v.violation_type == "active_model_count"));
    }

    #[test]
    fn detects_stale_vec_rowid() {
        let conn = open_test_db();
        insert_page(&conn, "test/vec");
        let page_id: i64 = conn
            .query_row("SELECT id FROM pages WHERE slug='test/vec'", [], |r| {
                r.get(0)
            })
            .unwrap();
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, \
             chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', 42, 'truth_section', 0, 'hello', 'hash', 1, '')",
            [page_id],
        )
        .unwrap();

        let report = execute_validate(
            &conn,
            &CheckFlags {
                links: false,
                assertions: false,
                embeddings: true,
            },
        )
        .unwrap();
        assert!(!report.passed);
        assert!(report
            .violations
            .iter()
            .any(|v| v.violation_type == "stale_vec_rowid"));
    }

    #[test]
    fn json_output_is_valid() {
        let conn = open_test_db();
        insert_page(&conn, "test/json");
        let report = execute_validate(&conn, &CheckFlags::all()).unwrap();
        let json = serde_json::to_string_pretty(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["passed"], true);
        assert!(parsed["checks"].is_array());
    }

    #[test]
    fn selective_checks_only_run_requested() {
        let conn = open_test_db();
        let report = execute_validate(
            &conn,
            &CheckFlags {
                links: true,
                assertions: false,
                embeddings: false,
            },
        )
        .unwrap();
        assert_eq!(report.checks, vec!["links"]);
    }
}
