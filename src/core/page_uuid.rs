use std::collections::HashMap;

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PageUuidError {
    #[error("frontmatter gbrain_id cannot be empty")]
    EmptyFrontmatterUuid,

    #[error("invalid frontmatter gbrain_id: {value}")]
    InvalidFrontmatterUuid { value: String },

    #[error(
        "frontmatter gbrain_id {frontmatter_uuid} does not match stored page uuid {stored_uuid}"
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
    let Some(raw_uuid) = frontmatter.get("gbrain_id") else {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{generate_uuid_v7, parse_frontmatter_uuid, resolve_page_uuid, PageUuidError};

    #[test]
    fn generate_uuid_v7_returns_a_uuid_string() {
        let uuid = generate_uuid_v7();
        let parsed = uuid::Uuid::parse_str(&uuid).expect("generated uuid should parse");

        assert_eq!(parsed.get_version_num(), 7);
    }

    #[test]
    fn parse_frontmatter_uuid_preserves_present_gbrain_id() {
        let frontmatter = HashMap::from([(
            "gbrain_id".to_string(),
            "01969f11-9448-7d79-8d3f-c68f54761234".to_string(),
        )]);

        let parsed = parse_frontmatter_uuid(&frontmatter).expect("gbrain_id should parse");

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
            "gbrain_id".to_string(),
            "01969f11-9448-7d79-8d3f-c68f54761235".to_string(),
        )]);

        let err = resolve_page_uuid(&frontmatter, Some("01969f11-9448-7d79-8d3f-c68f54761234"))
            .expect_err("mismatched gbrain_id should fail");

        assert!(matches!(err, PageUuidError::UuidMismatch { .. }));
    }
}
