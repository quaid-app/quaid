//! Export keying regressions: `export_dir` used to be collection/namespace
//! blind (same-slug pages overwrote each other) and aborted the entire
//! export when a single row had a NULL uuid. Exports are now keyed by
//! `(collection, namespace, slug)` while the flat `<slug>.md` layout is
//! preserved for the common single-collection vault.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

use quaid::core::db;
use quaid::core::migrate::export_dir;
use rusqlite::Connection;

fn open_test_db() -> Connection {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    std::mem::forget(dir);
    conn
}

fn insert_collection(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target)
         VALUES (?1, ?2, 'active', 1, 0)",
        rusqlite::params![name, format!("/{name}")],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn insert_page(
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    slug: &str,
    uuid: Option<&str>,
    truth: &str,
) {
    conn.execute(
        "INSERT INTO pages (collection_id, namespace, slug, uuid, type, title, summary, \
                            compiled_truth, timeline, frontmatter, wing, room, version) \
         VALUES (?1, ?2, ?3, ?4, 'concept', ?3, '', ?5, '', '{}', 'notes', '', 1)",
        rusqlite::params![collection_id, namespace, slug, uuid, truth],
    )
    .unwrap();
}

#[test]
fn export_keys_same_slug_pages_by_collection_and_skips_null_uuid_rows() {
    let conn = open_test_db();
    let work_id = insert_collection(&conn, "work");
    insert_page(
        &conn,
        1,
        "",
        "people/alice",
        Some("01969f11-9448-7d79-8d3f-c68f54761234"),
        "Alice in the default collection.",
    );
    insert_page(
        &conn,
        work_id,
        "",
        "people/alice",
        Some("01969f11-9448-7d79-8d3f-c68f54761235"),
        "Alice in the work collection.",
    );
    insert_page(&conn, 1, "", "people/legacy", None, "Row without a uuid.");

    let out = tempfile::TempDir::new().unwrap();
    let count = export_dir(&conn, out.path()).unwrap();

    assert_eq!(count, 2, "two exportable pages, one NULL-uuid row skipped");

    let default_file = out.path().join("default/people/alice.md");
    let work_file = out.path().join("work/people/alice.md");
    assert!(
        default_file.exists() && work_file.exists(),
        "same-slug pages must export to per-collection paths"
    );
    assert!(
        !out.path().join("people/alice.md").exists(),
        "flat layout must not be used when several collections exist"
    );
    assert!(
        std::fs::read_to_string(&default_file)
            .unwrap()
            .contains("Alice in the default collection."),
        "default-collection content must not be overwritten by the work page"
    );
    assert!(std::fs::read_to_string(&work_file)
        .unwrap()
        .contains("Alice in the work collection."));
    assert!(
        !out.path().join("default/people/legacy.md").exists(),
        "NULL-uuid rows are skipped, not exported"
    );
}

#[test]
fn export_preserves_flat_layout_for_single_collection_without_namespaces() {
    let conn = open_test_db();
    insert_page(
        &conn,
        1,
        "",
        "people/alice",
        Some("01969f11-9448-7d79-8d3f-c68f54761234"),
        "Alice content.",
    );
    insert_page(
        &conn,
        1,
        "",
        "notes/today",
        Some("01969f11-9448-7d79-8d3f-c68f54761235"),
        "Today content.",
    );

    let out = tempfile::TempDir::new().unwrap();
    let count = export_dir(&conn, out.path()).unwrap();

    assert_eq!(count, 2);
    assert!(
        out.path().join("people/alice.md").exists() && out.path().join("notes/today.md").exists(),
        "single default collection keeps the legacy flat <slug>.md layout"
    );
    assert!(!out.path().join("default").exists());
}

#[test]
fn export_keys_same_slug_pages_by_namespace() {
    let conn = open_test_db();
    insert_page(
        &conn,
        1,
        "",
        "people/alice",
        Some("01969f11-9448-7d79-8d3f-c68f54761234"),
        "Global-namespace Alice.",
    );
    insert_page(
        &conn,
        1,
        "agent-a",
        "people/alice",
        Some("01969f11-9448-7d79-8d3f-c68f54761235"),
        "Namespaced Alice.",
    );

    let out = tempfile::TempDir::new().unwrap();
    let count = export_dir(&conn, out.path()).unwrap();

    assert_eq!(count, 2);
    assert!(
        out.path().join("default/people/alice.md").exists(),
        "global-namespace page exports under the collection directory"
    );
    assert!(
        out.path().join("default/agent-a/people/alice.md").exists(),
        "namespaced page exports under <collection>/<namespace>/"
    );
}
