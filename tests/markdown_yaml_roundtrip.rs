//! Frontmatter round-trip fidelity: `render_page` → `parse_frontmatter`
//! must be an identity over adversarial string scalars. Under-quoted YAML
//! used to flip types (`"true"` → Bool, `"2024"` → Number) or invalidate
//! the document (`"- item"`), after which the parser silently dropped all
//! frontmatter on the next render.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

use quaid::core::markdown::{parse_frontmatter, render_page, split_content};
use quaid::core::types::{Frontmatter, Page};
use serde_json::{json, Value as JsonValue};

const PAGE_UUID: &str = "01969f11-9448-7d79-8d3f-c68f54761234";

fn make_page(frontmatter: Frontmatter, compiled_truth: &str, timeline: &str) -> Page {
    Page {
        slug: "notes/roundtrip".to_string(),
        uuid: PAGE_UUID.to_string(),
        page_type: "concept".to_string(),
        superseded_by: None,
        title: "Roundtrip".to_string(),
        summary: String::new(),
        compiled_truth: compiled_truth.to_string(),
        timeline: timeline.to_string(),
        frontmatter,
        wing: "notes".to_string(),
        room: String::new(),
        version: 1,
        created_at: String::new(),
        updated_at: String::new(),
        truth_updated_at: String::new(),
        timeline_updated_at: String::new(),
    }
}

/// Property-style identity check: every scalar must come back from
/// `render_page` → `parse_frontmatter` exactly as it went in — same value,
/// same JSON type.
#[test]
fn render_parse_identity_over_adversarial_string_scalars() {
    let adversarial: &[&str] = &[
        "true",
        "false",
        "yes",
        "no",
        "on",
        "off",
        "null",
        "~",
        "007",
        "2024",
        "1e5",
        "0x1F",
        "3.14",
        "-42",
        "2024-01-01",
        "- item",
        "? key",
        ": value",
        "a: b",
        "#comment",
        "[bracket]",
        "{brace}",
        "'single'",
        "\"double\"",
        "back\\slash",
        "",
        " leading space",
        "trailing space ",
        "line1\nline2",
        "ends with newline\n",
        "blank\n\nline",
        "multi\nline\nvalue\n",
    ];

    for scalar in adversarial {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert("probe".to_string(), json!(scalar));
        frontmatter.insert("title".to_string(), json!("Roundtrip"));
        let page = make_page(frontmatter, "Body text.", "2024-01-01: entry.");

        let rendered = render_page(&page);
        let (parsed, body) = parse_frontmatter(&rendered);

        assert_eq!(
            parsed.get("probe"),
            Some(&json!(scalar)),
            "scalar {scalar:?} did not survive render→parse; rendered:\n{rendered}"
        );
        assert_eq!(
            parsed.get("quaid_id"),
            Some(&json!(PAGE_UUID)),
            "quaid_id lost while round-tripping {scalar:?}"
        );

        let (truth, timeline) = split_content(&body);
        assert_eq!(truth, "Body text.", "body corrupted by scalar {scalar:?}");
        assert_eq!(
            timeline, "2024-01-01: entry.",
            "timeline corrupted by scalar {scalar:?}"
        );
    }
}

/// A second render of the parsed page must be byte-identical to the first
/// render (idempotence), so vault files stop churning.
#[test]
fn render_is_idempotent_for_adversarial_scalars() {
    for scalar in ["true", "007", "2024", "- item", "line1\nline2"] {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert("probe".to_string(), json!(scalar));
        let page = make_page(frontmatter, "Body.", "");

        let first = render_page(&page);
        let (parsed, body) = parse_frontmatter(&first);
        let (truth, timeline) = split_content(&body);

        let mut reparsed_page = make_page(parsed, &truth, &timeline);
        reparsed_page.uuid = PAGE_UUID.to_string();
        let second = render_page(&reparsed_page);

        assert_eq!(
            first, second,
            "render→parse→render not idempotent for scalar {scalar:?}"
        );
    }
}

/// Non-string scalar values must keep their JSON types through a round trip.
#[test]
fn render_parse_identity_preserves_non_string_types() {
    let mut frontmatter = Frontmatter::new();
    frontmatter.insert("flag".to_string(), json!(true));
    frontmatter.insert("count".to_string(), json!(7));
    frontmatter.insert("ratio".to_string(), json!(0.5));
    frontmatter.insert("nothing".to_string(), JsonValue::Null);
    frontmatter.insert("items".to_string(), json!(["a", "true", "007"]));
    let page = make_page(frontmatter, "Body.", "");

    let rendered = render_page(&page);
    let (parsed, _) = parse_frontmatter(&rendered);

    assert_eq!(parsed.get("flag"), Some(&json!(true)));
    assert_eq!(parsed.get("count"), Some(&json!(7)));
    assert_eq!(parsed.get("ratio"), Some(&json!(0.5)));
    assert_eq!(parsed.get("nothing"), Some(&JsonValue::Null));
    assert_eq!(parsed.get("items"), Some(&json!(["a", "true", "007"])));
}

/// Unparseable YAML frontmatter must be preserved verbatim as body content
/// instead of being silently dropped on the next render.
#[test]
fn unparseable_frontmatter_is_preserved_as_body_not_dropped() {
    let inputs = [
        "---\ntitle: [unclosed\n---\nBody text.\n",
        "---\na: 1\na: 2\n---\nBody text.\n",
        "---\n\tindented: tab\n---\nBody text.\n",
    ];

    for input in inputs {
        let (map, body) = parse_frontmatter(input);

        assert!(
            map.is_empty(),
            "unparseable YAML must not yield frontmatter: {input:?}"
        );
        assert_eq!(
            body, input,
            "unparseable frontmatter block must survive in the body verbatim"
        );
    }
}
