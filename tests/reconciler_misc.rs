#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Remaining public-API tests: bulk commit chunking, duplicate memory_id halt, stat-diff walker.

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::file_state::FileStat;
use quaid::core::reconciler::*;
use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

#[cfg(unix)]
#[test]
fn reconcile_commits_in_500_file_chunks() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());

    for index in 0..500 {
        fs::write(
            root.path().join(format!("note-{index:03}.md")),
            format!(
                "---\nslug: notes/{index:03}\ntitle: Note {index}\ntype: concept\n---\nBody {index} with enough text to stay well formed.\n"
            ),
        )
        .unwrap();
    }
    fs::write(
        root.path().join("note-500.md"),
        "---\nmemory_id: not-a-uuid\nslug: notes/500\ntitle: Broken\ntype: concept\n---\nBroken body.\n",
    )
    .unwrap();

    let error = reconcile(&conn, &collection).unwrap_err().to_string();
    let committed_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = ?1 AND slug LIKE 'notes/%'",
            [collection.id],
            |row| row.get(0),
        )
        .unwrap();

    assert!(error.contains("invalid frontmatter uuid"));
    assert_eq!(committed_count, 0);
}

#[cfg(unix)]
#[test]
fn reconcile_halts_when_two_files_share_the_same_memory_id() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let uuid = "01969f11-9448-7d79-8d3f-c68f54769999";
    let note_a = format!(
        "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nThis body is long enough to avoid the trivial-content path.\n"
    );
    let note_b = format!(
        "---\nmemory_id: {uuid}\nslug: notes/b\ntitle: B\ntype: concept\n---\nThis other body is also long enough to avoid the trivial-content path.\n"
    );
    fs::write(root.path().join("a.md"), note_a).unwrap();
    fs::write(root.path().join("b.md"), note_b).unwrap();

    let error = reconcile(&conn, &collection).unwrap_err().to_string();
    let page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = ?1",
            [collection.id],
            |row| row.get(0),
        )
        .unwrap();

    assert!(error.contains("DuplicateUuidError"));
    assert!(error.contains("a.md"));
    assert!(error.contains("b.md"));
    assert_eq!(
        page_count, 0,
        "duplicate uuid halt must abort before mutation"
    );
}

#[test]
#[cfg(unix)]
fn stat_diff_walk_classifies_new_modified_unchanged_and_missing_files() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("notes")).unwrap();
    fs::write(root.path().join("notes").join("same.md"), "# same").unwrap();
    fs::write(
        root.path().join("notes").join("changed.md"),
        "# changed on disk",
    )
    .unwrap();
    fs::write(root.path().join("notes").join("new.md"), "# new").unwrap();
    fs::write(root.path().join(".quaidignore"), "ignored/**\n").unwrap();
    fs::create_dir_all(root.path().join("ignored")).unwrap();
    fs::write(root.path().join("ignored").join("skip.md"), "# skip").unwrap();

    let collection = insert_collection(&conn, root.path());
    let same_stat = stat_for(root.path(), "notes/same.md");
    seed_file_state(
        &conn,
        collection.id,
        "notes/same",
        "notes/same.md",
        &same_stat,
    );

    let changed_stat = stat_for(root.path(), "notes/changed.md");
    seed_file_state(
        &conn,
        collection.id,
        "notes/changed",
        "notes/changed.md",
        &unique_old_stat(&changed_stat),
    );

    let missing_stat = FileStat {
        mtime_ns: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(1))
            .as_nanos() as i64,
        ctime_ns: Some(1),
        size_bytes: 12,
        inode: Some(1),
    };
    seed_file_state(
        &conn,
        collection.id,
        "notes/missing",
        "notes/missing.md",
        &missing_stat,
    );

    let diff = stat_diff(&conn, collection.id, root.path()).unwrap();

    assert!(diff.unchanged.contains(Path::new("notes/same.md")));
    assert!(diff.modified.contains_key(Path::new("notes/changed.md")));
    assert!(diff.new.contains_key(Path::new("notes/new.md")));
    assert!(diff.missing.contains(Path::new("notes/missing.md")));
    assert!(!diff.new.contains_key(Path::new("ignored/skip.md")));
}
