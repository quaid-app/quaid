#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for optimistic-concurrency update semantics of
//! `quaid::commands::put::put_from_string` — version bumps under the
//! correct `expected_version`, conflict reporting and body invariance
//! under stale `expected_version`, the unconditional upsert path on
//! non-Unix builds, the `quaid_id` retention rule on updates, and the
//! single-active-`raw_imports` invariant after rotation.

#[path = "common/put_fixtures.rs"]
mod fixtures;

use fixtures::{
    active_raw_import_bytes_for_slug, active_raw_import_count_for_slug, open_test_db, read_page,
};
use quaid::commands::put::put_from_string;

// ── update with OCC ───────────────────────────────────────

#[test]
fn update_with_correct_expected_version_bumps_version() {
    let conn = open_test_db();
    let md1 = "---\ntitle: Alice\ntype: person\n---\nOriginal.\n";
    put_from_string(&conn, "people/alice", md1, None).unwrap();

    let md2 = "---\ntitle: Alice\ntype: person\n---\nUpdated.\n";
    put_from_string(&conn, "people/alice", md2, Some(1)).unwrap();

    let (version, _, _, truth, _) = read_page(&conn, "people/alice").unwrap();
    assert_eq!(version, 2);
    assert!(truth.contains("Updated"));
}

#[test]
fn update_without_quaid_id_frontmatter_keeps_existing_page_uuid() {
    let conn = open_test_db();
    let original = "---\nquaid_id: 01969f11-9448-7d79-8d3f-c68f54761234\ntitle: Alice\ntype: person\n---\nOriginal.\n";
    put_from_string(&conn, "people/alice", original, None).unwrap();

    let updated = "---\ntitle: Alice\ntype: person\n---\nUpdated.\n";
    put_from_string(&conn, "people/alice", updated, Some(1)).unwrap();

    let stored_uuid: String = conn
        .query_row(
            "SELECT uuid FROM pages WHERE slug = ?1",
            ["people/alice"],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(stored_uuid, "01969f11-9448-7d79-8d3f-c68f54761234");
}

#[test]
fn update_with_stale_expected_version_returns_conflict_error() {
    let conn = open_test_db();
    let md = "---\ntitle: Alice\ntype: person\n---\nContent.\n";
    put_from_string(&conn, "people/alice", md, None).unwrap();

    // Simulate a concurrent update by bumping version directly.
    conn.execute(
        "UPDATE pages SET version = 2, updated_at = '2099-01-01T00:00:00Z' WHERE slug = 'people/alice'",
        [],
    )
    .unwrap();

    let md2 = "---\ntitle: Alice\ntype: person\n---\nStale update.\n";
    let result = put_from_string(&conn, "people/alice", md2, Some(1));

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Conflict"));
    assert!(err.contains("current version: 2"));
}

#[test]
fn update_with_stale_expected_version_leaves_existing_page_body_unchanged() {
    let conn = open_test_db();
    let original = "---\ntitle: Alice\ntype: person\n---\nOriginal body.\n";
    put_from_string(&conn, "people/alice", original, None).unwrap();

    conn.execute(
        "UPDATE pages SET version = 2, compiled_truth = 'Concurrent body' WHERE slug = 'people/alice'",
        [],
    )
    .unwrap();

    let stale = "---\ntitle: Alice\ntype: person\n---\nStale body.\n";
    let result = put_from_string(&conn, "people/alice", stale, Some(1));
    assert!(result.is_err());

    let (version, _, _, truth, _) = read_page(&conn, "people/alice").unwrap();
    assert_eq!(version, 2);
    assert_eq!(truth, "Concurrent body");
}

#[test]
fn put_occ_update_keeps_exactly_one_active_raw_import_row_for_latest_bytes() {
    let conn = open_test_db();
    let original = "---\ntitle: Alice\ntype: person\n---\nOriginal body.\n";
    put_from_string(&conn, "people/alice", original, None).unwrap();

    let updated = "---\ntitle: Alice\ntype: person\n---\nUpdated body.\n";
    put_from_string(&conn, "people/alice", updated, Some(1)).unwrap();

    assert_eq!(active_raw_import_count_for_slug(&conn, "people/alice"), 1);
    assert_eq!(
        active_raw_import_bytes_for_slug(&conn, "people/alice"),
        updated.as_bytes()
    );
}

// ── unconditional upsert ──────────────────────────────────

#[cfg(not(unix))]
#[test]
fn update_without_expected_version_upserts_unconditionally() {
    let conn = open_test_db();
    let md1 = "---\ntitle: Bob\ntype: person\n---\nOriginal.\n";
    put_from_string(&conn, "people/bob", md1, None).unwrap();

    let md2 = "---\ntitle: Bob\ntype: person\n---\nOverwritten.\n";
    put_from_string(&conn, "people/bob", md2, None).unwrap();

    let (version, _, _, truth, _) = read_page(&conn, "people/bob").unwrap();
    assert_eq!(version, 2);
    assert!(truth.contains("Overwritten"));
}
