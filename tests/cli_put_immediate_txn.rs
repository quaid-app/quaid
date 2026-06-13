#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests covering the `BEGIN IMMEDIATE` write-transaction path in
//! `quaid::commands::put`.
//!
//! The put write path now opens its transaction with `BEGIN IMMEDIATE` (via
//! `quaid::core::db::begin_immediate`) so the reserved write lock is acquired
//! at transaction start and the connection's 5s `busy_timeout` can retry
//! cross-process/cross-connection contention, rather than surfacing a transient
//! `SQLITE_BUSY` from inside an already-open deferred transaction.
//!
//! These tests assert (1) ordinary create/update still succeed through the
//! IMMEDIATE path, and (2) concurrent same-process writers contending on one
//! slug with `expected_version` still yield exactly-one-winner optimistic-
//! concurrency semantics (one success, the rest `ConflictError`) with no
//! `SQLITE_BUSY` failures leaking out. A heavier cross-process contention test
//! lives alongside the daemon serve harness.

#[path = "common/put_fixtures.rs"]
mod fixtures;

use std::sync::{Arc, Barrier};

use fixtures::{open_test_db, open_test_db_with_path, read_page, reopen_test_db};
use quaid::commands::put::put_from_string;

#[test]
fn create_through_immediate_path_succeeds() {
    let conn = open_test_db();
    let md = "---\ntitle: Alice\ntype: person\n---\nFirst.\n";
    put_from_string(&conn, "people/alice", md, None).unwrap();

    let (version, _, title, truth, _) = read_page(&conn, "people/alice").unwrap();
    assert_eq!(version, 1);
    assert_eq!(title, "Alice");
    assert!(truth.contains("First"));
}

#[test]
fn update_through_immediate_path_succeeds() {
    let conn = open_test_db();
    let md1 = "---\ntitle: Alice\ntype: person\n---\nFirst.\n";
    put_from_string(&conn, "people/alice", md1, None).unwrap();

    let md2 = "---\ntitle: Alice\ntype: person\n---\nSecond.\n";
    put_from_string(&conn, "people/alice", md2, Some(1)).unwrap();

    let (version, _, _, truth, _) = read_page(&conn, "people/alice").unwrap();
    assert_eq!(version, 2);
    assert!(truth.contains("Second"));
}

/// Several connections to the same on-disk database race to update one slug,
/// each supplying the same `expected_version`. The IMMEDIATE-begin write
/// transaction plus the 5s busy_timeout must serialize them so exactly one
/// writer wins (version bumps to 2) and every other writer observes a
/// `ConflictError` — never a `SQLITE_BUSY` error escaping the write path.
#[test]
fn concurrent_same_process_writers_with_expected_version_yield_exactly_one_winner() {
    const WRITERS: usize = 6;

    let (setup_conn, db_path) = open_test_db_with_path();
    let seed = "---\ntitle: Alice\ntype: person\n---\nSeed.\n";
    put_from_string(&setup_conn, "people/alice", seed, None).unwrap();
    let (seed_version, _, _, _, _) = read_page(&setup_conn, "people/alice").unwrap();
    assert_eq!(seed_version, 1);
    drop(setup_conn);

    let barrier = Arc::new(Barrier::new(WRITERS));
    let handles: Vec<_> = (0..WRITERS)
        .map(|i| {
            let barrier = Arc::clone(&barrier);
            let db_path = db_path.clone();
            std::thread::spawn(move || {
                let conn = reopen_test_db(&db_path);
                let md = format!("---\ntitle: Alice\ntype: person\n---\nWriter {i}.\n");
                // Release all writers simultaneously to maximize contention.
                barrier.wait();
                put_from_string(&conn, "people/alice", &md, Some(1))
            })
        })
        .collect();

    let mut winners = 0;
    let mut conflicts = 0;
    for handle in handles {
        match handle.join().unwrap() {
            Ok(()) => winners += 1,
            Err(err) => {
                let msg = err.to_string();
                assert!(
                    !msg.to_lowercase().contains("busy"),
                    "writer surfaced a SQLITE_BUSY error from the write path: {msg}"
                );
                assert!(
                    msg.contains("Conflict"),
                    "losing writer should report a ConflictError, got: {msg}"
                );
                conflicts += 1;
            }
        }
    }

    assert_eq!(
        winners, 1,
        "exactly one writer must win the compare-and-swap"
    );
    assert_eq!(conflicts, WRITERS - 1, "all other writers must conflict");

    // The winning compare-and-swap bumped the version exactly once.
    let verify = reopen_test_db(&db_path);
    let (version, _, _, _, _) = read_page(&verify, "people/alice").unwrap();
    assert_eq!(version, 2);
}
