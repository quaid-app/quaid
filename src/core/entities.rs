//! Regex-based entity-pattern extraction (Wave 5 / tasks 6.x–8.x).
//!
//! Loads a small set of compiled regex patterns from either
//! `~/.quaid/entity-patterns.yaml` (user override) or the embedded default set,
//! scans a page's `compiled_truth` under a 5 ms wall-clock budget, resolves
//! each `(subject, object)` surface within the source collection using a
//! role-aware resolver, and routes every match to the `assertions` side table.
//!
//! Per the repaired design (Decision 11), this change **does not** insert
//! `links` rows with `source_kind = 'entity_pattern'`. Durable entity edges
//! require source-page provenance and proven retraction semantics and are
//! deferred to a follow-on change. The extraction code path is also
//! deliberately free of any embedding/inference/network call (task 7.7).
//!
//! See also: `links` for derived-edge sync primitives, `assertions` for the
//! contradiction detection side, and `gaps` for budget-overrun logging.

#![expect(
    clippy::expect_used,
    reason = "expects guard compile-time invariants exercised by tests"
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use thiserror::Error;

use crate::core::gaps;
use crate::core::links::resolve_slug;

/// Wall-clock budget per page for the full pattern scan (task 7.1).
pub const EXTRACTION_BUDGET: Duration = Duration::from_millis(5);

/// Raw shape of a pattern entry as it appears in YAML (embedded or user).
#[derive(Debug, Clone, Deserialize)]
struct RawPattern {
    regex: String,
    relationship: String,
    #[serde(default)]
    subject_type: Option<String>,
    #[serde(default)]
    object_type: Option<String>,
    #[serde(default)]
    weight: Option<f64>,
}

/// A compiled, validated entity pattern.
#[derive(Debug, Clone)]
pub struct EntityPattern {
    /// Compiled regex with exactly two capture groups (subject, object).
    pub regex: Regex,
    /// Relationship label used as the assertion predicate.
    pub relationship: String,
    /// Role hint applied to the subject surface when resolving against pages.
    pub subject_type: Option<String>,
    /// Role hint applied to the object surface when resolving against pages.
    pub object_type: Option<String>,
    /// Confidence/edge weight in `[0.0, 1.0]` propagated to assertions.
    pub weight: f64,
}

/// A single (subject_surface, object_surface) match produced by the scanner.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityMatch {
    /// Raw subject surface as captured from the page text.
    pub subject_surface: String,
    /// Raw object surface as captured from the page text.
    pub object_surface: String,
    /// Relationship copied from the matching pattern.
    pub relationship: String,
    /// Weight copied from the matching pattern.
    pub weight: f64,
    /// Subject role hint copied from the matching pattern.
    pub subject_type: Option<String>,
    /// Object role hint copied from the matching pattern.
    pub object_type: Option<String>,
}

/// Outcome of resolving a single surface against the source collection.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceResolution {
    /// Surface resolved to exactly one page in the source collection.
    Resolved {
        /// Page row id of the resolved page.
        page_id: i64,
        /// Canonical slug of the resolved page.
        slug: String,
    },
    /// Surface could not be resolved (no page or ambiguous).
    Unresolved,
}

/// Failure mode raised by entity loading or routing. Extraction itself does
/// not raise; budget overrun is reported via `EntityMatch` accounting and the
/// knowledge-gap log.
#[derive(Debug, Error)]
pub enum EntityError {
    /// `~/.quaid/entity-patterns.yaml` could not be read.
    #[error("failed to read entity pattern file {path}: {source}")]
    PatternFileIo {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// YAML parse failure for either the embedded defaults or the user file.
    #[error("entity pattern YAML is invalid ({location}): {message}")]
    PatternYaml {
        /// `defaults` or the file path string.
        location: String,
        /// Underlying serde error message.
        message: String,
    },
    /// Pattern regex failed to compile.
    #[error("entity pattern regex failed to compile (relationship={relationship}): {source}")]
    PatternRegex {
        /// Relationship label of the offending entry.
        relationship: String,
        /// Underlying regex error.
        #[source]
        source: regex::Error,
    },
    /// Pattern has the wrong capture-group count.
    #[error("entity pattern (relationship={relationship}) must have exactly 2 capture groups, found {found}")]
    PatternCaptureGroups {
        /// Relationship label of the offending entry.
        relationship: String,
        /// Number of capture groups actually present.
        found: usize,
    },
    /// Pattern weight is outside the `[0.0, 1.0]` range.
    #[error("entity pattern (relationship={relationship}) weight {weight} is outside [0.0, 1.0]")]
    PatternWeight {
        /// Relationship label of the offending entry.
        relationship: String,
        /// Offending weight value.
        weight: f64,
    },
    /// Underlying SQLite failure.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Knowledge-gap log failure (only used for budget-overrun reporting).
    #[error("knowledge gap error: {0}")]
    Gaps(#[from] gaps::GapsError),
}

const DEFAULT_PATTERNS_YAML: &str = include_str!("entities_data/default_patterns.yaml");

/// Default weight applied when neither the pattern nor `config.edge_weight_entity_pattern` is set.
pub const DEFAULT_ENTITY_WEIGHT: f64 = 0.7;

/// Load the active pattern set (task 6.3).
///
/// Resolution order:
/// 1. `~/.quaid/entity-patterns.yaml` if it exists (override)
/// 2. The embedded default set
///
/// Compilation/validation happens **before** any page mutation; callers should
/// invoke this at the start of write/extraction commands and surface the
/// resulting error to the user (task 6.6, scenario "Malformed pattern file
/// fails the extraction command before ingesting pages").
pub fn load_patterns(conn: &Connection) -> Result<Vec<EntityPattern>, EntityError> {
    load_patterns_from(default_user_pattern_path().as_deref(), conn)
}

/// Same as `load_patterns` but allows the caller to inject an explicit
/// override path (used by tests).
pub fn load_patterns_from(
    user_path: Option<&Path>,
    conn: &Connection,
) -> Result<Vec<EntityPattern>, EntityError> {
    let default_weight = read_default_weight(conn);
    let (yaml, location): (String, String) = match user_path {
        Some(path) if path.exists() => {
            let body =
                std::fs::read_to_string(path).map_err(|source| EntityError::PatternFileIo {
                    path: path.to_path_buf(),
                    source,
                })?;
            (body, path.display().to_string())
        }
        _ => (DEFAULT_PATTERNS_YAML.to_owned(), "defaults".to_owned()),
    };

    let raw: Vec<RawPattern> =
        serde_yaml::from_str(&yaml).map_err(|err| EntityError::PatternYaml {
            location: location.clone(),
            message: err.to_string(),
        })?;

    let mut compiled = Vec::with_capacity(raw.len());
    for entry in raw {
        let mut pattern = compile_pattern(entry, default_weight)?;
        apply_role_defaults(&mut pattern);
        compiled.push(pattern);
    }
    Ok(compiled)
}

fn read_default_weight(conn: &Connection) -> f64 {
    match crate::core::db::read_config_value(conn, "edge_weight_entity_pattern") {
        Ok(Some(value)) => value.parse().unwrap_or(DEFAULT_ENTITY_WEIGHT),
        _ => DEFAULT_ENTITY_WEIGHT,
    }
}

fn compile_pattern(raw: RawPattern, default_weight: f64) -> Result<EntityPattern, EntityError> {
    let regex = Regex::new(&raw.regex).map_err(|source| EntityError::PatternRegex {
        relationship: raw.relationship.clone(),
        source,
    })?;

    let capture_groups = regex.captures_len().saturating_sub(1);
    if capture_groups != 2 {
        return Err(EntityError::PatternCaptureGroups {
            relationship: raw.relationship,
            found: capture_groups,
        });
    }

    let weight = raw.weight.unwrap_or(default_weight);
    if !(0.0..=1.0).contains(&weight) {
        return Err(EntityError::PatternWeight {
            relationship: raw.relationship,
            weight,
        });
    }

    Ok(EntityPattern {
        regex,
        relationship: raw.relationship,
        subject_type: raw.subject_type,
        object_type: raw.object_type,
        weight,
    })
}

/// Path to the user override file, if `$HOME` is set.
fn default_user_pattern_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut path = PathBuf::from(home);
    path.push(".quaid");
    path.push("entity-patterns.yaml");
    Some(path)
}

/// Apply role-hint defaults based on relationship (task 6.4). User-supplied
/// hints on the pattern always win; this only fills in `None`.
pub fn apply_role_defaults(pattern: &mut EntityPattern) {
    match pattern.relationship.as_str() {
        "works_at" | "founded" | "leads" => {
            pattern
                .subject_type
                .get_or_insert_with(|| "person".to_owned());
        }
        _ => {}
    }
    match pattern.relationship.as_str() {
        "works_at" | "founded" | "invested_in" | "acquired" => {
            pattern
                .object_type
                .get_or_insert_with(|| "company".to_owned());
        }
        "leads" => {
            pattern.object_type.get_or_insert_with(|| "team".to_owned());
        }
        _ => {}
    }
}

/// Run the pattern set against `compiled_truth` under a 5 ms wall-clock
/// budget (task 7.1). When the budget is exhausted between patterns the
/// remaining patterns are skipped and `over_budget` is set to `true`.
pub fn extract_entities(
    compiled_truth: &str,
    patterns: &[EntityPattern],
    deadline: Duration,
) -> ExtractionOutcome {
    let start = Instant::now();
    let mut matches = Vec::new();
    let mut over_budget = false;
    let mut patterns_run = 0_usize;

    for pattern in patterns {
        if start.elapsed() >= deadline {
            over_budget = true;
            break;
        }
        patterns_run += 1;
        for cap in pattern.regex.captures_iter(compiled_truth) {
            let Some(subject) = cap.get(1) else { continue };
            let Some(object) = cap.get(2) else { continue };
            matches.push(EntityMatch {
                subject_surface: subject.as_str().trim().to_owned(),
                object_surface: object.as_str().trim().to_owned(),
                relationship: pattern.relationship.clone(),
                weight: pattern.weight,
                subject_type: pattern.subject_type.clone(),
                object_type: pattern.object_type.clone(),
            });
        }
    }

    ExtractionOutcome {
        matches,
        over_budget,
        patterns_run,
        elapsed: start.elapsed(),
    }
}

/// Result of a single `extract_entities` call.
#[derive(Debug, Clone)]
pub struct ExtractionOutcome {
    /// All `(subject, object)` matches accumulated across patterns.
    pub matches: Vec<EntityMatch>,
    /// `true` if the 5 ms budget was exhausted between patterns.
    pub over_budget: bool,
    /// Number of patterns actually executed before the budget check tripped.
    pub patterns_run: usize,
    /// Total wall-clock elapsed running patterns.
    pub elapsed: Duration,
}

/// Collection-local role-aware surface resolver (task 6.5).
///
/// Resolution strategies, tried in order:
/// 1. Exact slug normalization (`resolve_slug(surface)`)
/// 2. Role-prefixed slug (`<role>/<slug-basename>`)
/// 3. Case-insensitive exact title match
/// 4. Unique slug basename match (last `/`-segment)
///
/// A surface resolves only if exactly one page matches. Ambiguity ⇒
/// `SurfaceResolution::Unresolved`.
pub fn resolve_entity_surface(
    surface: &str,
    role_hint: Option<&str>,
    source_collection_id: i64,
    conn: &Connection,
) -> Result<SurfaceResolution, rusqlite::Error> {
    let trimmed = surface.trim();
    if trimmed.is_empty() {
        return Ok(SurfaceResolution::Unresolved);
    }

    // (1) Exact slug normalization.
    let normalized = resolve_slug(trimmed);
    if !normalized.is_empty() {
        if let Some(row) = lookup_by_slug(conn, source_collection_id, &normalized)? {
            return Ok(row);
        }
    }

    // (2) Role-prefixed slug candidate: <role>s/<slug>.
    if let Some(role) = role_hint {
        let prefixed = format!("{}/{}", pluralise_role(role), basename(&normalized));
        if prefixed != normalized {
            if let Some(row) = lookup_by_slug(conn, source_collection_id, &prefixed)? {
                return Ok(row);
            }
        }
    }

    // (3) Case-insensitive title match.
    if let Some(row) = lookup_by_title_ci(conn, source_collection_id, trimmed, role_hint)? {
        return Ok(row);
    }

    // (4) Unique basename match.
    let candidate_basename = basename(&normalized);
    if !candidate_basename.is_empty() {
        if let Some(row) =
            lookup_by_basename_unique(conn, source_collection_id, candidate_basename, role_hint)?
        {
            return Ok(row);
        }
    }

    Ok(SurfaceResolution::Unresolved)
}

fn basename(slug: &str) -> &str {
    slug.rsplit('/').next().unwrap_or(slug)
}

fn pluralise_role(role: &str) -> String {
    match role {
        "person" => "people".to_owned(),
        "company" => "companies".to_owned(),
        "team" => "teams".to_owned(),
        other if other.ends_with('s') => other.to_owned(),
        other => format!("{other}s"),
    }
}

fn lookup_by_slug(
    conn: &Connection,
    collection_id: i64,
    slug: &str,
) -> Result<Option<SurfaceResolution>, rusqlite::Error> {
    Ok(crate::core::pages::resolve_optional(
        conn,
        &crate::core::pages::PageKey {
            collection_id,
            namespace: None,
            slug,
        },
    )?
    .map(|page_id| SurfaceResolution::Resolved {
        page_id,
        slug: slug.to_owned(),
    }))
}

fn lookup_by_title_ci(
    conn: &Connection,
    collection_id: i64,
    title: &str,
    role_hint: Option<&str>,
) -> Result<Option<SurfaceResolution>, rusqlite::Error> {
    let role_prefix = role_hint.map(role_slug_prefix);
    let mut stmt = conn.prepare(
        "SELECT id, slug FROM pages
         WHERE collection_id = ?1
           AND LOWER(title) = LOWER(?2)
           AND (?3 IS NULL OR slug LIKE ?3 || '/%')
         LIMIT 2",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map(
            params![collection_id, title, role_prefix.as_deref()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
        .collect::<Result<_, _>>()?;
    if rows.len() == 1 {
        let (page_id, slug) = rows.into_iter().next().expect("len==1");
        Ok(Some(SurfaceResolution::Resolved { page_id, slug }))
    } else {
        Ok(None)
    }
}

fn lookup_by_basename_unique(
    conn: &Connection,
    collection_id: i64,
    basename: &str,
    role_hint: Option<&str>,
) -> Result<Option<SurfaceResolution>, rusqlite::Error> {
    let pattern_with_prefix = format!("%/{basename}");
    let role_prefix = role_hint.map(role_slug_prefix);
    let mut stmt = conn.prepare(
        "SELECT id, slug FROM pages \
         WHERE collection_id = ?1 \
            AND (slug = ?2 OR slug LIKE ?3) \
           AND (?4 IS NULL OR slug LIKE ?4 || '/%') \
          LIMIT 2",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map(
            params![
                collection_id,
                basename,
                pattern_with_prefix,
                role_prefix.as_deref()
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
        .collect::<Result<_, _>>()?;
    if rows.len() == 1 {
        let (page_id, slug) = rows.into_iter().next().expect("len==1");
        Ok(Some(SurfaceResolution::Resolved { page_id, slug }))
    } else {
        Ok(None)
    }
}

fn role_slug_prefix(role: &str) -> String {
    pluralise_role(role)
}

/// Summary counts produced by `route_entity_matches`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RoutingSummary {
    /// Total matches routed (resolved + unresolved).
    pub matches_seen: usize,
    /// Newly inserted assertions (post idempotency check).
    pub assertions_inserted: usize,
    /// Matches whose subject and object both resolved to a page.
    pub fully_resolved: usize,
    /// Matches with at least one unresolved or ambiguous surface.
    pub unresolved: usize,
}

/// Stale-assertion behavior for an entity-pattern routing run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityAssertionSync {
    /// Treat the provided matches as the complete page truth and delete stale
    /// `agent`/`entity_pattern` assertions not present in this run.
    DeleteStale,
    /// Treat the provided matches as a partial scan and preserve existing
    /// `agent`/`entity_pattern` assertions absent from this run.
    PreserveStale,
}

struct AssertionCandidate {
    subject: String,
    predicate: String,
    object: String,
    confidence: f64,
    evidence_text: String,
}

/// Route every `EntityMatch` to the `assertions` side table (task 7.3–7.5).
///
/// Per Decision 11 this change writes assertions only — **no `links` rows are
/// inserted with `source_kind = 'entity_pattern'`**. Resolved endpoint info is
/// recorded in `evidence_text` for downstream consumers.
///
/// Idempotency/retraction: each run syncs the live assertion set for this
/// source page and `entity_pattern`, inserting missing assertions, updating
/// retained evidence, and deleting stale entity-pattern assertions that no
/// longer appear in the page text.
pub fn route_entity_matches(
    conn: &Connection,
    source_page_id: i64,
    source_collection_id: i64,
    matches: &[EntityMatch],
) -> Result<RoutingSummary, EntityError> {
    route_entity_matches_with_sync(
        conn,
        source_page_id,
        source_collection_id,
        matches,
        EntityAssertionSync::DeleteStale,
    )
}

/// Route entity matches with an explicit stale-assertion policy.
pub fn route_entity_matches_with_sync(
    conn: &Connection,
    source_page_id: i64,
    source_collection_id: i64,
    matches: &[EntityMatch],
    sync: EntityAssertionSync,
) -> Result<RoutingSummary, EntityError> {
    let mut summary = RoutingSummary::default();
    let mut seen_in_batch: HashSet<(String, String, String)> = HashSet::new();
    let mut candidates = Vec::new();

    for m in matches {
        summary.matches_seen += 1;

        let subject_norm = canonical_surface(&m.subject_surface);
        let object_norm = canonical_surface(&m.object_surface);
        if subject_norm.is_empty() || object_norm.is_empty() {
            continue;
        }

        let subject_res = resolve_entity_surface(
            &m.subject_surface,
            m.subject_type.as_deref(),
            source_collection_id,
            conn,
        )?;
        let object_res = resolve_entity_surface(
            &m.object_surface,
            m.object_type.as_deref(),
            source_collection_id,
            conn,
        )?;

        let fully_resolved = matches!(subject_res, SurfaceResolution::Resolved { .. })
            && matches!(object_res, SurfaceResolution::Resolved { .. });
        if fully_resolved {
            summary.fully_resolved += 1;
        } else {
            summary.unresolved += 1;
        }

        let key = (
            subject_norm.clone(),
            m.relationship.clone(),
            object_norm.clone(),
        );
        if !seen_in_batch.insert(key) {
            continue;
        }

        let evidence_text = render_evidence(&subject_res, &object_res, m);
        candidates.push(AssertionCandidate {
            subject: subject_norm,
            predicate: m.relationship.clone(),
            object: object_norm,
            confidence: m.weight,
            evidence_text,
        });
    }

    for candidate in &candidates {
        // Persistent idempotency: skip insert if an identical assertion already exists.
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM assertions \
                 WHERE page_id = ?1
                   AND subject = ?2
                   AND predicate = ?3
                   AND object = ?4
                   AND asserted_by = 'agent'
                   AND source_ref = 'entity_pattern' \
                 LIMIT 1",
                params![
                    source_page_id,
                    &candidate.subject,
                    &candidate.predicate,
                    &candidate.object
                ],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(assertion_id) = existing {
            conn.execute(
                "UPDATE assertions
                 SET confidence = ?2, evidence_text = ?3
                 WHERE id = ?1",
                params![assertion_id, candidate.confidence, &candidate.evidence_text],
            )?;
            continue;
        }

        conn.execute(
            "INSERT INTO assertions (
                page_id, subject, predicate, object, valid_from, valid_until,
                confidence, asserted_by, source_ref, evidence_text
             ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, 'agent', 'entity_pattern', ?6)",
            params![
                source_page_id,
                &candidate.subject,
                &candidate.predicate,
                &candidate.object,
                candidate.confidence,
                &candidate.evidence_text,
            ],
        )?;
        summary.assertions_inserted += 1;
    }

    if sync == EntityAssertionSync::DeleteStale {
        delete_stale_entity_assertions(conn, source_page_id, &seen_in_batch)?;
    }

    Ok(summary)
}

fn delete_stale_entity_assertions(
    conn: &Connection,
    source_page_id: i64,
    desired_keys: &HashSet<(String, String, String)>,
) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, subject, predicate, object
         FROM assertions
         WHERE page_id = ?1
           AND asserted_by = 'agent'
           AND source_ref = 'entity_pattern'",
    )?;
    let rows: Vec<(i64, String, String, String)> = stmt
        .query_map([source_page_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<_, _>>()?;
    drop(stmt);

    for (id, subject, predicate, object) in rows {
        if !desired_keys.contains(&(subject, predicate, object)) {
            conn.execute("DELETE FROM assertions WHERE id = ?1", [id])?;
        }
    }
    Ok(())
}

fn canonical_surface(surface: &str) -> String {
    let normalized = resolve_slug(surface);
    if normalized.is_empty() {
        surface.trim().to_lowercase()
    } else {
        normalized
    }
}

fn render_evidence(
    subject: &SurfaceResolution,
    object: &SurfaceResolution,
    m: &EntityMatch,
) -> String {
    let subj = match subject {
        SurfaceResolution::Resolved { slug, .. } => format!("resolved:{slug}"),
        SurfaceResolution::Unresolved => "unresolved".to_owned(),
    };
    let obj = match object {
        SurfaceResolution::Resolved { slug, .. } => format!("resolved:{slug}"),
        SurfaceResolution::Unresolved => "unresolved".to_owned(),
    };
    format!(
        "entity_pattern[{}] subject={subj} object={obj}",
        m.relationship
    )
}

/// Run extraction + routing for a single page, logging a knowledge gap if the
/// per-page budget was exhausted (tasks 7.1, 7.2, 7.6).
///
/// Wired after the page write so failures here cannot corrupt the page row.
/// Pattern validation must have been performed up-front via `load_patterns`
/// (task 7.6 — malformed config fails before page mutation).
pub fn run_for_page(
    conn: &Connection,
    page_id: i64,
    collection_id: i64,
    page_slug: &str,
    compiled_truth: &str,
    patterns: &[EntityPattern],
) -> Result<RoutingSummary, EntityError> {
    run_for_page_with_deadline(
        conn,
        page_id,
        collection_id,
        page_slug,
        compiled_truth,
        patterns,
        EXTRACTION_BUDGET,
    )
}

/// Same as `run_for_page` but with an explicit extraction deadline. This is
/// used by command/backfill callers that need deterministic budget behavior.
pub fn run_for_page_with_deadline(
    conn: &Connection,
    page_id: i64,
    collection_id: i64,
    page_slug: &str,
    compiled_truth: &str,
    patterns: &[EntityPattern],
    deadline: Duration,
) -> Result<RoutingSummary, EntityError> {
    let outcome = extract_entities(compiled_truth, patterns, deadline);
    if outcome.over_budget {
        let context = format!(
            "entity-pattern extraction exceeded 5ms budget on page {page_slug} \
             (patterns_run={}, elapsed_us={})",
            outcome.patterns_run,
            outcome.elapsed.as_micros()
        );
        gaps::log_gap_for_page(
            page_id,
            &format!("entity_pattern_budget:{page_slug}"),
            &context,
            None,
            conn,
        )?;
        return route_entity_matches_with_sync(
            conn,
            page_id,
            collection_id,
            &outcome.matches,
            EntityAssertionSync::PreserveStale,
        );
    }
    route_entity_matches(conn, page_id, collection_id, &outcome.matches)
}

/// Best-effort write-path wrapper used by `put` / `ingest`: runs extraction
/// for the page, logs but does not propagate failures so they cannot corrupt
/// the underlying page write (task 7.6).
pub fn try_run_for_page(
    conn: &Connection,
    page_id: i64,
    collection_id: i64,
    page_slug: &str,
    compiled_truth: &str,
    patterns: &[EntityPattern],
) {
    if patterns.is_empty() {
        return;
    }
    if let Err(err) = run_for_page(
        conn,
        page_id,
        collection_id,
        page_slug,
        compiled_truth,
        patterns,
    ) {
        // Best-effort: extraction failure must never corrupt the page write.
        // Surface to logs only.
        eprintln!("entity extraction failed for {page_slug}: {err}");
    }
}

// ============================================================
// Static no-LLM / no-inference proof (task 7.7)
// ============================================================
//
// The build-time check below guarantees that `entities.rs` does not depend on
// the embedding/inference module. It is exercised by `tests/no_llm_static.rs`
// which greps the compiled file. We additionally provide a runtime helper
// that returns the embedded source for test assertions.

/// Returns the embedded default-patterns YAML body. Used by tests to assert
/// every required relationship is present without re-reading the file.
pub fn default_patterns_source() -> &'static str {
    DEFAULT_PATTERNS_YAML
}

/// Returns the path of this source file as known at compile time. Used by the
/// static no-LLM/no-network audit test.
pub fn source_file_path() -> &'static str {
    file!()
}
