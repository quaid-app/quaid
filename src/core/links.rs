// Consumers (commands/, mcp/) not yet wired — remove when they are.
#![allow(dead_code)]
#![expect(
    clippy::expect_used,
    reason = "addressed in remove-production-panic-paths"
)]

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use thiserror::Error;

use super::gaps::{log_gap_for_page, GapsError};
use super::types::Frontmatter;

/// Derived-edge `source_kind` values that participate in the partial unique
/// index `idx_links_unique_derived_edge`. Mirrors the schema CHECK constraint.
pub const DERIVED_EDGE_KINDS: [&str; 3] = ["wiki_link", "frontmatter", "entity_pattern"];

/// Default edge weight for wiki-link derived edges when config is unreadable.
const DEFAULT_WIKILINK_EDGE_WEIGHT: f64 = 0.5;
/// Default edge weight for frontmatter derived edges when config is unreadable.
const DEFAULT_FRONTMATTER_EDGE_WEIGHT: f64 = 1.0;

/// Default relationship for `links:` string-shorthand and `related:` entries.
pub const DEFAULT_LINK_RELATIONSHIP: &str = "related";
/// Relationship used when expanding a `parent:` frontmatter field.
pub const PARENT_RELATIONSHIP: &str = "parent";
/// Relationship used when expanding `children:` frontmatter entries.
pub const CHILD_RELATIONSHIP: &str = "child";

/// A single derived edge candidate produced from a page's frontmatter.
///
/// This is a parse-layer value: `target` is already normalized via
/// `resolve_slug`, but no database resolution or insertion has happened yet.
/// Wave 3 (`sync_frontmatter_edges`) is responsible for resolving `target` to a
/// concrete `to_page_id` and upserting a `links` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterLink {
    /// Slug-normalized target page, e.g. `companies/brex-inc`.
    pub target: String,
    /// Edge relationship label (e.g. `related`, `parent`, `child`, `founded`).
    pub relationship: String,
    /// Optional ISO-8601 date string marking when the edge becomes valid.
    pub valid_from: Option<String>,
    /// Optional ISO-8601 date string marking when the edge stops being valid.
    pub valid_until: Option<String>,
}

/// Frontmatter-link parse failures surfaced before any DB mutation.
///
/// Every variant carries the offending `field` (e.g. `links[2]`, `parent`,
/// `children[0]`) so error messages are actionable when bubbled up by validate
/// or write paths.
#[derive(Debug, Error, PartialEq, Eq)]
#[allow(
    missing_docs,
    reason = "variant/field semantics encoded in #[error] messages"
)]
pub enum FrontmatterParseError {
    #[error("frontmatter `{field}`: expected {expected}, found {found}")]
    InvalidShape {
        field: String,
        expected: String,
        found: String,
    },
    #[error("frontmatter `{field}`: object link entry is missing required `target` field")]
    MissingTarget { field: String },
    #[error("frontmatter `{field}`: `target` must resolve to a non-empty slug")]
    EmptyTarget { field: String },
    #[error("frontmatter `{field}`: `{key}` must be a string, found {found}")]
    InvalidStringField {
        field: String,
        key: String,
        found: String,
    },
    #[error("frontmatter `{field}`: `{key}` must be a non-empty string")]
    EmptyStringField { field: String, key: String },
    #[error("frontmatter `{field}`: unknown key `{key}` (allowed: target, type, valid_from, valid_until)")]
    UnknownKey { field: String, key: String },
}

/// Expand all derived-edge candidates from a page's structured frontmatter.
///
/// Reads the canonical `links:` field (object form or string shorthand) plus
/// the fixed-relationship fields `parent:`, `children:`, and `related:`.
/// Targets are normalized via `resolve_slug` so callers can match directly
/// against `pages.slug`.
///
/// Tags (`tags:`) are NOT expanded as edges; see [`expand_frontmatter_tags`].
///
/// # Errors
/// Returns [`FrontmatterParseError`] for malformed entries (wrong shape,
/// missing `target`, non-string temporal fields, unknown object keys, etc.).
/// Callers in validate/write paths must fail closed before mutating state.
pub fn expand_frontmatter_edges(
    frontmatter: &Frontmatter,
) -> Result<Vec<FrontmatterLink>, FrontmatterParseError> {
    let mut edges = Vec::new();

    if let Some(value) = frontmatter.get("links") {
        expand_links_field(value, &mut edges)?;
    }

    if let Some(value) = frontmatter.get("parent") {
        expand_single_relationship_field("parent", PARENT_RELATIONSHIP, value, &mut edges)?;
    }

    if let Some(value) = frontmatter.get("children") {
        expand_list_relationship_field("children", CHILD_RELATIONSHIP, value, &mut edges)?;
    }

    if let Some(value) = frontmatter.get("related") {
        expand_list_relationship_field("related", DEFAULT_LINK_RELATIONSHIP, value, &mut edges)?;
    }

    Ok(edges)
}

/// Expand a page's frontmatter `tags:` field into a flat list of tag labels.
///
/// Accepts either a YAML list (`tags: [a, b]`) or a comma-separated scalar
/// string (`tags: "a, b"`). Returns labels in source order with surrounding
/// whitespace trimmed and empty entries discarded; duplicates are preserved
/// for the sync layer to dedupe alongside its `tags`-table writes.
///
/// Tags are intentionally label-only at this layer — they MUST NOT become
/// graph edges. See `specs/frontmatter-link-autowiring/spec.md`.
pub fn expand_frontmatter_tags(frontmatter: &Frontmatter) -> Vec<String> {
    let Some(value) = frontmatter.get("tags") else {
        return Vec::new();
    };

    match value {
        JsonValue::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        JsonValue::String(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn expand_links_field(
    value: &JsonValue,
    edges: &mut Vec<FrontmatterLink>,
) -> Result<(), FrontmatterParseError> {
    let JsonValue::Array(items) = value else {
        return Err(FrontmatterParseError::InvalidShape {
            field: "links".to_string(),
            expected: "list of strings or objects".to_string(),
            found: shape_label(value),
        });
    };

    for (index, item) in items.iter().enumerate() {
        let field = format!("links[{index}]");
        match item {
            JsonValue::String(raw) => {
                edges.push(make_string_edge(&field, raw, DEFAULT_LINK_RELATIONSHIP)?);
            }
            JsonValue::Object(map) => {
                edges.push(parse_object_link(&field, map)?);
            }
            other => {
                return Err(FrontmatterParseError::InvalidShape {
                    field,
                    expected: "string or object".to_string(),
                    found: shape_label(other),
                });
            }
        }
    }
    Ok(())
}

fn expand_single_relationship_field(
    field: &str,
    relationship: &str,
    value: &JsonValue,
    edges: &mut Vec<FrontmatterLink>,
) -> Result<(), FrontmatterParseError> {
    match value {
        JsonValue::String(raw) => {
            edges.push(make_string_edge(field, raw, relationship)?);
            Ok(())
        }
        JsonValue::Null => Ok(()),
        other => Err(FrontmatterParseError::InvalidShape {
            field: field.to_string(),
            expected: "string".to_string(),
            found: shape_label(other),
        }),
    }
}

fn expand_list_relationship_field(
    field: &str,
    relationship: &str,
    value: &JsonValue,
    edges: &mut Vec<FrontmatterLink>,
) -> Result<(), FrontmatterParseError> {
    match value {
        JsonValue::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let entry_field = format!("{field}[{index}]");
                let JsonValue::String(raw) = item else {
                    return Err(FrontmatterParseError::InvalidShape {
                        field: entry_field,
                        expected: "string".to_string(),
                        found: shape_label(item),
                    });
                };
                edges.push(make_string_edge(&entry_field, raw, relationship)?);
            }
            Ok(())
        }
        JsonValue::Null => Ok(()),
        other => Err(FrontmatterParseError::InvalidShape {
            field: field.to_string(),
            expected: "list of strings".to_string(),
            found: shape_label(other),
        }),
    }
}

fn parse_object_link(
    field: &str,
    map: &serde_json::Map<String, JsonValue>,
) -> Result<FrontmatterLink, FrontmatterParseError> {
    for key in map.keys() {
        match key.as_str() {
            "target" | "type" | "valid_from" | "valid_until" => {}
            other => {
                return Err(FrontmatterParseError::UnknownKey {
                    field: field.to_string(),
                    key: other.to_string(),
                });
            }
        }
    }

    let target_raw = map
        .get("target")
        .ok_or_else(|| FrontmatterParseError::MissingTarget {
            field: field.to_string(),
        })?;
    let target_str = require_string(field, "target", target_raw)?;
    let target = resolve_slug(target_str);
    if target.is_empty() {
        return Err(FrontmatterParseError::EmptyTarget {
            field: field.to_string(),
        });
    }

    let relationship = match map.get("type") {
        Some(JsonValue::Null) | None => DEFAULT_LINK_RELATIONSHIP.to_string(),
        Some(value) => {
            let raw = require_string(field, "type", value)?;
            if raw.trim().is_empty() {
                return Err(FrontmatterParseError::EmptyStringField {
                    field: field.to_string(),
                    key: "type".to_string(),
                });
            }
            raw.to_string()
        }
    };

    let valid_from = optional_string_field(field, "valid_from", map.get("valid_from"))?;
    let valid_until = optional_string_field(field, "valid_until", map.get("valid_until"))?;

    Ok(FrontmatterLink {
        target,
        relationship,
        valid_from,
        valid_until,
    })
}

fn make_string_edge(
    field: &str,
    raw: &str,
    relationship: &str,
) -> Result<FrontmatterLink, FrontmatterParseError> {
    let target = resolve_slug(raw);
    if target.is_empty() {
        return Err(FrontmatterParseError::EmptyTarget {
            field: field.to_string(),
        });
    }
    Ok(FrontmatterLink {
        target,
        relationship: relationship.to_string(),
        valid_from: None,
        valid_until: None,
    })
}

fn require_string<'a>(
    field: &str,
    key: &str,
    value: &'a JsonValue,
) -> Result<&'a str, FrontmatterParseError> {
    value
        .as_str()
        .ok_or_else(|| FrontmatterParseError::InvalidStringField {
            field: field.to_string(),
            key: key.to_string(),
            found: shape_label(value),
        })
}

fn optional_string_field(
    field: &str,
    key: &str,
    value: Option<&JsonValue>,
) -> Result<Option<String>, FrontmatterParseError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(raw)) => {
            if raw.trim().is_empty() {
                Err(FrontmatterParseError::EmptyStringField {
                    field: field.to_string(),
                    key: key.to_string(),
                })
            } else {
                Ok(Some(raw.clone()))
            }
        }
        Some(other) => Err(FrontmatterParseError::InvalidStringField {
            field: field.to_string(),
            key: key.to_string(),
            found: shape_label(other),
        }),
    }
}

fn shape_label(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "null".to_string(),
        JsonValue::Bool(_) => "boolean".to_string(),
        JsonValue::Number(_) => "number".to_string(),
        JsonValue::String(_) => "string".to_string(),
        JsonValue::Array(_) => "list".to_string(),
        JsonValue::Object(_) => "object".to_string(),
    }
}

/// Extract `[[slug]]` wiki-link patterns from markdown content.
pub fn extract_links(content: &str) -> Vec<String> {
    let re = Regex::new(r"\[\[([^\[\]]+)\]\]").expect("valid regex");
    re.captures_iter(content)
        .map(|cap| resolve_slug(&cap[1]))
        .collect()
}

/// Normalise a raw slug to lowercase kebab-case.
///
/// - Lowercases the entire string
/// - Replaces spaces with hyphens
/// - Strips leading and trailing slashes
/// - Collapses multiple consecutive slashes into one
pub fn resolve_slug(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    let replaced = lower.replace(' ', "-");
    let stripped = replaced.trim_matches('/');
    let mut result = String::with_capacity(stripped.len());
    let mut prev_slash = false;
    for ch in stripped.chars() {
        if ch == '/' {
            if !prev_slash {
                result.push(ch);
            }
            prev_slash = true;
        } else {
            result.push(ch);
            prev_slash = false;
        }
    }
    result
}

// ============================================================
// Wave 2 — Derived edge upsert + sync primitives (tasks 4.1–4.6)
// ============================================================

/// Errors surfaced from derived-edge sync (`upsert_derived_edge`,
/// `sync_frontmatter_edges`, `sync_wikilink_edges`).
#[derive(Debug, Error)]
#[allow(
    missing_docs,
    reason = "variant semantics encoded in #[error] messages"
)]
pub enum DerivedEdgeError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("knowledge-gap log error: {0}")]
    Gap(#[from] GapsError),

    #[error("source_kind `{0}` is not a derived edge kind")]
    NonDerivedKind(String),
}

/// Upsert a single derived edge using the partial unique index
/// `idx_links_unique_derived_edge` as the conflict target.
///
/// On conflict the row's `valid_from`, `valid_until`, `edge_weight`, and
/// `context` are replaced with the incoming values. The `source_kind` MUST be
/// one of [`DERIVED_EDGE_KINDS`]; `programmatic` links are deliberately not
/// upsertable here so their temporal history is preserved.
#[allow(clippy::too_many_arguments, reason = "matches OpenSpec contract 4.1")]
pub fn upsert_derived_edge(
    conn: &Connection,
    from_page_id: i64,
    to_page_id: i64,
    relationship: &str,
    source_kind: &str,
    edge_weight: f64,
    valid_from: Option<&str>,
    valid_until: Option<&str>,
    context: &str,
) -> Result<(), DerivedEdgeError> {
    if !DERIVED_EDGE_KINDS.contains(&source_kind) {
        return Err(DerivedEdgeError::NonDerivedKind(source_kind.to_string()));
    }

    conn.execute(
        "INSERT INTO links \
            (from_page_id, to_page_id, relationship, source_kind, edge_weight, valid_from, valid_until, context) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(from_page_id, to_page_id, relationship, source_kind) \
            WHERE source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern') \
         DO UPDATE SET \
            edge_weight = excluded.edge_weight, \
            valid_from  = excluded.valid_from, \
            valid_until = excluded.valid_until, \
            context     = excluded.context",
        params![
            from_page_id,
            to_page_id,
            relationship,
            source_kind,
            edge_weight,
            valid_from,
            valid_until,
            context,
        ],
    )?;
    Ok(())
}

/// Sync frontmatter-derived edges for a single source page.
///
/// Upserts every incoming [`FrontmatterLink`] whose target slug resolves to a
/// page in the same collection, then deletes any `frontmatter` rows for the
/// same source page that are NOT in the incoming set. Unresolved targets are
/// logged once per `(source_page, target, relationship)` triple via
/// [`log_gap_for_page`] using a deterministic structural query string so
/// repeated writes are deduped through the existing SHA-256 conflict key.
///
/// `programmatic`, `wiki_link`, and `entity_pattern` rows are never touched.
pub fn sync_frontmatter_edges(
    conn: &Connection,
    page_id: i64,
    collection_id: i64,
    edges: &[FrontmatterLink],
) -> Result<(), DerivedEdgeError> {
    let weight = read_edge_weight(
        conn,
        "edge_weight_frontmatter",
        DEFAULT_FRONTMATTER_EDGE_WEIGHT,
    );
    let source_slug = lookup_slug(conn, page_id)?;

    let mut keep: HashSet<(i64, String)> = HashSet::new();
    for edge in edges {
        match resolve_target_in_collection(conn, collection_id, &edge.target)? {
            Some(target_id) => {
                upsert_derived_edge(
                    conn,
                    page_id,
                    target_id,
                    &edge.relationship,
                    "frontmatter",
                    weight,
                    edge.valid_from.as_deref(),
                    edge.valid_until.as_deref(),
                    "",
                )?;
                keep.insert((target_id, edge.relationship.clone()));
            }
            None => {
                log_unresolved_target(
                    conn,
                    page_id,
                    source_slug.as_deref(),
                    "frontmatter",
                    &edge.relationship,
                    &edge.target,
                )?;
            }
        }
    }

    delete_stale_derived_rows(conn, page_id, "frontmatter", &keep)?;
    Ok(())
}

/// Sync wiki-link derived edges for a single source page.
///
/// Extracts `[[slug]]` references from `compiled_truth` and `timeline`,
/// resolves each target slug within the source page's collection, upserts a
/// `wiki_link` edge with relationship `related` and the configured wikilink
/// weight, then deletes any `wiki_link` rows for that source page whose
/// target/relationship is not in the incoming set. `programmatic` and other
/// derived kinds are never touched. Unresolved targets are logged via the
/// same dedup-by-hash path used by frontmatter sync.
pub fn sync_wikilink_edges(
    conn: &Connection,
    page_id: i64,
    collection_id: i64,
    compiled_truth: &str,
    timeline: &str,
) -> Result<(), DerivedEdgeError> {
    let weight = read_edge_weight(conn, "edge_weight_wikilink", DEFAULT_WIKILINK_EDGE_WEIGHT);
    let source_slug = lookup_slug(conn, page_id)?;

    let mut seen_targets: HashSet<String> = HashSet::new();
    let mut targets: Vec<String> = Vec::new();
    for body in [compiled_truth, timeline] {
        for raw in extract_links(body) {
            if raw.is_empty() {
                continue;
            }
            if seen_targets.insert(raw.clone()) {
                targets.push(raw);
            }
        }
    }

    let mut keep: HashSet<(i64, String)> = HashSet::new();
    for target in &targets {
        match resolve_target_in_collection(conn, collection_id, target)? {
            Some(target_id) => {
                upsert_derived_edge(
                    conn,
                    page_id,
                    target_id,
                    DEFAULT_LINK_RELATIONSHIP,
                    "wiki_link",
                    weight,
                    None,
                    None,
                    "",
                )?;
                keep.insert((target_id, DEFAULT_LINK_RELATIONSHIP.to_string()));
            }
            None => {
                log_unresolved_target(
                    conn,
                    page_id,
                    source_slug.as_deref(),
                    "wiki_link",
                    DEFAULT_LINK_RELATIONSHIP,
                    target,
                )?;
            }
        }
    }

    delete_stale_derived_rows(conn, page_id, "wiki_link", &keep)?;
    Ok(())
}

fn delete_stale_derived_rows(
    conn: &Connection,
    page_id: i64,
    source_kind: &str,
    keep: &HashSet<(i64, String)>,
) -> Result<(), DerivedEdgeError> {
    let mut stmt = conn.prepare(
        "SELECT id, to_page_id, relationship \
         FROM links \
         WHERE from_page_id = ?1 AND source_kind = ?2",
    )?;
    let rows = stmt
        .query_map(params![page_id, source_kind], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for (id, to_page_id, relationship) in rows {
        if !keep.contains(&(to_page_id, relationship)) {
            conn.execute("DELETE FROM links WHERE id = ?1", params![id])?;
        }
    }
    Ok(())
}

fn resolve_target_in_collection(
    conn: &Connection,
    collection_id: i64,
    slug: &str,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.query_row(
        "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
        params![collection_id, slug],
        |row| row.get::<_, i64>(0),
    )
    .optional()
}

fn lookup_slug(conn: &Connection, page_id: i64) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT slug FROM pages WHERE id = ?1",
        params![page_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
}

fn read_edge_weight(conn: &Connection, key: &str, default: f64) -> f64 {
    conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .ok()
    .and_then(|v| v.parse::<f64>().ok())
    .unwrap_or(default)
}

fn log_unresolved_target(
    conn: &Connection,
    page_id: i64,
    source_slug: Option<&str>,
    source_kind: &str,
    relationship: &str,
    target: &str,
) -> Result<(), GapsError> {
    let source = source_slug.unwrap_or("<unknown>");
    let query = format!("unresolved-link:{source_kind}:{source}->{target}:{relationship}");
    let context = format!(
        "derived {source_kind} edge target `{target}` did not resolve in source collection"
    );
    log_gap_for_page(page_id, &query, &context, None, conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod extract_links {
        use super::*;

        #[test]
        fn finds_single_wikilink() {
            let links = extract_links("See [[people/alice]] for details.");
            assert_eq!(links, vec!["people/alice"]);
        }

        #[test]
        fn finds_multiple_wikilinks() {
            let links = extract_links("See [[people/alice]] and [[companies/acme]].");
            assert_eq!(links, vec!["people/alice", "companies/acme"]);
        }

        #[test]
        fn returns_empty_for_no_links() {
            let links = extract_links("No links here.");
            assert!(links.is_empty());
        }

        #[test]
        fn normalises_extracted_slugs() {
            let links = extract_links("See [[People/Alice Jones]].");
            assert_eq!(links, vec!["people/alice-jones"]);
        }
    }

    mod resolve_slug_tests {
        use super::*;

        #[test]
        fn lowercases_and_replaces_spaces() {
            assert_eq!(resolve_slug("People/Alice Jones"), "people/alice-jones");
        }

        #[test]
        fn strips_leading_trailing_slashes() {
            assert_eq!(resolve_slug("/people/alice/"), "people/alice");
        }

        #[test]
        fn collapses_multiple_slashes() {
            assert_eq!(resolve_slug("people///alice"), "people/alice");
        }

        #[test]
        fn handles_empty_string() {
            assert_eq!(resolve_slug(""), "");
        }
    }
}
