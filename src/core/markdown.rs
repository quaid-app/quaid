// Consumers (migrate.rs, commands/) not yet wired — remove when they are.
#![allow(dead_code)]

use std::collections::HashMap;

use super::types::Page;

/// Parse YAML frontmatter delimited by `---` lines at the start of a file.
///
/// Returns `(frontmatter_map, body)`. If no valid frontmatter block is found,
/// returns an empty map and the full input string as the body.
pub fn parse_frontmatter(raw: &str) -> (HashMap<String, String>, String) {
    if !raw.starts_with("---\n") && raw != "---" {
        return (HashMap::new(), raw.to_string());
    }

    let after_open = &raw[4..]; // skip "---\n"

    // Find the closing "---" on its own line.
    let body_start;
    let yaml_str;

    if let Some(pos) = after_open.find("\n---\n") {
        yaml_str = &after_open[..pos];
        body_start = pos + 5; // skip "\n---\n"
    } else if let Some(stripped) = after_open.strip_suffix("\n---") {
        yaml_str = stripped;
        body_start = after_open.len();
    } else if after_open == "---" {
        // File is exactly "---\n---"
        yaml_str = "";
        body_start = 3;
    } else {
        // Opening "---" but no closing "---" — treat as no frontmatter.
        return (HashMap::new(), raw.to_string());
    }

    let map = parse_yaml_to_map(yaml_str);
    let body = &after_open[body_start..];
    (map, body.to_string())
}

/// Split a body (already stripped of frontmatter) at the first bare `---` line.
///
/// Returns `(compiled_truth, timeline)`. If no separator is found, the entire
/// body is compiled_truth and timeline is empty.
pub fn split_content(body: &str) -> (String, String) {
    // Case: separator at the very start
    if body == "---" {
        return (String::new(), String::new());
    }
    if let Some(rest) = body.strip_prefix("---\n") {
        return (String::new(), rest.to_string());
    }

    // Case: separator in the middle
    if let Some(pos) = body.find("\n---\n") {
        let truth = &body[..pos];
        let timeline = &body[pos + 5..];
        return (truth.to_string(), timeline.to_string());
    }

    // Case: separator at the end (body ends with "\n---")
    if let Some(truth) = body.strip_suffix("\n---") {
        return (truth.to_string(), String::new());
    }

    // No separator found
    (body.to_string(), String::new())
}

/// Extract a summary from compiled_truth content.
///
/// Returns the first non-heading, non-empty paragraph (capped at 200 characters).
/// Falls back to the first non-empty line if no paragraph qualifies.
pub fn extract_summary(compiled_truth: &str) -> String {
    let mut paragraph_lines: Vec<&str> = Vec::new();
    let mut in_paragraph = false;

    for line in compiled_truth.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            if in_paragraph {
                break; // end of the first qualifying paragraph
            }
            continue;
        }

        if trimmed.starts_with('#') {
            if in_paragraph {
                break; // heading ends the current paragraph
            }
            continue; // skip heading lines when looking for a paragraph
        }

        in_paragraph = true;
        paragraph_lines.push(trimmed);
    }

    if !paragraph_lines.is_empty() {
        let paragraph = paragraph_lines.join(" ");
        return truncate_chars(&paragraph, 200);
    }

    // Fallback: first non-empty line, even if it is a heading
    compiled_truth
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|l| truncate_chars(l, 200))
        .unwrap_or_default()
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

    // Frontmatter block
    if !page.frontmatter.is_empty() {
        out.push_str("---\n");
        let mut keys: Vec<&String> = page.frontmatter.keys().collect();
        keys.sort();
        for key in keys {
            let value = &page.frontmatter[key];
            out.push_str(key);
            out.push_str(": ");
            out.push_str(value);
            out.push('\n');
        }
        out.push_str("---\n");
    }

    // Compiled truth
    out.push_str(&page.compiled_truth);

    // Timeline (only emit separator when there is timeline content)
    if !page.timeline.is_empty() {
        out.push_str("\n---\n");
        out.push_str(&page.timeline);
    }

    out
}

// ── helpers ───────────────────────────────────────────────────

/// Parse a YAML string into a flat `HashMap<String, String>`.
///
/// Non-scalar values (sequences, mappings) are silently skipped.
/// Returns an empty map on any parse failure.
fn parse_yaml_to_map(yaml_str: &str) -> HashMap<String, String> {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(yaml_str) else {
        return HashMap::new();
    };

    let Some(mapping) = value.as_mapping() else {
        return HashMap::new();
    };

    mapping
        .iter()
        .filter_map(|(k, v)| {
            let key = k.as_str()?.to_string();
            let val = yaml_value_to_string(v)?;
            Some((key, val))
        })
        .collect()
}

/// Convert a scalar YAML value to its string representation.
fn yaml_value_to_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        serde_yaml::Value::Null => Some(String::new()),
        _ => None, // sequences and mappings are not representable in a flat map
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
        fn extracts_fields_and_body_from_valid_frontmatter() {
            let input = "---\ntitle: Alice\ntype: person\n---\n# Alice\nShe is an engineer.\n";
            let (map, body) = parse_frontmatter(input);

            assert_eq!(map.get("title").unwrap(), "Alice");
            assert_eq!(map.get("type").unwrap(), "person");
            assert!(body.starts_with("# Alice\n"));
        }

        #[test]
        fn returns_empty_map_and_full_body_when_no_frontmatter() {
            let input = "# Just a heading\nSome content";
            let (map, body) = parse_frontmatter(input);

            assert!(map.is_empty());
            assert_eq!(body, input);
        }

        #[test]
        fn handles_frontmatter_with_no_body() {
            let input = "---\ntitle: Lonely\n---\n";
            let (map, body) = parse_frontmatter(input);

            assert_eq!(map.get("title").unwrap(), "Lonely");
            assert!(body.is_empty());
        }

        #[test]
        fn gracefully_handles_malformed_yaml() {
            let input = "---\n: broken yaml [[[}}\n---\nBody here\n";
            let (map, body) = parse_frontmatter(input);

            // Malformed YAML → empty map, but body is still extracted
            assert!(map.is_empty());
            assert_eq!(body, "Body here\n");
        }

        #[test]
        fn handles_numeric_and_boolean_values() {
            let input = "---\nversion: 4\nactive: true\n---\ncontent\n";
            let (map, _body) = parse_frontmatter(input);

            assert_eq!(map.get("version").unwrap(), "4");
            assert_eq!(map.get("active").unwrap(), "true");
        }

        #[test]
        fn unclosed_frontmatter_treated_as_no_frontmatter() {
            let input = "---\ntitle: Broken\nNo closing delimiter\n";
            let (map, body) = parse_frontmatter(input);

            assert!(map.is_empty());
            assert_eq!(body, input);
        }
    }

    // ── split_content ─────────────────────────────────────────

    mod split_content {
        use super::*;

        #[test]
        fn splits_at_first_bare_separator() {
            let (truth, timeline) = split_content("above\n---\nbelow");

            assert_eq!(truth, "above");
            assert_eq!(timeline, "below");
        }

        #[test]
        fn no_boundary_returns_full_body_as_truth() {
            let (truth, timeline) = split_content("no boundary here");

            assert_eq!(truth, "no boundary here");
            assert_eq!(timeline, "");
        }

        #[test]
        fn separator_at_start_yields_empty_truth() {
            let (truth, timeline) = split_content("---\ntimeline only");

            assert_eq!(truth, "");
            assert_eq!(timeline, "timeline only");
        }

        #[test]
        fn separator_at_end_yields_empty_timeline() {
            let (truth, timeline) = split_content("truth only\n---");

            assert_eq!(truth, "truth only");
            assert_eq!(timeline, "");
        }

        #[test]
        fn only_first_separator_is_used() {
            let (truth, timeline) = split_content("above\n---\nmiddle\n---\nbelow");

            assert_eq!(truth, "above");
            assert_eq!(timeline, "middle\n---\nbelow");
        }

        #[test]
        fn preserves_trailing_newline_in_timeline() {
            let (truth, timeline) = split_content("truth\n---\ntimeline\n");

            assert_eq!(truth, "truth");
            assert_eq!(timeline, "timeline\n");
        }
    }

    // ── extract_summary ───────────────────────────────────────

    mod extract_summary {
        use super::*;

        #[test]
        fn returns_first_paragraph_text() {
            let truth = "# Heading\n\nAlice is an engineer.\nShe builds things.\n\nMore text.";
            let summary = extract_summary(truth);

            assert_eq!(summary, "Alice is an engineer. She builds things.");
        }

        #[test]
        fn truncates_to_200_chars() {
            let long_line = "x".repeat(300);
            let truth = format!("{long_line}\n");
            let summary = extract_summary(&truth);

            assert_eq!(summary.len(), 200);
        }

        #[test]
        fn falls_back_to_first_line_if_all_headings() {
            let truth = "# Only heading\n## Another heading";
            let summary = extract_summary(truth);

            assert_eq!(summary, "# Only heading");
        }

        #[test]
        fn returns_empty_for_empty_input() {
            assert_eq!(extract_summary(""), "");
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
        fn renders_full_page_with_frontmatter_truth_and_timeline() {
            let page = make_page(
                vec![("title", "Alice"), ("type", "person")],
                "Alice is an engineer.",
                "Met Alice on Jan 15.",
            );
            let rendered = render_page(&page);

            assert_eq!(
                rendered,
                "---\ntitle: Alice\ntype: person\n---\nAlice is an engineer.\n---\nMet Alice on Jan 15."
            );
        }

        #[test]
        fn omits_separator_when_timeline_is_empty() {
            let page = make_page(vec![("title", "Solo")], "Just truth.", "");
            let rendered = render_page(&page);

            assert_eq!(rendered, "---\ntitle: Solo\n---\nJust truth.");
        }

        #[test]
        fn omits_frontmatter_block_when_map_is_empty() {
            let page = make_page(vec![], "Plain content.", "");
            let rendered = render_page(&page);

            assert_eq!(rendered, "Plain content.");
        }

        #[test]
        fn render_then_reparse_then_rerender_is_idempotent() {
            let page = make_page(
                vec![("title", "Alice"), ("type", "person"), ("wing", "people")],
                "Alice is an engineer.",
                "Met Alice on Jan 15.\n",
            );

            // First render
            let rendered1 = render_page(&page);

            // Parse back
            let (map, body) = parse_frontmatter(&rendered1);
            let (truth, timeline) = split_content(&body);

            let page2 = make_page(vec![], &truth, &timeline);
            // Reconstruct with parsed frontmatter
            let mut page2_with_fm = page2;
            page2_with_fm.frontmatter = map;

            // Second render
            let rendered2 = render_page(&page2_with_fm);

            assert_eq!(
                rendered1, rendered2,
                "render → parse → render must be idempotent"
            );
        }

        #[test]
        fn byte_exact_round_trip_for_canonical_input() {
            // Canonical input: sorted keys, unquoted values, \n line endings
            let canonical = "---\ntitle: Alice\ntype: person\nwing: people\n---\nAlice is an engineer.\n---\n## 2024-01-15\nMet Alice.\n";

            let (map, body) = parse_frontmatter(canonical);
            let (truth, timeline) = split_content(&body);

            let mut page = make_page(vec![], &truth, &timeline);
            page.frontmatter = map;

            let rendered = render_page(&page);

            assert_eq!(
                rendered, canonical,
                "canonical input must round-trip byte-exact"
            );
        }
    }
}
