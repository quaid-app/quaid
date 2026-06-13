//! Quarantine-leak regressions: quarantined pages are filtered from FTS at
//! the trigger level and from vector search, but used to leak back through
//! the exact-slug short-circuit, `--hops` graph expansion, and `depth=auto`
//! progressive expansion. Retrieval must never surface quarantined pages;
//! they stay reachable only via explicit access paths (`memory_get`, raw).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

use quaid::core::db;
use quaid::core::progressive::progressive_retrieve;
use quaid::core::search::{hybrid_search, HybridSearch};
use quaid::core::types::SearchResult;
use rusqlite::Connection;

fn open_test_db() -> Connection {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    std::mem::forget(dir);
    conn
}

fn insert_page(conn: &Connection, slug: &str, truth: &str) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, 'concept', ?1, ?1, ?2, '', '{}', 'notes', '', 1)",
        rusqlite::params![slug, truth],
    )
    .unwrap();
}

fn quarantine_page(conn: &Connection, slug: &str) {
    let updated = conn
        .execute(
            "UPDATE pages SET quarantined_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
             WHERE slug = ?1",
            [slug],
        )
        .unwrap();
    assert_eq!(updated, 1, "quarantine fixture must hit exactly one page");
}

fn insert_link(conn: &Connection, from: &str, to: &str) {
    let from_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [from], |row| {
            row.get(0)
        })
        .unwrap();
    let to_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [to], |row| {
            row.get(0)
        })
        .unwrap();
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind) \
         VALUES (?1, ?2, 'related', 'programmatic')",
        rusqlite::params![from_id, to_id],
    )
    .unwrap();
}

fn make_result(slug: &str) -> SearchResult {
    SearchResult {
        slug: slug.to_owned(),
        title: slug.to_owned(),
        summary: slug.to_owned(),
        score: 1.0,
        wing: "notes".to_owned(),
        mmr_score: None,
        cross_ref_boost: 0.0,
        dedup_collapsed_count: 0,
    }
}

fn result_slugs(results: &[SearchResult]) -> Vec<String> {
    let mut slugs: Vec<String> = results.iter().map(|result| result.slug.clone()).collect();
    slugs.sort();
    slugs
}

#[test]
fn exact_slug_short_circuit_hides_quarantined_pages() {
    let conn = open_test_db();
    insert_page(&conn, "notes/poisoned", "Unparseable import body.");
    quarantine_page(&conn, "notes/poisoned");

    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "notes/poisoned",
            limit: 10,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(
        results.is_empty(),
        "quarantined page leaked through the exact-slug short-circuit: {results:?}",
        results = result_slugs(&results)
    );
}

#[test]
fn exact_slug_short_circuit_hides_quarantined_pages_in_canonical_mode() {
    let conn = open_test_db();
    insert_page(&conn, "notes/poisoned", "Unparseable import body.");
    quarantine_page(&conn, "notes/poisoned");

    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "notes/poisoned",
            canonical: true,
            limit: 10,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(
        results.is_empty(),
        "quarantined page leaked through the canonical exact-slug short-circuit: {results:?}",
        results = result_slugs(&results)
    );
}

#[test]
fn include_quarantined_flag_restores_explicit_exact_slug_access() {
    let conn = open_test_db();
    insert_page(&conn, "notes/poisoned", "Unparseable import body.");
    quarantine_page(&conn, "notes/poisoned");

    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "notes/poisoned",
            include_quarantined: true,
            limit: 10,
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(result_slugs(&results), vec!["notes/poisoned".to_owned()]);
}

#[test]
fn hops_graph_expansion_skips_quarantined_link_targets() {
    let conn = open_test_db();
    insert_page(&conn, "notes/source", "Source page body.");
    insert_page(&conn, "notes/healthy", "Healthy neighbour body.");
    insert_page(&conn, "notes/poisoned", "Quarantined neighbour body.");
    insert_link(&conn, "notes/source", "notes/healthy");
    insert_link(&conn, "notes/source", "notes/poisoned");
    quarantine_page(&conn, "notes/poisoned");

    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "notes/source",
            limit: 10,
            hops: Some(1),
            ..Default::default()
        },
    )
    .unwrap();

    let slugs = result_slugs(&results);
    assert!(
        slugs.contains(&"notes/healthy".to_owned()),
        "healthy neighbour must still arrive via graph expansion: {slugs:?}"
    );
    assert!(
        !slugs.contains(&"notes/poisoned".to_owned()),
        "quarantined page leaked through --hops graph expansion: {slugs:?}"
    );
}

#[test]
fn progressive_expansion_skips_quarantined_link_targets() {
    let conn = open_test_db();
    insert_page(&conn, "notes/source", "Source page body.");
    insert_page(&conn, "notes/healthy", "Healthy neighbour body.");
    insert_page(&conn, "notes/poisoned", "Quarantined neighbour body.");
    insert_link(&conn, "notes/source", "notes/healthy");
    insert_link(&conn, "notes/source", "notes/poisoned");
    quarantine_page(&conn, "notes/poisoned");

    let results = progressive_retrieve(
        vec![make_result("notes/source")],
        100_000,
        1,
        None,
        false,
        &conn,
    )
    .unwrap();

    let slugs = result_slugs(&results);
    assert_eq!(
        slugs,
        vec!["notes/healthy".to_owned(), "notes/source".to_owned()],
        "depth=auto expansion must skip quarantined neighbours"
    );
}

#[test]
fn direct_get_still_fetches_quarantined_pages_explicitly() {
    let conn = open_test_db();
    conn.execute(
        "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES ('notes/poisoned', '01969f11-9448-7d79-8d3f-c68f54761234', 'concept', \
                 'Poisoned', '', 'Unparseable import body.', '', '{}', 'notes', '', 1)",
        [],
    )
    .unwrap();
    quarantine_page(&conn, "notes/poisoned");

    let page = quaid::commands::get::get_page(&conn, "notes/poisoned")
        .expect("explicit get must still reach quarantined pages");
    assert_eq!(page.slug, "notes/poisoned");
}

#[test]
fn progressive_expansion_skips_quarantined_targets_even_with_include_superseded() {
    let conn = open_test_db();
    insert_page(&conn, "notes/source", "Source page body.");
    insert_page(&conn, "notes/poisoned", "Quarantined neighbour body.");
    insert_link(&conn, "notes/source", "notes/poisoned");
    quarantine_page(&conn, "notes/poisoned");

    let results = progressive_retrieve(
        vec![make_result("notes/source")],
        100_000,
        1,
        None,
        true,
        &conn,
    )
    .unwrap();

    assert_eq!(
        result_slugs(&results),
        vec!["notes/source".to_owned()],
        "include_superseded must not re-expose quarantined pages"
    );
}
