#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would generate noise across thousands of test sites"
)]
//! Public-API coverage for `resolve_model` after the flexible-model-resolution
//! change: the `medium`/`max` aliases, full HuggingFace ids normalising to
//! their alias equivalents, and arbitrary `owner/repo` ids being accepted as
//! custom models without error. These do not require a model channel feature
//! because `resolve_model` is pure alias-table lookup.

use quaid::core::inference::resolve_model;

#[test]
fn medium_alias_resolves_to_base() {
    let model = resolve_model("medium");
    assert_eq!(model.alias, "base");
    assert_eq!(model.model_id, "BAAI/bge-base-en-v1.5");
    assert_eq!(model.embedding_dim, 768);
}

#[test]
fn max_alias_resolves_to_m3() {
    let model = resolve_model("max");
    assert_eq!(model.alias, "m3");
    assert_eq!(model.model_id, "BAAI/bge-m3");
    assert_eq!(model.embedding_dim, 1024);
}

#[test]
fn full_hf_id_normalises_to_base_alias() {
    let from_id = resolve_model("BAAI/bge-base-en-v1.5");
    let from_alias = resolve_model("base");
    assert_eq!(from_id, from_alias);
}

#[test]
fn full_hf_id_normalises_to_m3_alias() {
    let from_id = resolve_model("BAAI/bge-m3");
    let from_alias = resolve_model("m3");
    assert_eq!(from_id, from_alias);
}

#[test]
fn arbitrary_owner_repo_id_is_accepted_as_custom() {
    let model = resolve_model("sentence-transformers/all-MiniLM-L6-v2");
    assert_eq!(model.alias, "custom");
    assert_eq!(model.model_id, "sentence-transformers/all-MiniLM-L6-v2");
    assert_eq!(model.embedding_dim, 0);
}

#[test]
fn custom_id_preserves_original_casing() {
    let model = resolve_model("MyOrg/My-Embedder");
    assert_eq!(model.alias, "custom");
    assert_eq!(model.model_id, "MyOrg/My-Embedder");
}

#[test]
fn aliases_are_case_and_whitespace_insensitive() {
    assert_eq!(resolve_model("  MEDIUM  ").alias, "base");
    assert_eq!(resolve_model("Max").alias, "m3");
}
