// Types defined ahead of consumers (db.rs, search.rs, etc.) — remove when wired.
#![allow(dead_code)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Page ──────────────────────────────────────────────────────

/// Core knowledge page — the unit of storage in a GigaBrain database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub slug: String,
    pub uuid: String,
    #[serde(rename = "type")]
    pub page_type: String,
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

/// An unanswered query detected by the brain.
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
    use super::Page;
    use std::collections::HashMap;

    #[test]
    fn page_serde_roundtrip_preserves_identifying_fields_and_tags_frontmatter() {
        let page = Page {
            slug: "people/alice".to_string(),
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234".to_string(),
            page_type: "person".to_string(),
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
                round_trip.frontmatter.get("tags").cloned(),
            ),
            (
                "people/alice".to_string(),
                "Alice".to_string(),
                7,
                Some("operator, founder".to_string()),
            )
        );
    }

    #[test]
    fn page_serde_roundtrip_preserves_gbrain_id_frontmatter() {
        let page = Page {
            slug: "people/alice".to_string(),
            uuid: "0195c7c0-2d06-7df0-bf59-acde48001122".to_string(),
            page_type: "person".to_string(),
            title: "Alice".to_string(),
            summary: "Operator".to_string(),
            compiled_truth: "Alice runs ops.".to_string(),
            timeline: "- **2024** | role — Joined Acme".to_string(),
            frontmatter: HashMap::from([
                (
                    "gbrain_id".to_string(),
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
            round_trip.frontmatter.get("gbrain_id").map(String::as_str),
            Some("0195c7c0-2d06-7df0-bf59-acde48001122")
        );
    }
}
