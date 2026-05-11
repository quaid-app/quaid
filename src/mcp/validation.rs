//! Input-shape validators for the MCP surface. Functions in this module
//! reject malformed slugs, tags, timestamps, and tokens by emitting
//! `rmcp::Error` with JSON-RPC error code `-32602` (invalid-params). This is
//! one of the two locations where direct `rmcp::Error` construction is
//! sanctioned (the other being `mcp::errors`); validators emit `-32602` as a
//! control-flow primitive that predates the `map_*_error` helper convention.
//! The `MAX_*` size limits and the temporal-format helpers also live here
//! because they parameterise the validators. The block was extracted
//! verbatim from `server.rs:38–356` during the
//! `decompose-mcp-server-module` change.

use crate::core::graph::TemporalFilter;
use crate::core::vault_sync;
use crate::mcp::errors::invalid_params;
use rmcp::model::ErrorCode;

/// Upper bound on the byte length of a slug accepted by `validate_slug`.
pub const MAX_SLUG_LEN: usize = 512;
/// Upper bound on the byte length of a page content body accepted by
/// `validate_content` (1 MB).
pub const MAX_CONTENT_LEN: usize = 1_048_576; // 1 MB
/// Maximum value any caller-supplied `limit` field on a list-style tool may
/// take; values above this are clamped at the tool body.
pub const MAX_LIMIT: u32 = 1000;
/// Upper bound on the byte length of a relationship token (`memory_link`'s
/// `relationship` field).
pub const MAX_RELATIONSHIP_LEN: usize = 64;
/// Upper bound on the byte length of any single tag token.
pub const MAX_TAG_LEN: usize = 64;
/// Upper bound on the number of tags any single `memory_tags` request may
/// add or remove.
pub const MAX_TAGS_PER_REQUEST: usize = 100;
/// Upper bound on the byte length of the `context` field accepted by
/// `memory_gap`.
pub const MAX_GAP_CONTEXT_LEN: usize = 500;
/// Upper bound on the serialised byte length of the `data` payload accepted
/// by `memory_raw` (1 MB).
pub const MAX_RAW_DATA_LEN: usize = 1_048_576; // 1 MB

/// Validate that a slug is non-empty, within length limits, and a
/// well-formed `vault_sync::parse_slug_input` candidate.
pub fn validate_slug(slug: &str) -> Result<(), rmcp::Error> {
    if slug.is_empty() {
        return Err(invalid_params("invalid slug: must not be empty"));
    }
    if slug.len() > MAX_SLUG_LEN {
        return Err(invalid_params(format!(
            "invalid slug: exceeds maximum length of {MAX_SLUG_LEN} characters"
        )));
    }
    vault_sync::parse_slug_input(slug).map_err(|err| invalid_params(err.to_string()))
}

/// Validate that page content does not exceed the 1 MB byte budget.
pub fn validate_content(content: &str) -> Result<(), rmcp::Error> {
    if content.len() > MAX_CONTENT_LEN {
        return Err(invalid_params(format!(
            "content too large: {} bytes exceeds maximum of {MAX_CONTENT_LEN} bytes",
            content.len()
        )));
    }
    Ok(())
}

/// Validate the `status` argument to `memory_close_action`.
pub fn validate_close_action_status(status: &str) -> Result<(), rmcp::Error> {
    match status {
        "done" | "cancelled" => Ok(()),
        other => Err(invalid_params(format!(
            "invalid status: expected 'done' or 'cancelled', got '{other}'"
        ))),
    }
}

/// Generic per-byte token validator used by tag and relationship checks.
pub fn validate_token(
    value: &str,
    field: &str,
    max_len: usize,
    allowed: fn(u8) -> bool,
    allowed_hint: &str,
) -> Result<(), rmcp::Error> {
    if value.is_empty() {
        return Err(invalid_params(format!(
            "invalid {field}: must not be empty"
        )));
    }
    if value.len() > max_len {
        return Err(invalid_params(format!(
            "invalid {field}: exceeds maximum length of {max_len} characters"
        )));
    }
    if !value.bytes().all(allowed) {
        return Err(invalid_params(format!(
            "invalid {field}: allowed characters are {allowed_hint}"
        )));
    }
    Ok(())
}

/// True if the byte is permitted in a tag or relationship name.
pub fn is_tag_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
}

/// Validate a relationship token (e.g. `works_at`).
pub fn validate_relationship(relationship: &str) -> Result<(), rmcp::Error> {
    validate_token(
        relationship,
        "relationship",
        MAX_RELATIONSHIP_LEN,
        is_tag_byte,
        "[a-z0-9_-]",
    )
}

/// Validate a list of tag tokens.
pub fn validate_tag_list(tags: &[String], field: &str) -> Result<(), rmcp::Error> {
    if tags.len() > MAX_TAGS_PER_REQUEST {
        return Err(invalid_params(format!(
            "invalid {field}: exceeds maximum of {MAX_TAGS_PER_REQUEST} tags"
        )));
    }
    for tag in tags {
        validate_token(tag, "tag", MAX_TAG_LEN, is_tag_byte, "[a-z0-9_-]")?;
    }
    Ok(())
}

fn parse_component(value: &str, start: usize, len: usize) -> Option<u32> {
    value.get(start..start + len)?.parse().ok()
}

/// True if `value` is a YYYY-MM, YYYY-MM-DD, or YYYY-MM-DDTHH:MM:SSZ
/// timestamp. Used by `validate_temporal_value` and `validate_turn_timestamp`.
pub fn is_valid_temporal_value(value: &str) -> bool {
    match value.len() {
        7 => matches!(
            (parse_component(value, 0, 4), value.as_bytes().get(4), parse_component(value, 5, 2)),
            (Some(_year), Some(b'-'), Some(month)) if (1..=12).contains(&month)
        ),
        10 => matches!(
            (
                parse_component(value, 0, 4),
                value.as_bytes().get(4),
                parse_component(value, 5, 2),
                value.as_bytes().get(7),
                parse_component(value, 8, 2)
            ),
            (Some(_year), Some(b'-'), Some(month), Some(b'-'), Some(day))
                if (1..=12).contains(&month) && (1..=31).contains(&day)
        ),
        20 => matches!(
            (
                parse_component(value, 0, 4),
                value.as_bytes().get(4),
                parse_component(value, 5, 2),
                value.as_bytes().get(7),
                parse_component(value, 8, 2),
                value.as_bytes().get(10),
                parse_component(value, 11, 2),
                value.as_bytes().get(13),
                parse_component(value, 14, 2),
                value.as_bytes().get(16),
                parse_component(value, 17, 2),
                value.as_bytes().get(19)
            ),
            (
                Some(_year),
                Some(b'-'),
                Some(month),
                Some(b'-'),
                Some(day),
                Some(b'T'),
                Some(hour),
                Some(b':'),
                Some(minute),
                Some(b':'),
                Some(second),
                Some(b'Z')
            ) if (1..=12).contains(&month)
                && (1..=31).contains(&day)
                && hour <= 23
                && minute <= 59
                && second <= 59
        ),
        _ => false,
    }
}

/// Validate a YYYY-MM, YYYY-MM-DD, or YYYY-MM-DDTHH:MM:SSZ temporal value.
pub fn validate_temporal_value(value: &str, field: &str) -> Result<(), rmcp::Error> {
    if is_valid_temporal_value(value) {
        Ok(())
    } else {
        Err(invalid_params(format!(
            "invalid {field}: expected YYYY-MM, YYYY-MM-DD, or YYYY-MM-DDTHH:MM:SSZ"
        )))
    }
}

/// Validate a conversation-turn timestamp (full YYYY-MM-DDTHH:MM:SSZ form).
pub fn validate_turn_timestamp(value: &str) -> Result<(), rmcp::Error> {
    if value.len() == 20 && is_valid_temporal_value(value) {
        Ok(())
    } else {
        Err(invalid_params(
            "invalid timestamp: expected YYYY-MM-DDTHH:MM:SSZ".to_owned(),
        ))
    }
}

/// Parse the optional `temporal` filter shared by graph/backlinks tools.
pub fn parse_temporal_filter(temporal: Option<&str>) -> Result<TemporalFilter, rmcp::Error> {
    match temporal.unwrap_or("active") {
        "active" | "current" => Ok(TemporalFilter::Active),
        "all" | "history" => Ok(TemporalFilter::All),
        other => Err(rmcp::Error::new(
            ErrorCode(-32602),
            format!("invalid temporal filter: {other}"),
            None,
        )),
    }
}
