#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `quaid collection add` (the public `run`/`CollectionAction::Add` surface).
//!
//! Covers root-validation refusals, ignore-file refusals, write-quaid-id
//! conflicts, successful attach with lease cleanup, and the read-only fallback
//! triggered when the writable probe hits `EACCES`.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use quaid::commands::collection::{run, CollectionAction, CollectionAddArgs};
use quaid::core::migrate::export_dir;

#[path = "common/collection_fixtures.rs"]
mod fixtures;
use fixtures::{open_test_db, open_test_db_file};

#[cfg(unix)]
#[test]
fn add_refuses_invalid_root_before_creating_collection_row() {
    let conn = open_test_db();
    let missing = PathBuf::from(r"D:\does-not-exist");

    let error = run(
        &conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "work".to_owned(),
            path: missing,
            read_only: false,
            writable: false,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    )
    .unwrap_err();

    assert!(error.to_string().contains("collection root"));
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM collections WHERE name = 'work'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn add_refuses_write_quaid_id_when_attach_is_read_only_before_creating_collection_row() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();

    let error = run(
        &conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "work".to_owned(),
            path: root.path().to_path_buf(),
            read_only: true,
            writable: false,
            write_quaid_id: true,
            namespace: None,
        }),
        true,
    )
    .unwrap_err();

    assert!(!error.to_string().is_empty());
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM collections WHERE name = 'work'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[cfg(unix)]
#[test]
fn add_refuses_invalid_ignore_before_creating_collection_row() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    fs::write(root.path().join(".quaidignore"), "[broken\n").unwrap();

    let error = run(
        &conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "work".to_owned(),
            path: root.path().to_path_buf(),
            read_only: false,
            writable: false,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    )
    .unwrap_err();

    assert!(error.to_string().contains("invalid .quaidignore"));
    assert!(error.to_string().contains("Fix .quaidignore and re-run"));
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM collections WHERE name = 'work'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[cfg(unix)]
#[test]
fn add_attaches_collection_and_cleans_short_lived_lease_residue() {
    let (_dir, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    fs::write(
        root.path().join("note.md"),
        "---\ntitle: Note\ntype: note\n---\nhello\n",
    )
    .unwrap();

    run(
        &conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "work".to_owned(),
            path: root.path().to_path_buf(),
            read_only: false,
            writable: false,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    )
    .unwrap();

    let row: (String, i64, Option<String>, i64, i64) = conn
        .query_row(
            "SELECT state, writable, active_lease_session_id,
                    (SELECT COUNT(*) FROM collection_owners),
                    (SELECT COUNT(*) FROM serve_sessions)
             FROM collections WHERE name = 'work'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, "active");
    assert_eq!(row.1, 1);
    assert!(row.2.is_none());
    assert_eq!(row.3, 0);
    assert_eq!(row.4, 0);

    let page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages p JOIN collections c ON c.id = p.collection_id WHERE c.name = 'work'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(page_count, 1);
}

#[cfg(unix)]
#[test]
fn add_preserves_para_type_inference_for_singular_and_plural_folders() {
    let (_dir, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    let cases = [
        ("1. Projects/alpha.md", "project"),
        ("2. Area/health.md", "area"),
        ("3. Resource/rust.md", "resource"),
        ("4. Archive/done.md", "archive"),
    ];
    for (relative_path, _expected_type) in cases {
        let path = root.path().join(relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "---\ntitle: Note\n---\nhello\n").unwrap();
    }

    run(
        &conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "work".to_owned(),
            path: root.path().to_path_buf(),
            read_only: false,
            writable: false,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    )
    .unwrap();

    for (relative_path, expected_type) in cases {
        let slug = relative_path.trim_end_matches(".md");
        let page_type: String = conn
            .query_row("SELECT type FROM pages WHERE slug = ?1", [slug], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(page_type, expected_type, "type for {slug}");
    }
}

#[cfg(unix)]
#[test]
fn add_and_export_preserve_page_with_scalar_related_frontmatter() {
    let (_dir, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    fs::write(
        root.path().join("target.md"),
        "---\ntitle: Target\n---\nhello\n",
    )
    .unwrap();
    fs::write(
        root.path().join("source.md"),
        "---\ntitle: Source\nrelated: target\n---\nhello\n",
    )
    .unwrap();

    run(
        &conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "work".to_owned(),
            path: root.path().to_path_buf(),
            read_only: false,
            writable: false,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    )
    .unwrap();

    let page_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(page_count, 2);

    let export_root = tempfile::TempDir::new().unwrap();
    let exported = export_dir(&conn, export_root.path()).unwrap();
    assert_eq!(exported, 2);
    assert!(export_root.path().join("source.md").exists());
}

#[cfg(unix)]
#[test]
fn add_and_export_preserve_page_with_invalid_optional_graph_frontmatter() {
    let (_dir, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    fs::write(
        root.path().join("source.md"),
        "---\ntitle: Source\nchildren: 42\ntags: [kept]\n---\nhello\n",
    )
    .unwrap();

    run(
        &conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "work".to_owned(),
            path: root.path().to_path_buf(),
            read_only: false,
            writable: false,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    )
    .unwrap();

    let page_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(page_count, 1);
    let tag_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM tags WHERE tag = 'kept'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(tag_count, 1);

    let export_root = tempfile::TempDir::new().unwrap();
    let exported = export_dir(&conn, export_root.path()).unwrap();
    assert_eq!(exported, 1);
    assert!(export_root.path().join("source.md").exists());
}

#[cfg(unix)]
#[test]
fn add_marks_collection_read_only_when_probe_hits_permission_denied() {
    if rustix::process::geteuid().is_root() {
        return;
    }

    let (_dir, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    fs::write(
        root.path().join("note.md"),
        "---\ntitle: Note\ntype: note\n---\nhello\n",
    )
    .unwrap();
    let original_permissions = fs::metadata(root.path()).unwrap().permissions();
    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_mode(0o555);
    fs::set_permissions(root.path(), read_only_permissions).unwrap();

    let result = run(
        &conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "ro-vault".to_owned(),
            path: root.path().to_path_buf(),
            read_only: false,
            writable: false,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    );

    fs::set_permissions(root.path(), original_permissions).unwrap();
    result.unwrap();

    let writable: i64 = conn
        .query_row(
            "SELECT writable FROM collections WHERE name = 'ro-vault'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(writable, 0);
    let residue = fs::read_dir(root.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".quaid-probe-")
        });
    assert!(!residue);
}
