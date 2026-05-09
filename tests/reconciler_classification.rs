#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! reconcile() hard-delete-vs-quarantine classification of missing pages.

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::file_state::{upsert_file_state, FileStat};
use quaid::core::reconciler::*;
use tempfile::TempDir;

#[cfg(unix)]
#[test]
fn reconcile_hard_deletes_missing_pages_without_db_only_state() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let stat = FileStat {
        mtime_ns: 1,
        ctime_ns: Some(1),
        size_bytes: 10,
        inode: Some(1),
    };
    let page_id = seed_file_state(&conn, collection.id, "notes/plain", "notes/plain.md", &stat);

    let stats = reconcile(&conn, &collection).unwrap();
    let page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(stats.hard_deleted, 1);
    assert_eq!(stats.quarantined_db_state, 0);
    assert_eq!(page_count, 0);
    assert!(
        quaid::core::file_state::get_file_state(&conn, collection.id, "notes/plain.md")
            .unwrap()
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn reconcile_quarantines_missing_pages_with_db_only_state() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let stat = FileStat {
        mtime_ns: 1,
        ctime_ns: Some(1),
        size_bytes: 10,
        inode: Some(1),
    };
    let page_id = seed_file_state(
        &conn,
        collection.id,
        "notes/quarantined",
        "notes/quarantined.md",
        &stat,
    );
    let other_page = insert_page(&conn, collection.id, "notes/other");
    insert_programmatic_link(&conn, page_id, other_page);

    let stats = reconcile(&conn, &collection).unwrap();
    let quarantined_at: Option<String> = conn
        .query_row(
            "SELECT quarantined_at FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(stats.hard_deleted, 0);
    assert_eq!(stats.quarantined_db_state, 1);
    assert!(quarantined_at.is_some());
    assert!(
        quaid::core::file_state::get_file_state(&conn, collection.id, "notes/quarantined.md")
            .unwrap()
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn reconcile_quarantines_missing_pages_for_each_db_only_state_branch() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());

    let cases = [
        ("programmatic-link", "missing/link.md"),
        ("manual-assertion", "missing/assertion.md"),
        ("raw-data", "missing/raw.md"),
        ("contradiction", "missing/contradiction.md"),
        ("knowledge-gap", "missing/gap.md"),
    ];

    for (index, (slug_suffix, relative_path)) in cases.iter().enumerate() {
        let slug = format!("notes/{slug_suffix}");
        let page_id = insert_page(&conn, collection.id, &slug);
        let stat = FileStat {
            mtime_ns: index as i64 + 1,
            ctime_ns: Some(index as i64 + 1),
            size_bytes: 10,
            inode: Some(index as i64 + 1),
        };
        upsert_file_state(&conn, collection.id, relative_path, page_id, &stat, "hash").unwrap();

        match *slug_suffix {
            "programmatic-link" => {
                let other_page = insert_page(&conn, collection.id, "notes/other-link");
                conn.execute(
                    "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
                     VALUES (?1, ?2, 'related', 'programmatic')",
                    rusqlite::params![page_id, other_page],
                )
                .unwrap();
            }
            "manual-assertion" => {
                conn.execute(
                    "INSERT INTO assertions (page_id, subject, predicate, object, asserted_by)
                     VALUES (?1, 'A', 'knows', 'B', 'manual')",
                    rusqlite::params![page_id],
                )
                .unwrap();
            }
            "raw-data" => {
                conn.execute(
                    "INSERT INTO raw_data (page_id, source, data) VALUES (?1, 'api', '{}')",
                    rusqlite::params![page_id],
                )
                .unwrap();
            }
            "contradiction" => {
                let other_page = insert_page(&conn, collection.id, "notes/other-contradiction");
                conn.execute(
                    "INSERT INTO contradictions (page_id, other_page_id, type, description)
                     VALUES (?1, ?2, 'assertion_conflict', 'conflict')",
                    rusqlite::params![page_id, other_page],
                )
                .unwrap();
            }
            "knowledge-gap" => {
                conn.execute(
                    "INSERT INTO knowledge_gaps (page_id, query_hash, context)
                     VALUES (?1, ?2, 'context')",
                    rusqlite::params![page_id, format!("gap-hash-{index}")],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
    }

    let stats = reconcile(&conn, &collection).unwrap();
    assert_eq!(stats.quarantined_db_state, cases.len());
    assert_eq!(stats.hard_deleted, 0);

    for (slug_suffix, relative_path) in cases {
        let quarantined_at: Option<String> = conn
            .query_row(
                "SELECT quarantined_at FROM pages WHERE slug = ?1",
                [format!("notes/{slug_suffix}")],
                |row| row.get(0),
            )
            .unwrap();
        assert!(quarantined_at.is_some());
        assert!(
            quaid::core::file_state::get_file_state(&conn, collection.id, relative_path)
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(unix)]
#[test]
fn reconcile_hard_deletes_missing_page_when_gap_is_not_attached_to_page() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let stat = FileStat {
        mtime_ns: 1,
        ctime_ns: Some(1),
        size_bytes: 10,
        inode: Some(1),
    };
    let page_id = seed_file_state(
        &conn,
        collection.id,
        "notes/plain-gap",
        "notes/plain-gap.md",
        &stat,
    );
    conn.execute(
        "INSERT INTO knowledge_gaps (page_id, query_hash, context)
         VALUES (NULL, 'orphan-gap', 'context')",
        [],
    )
    .unwrap();

    let stats = reconcile(&conn, &collection).unwrap();
    let page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(stats.hard_deleted, 1);
    assert_eq!(stats.quarantined_db_state, 0);
    assert_eq!(page_count, 0);
}
