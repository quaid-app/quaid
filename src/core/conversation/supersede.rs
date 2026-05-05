use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::core::conversation::{format, queue, turn_writer};
use crate::core::db;
use crate::core::inference::{embed, embedding_evidence_kind, EmbeddingEvidenceKind};
use crate::core::types::{
    frontmatter_insert_string, ExtractionJob, Frontmatter, Page, RawFact, Turn, WindowedTurns,
};
use crate::core::{collections, markdown, namespace};

const DEFAULT_DEDUP_COSINE_MIN: f64 = 0.92;
const DEFAULT_SUPERSEDE_COSINE_MIN: f64 = 0.4;
const DEFAULT_MODEL_ALIAS: &str = "phi-3.5-mini";
const MAX_SLUG_COLLISION_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    Drop { matched_slug: String, cosine: f64 },
    Supersede { prior_slug: String, cosine: f64 },
    Coexist,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactWriteContext {
    pub collection_id: i64,
    pub root_path: PathBuf,
    pub namespace: String,
    pub session_id: String,
    pub source_turns: Vec<String>,
    pub extracted_at: String,
    pub extracted_by: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FactWriteResult {
    pub resolution: Resolution,
    pub slug: Option<String>,
    pub relative_path: Option<String>,
}

#[derive(Debug, Default)]
pub struct ResolvingFactWriter;

#[derive(Debug, Error)]
pub enum FactResolutionError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("config error: {message}")]
    Config { message: String },

    #[error("embedding error: {message}")]
    Embed { message: String },

    #[error(
        "ambiguous same-key head partition for kind `{kind}` {key_field} `{key_value}`: {candidate_slugs:?}"
    )]
    AmbiguousMatchingHeads {
        kind: String,
        key_field: String,
        key_value: String,
        candidate_slugs: Vec<String>,
    },

    #[error(
        "untrustworthy embedding evidence for kind `{kind}` {key_field} `{key_value}`: {reason}"
    )]
    UntrustworthyEmbeddingEvidence {
        kind: String,
        key_field: String,
        key_value: String,
        reason: EmbeddingEvidenceFailure,
    },

    #[error("invalid conversation path: {path}")]
    InvalidConversationPath { path: String },

    #[error("unable to allocate a unique fact slug after {attempts} attempts: {base_slug}")]
    SlugCollisionExhausted { base_slug: String, attempts: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingEvidenceFailure {
    HashShimOnly,
    Unavailable { message: String },
}

impl std::fmt::Display for EmbeddingEvidenceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HashShimOnly => write!(f, "hash-shim-only backend"),
            Self::Unavailable { message } => write!(f, "{message}"),
        }
    }
}

#[derive(Debug, Clone)]
struct HeadCandidate {
    slug: String,
    body: String,
}

#[derive(Debug, Clone, Copy)]
struct ResolutionThresholds {
    dedup_cosine_min: f64,
    supersede_cosine_min: f64,
}

pub fn resolve(raw_fact: &RawFact, conn: &Connection) -> Result<Resolution, FactResolutionError> {
    let memory_root =
        turn_writer::resolve_memory_root(conn).map_err(|error| FactResolutionError::Config {
            message: error.to_string(),
        })?;
    resolve_in_scope(raw_fact, conn, memory_root.collection_id, "")
}

pub fn resolve_in_scope(
    raw_fact: &RawFact,
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
) -> Result<Resolution, FactResolutionError> {
    let candidates = head_candidates(conn, collection_id, namespace, raw_fact)?;
    resolve_from_candidates(raw_fact, conn, candidates, cosine_similarity, true)
}

pub fn resolve_in_scope_with_similarity<F>(
    raw_fact: &RawFact,
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    similarity: F,
) -> Result<Resolution, FactResolutionError>
where
    F: Fn(&str, &str) -> Result<f64, FactResolutionError>,
{
    let candidates = head_candidates(conn, collection_id, namespace, raw_fact)?;
    resolve_from_candidates(raw_fact, conn, candidates, similarity, false)
}

fn resolve_from_candidates<F>(
    raw_fact: &RawFact,
    conn: &Connection,
    candidates: Vec<HeadCandidate>,
    similarity: F,
    require_trustworthy_embeddings: bool,
) -> Result<Resolution, FactResolutionError>
where
    F: Fn(&str, &str) -> Result<f64, FactResolutionError>,
{
    if candidates.is_empty() {
        return Ok(Resolution::Coexist);
    }

    if candidates.len() > 1 {
        return Err(FactResolutionError::AmbiguousMatchingHeads {
            kind: raw_fact.kind_str().to_string(),
            key_field: raw_fact.type_key_field().to_string(),
            key_value: raw_fact.type_key().to_string(),
            candidate_slugs: candidates
                .into_iter()
                .map(|candidate| candidate.slug)
                .collect(),
        });
    }
    if require_trustworthy_embeddings {
        ensure_trustworthy_embedding_evidence(raw_fact)?;
    }

    let thresholds = resolution_thresholds(conn)?;
    let candidate = candidates
        .into_iter()
        .next()
        .expect("candidate count checked above");
    let cosine = similarity(raw_fact.summary(), &candidate.body)?;

    if cosine > thresholds.dedup_cosine_min {
        Ok(Resolution::Drop {
            matched_slug: candidate.slug,
            cosine,
        })
    } else if cosine >= thresholds.supersede_cosine_min {
        Ok(Resolution::Supersede {
            prior_slug: candidate.slug,
            cosine,
        })
    } else {
        Ok(Resolution::Coexist)
    }
}

pub fn write_fact(
    resolution: &Resolution,
    raw_fact: &RawFact,
    conn: &Connection,
) -> Result<FactWriteResult, FactResolutionError> {
    let context = default_write_context(conn)?;
    write_fact_in_context(resolution, raw_fact, conn, &context)
}

pub fn write_fact_in_context(
    resolution: &Resolution,
    raw_fact: &RawFact,
    conn: &Connection,
    context: &FactWriteContext,
) -> Result<FactWriteResult, FactResolutionError> {
    match resolution {
        Resolution::Drop {
            matched_slug,
            cosine,
        } => {
            eprintln!(
                "INFO: fact_resolution decision=drop matched_head={} kind={} key={} cosine={:.4}",
                matched_slug,
                raw_fact.kind_str(),
                raw_fact.type_key(),
                cosine
            );
            Ok(FactWriteResult {
                resolution: resolution.clone(),
                slug: None,
                relative_path: None,
            })
        }
        Resolution::Supersede { prior_slug, .. } => {
            let (slug, relative_path) = allocate_output_path(raw_fact, conn, context)?;
            let markdown =
                render_fact_markdown(raw_fact, context, &slug, Some(prior_slug.as_str()), None)?;
            write_markdown(&context.root_path, &relative_path, &markdown)?;
            Ok(FactWriteResult {
                resolution: resolution.clone(),
                slug: Some(slug),
                relative_path: Some(path_to_slash(&relative_path)),
            })
        }
        Resolution::Coexist => {
            let (slug, relative_path) = allocate_output_path(raw_fact, conn, context)?;
            let markdown = render_fact_markdown(raw_fact, context, &slug, None, None)?;
            write_markdown(&context.root_path, &relative_path, &markdown)?;
            Ok(FactWriteResult {
                resolution: resolution.clone(),
                slug: Some(slug),
                relative_path: Some(path_to_slash(&relative_path)),
            })
        }
    }
}

pub fn resolve_and_write_fact_in_context(
    raw_fact: &RawFact,
    conn: &Connection,
    context: &FactWriteContext,
) -> Result<FactWriteResult, FactResolutionError> {
    // The IMMEDIATE transaction serialises the head-lookup and resolution decision so that
    // two concurrent workers cannot both see the same head as the "current" state. However,
    // SQLite ACID does not cover the filesystem: `write_fact_in_context` drops a vault file
    // inside the transaction window, and that file may exist on disk before or after the
    // DB commit. The Phase 4 vault watcher performs the eventual page-row insert; there is
    // no cross-seam atomic guarantee between file creation and page ingestion.
    //
    // Callers must not assume the page row is visible in the DB immediately after this call
    // returns. The watcher provides eventual consistency.
    with_immediate_transaction(conn, |conn| {
        let resolution =
            resolve_in_scope(raw_fact, conn, context.collection_id, &context.namespace)?;
        write_fact_in_context(&resolution, raw_fact, conn, context)
    })
}

pub fn force_supersede_fact_in_context(
    raw_fact: &RawFact,
    prior_slug: &str,
    conn: &Connection,
    context: &FactWriteContext,
    corrected_via: &str,
) -> Result<FactWriteResult, FactResolutionError> {
    with_immediate_transaction(conn, |conn| {
        let (slug, relative_path) = allocate_output_path(raw_fact, conn, context)?;
        let markdown = render_fact_markdown(
            raw_fact,
            context,
            &slug,
            Some(prior_slug),
            Some(corrected_via),
        )?;
        write_markdown(&context.root_path, &relative_path, &markdown)?;
        Ok(FactWriteResult {
            resolution: Resolution::Supersede {
                prior_slug: prior_slug.to_string(),
                cosine: 1.0,
            },
            slug: Some(slug),
            relative_path: Some(path_to_slash(&relative_path)),
        })
    })
}

pub fn context_for_job_window(
    conn: &Connection,
    job: &ExtractionJob,
    window: &WindowedTurns,
) -> Result<FactWriteContext, FactResolutionError> {
    let memory_root =
        turn_writer::resolve_memory_root(conn).map_err(|error| FactResolutionError::Config {
            message: error.to_string(),
        })?;
    let extracted_at =
        queue::current_timestamp(conn).map_err(|error| FactResolutionError::Config {
            message: error.to_string(),
        })?;
    let extracted_by =
        db::read_config_value_or(conn, "extraction.model_alias", DEFAULT_MODEL_ALIAS).map_err(
            |error| FactResolutionError::Config {
                message: error.to_string(),
            },
        )?;
    let parsed_path =
        format::parse_relative_conversation_path(&job.conversation_path).map_err(|_| {
            FactResolutionError::InvalidConversationPath {
                path: job.conversation_path.clone(),
            }
        })?;

    Ok(FactWriteContext {
        collection_id: memory_root.collection_id,
        root_path: memory_root.root_path,
        namespace: parsed_path.namespace.unwrap_or_default(),
        session_id: parsed_path.session_id,
        source_turns: source_turn_refs(window),
        extracted_at,
        extracted_by,
    })
}

fn default_write_context(conn: &Connection) -> Result<FactWriteContext, FactResolutionError> {
    let memory_root =
        turn_writer::resolve_memory_root(conn).map_err(|error| FactResolutionError::Config {
            message: error.to_string(),
        })?;
    let extracted_at =
        queue::current_timestamp(conn).map_err(|error| FactResolutionError::Config {
            message: error.to_string(),
        })?;
    let extracted_by =
        db::read_config_value_or(conn, "extraction.model_alias", DEFAULT_MODEL_ALIAS).map_err(
            |error| FactResolutionError::Config {
                message: error.to_string(),
            },
        )?;

    Ok(FactWriteContext {
        collection_id: memory_root.collection_id,
        root_path: memory_root.root_path,
        namespace: String::new(),
        session_id: String::new(),
        source_turns: Vec::new(),
        extracted_at,
        extracted_by,
    })
}

fn source_turn_refs(window: &WindowedTurns) -> Vec<String> {
    let turns: &[Turn] = if window.new_turns.is_empty() {
        &window.lookback_turns
    } else {
        &window.new_turns
    };
    turns.iter().map(|turn| turn.ordinal.to_string()).collect()
}

fn head_candidates(
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    raw_fact: &RawFact,
) -> Result<Vec<HeadCandidate>, FactResolutionError> {
    let key_path = format!("$.{}", raw_fact.type_key_field());
    let mut stmt = conn.prepare(
        "SELECT slug,
                COALESCE(NULLIF(compiled_truth, ''), summary, title, '')
         FROM pages
         WHERE collection_id = ?1
           AND namespace = ?2
           AND type = ?3
           AND superseded_by IS NULL
           AND json_extract(IIF(json_valid(frontmatter), frontmatter, '{}'), ?4) = ?5
         ORDER BY id",
    )?;
    let rows = stmt.query_map(
        params![
            collection_id,
            namespace,
            raw_fact.kind_str(),
            key_path,
            raw_fact.type_key()
        ],
        |row| {
            Ok(HeadCandidate {
                slug: row.get(0)?,
                body: row.get(1)?,
            })
        },
    )?;

    let mut candidates = Vec::new();
    for row in rows {
        candidates.push(row?);
    }
    Ok(candidates)
}

fn resolution_thresholds(conn: &Connection) -> Result<ResolutionThresholds, FactResolutionError> {
    Ok(ResolutionThresholds {
        dedup_cosine_min: read_f64_config(
            conn,
            "fact_resolution.dedup_cosine_min",
            DEFAULT_DEDUP_COSINE_MIN,
        )?,
        supersede_cosine_min: read_f64_config(
            conn,
            "fact_resolution.supersede_cosine_min",
            DEFAULT_SUPERSEDE_COSINE_MIN,
        )?,
    })
}

fn read_f64_config(conn: &Connection, key: &str, default: f64) -> Result<f64, FactResolutionError> {
    let raw = db::read_config_value_or(conn, key, &default.to_string()).map_err(|error| {
        FactResolutionError::Config {
            message: error.to_string(),
        }
    })?;
    raw.parse::<f64>().map_err(|_| FactResolutionError::Config {
        message: format!("invalid {key} value: {raw}"),
    })
}

fn cosine_similarity(left: &str, right: &str) -> Result<f64, FactResolutionError> {
    let left = embed(left).map_err(|error| FactResolutionError::Embed {
        message: error.to_string(),
    })?;
    let right = embed(right).map_err(|error| FactResolutionError::Embed {
        message: error.to_string(),
    })?;
    Ok(cosine_from_embeddings(&left, &right))
}

fn cosine_from_embeddings(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f64;
    let mut left_norm = 0.0f64;
    let mut right_norm = 0.0f64;
    for (left_value, right_value) in left.iter().zip(right.iter()) {
        let left_value = *left_value as f64;
        let right_value = *right_value as f64;
        dot += left_value * right_value;
        left_norm += left_value * left_value;
        right_norm += right_value * right_value;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn ensure_trustworthy_embedding_evidence(raw_fact: &RawFact) -> Result<(), FactResolutionError> {
    match embedding_evidence_kind() {
        Ok(EmbeddingEvidenceKind::Semantic) => Ok(()),
        Ok(EmbeddingEvidenceKind::HashShim) => {
            Err(FactResolutionError::UntrustworthyEmbeddingEvidence {
                kind: raw_fact.kind_str().to_string(),
                key_field: raw_fact.type_key_field().to_string(),
                key_value: raw_fact.type_key().to_string(),
                reason: EmbeddingEvidenceFailure::HashShimOnly,
            })
        }
        Err(error) => Err(FactResolutionError::UntrustworthyEmbeddingEvidence {
            kind: raw_fact.kind_str().to_string(),
            key_field: raw_fact.type_key_field().to_string(),
            key_value: raw_fact.type_key().to_string(),
            reason: EmbeddingEvidenceFailure::Unavailable {
                message: error.to_string(),
            },
        }),
    }
}

fn allocate_output_path(
    raw_fact: &RawFact,
    conn: &Connection,
    context: &FactWriteContext,
) -> Result<(String, PathBuf), FactResolutionError> {
    let base_slug = fact_slug_base(raw_fact);
    for attempt in 0..MAX_SLUG_COLLISION_ATTEMPTS {
        let slug = if attempt == 0 {
            base_slug.clone()
        } else {
            format!("{base_slug}-{}", attempt + 1)
        };
        let relative_path = relative_fact_path(raw_fact.type_plural(), &context.namespace, &slug);
        let full_path = context.root_path.join(&relative_path);
        if !full_path.exists()
            && !page_slug_exists(conn, context.collection_id, &context.namespace, &slug)?
        {
            return Ok((slug, relative_path));
        }
    }

    Err(FactResolutionError::SlugCollisionExhausted {
        base_slug,
        attempts: MAX_SLUG_COLLISION_ATTEMPTS,
    })
}

fn page_slug_exists(
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    slug: &str,
) -> Result<bool, FactResolutionError> {
    Ok(conn
        .query_row(
            "SELECT 1
             FROM pages
             WHERE collection_id = ?1 AND namespace = ?2 AND slug = ?3
             LIMIT 1",
            params![collection_id, namespace, slug],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

fn fact_slug_base(raw_fact: &RawFact) -> String {
    let normalized_key = slugify(raw_fact.type_key());
    let mut hasher = Sha256::new();
    hasher.update(stable_fact_signature(raw_fact).as_bytes());
    let digest = hasher.finalize();
    let hash = digest
        .iter()
        .take(2)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{normalized_key}-{hash}")
}

fn stable_fact_signature(raw_fact: &RawFact) -> String {
    match raw_fact {
        RawFact::Decision {
            chose,
            rationale,
            summary,
        } => format!(
            "decision|{chose}|{}|{summary}",
            rationale.as_deref().unwrap_or_default()
        ),
        RawFact::Preference {
            about,
            strength,
            summary,
        } => format!(
            "preference|{about}|{}|{summary}",
            strength
                .as_ref()
                .map(|value| value.as_str())
                .unwrap_or_default()
        ),
        RawFact::Fact { about, summary } => format!("fact|{about}|{summary}"),
        RawFact::ActionItem {
            who,
            what,
            status,
            due,
            summary,
        } => format!(
            "action_item|{}|{what}|{}|{}|{summary}",
            who.as_deref().unwrap_or_default(),
            status.as_str(),
            due.as_deref().unwrap_or_default()
        ),
    }
}

fn slugify(raw: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in raw.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "fact".to_string()
    } else {
        trimmed
    }
}

fn relative_fact_path(type_plural: &str, namespace: &str, slug: &str) -> PathBuf {
    collections::validate_relative_path(type_plural).expect("type plural should be a safe path");
    if !namespace.is_empty() {
        namespace::validate_optional_namespace(Some(namespace))
            .expect("fact namespace should already be validated");
    }
    let mut relative = PathBuf::new();
    if !namespace.is_empty() {
        relative.push(namespace);
    }
    relative.push("extracted");
    relative.push(type_plural);
    relative.push(format!("{slug}.md"));
    relative
}

fn render_fact_markdown(
    raw_fact: &RawFact,
    context: &FactWriteContext,
    slug: &str,
    supersedes: Option<&str>,
    corrected_via: Option<&str>,
) -> Result<String, FactResolutionError> {
    let mut frontmatter = Frontmatter::new();
    frontmatter.insert(
        "corrected_via".to_string(),
        corrected_via
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
    );
    frontmatter_insert_string(
        &mut frontmatter,
        "extracted_at",
        context.extracted_at.clone(),
    );
    frontmatter_insert_string(
        &mut frontmatter,
        "extracted_by",
        context.extracted_by.clone(),
    );
    frontmatter_insert_string(&mut frontmatter, "kind", raw_fact.kind_str());
    frontmatter_insert_string(&mut frontmatter, "session_id", context.session_id.clone());
    frontmatter_insert_string(&mut frontmatter, "slug", slug.to_string());
    frontmatter.insert(
        "source_turns".to_string(),
        json!(qualified_source_turns(context)),
    );
    frontmatter.insert(
        "supersedes".to_string(),
        supersedes
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
    );
    frontmatter_insert_string(&mut frontmatter, "title", raw_fact.type_key().to_string());
    frontmatter_insert_string(&mut frontmatter, "type", raw_fact.kind_str());

    match raw_fact {
        RawFact::Decision {
            chose, rationale, ..
        } => {
            frontmatter_insert_string(&mut frontmatter, "chose", chose.clone());
            if let Some(rationale) = rationale.as_deref() {
                frontmatter_insert_string(&mut frontmatter, "rationale", rationale.to_string());
            }
        }
        RawFact::Preference {
            about, strength, ..
        } => {
            frontmatter_insert_string(&mut frontmatter, "about", about.clone());
            if let Some(strength) = strength {
                frontmatter_insert_string(&mut frontmatter, "strength", strength.as_str());
            }
        }
        RawFact::Fact { about, .. } => {
            frontmatter_insert_string(&mut frontmatter, "about", about.clone());
        }
        RawFact::ActionItem {
            who,
            what,
            status,
            due,
            ..
        } => {
            if let Some(who) = who.as_deref() {
                frontmatter_insert_string(&mut frontmatter, "who", who.to_string());
            }
            frontmatter_insert_string(&mut frontmatter, "what", what.clone());
            frontmatter_insert_string(&mut frontmatter, "status", status.as_str());
            if let Some(due) = due.as_deref() {
                frontmatter_insert_string(&mut frontmatter, "due", due.to_string());
            }
        }
    }

    Ok(markdown::render_page(&Page {
        slug: slug.to_string(),
        uuid: String::new(),
        page_type: raw_fact.kind_str().to_string(),
        superseded_by: None,
        title: raw_fact.type_key().to_string(),
        summary: raw_fact.summary().to_string(),
        compiled_truth: raw_fact.summary().to_string(),
        timeline: String::new(),
        frontmatter,
        wing: String::new(),
        room: String::new(),
        version: 1,
        created_at: String::new(),
        updated_at: String::new(),
        truth_updated_at: String::new(),
        timeline_updated_at: String::new(),
    }))
}

fn qualified_source_turns(context: &FactWriteContext) -> Vec<String> {
    if context.session_id.is_empty() {
        return context.source_turns.clone();
    }

    context
        .source_turns
        .iter()
        .map(|turn| format!("{}:{turn}", context.session_id))
        .collect()
}

fn write_markdown(
    root_path: &Path,
    relative_path: &Path,
    markdown: &str,
) -> Result<(), FactResolutionError> {
    let full_path = root_path.join(relative_path);
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(full_path, markdown)?;
    Ok(())
}

fn path_to_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn with_immediate_transaction<T>(
    conn: &Connection,
    action: impl FnOnce(&Connection) -> Result<T, FactResolutionError>,
) -> Result<T, FactResolutionError> {
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;
    match action(conn) {
        Ok(value) => {
            conn.execute_batch("COMMIT TRANSACTION")?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK TRANSACTION");
            Err(error)
        }
    }
}
