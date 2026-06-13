#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Watcher-ingest supersede wiring: `apply_reingest` must honour the
//! `supersedes:` frontmatter that the extraction pipeline writes, retiring
//! the prior head exactly like the synchronous `quaid ingest` path does, so
//! a `(kind, key)` partition never accumulates two live heads.

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::reconciler::reconcile;
use rusqlite::Connection;
use std::fs;
use tempfile::TempDir;

fn page_row(conn: &Connection, slug: &str) -> (i64, Option<i64>) {
    conn.query_row(
        "SELECT id, superseded_by FROM pages WHERE collection_id = 1 AND slug = ?1",
        [slug],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

fn preference_fact_markdown(slug: &str, supersedes: Option<&str>, body: &str) -> String {
    let mut out = format!(
        "---\nabout: beverage\nkind: preference\nslug: {slug}\ntitle: beverage\ntype: preference\n"
    );
    if let Some(supersedes) = supersedes {
        out.push_str(&format!("supersedes: {supersedes}\n"));
    }
    out.push_str("---\n");
    out.push_str(body);
    out.push('\n');
    out
}

#[cfg(unix)]
#[test]
fn reconcile_wires_superseded_by_from_supersedes_frontmatter() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let fact_dir = root.path().join("extracted").join("preferences");
    fs::create_dir_all(&fact_dir).unwrap();

    fs::write(
        fact_dir.join("beverage-aaaa.md"),
        preference_fact_markdown(
            "beverage-aaaa",
            None,
            "The user prefers tea over coffee in the evening.",
        ),
    )
    .unwrap();
    reconcile(&conn, &collection).unwrap();

    // The extractor's supersede resolution writes a new file carrying
    // `supersedes:` frontmatter; the watcher's reingest must retire the
    // prior head when it picks the file up.
    fs::write(
        fact_dir.join("beverage-bbbb.md"),
        preference_fact_markdown(
            "beverage-bbbb",
            Some("beverage-aaaa"),
            "The user now prefers coffee at all times of day.",
        ),
    )
    .unwrap();
    reconcile(&conn, &collection).unwrap();

    let (prior_id, prior_superseded_by) = page_row(&conn, "beverage-aaaa");
    let (successor_id, successor_superseded_by) = page_row(&conn, "beverage-bbbb");
    assert_eq!(
        prior_superseded_by,
        Some(successor_id),
        "prior head must be retired by the watcher reingest"
    );
    assert_eq!(
        successor_superseded_by, None,
        "the superseding fact must be the sole live head"
    );
    assert_ne!(prior_id, successor_id);

    let live_heads: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages
             WHERE collection_id = 1
               AND type = 'preference'
               AND superseded_by IS NULL
               AND json_extract(frontmatter, '$.about') = 'beverage'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        live_heads, 1,
        "the (preference, beverage) partition must have exactly one live head"
    );
}

#[cfg(unix)]
#[test]
fn reconcile_reingest_is_idempotent_for_already_wired_supersede_chain() {
    // Concept pages bypass the extracted-file edit flow, so a modified
    // reingest re-runs the full apply path — including the supersede chain
    // reconciliation — against an already-wired chain. The chain shape must
    // stay stable across the rerun.
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let notes_dir = root.path().join("notes");
    fs::create_dir_all(&notes_dir).unwrap();

    fs::write(
        notes_dir.join("note-a.md"),
        "---\nslug: notes/a\ntitle: Note A\ntype: concept\n---\nOriginal body that is long enough to ingest cleanly.\n",
    )
    .unwrap();
    reconcile(&conn, &collection).unwrap();
    fs::write(
        notes_dir.join("note-b.md"),
        "---\nslug: notes/b\nsupersedes: notes/a\ntitle: Note B\ntype: concept\n---\nReplacement body that is long enough to ingest cleanly.\n",
    )
    .unwrap();
    reconcile(&conn, &collection).unwrap();

    let (_, prior_superseded_by) = page_row(&conn, "notes/a");
    let (successor_id, _) = page_row(&conn, "notes/b");
    assert_eq!(prior_superseded_by, Some(successor_id), "chain wired");

    // Modify the successor's body: the reingest re-runs the supersede chain
    // call with the same target, which must be a no-op rather than an error.
    fs::write(
        notes_dir.join("note-b.md"),
        "---\nslug: notes/b\nsupersedes: notes/a\ntitle: Note B\ntype: concept\n---\nEdited replacement body that is still long enough to ingest cleanly.\n",
    )
    .unwrap();
    reconcile(&conn, &collection).unwrap();

    let (_, prior_superseded_by) = page_row(&conn, "notes/a");
    let (successor_id_after, successor_superseded_by) = page_row(&conn, "notes/b");
    assert_eq!(successor_id_after, successor_id);
    assert_eq!(prior_superseded_by, Some(successor_id));
    assert_eq!(successor_superseded_by, None);
}

#[cfg(unix)]
#[test]
fn reconcile_tolerates_unresolvable_supersedes_target() {
    // An externally edited file naming a nonexistent supersede target must
    // not wedge the reconcile pass: the page still ingests, the chain is
    // simply left unwired (logged at WARN).
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let fact_dir = root.path().join("extracted").join("preferences");
    fs::create_dir_all(&fact_dir).unwrap();

    fs::write(
        fact_dir.join("beverage-cccc.md"),
        preference_fact_markdown(
            "beverage-cccc",
            Some("no-such-slug"),
            "The user prefers sparkling water.",
        ),
    )
    .unwrap();
    reconcile(&conn, &collection).unwrap();

    let (_, superseded_by) = page_row(&conn, "beverage-cccc");
    assert_eq!(superseded_by, None);
    let page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(page_count, 1, "the page itself must still be ingested");
}
