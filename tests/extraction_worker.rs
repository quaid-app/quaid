use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quaid::core::conversation::{
    extractor::{FactWriter, PendingFactWriter, SlmClient, Worker, WorkerError},
    format, queue,
    slm::SlmError,
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

fn slash_to_platform(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

#[derive(Debug, Clone)]
struct StubSlm {
    results: Arc<Mutex<VecDeque<Result<String, SlmError>>>>,
    calls: Arc<Mutex<Vec<InferCall>>>,
}

impl StubSlm {
    fn empty() -> Self {
        Self {
            results: Arc::new(Mutex::new(VecDeque::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
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
