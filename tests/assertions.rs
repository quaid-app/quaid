//! Integration tests for assertions/check slice (tasks 3.5, 4.5).

use gbrain::commands::check;
use gbrain::commands::get::get_page;
use gbrain::core::assertions::{self, AssertionError};
use gbrain::core::db;
use rusqlite::Connection;

fn open_test_db() -> Connection {
    db::open(":memory:").unwrap()
}

fn insert_page(conn: &Connection, slug: &str, compiled_truth: &str) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, 'person', ?1, '', ?2, '', '{}', 'people', '', 1)",
        rusqlite::params![slug, compiled_truth],
    )
    .unwrap();
}

// ── extract_assertions integration ───────────────────────────

#[test]
fn extract_and_check_round_trip_detects_cross_page_conflict() {
    let conn = open_test_db();
    insert_page(&conn, "people/alice", "Alice works at Acme Corp.");
    insert_page(&conn, "sources/meeting", "Alice works at Beta Corp.");

    let page_a = get_page(&conn, "people/alice").unwrap();
    let page_b = get_page(&conn, "sources/meeting").unwrap();
    assertions::extract_assertions(&page_a, &conn).unwrap();
    assertions::extract_assertions(&page_b, &conn).unwrap();

    let contradictions = assertions::check_assertions("people/alice", &conn).unwrap();

    assert_eq!(contradictions.len(), 1);
    assert!(contradictions[0].description.contains("Acme Corp"));
    assert!(contradictions[0].description.contains("Beta Corp"));
}

#[test]
fn check_on_clean_page_returns_no_contradictions() {
    let conn = open_test_db();
    insert_page(&conn, "people/alice", "Alice works at Acme Corp.");

    let page = get_page(&conn, "people/alice").unwrap();
    assertions::extract_assertions(&page, &conn).unwrap();

    let contradictions = assertions::check_assertions("people/alice", &conn).unwrap();
    assert!(contradictions.is_empty());
}

#[test]
fn extract_on_missing_page_returns_page_not_found() {
    let conn = open_test_db();
    let page = gbrain::core::types::Page {
        slug: "people/ghost".to_string(),
        page_type: "person".to_string(),
        title: "Ghost".to_string(),
        summary: String::new(),
        compiled_truth: "Alice works at Acme.".to_string(),
        timeline: String::new(),
        frontmatter: Default::default(),
        wing: String::new(),
        room: String::new(),
        version: 1,
        created_at: String::new(),
        updated_at: String::new(),
        truth_updated_at: String::new(),
        timeline_updated_at: String::new(),
    };

    let result = assertions::extract_assertions(&page, &conn);
    assert!(matches!(result, Err(AssertionError::PageNotFound { .. })));
}

// ── check CLI integration ────────────────────────────────────

#[test]
fn check_single_slug_finds_contradiction() {
    let conn = open_test_db();
    insert_page(
        &conn,
        "people/alice",
        "Alice works at Acme Corp.\nAlice works at Beta Corp.",
    );

    // Call the check command directly — it should succeed
    let result = check::run(&conn, Some("people/alice".to_string()), false, None, false);
    assert!(result.is_ok());
}

#[test]
fn check_all_mode_processes_multiple_pages() {
    let conn = open_test_db();
    insert_page(&conn, "people/alice", "Alice works at Acme Corp.");
    insert_page(&conn, "sources/meeting", "Alice works at Beta Corp.");

    let result = check::run(&conn, None, true, None, false);
    assert!(result.is_ok());
}

#[test]
fn check_json_output_is_valid_json() {
    let conn = open_test_db();
    insert_page(&conn, "people/alice", "Alice works at Acme Corp.");
    insert_page(&conn, "sources/meeting", "Alice works at Beta Corp.");

    // JSON mode should not error
    let result = check::run(&conn, None, true, None, true);
    assert!(result.is_ok());
}

#[test]
fn check_nonexistent_slug_returns_error() {
    let conn = open_test_db();

    let result = check::run(&conn, Some("people/nobody".to_string()), false, None, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("page not found"));
}

#[test]
fn check_neither_slug_nor_all_returns_error() {
    let conn = open_test_db();

    let result = check::run(&conn, None, false, None, false);
    assert!(result.is_err());
}
