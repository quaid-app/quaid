//! Shared data types used across the `core` library: pages, links, tags,
//! search results, knowledge gaps, conversation turns, extraction facts, and
//! the structured-error envelopes that pass through the MCP wire.
//!
//! See also: `core::db` for the persistence layer that reads and writes these
//! types, `core::search` for the retrieval surface that produces `SearchResult`
//! values, and `core::conversation` for the extraction pipeline that emits
//! `RawFact` and `ExtractionJob` rows.

// Types defined ahead of consumers (db.rs, search.rs, etc.) — remove when wired.
#![allow(dead_code)]

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use thiserror::Error;

// ── Page ──────────────────────────────────────────────────────

/// Free-form YAML/JSON frontmatter map stored alongside a page's prose body.
pub type Frontmatter = JsonMap<String, JsonValue>;

/// Looks up a frontmatter entry by key, returning the raw JSON value if present.
pub fn frontmatter_get<'a>(frontmatter: &'a Frontmatter, key: &str) -> Option<&'a JsonValue> {
    frontmatter.get(key)
}

/// Looks up a frontmatter entry and narrows it to a borrowed string slice.
pub fn frontmatter_get_str<'a>(frontmatter: &'a Frontmatter, key: &str) -> Option<&'a str> {
    frontmatter_get(frontmatter, key)?.as_str()
}

/// Looks up a frontmatter entry and clones it into an owned `String`.
pub fn frontmatter_get_string(frontmatter: &Frontmatter, key: &str) -> Option<String> {
    frontmatter_get_str(frontmatter, key).map(str::to_owned)
}

/// Inserts a string-valued frontmatter entry, accepting any types that convert into `String`.
pub fn frontmatter_insert_string(
    frontmatter: &mut Frontmatter,
    key: impl Into<String>,
    value: impl Into<String>,
) {
    frontmatter.insert(key.into(), JsonValue::String(value.into()));
}

/// Builds a `Frontmatter` map from `(key, value)` string pairs — convenience for tests and fixtures.
pub fn string_frontmatter(entries: impl IntoIterator<Item = (String, String)>) -> Frontmatter {
    entries
        .into_iter()
        .map(|(key, value)| (key, JsonValue::String(value)))
        .collect()
}

/// Core knowledge page — the unit of storage in a Quaid database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    /// Stable per-namespace slug identifying the page.
    pub slug: String,
    /// Persistent UUID assigned on first write; survives slug renames.
    pub uuid: String,
    /// Semantic page kind (e.g. `person`, `decision`, `note`).
    #[serde(rename = "type")]
    pub page_type: String,
    /// Optional pointer to the page that supersedes this one, by integer page id.
    pub superseded_by: Option<i64>,
    /// Human-facing title rendered at the top of the page.
    pub title: String,
    /// One-paragraph summary used for retrieval previews.
    pub summary: String,
    /// Current synthesized "truth" section of the page body.
    pub compiled_truth: String,
    /// Append-only timeline section of the page body, in markdown.
    pub timeline: String,
    /// Raw structured frontmatter parsed from the source markdown.
    pub frontmatter: Frontmatter,
    /// Memory-palace wing the page is filed under.
    pub wing: String,
    /// Memory-palace room within `wing` (may be empty).
    pub room: String,
    /// Monotonic version counter used for optimistic concurrency.
    pub version: i64,
    /// RFC 3339 timestamp of first write.
    pub created_at: String,
    /// RFC 3339 timestamp of most recent write to any field.
    pub updated_at: String,
    /// RFC 3339 timestamp of the most recent change to `compiled_truth`.
    pub truth_updated_at: String,
    /// RFC 3339 timestamp of the most recent change to `timeline`.
    pub timeline_updated_at: String,
}

// ── Link ──────────────────────────────────────────────────────

/// Typed temporal cross-reference between two pages.
///
/// Uses slugs (`from_slug`, `to_slug`) as the application-layer identity.
/// The DB layer resolves slugs to integer page IDs (`from_page_id`, `to_page_id`)
/// on insert and reverses the join on read. Callers never see raw page IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    /// `None` before the link is persisted; `Some(id)` after insert.
    pub id: Option<i64>,
    /// Slug of the source page (the link's "from" endpoint).
    pub from_slug: String,
    /// Slug of the target page (the link's "to" endpoint).
    pub to_slug: String,
    /// Free-form relationship label (e.g. `works_with`, `supersedes`).
    pub relationship: String,
    /// Optional note stored alongside the link (schema column: `context`).
    pub context: String,
    /// RFC 3339 inclusive start of temporal validity, or `None` for unbounded past.
    pub valid_from: Option<String>,
    /// RFC 3339 exclusive end of temporal validity, or `None` if still valid.
    pub valid_until: Option<String>,
    /// RFC 3339 timestamp the link row was inserted.
    pub created_at: String,
}

// ── Tag ───────────────────────────────────────────────────────

/// A single tag attached to a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    /// Integer id of the page this tag belongs to.
    pub page_id: i64,
    /// The tag string itself, stored verbatim.
    pub tag: String,
}

// ── TimelineEntry ─────────────────────────────────────────────

/// A structured timeline row for a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    /// Row id within `timeline_entries`.
    pub id: i64,
    /// Integer id of the parent page.
    pub page_id: i64,
    /// Date the event occurred (free-form: `2024`, `2024-05`, `2024-05-12`, …).
    pub date: String,
    /// Short source-or-category label rendered before the summary.
    pub source: String,
    /// One-line description shown in the timeline body.
    pub summary: String,
    /// Optional longer detail block expanded after the summary.
    pub detail: String,
    /// RFC 3339 timestamp the row was inserted.
    pub created_at: String,
}

// ── SearchResult ──────────────────────────────────────────────

/// A single result from FTS5, vector, or hybrid search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Slug of the page that produced the hit.
    pub slug: String,
    /// Page title at the time of retrieval.
    pub title: String,
    /// Short page summary used as the retrieval preview.
    pub summary: String,
    /// Relevance score; sign and scale depend on the merge strategy.
    pub score: f64,
    /// Memory-palace wing of the page, exposed for downstream filtering.
    pub wing: String,
}

// ── Chunk ───────────────────────────────────────────────────────

/// A derived embedding/search chunk from a page section or timeline entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Slug of the page this chunk was derived from.
    pub page_slug: String,
    /// Slash-joined heading path locating the chunk within the page.
    pub heading_path: String,
    /// The chunk's prose body, ready for embedding.
    pub content: String,
    /// Stable hash of `content` used to detect unchanged chunks across reruns.
    pub content_hash: String,
    /// Approximate token count of `content`.
    pub token_count: usize,
    /// Chunk category (e.g. `truth`, `timeline_entry`).
    pub chunk_type: String,
}

// ── KnowledgeGap ──────────────────────────────────────────────

/// An unanswered query detected by the memory engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGap {
    /// Row id within `knowledge_gaps`.
    pub id: i64,
    /// Optional id of the page the query was meant to land on.
    pub page_id: Option<i64>,
    /// Stable hash of the normalized query string, used for dedup.
    pub query_hash: String,
    /// Free-form context recorded alongside the gap.
    pub context: String,
    /// Optional retrieval confidence at the time the gap was logged.
    pub confidence_score: Option<f64>,
    /// Sensitivity label controlling who can see the gap.
    pub sensitivity: String,
    /// RFC 3339 timestamp the gap was resolved, or `None` while still open.
    pub resolved_at: Option<String>,
    /// Slug of the page that resolved the gap, if any.
    pub resolved_by_slug: Option<String>,
    /// RFC 3339 timestamp the gap was first detected.
    pub detected_at: String,
}

// ── IngestRecord ──────────────────────────────────────────────

/// An entry in the idempotency audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRecord {
    /// Row id within `ingest_records`.
    pub id: i64,
    /// Caller-supplied idempotency key used to dedupe repeated ingests.
    pub ingest_key: String,
    /// Category of the source (e.g. `vault_sync`, `manual`).
    pub source_type: String,
    /// Free-form reference to the source artifact (path, URL, etc.).
    pub source_ref: String,
    /// JSON-encoded list of slugs that this ingest touched.
    pub pages_updated: String,
    /// Short prose summary of what the ingest accomplished.
    pub summary: String,
    /// RFC 3339 timestamp the ingest finished.
    pub completed_at: String,
}

// ── SearchMergeStrategy ───────────────────────────────────────

/// How hybrid search merges FTS5 and vector result sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchMergeStrategy {
    /// Take the set-union of hits, scored by the better of the two ranks.
    SetUnion,
    /// Merge via Reciprocal Rank Fusion across the two ranked lists.
    Rrf,
}

impl SearchMergeStrategy {
    /// Parses a config string into a strategy, defaulting to `SetUnion` on unrecognized input.
    pub fn from_config(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "rrf" => Self::Rrf,
            _ => Self::SetUnion,
        }
    }
}

// ── Conversation ────────────────────────────────────────────────

/// Current on-disk conversation format version. Version 1 (legacy)
/// wrote turn content verbatim, so pasted turn-boundary markers or
/// metadata fences could forge structure on re-parse. Version 2
/// escapes those markers inside turn content at render time and
/// decodes them at parse time; files without a `format_version`
/// frontmatter key are read as legacy version 1.
pub const CONVERSATION_FORMAT_VERSION: i64 = 2;

/// Legacy conversation format version assumed for files (and
/// serialized snapshots) that predate the `format_version` marker.
pub const LEGACY_CONVERSATION_FORMAT_VERSION: i64 = 1;

fn legacy_conversation_format_version() -> i64 {
    LEGACY_CONVERSATION_FORMAT_VERSION
}

/// Frontmatter block at the top of an on-disk conversation file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationFrontmatter {
    /// File-kind marker, always `conversation` for these files.
    #[serde(rename = "type")]
    pub file_type: String,
    /// On-disk format version this file was written at. Governs
    /// whether turn content uses version-2 marker escaping; absent in
    /// legacy files and legacy serialized snapshots (treated as 1).
    #[serde(default = "legacy_conversation_format_version")]
    pub format_version: i64,
    /// Stable session identifier shared by every turn in the file.
    pub session_id: String,
    /// Calendar date the session was filed under, in `YYYY-MM-DD` form.
    pub date: String,
    /// RFC 3339 timestamp of the first turn in the session.
    pub started_at: String,
    /// Whether the session is still being written to or has been closed.
    pub status: ConversationStatus,
    /// RFC 3339 timestamp of session close, or `None` while still open.
    pub closed_at: Option<String>,
    /// RFC 3339 timestamp of the most recent extraction run, if any.
    pub last_extracted_at: Option<String>,
    /// Highest turn ordinal already covered by extraction.
    pub last_extracted_turn: i64,
}

/// A full parsed conversation file: header plus all recorded turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationFile {
    /// Parsed frontmatter header.
    pub frontmatter: ConversationFrontmatter,
    /// Turns in their on-disk order.
    pub turns: Vec<Turn>,
}

/// A single turn (one message) inside a conversation file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    /// 1-based position of the turn within its session.
    pub ordinal: i64,
    /// Role that produced the turn (user, assistant, system, tool).
    pub role: TurnRole,
    /// RFC 3339 timestamp the turn was recorded.
    pub timestamp: String,
    /// Raw turn body, typically markdown.
    pub content: String,
    /// Optional structured metadata blob attached to the turn.
    pub metadata: Option<serde_json::Value>,
}

/// Speaker role attached to a conversation turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnRole {
    /// Message authored by the human user.
    User,
    /// Message authored by the assistant.
    Assistant,
    /// Out-of-band system or harness message.
    System,
    /// Tool-call result injected into the conversation.
    Tool,
}

impl TurnRole {
    /// Returns the canonical lowercase string form used in storage and JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
        }
    }
}

impl fmt::Display for TurnRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TurnRole {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            "tool" => Ok(Self::Tool),
            other => Err(format!("invalid turn role: {other}")),
        }
    }
}

/// Lifecycle state of a conversation file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    /// Still accepting new turns.
    Open,
    /// Session has been closed; no further turns will be appended.
    Closed,
}

impl ConversationStatus {
    /// Returns the canonical lowercase string form used in storage and JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

impl fmt::Display for ConversationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ConversationStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            other => Err(format!("invalid conversation status: {other}")),
        }
    }
}

/// Result of appending a single turn to a conversation file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnWriteResult {
    /// Stable per-turn identifier returned to the caller.
    pub turn_id: String,
    /// Final ordinal assigned to the written turn.
    pub ordinal: i64,
    /// Vault-relative path of the conversation file that was updated.
    pub conversation_path: String,
}

/// Result of closing a conversation session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseSessionResult {
    /// RFC 3339 timestamp recorded as the close time.
    pub closed_at: String,
    /// Vault-relative path of the conversation file that was closed.
    pub conversation_path: String,
    /// `true` if this call transitioned the session from open to closed.
    pub newly_closed: bool,
}

/// A queued unit of work for the conversation extraction pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionJob {
    /// Row id within `extraction_queue`.
    pub id: i64,
    /// Session id whose conversation file the job is bound to.
    pub session_id: String,
    /// Vault-relative path of the conversation file to process.
    pub conversation_path: String,
    /// What triggered the job to be enqueued.
    pub trigger_kind: ExtractionTriggerKind,
    /// RFC 3339 timestamp the job was enqueued.
    pub enqueued_at: String,
    /// RFC 3339 timestamp at or after which the job may run.
    pub scheduled_for: String,
    /// Number of execution attempts so far.
    pub attempts: i64,
    /// Last error message if the most recent attempt failed.
    pub last_error: Option<String>,
    /// Current lifecycle status of the job.
    pub status: ExtractionJobStatus,
}

/// SLM extraction output for a single batch of turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExtractionResponse {
    /// Facts the model emitted for this batch.
    pub facts: Vec<RawFact>,
    /// Per-fact validation errors collected during parsing; never serialized.
    #[serde(skip, default)]
    pub validation_errors: Vec<ExtractionFactValidationError>,
}

/// A single per-fact validation failure surfaced during extraction parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionFactValidationError {
    /// Position of the offending fact within the model's emitted list.
    pub index: usize,
    /// Declared fact kind, if the kind field parsed.
    pub kind: Option<String>,
    /// Human-readable description of what failed validation.
    pub message: String,
}

/// Qualitative strength label attached to a `Preference` fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceStrength {
    /// Mild or situational preference.
    Low,
    /// Default-but-overridable preference.
    Medium,
    /// Strongly held preference.
    High,
}

impl PreferenceStrength {
    /// Returns the canonical lowercase string form used in storage and JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Lifecycle state of an `ActionItem` fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionItemState {
    /// Outstanding; not yet done or cancelled.
    Open,
    /// Completed.
    Done,
    /// Withdrawn before completion.
    Cancelled,
}

impl ActionItemState {
    /// Returns the canonical lowercase string form used in storage and JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }
}

/// One typed fact emitted by the extraction pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RawFact {
    /// A choice that was made; pivot key is `chose`.
    Decision {
        /// What was chosen.
        chose: String,
        /// Optional reason recorded alongside the choice.
        rationale: Option<String>,
        /// Prose summary of the decision.
        summary: String,
    },
    /// A user preference about something; pivot key is `about`.
    Preference {
        /// Subject the preference is about.
        about: String,
        /// Optional strength label.
        strength: Option<PreferenceStrength>,
        /// Prose summary of the preference.
        summary: String,
    },
    /// A persistent factual statement; pivot key is `about`.
    Fact {
        /// Subject the fact concerns.
        about: String,
        /// Prose summary of the fact.
        summary: String,
    },
    /// A task to be done; pivot key is `what`.
    ActionItem {
        /// Optional owner of the action item.
        who: Option<String>,
        /// Short description of the work to do.
        what: String,
        /// Current lifecycle state.
        status: ActionItemState,
        /// Optional due date in `YYYY-MM-DD` form.
        due: Option<String>,
        /// Prose summary of the action item.
        summary: String,
    },
}

impl RawFact {
    /// Returns the kind tag as a static string.
    ///
    /// Used for page `type` field on write and for FTS head-lookup queries.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Decision { .. } => "decision",
            Self::Preference { .. } => "preference",
            Self::Fact { .. } => "fact",
            Self::ActionItem { .. } => "action_item",
        }
    }

    /// Returns the value of the structured type key for this fact.
    ///
    /// The type key is the resolution pivot:
    /// - `decision`    → `chose`
    /// - `preference`  → `about`
    /// - `fact`        → `about`
    /// - `action_item` → `what`
    pub fn type_key(&self) -> &str {
        match self {
            Self::Decision { chose, .. } => chose.as_str(),
            Self::Preference { about, .. } => about.as_str(),
            Self::Fact { about, .. } => about.as_str(),
            Self::ActionItem { what, .. } => what.as_str(),
        }
    }

    /// Returns the name of the type-key field (not its value).
    ///
    /// Used for JSON extraction in head-lookup queries:
    /// `json_extract(frontmatter, '$.<type_key_field>') = ?`
    pub fn type_key_field(&self) -> &'static str {
        match self {
            Self::Decision { .. } => "chose",
            Self::Preference { .. } => "about",
            Self::Fact { .. } => "about",
            Self::ActionItem { .. } => "what",
        }
    }

    /// Returns the plural directory segment used in extracted-fact vault paths.
    ///
    /// Path scheme: `<vault>/extracted/<type_plural>/<slug>.md`  
    /// (or `<vault>/<namespace>/extracted/<type_plural>/<slug>.md` with a namespace)
    pub fn type_plural(&self) -> &'static str {
        match self {
            Self::Decision { .. } => "decisions",
            Self::Preference { .. } => "preferences",
            Self::Fact { .. } => "facts",
            Self::ActionItem { .. } => "action-items",
        }
    }

    /// Returns the prose summary written by the SLM.
    pub fn summary(&self) -> &str {
        match self {
            Self::Decision { summary, .. } => summary.as_str(),
            Self::Preference { summary, .. } => summary.as_str(),
            Self::Fact { summary, .. } => summary.as_str(),
            Self::ActionItem { summary, .. } => summary.as_str(),
        }
    }
}

/// A bounded extraction window: new turns to extract plus prior turns for context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowedTurns {
    /// Turns whose facts have not yet been extracted.
    pub new_turns: Vec<Turn>,
    /// Already-extracted turns included only as context for the SLM.
    pub lookback_turns: Vec<Turn>,
    /// `true` when the window contains only context and nothing to extract.
    pub context_only: bool,
}

impl WindowedTurns {
    /// Returns the ordinal of the first new turn in the window, if any.
    pub fn first_new_ordinal(&self) -> Option<i64> {
        self.new_turns.first().map(|turn| turn.ordinal)
    }

    /// Returns the ordinal of the last new turn in the window, if any.
    pub fn last_new_ordinal(&self) -> Option<i64> {
        self.new_turns.last().map(|turn| turn.ordinal)
    }
}

/// What caused an extraction job to be enqueued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionTriggerKind {
    /// Quiet-period debounce timer fired after a burst of turns.
    Debounce,
    /// Session was closed and a final extraction pass is owed.
    SessionClose,
    /// Operator forced a run via the CLI or MCP surface.
    Manual,
}

impl ExtractionTriggerKind {
    /// Returns the canonical lowercase string form used in storage and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debounce => "debounce",
            Self::SessionClose => "session_close",
            Self::Manual => "manual",
        }
    }
}

impl fmt::Display for ExtractionTriggerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExtractionTriggerKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "debounce" => Ok(Self::Debounce),
            "session_close" => Ok(Self::SessionClose),
            "manual" => Ok(Self::Manual),
            other => Err(format!("invalid extraction trigger kind: {other}")),
        }
    }
}

/// Lifecycle status of an `ExtractionJob` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionJobStatus {
    /// Awaiting a worker lease.
    Pending,
    /// Currently leased and executing.
    Running,
    /// Completed successfully.
    Done,
    /// Exhausted retries without success.
    Failed,
}

impl ExtractionJobStatus {
    /// Returns the canonical lowercase string form used in storage and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for ExtractionJobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExtractionJobStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(format!("invalid extraction job status: {other}")),
        }
    }
}

// ── Errors ────────────────────────────────────────────────────

/// Optimistic concurrency control error.
///
/// The `ConflictError: ` prefix is the canonical spelling for every
/// conflict surfaced over JSON-RPC (`-32009`); see `src/mcp/errors.rs`.
#[derive(Debug, Error)]
pub enum OccError {
    /// The page was updated by another writer since the caller last read it.
    #[error("ConflictError: page updated elsewhere (current version: {current_version})")]
    Conflict {
        /// Version currently persisted in the database.
        current_version: i64,
    },
}

/// Errors from FTS5 or hybrid search operations.
#[derive(Debug, Error)]
pub enum SearchError {
    /// Underlying SQLite or FTS5 error.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A slug prefix matched more than one page and could not be resolved.
    #[error("ambiguous slug: {slug} ({candidates})")]
    Ambiguous {
        /// The slug fragment the caller supplied.
        slug: String,
        /// Comma-separated list of candidate full slugs.
        candidates: String,
    },

    /// Search failed for a non-SQLite reason; `message` carries the detail.
    #[error("search failed: {message}")]
    Internal {
        /// Human-readable failure description.
        message: String,
    },
}

/// Errors from text embedding and vector inference operations.
#[derive(Debug, Error)]
pub enum InferenceError {
    /// Caller passed an empty string to the embedder.
    #[error("input text is empty")]
    EmptyInput,

    /// Embedding or vector search failed; `message` carries the detail.
    #[error("inference failed: {message}")]
    Internal {
        /// Human-readable failure description.
        message: String,
    },
}

/// Database-layer errors surfaced by `src/core/`.
#[derive(Debug, Error)]
pub enum DbError {
    /// No page exists for the requested slug.
    #[error("page not found: {slug}")]
    NotFound {
        /// Slug the caller asked for.
        slug: String,
    },

    /// No page exists for the requested vault-relative path.
    #[error("path not found: {path}")]
    PathNotFound {
        /// Path the caller asked for.
        path: String,
    },

    /// Optimistic-concurrency conflict propagated up from the write path.
    #[error("OCC conflict: {0}")]
    Occ(#[from] OccError),

    /// Underlying SQLite error.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Schema mismatch or migration failure; `message` carries the detail.
    #[error("schema error: {message}")]
    Schema {
        /// Human-readable failure description.
        message: String,
    },

    /// The database was initialized with a different embedding model than requested.
    #[error("{message}")]
    ModelMismatch {
        /// Human-readable mismatch description.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        string_frontmatter, ActionItemState, ConversationFile, ConversationFrontmatter,
        ConversationStatus, ExtractionJob, ExtractionJobStatus, ExtractionResponse,
        ExtractionTriggerKind, Page, PreferenceStrength, RawFact, Turn, TurnRole,
        CONVERSATION_FORMAT_VERSION,
    };
    use serde_json::json;

    #[test]
    fn page_serde_roundtrip_preserves_identifying_fields_and_tags_frontmatter() {
        let page = Page {
            slug: "people/alice".to_string(),
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234".to_string(),
            page_type: "person".to_string(),
            superseded_by: Some(42),
            title: "Alice".to_string(),
            summary: "Operator".to_string(),
            compiled_truth: "Alice runs ops.".to_string(),
            timeline: "- **2024** | role — Joined Acme".to_string(),
            frontmatter: string_frontmatter([
                ("slug".to_string(), "people/alice".to_string()),
                ("tags".to_string(), "operator, founder".to_string()),
                ("title".to_string(), "Alice".to_string()),
                ("type".to_string(), "person".to_string()),
                ("wing".to_string(), "people".to_string()),
            ]),
            wing: "people".to_string(),
            room: String::new(),
            version: 7,
            created_at: "2026-04-15T00:00:00Z".to_string(),
            updated_at: "2026-04-15T00:00:00Z".to_string(),
            truth_updated_at: "2026-04-15T00:00:00Z".to_string(),
            timeline_updated_at: "2026-04-15T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&page).unwrap();
        let round_trip: Page = serde_json::from_str(&json).unwrap();

        assert_eq!(
            (
                round_trip.slug,
                round_trip.title,
                round_trip.version,
                round_trip.superseded_by,
                round_trip.frontmatter.get("tags").cloned(),
            ),
            (
                "people/alice".to_string(),
                "Alice".to_string(),
                7,
                Some(42),
                Some(json!("operator, founder")),
            )
        );
    }

    #[test]
    fn page_serde_roundtrip_preserves_quaid_id_frontmatter() {
        let page = Page {
            slug: "people/alice".to_string(),
            uuid: "0195c7c0-2d06-7df0-bf59-acde48001122".to_string(),
            page_type: "person".to_string(),
            superseded_by: None,
            title: "Alice".to_string(),
            summary: "Operator".to_string(),
            compiled_truth: "Alice runs ops.".to_string(),
            timeline: "- **2024** | role — Joined Acme".to_string(),
            frontmatter: string_frontmatter([
                (
                    "quaid_id".to_string(),
                    "0195c7c0-2d06-7df0-bf59-acde48001122".to_string(),
                ),
                ("title".to_string(), "Alice".to_string()),
            ]),
            wing: "people".to_string(),
            room: String::new(),
            version: 7,
            created_at: "2026-04-15T00:00:00Z".to_string(),
            updated_at: "2026-04-15T00:00:00Z".to_string(),
            truth_updated_at: "2026-04-15T00:00:00Z".to_string(),
            timeline_updated_at: "2026-04-15T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&page).unwrap();
        let round_trip: Page = serde_json::from_str(&json).unwrap();

        assert_eq!(
            round_trip.frontmatter.get("quaid_id"),
            Some(&json!("0195c7c0-2d06-7df0-bf59-acde48001122"))
        );
    }

    #[test]
    fn page_serde_defaults_missing_superseded_by_for_legacy_payloads() {
        let page: Page = serde_json::from_str(
            r#"{
                "slug":"people/alice",
                "uuid":"0195c7c0-2d06-7df0-bf59-acde48001122",
                "type":"person",
                "title":"Alice",
                "summary":"Operator",
                "compiled_truth":"Alice runs ops.",
                "timeline":"",
                "frontmatter":{"title":"Alice"},
                "wing":"people",
                "room":"",
                "version":7,
                "created_at":"2026-04-15T00:00:00Z",
                "updated_at":"2026-04-15T00:00:00Z",
                "truth_updated_at":"2026-04-15T00:00:00Z",
                "timeline_updated_at":"2026-04-15T00:00:00Z"
            }"#,
        )
        .unwrap();

        assert_eq!(page.superseded_by, None);
        assert_eq!(page.slug, "people/alice");
        assert_eq!(page.page_type, "person");
    }

    #[test]
    fn conversation_file_serde_roundtrip_preserves_turn_metadata_and_cursor() {
        let file = ConversationFile {
            frontmatter: ConversationFrontmatter {
                file_type: "conversation".to_string(),
                format_version: CONVERSATION_FORMAT_VERSION,
                session_id: "s1".to_string(),
                date: "2026-05-03".to_string(),
                started_at: "2026-05-03T09:14:22Z".to_string(),
                status: ConversationStatus::Open,
                closed_at: None,
                last_extracted_at: Some("2026-05-03T10:00:00Z".to_string()),
                last_extracted_turn: 7,
            },
            turns: vec![Turn {
                ordinal: 8,
                role: TurnRole::Assistant,
                timestamp: "2026-05-03T10:01:00Z".to_string(),
                content: "Done.".to_string(),
                metadata: Some(serde_json::json!({
                    "tool_name": "bash",
                    "importance": "high"
                })),
            }],
        };

        let json = serde_json::to_string(&file).unwrap();
        let round_trip: ConversationFile = serde_json::from_str(&json).unwrap();

        assert_eq!(round_trip.frontmatter.last_extracted_turn, 7);
        assert_eq!(round_trip.turns[0].role, TurnRole::Assistant);
        assert_eq!(
            round_trip.turns[0].metadata,
            Some(serde_json::json!({
                "tool_name": "bash",
                "importance": "high"
            }))
        );
    }

    #[test]
    fn extraction_job_serde_roundtrip_preserves_status_and_trigger_kind() {
        let job = ExtractionJob {
            id: 5,
            session_id: "s1".to_string(),
            conversation_path: "conversations/2026-05-03/s1.md".to_string(),
            trigger_kind: ExtractionTriggerKind::SessionClose,
            enqueued_at: "2026-05-03T10:00:00Z".to_string(),
            scheduled_for: "2026-05-03T10:00:00Z".to_string(),
            attempts: 2,
            last_error: Some("timeout".to_string()),
            status: ExtractionJobStatus::Running,
        };

        let json = serde_json::to_string(&job).unwrap();
        let round_trip: ExtractionJob = serde_json::from_str(&json).unwrap();

        assert_eq!(round_trip.trigger_kind, ExtractionTriggerKind::SessionClose);
        assert_eq!(round_trip.status, ExtractionJobStatus::Running);
        assert_eq!(round_trip.last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn extraction_response_roundtrip_preserves_typed_facts() {
        let response = ExtractionResponse {
            facts: vec![
                RawFact::Decision {
                    chose: "rust".to_string(),
                    rationale: Some("local-first runtime".to_string()),
                    summary: "The team chose Rust for the runtime.".to_string(),
                },
                RawFact::Preference {
                    about: "programming-language".to_string(),
                    strength: Some(PreferenceStrength::High),
                    summary: "Matt strongly prefers Rust.".to_string(),
                },
                RawFact::Fact {
                    about: "timezone".to_string(),
                    summary: "Matt works in UTC+8.".to_string(),
                },
                RawFact::ActionItem {
                    who: Some("Fry".to_string()),
                    what: "wire the runtime".to_string(),
                    status: ActionItemState::Open,
                    due: Some("2026-05-05".to_string()),
                    summary: "Fry will wire the runtime next.".to_string(),
                },
            ],
            validation_errors: vec![],
        };

        let json = serde_json::to_string(&response).unwrap();
        let round_trip: ExtractionResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(round_trip, response);
    }

    #[test]
    fn raw_fact_rejects_unknown_kind() {
        let error = serde_json::from_str::<RawFact>(
            r#"{"kind":"opinion","about":"rust","summary":"unsupported"}"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown variant"));
    }

    // ── RawFact resolution-pivot helpers ──────────────────────────────────

    #[test]
    fn raw_fact_type_key_returns_chose_value_for_decision() {
        let fact = RawFact::Decision {
            chose: "rust".to_string(),
            rationale: None,
            summary: "We chose Rust.".to_string(),
        };
        assert_eq!(fact.type_key(), "rust");
        assert_eq!(fact.type_key_field(), "chose");
        assert_eq!(fact.kind_str(), "decision");
        assert_eq!(fact.type_plural(), "decisions");
    }

    #[test]
    fn raw_fact_type_key_returns_about_value_for_preference() {
        let fact = RawFact::Preference {
            about: "programming-language".to_string(),
            strength: None,
            summary: "Matt prefers Rust.".to_string(),
        };
        assert_eq!(fact.type_key(), "programming-language");
        assert_eq!(fact.type_key_field(), "about");
        assert_eq!(fact.kind_str(), "preference");
        assert_eq!(fact.type_plural(), "preferences");
    }

    #[test]
    fn raw_fact_type_key_returns_about_value_for_fact() {
        let fact = RawFact::Fact {
            about: "timezone".to_string(),
            summary: "Matt is in UTC+8.".to_string(),
        };
        assert_eq!(fact.type_key(), "timezone");
        assert_eq!(fact.type_key_field(), "about");
        assert_eq!(fact.kind_str(), "fact");
        assert_eq!(fact.type_plural(), "facts");
    }

    #[test]
    fn raw_fact_type_key_returns_what_value_for_action_item() {
        let fact = RawFact::ActionItem {
            who: None,
            what: "ship the parser".to_string(),
            status: ActionItemState::Open,
            due: None,
            summary: "Fry will land the parser batch.".to_string(),
        };
        assert_eq!(fact.type_key(), "ship the parser");
        assert_eq!(fact.type_key_field(), "what");
        assert_eq!(fact.kind_str(), "action_item");
        assert_eq!(fact.type_plural(), "action-items");
    }

    #[test]
    fn raw_fact_summary_returns_prose_body_for_each_kind() {
        let cases: &[(&str, RawFact)] = &[
            (
                "We chose Rust.",
                RawFact::Decision {
                    chose: "rust".to_string(),
                    rationale: None,
                    summary: "We chose Rust.".to_string(),
                },
            ),
            (
                "Matt prefers Rust.",
                RawFact::Preference {
                    about: "programming-language".to_string(),
                    strength: None,
                    summary: "Matt prefers Rust.".to_string(),
                },
            ),
            (
                "Matt is in UTC+8.",
                RawFact::Fact {
                    about: "timezone".to_string(),
                    summary: "Matt is in UTC+8.".to_string(),
                },
            ),
            (
                "Fry will land the parser batch.",
                RawFact::ActionItem {
                    who: None,
                    what: "ship the parser".to_string(),
                    status: ActionItemState::Open,
                    due: None,
                    summary: "Fry will land the parser batch.".to_string(),
                },
            ),
        ];
        for (expected, fact) in cases {
            assert_eq!(fact.summary(), *expected, "kind: {}", fact.kind_str());
        }
    }
}
