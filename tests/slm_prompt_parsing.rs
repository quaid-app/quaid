use std::collections::VecDeque;
use std::fs;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quaid::core::conversation::{
    extractor::{PendingFactWriter, SlmClient, Worker, WorkerError, DEFAULT_EXTRACTION_MAX_TOKENS},
    format, queue,
    slm::{parse_response, SlmError},
};
use quaid::core::db;
use quaid::core::types::{
    ConversationFile, ConversationFrontmatter, ConversationStatus, ExtractionJob,
    ExtractionJobStatus, PreferenceStrength, RawFact, Turn, TurnRole, WindowedTurns,
};
use rusqlite::Connection;

#[test]
fn build_prompt_should_match_foundation_contract_for_sparse_window() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let worker = worker_with_stub(&conn, StubSlm::empty());
    let window = WindowedTurns {
        new_turns: vec![
            turn(11, TurnRole::User, "We should standardize on Rust."),
            turn(
                12,
                TurnRole::Assistant,
                "Agreed, let's make Rust the default.",
            ),
        ],
        lookback_turns: vec![
            turn(8, TurnRole::User, "What language should we use?"),
            turn(
                9,
                TurnRole::Assistant,
                "Rust would keep the CLI local-first.",
            ),
            turn(10, TurnRole::User, "I care most about stability."),
        ],
        context_only: false,
    };

    let prompt = worker.build_prompt("session-42", &window);

    let expected = concat!(
        "SYSTEM:\n",
        "You extract durable facts from conversations. Output JSON only — no prose,\n",
        "no markdown fences. Each fact is one of four kinds:\n\n",
        "  decision     — a choice made between alternatives\n",
        "  preference   — a stable inclination (\"X likes/wants/prefers Y\")\n",
        "  fact         — a claim about the world or a person (\"X is/has/works-at Y\")\n",
        "  action_item  — a commitment to do something with a clear actor\n\n",
        "Skip ephemeral content (greetings, clarifications, transient task state).\n",
        "Skip facts you already extracted in prior windows.\n",
        "Facts must be supported by the windowed turns; do not infer beyond what was said.\n\n",
        "Schema (one fact per object):\n",
        "  decision     { kind, chose, rationale?, summary }\n",
        "  preference   { kind, about, strength, summary }\n",
        "  fact         { kind, about, summary }\n",
        "  action_item  { kind, who?, what, status, due?, summary }\n\n",
        "Required: kind, summary, plus the type-specific structured field(s).\n",
        "Return: {\"facts\": [...]}. Empty array if nothing durable.\n\n",
        "USER:\n",
        "Session: session-42\n",
        "New turns to extract from (turns 11..12):\n",
        "  [turn 11, user, 2026-05-03T10:00:11Z]\n",
        "    We should standardize on Rust.\n",
        "  [turn 12, assistant, 2026-05-03T10:00:12Z]\n",
        "    Agreed, let's make Rust the default.\n",
        "Lookback context (do not extract from these — for reference only):\n",
        "  [turn 8, user, 2026-05-03T10:00:08Z]\n",
        "    What language should we use?\n",
        "  [turn 9, assistant, 2026-05-03T10:00:09Z]\n",
        "    Rust would keep the CLI local-first.\n",
        "  [turn 10, user, 2026-05-03T10:00:10Z]\n",
        "    I care most about stability."
    );

    assert_eq!(prompt, expected);
}

#[test]
fn parse_response_should_accept_bare_json() {
    let parsed = parse_response(
        r#"{"facts":[{"kind":"preference","about":"programming-language","strength":"high","summary":"Matt prefers Rust"}]}"#,
    )
    .unwrap();

    assert_eq!(
        parsed.facts,
        vec![RawFact::Preference {
            about: "programming-language".to_string(),
            strength: Some(PreferenceStrength::High),
            summary: "Matt prefers Rust".to_string(),
        }]
    );
    assert!(parsed.validation_errors.is_empty());
}

#[test]
fn parse_response_should_accept_fenced_json() {
    let parsed = parse_response("```json\n{\"facts\":[]}\n```").unwrap();

    assert!(parsed.facts.is_empty());
    assert!(parsed.validation_errors.is_empty());
}

#[test]
fn parse_response_should_reject_leading_commentary() {
    let error = parse_response("Sure, here you go:\n{\"facts\":[]}").unwrap_err();

    assert!(matches!(error, SlmError::Parse { .. }));
}

#[test]
fn parse_response_should_reject_unknown_kind_without_dropping_other_facts() {
    let parsed = parse_response(
        r#"{"facts":[
            {"kind":"opinion","about":"tooling","summary":"Unknown kind"},
            {"kind":"fact","about":"product","summary":"Quaid is local-first"}
        ]}"#,
    )
    .unwrap();

    assert_eq!(
        parsed.facts,
        vec![RawFact::Fact {
            about: "product".to_string(),
            summary: "Quaid is local-first".to_string(),
        }]
    );
    assert_eq!(parsed.validation_errors.len(), 1);
    assert_eq!(parsed.validation_errors[0].kind.as_deref(), Some("opinion"));
}

#[test]
fn parse_response_should_reject_missing_required_field_without_dropping_other_facts() {
    let parsed = parse_response(
        r#"{"facts":[
            {"kind":"decision","summary":"Missing chose"},
            {"kind":"fact","about":"repo","summary":"Repo is named quaid"}
        ]}"#,
    )
    .unwrap();

    assert_eq!(parsed.facts.len(), 1);
    assert_eq!(parsed.validation_errors.len(), 1);
    assert!(parsed.validation_errors[0]
        .message
        .contains("missing field `chose`"));
}

#[test]
fn parse_response_should_partially_accept_mixed_validity_facts() {
    let parsed = parse_response(
        r#"{"facts":[
            {"kind":"fact","about":"product","summary":"Quaid is local-first"},
            {"kind":"preference","strength":"high","summary":"Missing about"},
            {"kind":"action_item","what":"ship the parser","status":"open","summary":"Fry will land the parser batch"}
        ]}"#,
    )
    .unwrap();

    assert_eq!(parsed.facts.len(), 2);
    assert_eq!(parsed.validation_errors.len(), 1);
    assert_eq!(
        parsed.validation_errors[0].kind.as_deref(),
        Some("preference")
    );
}

#[test]
fn worker_should_increment_attempts_and_mark_failed_after_parse_retries() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let conversation_path = seed_conversation_file(dir.path(), sample_conversation());

    queue::enqueue(
        &conn,
        "s1",
        &conversation_path,
        quaid::core::types::ExtractionTriggerKind::Manual,
        "2000-01-01T00:00:00Z",
    )
    .unwrap();

    let worker = worker_with_stub(
        &conn,
        StubSlm::with_outputs(["not json at all", "still not json", "definitely not json"]),
    );

    for expected_attempts in 1..=3 {
        let job = queue::dequeue(&conn).unwrap().unwrap();
        let window = worker.plan_windows_for_job(&job).unwrap().remove(0);
        let error = worker.infer_and_parse_window(&job, &window).unwrap_err();
        assert!(matches!(error, WorkerError::Slm(SlmError::Parse { .. })));

        let (attempts, status, last_error): (i64, String, String) = conn
            .query_row(
                "SELECT attempts, status, last_error FROM extraction_queue WHERE id = ?1",
                [job.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(attempts, expected_attempts);
        if expected_attempts < 3 {
            assert_eq!(status, ExtractionJobStatus::Pending.as_str());
        } else {
            assert_eq!(status, ExtractionJobStatus::Failed.as_str());
        }
        assert!(last_error.contains("raw output:"));
    }
}

#[test]
fn worker_infer_window_uses_default_model_alias_and_token_budget() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_worker_db_at(&db_path);
    let slm = StubSlm::empty();
    let probe = slm.clone();
    let worker = Worker::new(&conn, slm, PendingFactWriter).unwrap();
    let window = WindowedTurns {
        new_turns: vec![turn(11, TurnRole::User, "Capture the durable decision.")],
        lookback_turns: vec![turn(
            10,
            TurnRole::Assistant,
            "We should keep it local-first.",
        )],
        context_only: false,
    };

    let response = worker.infer_window("session-42", &window).unwrap();

    assert!(response.facts.is_empty());
    assert!(response.validation_errors.is_empty());

    let calls = probe.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].alias, "phi-3.5-mini");
    assert_eq!(calls[0].max_tokens, DEFAULT_EXTRACTION_MAX_TOKENS);
    assert!(calls[0].prompt.contains("Session: session-42"));
}

fn open_worker_db_at(path: &std::path::Path) -> Connection {
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

fn seed_conversation_file(root: &std::path::Path, conversation: ConversationFile) -> String {
    let relative = std::path::Path::new("conversations")
        .join("2026-05-03")
        .join("s1.md");
    let absolute = root.join(&relative);
    fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    fs::write(&absolute, format::render(&conversation)).unwrap();
    relative.to_string_lossy().replace('\\', "/")
}

fn sample_conversation() -> ConversationFile {
    ConversationFile {
        frontmatter: ConversationFrontmatter {
            file_type: "conversation".to_string(),
            session_id: "s1".to_string(),
            date: "2026-05-03".to_string(),
            started_at: "2026-05-03T10:00:00Z".to_string(),
            status: ConversationStatus::Open,
            closed_at: None,
            last_extracted_at: None,
            last_extracted_turn: 0,
        },
        turns: vec![
            turn(1, TurnRole::User, "We should standardize on Rust."),
            turn(
                2,
                TurnRole::Assistant,
                "I'll capture that as a durable preference.",
            ),
        ],
    }
}

fn turn(ordinal: i64, role: TurnRole, content: &str) -> Turn {
    Turn {
        ordinal,
        role,
        timestamp: format!("2026-05-03T10:00:{ordinal:02}Z"),
        content: content.to_string(),
        metadata: None,
    }
}

#[derive(Debug, Clone)]
struct StubSlm {
    outputs: Arc<Mutex<VecDeque<String>>>,
    calls: Arc<Mutex<Vec<InferCall>>>,
}

impl StubSlm {
    fn empty() -> Self {
        Self {
            outputs: Arc::new(Mutex::new(VecDeque::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_outputs<const N: usize>(outputs: [&str; N]) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(
                outputs.into_iter().map(str::to_string).collect(),
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
        Ok(self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "{\"facts\":[]}".to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InferCall {
    alias: String,
    prompt: String,
    max_tokens: usize,
}

fn _assert_job_shape(_job: &ExtractionJob) -> Result<(), io::Error> {
    Ok(())
}
