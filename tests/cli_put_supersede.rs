#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for supersede-chain semantics of
//! `quaid::commands::put::put_from_string` — atomic linkage of both
//! ends of a successor pointer, rejection of attempts to supersede a
//! non-head page, and multi-step linear chains where each step points
//! at the latest predecessor.

#[path = "common/put_fixtures.rs"]
mod fixtures;

use fixtures::{open_test_db, page_id_for_slug, superseded_by_for_slug};
use quaid::commands::put::put_from_string;

#[test]
fn create_successor_updates_both_ends_of_supersede_chain_atomically() {
    let conn = open_test_db();
    put_from_string(
        &conn,
        "facts/a",
        "---\ntitle: A\ntype: fact\n---\nOriginal fact\n",
        None,
    )
    .unwrap();

    put_from_string(
        &conn,
        "facts/b",
        "---\ntitle: B\ntype: fact\nsupersedes: facts/a\n---\nUpdated fact\n",
        None,
    )
    .unwrap();

    assert_eq!(
        superseded_by_for_slug(&conn, "facts/a"),
        Some(page_id_for_slug(&conn, "facts/b"))
    );
    assert_eq!(superseded_by_for_slug(&conn, "facts/b"), None);
    assert_eq!(
        conn.query_row::<String, _, _>(
            "SELECT compiled_truth FROM pages WHERE slug = 'facts/a'",
            [],
            |row| row.get(0)
        )
        .unwrap(),
        "Original fact"
    );
}

#[test]
fn superseding_non_head_page_is_rejected_without_partial_write() {
    let conn = open_test_db();
    put_from_string(
        &conn,
        "facts/a",
        "---\ntitle: A\ntype: fact\n---\nA\n",
        None,
    )
    .unwrap();
    put_from_string(
        &conn,
        "facts/b",
        "---\ntitle: B\ntype: fact\nsupersedes: facts/a\n---\nB\n",
        None,
    )
    .unwrap();

    let error = put_from_string(
        &conn,
        "facts/c",
        "---\ntitle: C\ntype: fact\nsupersedes: facts/a\n---\nC\n",
        None,
    )
    .unwrap_err();

    assert!(error.to_string().contains("SupersedeConflictError"));
    assert_eq!(
        conn.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM pages WHERE slug = 'facts/c'",
            [],
            |row| row.get(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        superseded_by_for_slug(&conn, "facts/a"),
        Some(page_id_for_slug(&conn, "facts/b"))
    );
}

#[test]
fn multi_step_supersede_chain_stays_linked() {
    let conn = open_test_db();
    put_from_string(
        &conn,
        "facts/a",
        "---\ntitle: A\ntype: fact\n---\nA\n",
        None,
    )
    .unwrap();
    put_from_string(
        &conn,
        "facts/b",
        "---\ntitle: B\ntype: fact\nsupersedes: facts/a\n---\nB\n",
        None,
    )
    .unwrap();
    put_from_string(
        &conn,
        "facts/c",
        "---\ntitle: C\ntype: fact\nsupersedes: facts/b\n---\nC\n",
        None,
    )
    .unwrap();

    assert_eq!(
        superseded_by_for_slug(&conn, "facts/a"),
        Some(page_id_for_slug(&conn, "facts/b"))
    );
    assert_eq!(
        superseded_by_for_slug(&conn, "facts/b"),
        Some(page_id_for_slug(&conn, "facts/c"))
    );
    assert_eq!(superseded_by_for_slug(&conn, "facts/c"), None);
}
