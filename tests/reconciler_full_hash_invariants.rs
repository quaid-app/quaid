#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! full_hash_reconcile_authorized invariants: zero-row aborts and unchanged-hash skips.

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::file_state;
use quaid::core::reconciler::*;
use std::fs;
use tempfile::TempDir;

#[cfg(unix)]
#[test]
fn full_hash_reconcile_aborts_before_mutation_when_a_page_has_zero_total_raw_import_rows() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("note.md"), "# note").unwrap();
    let collection = insert_collection(&conn, root.path());
    let stat = stat_for(root.path(), "note.md");
    let page_id = seed_file_state(&conn, collection.id, "notes/note", "note.md", &stat);

    assert_eq!(active_raw_import_count(&conn, page_id), 0);

    let err = full_hash_reconcile(&conn, collection.id)
        .unwrap_err()
        .to_string();
    assert!(err.contains("InvariantViolation"));
}

#[cfg(unix)]
#[test]
fn full_hash_reconcile_aborts_before_mutation_when_history_has_zero_active_raw_import_rows() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let content = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body.\n";
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
            compiled_truth: "Updated body.",
            timeline: "",
        },
    );
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
         VALUES (?1, ?2, 0, ?3, ?4)",
        rusqlite::params![
            page_id,
            quaid::core::page_uuid::generate_uuid_v7(),
            b"stale",
            "note.md"
        ],
    )
    .unwrap();

    let err = full_hash_reconcile(&conn, collection.id)
        .unwrap_err()
        .to_string();

    assert!(err.contains("InvariantViolation"));
    assert_eq!(active_raw_import_count(&conn, page_id), 0);
}

#[cfg(unix)]
#[test]
fn full_hash_reconcile_unchanged_hash_updates_only_last_full_hash_at_without_rotation() {
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
    conn.execute(
        "UPDATE file_state
         SET last_full_hash_at = '2000-01-01T00:00:00Z'
         WHERE collection_id = ?1 AND relative_path = 'note.md'",
        [collection.id],
    )
    .unwrap();

    let before_row = quaid::core::file_state::get_file_state(&conn, collection.id, "note.md")
        .unwrap()
        .expect("file_state row should exist");

    let stats = full_hash_reconcile(&conn, collection.id).unwrap();
    let after_row = quaid::core::file_state::get_file_state(&conn, collection.id, "note.md")
        .unwrap()
        .expect("file_state row should still exist");

    assert_eq!(stats.unchanged, 1);
    assert_eq!(stats.modified, 0);
    assert_eq!(active_raw_import_count(&conn, page_id), 1);
    assert_eq!(active_raw_import_bytes(&conn, page_id), content.as_bytes());
    assert_eq!(after_row.sha256, before_row.sha256);
    assert_ne!(after_row.last_full_hash_at, before_row.last_full_hash_at);
}
