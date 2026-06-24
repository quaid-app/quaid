#![allow(
    missing_docs,
    reason = "build scripts are out of the public-api-docs scope per spec public-api-docs Requirement: Documentation scope boundary"
)]

// The embedded-model channel was removed (qwen3-models-airgapped §5): models
// are no longer baked in via `include_bytes!`, so there is nothing to prepare
// at build time. Quaid provisions its models on first use at runtime instead.
fn main() {}
