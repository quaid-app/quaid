#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Namespace matrix tests for `core::pages::resolve` — the single sanctioned
//! `(collection, namespace, slug)` → page-id lookup. One slug is written to
//! the global namespace and to two named namespaces; every filter shape must
//! resolve deterministically. These tests lock the semantics BEFORE call
//! sites migrate onto `resolve`, so any drift in the fallback ordering is a
//! test failure rather than a silent behaviour change.

use std::path::Path;

use quaid::commands::put;
use quaid::core::db;
use quaid::core::pages::{self, PageKey};
use rusqlite::Connection;

const SLUG: &str = "notes/shared-slug";

fn open_seeded_db() -> Connection {
    let conn = db::open(":memory:").expect("open db");
    for (namespace, marker) in [
        (Some("ns-b"), "bravo"),
        (Some("ns-a"), "alpha"),
        (None, "global"),
    ] {
        let content =
            format!("---\ntitle: Shared {marker}\ntype: concept\n---\n{marker} body content\n");
        put::put_from_string_with_namespace(&conn, SLUG, &content, namespace, None)
            .expect("seed page");
    }
    conn
}

fn page_id_for_namespace(conn: &Connection, namespace: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM pages WHERE namespace = ?1 AND slug = ?2",
        rusqlite::params![namespace, SLUG],
        |row| row.get(0),
    )
    .expect("seeded page id")
}

#[test]
fn resolve_exact_global_namespace_matches_global_row_only() {
    let conn = open_seeded_db();
    let global_id = page_id_for_namespace(&conn, "");

    let resolved = pages::resolve(
        &conn,
        &PageKey {
            collection_id: 1,
            namespace: Some(""),
            slug: SLUG,
        },
    )
    .expect("resolve global");
    assert_eq!(resolved, global_id);
}

#[test]
fn resolve_named_namespace_prefers_namespace_row_over_global() {
    let conn = open_seeded_db();
    let ns_a_id = page_id_for_namespace(&conn, "ns-a");
    let ns_b_id = page_id_for_namespace(&conn, "ns-b");

    for (namespace, expected) in [("ns-a", ns_a_id), ("ns-b", ns_b_id)] {
        let resolved = pages::resolve(
            &conn,
            &PageKey {
                collection_id: 1,
                namespace: Some(namespace),
                slug: SLUG,
            },
        )
        .expect("resolve namespaced");
        assert_eq!(resolved, expected, "namespace {namespace} must win");
    }
}

#[test]
fn resolve_named_namespace_falls_back_to_global_but_never_other_namespaces() {
    let conn = open_seeded_db();
    let global_id = page_id_for_namespace(&conn, "");

    // ns-c has no row of its own: documented fallback goes to global.
    let resolved = pages::resolve(
        &conn,
        &PageKey {
            collection_id: 1,
            namespace: Some("ns-c"),
            slug: SLUG,
        },
    )
    .expect("resolve with global fallback");
    assert_eq!(resolved, global_id);

    // Remove the global row: ns-c must NOT see ns-a/ns-b rows.
    conn.execute(
        "DELETE FROM pages WHERE namespace = '' AND slug = ?1",
        [SLUG],
    )
    .expect("delete global row");
    let resolved = pages::resolve_optional(
        &conn,
        &PageKey {
            collection_id: 1,
            namespace: Some("ns-c"),
            slug: SLUG,
        },
    )
    .expect("resolve_optional");
    assert_eq!(resolved, None, "other namespaces must never leak");
}

#[test]
fn resolve_unfiltered_prefers_global_then_lexicographic_namespace() {
    let conn = open_seeded_db();
    let global_id = page_id_for_namespace(&conn, "");
    let ns_a_id = page_id_for_namespace(&conn, "ns-a");

    let resolved = pages::resolve(
        &conn,
        &PageKey {
            collection_id: 1,
            namespace: None,
            slug: SLUG,
        },
    )
    .expect("resolve unfiltered");
    assert_eq!(resolved, global_id, "global row wins when present");

    conn.execute(
        "DELETE FROM pages WHERE namespace = '' AND slug = ?1",
        [SLUG],
    )
    .expect("delete global row");
    let resolved = pages::resolve(
        &conn,
        &PageKey {
            collection_id: 1,
            namespace: None,
            slug: SLUG,
        },
    )
    .expect("resolve unfiltered without global");
    assert_eq!(
        resolved, ns_a_id,
        "lexicographically smallest namespace breaks the tie deterministically"
    );
}

#[test]
fn resolve_missing_slug_returns_no_rows_and_resolve_optional_returns_none() {
    let conn = open_seeded_db();
    let missing = PageKey {
        collection_id: 1,
        namespace: None,
        slug: "notes/does-not-exist",
    };
    assert!(matches!(
        pages::resolve(&conn, &missing),
        Err(rusqlite::Error::QueryReturnedNoRows)
    ));
    assert_eq!(pages::resolve_optional(&conn, &missing).unwrap(), None);
}

#[test]
fn resolve_is_collection_scoped() {
    let conn = open_seeded_db();
    let missing = PageKey {
        collection_id: 999,
        namespace: None,
        slug: SLUG,
    };
    assert_eq!(pages::resolve_optional(&conn, &missing).unwrap(), None);
}

#[test]
fn derive_namespace_from_relative_path_matches_extraction_tree_layout() {
    for (path, expected) in [
        ("extracted/decisions/foo-ab12.md", ""),
        ("ns-a/extracted/decisions/foo-ab12.md", "ns-a"),
        (
            "session.2026/extracted/preferences/editor-1f00.md",
            "session.2026",
        ),
        ("people/alice.md", ""),
        ("projects/extracted-notes/alpha.md", ""),
        // Invalid namespace ids are treated as plain directories.
        ("bad ns/extracted/decisions/foo.md", ""),
    ] {
        assert_eq!(
            pages::derive_namespace_from_relative_path(Path::new(path)),
            expected,
            "path {path}"
        );
    }
}
