#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Changed-hash rotation and fail-closed paths through reconcile() and full_hash_reconcile_authorized().

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::file_state;
use quaid::core::reconciler::*;
use std::fs;
use tempfile::TempDir;

#[cfg(unix)]
#[test]
fn reconcile_changed_hash_modified_path_rotates_raw_imports_to_latest_bytes() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
    fs::write(root.path().join("note.md"), original).unwrap();
    let original_stat = stat_for(root.path(), "note.md");
    let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
    let page_id = seed_page_with_identity(
        &conn,
        collection.id,
        SeededPageIdentity {
            slug: "notes/note",
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
            relative_path: "note.md",
            stat: &original_stat,
            sha256: &original_sha,
            compiled_truth: "Original body.",
            timeline: "",
        },
    );
    quaid::core::raw_imports::rotate_active_raw_import(
        &conn,
        page_id,
        "note.md",
        original.as_bytes(),
    )
    .unwrap();

    let updated =
        "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body that is deliberately longer.\n";
    fs::write(root.path().join("note.md"), updated).unwrap();

    let stats = reconcile(&conn, &collection).unwrap();
    let file_state_row = quaid::core::file_state::get_file_state(&conn, collection.id, "note.md")
        .unwrap()
        .expect("modified path should still be tracked");
    let inactive_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1 AND is_active = 0",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    let compiled_truth: String = conn
        .query_row(
            "SELECT compiled_truth FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(stats.modified, 1);
    assert_eq!(stats.unchanged, 0);
    assert_eq!(
        file_state_row.sha256,
        file_state::hash_file(&root.path().join("note.md")).unwrap()
    );
    assert_eq!(active_raw_import_count(&conn, page_id), 1);
    assert_eq!(active_raw_import_bytes(&conn, page_id), updated.as_bytes());
    assert_eq!(inactive_count, 1);
    assert_eq!(
        compiled_truth.trim_end(),
        "Updated body that is deliberately longer."
    );
}

#[cfg(unix)]
#[test]
fn reconcile_changed_hash_aborts_before_mutation_when_history_has_zero_active_raw_import_rows() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
    fs::write(root.path().join("note.md"), original).unwrap();
    let original_stat = stat_for(root.path(), "note.md");
    let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
    let page_id = seed_page_with_identity(
        &conn,
        collection.id,
        SeededPageIdentity {
            slug: "notes/note",
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
            relative_path: "note.md",
            stat: &original_stat,
            sha256: &original_sha,
            compiled_truth: "Original body.",
            timeline: "",
        },
    );
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
         VALUES (?1, ?2, 0, ?3, ?4)",
        rusqlite::params![
            page_id,
            quaid::core::page_uuid::generate_uuid_v7(),
            original.as_bytes(),
            "note.md"
        ],
    )
    .unwrap();
    let before_row = quaid::core::file_state::get_file_state(&conn, collection.id, "note.md")
        .unwrap()
        .expect("file_state row should exist");

    let updated =
        "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body that is deliberately longer.\n";
    fs::write(root.path().join("note.md"), updated).unwrap();

    let error = reconcile(&conn, &collection).unwrap_err().to_string();
    let after_row = quaid::core::file_state::get_file_state(&conn, collection.id, "note.md")
        .unwrap()
        .expect("file_state row should still exist after abort");
    let compiled_truth: String = conn
        .query_row(
            "SELECT compiled_truth FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    let raw_import_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();

    assert!(error.contains("InvariantViolationError"));
    assert_eq!(compiled_truth, "Original body.");
    assert_eq!(after_row.sha256, before_row.sha256);
    assert_eq!(active_raw_import_count(&conn, page_id), 0);
    assert_eq!(raw_import_rows, 1);
}

#[cfg(unix)]
#[test]
fn full_hash_reconcile_changed_hash_rotates_raw_imports_atomically() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
    fs::write(root.path().join("note.md"), original).unwrap();
    let original_stat = stat_for(root.path(), "note.md");
    let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
    let page_id = seed_page_with_identity(
        &conn,
        collection.id,
        SeededPageIdentity {
            slug: "notes/note",
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
            relative_path: "note.md",
            stat: &original_stat,
            sha256: &original_sha,
            compiled_truth: "Original body.",
            timeline: "",
        },
    );
    quaid::core::raw_imports::rotate_active_raw_import(
        &conn,
        page_id,
        "note.md",
        original.as_bytes(),
    )
    .unwrap();

    let updated =
        "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body that is deliberately longer.\n";
    fs::write(root.path().join("note.md"), updated).unwrap();

    let stats = full_hash_reconcile(&conn, collection.id).unwrap();
    let file_state_row = quaid::core::file_state::get_file_state(&conn, collection.id, "note.md")
        .unwrap()
        .expect("file_state row should still exist");
    let inactive_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1 AND is_active = 0",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(stats.modified, 1);
    assert_eq!(active_raw_import_count(&conn, page_id), 1);
    assert_eq!(active_raw_import_bytes(&conn, page_id), updated.as_bytes());
    assert_eq!(inactive_count, 1);
    assert_eq!(
        file_state_row.sha256,
        file_state::hash_file(&root.path().join("note.md")).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn reconcile_fails_closed_when_existing_page_has_zero_total_raw_imports_on_modified_path() {
    // Nibbler adversarial seam: existing page on the stat-diff modified path with
    // row_count == 0 must fail with InvariantViolationError, not silently bootstrap.
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let original = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nOriginal body.\n";
    fs::write(root.path().join("note.md"), original).unwrap();
    let original_stat = stat_for(root.path(), "note.md");
    let original_sha = file_state::hash_file(&root.path().join("note.md")).unwrap();
    let page_id = seed_page_with_identity(
        &conn,
        collection.id,
        SeededPageIdentity {
            slug: "notes/note",
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
            relative_path: "note.md",
            stat: &original_stat,
            sha256: &original_sha,
            compiled_truth: "Original body.",
            timeline: "",
        },
    );
    // Intentionally leave raw_imports empty (row_count == 0, not just active_count == 0).
    let raw_import_rows_before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(raw_import_rows_before, 0);

    let updated = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nUpdated body.\n";
    fs::write(root.path().join("note.md"), updated).unwrap();

    let error = reconcile(&conn, &collection).unwrap_err().to_string();
    let compiled_truth: String = conn
        .query_row(
            "SELECT compiled_truth FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    let raw_import_rows_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();

    assert!(error.contains("InvariantViolationError"));
    assert_eq!(compiled_truth, "Original body.", "page must not be mutated");
    assert_eq!(
        raw_import_rows_after, 0,
        "no raw_imports row must be bootstrapped"
    );
}

#[cfg(unix)]
#[test]
fn reconcile_fails_closed_when_slug_matched_existing_page_has_zero_total_raw_imports() {
    // Nibbler adversarial seam: existing page found via slug-match on the remaining_new
    // path (existing_page_id = None at action construction time) must also fail closed
    // when row_count == 0.
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());

    // Insert page directly into pages with no file_state row — the stat-diff walk will
    // never see it as modified/missing; it's invisible to rename resolution.
    let page_id = insert_page(&conn, collection.id, "notes/note");
    let raw_import_rows_before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(raw_import_rows_before, 0);

    // A new file appears with a slug that matches the existing DB page.
    let content = "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nNew path body.\n";
    fs::write(root.path().join("new.md"), content).unwrap();

    // reconcile: "new.md" is in remaining_new (no file_state entry),
    // apply_reingest is called with existing_page_id = None,
    // load_existing_page_identity finds the DB page by slug "notes/note",
    // the zero-total-rows guard must fire before any mutation.
    let error = reconcile(&conn, &collection).unwrap_err().to_string();
    let raw_import_rows_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();

    assert!(error.contains("InvariantViolationError"));
    assert_eq!(
        raw_import_rows_after, 0,
        "no raw_imports row must be bootstrapped"
    );
}
