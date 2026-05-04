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

#[test]
#[ignore = "latency gate requires representative SSD hardware — run: cargo test --release --test turn_latency -- --ignored"]
fn memory_add_turn_100_calls_p95_under_50ms() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, db_path, server) = open_turn_server(vault_root.path());
    let _config_conn = enable_extraction(&db_path);

    let mut durations_ms = Vec::with_capacity(100);
    for ordinal in 0..100 {
        let start = Instant::now();
        server
            .memory_add_turn(MemoryAddTurnInput {
                session_id: "latency-session".to_string(),
                role: "user".to_string(),
                content: format!("turn {ordinal}"),
                timestamp: Some(format!("2026-05-03T09:14:{:02}Z", ordinal % 60)),
                metadata: None,
                namespace: None,
            })
            .unwrap();
        durations_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    durations_ms.sort_by(|a, b| a.total_cmp(b));
    let p95 = percentile(&durations_ms, 95);

    assert!(
        p95 < 50.0,
        "memory_add_turn p95 {p95:.1}ms exceeds 50ms gate — run on representative SSD hardware"
    );
}
