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
    find_active_daemon_session, find_active_runtime_host, heartbeat_session, register_session,
    sweep_stale_sessions, try_promote_to_serve_host, SessionType,
};

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
