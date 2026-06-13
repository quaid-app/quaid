#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Cross-process optimistic-concurrency race against the *production* write
//! path. Unlike `tests/concurrency_stress.rs` (which hand-rolls the OCC
//! `UPDATE` in-process), this test spawns the real `quaid` binary: N
//! concurrent `quaid put <slug> --expected-version 1` subprocesses all racing
//! to update the same page. The production guarantee this pins is
//! **exactly-one-winner via the `ConflictError` surface**: no matter how the
//! writers interleave, exactly one commits (version 1 → 2) and every other
//! writer is rejected, never silently clobbering the winner.
//!
//! SQLITE_BUSY caveat: the CLI write path serialises same-slug writers with an
//! *in-process* mutex (`with_write_slug_lock`), which does not span separate
//! processes, and it opens its write transaction with `BEGIN DEFERRED`
//! (`unchecked_transaction`). A losing writer that has already taken the read
//! lock can therefore observe a transient `SQLITE_BUSY` on the
//! deferred→write upgrade that the 5s busy timeout cannot retry from inside an
//! open transaction. That is a *loser* outcome — it never produces a second
//! winner or a corrupt row — so this test treats a BUSY loser as one valid
//! (if suboptimal) way to lose, and counts it separately. Converting the put
//! write transaction to `BEGIN IMMEDIATE` (the `db::with_immediate_transaction`
//! hygiene from the connection-hygiene workstream) would let every loser fail
//! cleanly via `ConflictError`; that is a production-path change outside this
//! test slice. See the assertions below for the exact invariant that holds
//! unconditionally today.
//!
//! The contention barrier is filesystem-free: every child is spawned with its
//! stdin held open first, and the parent only writes-and-closes the stdins
//! once all children exist. `quaid put` reads stdin to EOF before touching the
//! DB, so closing all stdins in a tight loop releases the writers into the
//! commit window together. Generous timeouts plus `#[serial]` keep CI stable.

#[path = "common/mod.rs"]
mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use std::io::Write;
use std::process::{Child, Command, Stdio};

use serial_test::serial;
use truth_fixtures::{open_test_db, run_quaid_with_stdin, test_db_path};

#[cfg(unix)]
fn spawn_put_child(db_path: &std::path::Path, slug: &str, expected_version: i64) -> Child {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .arg("--db")
        .arg(db_path)
        .args([
            "put",
            slug,
            "--expected-version",
            &expected_version.to_string(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().expect("spawn quaid put child")
}

#[cfg(unix)]
#[test]
#[serial]
fn concurrent_cli_put_yields_exactly_one_winner_via_conflict_error() {
    const WRITERS: usize = 6;

    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cross-process-occ.db");
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create vault root");
    let conn = open_test_db(&db_path);
    // Point the seeded default write-target collection (id=1) at our vault.
    // `is_write_target` is UNIQUE, so we reuse the default rather than insert a
    // second write-target.
    conn.execute(
        "UPDATE collections
         SET root_path = ?1, writable = 1, is_write_target = 1, state = 'active'
         WHERE id = 1",
        [root.display().to_string()],
    )
    .expect("point default collection at vault");
    drop(conn);

    // Create the page at version 1 through the production path so the file,
    // raw_imports, and DB row are all coherent before the race.
    let create = run_quaid_with_stdin(
        &db_path,
        &["put", "notes/race"],
        "---\ntitle: Race\ntype: note\n---\nseed body for the OCC race.\n",
    );
    assert!(
        create.status.success(),
        "seed create must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&create.stdout),
        String::from_utf8_lossy(&create.stderr)
    );

    // Spawn every writer first (stdin held open), so none can reach the commit
    // window until the parent releases them together below.
    let mut children: Vec<Child> = (0..WRITERS)
        .map(|_| spawn_put_child(&db_path, "notes/race", 1))
        .collect();

    // Release: write each child's update body and close its stdin. `quaid put`
    // blocks on stdin EOF before the DB write, so the writers fan into the
    // commit window here.
    for (index, child) in children.iter_mut().enumerate() {
        let body = format!("---\ntitle: Race\ntype: note\n---\nwriter {index} body.\n");
        child
            .stdin
            .take()
            .expect("child stdin pipe")
            .write_all(body.as_bytes())
            .expect("write child stdin");
    }

    let mut winners = 0usize;
    let mut conflicts = 0usize;
    let mut busy_losers = 0usize;
    let mut other_failures = Vec::new();
    for (index, child) in children.into_iter().enumerate() {
        let output = child.wait_with_output().expect("wait for child");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        if output.status.success() {
            winners += 1;
        } else if combined.contains("ConflictError") && combined.contains("StaleExpectedVersion") {
            // The production conflict surface: another writer won the CAS.
            conflicts += 1;
        } else if combined.contains("database is locked")
            || combined.contains("SQLITE_BUSY")
            || combined.contains("database table is locked")
        {
            // Tolerated loser outcome (see module docs): a deferred-transaction
            // upgrade lost the write lock. Still a loser, never a second winner.
            busy_losers += 1;
        } else {
            other_failures.push(format!("writer {index}: {combined}"));
        }
    }

    assert!(
        other_failures.is_empty(),
        "every non-winner must lose cleanly (ConflictError or transient busy); unexpected failures: {other_failures:?}"
    );
    // The load-bearing invariant: exactly one writer commits, full stop.
    assert_eq!(
        winners, 1,
        "exactly one concurrent writer may win the OCC race \
         (winners={winners}, conflicts={conflicts}, busy_losers={busy_losers})"
    );
    assert_eq!(
        conflicts + busy_losers,
        WRITERS - 1,
        "every non-winner must lose (conflicts={conflicts}, busy_losers={busy_losers})"
    );

    // The single winner advanced the version to 2 and its body is on disk.
    let verify = open_test_db(&db_path);
    let (version, truth): (i64, String) = verify
        .query_row(
            "SELECT version, compiled_truth FROM pages WHERE slug = 'notes/race'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("winner row present");
    assert_eq!(version, 2, "exactly one increment from the seed version");
    assert!(
        truth.contains("writer "),
        "winning body must be one of the racing writers: {truth}"
    );
}

/// Deterministic companion to the race above: a stale `--expected-version`
/// against a page that has already advanced MUST fail with the production
/// `StaleExpectedVersion` `ConflictError` and a non-zero exit, with no
/// contention involved. This pins the conflict *surface* itself so the race
/// test's exactly-one-winner guarantee rests on a verified error path.
#[cfg(unix)]
#[test]
#[serial]
fn stale_expected_version_put_fails_with_production_conflict_error() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cross-process-stale.db");
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create vault root");
    let conn = open_test_db(&db_path);
    conn.execute(
        "UPDATE collections
         SET root_path = ?1, writable = 1, is_write_target = 1, state = 'active'
         WHERE id = 1",
        [root.display().to_string()],
    )
    .expect("point default collection at vault");
    drop(conn);

    let create = run_quaid_with_stdin(
        &db_path,
        &["put", "notes/stale"],
        "---\ntitle: Stale\ntype: note\n---\nseed.\n",
    );
    assert!(
        create.status.success(),
        "seed create must succeed: {create:?}"
    );

    // Advance to version 2 with the correct expected version.
    let advance = run_quaid_with_stdin(
        &db_path,
        &["put", "notes/stale", "--expected-version", "1"],
        "---\ntitle: Stale\ntype: note\n---\nadvanced to v2.\n",
    );
    assert!(
        advance.status.success(),
        "advance to v2 must succeed: {advance:?}"
    );

    // Now a stale write at expected-version 1 must fail with the conflict.
    let stale = run_quaid_with_stdin(
        &db_path,
        &["put", "notes/stale", "--expected-version", "1"],
        "---\ntitle: Stale\ntype: note\n---\nthis must be rejected.\n",
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&stale.stdout),
        String::from_utf8_lossy(&stale.stderr)
    );
    assert!(!stale.status.success(), "stale put must fail: {combined}");
    assert!(
        combined.contains("ConflictError") && combined.contains("StaleExpectedVersion"),
        "stale put must surface the production StaleExpectedVersion ConflictError: {combined}"
    );
    assert!(
        combined.contains("current version: 2"),
        "conflict must report the current version the caller must refresh to: {combined}"
    );

    // The rejected write left the page at v2 with the advanced body intact.
    let verify = open_test_db(&db_path);
    let (version, truth): (i64, String) = verify
        .query_row(
            "SELECT version, compiled_truth FROM pages WHERE slug = 'notes/stale'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("page row present");
    assert_eq!(version, 2, "rejected write must not advance the version");
    assert!(
        truth.contains("advanced to v2"),
        "rejected write must not clobber the winning body: {truth}"
    );
}
