// Consumers (commands/, mcp/) not yet wired — remove when they are.
#![allow(dead_code)]
#![expect(
    clippy::expect_used,
    reason = "addressed in remove-production-panic-paths"
)]

use regex::Regex;
use serde_json::Value as JsonValue;
use thiserror::Error;

use super::types::Frontmatter;

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
