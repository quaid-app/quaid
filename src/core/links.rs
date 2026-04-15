// Consumers (commands/, mcp/) not yet wired — remove when they are.
#![allow(dead_code)]

use regex::Regex;

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
