use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use rusqlite::Connection;
use thiserror::Error;

use crate::core::conversation::{
    format,
    queue::{self, ExtractionQueueError},
    slm::{parse_response, LazySlmRunner, SlmError},
    turn_writer,
};
use crate::core::db;
use crate::core::types::{
    ConversationFile, ExtractionJob, ExtractionResponse, ExtractionTriggerKind, Turn, WindowedTurns,
};

pub const DEFAULT_EXTRACTION_MAX_TOKENS: usize = 2048;
pub const DEFAULT_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(1);

const DEFAULT_WINDOW_TURNS: usize = 5;
const DEFAULT_MODEL_ALIAS: &str = "phi-3.5-mini";
const MAX_RECORDED_PARSE_OUTPUT: usize = 240;
const EXTRACTION_SYSTEM_PROMPT: &str = concat!(
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
    "Return: {\"facts\": [...]}. Empty array if nothing durable."
);

pub trait SlmClient {
    fn infer(&self, alias: &str, prompt: &str, max_tokens: usize) -> Result<String, SlmError>;
}

impl SlmClient for LazySlmRunner {
    fn infer(&self, alias: &str, prompt: &str, max_tokens: usize) -> Result<String, SlmError> {
        LazySlmRunner::infer(self, alias, prompt, max_tokens)
    }
}

#[derive(Debug, Default)]
pub struct PendingFactWriter;

pub trait FactWriter {
    fn write_window(
        &self,
        _db: &Connection,
        _job: &ExtractionJob,
        _window: &WindowedTurns,
        _response: &ExtractionResponse,
    ) -> Result<(), WorkerError> {
        Ok(())
    }

    fn before_mark_done(&self, _db: &Connection, _job: &ExtractionJob) -> Result<(), WorkerError> {
        Ok(())
    }
}

impl FactWriter for PendingFactWriter {}

#[derive(Debug)]
pub struct Worker<'db, S = LazySlmRunner, W = PendingFactWriter> {
    db: &'db Connection,
    slm: S,
    _vault_writer: W,
    model_alias: String,
    window_turns: usize,
    poll_interval: Duration,
    max_tokens: usize,
}

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("queue error: {0}")]
    Queue(#[from] ExtractionQueueError),

    #[error("conversation format error: {0}")]
    Format(#[from] format::ConversationFormatError),

    #[error("SLM error: {0}")]
    Slm(#[from] SlmError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("worker config error: {message}")]
    Config { message: String },
}

impl<'db, S, W> Worker<'db, S, W>
where
    S: SlmClient,
    W: FactWriter,
{
    pub fn new(db: &'db Connection, slm: S, vault_writer: W) -> Result<Self, WorkerError> {
        let model_alias =
            db::read_config_value_or(db, "extraction.model_alias", DEFAULT_MODEL_ALIAS).map_err(
                |error| WorkerError::Config {
                    message: error.to_string(),
                },
            )?;
        let window_turns = parse_usize_config(db, "extraction.window_turns", DEFAULT_WINDOW_TURNS)?;

        Ok(Self {
            db,
            slm,
            _vault_writer: vault_writer,
            model_alias,
            window_turns,
            poll_interval: DEFAULT_WORKER_POLL_INTERVAL,
            max_tokens: DEFAULT_EXTRACTION_MAX_TOKENS,
        })
    }

    pub fn with_limits(mut self, poll_interval: Duration, max_tokens: usize) -> Self {
        self.poll_interval = poll_interval;
        self.max_tokens = max_tokens;
        self
    }

    pub fn claim_next_job(&self) -> Result<Option<ExtractionJob>, WorkerError> {
        queue::dequeue(self.db).map_err(WorkerError::from)
    }

    pub fn poll_once(&self) -> Result<bool, WorkerError> {
        Ok(self.process_next_job()?.is_some())
    }

    pub fn run_once(&self) -> Result<bool, WorkerError> {
        let processed = self.poll_once()?;
        if !processed {
            self.sleep_until_next_poll();
        }
        Ok(processed)
    }

    pub fn run_forever(&self) -> Result<(), WorkerError> {
        loop {
            let _ = self.run_once()?;
        }
    }

    pub fn sleep_until_next_poll(&self) {
        thread::sleep(self.poll_interval);
    }

    pub fn process_next_job(&self) -> Result<Option<ExtractionJob>, WorkerError> {
        let Some(job) = self.claim_next_job()? else {
            return Ok(None);
        };
        self.process_job(&job)?;
        Ok(Some(job))
    }

    pub fn plan_windows_for_job(
        &self,
        job: &ExtractionJob,
    ) -> Result<Vec<WindowedTurns>, WorkerError> {
        let conversation = format::parse(&self.resolve_conversation_path(&job.conversation_path)?)?;
        Ok(self.compute_windows(&conversation, job.trigger_kind))
    }

    pub fn compute_windows(
        &self,
        conversation: &ConversationFile,
        trigger_kind: ExtractionTriggerKind,
    ) -> Vec<WindowedTurns> {
        compute_windows(conversation, trigger_kind, self.window_turns)
    }

    pub fn build_prompt(&self, session_id: &str, window: &WindowedTurns) -> String {
        build_prompt(session_id, window)
    }

    pub fn infer_window(
        &self,
        session_id: &str,
        window: &WindowedTurns,
    ) -> Result<ExtractionResponse, WorkerError> {
        let prompt = self.build_prompt(session_id, window);
        let raw = self
            .slm
            .infer(&self.model_alias, &prompt, self.max_tokens)
            .map_err(WorkerError::from)?;
        parse_response(&raw).map_err(WorkerError::from)
    }

    pub fn infer_and_parse_window(
        &self,
        job: &ExtractionJob,
        window: &WindowedTurns,
    ) -> Result<ExtractionResponse, WorkerError> {
        let prompt = self.build_prompt(&job.session_id, window);
        let raw = self
            .slm
            .infer(&self.model_alias, &prompt, self.max_tokens)
            .map_err(WorkerError::from)?;

        match parse_response(&raw) {
            Ok(response) => Ok(response),
            Err(error) => {
                self.record_parse_failure(job, &raw, &error)?;
                Err(WorkerError::Slm(error))
            }
        }
    }

    pub fn process_job(&self, job: &ExtractionJob) -> Result<(), WorkerError> {
        let conversation_path = self.resolve_conversation_path(&job.conversation_path)?;
        let mut conversation = format::parse(&conversation_path)?;
        let windows = self.compute_windows(&conversation, job.trigger_kind);

        for window in &windows {
            let response = match self.infer_and_parse_window(job, window) {
                Ok(response) => response,
                Err(error) if is_parse_failure(&error) => return Err(error),
                Err(error) => {
                    self.record_job_failure(job, &error)?;
                    return Err(error);
                }
            };

            if let Err(error) = self
                ._vault_writer
                .write_window(self.db, job, window, &response)
            {
                self.record_job_failure(job, &error)?;
                return Err(error);
            }
        }

        if !windows.is_empty() {
            let extracted_at = queue::current_timestamp(self.db)?;
            persist_cursor_update(
                &conversation_path,
                &mut conversation,
                &windows,
                &extracted_at,
            )?;
        }

        self._vault_writer.before_mark_done(self.db, job)?;
        queue::mark_done(self.db, job.id, job.attempts).map_err(WorkerError::from)
    }

    pub fn record_parse_failure(
        &self,
        job: &ExtractionJob,
        raw_output: &str,
        error: &SlmError,
    ) -> Result<(), WorkerError> {
        let last_error = format!(
            "{}; raw output: {}",
            error,
            truncate_for_last_error(raw_output)
        );
        queue::mark_failed(self.db, job.id, job.attempts, &last_error).map_err(WorkerError::from)
    }

    fn record_job_failure(
        &self,
        job: &ExtractionJob,
        error: &WorkerError,
    ) -> Result<(), WorkerError> {
        queue::mark_failed(self.db, job.id, job.attempts, &error.to_string())
            .map_err(WorkerError::from)
    }

    fn resolve_conversation_path(&self, conversation_path: &str) -> Result<PathBuf, WorkerError> {
        let candidate = Path::new(conversation_path);
        if candidate.is_absolute() {
            return Ok(candidate.to_path_buf());
        }

        let memory_root =
            turn_writer::resolve_memory_root(self.db).map_err(|error| WorkerError::Config {
                message: error.to_string(),
            })?;
        Ok(memory_root
            .root_path
            .join(slash_path_to_platform(conversation_path)))
    }
}

pub fn compute_windows(
    conversation: &ConversationFile,
    trigger_kind: ExtractionTriggerKind,
    window_turns: usize,
) -> Vec<WindowedTurns> {
    let window_turns = window_turns.max(1);
    let cursor = conversation.frontmatter.last_extracted_turn;
    let new_turns = conversation
        .turns
        .iter()
        .filter(|turn| turn.ordinal > cursor)
        .cloned()
        .collect::<Vec<_>>();

    if new_turns.is_empty() {
        if trigger_kind != ExtractionTriggerKind::SessionClose || conversation.turns.is_empty() {
            return Vec::new();
        }

        return vec![WindowedTurns {
            new_turns: Vec::new(),
            lookback_turns: tail_turns(&conversation.turns, window_turns),
            context_only: true,
        }];
    }

    if new_turns.len() >= window_turns {
        return new_turns
            .chunks(window_turns)
            .map(|chunk| WindowedTurns {
                new_turns: chunk.to_vec(),
                lookback_turns: Vec::new(),
                context_only: false,
            })
            .collect();
    }

    let lookback_needed = window_turns.saturating_sub(new_turns.len());
    let lookback_turns = conversation
        .turns
        .iter()
        .filter(|turn| turn.ordinal <= cursor)
        .rev()
        .take(lookback_needed)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();

    vec![WindowedTurns {
        new_turns,
        lookback_turns,
        context_only: false,
    }]
}

pub fn build_prompt(session_id: &str, window: &WindowedTurns) -> String {
    format!(
        "SYSTEM:\n{EXTRACTION_SYSTEM_PROMPT}\n\nUSER:\nSession: {session_id}\nNew turns to extract from ({}):\n{}\nLookback context (do not extract from these — for reference only):\n{}",
        ordinal_range_label(&window.new_turns),
        render_prompt_turns(&window.new_turns),
        render_prompt_turns(&window.lookback_turns)
    )
}

fn ordinal_range_label(turns: &[Turn]) -> String {
    match (turns.first(), turns.last()) {
        (Some(first), Some(last)) => format!("turns {}..{}", first.ordinal, last.ordinal),
        _ => "turns none".to_string(),
    }
}

fn render_prompt_turns(turns: &[Turn]) -> String {
    if turns.is_empty() {
        return "  (none)".to_string();
    }

    turns
        .iter()
        .map(render_prompt_turn)
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_prompt_turn(turn: &Turn) -> String {
    let mut rendered = format!(
        "  [turn {}, {}, {}]\n    {}",
        turn.ordinal,
        turn.role.as_str(),
        turn.timestamp,
        indent_multiline(&turn.content)
    );
    if let Some(metadata) = &turn.metadata {
        rendered.push_str("\n    metadata: ");
        rendered.push_str(&metadata.to_string());
    }
    rendered
}

fn indent_multiline(content: &str) -> String {
    content.replace('\n', "\n    ")
}

fn tail_turns(turns: &[Turn], count: usize) -> Vec<Turn> {
    turns
        .iter()
        .rev()
        .take(count)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn truncate_for_last_error(raw_output: &str) -> String {
    let single_line = raw_output.trim().replace('\r', "\\r").replace('\n', "\\n");
    if single_line.chars().count() <= MAX_RECORDED_PARSE_OUTPUT {
        return single_line;
    }

    let truncated = single_line
        .chars()
        .take(MAX_RECORDED_PARSE_OUTPUT)
        .collect::<String>();
    format!("{truncated}…")
}

fn is_parse_failure(error: &WorkerError) -> bool {
    matches!(error, WorkerError::Slm(SlmError::Parse { .. }))
}

fn persist_cursor_update(
    path: &Path,
    conversation: &mut ConversationFile,
    windows: &[WindowedTurns],
    extracted_at: &str,
) -> Result<(), WorkerError> {
    if let Some(last_new_ordinal) = windows
        .iter()
        .filter_map(WindowedTurns::last_new_ordinal)
        .max()
    {
        conversation.frontmatter.last_extracted_turn = last_new_ordinal;
    }
    conversation.frontmatter.last_extracted_at = Some(extracted_at.to_owned());
    fs::write(path, format::render(conversation))?;
    Ok(())
}

fn parse_usize_config(db: &Connection, key: &str, default: usize) -> Result<usize, WorkerError> {
    let raw = db::read_config_value_or(db, key, &default.to_string()).map_err(|error| {
        WorkerError::Config {
            message: error.to_string(),
        }
    })?;
    raw.parse::<usize>().map_err(|_| WorkerError::Config {
        message: format!("invalid {key} value: {raw}"),
    })
}

fn slash_path_to_platform(path: &str) -> PathBuf {
    if std::path::MAIN_SEPARATOR == '/' {
        return PathBuf::from(path);
    }
    PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}
