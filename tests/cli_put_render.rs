#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for round-trip rendering, frontmatter
//! persistence, and FTS5 indexing of pages written via
//! `quaid::commands::put::put_from_string` — covers the read-back
//! path through `commands::get::get_page` + `markdown::render_page`,
//! the JSON storage of frontmatter, the `quaid_id` retention rule
//! across the render seam, and the trigger that mirrors page
//! contents into the `page_fts` virtual table.

#[path = "common/put_fixtures.rs"]
mod fixtures;

use fixtures::open_test_db;
use quaid::commands::put::put_from_string;
use quaid::core::markdown;

// ── round-trip fidelity ───────────────────────────────────

#[test]
fn put_then_get_roundtrips_through_render() {
    let conn = open_test_db();
    let md = "---\ntitle: Carol\ntype: person\n---\n# Carol\n\nCarol builds things.\n---\n2024-06-01: Shipped v1.\n";

    put_from_string(&conn, "people/carol", md, None).unwrap();

    // Read back through get path
    let page = quaid::commands::get::get_page(&conn, "people/carol").unwrap();
    let rendered = markdown::render_page(&page);
    assert!(rendered.contains("quaid_id: "));
    assert!(rendered.contains("title: Carol"));
    assert!(rendered.contains("type: person"));
    assert!(rendered.contains("# Carol\n\nCarol builds things."));
    assert!(rendered.contains("2024-06-01: Shipped v1."));
}

#[test]
fn put_render_cannot_strip_existing_quaid_id_when_update_omits_it() {
    let conn = open_test_db();
    let original = "---\nquaid_id: 01969f11-9448-7d79-8d3f-c68f54761234\ntitle: Carol\ntype: person\n---\n# Carol\n\nOriginal.\n";
    put_from_string(&conn, "people/carol", original, None).unwrap();

    let updated = "---\ntitle: Carol\ntype: person\n---\n# Carol\n\nUpdated.\n";
    put_from_string(&conn, "people/carol", updated, Some(1)).unwrap();

    let page = quaid::commands::get::get_page(&conn, "people/carol").unwrap();
    let rendered = markdown::render_page(&page);

    assert!(
        rendered.contains("quaid_id: 01969f11-9448-7d79-8d3f-c68f54761234"),
        "memory_put must not let a UUID-bearing page render back out without quaid_id"
    );
}

// ── frontmatter stored as JSON ────────────────────────────

#[test]
fn frontmatter_is_stored_as_json_and_recoverable() {
    let conn = open_test_db();
    let md = "---\nsource: manual\ntitle: Data\ntype: concept\n---\nContent.\n";

    put_from_string(&conn, "data/test", md, None).unwrap();

    let fm_json: String = conn
        .query_row(
            "SELECT frontmatter FROM pages WHERE slug = ?1",
            ["data/test"],
            |row| row.get(0),
        )
        .unwrap();
    let fm: quaid::core::types::Frontmatter = serde_json::from_str(&fm_json).unwrap();
    assert_eq!(fm.get("source"), Some(&serde_json::json!("manual")));
    assert_eq!(fm.get("title"), Some(&serde_json::json!("Data")));
    assert_eq!(fm.get("type"), Some(&serde_json::json!("concept")));
}

// ── FTS5 trigger fires ────────────────────────────────────

#[test]
fn insert_triggers_fts5_indexing() {
    let conn = open_test_db();
    let md = "---\ntitle: Searchable\ntype: concept\n---\n# Searchable\n\nUnique searchable keyword xylophone.\n";

    put_from_string(&conn, "test/searchable", md, None).unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM page_fts WHERE page_fts MATCH 'xylophone'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
