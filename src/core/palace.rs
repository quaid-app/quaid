/// Derive the wing (top-level category) from a page slug.
///
/// The wing is the first path segment before the first `/`.
/// Flat slugs with no `/` are assigned to the `"general"` wing.
pub fn derive_wing(slug: &str) -> String {
    match slug.split_once('/') {
        Some((first, _)) if !first.is_empty() => first.to_string(),
        _ => "general".to_string(),
    }
}

/// Derive the room (sub-category) from page content.
///
/// Extracts the first `## <heading>` from `content`, lowercases it,
/// replaces spaces with hyphens, and strips non-`[a-z0-9-]` characters.
/// Returns `""` if no `##` heading is found.
pub fn derive_room(content: &str) -> String {
    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            let heading = heading.trim();
            if heading.is_empty() {
                continue;
            }
            let kebab: String = heading
                .to_lowercase()
                .chars()
                .map(|c| if c == ' ' { '-' } else { c })
                .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
                .collect();
            return kebab;
        }
    }
    String::new()
}

/// Classify the intent of a search query by detecting slug-like tokens.
///
/// A token is "slug-like" if it contains at least one `/` separating
/// two non-empty segments (e.g. `people/alice`). Returns the wing
/// (first segment) of the first slug-like token found, or `None`.
pub fn classify_intent(query: &str) -> Option<String> {
    query
        .split_whitespace()
        .find_map(|token| match token.split_once('/') {
            Some((first, rest)) if !first.is_empty() && !rest.is_empty() => Some(first.to_string()),
            _ => None,
        })
}

// ── tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    // ── derive_wing ───────────────────────────────────────────

    mod derive_wing {
        use super::*;

        #[test]
        fn returns_first_segment_for_two_part_slug() {
            assert_eq!(derive_wing("people/alice"), "people");
        }

        #[test]
        fn returns_first_segment_for_deep_slug() {
            assert_eq!(derive_wing("companies/acme/finance"), "companies");
        }

        #[test]
        fn returns_general_for_flat_slug_without_slash() {
            assert_eq!(derive_wing("readme"), "general");
        }

        #[test]
        fn returns_general_for_empty_string() {
            assert_eq!(derive_wing(""), "general");
        }

        #[test]
        fn returns_general_for_leading_slash() {
            assert_eq!(derive_wing("/alice"), "general");
        }

        #[test]
        fn returns_first_segment_with_trailing_slash() {
            assert_eq!(derive_wing("people/"), "people");
        }
    }

    // ── derive_room ───────────────────────────────────────────

    mod derive_room {
        use super::*;

        #[test]
        fn h2_heading_produces_kebab_case_room() {
            assert_eq!(
                derive_room("# Title\n\n## Current Role\n\nContent here"),
                "current-role"
            );
        }

        #[test]
        fn no_heading_returns_empty_string() {
            assert_eq!(derive_room("Just some plain content"), "");
        }

        #[test]
        fn heading_with_special_characters_is_cleaned() {
            assert_eq!(derive_room("## Hello, World! (2024)"), "hello-world-2024");
        }

        #[test]
        fn second_h2_heading_is_ignored() {
            assert_eq!(
                derive_room("## First Heading\n\nParagraph\n\n## Second Heading"),
                "first-heading"
            );
        }

        #[test]
        fn returns_empty_string_for_empty_input() {
            assert_eq!(derive_room(""), "");
        }

        #[test]
        fn h3_heading_is_not_a_room() {
            assert_eq!(derive_room("### Not a room heading"), "");
        }
    }

    // ── classify_intent ───────────────────────────────────────

    mod classify_intent {
        use super::*;

        #[test]
        fn returns_wing_from_slug_like_token_in_query() {
            assert_eq!(
                classify_intent("who is people/alice"),
                Some("people".to_string())
            );
        }

        #[test]
        fn returns_none_when_no_slug_like_token_exists() {
            assert_eq!(classify_intent("general search query"), None);
        }

        #[test]
        fn returns_first_slug_wing_when_query_has_multiple_slugs() {
            assert_eq!(
                classify_intent("link people/alice companies/acme"),
                Some("people".to_string())
            );
        }

        #[test]
        fn returns_none_for_empty_query() {
            assert_eq!(classify_intent(""), None);
        }

        #[test]
        fn rejects_bare_slash_without_segments() {
            assert_eq!(classify_intent("just a / here"), None);
        }

        #[test]
        fn rejects_leading_slash_token() {
            assert_eq!(classify_intent("see /alice"), None);
        }

        #[test]
        fn rejects_trailing_slash_token() {
            assert_eq!(classify_intent("see people/"), None);
        }
    }
}
