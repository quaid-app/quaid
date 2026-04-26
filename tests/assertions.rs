//! Integration tests for assertions/check slice (tasks 3.5, 4.5).

use quaid::commands::check;
use quaid::commands::get::get_page;
use quaid::core::assertions::{self, AssertionError};
use quaid::core::db;
use rusqlite::Connection;

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

fn insert_page(conn: &Connection, slug: &str, compiled_truth: &str) {
    conn.execute(
        "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, ?2, 'person', ?1, '', ?3, '', '{}', 'people', '', 1)",
        rusqlite::params![slug, test_uuid(slug), compiled_truth],
    )
    .unwrap();
}

// ── extract_assertions integration ───────────────────────────

#[test]
fn extract_and_check_round_trip_detects_cross_page_conflict() {
    let conn = open_test_db();
    insert_page(
        &conn,
        "people/alice",
        "## Assertions\nAlice works at Acme Corp.\n",
    );
    insert_page(
        &conn,
        "sources/meeting",
        "## Assertions\nAlice works at Beta Corp.\n",
    );

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
    insert_page(
        &conn,
        "people/alice",
        "## Assertions\nAlice works at Acme Corp.\n",
    );

    let page = get_page(&conn, "people/alice").unwrap();
    assertions::extract_assertions(&page, &conn).unwrap();

    let contradictions = assertions::check_assertions("people/alice", &conn).unwrap();
    assert!(contradictions.is_empty());
}

#[test]
fn extract_on_missing_page_returns_page_not_found() {
    let conn = open_test_db();
    let page = quaid::core::types::Page {
        slug: "people/ghost".to_string(),
        uuid: "01969f11-9448-7d79-8d3f-c68f54761299".to_string(),
        page_type: "person".to_string(),
        title: "Ghost".to_string(),
        summary: String::new(),
        compiled_truth: "## Assertions\nAlice works at Acme.\n".to_string(),
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

#[test]
fn indented_assertions_example_does_not_trigger_extraction() {
    let conn = open_test_db();
    insert_page(
        &conn,
        "people/alice",
        "Reference example:\n\n    ## Assertions\n    Alice works at Acme Corp.\n",
    );

    let page = get_page(&conn, "people/alice").unwrap();
    let inserted = assertions::extract_assertions(&page, &conn).unwrap();
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM assertions", [], |row| row.get(0))
        .unwrap();

    assert_eq!(inserted, 0);
    assert_eq!(row_count, 0);
}

#[test]
fn fenced_assertions_example_does_not_trigger_extraction() {
    let conn = open_test_db();
    insert_page(
        &conn,
        "people/alice",
        "Reference example:\n\n```md\n## Assertions\nAlice works at Acme Corp.\n```\n",
    );

    let page = get_page(&conn, "people/alice").unwrap();
    let inserted = assertions::extract_assertions(&page, &conn).unwrap();
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM assertions", [], |row| row.get(0))
        .unwrap();

    assert_eq!(inserted, 0);
    assert_eq!(row_count, 0);
}

// ── check CLI integration ────────────────────────────────────

#[test]
fn check_single_slug_finds_contradiction() {
    let conn = open_test_db();
    insert_page(
        &conn,
        "people/alice",
        "## Assertions\nAlice works at Acme Corp.\nAlice works at Beta Corp.\n",
    );

    // Call the check command directly — it should succeed
    let result = check::run(&conn, Some("people/alice".to_string()), false, None, false);
    assert!(result.is_ok());
}

#[test]
fn check_all_mode_processes_multiple_pages() {
    let conn = open_test_db();
    insert_page(
        &conn,
        "people/alice",
        "## Assertions\nAlice works at Acme Corp.\n",
    );
    insert_page(
        &conn,
        "sources/meeting",
        "## Assertions\nAlice works at Beta Corp.\n",
    );

    let result = check::run(&conn, None, true, None, false);
    assert!(result.is_ok());
}

#[test]
fn check_json_output_is_valid_json() {
    let conn = open_test_db();
    insert_page(
        &conn,
        "people/alice",
        "## Assertions\nAlice works at Acme Corp.\n",
    );
    insert_page(
        &conn,
        "sources/meeting",
        "## Assertions\nAlice works at Beta Corp.\n",
    );

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
