//! Opt-in extractive sentence reranker.
//!
//! When `config.search.rerank_extractive` is enabled, each candidate chunk's
//! snippet is replaced by the most query-relevant contiguous span of
//! `rerank_extractive_top_n` sentences. Sentence relevance is the cosine
//! similarity between a sentence's embedding and the query embedding. The pass
//! is deterministic, dependency-free (no new crates, no LLM, no network), and
//! enforces a per-chunk wall-clock budget: a chunk that exceeds the budget
//! falls through to its original text with a debug log and is never removed
//! from the result set.
//!
//! The module is intentionally free of any SQLite dependency so it can be
//! unit-tested in isolation by injecting synthetic sentence vectors through the
//! `embed_sentence` closure (see [`extractive_rerank`]).
//!
//! See also: `core::search` and `core::progressive`, which gate this pass
//! behind the `rerank_extractive` config flag and supply the real embedder.

use std::time::{Duration, Instant};

/// Outcome of an extractive rerank attempt on a single chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RerankOutcome {
    /// A span was selected; the wrapped string is the new snippet.
    Selected(String),
    /// The chunk passed through unchanged (too short, no embeddings, or the
    /// per-chunk budget was exceeded). The wrapped string is the original
    /// chunk text.
    PassedThrough(String),
}

impl RerankOutcome {
    /// The resulting snippet for either outcome.
    pub fn snippet(&self) -> &str {
        match self {
            Self::Selected(text) | Self::PassedThrough(text) => text,
        }
    }

    /// Consume the outcome, returning the resulting snippet.
    pub fn into_snippet(self) -> String {
        match self {
            Self::Selected(text) | Self::PassedThrough(text) => text,
        }
    }
}

/// Select the most query-relevant contiguous span of up to `top_n` sentences
/// from `chunk` and return it as the new snippet.
///
/// `query_vec` is the query embedding. `embed_sentence` produces a sentence's
/// embedding (or `None` when it cannot be embedded); injecting it keeps the
/// function database- and model-free for unit testing. `budget_ms` is the
/// per-chunk wall-clock budget — when sentence embedding plus scoring exceeds
/// it, the chunk passes through with its original text and a debug log.
///
/// The chunk passes through unchanged (no embedding attempts) when it contains
/// fewer than `top_n + 1` sentences, when `query_vec` is empty, or when any
/// required sentence fails to embed.
pub fn extractive_rerank<F>(
    chunk: &str,
    query_vec: &[f32],
    top_n: usize,
    budget_ms: u64,
    mut embed_sentence: F,
) -> RerankOutcome
where
    F: FnMut(&str) -> Option<Vec<f32>>,
{
    let original = chunk.to_owned();
    let sentences = segment_sentences(chunk);

    // Short chunk: fewer than top_n + 1 sentences means there is nothing to
    // select from — pass through unchanged.
    if top_n == 0 || query_vec.is_empty() || sentences.len() < top_n + 1 {
        return RerankOutcome::PassedThrough(original);
    }

    let deadline = Instant::now() + Duration::from_millis(budget_ms);

    // Embed each sentence, scoring against the query. A missing embedding for
    // any sentence aborts the attempt (the chunk lacks usable vectors). The
    // budget is checked between sentences so a long chunk falls through cleanly
    // rather than blocking.
    let mut scores: Vec<f64> = Vec::with_capacity(sentences.len());
    for sentence in &sentences {
        if Instant::now() >= deadline {
            // Debug-level diagnostic. The codebase has no `log`/`tracing`
            // dependency (and task 6.7 forbids adding one), so this mirrors the
            // `eprintln!`-based diagnostics used elsewhere in `core`.
            eprintln!(
                "DEBUG: extractive_rerank per-chunk budget of {budget_ms}ms exceeded; \
                 falling through to original chunk text"
            );
            return RerankOutcome::PassedThrough(original);
        }
        let Some(sentence_vec) = embed_sentence(sentence) else {
            return RerankOutcome::PassedThrough(original);
        };
        scores.push(cosine_similarity(&sentence_vec, query_vec));
    }

    match best_contiguous_span(&scores, top_n) {
        Some((start, end)) => {
            let span = sentences[start..end].join(" ");
            RerankOutcome::Selected(span)
        }
        None => RerankOutcome::PassedThrough(original),
    }
}

/// Find the contiguous window of `top_n` sentences (a shorter window only when
/// the chunk has exactly `top_n` sentences, which the caller already excludes)
/// with the maximum summed score. Returns the `[start, end)` index range.
///
/// Ties on summed score break toward the earliest start index, which makes the
/// selection deterministic for identical inputs.
fn best_contiguous_span(scores: &[f64], top_n: usize) -> Option<(usize, usize)> {
    if scores.is_empty() || top_n == 0 {
        return None;
    }
    let window = top_n.min(scores.len());
    let mut best_start = 0usize;
    // Sum of the first window.
    let mut window_sum: f64 = scores[..window].iter().sum();
    let mut best_sum = window_sum;

    // Slide the window one position at a time, updating the rolling sum.
    for start in 1..=(scores.len() - window) {
        window_sum += scores[start + window - 1] - scores[start - 1];
        // Strictly greater keeps the earliest window on ties (determinism).
        if window_sum > best_sum {
            best_sum = window_sum;
            best_start = start;
        }
    }

    Some((best_start, best_start + window))
}

/// Deterministic punctuation-based sentence segmentation.
///
/// A sentence boundary is a run of one or more terminal punctuation marks
/// (`.`, `!`, `?`) followed by whitespace or end-of-input. Terminal
/// punctuation is retained on the sentence it ends. Leading/trailing
/// whitespace is trimmed and empty fragments are dropped. No tokenizer model
/// or external crate is used, so the split is byte-stable across runs and
/// platforms.
pub fn segment_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        current.push(ch);
        if ch == '.' || ch == '!' || ch == '?' {
            // Consume any further terminal punctuation (e.g. "?!", "...").
            let mut look = index + 1;
            while look < chars.len()
                && (chars[look] == '.' || chars[look] == '!' || chars[look] == '?')
            {
                current.push(chars[look]);
                look += 1;
            }
            // A boundary requires whitespace or end-of-input after the run.
            let at_boundary = look >= chars.len() || chars[look].is_whitespace();
            if at_boundary {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    sentences.push(trimmed.to_owned());
                }
                current.clear();
            }
            index = look;
            continue;
        }
        index += 1;
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_owned());
    }

    sentences
}

/// Cosine similarity between two `f32` vectors using f64 accumulation for
/// numerical stability. Returns `0.0` for mismatched-length or empty inputs.
fn cosine_similarity(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut left_norm = 0.0f64;
    let mut right_norm = 0.0f64;
    for (l, r) in left.iter().zip(right.iter()) {
        let l = *l as f64;
        let r = *r as f64;
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}
