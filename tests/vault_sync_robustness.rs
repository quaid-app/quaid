#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Vault-sync robustness: heartbeat isolation from the supervisor work
//! thread, pid-aware stale-session sweeping, and fail-safe `needs_full_sync`
//! flagging on watcher degradation (notify errors, crash re-arm).

#[path = "common/vault_sync_fixtures.rs"]
mod fixtures;

use fixtures::*;

use std::thread;
use std::time::Duration;

use rusqlite::Connection;

use quaid::core::vault_sync::{
    current_host, register_session, start_serve_runtime, sweep_stale_sessions, SessionType,
};

// ── Heartbeat isolation ───────────────────────────────────────

/// The session heartbeat must tick while the supervisor work thread is
/// blocked (the deterministic equivalent of a 20s reconcile):
/// `QUAID_TEST_SUPERVISOR_TICK_HOLD_MS` pins the work thread inside one tick
/// for 8s — past the 5s heartbeat interval — and the dedicated heartbeat
/// thread must still refresh `heartbeat_at` so another process's sweep can
/// never reap the live session mid-operation.
#[test]
fn heartbeat_ticks_while_supervisor_work_thread_is_blocked() {
    let _lock = env_mutation_lock().lock().unwrap();
    let _hold = EnvVarGuard::set("QUAID_TEST_SUPERVISOR_TICK_HOLD_MS", "8000");

    let (_dir, db_path, conn) = open_test_db_file();
    drop(conn);

    let runtime = start_serve_runtime(db_path.clone()).unwrap();
    let conn = Connection::open(&db_path).unwrap();
    let first_heartbeat: String = conn
        .query_row(
            "SELECT heartbeat_at FROM serve_sessions WHERE session_id = ?1",
            [runtime.session_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    drop(conn);

    // Well inside the work thread's 8s hold, but past one 5s heartbeat
    // interval. Before heartbeat isolation, the keepalive shared the held
    // thread and could not have ticked yet.
    thread::sleep(Duration::from_millis(6500));

    let conn = Connection::open(&db_path).unwrap();
    let row: Option<String> = conn
        .query_row(
            "SELECT heartbeat_at FROM serve_sessions WHERE session_id = ?1",
            [runtime.session_id.as_str()],
            |row| row.get(0),
        )
        .ok();
    drop(conn);

    let second_heartbeat = row.expect("session row must survive while the work thread is held");
    assert_ne!(
        first_heartbeat, second_heartbeat,
        "heartbeat must advance while the supervisor work thread is blocked"
    );

    drop(runtime);
}

// ── pid-aware stale-session sweep ─────────────────────────────

/// `sweep_stale_sessions` must skip a stale-by-time row whose pid is alive
/// on this host (a busy-but-live process must not be reaped mid-operation),
/// while keeping the pure time-window behaviour for dead local pids and for
/// rows registered from other hosts.
#[test]
fn sweep_skips_live_local_pid_but_reaps_dead_and_foreign_rows() {
    let conn = open_test_db();

    // Live local session: this test process's pid, this host.
    let live_session = register_session(&conn, SessionType::Serve).unwrap();
    conn.execute(
        "UPDATE serve_sessions
         SET heartbeat_at = datetime('now', '-60 seconds')
         WHERE session_id = ?1",
        [live_session.as_str()],
    )
    .unwrap();

    // Dead local pid: a spawned-and-reaped child on this host.
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = i64::from(child.id());
    child.wait().unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('dead-local', ?1, ?2, 'serve', datetime('now', '-60 seconds'))",
        rusqlite::params![dead_pid, current_host()],
    )
    .unwrap();

    // Foreign host: pid happens to be alive here, but liveness cannot be
    // probed across hosts, so the time window must still apply.
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, session_type, heartbeat_at)
         VALUES ('stale-foreign', ?1, 'definitely-not-this-host', 'serve',
                 datetime('now', '-60 seconds'))",
        [i64::from(std::process::id())],
    )
    .unwrap();

    let removed = sweep_stale_sessions(&conn).unwrap();

    assert_eq!(removed, 2, "dead-local and stale-foreign rows are reaped");
    let survivors: Vec<String> = conn
        .prepare("SELECT session_id FROM serve_sessions ORDER BY session_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(survivors, vec![live_session]);
}

// ── Fail-safe needs_full_sync flagging (source audits) ────────
//
// `watch_callback` and `sync_collection_watchers` are private to the
// vault_sync module and notify errors cannot be injected through the public
// API, so these follow the repo's established source-introspection pattern
// (see tests/vault_sync_runtime.rs) to pin the fail-safe behaviour.

#[test]
fn watch_callback_flags_full_sync_on_every_degradation_path() {
    let source = production_vault_sync_source();
    let start = source.find("pub(super) fn watch_callback(").unwrap();
    let end = start
        + source[start..]
            .find("pub(super) fn flag_needs_full_sync_via_runtime_connection(")
            .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("ERROR: watch_event_error"),
        "notify backend errors must be logged, not silently dropped: {snippet}"
    );
    assert!(
        snippet.contains("ERROR: watch_classify_error"),
        "classification failures must be logged, not silently dropped: {snippet}"
    );
    assert!(
        !snippet.contains("let Ok(event) = result else"),
        "the silent notify-error drop pattern must not return: {snippet}"
    );
    assert_eq!(
        snippet
            .matches("flag_needs_full_sync_via_runtime_connection(")
            .count(),
        3,
        "notify error, classify error, and channel overflow must all set needs_full_sync: {snippet}"
    );

    let helper_start = source
        .find("pub(super) fn flag_needs_full_sync_via_runtime_connection(")
        .unwrap();
    let helper = &source[helper_start..helper_start + 1500];
    assert!(
        helper.contains("db::open_runtime")
            && helper.contains("needs_full_sync_flag_write_failed")
            && helper.contains("needs_full_sync_flag_open_failed"),
        "flag writes must use a runtime connection (5s busy timeout) and log both failure modes loudly: {helper}"
    );
}

#[test]
fn sync_collection_watchers_flags_full_sync_when_rearming_after_crash() {
    let source = production_vault_sync_source();
    let start = source.find("fn sync_collection_watchers(").unwrap();
    let end = start
        + source[start..]
            .find("fn detach_active_collections_with_empty_root_path(")
            .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("rearming_after_crash")
            && snippet.contains("mark_collection_needs_full_sync(conn, collection_id)")
            && snippet.contains("watcher_rearm_flagged_full_sync")
            && snippet.contains("watcher_rearm_needs_full_sync_write_failed"),
        "crash re-arm must flag needs_full_sync and log flag-write failures: {snippet}"
    );
}

#[test]
fn supervisor_heartbeat_runs_on_dedicated_thread_and_logs_failures() {
    let source = production_vault_sync_source();
    let loop_start = source.find("fn run_supervisor_loop(").unwrap();
    let loop_end = loop_start
        + source[loop_start..]
            .find("fn run_session_heartbeat_loop(")
            .unwrap();
    let loop_snippet = &source[loop_start..loop_end];

    assert!(
        loop_snippet.contains("run_session_heartbeat_loop(")
            && loop_snippet.contains("thread::spawn"),
        "the supervisor must spawn the heartbeat on a dedicated thread: {loop_snippet}"
    );
    assert!(
        !loop_snippet.contains("let _ = heartbeat_session"),
        "the work loop must no longer run (or swallow) heartbeats inline: {loop_snippet}"
    );

    let hb_start = source.find("fn run_session_heartbeat_loop(").unwrap();
    let hb_snippet = &source[hb_start..hb_start + 2500];
    assert!(
        hb_snippet.contains("db::open_runtime")
            && hb_snippet.contains("WARN: session_heartbeat_failed")
            && hb_snippet.contains("WARN: session_heartbeat_open_failed")
            && hb_snippet.contains("WARN: session_sweep_failed"),
        "the heartbeat loop must use a runtime connection and log every failure instead of `let _ =`: {hb_snippet}"
    );
}
