#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure"
)]

//! Wave 3 integration tests — structured frontmatter persistence and
//! side-table sync wired into the write paths (`put`, `ingest`, reconciler
//! directory imports, and exports).
//!
//! Locks in:
//!   - frontmatter `links:` (object + string shorthand) become `frontmatter`
//!     rows in `links`,
//!   - body `[[wikilinks]]` become `wiki_link` rows in `links`,
//!   - `tags:` become rows in `tags`,
//!   - structured frontmatter (arrays/objects) survive export → re-ingest,
//!   - stale derived edges are deleted when frontmatter changes,
//!   - malformed frontmatter graph input fails BEFORE any DB mutation.

#[path = "common/put_fixtures.rs"]
mod put_fixtures;

use put_fixtures::{open_test_db, page_id_for_slug};
use quaid::commands::{ingest, put::put_from_string};
use quaid::core::migrate;
use rusqlite::Connection;

// ── helpers ─────────────────────────────────────────────────

fn derived_targets(conn: &Connection, from_slug: &str, source_kind: &str) -> Vec<String> {
    let from_id = page_id_for_slug(conn, from_slug);
    let mut stmt = conn
        .prepare(
            "SELECT p.slug FROM links l JOIN pages p ON p.id = l.to_page_id \
             WHERE l.from_page_id = ?1 AND l.source_kind = ?2 ORDER BY p.slug",
        )
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![from_id, source_kind], |row| row.get(0))
        .unwrap();
    let mut out = Vec::new();
    for row in rows {
        out.push(row.unwrap());
    }
    out
}

fn tags_for(conn: &Connection, slug: &str) -> Vec<String> {
    let page_id = page_id_for_slug(conn, slug);
    let mut stmt = conn
        .prepare("SELECT tag FROM tags WHERE page_id = ?1 ORDER BY tag")
        .unwrap();
    let rows = stmt.query_map([page_id], |row| row.get(0)).unwrap();
    let mut out = Vec::new();
    for row in rows {
        out.push(row.unwrap());
    }
    out
}

fn seed_target(conn: &Connection, slug: &str) {
    put_from_string(
        conn,
        slug,
        "---\ntitle: target\ntype: concept\n---\nstub\n",
        None,
    )
    .unwrap();
}

// ── put: frontmatter edges + wikilinks + tags ──────────────────

#[test]
fn put_wires_frontmatter_links_and_wikilinks_and_tags_inside_one_write() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    seed_target(&conn, "people/bob");
    seed_target(&conn, "people/carol");

    let md = concat!(
        "---\n",
        "title: Alice\n",
        "type: person\n",
        "tags: [founder, investor]\n",
        "links:\n",
        "  - target: companies/brex\n",
        "    type: founded\n",
        "    valid_from: 2017-01-01\n",
        "related:\n",
        "  - people/bob\n",
        "---\n",
        "# Alice\n\nAlice works with [[people/carol]] and [[people/bob]].\n",
    );

    put_from_string(&conn, "people/alice", md, None).unwrap();

    let fm_targets = derived_targets(&conn, "people/alice", "frontmatter");
    assert!(fm_targets.contains(&"companies/brex".to_string()));
    assert!(fm_targets.contains(&"people/bob".to_string()));

    let wiki_targets = derived_targets(&conn, "people/alice", "wiki_link");
    assert!(wiki_targets.contains(&"people/carol".to_string()));
    assert!(wiki_targets.contains(&"people/bob".to_string()));

    let tags = tags_for(&conn, "people/alice");
    assert_eq!(tags, vec!["founder", "investor"]);

    // frontmatter edge carries temporal validity
    let from_id = page_id_for_slug(&conn, "people/alice");
    let valid_from: Option<String> = conn
        .query_row(
            "SELECT valid_from FROM links \
             WHERE from_page_id = ?1 AND source_kind = 'frontmatter' AND relationship = 'founded'",
            [from_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(valid_from.as_deref(), Some("2017-01-01"));
}

#[test]
fn put_replaces_stale_derived_edges_and_tags_on_re_put() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    seed_target(&conn, "companies/acme");

    let v1 = "---\ntitle: Alice\ntype: person\ntags: [a]\nlinks:\n  - target: companies/brex\n    type: founded\n---\nbody [[companies/brex]]\n";
    put_from_string(&conn, "people/alice", v1, None).unwrap();
    assert_eq!(
        derived_targets(&conn, "people/alice", "frontmatter"),
        vec!["companies/brex".to_string()]
    );
    assert_eq!(tags_for(&conn, "people/alice"), vec!["a"]);

    let v2 = "---\ntitle: Alice\ntype: person\ntags: [b, c]\nlinks:\n  - target: companies/acme\n    type: founded\n---\nbody [[companies/acme]]\n";
    put_from_string(&conn, "people/alice", v2, Some(1)).unwrap();

    let fm = derived_targets(&conn, "people/alice", "frontmatter");
    assert_eq!(fm, vec!["companies/acme".to_string()]);
    let wiki = derived_targets(&conn, "people/alice", "wiki_link");
    assert_eq!(wiki, vec!["companies/acme".to_string()]);
    assert_eq!(tags_for(&conn, "people/alice"), vec!["b", "c"]);
}

#[test]
fn put_with_malformed_frontmatter_links_fails_before_any_db_write() {
    let conn = open_test_db();
    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();

    // `links:` must be a list — pass a scalar to trigger an InvalidShape error.
    let md = "---\ntitle: Bad\ntype: person\nlinks: not-a-list\n---\nbody\n";
    let err = put_from_string(&conn, "people/bad", md, None).unwrap_err();
    assert!(
        err.to_string()
            .contains("malformed frontmatter graph input"),
        "unexpected error: {err}"
    );

    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        before, after,
        "page row must NOT be created on parse failure"
    );

    // No tags or links rows leaked either.
    let leaked_tags: i64 = conn
        .query_row("SELECT COUNT(*) FROM tags", [], |row| row.get(0))
        .unwrap();
    assert_eq!(leaked_tags, 0);
}

// ── ingest: same contract ──────────────────────────────────────

#[test]
fn ingest_wires_frontmatter_links_wikilinks_and_tags() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    seed_target(&conn, "people/bob");

    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("alice.md");
    let md = concat!(
        "---\n",
        "slug: people/alice\n",
        "title: Alice\n",
        "type: person\n",
        "tags: [a, b]\n",
        "links:\n",
        "  - companies/brex\n",
        "---\nbody [[people/bob]]\n",
    );
    std::fs::write(&path, md).unwrap();

    ingest::run(&conn, path.to_str().unwrap(), false).unwrap();

    let fm = derived_targets(&conn, "people/alice", "frontmatter");
    assert!(fm.contains(&"companies/brex".to_string()));
    let wiki = derived_targets(&conn, "people/alice", "wiki_link");
    assert!(wiki.contains(&"people/bob".to_string()));
    assert_eq!(tags_for(&conn, "people/alice"), vec!["a", "b"]);
}

#[test]
fn ingest_with_malformed_frontmatter_graph_fails_before_mutation() {
    let conn = open_test_db();
    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();

    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("bad.md");
    let md = "---\nslug: people/bad\ntitle: Bad\nparent: 42\n---\nbody\n";
    std::fs::write(&path, md).unwrap();

    let err = ingest::run(&conn, path.to_str().unwrap(), false).unwrap_err();
    assert!(
        err.to_string()
            .contains("malformed frontmatter graph input"),
        "unexpected error: {err}"
    );

    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, after);
}

// ── export → re-ingest preserves structured frontmatter + edges ───

#[test]
fn export_then_reingest_preserves_links_tags_and_structured_frontmatter() {
    let conn = open_test_db();
    seed_target(&conn, "companies/brex");
    seed_target(&conn, "people/bob");

    let md = concat!(
        "---\n",
        "slug: people/alice\n",
        "title: Alice\n",
        "type: person\n",
        "tags: [a, b]\n",
        "links:\n",
        "  - target: companies/brex\n",
        "    type: founded\n",
        "    valid_from: 2017-01-01\n",
        "related:\n",
        "  - people/bob\n",
        "---\nAlice prose.\n",
    );
    put_from_string(&conn, "people/alice", md, None).unwrap();

    let export_dir = tempfile::TempDir::new().unwrap();
    migrate::export_dir(&conn, export_dir.path()).unwrap();

    // Fresh DB, re-ingest the exported tree.
    let conn2 = open_test_db();
    seed_target(&conn2, "companies/brex");
    seed_target(&conn2, "people/bob");
    let alice_md = export_dir.path().join("people").join("alice.md");
    ingest::run(&conn2, alice_md.to_str().unwrap(), false).unwrap();

    // Edges and tags round-trip.
    let fm = derived_targets(&conn2, "people/alice", "frontmatter");
    assert!(
        fm.contains(&"companies/brex".to_string()),
        "frontmatter edge to companies/brex missing after roundtrip: {fm:?}"
    );
    assert!(
        fm.contains(&"people/bob".to_string()),
        "frontmatter edge to people/bob (related) missing after roundtrip: {fm:?}"
    );
    assert_eq!(tags_for(&conn2, "people/alice"), vec!["a", "b"]);

    // Structured frontmatter `links:` value survives as a JSON array.
    let frontmatter_json: String = conn2
        .query_row(
            "SELECT frontmatter FROM pages WHERE slug = 'people/alice'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let frontmatter: serde_json::Value = serde_json::from_str(&frontmatter_json).unwrap();
    assert!(
        frontmatter
            .get("links")
            .map(|v| v.is_array())
            .unwrap_or(false),
        "links should round-trip as a YAML/JSON array: {frontmatter_json}"
    );
}
