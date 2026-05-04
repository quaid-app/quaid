use std::collections::HashMap;

use thiserror::Error;
use uuid::Uuid;

pub const QUAID_ID_FRONTMATTER_KEY: &str = "quaid_id";
pub const LEGACY_MEMORY_ID_FRONTMATTER_KEY: &str = "memory_id";

#[derive(Debug, Error)]
pub enum PageUuidError {
    #[error("frontmatter quaid_id/memory_id cannot be empty")]
    EmptyFrontmatterUuid,

    #[error("invalid frontmatter quaid_id/memory_id: {value}")]
    InvalidFrontmatterUuid { value: String },

    #[error(
        "frontmatter quaid_id/memory_id {frontmatter_uuid} does not match stored page uuid {stored_uuid}"
    )]
    UuidMismatch {
        stored_uuid: String,
        frontmatter_uuid: String,
    },
}

pub fn generate_uuid_v7() -> String {
    Uuid::now_v7().to_string()
}

pub fn parse_frontmatter_uuid(
    frontmatter: &HashMap<String, String>,
) -> Result<Option<String>, PageUuidError> {
    let Some(raw_uuid) = frontmatter
        .get(QUAID_ID_FRONTMATTER_KEY)
        .or_else(|| frontmatter.get(LEGACY_MEMORY_ID_FRONTMATTER_KEY))
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
            value: raw_uuid.clone(),
        })
}

pub fn resolve_page_uuid(
    frontmatter: &HashMap<String, String>,
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

pub fn canonicalize_frontmatter_uuid(frontmatter: &mut HashMap<String, String>, stored_uuid: &str) {
    frontmatter.remove(LEGACY_MEMORY_ID_FRONTMATTER_KEY);
    frontmatter.remove(QUAID_ID_FRONTMATTER_KEY);

    if !stored_uuid.trim().is_empty() {
        frontmatter.insert(
            QUAID_ID_FRONTMATTER_KEY.to_string(),
            stored_uuid.to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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
        let frontmatter = HashMap::from([(
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
        let frontmatter = HashMap::from([(
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
        let frontmatter = HashMap::new();

        let uuid = resolve_page_uuid(&frontmatter, None).expect("uuid should be generated");
        let parsed = uuid::Uuid::parse_str(&uuid).expect("generated uuid should parse");

        assert_eq!(parsed.get_version_num(), 7);
    }

    #[test]
    fn resolve_page_uuid_reuses_stored_uuid_when_frontmatter_is_absent() {
        let frontmatter = HashMap::new();

        let uuid = resolve_page_uuid(&frontmatter, Some("01969f11-9448-7d79-8d3f-c68f54761234"))
            .expect("stored uuid should win");

        assert_eq!(uuid, "01969f11-9448-7d79-8d3f-c68f54761234");
    }

    #[test]
    fn resolve_page_uuid_rejects_mismatched_frontmatter_uuid() {
        let frontmatter = HashMap::from([(
            QUAID_ID_FRONTMATTER_KEY.to_string(),
            "01969f11-9448-7d79-8d3f-c68f54761235".to_string(),
        )]);

        let err = resolve_page_uuid(&frontmatter, Some("01969f11-9448-7d79-8d3f-c68f54761234"))
            .expect_err("mismatched quaid_id should fail");

        assert!(matches!(err, PageUuidError::UuidMismatch { .. }));
    }

    #[test]
    fn canonicalize_frontmatter_uuid_replaces_legacy_memory_id() {
        let mut frontmatter = HashMap::from([
            (
                LEGACY_MEMORY_ID_FRONTMATTER_KEY.to_string(),
                "01969f11-9448-7d79-8d3f-c68f54760000".to_string(),
            ),
            ("title".to_string(), "Alice".to_string()),
        ]);

        canonicalize_frontmatter_uuid(&mut frontmatter, "01969f11-9448-7d79-8d3f-c68f54761234");

        assert_eq!(
            frontmatter
                .get(QUAID_ID_FRONTMATTER_KEY)
                .map(String::as_str),
            Some("01969f11-9448-7d79-8d3f-c68f54761234")
        );
        assert!(!frontmatter.contains_key(LEGACY_MEMORY_ID_FRONTMATTER_KEY));
        assert_eq!(frontmatter.get("title").map(String::as_str), Some("Alice"));
    }
}
