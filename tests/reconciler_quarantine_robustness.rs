#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Quarantine-instead-of-destroy reconciler behaviour: ambiguous hash
//! renames quarantine the original page (surviving across passes, with
//! truthful stats), duplicate frontmatter uuids quarantine individual files
//! instead of halting the collection, and the `file_state.frontmatter_uuid`
//! cache populates lazily and resets on content change.

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::file_state::{self, upsert_file_state, FileStat};
use quaid::core::reconciler::*;
use rusqlite::Connection;
use std::fs;
use tempfile::TempDir;

#[cfg(unix)]
fn page_quarantined_at(conn: &Connection, page_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT quarantined_at FROM pages WHERE id = ?1",
        [page_id],
        |row| row.get(0),
    )
    .unwrap()
}

#[cfg(unix)]
fn quarantined_page_count(conn: &Connection, collection_id: i64) -> usize {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages
             WHERE collection_id = ?1 AND quarantined_at IS NOT NULL",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap();
    usize::try_from(count).unwrap()
}

#[cfg(unix)]
fn cached_frontmatter_uuid(
    conn: &Connection,
    collection_id: i64,
    relative_path: &str,
) -> Option<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT frontmatter_uuid FROM file_state
         WHERE collection_id = ?1 AND relative_path = ?2",
        rusqlite::params![collection_id, relative_path],
        |row| row.get(0),
    )
    .optional()
    .unwrap()
}

/// An ambiguous hash rename (one missing page, two identical new candidate
/// files) must quarantine the original page — preserving its row, links,
/// and history — instead of dropping the path so the next pass deletes it.
/// The reported `quarantined_ambiguous` count must match what was actually
/// quarantined on both passes (the input `DriftCaptureSummary` builds from).
#[cfg(unix)]
#[test]
fn ambiguous_hash_rename_quarantines_original_page_across_two_reconciles() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());

    // No frontmatter: candidate slugs derive from their paths, so neither
    // candidate slug-matches (and thereby reclaims) the original page —
    // exercising the true quarantine path.
    let content =
        "This body is intentionally longer than sixty-four bytes so conservative hash rename inference still considers it.\n";
    fs::write(root.path().join("candidate-a.md"), content).unwrap();
    fs::write(root.path().join("candidate-b.md"), content).unwrap();
    let sha256 = file_state::hash_file(&root.path().join("candidate-a.md")).unwrap();

    // Seed the missing page (uuid-less so only hash inference applies) bound
    // to a path that no longer exists on disk.
    conn.execute(
        "INSERT INTO pages (collection_id, slug, type, title, compiled_truth, timeline)
         VALUES (?1, 'notes/original', 'concept', 'Original', ?2, '')",
        rusqlite::params![collection.id, content.trim()],
    )
    .unwrap();
    let page_id = conn.last_insert_rowid();
    let stale_stat = FileStat {
        mtime_ns: 1,
        ctime_ns: Some(1),
        size_bytes: content.len() as i64,
        inode: Some(99),
    };
    upsert_file_state(
        &conn,
        collection.id,
        "old-name.md",
        page_id,
        &stale_stat,
        &sha256,
    )
    .unwrap();

    let first = reconcile(&conn, &collection).unwrap();

    assert_eq!(first.quarantined_ambiguous, 1);
    assert_eq!(first.hard_deleted, 0);
    assert_eq!(first.new, 2, "both candidates reingest as new pages");
    assert!(
        page_quarantined_at(&conn, page_id).is_some(),
        "original page must be quarantined, not deleted"
    );
    assert!(
        file_state::get_file_state(&conn, collection.id, "old-name.md")
            .unwrap()
            .is_none(),
        "stale file_state binding must be cleared"
    );
    // Truthful reporting: the stat the drift guard consumes equals the
    // number of pages actually quarantined.
    assert_eq!(
        first.quarantined_ambiguous + first.quarantined_db_state,
        quarantined_page_count(&conn, collection.id)
    );

    // Second pass: nothing left to delete or quarantine — the page survives.
    let second = reconcile(&conn, &collection).unwrap();

    assert_eq!(second.hard_deleted, 0);
    assert_eq!(second.quarantined_ambiguous, 0);
    assert_eq!(second.quarantined_db_state, 0);
    assert!(
        page_quarantined_at(&conn, page_id).is_some(),
        "original page must survive quarantined across passes"
    );
    let page_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(page_exists, 1);
}

/// The git merge-conflict shape: a tracked original plus an untracked copy
/// sharing its frontmatter uuid. The copy (untracked, newer) loses and is
/// excluded; the original page keeps syncing untouched and the collection
/// is never halted.
#[cfg(unix)]
#[test]
fn duplicate_uuid_conflict_copy_is_skipped_and_tracked_original_survives() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let uuid = "01969f11-9448-7d79-8d3f-c68f54761111";
    let note = format!(
        "---\nmemory_id: {uuid}\nslug: notes/original\ntitle: Original\ntype: concept\n---\nA body long enough to be a perfectly ordinary note for this test.\n"
    );
    fs::write(root.path().join("original.md"), &note).unwrap();
    reconcile(&conn, &collection).unwrap();
    let page_id: i64 = conn
        .query_row(
            "SELECT id FROM pages WHERE collection_id = ?1 AND slug = 'notes/original'",
            [collection.id],
            |row| row.get(0),
        )
        .unwrap();

    // Merge-conflict copy: identical bytes (same uuid), newer mtime.
    fs::write(root.path().join("original (copy).md"), &note).unwrap();
    filetime::set_file_mtime(
        root.path().join("original (copy).md"),
        filetime::FileTime::from_unix_time(4_000_000_000, 0),
    )
    .unwrap();

    let stats = reconcile(&conn, &collection).unwrap();

    assert_eq!(
        stats.quarantined_ambiguous, 0,
        "untracked loser touches no page"
    );
    assert_eq!(stats.hard_deleted, 0);
    assert!(page_quarantined_at(&conn, page_id).is_none());
    let page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = ?1",
            [collection.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(page_count, 1, "the conflict copy must not ingest");
    assert!(root.path().join("original (copy).md").exists());

    // Stable on subsequent passes.
    let again = reconcile(&conn, &collection).unwrap();
    assert_eq!(again.hard_deleted, 0);
    assert_eq!(again.quarantined_ambiguous, 0);
}

/// When both duplicate-uuid files are tracked, the newest-mtime file loses:
/// its page is quarantined (not deleted) and the file stays on disk, while
/// the older file keeps its page and the collection keeps reconciling.
#[cfg(unix)]
#[test]
fn duplicate_uuid_between_tracked_files_quarantines_newest_loser_page() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let uuid_a = "01969f11-9448-7d79-8d3f-c68f54762222";
    let uuid_b = "01969f11-9448-7d79-8d3f-c68f54763333";
    fs::write(
        root.path().join("a.md"),
        format!("---\nmemory_id: {uuid_a}\nslug: notes/a\ntitle: A\ntype: concept\n---\nBody A long enough to be an ordinary note in this scenario.\n"),
    )
    .unwrap();
    fs::write(
        root.path().join("b.md"),
        format!("---\nmemory_id: {uuid_b}\nslug: notes/b\ntitle: B\ntype: concept\n---\nBody B long enough to be an ordinary note in this scenario.\n"),
    )
    .unwrap();
    reconcile(&conn, &collection).unwrap();
    let page_b: i64 = conn
        .query_row(
            "SELECT id FROM pages WHERE collection_id = ?1 AND slug = 'notes/b'",
            [collection.id],
            |row| row.get(0),
        )
        .unwrap();

    // b.md is hand-edited to claim a.md's uuid (e.g. a careless copy-paste),
    // making it the newer of the two duplicates.
    fs::write(
        root.path().join("b.md"),
        format!("---\nmemory_id: {uuid_a}\nslug: notes/b\ntitle: B\ntype: concept\n---\nBody B long enough to be an ordinary note in this scenario.\n"),
    )
    .unwrap();
    filetime::set_file_mtime(
        root.path().join("a.md"),
        filetime::FileTime::from_unix_time(1_000_000, 0),
    )
    .unwrap();
    filetime::set_file_mtime(
        root.path().join("b.md"),
        filetime::FileTime::from_unix_time(2_000_000, 0),
    )
    .unwrap();

    let stats = reconcile(&conn, &collection).unwrap();

    assert_eq!(stats.quarantined_ambiguous, 1, "loser page is quarantined");
    assert_eq!(stats.hard_deleted, 0);
    assert!(
        page_quarantined_at(&conn, page_b).is_some(),
        "the tracked loser's page must survive quarantined"
    );
    assert!(
        file_state::get_file_state(&conn, collection.id, "b.md")
            .unwrap()
            .is_none(),
        "loser binding must be cleared"
    );
    assert!(root.path().join("b.md").exists());

    // Next pass is stable: the loser stays excluded, nothing is deleted.
    let again = reconcile(&conn, &collection).unwrap();
    assert_eq!(again.hard_deleted, 0);
    assert!(page_quarantined_at(&conn, page_b).is_some());
}

/// The duplicate-uuid scan caches each file's frontmatter uuid in
/// `file_state.frontmatter_uuid` ('' = no uuid) so unchanged files are not
/// re-read every pass, and any content upsert resets the cache to NULL so
/// it can never go stale.
#[cfg(unix)]
#[test]
fn frontmatter_uuid_cache_populates_lazily_and_resets_on_content_change() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let uuid = "01969f11-9448-7d79-8d3f-c68f54764444";
    fs::write(
        root.path().join("with-uuid.md"),
        format!("---\nmemory_id: {uuid}\nslug: notes/with-uuid\ntitle: U\ntype: concept\n---\nBody one.\n"),
    )
    .unwrap();
    fs::write(
        root.path().join("no-uuid.md"),
        "Plain body, no frontmatter uuid.\n",
    )
    .unwrap();

    // Pass 1 ingests; the upsert leaves the cache NULL (not yet cached).
    reconcile(&conn, &collection).unwrap();
    assert_eq!(
        cached_frontmatter_uuid(&conn, collection.id, "with-uuid.md"),
        Some(None)
    );

    // Pass 2 re-reads once and backfills the cache.
    reconcile(&conn, &collection).unwrap();
    assert_eq!(
        cached_frontmatter_uuid(&conn, collection.id, "with-uuid.md"),
        Some(Some(uuid.to_owned()))
    );
    assert_eq!(
        cached_frontmatter_uuid(&conn, collection.id, "no-uuid.md"),
        Some(Some(String::new())),
        "uuid-less files cache the empty-string sentinel"
    );

    // Content change: the reingest upsert must reset the cache to NULL …
    fs::write(
        root.path().join("with-uuid.md"),
        format!("---\nmemory_id: {uuid}\nslug: notes/with-uuid\ntitle: U\ntype: concept\n---\nBody two, edited.\n"),
    )
    .unwrap();
    reconcile(&conn, &collection).unwrap();
    assert_eq!(
        cached_frontmatter_uuid(&conn, collection.id, "with-uuid.md"),
        Some(None)
    );

    // … and the next pass lazily re-caches the fresh value.
    reconcile(&conn, &collection).unwrap();
    assert_eq!(
        cached_frontmatter_uuid(&conn, collection.id, "with-uuid.md"),
        Some(Some(uuid.to_owned()))
    );
}
