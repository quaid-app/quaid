#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `quaid collection ignore` (the public
//! `CollectionAction::Ignore` surface).
//!
//! Covers the add/remove/clear/list ignore subcommands, file/mirror
//! synchronization, glob validation, and the reconcile interaction with
//! pages already indexed under matching paths.

use std::fs;

use quaid::commands::collection::{run, CollectionAction, CollectionIgnoreAction};

#[path = "common/collection_fixtures.rs"]
mod fixtures;
use fixtures::{
    attach_collection, collection_page_count, fetch_ignore_mirror, insert_collection, open_test_db,
    open_test_db_file,
};

#[cfg(unix)]
#[test]
fn ignore_add_updates_file_mirror_and_reconciles() {
    let (_db_dir, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    fs::write(
        root.path().join("note.md"),
        "---\ntitle: Note\ntype: note\n---\nhello\n",
    )
    .unwrap();
    attach_collection(&conn, "work", root.path());

    run(
        &conn,
        CollectionAction::Ignore {
            action: CollectionIgnoreAction::Add {
                name: "work".to_owned(),
                pattern: "note.md".to_owned(),
            },
        },
        true,
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(root.path().join(".quaidignore")).unwrap(),
        "note.md\n"
    );
    let mirror: Vec<String> =
        serde_json::from_str(&fetch_ignore_mirror(&conn, "work").unwrap()).unwrap();
    assert_eq!(mirror, vec!["note.md"]);
    assert_eq!(collection_page_count(&conn, "work"), 0);
}

#[cfg(unix)]
#[test]
fn ignore_clear_removes_file_clears_mirror_and_reconciles() {
    let (_db_dir, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    fs::write(
        root.path().join("note.md"),
        "---\ntitle: Note\ntype: note\n---\nhello\n",
    )
    .unwrap();
    attach_collection(&conn, "work", root.path());
    run(
        &conn,
        CollectionAction::Ignore {
            action: CollectionIgnoreAction::Add {
                name: "work".to_owned(),
                pattern: "note.md".to_owned(),
            },
        },
        true,
    )
    .unwrap();

    run(
        &conn,
        CollectionAction::Ignore {
            action: CollectionIgnoreAction::Clear {
                name: "work".to_owned(),
                confirm: true,
            },
        },
        true,
    )
    .unwrap();

    assert!(!root.path().join(".quaidignore").exists());
    assert!(fetch_ignore_mirror(&conn, "work").is_none());
    assert_eq!(collection_page_count(&conn, "work"), 1);
}

#[cfg(unix)]
#[test]
fn ignore_add_invalid_glob_refuses_without_disk_or_db_mutation() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    insert_collection(&conn, "work", root.path());

    let error = run(
        &conn,
        CollectionAction::Ignore {
            action: CollectionIgnoreAction::Add {
                name: "work".to_owned(),
                pattern: "[broken".to_owned(),
            },
        },
        true,
    )
    .unwrap_err();

    assert!(error.to_string().contains("Invalid glob pattern"));
    assert!(!root.path().join(".quaidignore").exists());
    assert!(fetch_ignore_mirror(&conn, "work").is_none());
}

#[cfg(unix)]
#[test]
fn ignore_remove_updates_file_and_mirror() {
    let (_db_dir, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    fs::write(
        root.path().join("note.md"),
        "---\ntitle: Note\ntype: note\n---\nhello\n",
    )
    .unwrap();
    attach_collection(&conn, "work", root.path());
    run(
        &conn,
        CollectionAction::Ignore {
            action: CollectionIgnoreAction::Add {
                name: "work".to_owned(),
                pattern: "note.md".to_owned(),
            },
        },
        true,
    )
    .unwrap();
    run(
        &conn,
        CollectionAction::Ignore {
            action: CollectionIgnoreAction::Add {
                name: "work".to_owned(),
                pattern: "archive/**".to_owned(),
            },
        },
        true,
    )
    .unwrap();

    run(
        &conn,
        CollectionAction::Ignore {
            action: CollectionIgnoreAction::Remove {
                name: "work".to_owned(),
                pattern: "note.md".to_owned(),
            },
        },
        true,
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(root.path().join(".quaidignore")).unwrap(),
        "archive/**\n"
    );
    let mirror: Vec<String> =
        serde_json::from_str(&fetch_ignore_mirror(&conn, "work").unwrap()).unwrap();
    assert_eq!(mirror, vec!["archive/**"]);
    assert_eq!(collection_page_count(&conn, "work"), 1);
}

#[test]
fn run_routes_ignore_list_action() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    conn.execute(
        "UPDATE collections SET ignore_patterns = ?2 WHERE id = ?1",
        rusqlite::params![
            collection_id,
            serde_json::to_string(&vec!["private/**"]).unwrap()
        ],
    )
    .unwrap();

    run(
        &conn,
        CollectionAction::Ignore {
            action: CollectionIgnoreAction::List {
                name: "work".to_owned(),
            },
        },
        true,
    )
    .unwrap();
}
