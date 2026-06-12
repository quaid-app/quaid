#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for SQLite connection & transaction hygiene:
//!
//! - `db::open_runtime` — the runtime connection factory used by background
//!   workers, watcher callbacks, supervisor ticks, and IPC handlers. Must
//!   apply the standard 5s busy timeout (so writes wait out contention
//!   instead of failing instantly with `SQLITE_BUSY`) and must refuse
//!   uninitialized databases without creating files.
//! - `db::with_immediate_transaction` — the shared `BEGIN IMMEDIATE` helper.
//!   A failed COMMIT must roll back so the shared connection is not wedged
//!   ("cannot start a transaction within a transaction"), both through the
//!   helper directly and through the extractor-worker supersede path that
//!   previously had its own rollback-free copy.
//! - `db::open` — must perform no filesystem side effects: the default
//!   collection row is seeded with an empty `root_path` placeholder and the
//!   on-disk `~/.quaid/vault` root is only provisioned by `quaid init` or
//!   write-target resolution.
//! - crash-partial fresh-bootstrap recovery — must refuse to reclaim a
//!   database with rows in `extraction_queue`, `correction_sessions`, or
//!   `namespaces`.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

use quaid::core::conversation::queue;
use quaid::core::conversation::supersede::{resolve_and_write_fact_in_context, FactWriteContext};
use quaid::core::db;
use quaid::core::types::{DbError, ExtractionTriggerKind, RawFact};
use tempfile::TempDir;

// ── open_runtime ──────────────────────────────────────────────

#[test]
fn open_runtime_write_waits_for_competing_immediate_transaction() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let path = db_path.to_str().unwrap().to_owned();

    let holder = db::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE TRANSACTION").unwrap();
    holder
        .execute("INSERT INTO namespaces (id) VALUES ('holder')", [])
        .unwrap();

    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let writer_path = path;
    let contender = std::thread::spawn(move || {
        let conn = db::open_runtime(&writer_path).expect("open_runtime on initialized db");
        started_tx.send(()).unwrap();
        let start = Instant::now();
        let result = conn.execute("INSERT INTO namespaces (id) VALUES ('contender')", []);
        (result, start.elapsed())
    });

    // Keep the write lock held well past the contender's first attempt, then
    // release it. With the default busy_timeout of 0 the contender would fail
    // instantly; with open_runtime's 5s budget it must wait and succeed.
    started_rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(750));
    holder.execute_batch("COMMIT TRANSACTION").unwrap();

    let (result, waited) = contender.join().unwrap();
    result.expect("open_runtime write must wait out the held lock and succeed");
    assert!(
        waited >= Duration::from_millis(250),
        "write should have been blocked by the held IMMEDIATE transaction, waited {waited:?}"
    );
    assert!(
        waited < Duration::from_secs(5),
        "write must succeed within the 5s busy budget, waited {waited:?}"
    );

    let count: i64 = holder
        .query_row("SELECT COUNT(*) FROM namespaces", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2, "both writers' rows must be present");
}

#[test]
fn open_runtime_rejects_missing_database_file() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing.db");

    let err = db::open_runtime(&missing).expect_err("missing file must be rejected");
    assert!(
        matches!(err, DbError::PathNotFound { .. }),
        "expected DbError::PathNotFound, got: {err:?}"
    );
    assert!(
        !missing.exists(),
        "open_runtime must not create the database file"
    );
}

#[test]
fn open_runtime_rejects_uninitialized_database() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("foreign.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE t (x INTEGER);").unwrap();
    }

    let err = db::open_runtime(&db_path).expect_err("uninitialized db must be rejected");
    assert!(
        matches!(err, DbError::Schema { .. }),
        "expected DbError::Schema, got: {err:?}"
    );
}

// ── with_immediate_transaction ────────────────────────────────

#[test]
fn shared_immediate_transaction_helper_recovers_after_aborted_commit() {
    // Force the next COMMIT to fail by registering a commit_hook that
    // returns true (aborts the commit, surfacing as a SQLite error).
    let conn = db::open(":memory:").unwrap();
    conn.commit_hook(Some(|| true));

    let aborted = db::with_immediate_transaction(&conn, |conn| {
        conn.execute("INSERT INTO namespaces (id) VALUES ('wedge')", [])?;
        Ok::<_, rusqlite::Error>(())
    });
    assert!(
        aborted.is_err(),
        "commit_hook abort must surface as an error"
    );

    // Clear the hook so the recovery transaction can commit normally. If the
    // helper failed to roll back after the aborted commit, the next
    // BEGIN IMMEDIATE would fail with "cannot start a transaction within a
    // transaction".
    conn.commit_hook::<fn() -> bool>(None);

    db::with_immediate_transaction(&conn, |conn| {
        conn.execute("INSERT INTO namespaces (id) VALUES ('recovered')", [])?;
        Ok::<_, rusqlite::Error>(())
    })
    .expect("follow-up transaction must succeed; connection must not be wedged");

    let rows: Vec<String> = conn
        .prepare("SELECT id FROM namespaces ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec!["recovered".to_owned()],
        "aborted commit must roll back its insert; recovery insert must land"
    );
}

#[test]
fn fact_write_transaction_recovers_after_aborted_commit() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active'
         WHERE id = 1",
        [dir.path().display().to_string()],
    )
    .unwrap();

    let context = FactWriteContext {
        collection_id: 1,
        root_path: dir.path().to_path_buf(),
        namespace: String::new(),
        session_id: "session-1".to_owned(),
        source_turns: vec!["1".to_owned()],
        extracted_at: "2026-06-12T09:00:00Z".to_owned(),
        extracted_by: "test-extractor".to_owned(),
    };
    let fact = RawFact::Preference {
        about: "programming-language".to_owned(),
        strength: None,
        summary: "Prefers Rust".to_owned(),
    };

    // Abort the COMMIT inside resolve_and_write_fact_in_context's IMMEDIATE
    // transaction. The supersede module's former private helper propagated
    // the commit error without rolling back, wedging the shared extraction
    // worker connection until restart.
    conn.commit_hook(Some(|| true));
    let aborted = resolve_and_write_fact_in_context(&fact, &conn, &context);
    assert!(
        aborted.is_err(),
        "aborted commit must surface as an error, got: {aborted:?}"
    );

    conn.commit_hook::<fn() -> bool>(None);

    // The next operations on the same connection must succeed — no
    // "cannot start a transaction within a transaction".
    let written = resolve_and_write_fact_in_context(&fact, &conn, &context)
        .expect("fact write must succeed after the aborted commit");
    assert!(
        written.slug.is_some(),
        "recovered fact write must allocate a slug"
    );

    // The extraction queue shares the worker connection in production; its
    // own IMMEDIATE transaction must also go through.
    queue::enqueue(
        &conn,
        "session-1",
        "conversations/2026-06-12/session-1.md",
        ExtractionTriggerKind::Debounce,
        "2026-06-12T09:00:00Z",
    )
    .expect("enqueue must succeed on the shared connection after recovery");
}

// ── open: no filesystem side effects ──────────────────────────

#[test]
fn open_seeds_default_collection_with_empty_root_placeholder() {
    let conn = db::open(":memory:").unwrap();

    let (root_path, writable, is_write_target): (String, i64, i64) = conn
        .query_row(
            "SELECT root_path, writable, is_write_target
             FROM collections
             WHERE id = 1 AND name = 'default'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();

    assert_eq!(
        root_path, "",
        "open must seed the default collection with an empty root placeholder \
         instead of provisioning ~/.quaid/vault"
    );
    assert_eq!(writable, 1);
    assert_eq!(is_write_target, 1);
}

#[test]
fn in_memory_open_succeeds_when_home_is_not_writable() {
    // HOME points at a *file*, so any attempt to create `$HOME/.quaid/...`
    // fails even when the test runs as root (where read-only directory
    // permissions would not bind). Opening `:memory:` must not touch HOME.
    let dir = TempDir::new().unwrap();
    let home_file = dir.path().join("not-a-directory");
    fs::write(&home_file, "not a directory").unwrap();

    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    let output = command
        .env("HOME", &home_file)
        .env("USERPROFILE", &home_file)
        .args(["--db", ":memory:", "stats"])
        .output()
        .expect("run quaid");

    assert!(
        output.status.success(),
        "db::open(\":memory:\") must succeed without a writable HOME\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn in_memory_open_creates_no_directories_under_home() {
    let dir = TempDir::new().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&home, fs::Permissions::from_mode(0o555)).unwrap();
    }

    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    let output = command
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .args(["--db", ":memory:", "stats"])
        .output()
        .expect("run quaid");

    let leftover: Vec<_> = fs::read_dir(&home)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).unwrap();
    }

    assert!(
        output.status.success(),
        "db::open(\":memory:\") must succeed with a read-only HOME\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        leftover.is_empty(),
        "open must create no directories under HOME, found: {leftover:?}"
    );
}

// ── crash-partial fresh-bootstrap recovery ────────────────────

/// Builds a crash-partial database: fully bootstrapped, then `quaid_config`
/// emptied (as if the process died between schema DDL and the config write),
/// with optional activity rows seeded.
fn crash_partial_db_with(seed_sql: Option<&str>) -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let path = db_path.to_str().unwrap().to_owned();
    {
        let conn = db::open(&path).unwrap();
        conn.execute("DELETE FROM quaid_config", []).unwrap();
        if let Some(sql) = seed_sql {
            conn.execute_batch(sql).unwrap();
        }
    }
    (dir, path)
}

#[test]
fn crash_partial_recovery_reclaims_truly_fresh_db() {
    let (_dir, path) = crash_partial_db_with(None);
    db::open(&path).expect("fresh-bootstrap recovery must reclaim an activity-free db");
}

#[test]
fn crash_partial_recovery_rejects_seeded_extraction_queue() {
    let (_dir, path) = crash_partial_db_with(Some(
        "INSERT INTO extraction_queue
             (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, status)
         VALUES
             ('s1', 'conversations/2026-06-12/s1.md', 'debounce',
              '2026-06-12T00:00:00Z', '2026-06-12T00:00:05Z', 'pending');",
    ));

    let err = db::open(&path).expect_err("queued extraction jobs must block recovery");
    assert!(
        matches!(err, DbError::Schema { .. }),
        "expected DbError::Schema, got: {err:?}"
    );
}

#[test]
fn crash_partial_recovery_rejects_seeded_correction_sessions() {
    let (_dir, path) = crash_partial_db_with(Some(
        "INSERT INTO correction_sessions
             (correction_id, fact_slug, exchange_log)
         VALUES
             ('correction-1', 'facts/example', '[]');",
    ));

    let err = db::open(&path).expect_err("open correction sessions must block recovery");
    assert!(
        matches!(err, DbError::Schema { .. }),
        "expected DbError::Schema, got: {err:?}"
    );
}

#[test]
fn crash_partial_recovery_rejects_seeded_namespaces() {
    let (_dir, path) = crash_partial_db_with(Some("INSERT INTO namespaces (id) VALUES ('alpha');"));

    let err = db::open(&path).expect_err("registered namespaces must block recovery");
    assert!(
        matches!(err, DbError::Schema { .. }),
        "expected DbError::Schema, got: {err:?}"
    );
}
