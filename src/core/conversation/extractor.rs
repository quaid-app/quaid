//! Extraction worker that drains the queue, slices each conversation into
//! lookback-aware turn windows, prompts the SLM for structured facts, and
//! hands the validated facts to a pluggable `FactWriter` (either the
//! resolving writer that performs full supersede semantics or a no-op
//! writer for tests). Cursor state in the conversation frontmatter is
//! advanced after each window's facts are written, so a mid-job failure or
//! lease-expiry rerun resumes from the first incomplete window instead of
//! re-emitting facts for windows that already succeeded. The cursor write
//! holds the same per-session mutex + flock as `turn_writer::append_turn`
//! and rewrites only the cursor frontmatter lines, so concurrently appended
//! turns survive.
//!
//! See also: `super::queue` for the SQLite-backed job table this worker
//! consumes, `super::slm` for the SLM runtime, `super::supersede` for the
//! resolve-and-write pipeline that turns raw facts into pages, and
//! `super::format` for the on-disk conversation parser used to materialise
//! windows.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use rusqlite::Connection;
use thiserror::Error;

use crate::core::conversation::{
    format,
    queue::{self, ExtractionQueueError},
    slm::{parse_response, LazySlmRunner, SlmError},
    supersede::{FactResolutionError, ResolvingFactWriter},
    turn_writer,
};
use crate::core::db;
use crate::core::types::{
    ConversationFile, ExtractionJob, ExtractionResponse, ExtractionTriggerKind, Turn, WindowedTurns,
};

/// Default `max_tokens` budget for a single SLM extraction inference call.
pub const DEFAULT_EXTRACTION_MAX_TOKENS: usize = 2048;
/// Default sleep interval between worker polls when the queue is empty.
pub const DEFAULT_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// How long the loaded SLM may sit idle before the worker drops it to
/// reclaim its multi-GB resident footprint; the next job transparently
/// reloads it.
pub const DEFAULT_SLM_IDLE_UNLOAD_TTL: Duration = Duration::from_secs(300);

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
    "You are not a chat partner. Return exactly one JSON object and nothing else.\n",
    "Skip ephemeral content (greetings, clarifications, transient task state).\n",
    "Skip facts you already extracted in prior windows.\n",
    "Facts must be supported by the windowed turns; do not infer beyond what was said.\n\n",
    "Schema (one fact per object):\n",
    "  decision     { kind, chose, rationale?, summary }\n",
    "  preference   { kind, about, strength, summary }\n",
    "  fact         { kind, about, summary }\n",
    "  action_item  { kind, who?, what, status, due?, summary }\n\n",
    "Required: kind, summary, plus the type-specific structured field(s).\n",
    "Allowed outputs only:\n",
    "  {\"facts\":[]}\n",
    "  {\"facts\":[{\"kind\":\"preference\",\"about\":\"beverage\",\"strength\":\"high\",\"summary\":\"The user prefers coffee to tea.\"}]}\n",
    "Return: {\"facts\": [...]}. Empty array if nothing durable."
);

/// SLM seam consumed by [`Worker`]; production code uses
/// [`super::slm::LazySlmRunner`] and tests plug in stubs to drive the
/// worker without loading model weights.
pub trait SlmClient {
    /// Run inference under the given model alias with the supplied prompt
    /// and token budget, returning the raw model output.
    fn infer(&self, alias: &str, prompt: &str, max_tokens: usize) -> Result<String, SlmError>;

    /// Returns `true` when the SLM runtime is administratively disabled
    /// (e.g. the embedded-model build is running without an installed
    /// extraction model); the worker treats this like an empty queue.
    fn is_runtime_disabled(&self) -> bool {
        false
    }

    /// Hook invoked when the worker has been idle, giving a runtime that
    /// caches a loaded model the chance to drop it after `idle_ttl`.
    /// Defaults to a no-op for stub clients that hold no weights.
    fn unload_if_idle(&self, _idle_ttl: Duration) {}
}

impl SlmClient for LazySlmRunner {
    fn infer(&self, alias: &str, prompt: &str, max_tokens: usize) -> Result<String, SlmError> {
        LazySlmRunner::infer(self, alias, prompt, max_tokens)
    }

    fn is_runtime_disabled(&self) -> bool {
        LazySlmRunner::is_runtime_disabled(self)
    }

    fn unload_if_idle(&self, idle_ttl: Duration) {
        LazySlmRunner::unload_if_idle(self, idle_ttl);
    }
}

/// `FactWriter` that intentionally discards extracted facts; used in
/// dry-run paths and tests that exercise the queue and prompt flow
/// without touching the vault.
#[derive(Debug, Default)]
pub struct PendingFactWriter;

/// Strategy plugged into [`Worker`] to decide what happens with extracted
/// facts after the SLM call succeeds.
pub trait FactWriter {
    /// Persist (or otherwise act on) the facts produced for one window of
    /// a job; the default implementation is a no-op.
    fn write_window(
        &self,
        _db: &Connection,
        _job: &ExtractionJob,
        _window: &WindowedTurns,
        _response: &ExtractionResponse,
    ) -> Result<(), WorkerError> {
        Ok(())
    }

    /// Hook invoked after every window of a job has been written, just
    /// before the worker marks the queue row done; defaults to a no-op.
    fn before_mark_done(&self, _db: &Connection, _job: &ExtractionJob) -> Result<(), WorkerError> {
        Ok(())
    }
}

impl FactWriter for PendingFactWriter {}

impl FactWriter for ResolvingFactWriter {
    fn write_window(
        &self,
        db: &Connection,
        job: &ExtractionJob,
        window: &WindowedTurns,
        response: &ExtractionResponse,
    ) -> Result<(), WorkerError> {
        let context =
            crate::core::conversation::supersede::context_for_job_window(db, job, window)?;
        for fact in &response.facts {
            crate::core::conversation::supersede::resolve_and_write_fact_in_context(
                fact, db, &context,
            )?;
        }
        Ok(())
    }
}

/// Extraction worker that owns a borrowed DB connection plus a pluggable
/// SLM client and fact writer, and exposes the polling, windowing,
/// inference, and persistence steps as a small set of methods.
#[derive(Debug)]
pub struct Worker<'db, S = LazySlmRunner, W = ResolvingFactWriter> {
    db: &'db Connection,
    slm: S,
    _vault_writer: W,
    model_alias: String,
    window_turns: usize,
    poll_interval: Duration,
    max_tokens: usize,
}

/// Errors returned from the extraction worker's polling and processing loop.
#[derive(Debug, Error)]
pub enum WorkerError {
    /// An operation against the extraction queue table failed.
    #[error("queue error: {0}")]
    Queue(#[from] ExtractionQueueError),

    /// Parsing or rendering the on-disk conversation file failed.
    #[error("conversation format error: {0}")]
    Format(#[from] format::ConversationFormatError),

    /// The SLM runner surfaced an error during inference or parsing.
    #[error("SLM error: {0}")]
    Slm(#[from] SlmError),

    /// Filesystem I/O failed (typically when updating the cursor frontmatter).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Required runtime configuration is missing or unreadable.
    #[error("worker config error: {message}")]
    Config {
        /// Human-readable explanation of the config failure.
        message: String,
    },

    /// The fact-write seam refused or failed on a produced fact.
    #[error("fact resolution error: {0}")]
    FactResolution(#[from] FactResolutionError),

    /// Acquiring the per-session locks (or resolving the memory root) failed
    /// while persisting the extraction cursor.
    #[error("turn writer error: {0}")]
    TurnWrite(#[from] turn_writer::TurnWriteError),
}

impl<'db, S, W> Worker<'db, S, W>
where
    S: SlmClient,
    W: FactWriter,
{
    /// Build a worker bound to a database connection, SLM client, and
    /// fact-writer strategy, reading model alias and window size from
    /// the on-disk configuration.
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

    /// Override the worker's poll interval and inference token budget;
    /// used by tests and the CLI to tune behavior without touching config.
    pub fn with_limits(mut self, poll_interval: Duration, max_tokens: usize) -> Self {
        self.poll_interval = poll_interval;
        self.max_tokens = max_tokens;
        self
    }

    /// Atomically lease the next ready job from the queue, returning
    /// `None` when extraction is disabled, the SLM runtime is unavailable,
    /// or the queue has no due rows.
    pub fn claim_next_job(&self) -> Result<Option<ExtractionJob>, WorkerError> {
        if !extraction_enabled(self.db)? || self.slm.is_runtime_disabled() {
            return Ok(None);
        }
        queue::dequeue(self.db).map_err(WorkerError::from)
    }

    /// Attempt to lease and fully process a single job, returning `true`
    /// when work was done and `false` when the queue had nothing ready.
    pub fn poll_once(&self) -> Result<bool, WorkerError> {
        Ok(self.process_next_job()?.is_some())
    }

    /// Like [`Self::poll_once`] but sleeps for the configured poll
    /// interval when no work is found, so callers can write a tight loop
    /// without busy-spinning.
    pub fn run_once(&self) -> Result<bool, WorkerError> {
        let processed = self.poll_once()?;
        if !processed {
            // Queue is quiet: give the SLM client a chance to drop a
            // cold, cached model before parking, so an idle MCP server
            // does not pin ~8 GB indefinitely.
            self.slm.unload_if_idle(DEFAULT_SLM_IDLE_UNLOAD_TTL);
            self.sleep_until_next_poll();
        }
        Ok(processed)
    }

    /// Drive the worker forever, calling [`Self::run_once`] in a loop
    /// until an unrecoverable error bubbles up.
    pub fn run_forever(&self) -> Result<(), WorkerError> {
        loop {
            let _ = self.run_once()?;
        }
    }

    /// Park the current thread for the worker's configured poll interval.
    pub fn sleep_until_next_poll(&self) {
        thread::sleep(self.poll_interval);
    }

    /// Lease the next job and run [`Self::process_job`] against it,
    /// returning the job on success or `None` if the queue was empty.
    pub fn process_next_job(&self) -> Result<Option<ExtractionJob>, WorkerError> {
        let Some(job) = self.claim_next_job()? else {
            return Ok(None);
        };
        self.process_job(&job)?;
        Ok(Some(job))
    }

    /// Parse the job's conversation file and compute the turn windows
    /// that would be sent to the SLM, without invoking the model; used by
    /// the CLI's dry-run path and by tests.
    pub fn plan_windows_for_job(
        &self,
        job: &ExtractionJob,
    ) -> Result<Vec<WindowedTurns>, WorkerError> {
        let conversation = format::parse(&self.resolve_conversation_path(&job.conversation_path)?)?;
        Ok(self.compute_windows(&conversation, job.trigger_kind))
    }

    /// Slice a parsed conversation into extraction windows using the
    /// worker's configured `window_turns`, applying trigger-specific
    /// lookback rules.
    pub fn compute_windows(
        &self,
        conversation: &ConversationFile,
        trigger_kind: ExtractionTriggerKind,
    ) -> Vec<WindowedTurns> {
        compute_windows(conversation, trigger_kind, self.window_turns)
    }

    /// Assemble the SLM prompt for a single window under the given session id.
    pub fn build_prompt(&self, session_id: &str, window: &WindowedTurns) -> String {
        build_prompt(session_id, window)
    }

    /// Build the prompt for a single window, invoke the SLM, and parse
    /// the JSON envelope into an [`ExtractionResponse`].
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

    /// Like [`Self::infer_window`] but tied to a specific job so a parse
    /// failure is recorded against the queue row's `last_error` before
    /// being surfaced.
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

    /// Run every window of a leased job through the SLM and the fact
    /// writer, advancing the conversation's extraction cursor after each
    /// window so a rerun skips windows whose facts were already written,
    /// and mark the queue row done; records `last_error` and surfaces the
    /// failure when any step fails.
    pub fn process_job(&self, job: &ExtractionJob) -> Result<(), WorkerError> {
        let conversation_path = self.resolve_conversation_path(&job.conversation_path)?;
        let conversation = format::parse(&conversation_path)?;
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

            // Per-window cursor advance: the facts for this window are on
            // disk, so persist the cursor now. Partial progress surviving a
            // later-window failure is deliberate — facts are already written
            // per window, and a rerun must not re-emit them.
            if let Err(error) = self.persist_window_cursor(job, &conversation_path, window) {
                self.record_job_failure(job, &error)?;
                return Err(error);
            }
        }

        self._vault_writer.before_mark_done(self.db, job)?;
        queue::mark_done(self.db, job.id, job.attempts).map_err(WorkerError::from)
    }

    /// Persist the extraction cursor for one completed window by rewriting
    /// only the cursor frontmatter lines of the day-file, under the same
    /// per-session mutex + flock that `turn_writer::append_turn` holds, so
    /// a turn appended while the SLM was running survives the rewrite.
    fn persist_window_cursor(
        &self,
        job: &ExtractionJob,
        conversation_path: &Path,
        window: &WindowedTurns,
    ) -> Result<(), WorkerError> {
        let extracted_at = queue::current_timestamp(self.db)?;
        let last_new_ordinal = window.last_new_ordinal();
        match self.session_lock_scope(&job.conversation_path)? {
            Some((root, namespace, session_id)) => {
                turn_writer::with_session_locks(&root, namespace.as_deref(), &session_id, || {
                    rewrite_cursor_frontmatter(conversation_path, last_new_ordinal, &extracted_at)
                })
            }
            // Absolute job paths cannot be mapped back to a session lock
            // scope; fall back to an unlocked (but still frontmatter-only)
            // rewrite.
            None => rewrite_cursor_frontmatter(conversation_path, last_new_ordinal, &extracted_at),
        }
    }

    /// Derive the `(memory root, namespace, session id)` lock scope for a
    /// job's vault-relative conversation path, or `None` when the path does
    /// not follow the canonical relative scheme (e.g. absolute test paths).
    fn session_lock_scope(
        &self,
        job_conversation_path: &str,
    ) -> Result<Option<(turn_writer::MemoryRoot, Option<String>, String)>, WorkerError> {
        let Ok(parsed) = format::parse_relative_conversation_path(job_conversation_path) else {
            return Ok(None);
        };
        let root =
            turn_writer::resolve_memory_root(self.db).map_err(|error| WorkerError::Config {
                message: error.to_string(),
            })?;
        Ok(Some((root, parsed.namespace, parsed.session_id)))
    }

    /// Persist a truncated copy of the offending raw SLM output to the
    /// queue row's `last_error` column so the failure can be inspected
    /// after the worker increments the attempt counter.
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

/// Slice a parsed conversation into extraction windows: chunk new turns
/// past the cursor into `window_turns`-sized batches, optionally fall back
/// to a trailing-context-only window on session close, and pad short
/// batches with lookback turns for context.
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

/// Render the SLM prompt for one extraction window, framing the new
/// turns as the extraction target and the lookback turns as
/// reference-only context.
///
/// The prompt is emitted in Phi-3's chat template
/// (`<|system|>…<|end|><|user|>…<|end|><|assistant|>`) so the
/// instruct-tuned model receives the role markers and the trailing
/// `<|assistant|>` generation cue it was fine-tuned on, instead of the
/// plain `SYSTEM:/USER:` framing that made it behave base-like. The
/// `<|system|>`, `<|user|>`, `<|end|>`, and `<|assistant|>` markers are
/// recognized as special tokens by the Phi-3 tokenizer (see
/// `crate::core::conversation::slm::SlmRunner::infer`, which encodes
/// with `add_special_tokens` enabled).
pub fn build_prompt(session_id: &str, window: &WindowedTurns) -> String {
    let user = format!(
        "Session: {session_id}\nNew turns to extract from ({}):\n{}\nLookback context (do not extract from these — for reference only):\n{}",
        ordinal_range_label(&window.new_turns),
        render_prompt_turns(&window.new_turns),
        render_prompt_turns(&window.lookback_turns)
    );
    render_phi3_chat_prompt(EXTRACTION_SYSTEM_PROMPT, &user)
}

/// Wrap a system+user message pair in the Phi-3 chat template the
/// instruct model was trained on. Kept separate from
/// [`build_prompt`] so the exact rendered string is covered by a
/// golden-prompt snapshot test.
pub fn render_phi3_chat_prompt(system: &str, user: &str) -> String {
    format!("<|system|>\n{system}<|end|>\n<|user|>\n{user}<|end|>\n<|assistant|>\n")
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

/// Rewrite only the `last_extracted_at` / `last_extracted_turn` frontmatter
/// lines of the day-file at `path`, re-reading the file fresh so turns
/// appended after the worker's pre-inference snapshot are preserved
/// byte-for-byte instead of being clobbered by a stale full-file render.
fn rewrite_cursor_frontmatter(
    path: &Path,
    last_new_ordinal: Option<i64>,
    extracted_at: &str,
) -> Result<(), WorkerError> {
    let raw = fs::read_to_string(path)?;
    let updated = update_cursor_frontmatter(&raw, last_new_ordinal, extracted_at)?;
    let mut file = fs::File::create(path)?;
    file.write_all(updated.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn update_cursor_frontmatter(
    raw: &str,
    last_new_ordinal: Option<i64>,
    extracted_at: &str,
) -> Result<String, WorkerError> {
    let mut out = String::with_capacity(raw.len() + 64);
    let mut in_frontmatter = false;
    let mut frontmatter_done = false;
    let mut wrote_extracted_at = false;
    // No ordinal to persist means the existing cursor line is kept as-is.
    let mut wrote_extracted_turn = last_new_ordinal.is_none();

    for line in raw.split_inclusive('\n') {
        if !frontmatter_done {
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if !in_frontmatter {
                if out.is_empty() && trimmed == "---" {
                    in_frontmatter = true;
                }
            } else if trimmed == "---" {
                // Closing boundary: insert any cursor lines the file did not
                // already carry (legacy files may lack `last_extracted_at`).
                if !wrote_extracted_at {
                    out.push_str("last_extracted_at: ");
                    out.push_str(extracted_at);
                    out.push('\n');
                }
                if !wrote_extracted_turn {
                    if let Some(ordinal) = last_new_ordinal {
                        out.push_str(&format!("last_extracted_turn: {ordinal}\n"));
                    }
                }
                in_frontmatter = false;
                frontmatter_done = true;
            } else if trimmed.starts_with("last_extracted_at:") {
                out.push_str("last_extracted_at: ");
                out.push_str(extracted_at);
                out.push('\n');
                wrote_extracted_at = true;
                continue;
            } else if trimmed.starts_with("last_extracted_turn:") {
                if let Some(ordinal) = last_new_ordinal {
                    out.push_str(&format!("last_extracted_turn: {ordinal}\n"));
                    wrote_extracted_turn = true;
                    continue;
                }
            }
        }
        out.push_str(line);
    }

    if !frontmatter_done {
        return Err(WorkerError::Format(
            format::ConversationFormatError::InvalidFrontmatter {
                message: "cursor update requires a closed frontmatter block".to_owned(),
            },
        ));
    }
    Ok(out)
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

fn extraction_enabled(db: &Connection) -> Result<bool, WorkerError> {
    let raw = db::read_config_value_or(db, "extraction.enabled", "false").map_err(|error| {
        WorkerError::Config {
            message: error.to_string(),
        }
    })?;

    match raw.as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(WorkerError::Config {
            message: format!("invalid extraction.enabled value: {other}"),
        }),
    }
}

fn slash_path_to_platform(path: &str) -> PathBuf {
    if std::path::MAIN_SEPARATOR == '/' {
        return PathBuf::from(path);
    }
    PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}
