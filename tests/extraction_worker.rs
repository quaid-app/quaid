use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quaid::commands::ingest;
use quaid::core::conversation::{
    extractor::{FactWriter, PendingFactWriter, SlmClient, Worker, WorkerError},
    format, queue,
    slm::{parse_response, SlmError},
    supersede::ResolvingFactWriter,
};
use quaid::core::db;
use quaid::core::types::{
    ConversationFile, ConversationFrontmatter, ConversationStatus, ExtractionJob,
    ExtractionJobStatus, ExtractionResponse, ExtractionTriggerKind, Turn, TurnRole, WindowedTurns,
};
use rusqlite::Connection;

#[test]
fn process_next_job_should_drain_due_jobs_in_scheduled_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let fast_path =
        seed_conversation_file(dir.path(), "fast", conversation_with_cursor("fast", 0, 1));
    let slow_path =
        seed_conversation_file(dir.path(), "slow", conversation_with_cursor("slow", 0, 1));
    let slm = StubSlm::with_results([
        Ok("{\"facts\":[]}"),
        Ok("{\"facts\":[]}"),
        Ok("{\"facts\":[]}"),
    ]);
    let probe = slm.clone();
    let worker = worker_with_stub(&conn, slm);

    queue::enqueue(
        &conn,
        "slow",
        &slow_path,
        ExtractionTriggerKind::Debounce,
        "2000-01-01T00:00:05Z",
    )
    .unwrap();
    queue::enqueue(
        &conn,
        "fast",
        &fast_path,
        ExtractionTriggerKind::Debounce,
        "2000-01-01T00:00:01Z",
    )
    .unwrap();

    let first = worker.claim_next_job().unwrap().unwrap();
    worker.process_job(&first).unwrap();
    let second = worker.claim_next_job().unwrap().unwrap();
    worker.process_job(&second).unwrap();

    assert_eq!(first.session_id, "fast");
    assert_eq!(second.session_id, "slow");
    assert_eq!(worker.claim_next_job().unwrap(), None);
    assert_eq!(
        job_status(&conn, first.id),
        ExtractionJobStatus::Done.as_str()
    );
    assert_eq!(
        job_status(&conn, second.id),
        ExtractionJobStatus::Done.as_str()
    );

    let calls = probe.recorded_calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].prompt.contains("Session: fast"));
    assert!(calls[1].prompt.contains("Session: slow"));
}

#[test]
fn process_job_should_advance_cursor_and_infer_once_per_window_on_success() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let conversation_path =
        seed_conversation_file(dir.path(), "s1", conversation_with_cursor("s1", 0, 12));
    let slm = StubSlm::with_results([
        Ok("{\"facts\":[]}"),
        Ok("{\"facts\":[]}"),
        Ok("{\"facts\":[]}"),
        Ok("{\"facts\":[]}"),
    ]);
    let probe = slm.clone();
    let worker = worker_with_stub(&conn, slm);

    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        ExtractionTriggerKind::Manual,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let processed = worker.claim_next_job().unwrap().unwrap();
    worker.process_job(&processed).unwrap();
    let parsed = format::parse(&dir.path().join(slash_to_platform(&conversation_path))).unwrap();

    assert_eq!(processed.session_id, "s1");
    assert_eq!(parsed.frontmatter.last_extracted_turn, 12);
    assert!(parsed.frontmatter.last_extracted_at.is_some());
    assert_eq!(
        job_status(&conn, processed.id),
        ExtractionJobStatus::Done.as_str()
    );

    let calls = probe.recorded_calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].alias, "phi-3.5-mini");
    assert_eq!(calls[0].max_tokens, 128);
    assert!(calls[0]
        .prompt
        .contains("New turns to extract from (turns 1..5):"));
    assert!(calls[1]
        .prompt
        .contains("New turns to extract from (turns 6..10):"));
    assert!(calls[2]
        .prompt
        .contains("New turns to extract from (turns 11..12):"));
}

#[test]
fn process_job_should_leave_cursor_unchanged_and_retry_on_parse_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let conversation_path =
        seed_conversation_file(dir.path(), "s1", conversation_with_cursor("s1", 0, 2));
    let worker = worker_with_stub(&conn, StubSlm::with_results([Ok("not json at all")]));

    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        ExtractionTriggerKind::Manual,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let claimed = worker.claim_next_job().unwrap().unwrap();
    let error = worker.process_job(&claimed).unwrap_err();
    let parsed = format::parse(&dir.path().join(slash_to_platform(&conversation_path))).unwrap();
    let queue_row: (i64, String) = conn
        .query_row(
            "SELECT attempts, status FROM extraction_queue WHERE session_id = 's1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert!(matches!(error, WorkerError::Slm(SlmError::Parse { .. })));
    assert_eq!(parsed.frontmatter.last_extracted_turn, 0);
    assert_eq!(parsed.frontmatter.last_extracted_at, None);
    assert_eq!(queue_row.0, 1);
    assert_eq!(queue_row.1, ExtractionJobStatus::Pending.as_str());
}

#[test]
fn process_job_should_flush_session_close_context_without_advancing_cursor() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let conversation_path =
        seed_conversation_file(dir.path(), "s1", conversation_with_cursor("s1", 2, 2));
    let slm = StubSlm::with_results([Ok("{\"facts\":[]}"), Ok("{\"facts\":[]}")]);
    let probe = slm.clone();
    let worker = worker_with_stub(&conn, slm);

    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        ExtractionTriggerKind::SessionClose,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let processed = worker.claim_next_job().unwrap().unwrap();
    worker.process_job(&processed).unwrap();
    let parsed = format::parse(&dir.path().join(slash_to_platform(&conversation_path))).unwrap();

    assert_eq!(parsed.frontmatter.last_extracted_turn, 2);
    assert_eq!(
        job_status(&conn, processed.id),
        ExtractionJobStatus::Done.as_str()
    );

    let calls = probe.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0]
        .prompt
        .contains("New turns to extract from (turns none):"));
    assert!(calls[0].prompt.contains("[turn 1, user"));
    assert!(calls[0].prompt.contains("[turn 2, assistant"));
}

#[test]
fn run_once_should_sleep_and_return_false_when_queue_is_empty() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let worker = worker_with_stub(&conn, StubSlm::empty());

    assert!(!worker.run_once().unwrap());
}

#[test]
fn process_job_persists_cursor_before_done_transition() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let conversation_path =
        seed_conversation_file(dir.path(), "s1", conversation_with_cursor("s1", 0, 2));
    let worker = worker_with_writer(
        &conn,
        StubSlm::with_results([Ok("{\"facts\":[]}")]),
        StaleDoneWriter,
    );

    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        ExtractionTriggerKind::Manual,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let error = worker.process_next_job().unwrap_err();
    let parsed = format::parse(&dir.path().join(slash_to_platform(&conversation_path))).unwrap();
    let queue_row: (i64, String) = conn
        .query_row(
            "SELECT attempts, status FROM extraction_queue WHERE session_id = 's1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert!(matches!(
        error,
        WorkerError::Queue(queue::ExtractionQueueError::StaleLease { .. })
    ));
    assert_eq!(parsed.frontmatter.last_extracted_turn, 2);
    assert!(parsed.frontmatter.last_extracted_at.is_some());
    assert_eq!(queue_row.0, 99);
    assert_eq!(queue_row.1, ExtractionJobStatus::Running.as_str());
}

#[test]
fn session_close_reclaim_replays_same_turn_slice_without_duplicate_fact_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let conversation_path =
        seed_conversation_file(dir.path(), "s1", conversation_with_cursor("s1", 0, 2));

    let slm_output = r#"{"facts":[{"kind":"preference","about":"programming-language","strength":"high","summary":"Matt prefers Rust"}]}"#;
    let slm = StubSlm::with_results([Ok(slm_output), Ok(slm_output), Ok(slm_output)]);
    let probe = slm.clone();
    let worker = worker_with_writer(&conn, slm, FailFirstDoneAfterResolvingWrite::default());

    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        ExtractionTriggerKind::SessionClose,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let first = worker.claim_next_job().unwrap().unwrap();
    let first_error = worker.process_job(&first).unwrap_err();
    assert!(matches!(
        first_error,
        WorkerError::Queue(queue::ExtractionQueueError::StaleLease { .. })
    ));

    let after_first_run =
        format::parse(&dir.path().join(slash_to_platform(&conversation_path))).unwrap();
    assert_eq!(after_first_run.frontmatter.last_extracted_turn, 2);
    assert!(after_first_run.frontmatter.last_extracted_at.is_some());
    assert_eq!(
        job_attempts_and_status(&conn, first.id),
        (1, "running".to_string())
    );

    let extracted_files = extracted_markdown_files(&dir.path().join("extracted"));
    assert_eq!(extracted_files.len(), 1);
    ingest::run(&conn, extracted_files[0].to_str().unwrap(), false).unwrap();

    let heads_after_first_ingest: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE type = 'preference' AND superseded_by IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(heads_after_first_ingest, 1);

    conn.execute(
        "UPDATE extraction_queue SET scheduled_for = '2000-01-01T00:00:00Z' WHERE id = ?1",
        [first.id],
    )
    .unwrap();

    let reclaimed = worker.claim_next_job().unwrap().unwrap();
    assert_eq!(reclaimed.id, first.id);
    assert_eq!(reclaimed.attempts, 2);
    worker.process_job(&reclaimed).unwrap();

    let after_replay =
        format::parse(&dir.path().join(slash_to_platform(&conversation_path))).unwrap();
    assert_eq!(after_replay.frontmatter.last_extracted_turn, 2);
    assert!(after_replay.frontmatter.last_extracted_at.is_some());
    assert_eq!(
        job_status(&conn, reclaimed.id),
        ExtractionJobStatus::Done.as_str()
    );

    let extracted_files_after_replay = extracted_markdown_files(&dir.path().join("extracted"));
    assert_eq!(extracted_files_after_replay.len(), 1);
    let heads_after_replay: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE type = 'preference' AND superseded_by IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(heads_after_replay, 1);

    let calls = probe.recorded_calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[0]
        .prompt
        .contains("New turns to extract from (turns 1..2):"));
    // Keep the proof honest to the shipped worker: after the cursor is persisted, lease
    // recovery replays the same conversational slice via the session_close context-only path,
    // not as fresh new turns. The dedup claim here is only that this replay does not create
    // an extra fact file after the first write has already been ingested.
    assert!(calls[1]
        .prompt
        .contains("New turns to extract from (turns none):"));
    assert!(calls[1].prompt.contains("[turn 1, user"));
    assert!(calls[1].prompt.contains("[turn 2, assistant"));
}

// ── spec item 9.4 — pre-cursor-write crash: lease expiry + dedup backstop ────
//
// The *post*-cursor-write crash path is proven by
// `session_close_reclaim_replays_same_turn_slice_without_duplicate_fact_files`
// (cursor already at 2, replay uses session_close context-only window).
// This test covers the complementary path: crash happens *before*
// `persist_cursor_update`, so cursor stays at 0.  On replay the worker
// recomputes the same ordinal window [1..2] and dedup must prevent a
// duplicate fact file.
//
// Note: dedup correctness (cosine policy, multi-head disambiguation) is
// deferred to 7.*.  This test proves only that (a) the stale running row is
// re-eligibilised by lease expiry, (b) replay targets the same window, and
// (c) the write path's dedup check prevents a second file from appearing on
// disk.

#[test]
fn precursor_crash_replay_via_lease_expiry_contains_duplicate_via_dedup() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let conversation_path =
        seed_conversation_file(dir.path(), "s1", conversation_with_cursor("s1", 0, 2));
    let slm_output = r#"{"facts":[{"kind":"preference","about":"programming-language","strength":"high","summary":"Matt prefers Rust"}]}"#;

    // ── step 1: enqueue and claim the job ────────────────────────────────────
    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        ExtractionTriggerKind::Manual,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();
    let initial_claim = queue::dequeue(&conn).unwrap().unwrap();
    assert_eq!(initial_claim.session_id, "s1");

    // ── step 2: simulate partial execution ───────────────────────────────────
    // Write the fact for window [1..2] but do NOT advance the cursor.
    // In the real crash scenario, `persist_cursor_update` is called only
    // after *all* windows succeed; a crash during write_window leaves
    // cursor at 0 on disk.
    let first_window = WindowedTurns {
        new_turns: conversation_with_cursor("s1", 0, 2).turns,
        lookback_turns: Vec::new(),
        context_only: false,
    };
    let slm_response = parse_response(slm_output).unwrap();
    ResolvingFactWriter
        .write_window(&conn, &initial_claim, &first_window, &slm_response)
        .unwrap();

    for file in extracted_markdown_files(&dir.path().join("extracted")) {
        ingest::run(&conn, file.to_str().unwrap(), false).unwrap();
    }
    assert_eq!(
        extracted_markdown_files(&dir.path().join("extracted")).len(),
        1,
        "exactly one fact file after partial first run"
    );

    // ── step 3: simulate crash ───────────────────────────────────────────────
    // The job row stays `running`.  Set scheduled_for to an ancient timestamp
    // so the lease-expiry guard in `dequeue` considers it expired.
    conn.execute(
        "UPDATE extraction_queue
         SET status = 'running', scheduled_for = '2000-01-01T00:00:00Z'
         WHERE id = ?1",
        [initial_claim.id],
    )
    .unwrap();

    let parsed_before_replay =
        format::parse(&dir.path().join(slash_to_platform(&conversation_path))).unwrap();
    assert_eq!(
        parsed_before_replay.frontmatter.last_extracted_turn,
        0,
        "cursor must still be 0 — crash happened before persist_cursor_update"
    );

    // ── step 4: replay via lease expiry ──────────────────────────────────────
    // process_next_job → claim_next_job → dequeue → recover_expired_leases
    // resets the row to pending (attempts 0→1) → job is claimed and processed.
    let slm = StubSlm::with_results([Ok(slm_output)]);
    let probe = slm.clone();
    let worker = worker_with_writer(&conn, slm, ResolvingFactWriter);
    let replayed = worker.process_next_job().unwrap().unwrap();

    // (a) Stale running job was re-eligibilised and completed.
    assert_eq!(replayed.session_id, "s1");
    assert_eq!(replayed.attempts, 1, "lease-expiry increments attempts");
    assert_eq!(
        job_status(&conn, replayed.id),
        ExtractionJobStatus::Done.as_str()
    );

    // (b) Replay targeted the same ordinal window: cursor=0 → new turns [1..2].
    let calls = probe.recorded_calls();
    assert_eq!(calls.len(), 1, "exactly one SLM call on replay");
    assert!(
        calls[0].prompt.contains("New turns to extract from (turns 1..2):"),
        "replay window must cover the same ordinal range as the original job \
         (cursor unchanged at 0); prompt: {}",
        calls[0].prompt
    );

    // (c) Dedup contained the duplicate: still exactly 1 fact file on disk.
    assert_eq!(
        extracted_markdown_files(&dir.path().join("extracted")).len(),
        1,
        "dedup must prevent a duplicate fact file on replay"
    );

    // (d) Cursor now advanced to 2 — replay completed successfully.
    let parsed_after_replay =
        format::parse(&dir.path().join(slash_to_platform(&conversation_path))).unwrap();
    assert_eq!(parsed_after_replay.frontmatter.last_extracted_turn, 2);
}

fn open_worker_db_at(path: &Path) -> Connection {
    let conn = db::open(path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active'
         WHERE id = 1",
        [path.parent().unwrap().display().to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('extraction.enabled', 'true')",
        [],
    )
    .unwrap();
    conn
}

fn worker_with_stub<'db>(
    conn: &'db Connection,
    slm: StubSlm,
) -> Worker<'db, StubSlm, PendingFactWriter> {
    Worker::new(conn, slm, PendingFactWriter)
        .unwrap()
        .with_limits(Duration::from_millis(1), 128)
}

fn worker_with_writer<'db, W: FactWriter>(
    conn: &'db Connection,
    slm: StubSlm,
    writer: W,
) -> Worker<'db, StubSlm, W> {
    Worker::new(conn, slm, writer)
        .unwrap()
        .with_limits(Duration::from_millis(1), 128)
}

fn seed_conversation_file(root: &Path, session_id: &str, conversation: ConversationFile) -> String {
    let relative = Path::new("conversations")
        .join("2026-05-03")
        .join(format!("{session_id}.md"));
    let absolute = root.join(&relative);
    fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    fs::write(&absolute, format::render(&conversation)).unwrap();
    relative.to_string_lossy().replace('\\', "/")
}

fn conversation_with_cursor(session_id: &str, cursor: i64, last: i64) -> ConversationFile {
    ConversationFile {
        frontmatter: ConversationFrontmatter {
            file_type: "conversation".to_string(),
            session_id: session_id.to_string(),
            date: "2026-05-03".to_string(),
            started_at: "2026-05-03T10:00:00Z".to_string(),
            status: ConversationStatus::Open,
            closed_at: None,
            last_extracted_at: None,
            last_extracted_turn: cursor,
        },
        turns: (1..=last)
            .map(|ordinal| Turn {
                ordinal,
                role: if ordinal % 2 == 0 {
                    TurnRole::Assistant
                } else {
                    TurnRole::User
                },
                timestamp: format!("2026-05-03T10:00:{ordinal:02}Z"),
                content: format!("turn {ordinal}"),
                metadata: None,
            })
            .collect(),
    }
}

fn job_status(conn: &Connection, job_id: i64) -> String {
    conn.query_row(
        "SELECT status FROM extraction_queue WHERE id = ?1",
        [job_id],
        |row| row.get(0),
    )
    .unwrap()
}

fn job_attempts_and_status(conn: &Connection, job_id: i64) -> (i64, String) {
    conn.query_row(
        "SELECT attempts, status FROM extraction_queue WHERE id = ?1",
        [job_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

fn extracted_markdown_files(root: &Path) -> Vec<PathBuf> {
    fn visit(dir: &Path, files: &mut Vec<PathBuf>) {
        if !dir.exists() {
            return;
        }

        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
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

fn slash_to_platform(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

// ── spec item 9.3 — no partial cursor advance when a later window fails ────

#[test]
fn process_job_should_not_partially_advance_cursor_when_later_window_fails() {
    // 10 turns with window_turns=5 → 2 windows: [1..5] then [6..10].
    // Window 1 gets valid JSON; window 2 gets garbage. persist_cursor_update is
    // only called after all windows succeed, so the cursor must stay at 0.
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let conversation_path =
        seed_conversation_file(dir.path(), "s1", conversation_with_cursor("s1", 0, 10));

    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        ExtractionTriggerKind::Manual,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let worker = worker_with_stub(
        &conn,
        StubSlm::with_results([Ok("{\"facts\":[]}"), Ok("not json — window 2 explodes")]),
    );
    let result = worker.process_next_job();
    assert!(
        result.is_err(),
        "process_job must fail when a window parse errors"
    );

    let parsed = format::parse(&dir.path().join(slash_to_platform(&conversation_path))).unwrap();
    assert_eq!(
        parsed.frontmatter.last_extracted_turn, 0,
        "cursor must not partially advance — window-1 success does not count when window-2 fails"
    );
    assert_eq!(
        parsed.frontmatter.last_extracted_at, None,
        "last_extracted_at must remain unset when any window fails"
    );
}

// ── spec item 5.2 gap — acceptance bar (currently unimplemented) ───────────

#[test]
fn claim_next_job_returns_none_when_extraction_is_disabled() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);

    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('extraction.enabled', '0')",
        [],
    )
    .unwrap();

    let conversation_path =
        seed_conversation_file(dir.path(), "s1", conversation_with_cursor("s1", 0, 2));
    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        ExtractionTriggerKind::Manual,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let worker = worker_with_stub(&conn, StubSlm::empty());
    let result = worker.claim_next_job().unwrap();

    assert!(
        result.is_none(),
        "claim_next_job must return None when extraction.enabled=false — \
          currently returns Some (guard not yet implemented in claim_next_job)"
    );
}

#[test]
fn claim_next_job_returns_none_when_runtime_is_disabled() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);

    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('extraction.enabled', 'true')",
        [],
    )
    .unwrap();

    let conversation_path =
        seed_conversation_file(dir.path(), "s1", conversation_with_cursor("s1", 0, 2));
    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        ExtractionTriggerKind::Manual,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let worker = worker_with_stub(&conn, StubSlm::runtime_disabled());

    assert_eq!(worker.claim_next_job().unwrap(), None);
}

#[derive(Debug, Clone)]
struct StubSlm {
    results: Arc<Mutex<VecDeque<Result<String, SlmError>>>>,
    calls: Arc<Mutex<Vec<InferCall>>>,
    runtime_disabled: bool,
}

impl StubSlm {
    fn empty() -> Self {
        Self {
            results: Arc::new(Mutex::new(VecDeque::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
            runtime_disabled: false,
        }
    }

    fn with_results<const N: usize>(results: [Result<&str, SlmError>; N]) -> Self {
        Self {
            results: Arc::new(Mutex::new(
                results
                    .into_iter()
                    .map(|result| result.map(str::to_string))
                    .collect(),
            )),
            calls: Arc::new(Mutex::new(Vec::new())),
            runtime_disabled: false,
        }
    }

    fn runtime_disabled() -> Self {
        Self {
            runtime_disabled: true,
            ..Self::empty()
        }
    }

    fn recorded_calls(&self) -> Vec<InferCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl SlmClient for StubSlm {
    fn infer(&self, alias: &str, prompt: &str, max_tokens: usize) -> Result<String, SlmError> {
        self.calls.lock().unwrap().push(InferCall {
            alias: alias.to_string(),
            prompt: prompt.to_string(),
            max_tokens,
        });
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok("{\"facts\":[]}".to_string()))
    }

    fn is_runtime_disabled(&self) -> bool {
        self.runtime_disabled
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InferCall {
    alias: String,
    prompt: String,
    max_tokens: usize,
}

#[derive(Debug, Default)]
struct StaleDoneWriter;

impl FactWriter for StaleDoneWriter {
    fn write_window(
        &self,
        _db: &Connection,
        _job: &ExtractionJob,
        _window: &WindowedTurns,
        _response: &ExtractionResponse,
    ) -> Result<(), WorkerError> {
        Ok(())
    }

    fn before_mark_done(&self, db: &Connection, job: &ExtractionJob) -> Result<(), WorkerError> {
        db.execute(
            "UPDATE extraction_queue SET attempts = 99 WHERE id = ?1",
            [job.id],
        )
        .unwrap();
        Ok(())
    }
}

#[derive(Debug)]
struct FailFirstDoneAfterResolvingWrite {
    inner: ResolvingFactWriter,
    fail_once: Mutex<bool>,
}

impl Default for FailFirstDoneAfterResolvingWrite {
    fn default() -> Self {
        Self {
            inner: ResolvingFactWriter,
            fail_once: Mutex::new(true),
        }
    }
}

impl FactWriter for FailFirstDoneAfterResolvingWrite {
    fn write_window(
        &self,
        db: &Connection,
        job: &ExtractionJob,
        window: &WindowedTurns,
        response: &ExtractionResponse,
    ) -> Result<(), WorkerError> {
        self.inner.write_window(db, job, window, response)
    }

    fn before_mark_done(&self, db: &Connection, job: &ExtractionJob) -> Result<(), WorkerError> {
        let mut fail_once = self.fail_once.lock().unwrap();
        if *fail_once {
            db.execute(
                "UPDATE extraction_queue SET attempts = attempts + 1 WHERE id = ?1",
                [job.id],
            )
            .unwrap();
            *fail_once = false;
        }
        Ok(())
    }
}
