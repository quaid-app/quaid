#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for first-write (create) semantics of
//! `quaid::commands::put::put_from_string` — initial version, wing
//! derivation from slug, and default page type when the frontmatter
//! omits `type`.

#[path = "common/put_fixtures.rs"]
mod fixtures;

use fixtures::{open_test_db, read_page};
use quaid::commands::put::put_from_string;

// ── create ─────────────────────────────────────────────────

#[test]
fn create_page_sets_version_to_1() {
    let conn = open_test_db();
    let md = "---\ntitle: Alice\ntype: person\n---\n# Alice\n\nAlice is an operator.\n---\n2024-01-01: Joined Acme.\n";

    put_from_string(&conn, "people/alice", md, None).unwrap();

    let (version, page_type, title, truth, timeline) = read_page(&conn, "people/alice").unwrap();
    assert_eq!(version, 1);
    assert_eq!(page_type, "person");
    assert_eq!(title, "Alice");
    assert!(truth.contains("Alice is an operator"));
    assert!(timeline.contains("Joined Acme"));
}

#[test]
fn create_page_derives_wing_from_slug() {
    let conn = open_test_db();
    let md = "---\ntitle: Alice\ntype: person\n---\nContent.\n";

    put_from_string(&conn, "people/alice-jones", md, None).unwrap();

    let wing: String = conn
        .query_row(
            "SELECT wing FROM pages WHERE slug = ?1",
            ["people/alice-jones"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(wing, "people");
}

#[test]
fn create_page_defaults_type_to_concept_when_missing() {
    let conn = open_test_db();
    let md = "---\ntitle: Readme\n---\nJust a concept.\n";

    put_from_string(&conn, "readme", md, None).unwrap();

    let (_, page_type, _, _, _) = read_page(&conn, "readme").unwrap();
    assert_eq!(page_type, "concept");
}
