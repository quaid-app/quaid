# Compound Term Recall — Implementation Checklist

**Scope:** Add `expand_fts_query_or` helper and `search_fts_tiered` /
`search_fts_canonical_tiered` to `src/core/fts.rs`; wire tiered search into
`src/commands/search.rs` and `src/core/search.rs`.
Closes: #67, #69

---

## Phase A — src/core/fts.rs

- [x] A.1 Add `expand_fts_query_or(sanitized: &str) -> String`.
  - Split `sanitized` on whitespace.
  - If ≤ 1 token, return the input unchanged.
  - Otherwise join with ` OR `: `neural network inference` → `neural OR network OR inference`.
  - No FTS5 keyword quoting needed (tokens come from `sanitize_fts_query` output, which
    already quotes bare AND/OR/NOT/NEAR tokens).

- [x] A.2 Add `search_fts_tiered` (and `search_fts_canonical_tiered`) with the same
  signature as `search_fts` (and `search_fts_canonical`):
  ```rust
  pub fn search_fts_tiered(
      query: &str,
      wing_filter: Option<&str>,
      collection_filter: Option<i64>,
      conn: &Connection,
      limit: usize,
  ) -> Result<Vec<SearchResult>, SearchError>
  ```
  Implementation:
  ```rust
  let and_results = search_fts(query, wing_filter, collection_filter, conn, limit)?;
  if !and_results.is_empty() {
      return Ok(and_results);
  }
  // Only expand if there are multiple tokens to OR-join.
  let or_query = expand_fts_query_or(query);
  if or_query == query {
      return Ok(Vec::new()); // single token, already tried, no fallback possible
  }
  search_fts(&or_query, wing_filter, collection_filter, conn, limit)
  ```

- [x] A.3 Update `sanitize_fts_query` doc comment to note the three-layer architecture:
  sanitize → tiered search (AND → OR fallback). Reference `search_fts_tiered`.

- [x] A.4 Update `search_fts` doc comment to note it is called by `search_fts_tiered`
  as the AND pass, and to say expert callers should use `search_fts` directly for
  precise FTS5 control.

---

## Phase B — src/commands/search.rs

- [x] B.1 Import `search_fts_tiered` from `crate::core::fts` alongside the existing
  `search_fts` import.

- [x] B.2 On the non-`--raw` path in `run()`, replace:
  ```rust
  search_fts(&effective_query, ...)
  ```
  with:
  ```rust
  search_fts_tiered(&effective_query, ...)
  ```
  The `--raw` path must continue to call `search_fts` directly (no tiered fallback
  for expert users — they control OR expansion themselves via `term1 OR term2` syntax).

---

## Phase C — src/core/search.rs

- [x] C.1 Import `search_fts_tiered` and `search_fts_canonical_tiered` from
  `crate::core::fts`.

- [x] C.2 In `hybrid_search_impl`, replace:
  ```rust
  search_fts(&fts_safe, wing, collection_filter, conn, limit)?
  search_fts_canonical(&fts_safe, wing, collection_filter, conn, limit)?
  ```
  with:
  ```rust
  search_fts_tiered(&fts_safe, wing, collection_filter, conn, limit)?
  search_fts_canonical_tiered(&fts_safe, wing, collection_filter, conn, limit)?
  ```

---

## Phase D — tests

- [x] D.1 Unit test in `src/core/fts.rs`: `expand_fts_query_or("neural network inference")`
  returns `"neural OR network OR inference"`.

- [x] D.2 Unit test: `expand_fts_query_or("inference")` returns `"inference"` (single token,
  no OR expansion).

- [x] D.3 Unit test: `expand_fts_query_or("")` returns `""` (empty input, no change).

- [x] D.4 Integration test in `src/core/fts.rs`: insert a page whose content contains
  `inference` and `embedding` but NOT the exact sequence `neural network inference`.
  Assert `search_fts("neural network inference", ...)` returns empty (AND semantics confirmed).
  Assert `search_fts_tiered("neural network inference", ...)` returns the page (OR fallback
  triggered).

- [x] D.5 Integration test: insert a page whose content contains
  `neural network inference` verbatim. Assert `search_fts_tiered` returns it in a single
  AND pass (OR fallback is NOT triggered — verify by confirming result count matches AND-only).

- [x] D.6 Integration test: `search_fts_tiered` with a single-token query on an empty corpus
  returns empty vec without error (no fallback attempted for single-token queries).

- [x] D.7 Test in `src/commands/search.rs` (or `tests/`): call `run()` with `raw = false`
  and query `"neural network inference"` against a corpus with `inference` in one page —
  verify non-empty results (tiered search engaged on non-raw path).

- [x] D.8 Test: call `run()` with `raw = true` and query `"neural network inference"` against
  the same corpus — verify empty results (raw path uses AND semantics only, no tiered
  fallback).

---

## Phase E — verification

- [ ] E.1 All Phase D tests pass. Full `cargo test` suite green.

- [ ] E.2 Manually verify `quaid search "neural network inference"` returns results on a
  corpus that contains documents about ML inference. Close issues #67 and #69.

- [ ] E.3 Verify `quaid search --raw "neural network inference"` still returns zero on the
  same corpus (confirming raw path is unaffected).

- [ ] E.4 DAB benchmark §4 rerun: run the DAB v1.0 §4 Semantic/Hybrid slice against this
  branch. Confirm P04b paraphrase pair no longer produces zero results. Record result in
  issue #67 before closing the lane.
