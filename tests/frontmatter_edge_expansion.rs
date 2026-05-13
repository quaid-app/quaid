#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure"
)]

//! Frontmatter edge/tag expansion coverage (tasks.md §3.6).
//!
//! Locks in the parse-only contract before any DB write path is wired:
//!   - `links:` accepts object form, string shorthand, and mixed lists,
//!   - `parent:`, `children:`, and `related:` map to fixed relationships,
//!   - object entries carry temporal `valid_from` / `valid_until` through
//!     unchanged,
//!   - malformed entries fail with actionable parse errors before mutation,
//!   - `tags:` accepts both YAML lists and comma-separated scalars and never
//!     produces edges at the parse layer.

use quaid::core::links::{
    expand_frontmatter_edges, expand_frontmatter_tags, FrontmatterLink, FrontmatterParseError,
};
use quaid::core::markdown::parse_frontmatter;
use quaid::core::types::Frontmatter;

fn fm(yaml: &str) -> Frontmatter {
    let raw = format!("---\n{yaml}---\nbody\n");
    let (frontmatter, _body) = parse_frontmatter(&raw);
    frontmatter
}

fn link(target: &str, relationship: &str) -> FrontmatterLink {
    FrontmatterLink {
        target: target.to_string(),
        relationship: relationship.to_string(),
        valid_from: None,
        valid_until: None,
    }
}

// ── links: object form ───────────────────────────────────────────────────

#[test]
fn object_form_link_keeps_target_relationship_and_temporal_fields() {
    let frontmatter =
        fm("links:\n  - target: companies/brex\n    type: founded\n    valid_from: 2017-01-01\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(
        edges,
        vec![FrontmatterLink {
            target: "companies/brex".to_string(),
            relationship: "founded".to_string(),
            valid_from: Some("2017-01-01".to_string()),
            valid_until: None,
        }]
    );
}

#[test]
fn object_form_link_normalizes_target_via_resolve_slug() {
    let frontmatter = fm("links:\n  - target: 'Companies/Brex Inc'\n    type: founded\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(edges[0].target, "companies/brex-inc");
}

#[test]
fn object_form_link_defaults_relationship_to_related_when_type_missing() {
    let frontmatter = fm("links:\n  - target: companies/brex\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(edges, vec![link("companies/brex", "related")]);
}

#[test]
fn object_form_link_preserves_valid_until_when_set() {
    let frontmatter = fm(
        "links:\n  - target: companies/brex\n    type: founded\n    valid_from: 2017-01-01\n    valid_until: 2024-12-31\n",
    );
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(edges[0].valid_from.as_deref(), Some("2017-01-01"));
    assert_eq!(edges[0].valid_until.as_deref(), Some("2024-12-31"));
}

// ── links: string shorthand ──────────────────────────────────────────────

#[test]
fn string_form_link_defaults_to_related_relationship() {
    let frontmatter = fm("links:\n  - companies/brex\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(edges, vec![link("companies/brex", "related")]);
}

#[test]
fn mixed_list_of_string_and_object_entries_is_supported() {
    let frontmatter =
        fm("links:\n  - companies/brex\n  - target: companies/scale\n    type: invested_in\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(
        edges,
        vec![
            link("companies/brex", "related"),
            link("companies/scale", "invested_in"),
        ]
    );
}

// ── parent / children / related ──────────────────────────────────────────

#[test]
fn parent_field_produces_single_parent_edge() {
    let frontmatter = fm("parent: programs/yc-w17\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(edges, vec![link("programs/yc-w17", "parent")]);
}

#[test]
fn children_field_produces_one_child_edge_per_entry() {
    let frontmatter = fm("children:\n  - companies/brex\n  - companies/scale\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(
        edges,
        vec![
            link("companies/brex", "child"),
            link("companies/scale", "child"),
        ]
    );
}

#[test]
fn related_field_produces_related_edges_for_each_entry() {
    let frontmatter = fm("related:\n  - people/alice\n  - people/bob\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(
        edges,
        vec![
            link("people/alice", "related"),
            link("people/bob", "related"),
        ]
    );
}

#[test]
fn scalar_related_field_is_coerced_to_one_related_edge() {
    let frontmatter = fm("related: karpathy-llm-wiki-workflow-breakdown\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("scalar related parses");
    assert_eq!(
        edges,
        vec![link("karpathy-llm-wiki-workflow-breakdown", "related")]
    );
}

#[test]
fn fixed_fields_normalize_targets_via_resolve_slug() {
    let frontmatter = fm("parent: 'Programs/YC W17'\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(edges[0].target, "programs/yc-w17");
}

#[test]
fn all_edge_sources_combine_in_order() {
    let frontmatter = fm(
        "links:\n  - companies/brex\nparent: programs/yc-w17\nchildren:\n  - companies/scale\nrelated:\n  - people/alice\n",
    );
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert_eq!(
        edges,
        vec![
            link("companies/brex", "related"),
            link("programs/yc-w17", "parent"),
            link("companies/scale", "child"),
            link("people/alice", "related"),
        ]
    );
}

// ── malformed entries fail before mutation ───────────────────────────────

#[test]
fn links_field_must_be_a_list_not_a_scalar() {
    let frontmatter = fm("links: companies/brex\n");
    let err = expand_frontmatter_edges(&frontmatter).expect_err("scalar links must reject");
    assert!(matches!(err, FrontmatterParseError::InvalidShape { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("links"),
        "actionable field name in error: {msg}"
    );
}

#[test]
fn children_field_must_be_a_list_not_a_scalar() {
    let frontmatter = fm("children: companies/brex\n");
    let err = expand_frontmatter_edges(&frontmatter).expect_err("scalar children must reject");
    assert!(matches!(err, FrontmatterParseError::InvalidShape { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("children"),
        "actionable field name in error: {msg}"
    );
}

#[test]
fn object_link_missing_target_is_rejected() {
    let frontmatter = fm("links:\n  - type: founded\n");
    let err = expand_frontmatter_edges(&frontmatter).expect_err("missing target must reject");
    assert!(matches!(err, FrontmatterParseError::MissingTarget { .. }));
    assert!(err.to_string().contains("links[0]"));
}

#[test]
fn object_link_with_non_string_target_is_rejected() {
    let frontmatter = fm("links:\n  - target: 42\n");
    let err = expand_frontmatter_edges(&frontmatter).expect_err("number target must reject");
    assert!(matches!(
        err,
        FrontmatterParseError::InvalidStringField { ref key, .. } if key == "target"
    ));
}

#[test]
fn object_link_with_unknown_key_is_rejected() {
    let frontmatter = fm("links:\n  - target: companies/brex\n    weight: 0.9\n");
    let err = expand_frontmatter_edges(&frontmatter).expect_err("unknown key must reject");
    assert!(matches!(err, FrontmatterParseError::UnknownKey { ref key, .. } if key == "weight"));
}

#[test]
fn object_link_with_non_string_temporal_field_is_rejected() {
    let frontmatter = fm("links:\n  - target: companies/brex\n    valid_from: 2017\n");
    let err = expand_frontmatter_edges(&frontmatter).expect_err("integer valid_from must reject");
    assert!(matches!(
        err,
        FrontmatterParseError::InvalidStringField { ref key, .. } if key == "valid_from"
    ));
}

#[test]
fn empty_string_target_is_rejected() {
    let frontmatter = fm("links:\n  - ''\n");
    let err = expand_frontmatter_edges(&frontmatter).expect_err("empty target must reject");
    assert!(matches!(err, FrontmatterParseError::EmptyTarget { .. }));
}

#[test]
fn parent_field_must_be_a_string_not_a_list() {
    let frontmatter = fm("parent:\n  - programs/yc-w17\n");
    let err = expand_frontmatter_edges(&frontmatter).expect_err("list parent must reject");
    assert!(
        matches!(err, FrontmatterParseError::InvalidShape { ref field, .. } if field == "parent")
    );
}

#[test]
fn children_entries_must_be_strings() {
    let frontmatter = fm("children:\n  - target: companies/brex\n");
    let err = expand_frontmatter_edges(&frontmatter).expect_err("object child must reject");
    assert!(
        matches!(err, FrontmatterParseError::InvalidShape { ref field, .. } if field == "children[0]")
    );
}

// ── tags ─────────────────────────────────────────────────────────────────

#[test]
fn tags_yaml_list_returns_each_tag_in_order() {
    let frontmatter = fm("tags:\n  - fintech\n  - yc-w17\n");
    assert_eq!(
        expand_frontmatter_tags(&frontmatter),
        vec!["fintech", "yc-w17"]
    );
}

#[test]
fn tags_comma_separated_scalar_is_split_and_trimmed() {
    let frontmatter = fm("tags: 'fintech, yc-w17 ,  agents'\n");
    assert_eq!(
        expand_frontmatter_tags(&frontmatter),
        vec!["fintech", "yc-w17", "agents"]
    );
}

#[test]
fn tags_missing_field_returns_empty_vec() {
    let frontmatter = fm("title: Brex\n");
    assert!(expand_frontmatter_tags(&frontmatter).is_empty());
}

#[test]
fn tags_empty_entries_are_dropped() {
    let frontmatter = fm("tags: 'fintech, , agents,'\n");
    assert_eq!(
        expand_frontmatter_tags(&frontmatter),
        vec!["fintech", "agents"]
    );
}

#[test]
fn tags_do_not_become_edges_at_parse_layer() {
    let frontmatter = fm("tags:\n  - fintech\n  - yc-w17\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    assert!(
        edges.is_empty(),
        "tags must never expand into graph edges, got {edges:?}"
    );
}

#[test]
fn tags_alongside_links_do_not_pollute_edges() {
    let frontmatter = fm("links:\n  - companies/brex\ntags:\n  - fintech\n  - yc-w17\n");
    let edges = expand_frontmatter_edges(&frontmatter).expect("edges parse");
    let tags = expand_frontmatter_tags(&frontmatter);
    assert_eq!(edges, vec![link("companies/brex", "related")]);
    assert_eq!(tags, vec!["fintech", "yc-w17"]);
}
