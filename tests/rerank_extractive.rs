#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Extractive reranker tests (`extractive-rerank` capability, openspec change
//! `retrieval-quality-rerank` task 6.8).
//!
//! Scenarios tested:
//!   1. Top-3 contiguous span selection
//!   2. Single-sentence selection (`top_n = 1`)
//!   3. Short-chunk passthrough (fewer than `top_n + 1` sentences)
//!   4. Missing-embedding passthrough (a sentence fails to embed)
//!   5. Budget-timeout fall-through to original text
//!   6. Deterministic punctuation sentence segmentation
//!
//! These tests run entirely in-process and NEVER open a SQLite connection:
//! sentence relevance is injected through the `embed_sentence` closure, so the
//! span selection and budget logic are exercised in isolation.

use std::time::Duration;

use quaid::core::rerank::{extractive_rerank, segment_sentences, RerankOutcome};

const QUERY: [f32; 2] = [1.0, 0.0];

/// Sentences marked "RELEVANT" embed parallel to the query (cosine 1.0); all
/// others embed orthogonally (cosine 0.0). This makes the highest-scoring
/// contiguous span deterministic and independent of any model.
fn relevance_embedder(sentence: &str) -> Option<Vec<f32>> {
    if sentence.contains("RELEVANT") {
        Some(vec![1.0, 0.0])
    } else {
        Some(vec![0.0, 1.0])
    }
}

#[test]
fn selects_top_three_contiguous_sentences() {
    // 12 sentences; sentences 5,6,7 (1-based) are the relevant contiguous run.
    let mut parts: Vec<String> = Vec::new();
    for index in 1..=12 {
        if (5..=7).contains(&index) {
            parts.push(format!("Sentence {index} is RELEVANT here."));
        } else {
            parts.push(format!("Sentence {index} is filler text."));
        }
    }
    let chunk = parts.join(" ");

    let outcome = extractive_rerank(&chunk, &QUERY, 3, 1000, relevance_embedder);

    let snippet = match outcome {
        RerankOutcome::Selected(span) => span,
        RerankOutcome::PassedThrough(text) => {
            panic!("expected a selected span, got passthrough: {text}")
        }
    };
    assert!(snippet.contains("Sentence 5"), "span: {snippet}");
    assert!(snippet.contains("Sentence 6"), "span: {snippet}");
    assert!(snippet.contains("Sentence 7"), "span: {snippet}");
    assert!(
        !snippet.contains("Sentence 4") && !snippet.contains("Sentence 8"),
        "span must be exactly the 3 relevant sentences: {snippet}"
    );
}

#[test]
fn single_sentence_selection_with_top_n_one() {
    let chunk = "Alpha is filler. Beta is RELEVANT. Gamma is filler. Delta is filler.";
    let outcome = extractive_rerank(chunk, &QUERY, 1, 1000, relevance_embedder);

    let snippet = match outcome {
        RerankOutcome::Selected(span) => span,
        RerankOutcome::PassedThrough(text) => panic!("expected selection, got: {text}"),
    };
    assert_eq!(snippet, "Beta is RELEVANT.");
}

#[test]
fn short_chunk_passes_through_unchanged() {
    // 2 sentences, top_n = 3 → fewer than top_n + 1 → passthrough.
    let chunk = "First sentence is RELEVANT. Second sentence is filler.";
    let outcome = extractive_rerank(chunk, &QUERY, 3, 1000, relevance_embedder);

    match outcome {
        RerankOutcome::PassedThrough(text) => assert_eq!(text, chunk),
        RerankOutcome::Selected(span) => panic!("short chunk must pass through, got: {span}"),
    }
}

#[test]
fn missing_embedding_passes_through_unchanged() {
    let chunk = "One is fine. Two is fine. Three is fine. Four is fine.";
    // Embedder returns None for the third sentence → whole chunk passes through.
    let embedder = |sentence: &str| {
        if sentence.contains("Three") {
            None
        } else {
            Some(vec![1.0, 0.0])
        }
    };
    let outcome = extractive_rerank(chunk, &QUERY, 3, 1000, embedder);

    match outcome {
        RerankOutcome::PassedThrough(text) => assert_eq!(text, chunk),
        RerankOutcome::Selected(span) => {
            panic!("missing embedding must force passthrough, got: {span}")
        }
    }
}

#[test]
fn budget_timeout_falls_through_to_original_text() {
    let chunk = "One is fine. Two is fine. Three is fine. Four is fine.";
    // Each embed call sleeps 5ms; with a 1ms budget the first check after the
    // first embed already exceeds the deadline → passthrough.
    let embedder = |_sentence: &str| {
        std::thread::sleep(Duration::from_millis(5));
        Some(vec![1.0, 0.0])
    };
    let outcome = extractive_rerank(chunk, &QUERY, 3, 1, embedder);

    match outcome {
        RerankOutcome::PassedThrough(text) => assert_eq!(text, chunk),
        RerankOutcome::Selected(span) => panic!("over-budget chunk must pass through, got: {span}"),
    }
}

#[test]
fn empty_query_vector_passes_through() {
    let chunk = "One is RELEVANT. Two is filler. Three is filler. Four is filler.";
    let outcome = extractive_rerank(chunk, &[], 3, 1000, relevance_embedder);
    match outcome {
        RerankOutcome::PassedThrough(text) => assert_eq!(text, chunk),
        RerankOutcome::Selected(span) => panic!("empty query must pass through, got: {span}"),
    }
}

// ── Sentence segmentation ──────────────────────────────────────────────────

#[test]
fn segments_on_period_exclaim_question() {
    let sentences = segment_sentences("First sentence. Second one! Third one? Trailing");
    assert_eq!(
        sentences,
        vec![
            "First sentence.".to_string(),
            "Second one!".to_string(),
            "Third one?".to_string(),
            "Trailing".to_string(),
        ]
    );
}

#[test]
fn segmentation_keeps_runs_of_terminal_punctuation_together() {
    let sentences = segment_sentences("Wait... really?! Yes.");
    assert_eq!(
        sentences,
        vec![
            "Wait...".to_string(),
            "really?!".to_string(),
            "Yes.".to_string(),
        ]
    );
}

#[test]
fn segmentation_is_deterministic_for_identical_input() {
    let text = "Alpha beta. Gamma delta! Epsilon zeta?";
    assert_eq!(segment_sentences(text), segment_sentences(text));
}
