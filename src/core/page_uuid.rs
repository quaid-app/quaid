//! Stable per-page UUID derivation, validation, and frontmatter
//! canonicalization. UUIDs are generated as v7 (time-ordered), persisted on
//! the page row, and mirrored into the markdown frontmatter so external edits
//! can survive a slug change without losing identity.
//!
//! See also: `markdown` for frontmatter parsing, `types` for the
//! `Frontmatter` map type and helpers.

use thiserror::Error;
use uuid::Uuid;

use crate::core::types::{frontmatter_get_str, frontmatter_insert_string, Frontmatter};

/// Frontmatter key under which the page's stable UUID is stored.
pub const QUAID_ID_FRONTMATTER_KEY: &str = "quaid_id";
/// Legacy frontmatter key from the pre-rename `memory` era; still accepted on read.
pub const LEGACY_MEMORY_ID_FRONTMATTER_KEY: &str = "memory_id";

/// Failure mode raised when parsing or reconciling a page's UUID against its
/// stored value and frontmatter representation.
#[derive(Debug, Error)]
pub enum PageUuidError {
    /// Frontmatter contained an empty `quaid_id`/`memory_id` value.
    #[error("frontmatter quaid_id/memory_id cannot be empty")]
    EmptyFrontmatterUuid,

    /// Frontmatter UUID failed to parse as a valid UUID.
    #[error("invalid frontmatter quaid_id/memory_id: {value}")]
    InvalidFrontmatterUuid {
        /// The unparseable frontmatter value as-supplied.
        value: String,
    },

    /// Frontmatter and stored UUID both exist but disagree.
    #[error(
        "frontmatter quaid_id/memory_id {frontmatter_uuid} does not match stored page uuid {stored_uuid}"
    )]
    UuidMismatch {
        /// UUID currently stored on the page row.
        stored_uuid: String,
        /// UUID found in the frontmatter.
        frontmatter_uuid: String,
    },
}

/// Returns a freshly-generated time-ordered (v7) UUID as a hyphenated string.
pub fn generate_uuid_v7() -> String {
    Uuid::now_v7().to_string()
}

/// Reads the page UUID out of frontmatter (preferring `quaid_id`, falling back
/// to legacy `memory_id`), returning `Ok(None)` when neither key is present.
pub fn parse_frontmatter_uuid(frontmatter: &Frontmatter) -> Result<Option<String>, PageUuidError> {
    let Some(raw_uuid) = frontmatter
        .get(QUAID_ID_FRONTMATTER_KEY)
        .and_then(|value| value.as_str())
        .or_else(|| frontmatter_get_str(frontmatter, LEGACY_MEMORY_ID_FRONTMATTER_KEY))
    else {
        return Ok(None);
    };

    let trimmed = raw_uuid.trim();
    if trimmed.is_empty() {
        return Err(PageUuidError::EmptyFrontmatterUuid);
    }

    Uuid::parse_str(trimmed)
        .map(|uuid| Some(uuid.to_string()))
        .map_err(|_| PageUuidError::InvalidFrontmatterUuid {
            value: raw_uuid.to_string(),
        })
}

/// Reconciles the frontmatter UUID with the page's stored UUID: the stored
/// value wins when both are present and agree, a mismatch is an error, and a
/// missing-on-both case falls back to a fresh v7 generation.
pub fn resolve_page_uuid(
    frontmatter: &Frontmatter,
    stored_uuid: Option<&str>,
) -> Result<String, PageUuidError> {
    let frontmatter_uuid = parse_frontmatter_uuid(frontmatter)?;

    match (stored_uuid, frontmatter_uuid) {
        (Some(stored_uuid), Some(frontmatter_uuid)) if stored_uuid != frontmatter_uuid => {
            Err(PageUuidError::UuidMismatch {
                stored_uuid: stored_uuid.to_string(),
                frontmatter_uuid,
            })
        }
        (Some(stored_uuid), _) => Ok(stored_uuid.to_string()),
        (None, Some(frontmatter_uuid)) => Ok(frontmatter_uuid),
        (None, None) => Ok(generate_uuid_v7()),
    }
}

/// Rewrites the frontmatter so it contains exactly one `quaid_id` entry equal
/// to `stored_uuid`, removing any legacy `memory_id` along the way.
pub fn canonicalize_frontmatter_uuid(frontmatter: &mut Frontmatter, stored_uuid: &str) {
    frontmatter.remove(LEGACY_MEMORY_ID_FRONTMATTER_KEY);
    frontmatter.remove(QUAID_ID_FRONTMATTER_KEY);

    if !stored_uuid.trim().is_empty() {
        frontmatter_insert_string(frontmatter, QUAID_ID_FRONTMATTER_KEY, stored_uuid);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::core::types::{string_frontmatter, Frontmatter};

    use super::{
        canonicalize_frontmatter_uuid, generate_uuid_v7, parse_frontmatter_uuid, resolve_page_uuid,
        PageUuidError, LEGACY_MEMORY_ID_FRONTMATTER_KEY, QUAID_ID_FRONTMATTER_KEY,
    };

    #[test]
    fn generate_uuid_v7_returns_a_uuid_string() {
        let uuid = generate_uuid_v7();
        let parsed = uuid::Uuid::parse_str(&uuid).expect("generated uuid should parse");

        assert_eq!(parsed.get_version_num(), 7);
    }

    #[test]
    fn parse_frontmatter_uuid_preserves_present_quaid_id() {
        let frontmatter = string_frontmatter([(
            QUAID_ID_FRONTMATTER_KEY.to_string(),
            "01969f11-9448-7d79-8d3f-c68f54761234".to_string(),
        )]);

        let parsed = parse_frontmatter_uuid(&frontmatter).expect("quaid_id should parse");

        assert_eq!(
            parsed,
            Some("01969f11-9448-7d79-8d3f-c68f54761234".to_string())
        );
    }

    #[test]
    fn parse_frontmatter_uuid_accepts_legacy_memory_id() {
        let frontmatter = string_frontmatter([(
            LEGACY_MEMORY_ID_FRONTMATTER_KEY.to_string(),
            "01969f11-9448-7d79-8d3f-c68f54761234".to_string(),
        )]);

        let parsed = parse_frontmatter_uuid(&frontmatter).expect("legacy memory_id should parse");

        assert_eq!(
            parsed,
            Some("01969f11-9448-7d79-8d3f-c68f54761234".to_string())
        );
    }

    #[test]
    fn resolve_page_uuid_generates_when_frontmatter_is_absent() {
        let frontmatter = Frontmatter::new();

        let uuid = resolve_page_uuid(&frontmatter, None).expect("uuid should be generated");
        let parsed = uuid::Uuid::parse_str(&uuid).expect("generated uuid should parse");

        assert_eq!(parsed.get_version_num(), 7);
    }

    #[test]
    fn resolve_page_uuid_reuses_stored_uuid_when_frontmatter_is_absent() {
        let frontmatter = Frontmatter::new();

        let uuid = resolve_page_uuid(&frontmatter, Some("01969f11-9448-7d79-8d3f-c68f54761234"))
            .expect("stored uuid should win");

        assert_eq!(uuid, "01969f11-9448-7d79-8d3f-c68f54761234");
    }

    #[test]
    fn resolve_page_uuid_rejects_mismatched_frontmatter_uuid() {
        let frontmatter = string_frontmatter([(
            QUAID_ID_FRONTMATTER_KEY.to_string(),
            "01969f11-9448-7d79-8d3f-c68f54761235".to_string(),
        )]);

        let err = resolve_page_uuid(&frontmatter, Some("01969f11-9448-7d79-8d3f-c68f54761234"))
            .expect_err("mismatched quaid_id should fail");

        assert!(matches!(err, PageUuidError::UuidMismatch { .. }));
    }

    #[test]
    fn canonicalize_frontmatter_uuid_replaces_legacy_memory_id() {
        let mut frontmatter = string_frontmatter([
            (
                LEGACY_MEMORY_ID_FRONTMATTER_KEY.to_string(),
                "01969f11-9448-7d79-8d3f-c68f54760000".to_string(),
            ),
            ("title".to_string(), "Alice".to_string()),
        ]);

        canonicalize_frontmatter_uuid(&mut frontmatter, "01969f11-9448-7d79-8d3f-c68f54761234");

        assert_eq!(
            frontmatter.get(QUAID_ID_FRONTMATTER_KEY),
            Some(&json!("01969f11-9448-7d79-8d3f-c68f54761234"))
        );
        assert!(!frontmatter.contains_key(LEGACY_MEMORY_ID_FRONTMATTER_KEY));
        assert_eq!(frontmatter.get("title"), Some(&json!("Alice")));
    }
}
