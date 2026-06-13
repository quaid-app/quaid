#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics"
)]

//! `SessionType` lifecycle, `register_session` persistence, sweep behaviour
//! across all four types, partial-rollback fallback (old binary filtering
//! `session_type = 'serve'`), and the atomic `try_promote_to_serve_host`
//! lease used by `quaid serve` to elect the no-daemon runtime owner.
//!
//! Spec backing: `openspec/changes/daemon-and-http-transport/specs/vault-sync/spec.md`
//! requirements:
//!  - `serve_sessions.session_type` accepts daemon/serve_host/serve/cli
//!  - Atomic runtime-host promotion from `serve` to `serve_host`
//!  - Old binary safely reads new session_type values

#[path = "common/vault_sync_fixtures.rs"]
mod fixtures;

use fixtures::open_test_db;

use std::sync::{Arc, Barrier};
use std::thread;

use rusqlite::params;

use quaid::core::vault_sync::{
    current_host, find_active_daemon_session, find_active_runtime_host, heartbeat_session,
    register_cli_session, register_session, start_daemon_runtime, start_serve_runtime,
    sweep_stale_sessions, try_claim_daemon_session, try_promote_to_serve_host, DaemonSessionClaim,
    SessionType,
};

#[test]
fn session_type_maps_to_db_strings_and_runtime_host_flags() {
    assert_eq!(SessionType::Daemon.to_db_str(), "daemon");
    assert_eq!(SessionType::ServeHost.to_db_str(), "serve_host");
    assert_eq!(SessionType::Serve.to_db_str(), "serve");
    assert_eq!(SessionType::Cli.to_db_str(), "cli");

    assert!(SessionType::Daemon.is_runtime_host());
    assert!(SessionType::ServeHost.is_runtime_host());
    assert!(!SessionType::Serve.is_runtime_host());
    assert!(!SessionType::Cli.is_runtime_host());
}

#[test]
fn register_session_persists_correct_session_type_for_each_variant() {
    let conn = open_test_db();

    let daemon_id = register_session(&conn, SessionType::Daemon).unwrap();
    let serve_id = register_session(&conn, SessionType::Serve).unwrap();
    let cli_id = register_session(&conn, SessionType::Cli).unwrap();
    // ServeHost cannot be registered directly — it is produced by promotion
    // from a `Serve` row via `try_promote_to_serve_host`. Verified separately
    // below; here we just confirm Daemon/Serve/Cli round-trip.

    for (sid, expected) in [
        (&daemon_id, "daemon"),
        (&serve_id, "serve"),
        (&cli_id, "cli"),
    ] {
        let actual: String = conn
            .query_row(
                "SELECT session_type FROM serve_sessions WHERE session_id = ?1",
                [sid],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(actual, expected, "session_id={sid}");
    }
}

#[test]
fn register_cli_session_persists_cli_role() {
    let conn = open_test_db();

    let session_id = register_cli_session(&conn).unwrap();

    let session_type: String = conn
        .query_row(
            "SELECT session_type FROM serve_sessions WHERE session_id = ?1",
            [&session_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(session_type, "cli");
}

#[test]
fn sweep_stale_sessions_is_session_type_agnostic() {
    let conn = open_test_db();

    // Insert four sessions, all aged past the 15s liveness threshold.
    for (id, ty) in [
        ("stale-daemon", "daemon"),
        ("stale-serve_host", "serve_host"),
        ("stale-serve", "serve"),
        ("stale-cli", "cli"),
    ] {
        conn.execute(
            "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
             VALUES (?1, 1, 'host', ?2, datetime('now', '-60 seconds'))",
            params![id, ty],
        )
        .unwrap();
    }

    let reaped = sweep_stale_sessions(&conn).unwrap();
    assert_eq!(reaped, 4, "all four session types should be swept");

    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM serve_sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn old_binary_filter_treats_daemon_and_serve_host_rows_as_non_owners() {
    // Partial-rollback safety net: a binary that hasn't been updated to
    // accept `daemon` / `serve_host` in its ownership filter (today's
    // `session_type = 'serve'`-only filter) must treat the new-typed rows
    // as non-owners, the safe fallback.
    let conn = open_test_db();

    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('d', 1, 'host', 'daemon', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('sh', 2, 'host', 'serve_host', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        [],
    )
    .unwrap();

    // Simulate the old-binary filter exactly.
    let owner_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions
             WHERE session_type = 'serve'
               AND heartbeat_at >= datetime('now', '-15 seconds')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        owner_count, 0,
        "old binary must see zero owners — the safe fallback for partial rollback"
    );
}

#[test]
fn find_active_daemon_session_returns_only_live_daemons() {
    let conn = open_test_db();
    assert!(find_active_daemon_session(&conn).unwrap().is_none());

    let daemon_id = register_session(&conn, SessionType::Daemon).unwrap();
    heartbeat_session(&conn, &daemon_id).unwrap();

    let found = find_active_daemon_session(&conn).unwrap().unwrap();
    assert_eq!(found.session_id, daemon_id);
    assert_eq!(found.session_type, "daemon");

    // A live serve_host or serve does not satisfy this query.
    let serve_id = register_session(&conn, SessionType::Serve).unwrap();
    heartbeat_session(&conn, &serve_id).unwrap();
    let found = find_active_daemon_session(&conn).unwrap().unwrap();
    assert_eq!(
        found.session_id, daemon_id,
        "serve insert must not displace the daemon match"
    );
}

#[test]
fn find_active_helpers_ignore_stale_runtime_hosts() {
    let conn = open_test_db();
    let daemon_id = register_session(&conn, SessionType::Daemon).unwrap();
    conn.execute(
        "UPDATE serve_sessions
         SET heartbeat_at = datetime('now', '-2 hours')
         WHERE session_id = ?1",
        [&daemon_id],
    )
    .unwrap();

    assert!(find_active_daemon_session(&conn).unwrap().is_none());
    assert!(find_active_runtime_host(&conn).unwrap().is_none());
}

#[test]
fn find_active_runtime_host_prefers_daemon_over_serve_host() {
    let conn = open_test_db();
    assert!(find_active_runtime_host(&conn).unwrap().is_none());

    // Promote a serve to serve_host first.
    let serve_id = register_session(&conn, SessionType::Serve).unwrap();
    heartbeat_session(&conn, &serve_id).unwrap();
    assert!(try_promote_to_serve_host(&conn, &serve_id).unwrap());

    let host = find_active_runtime_host(&conn).unwrap().unwrap();
    assert_eq!(host.session_type, "serve_host");

    // Now insert a daemon row. The runtime-host query must prefer it.
    // (By invariant the daemon refuses to start when a serve_host is live;
    // this test bypasses that invariant by direct insert to verify the ORDER BY.)
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('daemon-explicit', 99, 'host', 'daemon', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        [],
    )
    .unwrap();
    let host = find_active_runtime_host(&conn).unwrap().unwrap();
    assert_eq!(host.session_type, "daemon");
    assert_eq!(host.session_id, "daemon-explicit");
}

#[test]
fn try_promote_to_serve_host_succeeds_when_no_live_owner() {
    let conn = open_test_db();
    let serve_id = register_session(&conn, SessionType::Serve).unwrap();
    heartbeat_session(&conn, &serve_id).unwrap();

    let promoted = try_promote_to_serve_host(&conn, &serve_id).unwrap();
    assert!(promoted);

    let actual: String = conn
        .query_row(
            "SELECT session_type FROM serve_sessions WHERE session_id = ?1",
            [&serve_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual, "serve_host");
}

#[test]
fn try_promote_to_serve_host_refused_when_daemon_live() {
    let conn = open_test_db();
    let daemon_id = register_session(&conn, SessionType::Daemon).unwrap();
    heartbeat_session(&conn, &daemon_id).unwrap();

    let serve_id = register_session(&conn, SessionType::Serve).unwrap();
    heartbeat_session(&conn, &serve_id).unwrap();

    let promoted = try_promote_to_serve_host(&conn, &serve_id).unwrap();
    assert!(!promoted);

    // serve row remains 'serve', not 'serve_host'.
    let actual: String = conn
        .query_row(
            "SELECT session_type FROM serve_sessions WHERE session_id = ?1",
            [&serve_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual, "serve");
}

#[test]
fn try_promote_to_serve_host_rejects_missing_or_non_promotable_sessions() {
    let conn = open_test_db();

    let missing = try_promote_to_serve_host(&conn, "missing-session").unwrap_err();
    assert!(missing
        .to_string()
        .contains("session_id=missing-session not found"));

    let cli_id = register_session(&conn, SessionType::Cli).unwrap();
    let cli_error = try_promote_to_serve_host(&conn, &cli_id).unwrap_err();
    assert!(cli_error
        .to_string()
        .contains("caller session_type=cli is not promotable"));
}

#[test]
fn try_promote_to_serve_host_refused_when_another_serve_host_live() {
    let conn = open_test_db();
    let first = register_session(&conn, SessionType::Serve).unwrap();
    heartbeat_session(&conn, &first).unwrap();
    assert!(try_promote_to_serve_host(&conn, &first).unwrap());

    let second = register_session(&conn, SessionType::Serve).unwrap();
    heartbeat_session(&conn, &second).unwrap();
    let promoted = try_promote_to_serve_host(&conn, &second).unwrap();
    assert!(!promoted, "second promotion must be refused");
}

#[test]
fn try_promote_to_serve_host_sweeps_stale_daemon_then_promotes() {
    let conn = open_test_db();

    // Insert a stale daemon row (heartbeat past 15s threshold).
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('stale-d', 1, 'host', 'daemon', datetime('now', '-60 seconds'))",
        [],
    )
    .unwrap();

    let serve_id = register_session(&conn, SessionType::Serve).unwrap();
    heartbeat_session(&conn, &serve_id).unwrap();

    let promoted = try_promote_to_serve_host(&conn, &serve_id).unwrap();
    assert!(
        promoted,
        "stale daemon should be swept inside the same tx and promotion should succeed"
    );

    // Stale row is gone.
    let stale_remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'stale-d'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale_remaining, 0);

    // Caller is now serve_host.
    let actual: String = conn
        .query_row(
            "SELECT session_type FROM serve_sessions WHERE session_id = ?1",
            [&serve_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual, "serve_host");
}

#[test]
fn try_promote_to_serve_host_is_idempotent_when_caller_already_serve_host() {
    let conn = open_test_db();
    let serve_id = register_session(&conn, SessionType::Serve).unwrap();
    heartbeat_session(&conn, &serve_id).unwrap();
    assert!(try_promote_to_serve_host(&conn, &serve_id).unwrap());

    // Calling again must return true without error.
    let promoted = try_promote_to_serve_host(&conn, &serve_id).unwrap();
    assert!(promoted, "idempotent re-promotion must return true");

    let actual: String = conn
        .query_row(
            "SELECT session_type FROM serve_sessions WHERE session_id = ?1",
            [&serve_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual, "serve_host");
}

#[test]
fn try_promote_to_serve_host_concurrent_elects_exactly_one_winner() {
    // Two threads racing for promotion against a shared on-disk DB.
    // Exactly one must succeed; the other must observe a live `serve_host`
    // and be refused.
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("memory.db");

    let db_path_str = db_path.to_str().unwrap().to_string();

    // Initialize schema through the standard open path.
    let s1 = {
        let conn = quaid::core::db::open(&db_path_str).unwrap();
        let id = register_session(&conn, SessionType::Serve).unwrap();
        heartbeat_session(&conn, &id).unwrap();
        id
    };
    let s2 = {
        let conn = quaid::core::db::open(&db_path_str).unwrap();
        let id = register_session(&conn, SessionType::Serve).unwrap();
        heartbeat_session(&conn, &id).unwrap();
        id
    };

    let barrier = Arc::new(Barrier::new(2));

    let path_a = db_path_str.clone();
    let b_a = Arc::clone(&barrier);
    let t1 = thread::spawn(move || {
        let conn = quaid::core::db::open(&path_a).unwrap();
        b_a.wait();
        try_promote_to_serve_host(&conn, &s1).unwrap()
    });

    let path_b = db_path_str.clone();
    let b_b = Arc::clone(&barrier);
    let t2 = thread::spawn(move || {
        let conn = quaid::core::db::open(&path_b).unwrap();
        b_b.wait();
        try_promote_to_serve_host(&conn, &s2).unwrap()
    });

    let r1 = t1.join().unwrap();
    let r2 = t2.join().unwrap();

    assert!(
        r1 ^ r2,
        "exactly one promotion must succeed (r1={r1} r2={r2})"
    );

    // DB now contains exactly one serve_host row.
    let conn = quaid::core::db::open(&db_path_str).unwrap();
    let host_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_type = 'serve_host'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(host_count, 1);
}

#[test]
fn start_serve_runtime_returns_transport_only_when_daemon_is_live() {
    let (_dir, db_path, conn) = fixtures::open_test_db_file();
    let daemon_session = register_session(&conn, SessionType::Daemon).unwrap();
    drop(conn);

    let runtime = start_serve_runtime(db_path.clone()).unwrap();
    let serve_session = runtime.session_id.clone();

    let conn = quaid::core::db::open(&db_path).unwrap();
    let serve_type: String = conn
        .query_row(
            "SELECT session_type FROM serve_sessions WHERE session_id = ?1",
            [&serve_session],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(serve_type, "serve");

    drop(runtime);

    let remaining_transport_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_id = ?1",
            [&serve_session],
            |row| row.get(0),
        )
        .unwrap();
    let daemon_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_id = ?1",
            [&daemon_session],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_transport_rows, 0);
    assert_eq!(daemon_rows, 1);
}

/// A `daemon` row with a *fresh* heartbeat but a provably dead pid on
/// this host is what a SIGKILLed daemon leaves behind. The claim must
/// reap it and register the caller — otherwise the service manager's
/// replacement crash-loops until the row ages past the 15s liveness
/// window.
#[cfg(unix)]
#[test]
fn try_claim_daemon_session_reaps_dead_pid_row_with_fresh_heartbeat() {
    let conn = open_test_db();

    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = i64::from(child.id());
    child.wait().unwrap();

    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('dead-daemon', ?1, ?2, 'daemon', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![dead_pid, current_host()],
    )
    .unwrap();

    let session_id = match try_claim_daemon_session(&conn).unwrap() {
        DaemonSessionClaim::Claimed(session_id) => session_id,
        DaemonSessionClaim::AlreadyRunning(info) => {
            panic!("dead-pid fresh-heartbeat row must be reaped, got AlreadyRunning({info:?})")
        }
    };

    let dead_remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'dead-daemon'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dead_remaining, 0, "dead daemon's row must be deleted");

    let daemon_rows: Vec<String> = conn
        .prepare("SELECT session_id FROM serve_sessions WHERE session_type = 'daemon'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(daemon_rows, vec![session_id]);
}

/// Rows the claim must *not* reap: a foreign-host row (pid liveness
/// cannot be probed across hosts) and a live local pid. Both refuse
/// the claim with the existing row's snapshot.
#[test]
fn try_claim_daemon_session_respects_foreign_host_and_live_local_rows() {
    let conn = open_test_db();

    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('foreign-daemon', 1, 'definitely-not-this-host', 'daemon',
                 strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        [],
    )
    .unwrap();
    match try_claim_daemon_session(&conn).unwrap() {
        DaemonSessionClaim::AlreadyRunning(info) => {
            assert_eq!(info.session_id, "foreign-daemon");
        }
        DaemonSessionClaim::Claimed(id) => {
            panic!("foreign-host fresh-heartbeat row must refuse the claim, got Claimed({id})")
        }
    }
    conn.execute(
        "DELETE FROM serve_sessions WHERE session_id = 'foreign-daemon'",
        [],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('live-local', ?1, ?2, 'daemon', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![i64::from(std::process::id()), current_host()],
    )
    .unwrap();
    match try_claim_daemon_session(&conn).unwrap() {
        DaemonSessionClaim::AlreadyRunning(info) => {
            assert_eq!(info.session_id, "live-local");
        }
        DaemonSessionClaim::Claimed(id) => {
            panic!("live local pid must refuse the claim, got Claimed({id})")
        }
    }
}

/// Full-path version of the dead-pid reap: `start_daemon_runtime`
/// against a database whose `daemon` row points at a dead local pid
/// with a fresh heartbeat must boot immediately instead of erroring
/// with DaemonAlreadyRunningError for up to 15s.
#[cfg(unix)]
#[test]
fn start_daemon_runtime_reaps_dead_pid_daemon_row_and_starts() {
    let (_dir, db_path, conn) = fixtures::open_test_db_file();

    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = i64::from(child.id());
    child.wait().unwrap();

    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('sigkilled-daemon', ?1, ?2, 'daemon', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![dead_pid, current_host()],
    )
    .unwrap();
    drop(conn);

    let runtime = start_daemon_runtime(db_path.clone()).unwrap();

    let conn = quaid::core::db::open(&db_path).unwrap();
    let dead_remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_id = 'sigkilled-daemon'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dead_remaining, 0);
    let daemon_session: String = conn
        .query_row(
            "SELECT session_id FROM serve_sessions WHERE session_type = 'daemon'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(daemon_session, runtime.session_id);
    drop(conn);

    drop(runtime);
}

/// Two concurrent daemon starts against the same database must elect
/// exactly one winner: the claim's `BEGIN IMMEDIATE` closes the old
/// sweep→find→register TOCTOU where both could observe "no live
/// daemon" and both register.
#[test]
fn start_daemon_runtime_concurrent_starts_elect_exactly_one_winner() {
    let (_dir, db_path, conn) = fixtures::open_test_db_file();
    drop(conn);

    let barrier = Arc::new(Barrier::new(2));
    let spawn_start = |path: String, barrier: Arc<Barrier>| {
        thread::spawn(move || {
            barrier.wait();
            start_daemon_runtime(path)
        })
    };
    let t1 = spawn_start(db_path.clone(), Arc::clone(&barrier));
    let t2 = spawn_start(db_path.clone(), Arc::clone(&barrier));

    let results = [t1.join().unwrap(), t2.join().unwrap()];
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "exactly one concurrent daemon start may win"
    );
    for result in &results {
        if let Err(error) = result {
            assert!(
                error.to_string().contains("DaemonAlreadyRunningError"),
                "loser must observe the winner's row, got: {error}"
            );
        }
    }

    let conn = quaid::core::db::open(&db_path).unwrap();
    let daemon_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_type = 'daemon'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(daemon_rows, 1);
    drop(conn);

    drop(results);
}

#[test]
fn start_daemon_runtime_refuses_existing_live_daemon() {
    let (_dir, db_path, conn) = fixtures::open_test_db_file();
    let daemon_session = register_session(&conn, SessionType::Daemon).unwrap();
    drop(conn);

    let error = match start_daemon_runtime(db_path.clone()) {
        Ok(_) => panic!("second daemon runtime should be refused"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("DaemonAlreadyRunningError"));
    let conn = quaid::core::db::open(&db_path).unwrap();
    let daemon_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_type = 'daemon'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let existing_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM serve_sessions WHERE session_id = ?1",
            [&daemon_session],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(daemon_rows, 1);
    assert_eq!(existing_rows, 1);
}
