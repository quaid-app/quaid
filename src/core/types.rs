// Types defined ahead of consumers (db.rs, search.rs, etc.) — remove when wired.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Page ──────────────────────────────────────────────────────

/// Core knowledge page — the unit of storage in a Quaid database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub slug: String,
    pub uuid: String,
    #[serde(rename = "type")]
    pub page_type: String,
    pub superseded_by: Option<i64>,
    pub title: String,
    pub summary: String,
    pub compiled_truth: String,
    pub timeline: String,
    pub frontmatter: HashMap<String, String>,
    pub wing: String,
    pub room: String,
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
    pub truth_updated_at: String,
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
    pub from_slug: String,
    pub to_slug: String,
    pub relationship: String,
    /// Optional note stored alongside the link (schema column: `context`).
    pub context: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub created_at: String,
}

// ── Tag ───────────────────────────────────────────────────────

/// A single tag attached to a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub page_id: i64,
    pub tag: String,
}

// ── TimelineEntry ─────────────────────────────────────────────

/// A structured timeline row for a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub id: i64,
    pub page_id: i64,
    pub date: String,
    pub source: String,
    pub summary: String,
    pub detail: String,
    pub created_at: String,
}

// ── SearchResult ──────────────────────────────────────────────

/// A single result from FTS5, vector, or hybrid search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub score: f64,
    pub wing: String,
}

// ── Chunk ───────────────────────────────────────────────────────

/// A derived embedding/search chunk from a page section or timeline entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub page_slug: String,
    pub heading_path: String,
    pub content: String,
    pub content_hash: String,
    pub token_count: usize,
    pub chunk_type: String,
}

// ── KnowledgeGap ──────────────────────────────────────────────

/// An unanswered query detected by the memory engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGap {
    pub id: i64,
    pub page_id: Option<i64>,
    pub query_hash: String,
    pub context: String,
    pub confidence_score: Option<f64>,
    pub sensitivity: String,
    pub resolved_at: Option<String>,
    pub resolved_by_slug: Option<String>,
    pub detected_at: String,
}

// ── IngestRecord ──────────────────────────────────────────────

/// An entry in the idempotency audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRecord {
    pub id: i64,
    pub ingest_key: String,
    pub source_type: String,
    pub source_ref: String,
    pub pages_updated: String,
    pub summary: String,
    pub completed_at: String,
}

// ── SearchMergeStrategy ───────────────────────────────────────

/// How hybrid search merges FTS5 and vector result sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchMergeStrategy {
    SetUnion,
    Rrf,
}

impl SearchMergeStrategy {
    pub fn from_config(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "rrf" => Self::Rrf,
            _ => Self::SetUnion,
        }
    }
}

// ── Conversation ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationFrontmatter {
    #[serde(rename = "type")]
    pub file_type: String,
    pub session_id: String,
    pub date: String,
    pub started_at: String,
    pub status: ConversationStatus,
    pub closed_at: Option<String>,
    pub last_extracted_at: Option<String>,
    pub last_extracted_turn: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationFile {
    pub frontmatter: ConversationFrontmatter,
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub ordinal: i64,
    pub role: TurnRole,
    pub timestamp: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnRole {
    User,
    Assistant,
    System,
    Tool,
}

impl TurnRole {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    Open,
    Closed,
}

impl ConversationStatus {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnWriteResult {
    pub turn_id: String,
    pub ordinal: i64,
    pub conversation_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseSessionResult {
    pub closed_at: String,
    pub conversation_path: String,
    pub newly_closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionJob {
    pub id: i64,
    pub session_id: String,
    pub conversation_path: String,
    pub trigger_kind: ExtractionTriggerKind,
    pub enqueued_at: String,
    pub scheduled_for: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub status: ExtractionJobStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExtractionResponse {
    pub facts: Vec<RawFact>,
    #[serde(skip, default)]
    pub validation_errors: Vec<ExtractionFactValidationError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionFactValidationError {
    pub index: usize,
    pub kind: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceStrength {
    Low,
    Medium,
    High,
}

impl PreferenceStrength {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionItemState {
    Open,
    Done,
    Cancelled,
}

impl ActionItemState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RawFact {
    Decision {
        chose: String,
        rationale: Option<String>,
        summary: String,
    },
    Preference {
        about: String,
        strength: Option<PreferenceStrength>,
        summary: String,
    },
    Fact {
        about: String,
        summary: String,
    },
    ActionItem {
        who: Option<String>,
        what: String,
        status: ActionItemState,
        due: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowedTurns {
    pub new_turns: Vec<Turn>,
    pub lookback_turns: Vec<Turn>,
    pub context_only: bool,
}

impl WindowedTurns {
    pub fn first_new_ordinal(&self) -> Option<i64> {
        self.new_turns.first().map(|turn| turn.ordinal)
    }

    pub fn last_new_ordinal(&self) -> Option<i64> {
        self.new_turns.last().map(|turn| turn.ordinal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionTriggerKind {
    Debounce,
    SessionClose,
    Manual,
}

impl ExtractionTriggerKind {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionJobStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl ExtractionJobStatus {
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
#[derive(Debug, Error)]
pub enum OccError {
    #[error("conflict: page updated elsewhere (current version: {current_version})")]
    Conflict { current_version: i64 },
}

/// Errors from FTS5 or hybrid search operations.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("ambiguous slug: {slug} ({candidates})")]
    Ambiguous { slug: String, candidates: String },

    #[error("search failed: {message}")]
    Internal { message: String },
}

/// Errors from text embedding and vector inference operations.
#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("input text is empty")]
    EmptyInput,

    #[error("inference failed: {message}")]
    Internal { message: String },
}

/// Database-layer errors surfaced by `src/core/`.
#[derive(Debug, Error)]
pub enum DbError {
    #[error("page not found: {slug}")]
    NotFound { slug: String },

    #[error("path not found: {path}")]
    PathNotFound { path: String },

    #[error("OCC conflict: {0}")]
    Occ(#[from] OccError),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("schema error: {message}")]
    Schema { message: String },

    #[error("{message}")]
    ModelMismatch { message: String },
}

#[cfg(test)]
mod tests {
    use super::{
        ActionItemState, ConversationFile, ConversationFrontmatter, ConversationStatus,
        ExtractionJob, ExtractionJobStatus, ExtractionResponse, ExtractionTriggerKind, Page,
        PreferenceStrength, RawFact, Turn, TurnRole,
    };
    use std::collections::HashMap;

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
            frontmatter: HashMap::from([
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
                Some("operator, founder".to_string()),
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
            frontmatter: HashMap::from([
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
            round_trip.frontmatter.get("quaid_id").map(String::as_str),
            Some("0195c7c0-2d06-7df0-bf59-acde48001122")
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
