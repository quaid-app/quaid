#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use quaid::core::db;
use quaid::mcp::server::{MemoryAddTurnInput, QuaidServer};
use rusqlite::Connection;

fn open_turn_server(root: &Path) -> (tempfile::TempDir, PathBuf, QuaidServer) {
    let db_dir = tempfile::TempDir::new().unwrap();
    let db_path = db_dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active'
         WHERE id = 1",
        [root.display().to_string()],
    )
    .unwrap();
    (db_dir, db_path, QuaidServer::new(conn))
}

fn enable_extraction(db_path: &Path) -> Connection {
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO config(key, value)
         VALUES ('extraction.enabled', 'true'),
                ('extraction.debounce_ms', '5000')",
        [],
    )
    .unwrap();
    conn
}

fn percentile(sorted: &[f64], pct: usize) -> f64 {
    let index = (pct * (sorted.len() - 1)).div_ceil(100);
    sorted[index]
}

/// Absolute p95 budget for `memory_add_turn`, overridable via the
/// `QUAID_LATENCY_P95_MS` env var so CI on slow/debug hardware can set a
/// generous ceiling while SSD release runs keep the tight default. Mirrors the
/// `QUAID_LATENCY_P95_MS` pattern used by `tests/corpus_reality.rs`.
fn p95_budget_ms() -> f64 {
    std::env::var("QUAID_LATENCY_P95_MS")
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(250.0)
}

/// Appends `count` turns to one session and returns per-call wall times in ms.
fn time_turns(server: &QuaidServer, session_id: &str, count: usize) -> Vec<f64> {
    let mut durations_ms = Vec::with_capacity(count);
    for ordinal in 0..count {
        let start = Instant::now();
        server
            .memory_add_turn(MemoryAddTurnInput {
                session_id: session_id.to_string(),
                role: "user".to_string(),
                content: format!("turn {ordinal}"),
                timestamp: Some(format!("2026-05-03T09:14:{:02}Z", ordinal % 60)),
                metadata: None,
                namespace: None,
            })
            .unwrap();
        durations_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    durations_ms
}

// Un-ignored: with the per-session cursor cache, `memory_add_turn` no longer
// rescans every day-file per turn, so 100 same-day appends stay well under a
// generous, env-overridable budget even on debug CI hardware.
#[test]
fn memory_add_turn_100_calls_p95_within_budget() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, db_path, server) = open_turn_server(vault_root.path());
    let _config_conn = enable_extraction(&db_path);

    let mut durations_ms = time_turns(&server, "latency-session", 100);
    durations_ms.sort_by(|a, b| a.total_cmp(b));
    let p95 = percentile(&durations_ms, 95);
    let budget = p95_budget_ms();

    assert!(
        p95 < budget,
        "memory_add_turn p95 {p95:.1}ms exceeds {budget:.1}ms budget (override via QUAID_LATENCY_P95_MS)"
    );
}

// Scaling guard: the per-turn cost must not grow with the number of prior
// same-session turns. Before the cursor cache, append_turn was O(session) per
// turn (O(N^2) per session); the late-window median must stay close to the
// early-window median. Ratio-based so it is robust to absolute hardware speed.
#[test]
fn memory_add_turn_cost_does_not_scale_with_session_length() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, db_path, server) = open_turn_server(vault_root.path());
    let _config_conn = enable_extraction(&db_path);

    let durations_ms = time_turns(&server, "scaling-session", 200);

    let median = |slice: &[f64]| -> f64 {
        let mut sorted = slice.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        sorted[sorted.len() / 2]
    };
    // Skip the first few warm-up turns (lock/dir creation, model config read).
    let early = median(&durations_ms[5..25]);
    let late = median(&durations_ms[180..200]);

    // A small additive floor avoids dividing a sub-millisecond early median.
    let ratio = (late + 0.5) / (early + 0.5);
    assert!(
        ratio < 4.0,
        "late-window median {late:.3}ms vs early-window {early:.3}ms (ratio {ratio:.2}) suggests per-turn cost still scales with session length"
    );
}
