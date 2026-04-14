# Decision: T20 Novelty Detection Implementation

**Author:** Fry
**Date:** 2026-04-14
**Status:** Implemented

## Context

T20 requires a `check_novelty` function to prevent duplicate content from being ingested. The function must combine Jaccard token-set similarity with cosine similarity from stored embeddings when available.

## Decisions

1. **Dual-signal approach:** Jaccard similarity (whitespace-tokenised word sets) is always computed. Cosine similarity from stored page embeddings is used when the page has vectors in `page_embeddings_vec_384`. When both are available, they are averaged with equal weight.

2. **Similarity threshold:** Combined similarity ≥ 0.85 → content is NOT novel (likely duplicate). Below 0.85 → novel. This threshold balances false positives (rejecting genuine updates) vs false negatives (accepting near-duplicates).

3. **Existing text composition:** Both `compiled_truth` and `timeline` are concatenated for comparison, since timeline content is meaningful and should count toward deduplication.

4. **Embedding honesty:** The module doc comment explicitly acknowledges the T14 SHA-256 hash shim limitation. Cosine scores reflect hash proximity, not semantic similarity. Jaccard provides genuine token-level dedup regardless.

5. **Graceful degradation:** If no embeddings exist for the page, or embedding fails, the function falls back to Jaccard-only. No errors are surfaced for missing embeddings.

6. **Module-level `#![allow(dead_code)]`:** Applied because `check_novelty` is not yet wired into the ingest pipeline (that's T22 `migrate.rs` work). Will be removed when wired.

## Test coverage

- 4 Jaccard unit tests (identical, disjoint, partial overlap, both empty)
- 5 check_novelty integration tests (identical, clearly different, minor edit, substantial addition, timeline inclusion)
- Total: 9 new tests, 128 total (119 baseline + 9).
