#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Extraction pipeline atomicity tests: a concurrently appended turn must
//! survive the worker's cursor rewrite, and a session spanning a midnight
//! rollover must extract the tails of *both* day-files.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::thread;
use std::time::Duration;

use quaid::core::conversation::{
    extractor::{SlmClient, Worker},
    format, queue,
    slm::SlmError,
    supersede::ResolvingFactWriter,
    turn_writer,
};
use quaid::core::db;
use quaid::core::types::{ExtractionTriggerKind, TurnRole};
use rusqlite::Connection;

fn open_vault_db(dir: &Path) -> (Connection, PathBuf) {
    let db_path = dir.join("memory.db");
    let vault_root = dir.join("vault");
    fs::create_dir_all(&vault_root).unwrap();
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             writable = 1,
             is_write_target = 1,
             state = 'active',
             needs_full_sync = 0
         WHERE id = 1",
        [vault_root.display().to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO config(key, value) VALUES ('extraction.enabled', 'true')",
        [],
    )
    .unwrap();
    (conn, vault_root)
}

fn append_turn(conn: &Connection, session_id: &str, ordinal: i64, content: &str, timestamp: &str) {
    let role = if ordinal % 2 == 0 {
        TurnRole::Assistant
    } else {
        TurnRole::User
    };
    turn_writer::append_turn(conn, session_id, role, content, timestamp, None, None).unwrap();
}

fn extracted_markdown_files(root: &Path) -> Vec<PathBuf> {
    fn visit(dir: &Path, files: &mut Vec<PathBuf>) {
        if !dir.exists() {
            return;
        }
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    visit(root, &mut files);
    files.sort();
    files
}

/// SLM stub that signals when inference starts and blocks until the test
/// releases it, so the test can deterministically interleave an
/// `append_turn` with an in-flight extraction window.
struct BlockingSlm {
    entered_tx: mpsc::Sender<()>,
    release_rx: mpsc::Receiver<()>,
    output: String,
}

impl SlmClient for BlockingSlm {
    fn infer(&self, _alias: &str, _prompt: &str, _max_tokens: usize) -> Result<String, SlmError> {
        self.entered_tx.send(()).expect("signal inference entered");
        self.release_rx
            .recv()
            .expect("await release from test thread");
        Ok(self.output.clone())
    }
}

/// Minimal FIFO stub that pops one canned response per inference call.
struct QueueStubSlm {
    outputs: Mutex<VecDeque<String>>,
}

impl QueueStubSlm {
    fn new(outputs: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into_iter().map(str::to_owned).collect()),
        }
    }
}

impl SlmClient for QueueStubSlm {
    fn infer(&self, _alias: &str, _prompt: &str, _max_tokens: usize) -> Result<String, SlmError> {
        Ok(self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "{\"facts\":[]}".to_string()))
    }
}

#[test]
fn turn_appended_during_slow_extraction_window_survives_cursor_rewrite() {
    let dir = tempfile::TempDir::new().unwrap();
    let (conn, vault_root) = open_vault_db(dir.path());
    let db_path = dir.path().join("memory.db");

    for ordinal in 1..=4 {
        append_turn(
            &conn,
            "slow-session",
            ordinal,
            &format!("turn {ordinal}"),
            &format!("2026-05-05T09:0{ordinal}:00Z"),
        );
    }
    queue::enqueue(
        &conn,
        "slow-session",
        "conversations/2026-05-05/slow-session.md",
        ExtractionTriggerKind::Manual,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let slm = BlockingSlm {
        entered_tx,
        release_rx,
        output: r#"{"facts":[{"kind":"preference","about":"systems-language","strength":"high","summary":"The team prefers Rust for systems work."}]}"#
            .to_string(),
    };

    let worker_thread = thread::spawn(move || {
        let worker_conn = db::open(db_path.to_str().unwrap()).unwrap();
        let worker = Worker::new(&worker_conn, slm, ResolvingFactWriter)
            .unwrap()
            .with_limits(Duration::from_millis(1), 256);
        worker.process_next_job().map(|job| job.is_some())
    });

    entered_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("worker must reach SLM inference");
    // The window [1..4] is mid-inference; append a fifth turn through the
    // production locked append path. The worker's cursor write must take the
    // same per-session locks afterwards and must not clobber this turn with
    // its pre-inference snapshot.
    append_turn(
        &conn,
        "slow-session",
        5,
        "fifth turn arrives mid-extraction",
        "2026-05-05T09:05:00Z",
    );
    release_tx.send(()).expect("release the stubbed SLM");

    let processed = worker_thread
        .join()
        .expect("worker thread must not panic")
        .expect("process_next_job must succeed");
    assert!(processed, "worker must have claimed and processed the job");

    let day_file = vault_root
        .join("conversations")
        .join("2026-05-05")
        .join("slow-session.md");
    let parsed = format::parse(&day_file).unwrap();
    assert_eq!(
        parsed.turns.len(),
        5,
        "turn appended during extraction must survive the cursor rewrite"
    );
    assert_eq!(parsed.turns[4].ordinal, 5);
    assert!(parsed.turns[4]
        .content
        .contains("fifth turn arrives mid-extraction"));
    assert_eq!(
        parsed.frontmatter.last_extracted_turn, 4,
        "cursor must reflect only the extracted window, leaving turn 5 for the next job"
    );
    assert!(parsed.frontmatter.last_extracted_at.is_some());

    let fact_files = extracted_markdown_files(&vault_root.join("extracted"));
    assert_eq!(fact_files.len(), 1, "the window's fact must be on disk");
}

#[test]
fn midnight_rollover_extracts_both_day_file_tails() {
    let dir = tempfile::TempDir::new().unwrap();
    let (conn, vault_root) = open_vault_db(dir.path());

    // Two turns just before midnight land in the 2026-05-05 day-file; two
    // more after midnight land in 2026-05-06 (ordinals continue at 3, 4).
    append_turn(
        &conn,
        "roll-session",
        1,
        "evening: we settled on tabs over spaces",
        "2026-05-05T23:58:00Z",
    );
    append_turn(
        &conn,
        "roll-session",
        2,
        "noted before midnight",
        "2026-05-05T23:59:00Z",
    );
    append_turn(
        &conn,
        "roll-session",
        3,
        "morning: we chose pnpm for the monorepo",
        "2026-05-06T00:01:00Z",
    );
    append_turn(
        &conn,
        "roll-session",
        4,
        "captured after midnight",
        "2026-05-06T00:02:00Z",
    );

    // Mirror the production add-turn enqueue across the rollover: one
    // enqueue per day-file path. Pre-fix, the second call overwrote the
    // pending row's conversation_path and stranded the 05-05 tail.
    queue::enqueue(
        &conn,
        "roll-session",
        "conversations/2026-05-05/roll-session.md",
        ExtractionTriggerKind::Debounce,
        "2000-01-01T00:00:01Z",
    )
    .unwrap();
    queue::enqueue(
        &conn,
        "roll-session",
        "conversations/2026-05-06/roll-session.md",
        ExtractionTriggerKind::Debounce,
        "2000-01-01T00:00:02Z",
    )
    .unwrap();

    let pending: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM extraction_queue
             WHERE session_id = 'roll-session' AND status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 2, "each day-file must keep its own pending row");

    let slm = QueueStubSlm::new([
        r#"{"facts":[{"kind":"decision","chose":"tabs-over-spaces","summary":"The team settled on tabs over spaces."}]}"#,
        r#"{"facts":[{"kind":"decision","chose":"pnpm-monorepo","summary":"The team chose pnpm for the monorepo."}]}"#,
    ]);
    let worker = Worker::new(&conn, slm, ResolvingFactWriter)
        .unwrap()
        .with_limits(Duration::from_millis(1), 256);

    let first = worker.process_next_job().unwrap();
    assert!(first.is_some(), "first day-file job must be claimed");
    let second = worker.process_next_job().unwrap();
    assert!(second.is_some(), "second day-file job must be claimed");
    assert!(
        worker.process_next_job().unwrap().is_none(),
        "no third job expected"
    );

    let day_one = format::parse(
        &vault_root
            .join("conversations")
            .join("2026-05-05")
            .join("roll-session.md"),
    )
    .unwrap();
    let day_two = format::parse(
        &vault_root
            .join("conversations")
            .join("2026-05-06")
            .join("roll-session.md"),
    )
    .unwrap();
    assert_eq!(
        day_one.frontmatter.last_extracted_turn, 2,
        "the prior day's tail must have been extracted"
    );
    assert_eq!(
        day_two.frontmatter.last_extracted_turn, 4,
        "the new day's tail must have been extracted"
    );

    let fact_files = extracted_markdown_files(&vault_root.join("extracted"));
    assert_eq!(
        fact_files.len(),
        2,
        "both day tails must have produced their fact file: {fact_files:?}"
    );
}
