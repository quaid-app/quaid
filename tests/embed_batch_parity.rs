#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! `embed_batch` must produce the same vectors as per-text `embed`, so the
//! batched ingest path and the single-text query path stay comparable in
//! vector space. Padding in the batched forward pass is masked out, so a mix
//! of short and long inputs must not perturb any row's embedding.

use quaid::core::inference::{embed, embed_batch};
use quaid::core::types::InferenceError;

#[test]
fn embed_batch_matches_per_text_embed_within_tolerance() {
    let texts = [
        "short",
        "a substantially longer sentence about vector search and embeddings",
        "brex is a corporate card fintech company",
        "x",
    ];

    let batched = embed_batch(&texts).expect("batch embed");
    assert_eq!(batched.len(), texts.len());

    for (text, batch_vec) in texts.iter().zip(batched.iter()) {
        let single = embed(text).expect("single embed");
        assert_eq!(
            single.len(),
            batch_vec.len(),
            "dimension mismatch for {text:?}"
        );
        let max_abs_diff = single
            .iter()
            .zip(batch_vec.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_abs_diff < 1e-4,
            "batched embedding for {text:?} diverges from single embed (max abs diff {max_abs_diff})"
        );
    }
}

#[test]
fn embed_batch_rejects_blank_inputs_and_handles_empty() {
    assert!(matches!(
        embed_batch(&["valid", "   "]),
        Err(InferenceError::EmptyInput)
    ));
    assert!(embed_batch(&[]).expect("empty batch ok").is_empty());
}
