#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure"
)]

//! Structured-frontmatter parsing/render coverage (tasks.md §2.6).
//!
//! Locks in:
//!   - `parse_frontmatter` does not skip arrays/objects/nested values,
//!   - the `frontmatter_get_*` scalar helpers preserve string-only behaviour,
//!   - `parse_frontmatter` → `render_page` → `parse_frontmatter` is a stable
//!     round-trip for arrays, objects, and mixed-scalar frontmatter so the
//!     forthcoming derived-edge sync paths cannot silently drop keys.

use quaid::core::markdown::{parse_frontmatter, render_page, split_content};
use quaid::core::types::{frontmatter_get, frontmatter_get_str, frontmatter_get_string, Page};
use serde_json::{json, Value as JsonValue};

fn make_page(uuid: &str, frontmatter: quaid::core::types::Frontmatter) -> Page {
    Page {
        slug: String::new(),
        uuid: uuid.to_string(),
        page_type: String::new(),
        superseded_by: None,
        title: String::new(),
        summary: String::new(),
        compiled_truth: String::from("Body"),
        timeline: String::new(),
        frontmatter,
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
fn parse_frontmatter_preserves_array_of_strings() {
    let input = "---\ntags:\n  - alpha\n  - beta\n  - gamma\n---\nBody";
    let (map, body) = parse_frontmatter(input);

    assert_eq!(
        map.get("tags"),
        Some(&json!(["alpha", "beta", "gamma"])),
        "string array must round-trip as a JSON array, not be flattened"
    );
    assert_eq!(body, "Body");
}

#[test]
fn parse_frontmatter_preserves_nested_object() {
    let input = "---\nlinks:\n  works_at: companies/acme\n  founded:\n    target: companies/acme\n    valid_from: 2024-01\n---\nBody";
    let (map, _) = parse_frontmatter(input);

    let links = map
        .get("links")
        .expect("links key must survive parse")
        .as_object()
        .expect("links must be a JSON object, not a stringified scalar");
    assert_eq!(
        links.get("works_at"),
        Some(&JsonValue::String("companies/acme".to_string()))
    );
    let founded = links
        .get("founded")
        .and_then(JsonValue::as_object)
        .expect("nested object must survive parse");
    assert_eq!(
        founded.get("target"),
        Some(&JsonValue::String("companies/acme".to_string()))
    );
    assert_eq!(
        founded.get("valid_from"),
        Some(&JsonValue::String("2024-01".to_string()))
    );
}

#[test]
fn parse_frontmatter_preserves_mixed_array_of_objects() {
    let input = "---\nlinks:\n  - target: people/alice\n    relationship: works_at\n  - target: companies/acme\n    relationship: founded\n    valid_from: 2024-01\n---\nBody";
    let (map, _) = parse_frontmatter(input);

    let arr = map
        .get("links")
        .and_then(JsonValue::as_array)
        .expect("array-of-objects must survive parse");
    assert_eq!(arr.len(), 2);
    assert_eq!(
        arr[0].get("target"),
        Some(&JsonValue::String("people/alice".to_string()))
    );
    assert_eq!(
        arr[1].get("valid_from"),
        Some(&JsonValue::String("2024-01".to_string()))
    );
}

#[test]
fn scalar_helpers_preserve_string_only_behavior() {
    let input = "---\ntitle: Alice\ntype: person\ncount: 7\n---\nBody";
    let (map, _) = parse_frontmatter(input);

    // String scalars resolve through both helpers.
    assert_eq!(frontmatter_get_str(&map, "title"), Some("Alice"));
    assert_eq!(
        frontmatter_get_string(&map, "type"),
        Some("person".to_string())
    );

    // Non-string values must not be coerced — `*_str`/`*_string` are
    // intentionally string-only helpers, while `frontmatter_get` returns the
    // raw structured value for downstream consumers.
    assert_eq!(frontmatter_get_str(&map, "count"), None);
    assert_eq!(frontmatter_get_string(&map, "count"), None);
    assert_eq!(
        frontmatter_get(&map, "count"),
        Some(&JsonValue::Number(7.into()))
    );
}

#[test]
fn render_then_parse_roundtrips_arrays_and_objects() {
    let mut frontmatter = quaid::core::types::Frontmatter::new();
    frontmatter.insert("title".to_string(), json!("Alice"));
    frontmatter.insert("type".to_string(), json!("person"));
    frontmatter.insert("tags".to_string(), json!(["operator", "seed-stage"]));
    frontmatter.insert(
        "links".to_string(),
        json!({
            "works_at": "companies/acme",
            "founded": {
                "target": "companies/acme",
                "valid_from": "2024-01"
            }
        }),
    );

    let uuid = "01969f11-9448-7d79-8d3f-c68f54761234";
    let page = make_page(uuid, frontmatter.clone());
    let rendered = render_page(&page);

    let (reparsed, body) = parse_frontmatter(&rendered);
    let (truth, _timeline) = split_content(&body);
    assert_eq!(truth, "Body");

    // quaid_id is canonical-injected by render_page; strip it for comparison.
    let mut roundtripped = reparsed.clone();
    roundtripped.remove("quaid_id");

    assert_eq!(
        roundtripped.get("tags"),
        Some(&json!(["operator", "seed-stage"])),
        "tags array must survive render → parse"
    );

    let links = roundtripped
        .get("links")
        .and_then(JsonValue::as_object)
        .expect("links object must survive render → parse");
    assert_eq!(
        links.get("works_at"),
        Some(&JsonValue::String("companies/acme".to_string()))
    );
    let founded = links
        .get("founded")
        .and_then(JsonValue::as_object)
        .expect("nested founded object must survive render → parse");
    assert_eq!(
        founded.get("target"),
        Some(&JsonValue::String("companies/acme".to_string()))
    );
    assert_eq!(
        founded.get("valid_from"),
        Some(&JsonValue::String("2024-01".to_string()))
    );

    assert_eq!(
        reparsed.get("quaid_id").and_then(JsonValue::as_str),
        Some(uuid),
        "render_page must emit quaid_id from the persisted Page.uuid"
    );
}
