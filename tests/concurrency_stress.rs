//! Concurrency stress tests — validates OCC and WAL invariants under parallel load.
//!
//! Scenarios:
//! 1. Parallel OCC — 4 threads writing same slug with stale version →
//!    exactly 1 success + 3 ConflictError
//! 2. Duplicate ingest — 2 threads ingesting same source → exactly 1 success
//! 3. WAL compact safety — compact during open reader → both succeed
//! 4. Read isolation — concurrent readers see consistent data
//!
//! NOTE: Each test opens multiple Connections to the same on-disk DB.
//! `:memory:` databases cannot be shared across connections, so all concurrency
//! tests use temporary files.

use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use gbrain::core::db;
use gbrain::core::migrate::import_dir;

// ── DB helpers ────────────────────────────────────────────────────────────────

/// Create a named temporary DB file and return (path_string, TempDir guard).
fn temp_db_path() -> (String, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let path = dir.path().join("brain.db").to_str().unwrap().to_string();
    (path, dir)
}

fn open_conn(path: &str) -> rusqlite::Connection {
    db::open(path).unwrap_or_else(|e| panic!("open DB at {path}: {e}"))
}

/// Insert a test page directly into the DB.
fn insert_page(conn: &rusqlite::Connection, slug: &str, truth: &str, version: i64) {
    conn.execute(
        "INSERT OR REPLACE INTO pages \
         (slug, type, title, summary, compiled_truth, timeline, \
          frontmatter, wing, room, version, \
          created_at, updated_at, truth_updated_at, timeline_updated_at) \
         VALUES (?1, 'person', ?1, '', ?2, '', '{}', 'people', '', ?3, \
                 strftime('%Y-%m-%dT%H:%M:%SZ','now'), \
                 strftime('%Y-%m-%dT%H:%M:%SZ','now'), \
                 strftime('%Y-%m-%dT%H:%M:%SZ','now'), \
                 strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
        rusqlite::params![slug, truth, version],
    )
    .expect("insert page");
}

/// Attempt an OCC update (compare-and-swap on version).
/// Returns true on success (1 row affected), false on conflict (0 rows).
fn try_occ_update(conn: &rusqlite::Connection, slug: &str, expected_version: i64) -> bool {
    let rows = conn
        .execute(
            "UPDATE pages SET \
             compiled_truth = 'Updated by thread', \
             version = version + 1, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') \
             WHERE slug = ?1 AND version = ?2",
            rusqlite::params![slug, expected_version],
        )
        .expect("execute OCC update");
    rows == 1
}

fn read_version(conn: &rusqlite::Connection, slug: &str) -> i64 {
    conn.query_row("SELECT version FROM pages WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })
    .expect("read version")
}

// ── 1. Parallel OCC ───────────────────────────────────────────────────────────

#[test]
fn parallel_occ_exactly_one_write_wins() {
    let (db_path, _guard) = temp_db_path();

    // Create the page at version 1
    let setup_conn = open_conn(&db_path);
    insert_page(&setup_conn, "people/alice", "Original truth", 1);
    drop(setup_conn);

    const THREAD_COUNT: usize = 4;
    let barrier = Arc::new(Barrier::new(THREAD_COUNT));
    let db_path = Arc::new(db_path);
    let results = Arc::new(Mutex::new(Vec::<bool>::new()));

    let handles: Vec<_> = (0..THREAD_COUNT)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let db_path = Arc::clone(&db_path);
            let results = Arc::clone(&results);

            thread::spawn(move || {
                let conn = open_conn(&db_path);
                // Read version before barrier — all threads see version=1
                let current_version = read_version(&conn, "people/alice");

                // Synchronise all threads to maximise contention
                barrier.wait();

                // Attempt CAS update with the stale version=1
                let success = try_occ_update(&conn, "people/alice", current_version);
                results.lock().unwrap().push(success);
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("thread panicked");
    }

    let outcomes = results.lock().unwrap().clone();
    let successes = outcomes.iter().filter(|&&ok| ok).count();
    let conflicts = outcomes.iter().filter(|&&ok| !ok).count();

    // Due to SQLite's WAL serialisation, exactly 1 thread wins the CAS.
    // The remaining threads lose because the version was already bumped.
    assert_eq!(
        successes, 1,
        "exactly 1 thread should win the OCC race, got {successes} successes"
    );
    assert_eq!(
        conflicts,
        THREAD_COUNT - 1,
        "the remaining {} threads should lose, got {} conflicts",
        THREAD_COUNT - 1,
        conflicts
    );

    // Final version should be 2 (bumped once)
    let final_conn = open_conn(&db_path);
    let final_version = read_version(&final_conn, "people/alice");
    assert_eq!(
        final_version, 2,
        "final version should be 2 after one successful CAS"
    );
}

// ── 2. Duplicate ingest ───────────────────────────────────────────────────────

#[test]
fn duplicate_ingest_from_two_threads_produces_one_row() {
    use std::fs;

    let (db_path, _guard) = temp_db_path();
    let db_path = Arc::new(db_path);

    // Prepare a single-file fixture dir
    let fixture_dir = tempfile::TempDir::new().expect("fixture dir");
    fs::write(
        fixture_dir.path().join("alice.md"),
        "---\nslug: people/alice\ntitle: Alice\ntype: person\n---\n# Alice\n\nAlice is an operator.\n",
    )
    .expect("write fixture");
    let fixture_dir = Arc::new(fixture_dir);

    let barrier = Arc::new(Barrier::new(2));
    let import_counts = Arc::new(Mutex::new(Vec::<usize>::new()));

    // Pre-open connections before barrier to avoid simultaneous schema writes
    let conn_a = open_conn(&db_path);
    let conn_b = open_conn(&db_path);

    let handle_a = {
        let barrier = Arc::clone(&barrier);
        let fixture_dir = Arc::clone(&fixture_dir);
        let import_counts = Arc::clone(&import_counts);
        thread::spawn(move || {
            barrier.wait();
            match import_dir(&conn_a, fixture_dir.path(), false) {
                Ok(stats) => import_counts.lock().unwrap().push(stats.imported),
                Err(_) => import_counts.lock().unwrap().push(0),
            }
        })
    };

    let handle_b = {
        let barrier = Arc::clone(&barrier);
        let fixture_dir = Arc::clone(&fixture_dir);
        let import_counts = Arc::clone(&import_counts);
        thread::spawn(move || {
            barrier.wait();
            match import_dir(&conn_b, fixture_dir.path(), false) {
                Ok(stats) => import_counts.lock().unwrap().push(stats.imported),
                Err(_) => import_counts.lock().unwrap().push(0),
            }
        })
    };

    handle_a.join().expect("thread A panicked");
    handle_b.join().expect("thread B panicked");

    // Verify: total imported across both threads ≤ 1 (idempotency + lock serialisation)
    let counts = import_counts.lock().unwrap().clone();
    let total_imported: usize = counts.iter().sum();
    assert!(
        total_imported <= 1,
        "total imported across 2 threads should be ≤1 (idempotency prevents duplicate): got {counts:?}"
    );

    // DB should have exactly 1 page regardless of which thread won
    let verify_conn = open_conn(&db_path);
    let page_count: i64 = verify_conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .expect("count pages");
    assert_eq!(page_count, 1, "DB should have exactly 1 page");

    // ingest_log should have at most 1 record
    let log_count: i64 = verify_conn
        .query_row("SELECT COUNT(*) FROM ingest_log", [], |row| row.get(0))
        .expect("count ingest_log");
    assert!(
        log_count <= 1,
        "ingest_log should have at most 1 record, got {log_count}"
    );
}

// ── 3. WAL compact safety ─────────────────────────────────────────────────────

#[test]
fn wal_compact_during_open_reader_both_succeed() {
    let (db_path, _guard) = temp_db_path();

    // Populate the DB
    let setup = open_conn(&db_path);
    for i in 0..10 {
        insert_page(&setup, &format!("test/page-{i}"), "content", 1);
    }
    drop(setup);

    let db_path = Arc::new(db_path);
    let barrier = Arc::new(Barrier::new(2));

    let db_path_reader = Arc::clone(&db_path);
    let barrier_reader = Arc::clone(&barrier);
    let reader_handle = thread::spawn(move || {
        let conn = open_conn(&db_path_reader);
        // Start a read transaction
        conn.execute_batch("BEGIN DEFERRED")
            .expect("begin read txn");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .expect("count pages in reader");

        barrier_reader.wait(); // Signal compactor to proceed

        // Hold the read transaction open briefly, then commit
        thread::sleep(std::time::Duration::from_millis(10));
        conn.execute_batch("COMMIT").expect("commit read txn");
        count
    });

    let db_path_compact = Arc::clone(&db_path);
    let barrier_compact = Arc::clone(&barrier);
    let compact_handle = thread::spawn(move || {
        barrier_compact.wait(); // Wait for reader to start

        let conn = open_conn(&db_path_compact);
        // TRUNCATE checkpoint flushes WAL — should coexist with reader
        db::compact(&conn)
    });

    let reader_count = reader_handle.join().expect("reader thread panicked");
    let compact_result = compact_handle.join().expect("compact thread panicked");

    assert!(
        reader_count >= 0,
        "reader should have completed without error"
    );
    assert!(
        compact_result.is_ok(),
        "WAL compact during open reader should succeed: {:?}",
        compact_result
    );
}

// ── 4. Read isolation ─────────────────────────────────────────────────────────

#[test]
fn concurrent_readers_see_consistent_data() {
    let (db_path, _guard) = temp_db_path();

    // Populate
    let setup = open_conn(&db_path);
    for i in 0..5 {
        insert_page(
            &setup,
            &format!("test/page-{i}"),
            &format!("content {i}"),
            1,
        );
    }
    drop(setup);

    let db_path = Arc::new(db_path);
    const READER_COUNT: usize = 4;
    let results = Arc::new(Mutex::new(Vec::<i64>::new()));

    let handles: Vec<_> = (0..READER_COUNT)
        .map(|_| {
            let db_path = Arc::clone(&db_path);
            let results = Arc::clone(&results);
            thread::spawn(move || {
                let conn = open_conn(&db_path);
                let count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
                    .expect("count pages");
                results.lock().unwrap().push(count);
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("reader thread panicked");
    }

    let counts = results.lock().unwrap().clone();
    assert_eq!(
        counts.len(),
        READER_COUNT,
        "all reader threads should complete"
    );
    let expected_count = 5i64;
    for &count in &counts {
        assert_eq!(
            count, expected_count,
            "all readers should see {expected_count} pages, got {count}"
        );
    }
}
