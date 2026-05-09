#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! public-API reconcile() idempotency, unchanged-path, and symlink-boundary tests.

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::file_state;
use quaid::core::reconciler::*;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use tempfile::TempDir;

#[cfg(unix)]
#[test]
fn reconcile_is_idempotent_when_disk_matches_file_state() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("note.md"), "# note").unwrap();
    let collection = insert_collection(&conn, root.path());
    let stat = stat_for(root.path(), "note.md");
    seed_file_state(&conn, collection.id, "notes/note", "note.md", &stat);

    let first = reconcile(&conn, &collection).unwrap();
    let second = reconcile(&conn, &collection).unwrap();

    assert_eq!(first.walked, 1);
    assert_eq!(first.unchanged, 1);
    assert_eq!(first.modified, 0);
    assert_eq!(first.new, 0);
    assert_eq!(first.missing, 0);
    assert_eq!(first.walked, second.walked);
    assert_eq!(first.unchanged, second.unchanged);
    assert_eq!(first.modified, second.modified);
    assert_eq!(first.new, second.new);
    assert_eq!(first.missing, second.missing);
}

#[cfg(unix)]
#[test]
fn reconcile_unchanged_path_keeps_existing_raw_import_row_without_rotation() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let content = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nStable body.\n";
    fs::write(root.path().join("note.md"), content).unwrap();
    let collection = insert_collection(&conn, root.path());
    let stat = stat_for(root.path(), "note.md");
    let sha256 = file_state::hash_file(&root.path().join("note.md")).unwrap();
    let page_id = seed_page_with_identity(
        &conn,
        collection.id,
        SeededPageIdentity {
            slug: "notes/note",
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
            relative_path: "note.md",
            stat: &stat,
            sha256: &sha256,
            compiled_truth: "Stable body.",
            timeline: "",
        },
    );
    quaid::core::raw_imports::rotate_active_raw_import(
        &conn,
        page_id,
        "note.md",
        content.as_bytes(),
    )
    .unwrap();

    let before_full_hash_at =
        quaid::core::file_state::get_file_state(&conn, collection.id, "note.md")
            .unwrap()
            .expect("file_state row should exist")
            .last_full_hash_at;
    let stats = reconcile(&conn, &collection).unwrap();
    let after_row = quaid::core::file_state::get_file_state(&conn, collection.id, "note.md")
        .unwrap()
        .expect("file_state row should still exist");
    let raw_import_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(stats.unchanged, 1);
    assert_eq!(stats.modified, 0);
    assert_eq!(active_raw_import_count(&conn, page_id), 1);
    assert_eq!(raw_import_rows, 1);
    assert_eq!(active_raw_import_bytes(&conn, page_id), content.as_bytes());
    assert_eq!(after_row.last_full_hash_at, before_full_hash_at);
}

#[cfg(unix)]
#[test]
fn reconcile_skips_symlinked_entries_at_the_reconciler_boundary() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("notes")).unwrap();
    fs::write(root.path().join("notes").join("real.md"), "# real").unwrap();
    symlink(
        root.path().join("notes").join("real.md"),
        root.path().join("notes").join("real-link.md"),
    )
    .unwrap();
    fs::create_dir_all(root.path().join("actual")).unwrap();
    fs::write(root.path().join("actual").join("inside.md"), "# hidden").unwrap();
    symlink(root.path().join("actual"), root.path().join("linked-dir")).unwrap();

    let collection = insert_collection(&conn, root.path());

    let stats = reconcile(&conn, &collection).unwrap();

    assert_eq!(stats.walked, 2);
    assert_eq!(stats.new, 2);
    assert_eq!(stats.modified, 0);
    assert_eq!(stats.missing, 0);
}
