// Consumers (migrate.rs, commands/) not yet wired — remove when they are.
#![allow(dead_code)]

use std::collections::HashMap;

use super::types::Page;

/// Parse YAML frontmatter delimited by `---` lines at the start of a file.
///
/// Returns `(frontmatter_map, body)`. If no valid frontmatter block is found,
/// returns an empty map and the full input string as the body.
pub fn parse_frontmatter(raw: &str) -> (HashMap<String, String>, String) {
    let (first_line, mut next_start) = next_line(raw, 0);
    if !is_bare_boundary(first_line) {
        return (HashMap::new(), raw.to_string());
    }

    let frontmatter_start = next_start;
    let mut frontmatter_end = None;
    let mut body_start = None;

    while next_start <= raw.len() {
        let (line, new_next) = next_line(raw, next_start);
        if is_bare_boundary(line) {
            frontmatter_end = Some(next_start);
            body_start = Some(new_next);
            break;
        }

        if new_next == raw.len() {
            break;
        }
        next_start = new_next;
    }

    let Some(frontmatter_end) = frontmatter_end else {
        return (HashMap::new(), raw.to_string());
    };

    let yaml_str = &raw[frontmatter_start..frontmatter_end];
    let map = parse_yaml_to_map(yaml_str);
    let body = &raw[body_start.unwrap_or(raw.len())..];
    (map, body.to_string())
}

/// Split a body (already stripped of frontmatter) at the first bare `---` line.
///
/// Returns `(compiled_truth, timeline)`. If no separator is found, the entire
/// body is compiled_truth and timeline is empty.
pub fn split_content(body: &str) -> (String, String) {
    let mut line_start = 0;
    while line_start <= body.len() {
        let (line, next_start) = next_line(body, line_start);
        if is_bare_boundary(line) {
            let mut truth_end = line_start;
            if truth_end > 0 && body.as_bytes()[truth_end - 1] == b'\n' {
                truth_end -= 1;
                if truth_end > 0 && body.as_bytes()[truth_end - 1] == b'\r' {
                    truth_end -= 1;
                }
            }
            let compiled_truth = &body[..truth_end];
            let timeline = &body[next_start..];
            return (compiled_truth.to_string(), timeline.to_string());
        }

        if next_start == body.len() {
            break;
        }
        line_start = next_start;
    }

    (body.to_string(), String::new())
}

/// Extract a summary from compiled_truth content.
///
/// Returns the first non-heading, non-empty paragraph (capped at 200 characters).
/// Falls back to the first non-empty line if no paragraph qualifies.
pub fn extract_summary(compiled_truth: &str) -> String {
    let mut paragraph_lines = Vec::new();
    let mut in_paragraph = false;
    let mut first_non_empty = None;

    for line in compiled_truth.lines() {
        if first_non_empty.is_none() && !line.trim().is_empty() {
            first_non_empty = Some(line);
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            if in_paragraph {
                break;
            }
            continue;
        }

        if trimmed.starts_with('#') {
            if in_paragraph {
                break;
            }
            continue;
        }

        in_paragraph = true;
        paragraph_lines.push(trimmed);
    }

    let summary = if !paragraph_lines.is_empty() {
        paragraph_lines.join(" ")
    } else if let Some(line) = first_non_empty {
        line.trim().to_string()
    } else {
        String::new()
    };

    truncate_chars(&summary, 200)
}

/// Render a `Page` back to its canonical markdown representation.
///
/// Canonical format:
/// ```text
/// ---
/// key1: value1
/// key2: value2
/// ---
/// <compiled_truth>
/// ---
/// <timeline>
/// ```
///
/// Frontmatter keys are emitted in sorted order so the output is deterministic
/// and byte-exact for canonical input.
pub fn render_page(page: &Page) -> String {
    let mut out = String::new();

    out.push_str("---\n");
    if !page.frontmatter.is_empty() {
        let mut keys: Vec<&String> = page.frontmatter.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(value) = page.frontmatter.get(key) {
                out.push_str(key);
                out.push_str(": ");
                out.push_str(value);
                out.push('\n');
            }
        }
    }
    out.push_str("---\n");

    out.push_str(&page.compiled_truth);
    if !page.compiled_truth.is_empty() && !page.compiled_truth.ends_with('\n') {
        out.push('\n');
    }

    out.push_str("---\n");
    out.push_str(&page.timeline);

    out
}

// ── helpers ───────────────────────────────────────────────────

fn is_bare_boundary(line: &str) -> bool {
    line.trim_end_matches('\r') == "---"
}

fn next_line(s: &str, start: usize) -> (&str, usize) {
    if start >= s.len() {
        return ("", s.len());
    }

    match s[start..].find('\n') {
        Some(pos) => (&s[start..start + pos], start + pos + 1),
        None => (&s[start..], s.len()),
    }
}

/// Parse a YAML string into a flat `HashMap<String, String>`.
///
/// Non-scalar values (sequences, mappings) are silently skipped.
/// Returns an empty map on any parse failure.
fn parse_yaml_to_map(yaml_str: &str) -> HashMap<String, String> {
    if yaml_str.trim().is_empty() {
        return HashMap::new();
    }

    let Ok(map) = serde_yaml::from_str::<HashMap<String, serde_yaml::Value>>(yaml_str) else {
        return HashMap::new();
    };

    map.into_iter()
        .filter_map(|(k, v)| yaml_value_to_string(&v).map(|val| (k, val)))
        .collect()
}

/// Convert a scalar YAML value to its string representation.
fn yaml_value_to_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        serde_yaml::Value::Null => Some(String::new()),
        _ => None,
    }
}

/// Truncate a string to at most `max` characters (respects char boundaries).
fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

// ── tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_frontmatter ─────────────────────────────────────

    mod parse_frontmatter {
        use super::*;

        #[test]
        fn parses_string_scalar_frontmatter_when_file_starts_with_bare_boundary() {
            let input = "---\ntitle: Alice\ntype: person\n---\n# Alice\n";
            let (map, body) = parse_frontmatter(input);

            assert_eq!(map.get("title").unwrap(), "Alice");
            assert_eq!(map.get("type").unwrap(), "person");
            assert_eq!(body, "# Alice\n");
        }

        #[test]
        fn returns_empty_map_and_full_body_when_opening_boundary_is_missing() {
            let input = "# Just a heading\nSome content";
            let (map, body) = parse_frontmatter(input);

            assert!(map.is_empty());
            assert_eq!(body, input);
        }

        #[test]
        fn treats_leading_newline_before_boundary_as_no_frontmatter() {
            let input = "\n---\ntitle: Alice\n---\nBody";
            let (map, body) = parse_frontmatter(input);

            assert!(map.is_empty());
            assert_eq!(body, input);
        }

        #[test]
        fn accepts_empty_frontmatter_block() {
            let input = "---\n---\n# Title";
            let (map, body) = parse_frontmatter(input);

            assert!(map.is_empty());
            assert_eq!(body, "# Title");
        }

        #[test]
        fn stops_at_first_closing_bare_boundary() {
            let input =
                "---\ntitle: Boundary Case\n---\nIntro paragraph\n---\n2024-01-01: Event one";
            let (map, body) = parse_frontmatter(input);

            assert_eq!(map.get("title").unwrap(), "Boundary Case");
            assert_eq!(body, "Intro paragraph\n---\n2024-01-01: Event one");
        }

        #[test]
        fn does_not_treat_non_bare_rules_as_frontmatter_boundaries() {
            let inputs = [
                "--- \ntitle: Alice\n---\nBody",
                " ----\ntitle: Alice\n---\nBody",
                " ---\ntitle: Alice\n---\nBody",
            ];

            for input in inputs {
                let (map, body) = parse_frontmatter(input);
                assert!(map.is_empty());
                assert_eq!(body, input);
            }
        }
    }

    // ── split_content ─────────────────────────────────────────

    mod split_content {
        use super::*;

        #[test]
        fn splits_on_first_bare_boundary_line() {
            let (truth, timeline) = split_content("above\n---\nbelow");

            assert_eq!(truth, "above");
            assert_eq!(timeline, "below");
        }

        #[test]
        fn returns_full_body_and_empty_timeline_when_boundary_missing() {
            let (truth, timeline) = split_content("no boundary here");

            assert_eq!(truth, "no boundary here");
            assert!(timeline.is_empty());
        }

        #[test]
        fn only_splits_once_when_timeline_contains_additional_boundaries() {
            let (truth, timeline) = split_content("above\n---\nentry one\n---\nentry two");

            assert_eq!(truth, "above");
            assert_eq!(timeline, "entry one\n---\nentry two");
        }

        #[test]
        fn does_not_split_on_horizontal_rule_variants() {
            let inputs = [
                "above\n ---\nbelow",
                "above\n--- \nbelow",
                "above\n----\nbelow",
            ];

            for input in inputs {
                let (truth, timeline) = split_content(input);
                assert_eq!(truth, input);
                assert!(timeline.is_empty());
            }
        }

        #[test]
        fn preserves_newlines_around_sections_without_trimming_content() {
            let (truth, timeline) = split_content("above\n\n---\n\nbelow");

            assert_eq!(truth, "above\n");
            assert_eq!(timeline, "\nbelow");
        }
    }

    // ── extract_summary ───────────────────────────────────────

    mod extract_summary {
        use super::*;

        #[test]
        fn returns_first_non_heading_non_empty_paragraph() {
            let truth = "# Title\n\nAlice is an operator.\nShe ships things.\n\nMore text.";
            let summary = extract_summary(truth);

            assert_eq!(summary, "Alice is an operator. She ships things.");
        }

        #[test]
        fn falls_back_to_first_line_when_no_paragraph_exists() {
            let truth = "# Only heading\n## Another heading";
            let summary = extract_summary(truth);

            assert_eq!(summary, "# Only heading");
        }

        #[test]
        fn caps_summary_at_200_chars() {
            let long_line = "x".repeat(240);
            let summary = extract_summary(&long_line);

            assert_eq!(summary.len(), 200);
        }

        #[test]
        fn ignores_leading_blank_lines() {
            let truth = "\n\n# Title\n\nFirst paragraph.";
            let summary = extract_summary(truth);

            assert_eq!(summary, "First paragraph.");
        }
    }

    // ── render_page + round-trip ──────────────────────────────

    mod render_page {
        use super::*;

        fn make_page(frontmatter: Vec<(&str, &str)>, compiled_truth: &str, timeline: &str) -> Page {
            Page {
                slug: String::new(),
                page_type: String::new(),
                title: String::new(),
                summary: String::new(),
                compiled_truth: compiled_truth.to_string(),
                timeline: timeline.to_string(),
                frontmatter: frontmatter
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                wing: String::new(),
                room: String::new(),
                version: 1,
                created_at: String::new(),
                updated_at: String::new(),
                truth_updated_at: String::new(),
                timeline_updated_at: String::new(),
            }
        }

        #[test]
        fn renders_frontmatter_compiled_truth_and_timeline_in_canonical_order() {
            let page = make_page(
                vec![("title", "Alice"), ("source", "manual"), ("type", "person")],
                "# Alice\n\nAlice is an operator.",
                "2024-01-01: Joined Acme.",
            );

            let rendered = render_page(&page);

            assert_eq!(
                rendered,
                "---\nsource: manual\ntitle: Alice\ntype: person\n---\n# Alice\n\nAlice is an operator.\n---\n2024-01-01: Joined Acme."
            );
        }

        #[test]
        fn render_parse_render_is_idempotent_for_canonical_page() {
            let canonical = "---\nsource: manual\ntitle: Alice\ntype: person\n---\n# Alice\n\nAlice is an operator.\n---\n2024-01-01: Joined Acme.\n";

            let (map, body) = parse_frontmatter(canonical);
            let (truth, timeline) = split_content(&body);

            let mut page = make_page(vec![], &truth, &timeline);
            page.frontmatter = map;

            let rendered = render_page(&page);
            assert_eq!(rendered, canonical);
        }

        #[test]
        fn renders_empty_timeline_without_losing_boundary_contract() {
            let page = make_page(vec![("title", "Solo")], "Just truth.", "");
            let rendered = render_page(&page);

            assert_eq!(rendered, "---\ntitle: Solo\n---\nJust truth.\n---\n");
        }

        #[test]
        fn renders_empty_frontmatter_deterministically() {
            let page = make_page(vec![], "Plain content.", "Timeline entry.");
            let rendered = render_page(&page);

            assert_eq!(rendered, "---\n---\nPlain content.\n---\nTimeline entry.");
        }
    }
}
