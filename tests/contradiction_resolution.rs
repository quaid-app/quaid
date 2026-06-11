#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for the contradiction detection & resolution slice:
//! fuzzy type-key head matching in the fact resolver, extracted-fact pages
//! mirrored into the `assertions` table, and the `resolved_at` resolution
//! surface (`commands::check::execute_resolve`, the `quaid check --resolve`
//! CLI, and MCP `memory_check`).

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::path::Path;
use std::process::{Command, Output};

use quaid::commands::check;
use quaid::core::assertions;
use quaid::core::conversation::supersede::{resolve_in_scope_with_similarity, Resolution};
use quaid::core::db;
use quaid::core::types::RawFact;
use quaid::mcp::server::{MemoryCheckInput, MemoryStatsInput, QuaidServer};
use rmcp::model::RawContent;
use rusqlite::{params, Connection};

fn open_test_db(path: &Path) -> Connection {
    let conn = db::open(path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active'
         WHERE id = 1",
        [path.parent().unwrap().display().to_string()],
    )
    .unwrap();
    conn
}

fn insert_head_page(
    conn: &Connection,
    slug: &str,
    kind: &str,
    key_name: &str,
    key_value: &str,
    summary: &str,
) -> i64 {
    let frontmatter = serde_json::json!({
        "kind": kind,
        key_name: key_value,
    })
    .to_string();
    conn.execute(
        "INSERT INTO pages
             (collection_id, namespace, slug, uuid, type, title, summary, compiled_truth, timeline,
              frontmatter, wing, room, version)
         VALUES
             (1, '', ?1, ?2, ?3, ?1, ?4, ?4, '', ?5, '', '', 1)",
        params![slug, format!("uuid-{slug}"), kind, summary, frontmatter],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn fact_about(about: &str, summary: &str) -> RawFact {
    RawFact::Fact {
        about: about.to_string(),
        summary: summary.to_string(),
    }
}

fn extract_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| match &content.raw {
            RawContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect()
}

// ── fuzzy type-key head matching ─────────────────────────────

#[test]
fn paraphrased_key_supersedes_existing_head_via_fuzzy_match() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_head_page(
        &conn,
        "salary-fact",
        "fact",
        "about",
        "salary",
        "Matt earns 100k",
    );

    let resolution = resolve_in_scope_with_similarity(
        &fact_about("compensation", "Matt now earns 120k"),
        &conn,
        1,
        "",
        |left, right| {
            if left == "compensation" && right == "salary" {
                return Ok(0.9); // paraphrased key clears key_match_cosine_min
            }
            Ok(0.6) // body similarity lands in the supersede band
        },
    )
    .unwrap();

    assert!(matches!(
        resolution,
        Resolution::Supersede { prior_slug, .. } if prior_slug == "salary-fact"
    ));
}

#[test]
fn fuzzy_key_match_normalizes_case_and_whitespace() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_head_page(
        &conn,
        "salary-fact",
        "fact",
        "about",
        "  Salary  ",
        "Matt earns 100k",
    );

    let resolution = resolve_in_scope_with_similarity(
        &fact_about("salary band", "Matt now earns 120k"),
        &conn,
        1,
        "",
        |left, right| {
            // Keys arrive lowercased and trimmed.
            if left == "salary band" && right == "salary" {
                return Ok(0.92);
            }
            Ok(0.6)
        },
    )
    .unwrap();

    assert!(matches!(
        resolution,
        Resolution::Supersede { prior_slug, .. } if prior_slug == "salary-fact"
    ));
}

#[test]
fn fuzzy_key_margin_ambiguity_falls_back_to_coexist() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_head_page(
        &conn,
        "salary-acme",
        "fact",
        "about",
        "salary at acme",
        "100k at Acme",
    );
    insert_head_page(
        &conn,
        "salary-beta",
        "fact",
        "about",
        "salary at beta",
        "90k at Beta",
    );

    // Both candidates clear the threshold but sit within the 0.05 margin:
    // resolution must coexist rather than guess (and must NOT surface an
    // AmbiguousMatchingHeads error).
    let resolution = resolve_in_scope_with_similarity(
        &fact_about("compensation", "Matt earns 120k"),
        &conn,
        1,
        "",
        |_, right| match right {
            "salary at acme" => Ok(0.90),
            "salary at beta" => Ok(0.88),
            _ => panic!("body similarity should not run for ambiguous fuzzy key matches"),
        },
    )
    .unwrap();

    assert_eq!(resolution, Resolution::Coexist);
}

#[test]
fn fuzzy_key_below_threshold_falls_back_to_coexist() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_head_page(
        &conn,
        "editor-fact",
        "fact",
        "about",
        "editor",
        "Matt uses Helix",
    );

    let resolution = resolve_in_scope_with_similarity(
        &fact_about("compensation", "Matt earns 120k"),
        &conn,
        1,
        "",
        |_, right| match right {
            "editor" => Ok(0.4),
            _ => panic!("body similarity should not run below the fuzzy key threshold"),
        },
    )
    .unwrap();

    assert_eq!(resolution, Resolution::Coexist);
}

#[test]
fn key_match_cosine_min_is_seeded_and_configurable() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));

    let seeded: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'fact_resolution.key_match_cosine_min'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(seeded, "0.85");

    // Raise the threshold above the reported cosine: the fuzzy match must
    // no longer fire.
    conn.execute(
        "UPDATE config SET value = '0.95' WHERE key = 'fact_resolution.key_match_cosine_min'",
        [],
    )
    .unwrap();
    insert_head_page(
        &conn,
        "salary-fact",
        "fact",
        "about",
        "salary",
        "Matt earns 100k",
    );

    let resolution = resolve_in_scope_with_similarity(
        &fact_about("compensation", "Matt earns 120k"),
        &conn,
        1,
        "",
        |_, right| match right {
            "salary" => Ok(0.9),
            _ => panic!("body similarity should not run below the configured threshold"),
        },
    )
    .unwrap();

    assert_eq!(resolution, Resolution::Coexist);
}

// ── extracted facts mirrored into assertions ─────────────────

#[test]
fn extracted_fact_pages_yield_assertion_rows_and_check_detects_conflict() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    // Two coexisting extracted-fact heads about the same key; neither page
    // contains a literal "## Assertions" heading.
    insert_head_page(
        &conn,
        "extracted/facts/timezone-1",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+9",
    );
    insert_head_page(
        &conn,
        "extracted/facts/timezone-2",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+2",
    );

    check::execute_check(&conn, None, true, None).unwrap();
    assert_eq!(report_contradiction_count(&conn), 1);

    let rows: Vec<(String, String, String, String)> = conn
        .prepare(
            "SELECT subject, predicate, object, asserted_by
             FROM assertions
             ORDER BY object",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                "timezone".to_string(),
                "fact".to_string(),
                "Matt is based in UTC+2".to_string(),
                "extraction".to_string(),
            ),
            (
                "timezone".to_string(),
                "fact".to_string(),
                "Matt is based in UTC+9".to_string(),
                "extraction".to_string(),
            ),
        ]
    );
}

#[test]
fn superseded_fact_pages_are_not_mirrored_into_assertions() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    let head_id = insert_head_page(
        &conn,
        "extracted/facts/timezone-new",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+2",
    );
    insert_head_page(
        &conn,
        "extracted/facts/timezone-old",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+9",
    );
    conn.execute(
        "UPDATE pages SET superseded_by = ?1 WHERE slug = 'extracted/facts/timezone-old'",
        [head_id],
    )
    .unwrap();

    check::execute_check(&conn, None, true, None).unwrap();

    let assertion_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM assertions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        assertion_count, 1,
        "only the head fact page should be mirrored"
    );
    assert_eq!(report_contradiction_count(&conn), 0);
}

// ── resolution surface ───────────────────────────────────────

#[test]
fn resolve_stamps_resolved_at_and_removes_row_from_check_and_stats() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_head_page(
        &conn,
        "extracted/facts/timezone-1",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+9",
    );
    insert_head_page(
        &conn,
        "extracted/facts/timezone-2",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+2",
    );
    check::execute_check(&conn, None, true, None).unwrap();
    let contradiction_id: i64 = conn
        .query_row(
            "SELECT id FROM contradictions WHERE resolved_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let report = check::execute_resolve(&conn, contradiction_id, None).unwrap();
    assert_eq!(report.contradiction_id, contradiction_id);
    assert_eq!(report.kept_slug, None);

    let resolved_at: Option<String> = conn
        .query_row(
            "SELECT resolved_at FROM contradictions WHERE id = ?1",
            [contradiction_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(resolved_at.is_some(), "resolved_at must be stamped");

    let server = QuaidServer::new(conn);
    let check_result = server
        .memory_check(MemoryCheckInput {
            slug: None,
            resolve: None,
            keep: None,
        })
        .unwrap();
    let listed: serde_json::Value = serde_json::from_str(&extract_text(&check_result)).unwrap();
    assert_eq!(
        listed.as_array().unwrap().len(),
        0,
        "resolved contradiction must not be re-detected or listed by memory_check"
    );

    let stats_result = server.memory_stats(MemoryStatsInput {}).unwrap();
    let stats: serde_json::Value = serde_json::from_str(&extract_text(&stats_result)).unwrap();
    assert_eq!(
        stats["contradiction_count"], 0,
        "stats must count only unresolved contradictions"
    );
}

#[test]
fn resolve_with_keep_supersedes_the_other_page() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    let keeper_id = insert_head_page(
        &conn,
        "extracted/facts/timezone-keep",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+2",
    );
    let loser_id = insert_head_page(
        &conn,
        "extracted/facts/timezone-drop",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+9",
    );
    check::execute_check(&conn, None, true, None).unwrap();
    let contradiction_id: i64 = conn
        .query_row(
            "SELECT id FROM contradictions WHERE resolved_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let report = check::execute_resolve(
        &conn,
        contradiction_id,
        Some("extracted/facts/timezone-keep"),
    )
    .unwrap();
    assert_eq!(
        report.kept_slug.as_deref(),
        Some("default::extracted/facts/timezone-keep")
    );
    assert_eq!(
        report.superseded_slug.as_deref(),
        Some("default::extracted/facts/timezone-drop")
    );

    let superseded_by: Option<i64> = conn
        .query_row(
            "SELECT superseded_by FROM pages WHERE id = ?1",
            [loser_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(superseded_by, Some(keeper_id));

    // The superseded page's mirrored assertion is gone, so a fresh check
    // run does not re-detect the conflict.
    check::execute_check(&conn, None, true, None).unwrap();
    let open_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM contradictions WHERE resolved_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(open_count, 0);
}

#[test]
fn resolve_unknown_or_mismatched_inputs_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));

    let missing = check::execute_resolve(&conn, 4242, None).unwrap_err();
    assert!(missing.to_string().contains("contradiction not found"));

    insert_head_page(
        &conn,
        "extracted/facts/timezone-1",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+9",
    );
    insert_head_page(
        &conn,
        "extracted/facts/timezone-2",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+2",
    );
    insert_head_page(
        &conn,
        "extracted/facts/bystander",
        "fact",
        "about",
        "editor",
        "Matt uses Helix today",
    );
    check::execute_check(&conn, None, true, None).unwrap();
    let contradiction_id: i64 = conn
        .query_row(
            "SELECT id FROM contradictions WHERE resolved_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let mismatched =
        check::execute_resolve(&conn, contradiction_id, Some("extracted/facts/bystander"))
            .unwrap_err();
    assert!(mismatched.to_string().contains("is not part of"));

    check::execute_resolve(&conn, contradiction_id, None).unwrap();
    let already = check::execute_resolve(&conn, contradiction_id, None).unwrap_err();
    assert!(already.to_string().contains("already resolved"));
}

#[test]
fn memory_check_resolve_param_resolves_contradiction() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_head_page(
        &conn,
        "extracted/facts/timezone-keep",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+2",
    );
    insert_head_page(
        &conn,
        "extracted/facts/timezone-drop",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+9",
    );
    check::execute_check(&conn, None, true, None).unwrap();
    let contradiction_id: i64 = conn
        .query_row(
            "SELECT id FROM contradictions WHERE resolved_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let server = QuaidServer::new(conn);
    let resolve_result = server
        .memory_check(MemoryCheckInput {
            slug: None,
            resolve: Some(contradiction_id),
            keep: Some("extracted/facts/timezone-keep".to_string()),
        })
        .unwrap();
    let report: serde_json::Value = serde_json::from_str(&extract_text(&resolve_result)).unwrap();
    assert_eq!(report["contradiction_id"], contradiction_id);
    assert_eq!(
        report["kept_slug"],
        "default::extracted/facts/timezone-keep"
    );
    assert_eq!(
        report["superseded_slug"],
        "default::extracted/facts/timezone-drop"
    );

    let check_result = server
        .memory_check(MemoryCheckInput {
            slug: None,
            resolve: None,
            keep: None,
        })
        .unwrap();
    let listed: serde_json::Value = serde_json::from_str(&extract_text(&check_result)).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 0);
}

#[test]
fn superseding_a_page_auto_resolves_open_contradictions_touching_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    let keeper_id = insert_head_page(
        &conn,
        "extracted/facts/timezone-new",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+2",
    );
    let loser_id = insert_head_page(
        &conn,
        "extracted/facts/timezone-old",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+9",
    );
    check::execute_check(&conn, None, true, None).unwrap();
    assert_eq!(report_contradiction_count(&conn), 1);

    quaid::core::supersede::mark_page_superseded(&conn, keeper_id, loser_id).unwrap();

    assert_eq!(
        report_contradiction_count(&conn),
        0,
        "supersede must auto-resolve open contradictions touching the superseded page"
    );
}

#[test]
fn resolve_contradiction_core_api_is_idempotent_and_404s_on_unknown_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_head_page(
        &conn,
        "extracted/facts/timezone-1",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+9",
    );
    insert_head_page(
        &conn,
        "extracted/facts/timezone-2",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+2",
    );
    check::execute_check(&conn, None, true, None).unwrap();
    let contradiction_id: i64 = conn
        .query_row("SELECT id FROM contradictions", [], |row| row.get(0))
        .unwrap();

    assert!(assertions::resolve_contradiction(contradiction_id, &conn).unwrap());
    assert!(!assertions::resolve_contradiction(contradiction_id, &conn).unwrap());
    assert!(matches!(
        assertions::resolve_contradiction(4242, &conn).unwrap_err(),
        assertions::AssertionError::ContradictionNotFound { id: 4242 }
    ));
}

fn report_contradiction_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM contradictions WHERE resolved_at IS NULL",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

fn run_quaid(db_path: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .arg("--db")
        .arg(db_path)
        .args(args)
        .output()
        .expect("run quaid")
}

#[test]
fn cli_check_resolve_keep_flow_round_trips() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    insert_head_page(
        &conn,
        "extracted/facts/timezone-keep",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+2",
    );
    insert_head_page(
        &conn,
        "extracted/facts/timezone-drop",
        "fact",
        "about",
        "timezone",
        "Matt is based in UTC+9",
    );
    drop(conn);

    let check = run_quaid(&db_path, &["check", "--all", "--json"]);
    assert!(
        check.status.success(),
        "check stderr: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let listed: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    let contradiction = &listed.as_array().unwrap()[0];
    let contradiction_id = contradiction["id"].as_i64().unwrap();
    assert_eq!(contradiction["type"], "assertion_conflict");

    let resolve = run_quaid(
        &db_path,
        &[
            "check",
            "--resolve",
            &contradiction_id.to_string(),
            "--keep",
            "extracted/facts/timezone-keep",
            "--json",
        ],
    );
    assert!(
        resolve.status.success(),
        "resolve stderr: {}",
        String::from_utf8_lossy(&resolve.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&resolve.stdout).unwrap();
    assert_eq!(report["contradiction_id"], contradiction_id);
    assert_eq!(
        report["superseded_slug"],
        "default::extracted/facts/timezone-drop"
    );

    let recheck = run_quaid(&db_path, &["check", "--all", "--json"]);
    assert!(recheck.status.success());
    let relisted: serde_json::Value = serde_json::from_slice(&recheck.stdout).unwrap();
    assert_eq!(relisted.as_array().unwrap().len(), 0);
}

#[test]
fn cli_check_keep_without_resolve_is_rejected() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    drop(open_test_db(&db_path));

    let output = run_quaid(&db_path, &["check", "--keep", "people/alice"]);
    assert!(
        !output.status.success(),
        "--keep without --resolve must be rejected by clap"
    );
}
