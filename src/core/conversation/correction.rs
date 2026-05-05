use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use thiserror::Error;
use uuid::Uuid;

use crate::commands::ingest;
use crate::core::conversation::{
    extractor::SlmClient,
    slm::{validate_raw_fact, SlmError},
    supersede::{self, FactResolutionError, FactWriteContext},
};
use crate::core::db;
use crate::core::supersede::successor_slug_by_id;
use crate::core::types::{frontmatter_get_string, Frontmatter, RawFact};
use crate::core::vault_sync::{self, ResolvedSlug, VaultSyncError};

const DEFAULT_MODEL_ALIAS: &str = "phi-3.5-mini";
pub const DEFAULT_CORRECTION_MAX_TOKENS: usize = 2048;
pub const MAX_CORRECTION_TURNS: i64 = 3;

const CORRECTION_SYSTEM_PROMPT: &str = concat!(
    "You repair one extracted fact. Output JSON only — no prose, no markdown fences.\n",
    "You are not a chat partner. Your job is to either commit a corrected fact,\n",
    "ask exactly one clarifying question, or abandon.\n\n",
    "Allowed outputs only:\n",
    "  {\"outcome\":\"commit\",\"fact\":{...}}\n",
    "  {\"outcome\":\"clarify\",\"question\":\"...\"}\n",
    "  {\"outcome\":\"abandon\",\"reason\":\"...\"}\n\n",
    "Rules:\n",
    "- `commit.fact` must be exactly one extracted fact object using the existing schema:\n",
    "    decision     { kind, chose, rationale?, summary }\n",
    "    preference   { kind, about, strength?, summary }\n",
    "    fact         { kind, about, summary }\n",
    "    action_item  { kind, who?, what, status, due?, summary }\n",
    "- Do not emit arrays, markdown, or extra keys outside the selected outcome shape.\n",
    "- Ask at most one direct question in `clarify.question`.\n",
    "- Use `abandon` only when the correction cannot be made actionable from the dialogue.\n"
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionExchange {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    pub correction_id: String,
    pub fact_slug: String,
    pub exchange_log: Vec<CorrectionExchange>,
    pub turns_used: i64,
    pub turn_budget: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CorrectionStep {
    Committed {
        new_fact_slug: String,
        supersedes: String,
    },
    NeedsClarification {
        correction_id: String,
        question: String,
        turns_remaining: i64,
    },
    Abandoned {
        reason: String,
    },
}

pub trait CorrectionSlmClient {
    fn infer(&self, alias: &str, prompt: &str, max_tokens: usize) -> Result<String, SlmError>;
}

impl<T> CorrectionSlmClient for T
where
    T: SlmClient + ?Sized,
{
    fn infer(&self, alias: &str, prompt: &str, max_tokens: usize) -> Result<String, SlmError> {
        SlmClient::infer(self, alias, prompt, max_tokens)
    }
}

#[derive(Debug, Error)]
pub enum CorrectionError {
    #[error("NotFoundError: page `{slug}` not found")]
    NotFound { slug: String },

    #[error("KindError: page `{slug}` is `{page_type}` not one of decision, preference, fact, action_item")]
    Kind { slug: String, page_type: String },

    #[error("{message}")]
    Conflict { message: String },

    #[error("invalid correction request: {message}")]
    InvalidRequest { message: String },

    #[error("correction config error: {message}")]
    Config { message: String },

    #[error("correction output error: {message}")]
    Output { message: String },

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Slm(#[from] SlmError),

    #[error(transparent)]
    FactResolution(#[from] FactResolutionError),

    #[error(transparent)]
    VaultSync(#[from] VaultSyncError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CorrectionSessionStatus {
    Open,
    Committed,
    Abandoned,
    Expired,
}

impl CorrectionSessionStatus {
    fn parse(value: &str) -> Result<Self, CorrectionError> {
        match value {
            "open" => Ok(Self::Open),
            "committed" => Ok(Self::Committed),
            "abandoned" => Ok(Self::Abandoned),
            "expired" => Ok(Self::Expired),
            other => Err(CorrectionError::Output {
                message: format!("invalid correction session status: {other}"),
            }),
        }
    }
}

#[derive(Debug, Clone)]
struct CorrectionSessionRow {
    correction: Correction,
    status: CorrectionSessionStatus,
    expires_at: String,
}

#[derive(Debug, Clone)]
struct TargetFact {
    resolved: ResolvedSlug,
    namespace: String,
    page_type: String,
    compiled_truth: String,
    summary: String,
    frontmatter: Frontmatter,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CorrectionOutcomeEnvelope {
    Commit,
    Clarify,
    Abandon,
}

#[derive(Debug, Deserialize)]
struct RawCorrectionResponse {
    outcome: CorrectionOutcomeEnvelope,
    fact: Option<JsonValue>,
    question: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedCorrectionResponse {
    Commit(RawFact),
    Clarify { question: String },
    Abandon { reason: String },
}

pub fn start_correction<S: CorrectionSlmClient + ?Sized>(
    conn: &Connection,
    slm: &S,
    fact_slug: &str,
    correction_text: &str,
) -> Result<CorrectionStep, CorrectionError> {
    if correction_text.trim().is_empty() {
        return Err(CorrectionError::InvalidRequest {
            message: "`correction` must not be empty".to_string(),
        });
    }
    let target = resolve_target_fact(conn, fact_slug)?;
    let canonical_fact_slug = target.resolved.canonical_slug();
    let correction_id = Uuid::now_v7().to_string();
    let mut exchange_log = vec![CorrectionExchange {
        role: "user".to_string(),
        content: correction_text.trim().to_string(),
    }];

    conn.execute(
        "INSERT INTO correction_sessions
             (correction_id, fact_slug, exchange_log, turns_used, status, created_at, expires_at)
         VALUES (?1, ?2, ?3, 1, 'open',
                 strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+1 hour'))",
        params![
            correction_id,
            canonical_fact_slug,
            serde_json::to_string(&exchange_log)?
        ],
    )?;

    let prompt = build_correction_prompt(&target, &exchange_log);
    let model_alias = configured_model_alias(conn)?;
    let raw = slm.infer(&model_alias, &prompt, DEFAULT_CORRECTION_MAX_TOKENS)?;
    let outcome = parse_correction_response(&raw, &target.page_type)?;

    apply_slm_outcome(conn, &target, &correction_id, &mut exchange_log, 1, outcome)
}

pub fn continue_correction<S: CorrectionSlmClient + ?Sized>(
    conn: &Connection,
    slm: &S,
    correction_id: &str,
    response: Option<&str>,
    abandon: bool,
) -> Result<CorrectionStep, CorrectionError> {
    if abandon == response.is_some() {
        return Err(CorrectionError::InvalidRequest {
            message: "exactly one of `response` or `abandon: true` must be provided".to_string(),
        });
    }

    let now = current_timestamp(conn)?;
    let mut session = load_correction_session(conn, correction_id)?;
    if session.status == CorrectionSessionStatus::Open && session.expires_at < now {
        expire_session(conn, correction_id)?;
        return Err(expired_conflict(correction_id));
    }
    ensure_session_open(&session, correction_id)?;

    if abandon {
        conn.execute(
            "UPDATE correction_sessions
             SET status = 'abandoned'
             WHERE correction_id = ?1",
            [correction_id],
        )?;
        return Ok(CorrectionStep::Abandoned {
            reason: "user_requested".to_string(),
        });
    }

    let response = response.expect("validated above").trim();
    if response.is_empty() {
        return Err(CorrectionError::InvalidRequest {
            message: "`response` must not be empty".to_string(),
        });
    }

    session.correction.exchange_log.push(CorrectionExchange {
        role: "user".to_string(),
        content: response.to_string(),
    });
    session.correction.turns_used += 1;

    conn.execute(
        "UPDATE correction_sessions
         SET exchange_log = ?2,
             turns_used = ?3
         WHERE correction_id = ?1",
        params![
            correction_id,
            serde_json::to_string(&session.correction.exchange_log)?,
            session.correction.turns_used
        ],
    )?;

    let target = resolve_target_fact(conn, &session.correction.fact_slug)?;
    let prompt = build_correction_prompt(&target, &session.correction.exchange_log);
    let model_alias = configured_model_alias(conn)?;
    let raw = slm.infer(&model_alias, &prompt, DEFAULT_CORRECTION_MAX_TOKENS)?;
    let outcome = parse_correction_response(&raw, &target.page_type)?;

    apply_slm_outcome(
        conn,
        &target,
        correction_id,
        &mut session.correction.exchange_log,
        session.correction.turns_used,
        outcome,
    )
}

fn apply_slm_outcome(
    conn: &Connection,
    target: &TargetFact,
    correction_id: &str,
    exchange_log: &mut Vec<CorrectionExchange>,
    turns_used: i64,
    outcome: ParsedCorrectionResponse,
) -> Result<CorrectionStep, CorrectionError> {
    match outcome {
        ParsedCorrectionResponse::Commit(raw_fact) => {
            exchange_log.push(CorrectionExchange {
                role: "assistant".to_string(),
                content: format!("commit: {}", raw_fact.summary()),
            });
            let write_context = correction_write_context(conn, target)?;
            let write_result = supersede::force_supersede_fact_in_context(
                &raw_fact,
                &target.resolved.slug,
                conn,
                &write_context,
                "explicit",
            )?;
            let relative_path =
                write_result
                    .relative_path
                    .clone()
                    .ok_or_else(|| CorrectionError::Output {
                        message: "forced supersede write did not return a relative path"
                            .to_string(),
                    })?;
            let full_path = PathBuf::from(&write_context.root_path)
                .join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            ingest::run(
                conn,
                full_path.to_str().ok_or_else(|| CorrectionError::Output {
                    message: format!("non-utf8 correction path: {}", full_path.display()),
                })?,
                false,
            )
            .map_err(|error| CorrectionError::Output {
                message: format!("failed to ingest corrected fact: {error}"),
            })?;
            let new_fact_slug = write_result.slug.ok_or_else(|| CorrectionError::Output {
                message: "forced supersede write did not return a slug".to_string(),
            })?;
            conn.execute(
                "UPDATE correction_sessions
                 SET exchange_log = ?2,
                     turns_used = ?3,
                     status = 'committed'
                 WHERE correction_id = ?1",
                params![
                    correction_id,
                    serde_json::to_string(exchange_log)?,
                    turns_used
                ],
            )?;
            Ok(CorrectionStep::Committed {
                new_fact_slug: format!("{}::{}", target.resolved.collection_name, new_fact_slug),
                supersedes: target.resolved.canonical_slug(),
            })
        }
        ParsedCorrectionResponse::Clarify { question } => {
            if turns_used >= MAX_CORRECTION_TURNS {
                conn.execute(
                    "UPDATE correction_sessions
                     SET exchange_log = ?2,
                         turns_used = ?3,
                         status = 'abandoned'
                     WHERE correction_id = ?1",
                    params![
                        correction_id,
                        serde_json::to_string(exchange_log)?,
                        turns_used
                    ],
                )?;
                return Ok(CorrectionStep::Abandoned {
                    reason: "turn_cap_reached".to_string(),
                });
            }

            exchange_log.push(CorrectionExchange {
                role: "assistant".to_string(),
                content: question.clone(),
            });
            conn.execute(
                "UPDATE correction_sessions
                 SET exchange_log = ?2,
                     turns_used = ?3,
                     status = 'open'
                 WHERE correction_id = ?1",
                params![
                    correction_id,
                    serde_json::to_string(exchange_log)?,
                    turns_used
                ],
            )?;
            Ok(CorrectionStep::NeedsClarification {
                correction_id: correction_id.to_string(),
                question,
                turns_remaining: MAX_CORRECTION_TURNS - turns_used,
            })
        }
        ParsedCorrectionResponse::Abandon { reason } => {
            exchange_log.push(CorrectionExchange {
                role: "assistant".to_string(),
                content: format!("abandon: {reason}"),
            });
            let reason = if turns_used >= MAX_CORRECTION_TURNS {
                "turn_cap_reached".to_string()
            } else {
                "slm_abandoned".to_string()
            };
            conn.execute(
                "UPDATE correction_sessions
                 SET exchange_log = ?2,
                     turns_used = ?3,
                     status = 'abandoned'
                 WHERE correction_id = ?1",
                params![
                    correction_id,
                    serde_json::to_string(exchange_log)?,
                    turns_used
                ],
            )?;
            Ok(CorrectionStep::Abandoned { reason })
        }
    }
}

fn resolve_target_fact(conn: &Connection, fact_slug: &str) -> Result<TargetFact, CorrectionError> {
    let resolved =
        vault_sync::resolve_page_for_read(conn, fact_slug).map_err(|error| match error {
            VaultSyncError::PageNotFound { .. } => CorrectionError::NotFound {
                slug: fact_slug.to_string(),
            },
            other => CorrectionError::VaultSync(other),
        })?;

    let row = conn
        .query_row(
            "SELECT type, frontmatter, COALESCE(NULLIF(compiled_truth, ''), summary, ''),
                    summary, superseded_by, namespace
             FROM pages
             WHERE collection_id = ?1 AND slug = ?2
             LIMIT 1",
            params![resolved.collection_id, &resolved.slug],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;

    let Some((page_type, frontmatter_json, compiled_truth, summary, successor_id, namespace)) = row
    else {
        return Err(CorrectionError::NotFound {
            slug: resolved.canonical_slug(),
        });
    };

    if !is_correctable_kind(&page_type) {
        return Err(CorrectionError::Kind {
            slug: resolved.canonical_slug(),
            page_type,
        });
    }

    if let Some(successor_id) = successor_id {
        let successor_slug = successor_slug_by_id(conn, Some(successor_id))?
            .unwrap_or_else(|| "<unknown>".to_string());
        return Err(CorrectionError::Conflict {
            message: format!(
                "ConflictError: page `{}` is already superseded by `{}`; correct the current head instead",
                resolved.canonical_slug(),
                successor_slug
            ),
        });
    }

    Ok(TargetFact {
        resolved,
        namespace,
        page_type,
        compiled_truth,
        summary,
        frontmatter: serde_json::from_str(&frontmatter_json)?,
    })
}

fn load_correction_session(
    conn: &Connection,
    correction_id: &str,
) -> Result<CorrectionSessionRow, CorrectionError> {
    let row = conn
        .query_row(
            "SELECT fact_slug, exchange_log, turns_used, status, expires_at
             FROM correction_sessions
             WHERE correction_id = ?1",
            [correction_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;

    let Some((fact_slug, exchange_log, turns_used, status, expires_at)) = row else {
        return Err(CorrectionError::NotFound {
            slug: correction_id.to_string(),
        });
    };

    Ok(CorrectionSessionRow {
        correction: Correction {
            correction_id: correction_id.to_string(),
            fact_slug,
            exchange_log: serde_json::from_str(&exchange_log)?,
            turns_used,
            turn_budget: MAX_CORRECTION_TURNS,
        },
        status: CorrectionSessionStatus::parse(&status)?,
        expires_at,
    })
}

fn ensure_session_open(
    session: &CorrectionSessionRow,
    correction_id: &str,
) -> Result<(), CorrectionError> {
    match session.status {
        CorrectionSessionStatus::Open => Ok(()),
        CorrectionSessionStatus::Expired => Err(expired_conflict(correction_id)),
        CorrectionSessionStatus::Committed => Err(CorrectionError::Conflict {
            message: format!(
                "ConflictError: correction session `{correction_id}` is already committed"
            ),
        }),
        CorrectionSessionStatus::Abandoned => Err(CorrectionError::Conflict {
            message: format!(
                "ConflictError: correction session `{correction_id}` is already abandoned"
            ),
        }),
    }
}

fn expired_conflict(correction_id: &str) -> CorrectionError {
    CorrectionError::Conflict {
        message: format!(
            "ConflictError: correction session `{correction_id}` has expired; start a new correction"
        ),
    }
}

fn expire_session(conn: &Connection, correction_id: &str) -> Result<(), CorrectionError> {
    conn.execute(
        "UPDATE correction_sessions
         SET status = 'expired'
         WHERE correction_id = ?1 AND status = 'open'",
        [correction_id],
    )?;
    Ok(())
}

fn configured_model_alias(conn: &Connection) -> Result<String, CorrectionError> {
    db::read_config_value_or(conn, "extraction.model_alias", DEFAULT_MODEL_ALIAS).map_err(|error| {
        CorrectionError::Config {
            message: error.to_string(),
        }
    })
}

fn current_timestamp(conn: &Connection) -> Result<String, CorrectionError> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })
    .map_err(CorrectionError::from)
}

fn correction_write_context(
    conn: &Connection,
    target: &TargetFact,
) -> Result<FactWriteContext, CorrectionError> {
    vault_sync::ensure_collection_vault_write_allowed(conn, target.resolved.collection_id)?;
    let collection = vault_sync::load_collection_by_id(conn, target.resolved.collection_id)?;
    if collection.root_path.trim().is_empty() {
        return Err(CorrectionError::Config {
            message: format!(
                "collection `{}` has no writable root_path configured",
                collection.name
            ),
        });
    }

    let session_id = frontmatter_get_string(&target.frontmatter, "session_id").unwrap_or_default();
    Ok(FactWriteContext {
        collection_id: target.resolved.collection_id,
        root_path: PathBuf::from(collection.root_path),
        namespace: target.namespace.clone(),
        session_id: session_id.clone(),
        source_turns: source_turns_from_frontmatter(&target.frontmatter, &session_id),
        extracted_at: current_timestamp(conn)?,
        extracted_by: configured_model_alias(conn)?,
    })
}

fn source_turns_from_frontmatter(frontmatter: &Frontmatter, session_id: &str) -> Vec<String> {
    frontmatter
        .get("source_turns")
        .and_then(JsonValue::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(JsonValue::as_str)
                .map(|entry| strip_session_prefix(entry, session_id))
                .collect()
        })
        .unwrap_or_default()
}

fn strip_session_prefix(entry: &str, session_id: &str) -> String {
    if session_id.is_empty() {
        return entry.to_string();
    }
    entry
        .strip_prefix(&format!("{session_id}:"))
        .unwrap_or(entry)
        .to_string()
}

fn build_correction_prompt(target: &TargetFact, exchange_log: &[CorrectionExchange]) -> String {
    let current_fact = json!({
        "slug": target.resolved.canonical_slug(),
        "kind": target.page_type,
        "frontmatter": target.frontmatter,
        "compiled_truth": target.compiled_truth,
        "summary": target.summary,
    });
    let dialogue = serde_json::to_string_pretty(exchange_log).unwrap_or_else(|_| "[]".to_string());

    format!(
        "{CORRECTION_SYSTEM_PROMPT}\n\
         Existing head fact:\n{}\n\n\
         Correction dialogue so far:\n{}\n\n\
         Produce exactly one JSON object using one allowed outcome.",
        serde_json::to_string_pretty(&current_fact).unwrap_or_else(|_| "{}".to_string()),
        dialogue
    )
}

fn parse_correction_response(
    raw: &str,
    expected_kind: &str,
) -> Result<ParsedCorrectionResponse, CorrectionError> {
    let trimmed = raw.trim();
    let json = strip_json_fence(trimmed).unwrap_or(trimmed);
    let response: RawCorrectionResponse = serde_json::from_str(json)?;
    match response.outcome {
        CorrectionOutcomeEnvelope::Commit => {
            let fact_value = response.fact.ok_or_else(|| CorrectionError::Output {
                message: "commit outcome requires a `fact` object".to_string(),
            })?;
            let fact = serde_json::from_value::<RawFact>(fact_value).map_err(|error| {
                CorrectionError::Output {
                    message: format!("invalid commit fact: {error}"),
                }
            })?;
            if let Some(message) = validate_raw_fact(&fact) {
                return Err(CorrectionError::Output { message });
            }
            if fact.kind_str() != expected_kind {
                return Err(CorrectionError::Output {
                    message: format!(
                        "commit fact kind `{}` does not match target kind `{expected_kind}`",
                        fact.kind_str()
                    ),
                });
            }
            Ok(ParsedCorrectionResponse::Commit(fact))
        }
        CorrectionOutcomeEnvelope::Clarify => {
            let question = response
                .question
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| CorrectionError::Output {
                    message: "clarify outcome requires a non-empty `question`".to_string(),
                })?;
            Ok(ParsedCorrectionResponse::Clarify { question })
        }
        CorrectionOutcomeEnvelope::Abandon => {
            let reason = response
                .reason
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| CorrectionError::Output {
                    message: "abandon outcome requires a non-empty `reason`".to_string(),
                })?;
            Ok(ParsedCorrectionResponse::Abandon { reason })
        }
    }
}

fn strip_json_fence(raw: &str) -> Option<&str> {
    let first_newline = raw.find('\n')?;
    let header = raw[..first_newline].trim();
    if !(header.eq_ignore_ascii_case("```json") || header == "```") {
        return None;
    }
    let body = raw[first_newline + 1..].trim_end();
    let inner = body.strip_suffix("```")?;
    Some(inner.trim())
}

fn is_correctable_kind(page_type: &str) -> bool {
    matches!(
        page_type,
        "decision" | "preference" | "fact" | "action_item"
    )
}
