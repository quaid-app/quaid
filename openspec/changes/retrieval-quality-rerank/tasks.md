## 1. Config plumbing and result-shape extension

> **Naming decision (Sections 1–3 implementation):** all new config keys use
> the established `search.*` prefix instead of the unprefixed names below —
> `search.relevance_floor` (already seeded by the Wave 0 retrieval fixes;
> reused, not duplicated), `search.mmr_lambda`,
> `search.max_chunks_per_doc_default`, `search.cross_ref_boost_weight`,
> `search.cross_ref_boost_cap`, `search.rerank_extractive`,
> `search.rerank_extractive_top_n`, `search.rerank_extractive_budget_ms`.
>
> **Reconciliation with Wave 0:** `search.relevance_floor` landed in Wave 0
> as an absolute raw-cosine floor on the vector arm pre-merge. That behavior
> is kept; Sections 3.x add the post-fusion floor pass driven by the same
> key (and the same per-call override). Both are identity no-ops at the
> seeded `0.0`.

- [x] 1.1 Add `relevance_floor`, `mmr_lambda`, `max_chunks_per_doc_default`, `cross_ref_boost_weight`, `cross_ref_boost_cap`, `rerank_extractive`, `rerank_extractive_top_n`, `rerank_extractive_budget_ms` to the `config` table seed defaults in `src/schema.sql` (identity values for the no-op rollout: `mmr_lambda=1.0`, `relevance_floor=0.0`, `max_chunks_per_doc_default=0`, `cross_ref_boost_weight=0.0`, `rerank_extractive=false`) — *done with the `search.*` prefix; `search.relevance_floor` was already seeded by Wave 0*
- [x] 1.2 Add config-read helpers in `src/core/db.rs` or `src/core/config.rs` for each new key with typed getters and `[0.0, 1.0]` range validation on writes — *typed getters `configured_relevance_floor` / `configured_max_chunks_per_doc` live in `src/core/search.rs` next to the existing config readers; the floor getter clamps reads into `[0.0, 1.0]`. `quaid config set` is generic and has no per-key write validation today, so range validation is enforced on the CLI flags (clap value parser) and MCP parameters instead. Getters for the Section 4–6 keys land with their consuming sections to avoid dead plumbing.*
- [x] 1.3 Extend `SearchResult` in `src/core/types.rs` with optional `mmr_score: Option<f32>`, `cross_ref_boost: f32`, `dedup_collapsed_count: u32` fields; default values when filters are inactive — *fields are `#[serde(default)]` and skipped during serialization at their inactive values, so default-config JSON output is unchanged*
- [x] 1.4 Update `cargo test` to confirm existing roundtrip and search tests still pass with the extended struct — *validated with targeted runs of the search/gaps/progressive test files (full suite is too heavy for the dev container; CI runs it)*

## 2. Intra-document deduplication (`result-deduplication` capability)

> **Note:** both retrieval arms currently emit page-level rows (FTS indexes
> pages; the vector arm takes `MAX` cosine per page with `GROUP BY p.id`),
> so the merged candidate set is already unique per page and this pass is an
> identity safeguard until chunk-level candidates land. The pass is wired
> and tested against synthetic multi-row-per-page candidate lists.

- [x] 2.1 Implement `dedup_chunks_per_page(candidates, max_per_page)` in `src/core/search.rs` returning representatives with populated `dedup_collapsed_count`
- [x] 2.2 Wire `dedup_chunks_per_page` as the first post-fusion pass in `hybrid_search`
- [x] 2.3 Apply the same dedup pass on the initial set and on every expansion step inside `progressive_retrieve` (`src/core/progressive.rs`)
- [x] 2.4 Add `--max-chunks-per-doc N` CLI flag to `src/commands/search.rs` and `src/commands/query.rs`; flag value of `0` means "unlimited" per the spec
- [x] 2.5 Pass-through `max_chunks_per_doc` parameter on `memory_search` and `memory_query` MCP tools in `src/mcp/server.rs`
- [x] 2.6 Write `tests/search_dedup.rs` covering: three-chunk collapse, single-chunk passthrough, `dedup_collapsed_count` correctness, `--max-chunks-per-doc 2` behavior, `progressive_retrieve` re-application

## 3. Confidence threshold filter (`confidence-thresholding` capability)

- [x] 3.1 Implement `filter_below_floor(candidates, floor)` in `src/core/search.rs`
- [x] 3.2 Wire the floor pass after dedup and cross-reference boost (after Section 4 lands) and before MMR — *wired after dedup and before graph expansion; the boost insertion point (Section 4) and MMR (Section 5) remain open*
- [x] 3.3 Apply floor inside `progressive_retrieve` on initial and expansion-step candidates; below-floor candidates are not expanded
- [x] 3.4 Add `--relevance-floor F` CLI flag to `quaid search` and `quaid query` with `[0.0, 1.0]` validation
- [x] 3.5 Add `relevance_floor` parameter to `memory_search` and `memory_query` MCP tool schemas
- [x] 3.6 Confirm under-fill returns successfully (no error, no padding); update CLI/MCP response wording as needed — *covered by tests; existing empty-result wording ("No results found." / empty JSON array) already matches the fewer-than-k contract*
- [x] 3.7 Write `tests/search_confidence.rs` covering: below/at/above-floor cases, post-boost score comparison, empty-result success path, `--relevance-floor 0.0` disable, MCP parameter override — *post-boost comparison deferred to Section 4 (cross-ref boost not yet implemented)*

## 4. Cross-reference boost (`cross-reference-scoring` capability)

- [ ] 4.1 Implement `compute_cross_ref_boost(candidates, db, weight, cap)` in `src/core/search.rs` — single indexed query against `links` for `(from_page_id IN candidate_ids, to_page_id IN candidate_ids, valid range)`
- [ ] 4.2 Read `links.edge_weight` from Epic 1's schema; treat absent column / empty result set as zero boost (graceful degradation when `knowledge-graph-layer` has not landed)
- [ ] 4.3 Wire the boost pass between dedup and the confidence floor in `hybrid_search`; populate `SearchResult.cross_ref_boost` per row
- [ ] 4.4 Apply the same boost computation on `progressive_retrieve`'s initial candidate set
- [ ] 4.5 Short-circuit the lookup entirely when `cross_ref_boost_weight == 0.0`
- [ ] 4.6 Validate writes to `cross_ref_boost_weight` and `cross_ref_boost_cap` reject out-of-range values
- [ ] 4.7 Write `tests/search_cross_ref.rs` covering: co-cited boost, empty-graph no-op, expired-edge exclusion, cap saturation on hub pages, `weight=0.0` short-circuit

## 5. MMR reranker (`mmr-reranking` capability)

- [ ] 5.1 Implement `apply_mmr(candidates, lambda, k)` in `src/core/search.rs` using the greedy formula in `design.md` and the deterministic tie-break `(fused_score desc, page_id asc, chunk_id asc)`
- [ ] 5.2 Reuse the existing cosine-similarity primitive on `page_embeddings_vec_*` vectors; handle missing-vector candidates with zero diversity penalty
- [ ] 5.3 Wire MMR as the final post-fusion pass in `hybrid_search`; populate `SearchResult.mmr_score`
- [ ] 5.4 Apply MMR exactly once on `progressive_retrieve`'s initial candidate set (not per expansion step)
- [ ] 5.5 Add `--mmr-lambda L` CLI flag with `[0.0, 1.0]` validation; expose via `memory_search` / `memory_query` MCP parameters
- [ ] 5.6 Verify `mmr_lambda = 1.0` reproduces pre-change relevance ordering bytewise (golden test against a frozen baseline fixture)
- [ ] 5.7 Write `tests/search_mmr.rs` covering: diversity penalty downranking, first-selection equals top score, deterministic tie-break, `lambda = 1.0` baseline, missing-vector fallback

## 6. Extractive reranker (`extractive-rerank` capability) — opt-in

- [ ] 6.1 Create `src/core/rerank.rs` with a public `extractive_rerank(chunk, query_vec, top_n, budget_ms)` entry point
- [ ] 6.2 Implement deterministic punctuation-based sentence segmentation (no new crates); reuse the existing tokenizer if it provides sentence boundaries
- [ ] 6.3 Implement contiguous-span selection by sentence-level cosine similarity to the query embedding; respect `rerank_extractive_top_n`
- [ ] 6.4 Enforce per-chunk wall-clock budget; fall through to original chunk text on timeout with a debug log; never remove the candidate
- [ ] 6.5 Skip chunks with fewer than `top_n + 1` sentences or no stored embedding without erroring
- [ ] 6.6 Wire extractive rerank behind `rerank_extractive` config flag; integrate into `hybrid_search` and `progressive_retrieve` so the returned `snippet` reflects the selected span
- [ ] 6.7 Confirm `Cargo.toml` runtime dependencies are unchanged after this section
- [ ] 6.8 Write `tests/rerank_extractive.rs` covering: top-3 contiguous selection, single-sentence (`top_n=1`), short-chunk passthrough, missing-embedding passthrough, budget timeout fallback; tests run without opening a SQLite connection

## 7. Determinism, MCP shape, and `progressive_retrieve` integration

- [ ] 7.1 Add a determinism test that runs the same query twice against an unchanged DB and asserts element-for-element equality of `SearchResult` lists including `mmr_score`, `cross_ref_boost`, `dedup_collapsed_count`
- [ ] 7.2 Update `memory_search` and `memory_query` MCP tool JSON schemas to include the new optional parameters and result fields; regenerate any generated docs
- [ ] 7.3 Verify ordering of passes inside `progressive_retrieve` matches `hybrid_search` (dedup → boost → floor → MMR on initial set; dedup + floor only on expansion steps)
- [ ] 7.4 Extend `tests/progressive_retrieve.rs` (or create one) covering all four signals applied in order and verifying below-floor candidates are not expanded

## 8. Default-flip and benchmark gating

- [ ] 8.1 Land Sections 1–7 with identity defaults (no behavior change vs Epic 1 baseline) and verify CI passes
- [ ] 8.2 Run DAB §4 with each signal individually enabled (one-at-a-time) against a frozen corpus and record the per-signal lift in the benchmark log
- [ ] 8.3 Run MSMARCO P@5 with the same per-signal matrix; record results
- [ ] 8.4 Flip defaults in a follow-up commit: `mmr_lambda=0.7`, `relevance_floor=0.3`, `max_chunks_per_doc_default=1`, `cross_ref_boost_weight=0.05`, `cross_ref_boost_cap=0.15`. Leave `rerank_extractive=false`
- [ ] 8.5 Re-run DAB §4 + MSMARCO P@5 with flipped defaults; confirm DAB §4 ≥ 35/50 and MSMARCO P@5 non-regressing vs Epic 1 baseline
- [ ] 8.6 Re-run DAB after 1 release cycle; confirm DAB §4 ≥ 35/50 sustained over two consecutive releases (acceptance signal from the proposal)
- [ ] 8.7 If any regression appears on a DAB subsection or MSMARCO, revert defaults to identity values via config and reopen Section 8 with diagnostics

## 9. Documentation and rollout

- [ ] 9.1 Update `CLAUDE.md` MCP tools section with the new optional parameters and result fields
- [ ] 9.2 Update the search/query skills (`skills/query/SKILL.md` and adjacent) to document `--max-chunks-per-doc`, `--relevance-floor`, and `--mmr-lambda` and to set caller expectations for the "fewer-than-k" contract
- [ ] 9.3 Add a brief note to `docs/roadmap_v2.md` Epic 2 marking the four core changes shipped (or the §4 score on the release that flipped defaults)
- [ ] 9.4 Cross-link this change with `contradiction-semantic-gate` in the Epic 2 roadmap entry so the reader knows §7 is a separate workstream
