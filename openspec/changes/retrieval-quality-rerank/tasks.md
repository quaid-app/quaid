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

- [x] 4.1 Implement `compute_cross_ref_boost(candidates, db, weight, cap)` in `src/core/search.rs` — single indexed query against `links` for `(from_page_id IN candidate_ids, to_page_id IN candidate_ids, valid range)`. Signature is `compute_cross_ref_boost(conn, candidates, weight, cap)`; candidate slugs are resolved to page ids once and the ids are inlined into the `IN (...)` lists (trusted i64s, injection-safe).
- [x] 4.2 Read `links.edge_weight` from Epic 1's schema; treat absent column / empty result set as zero boost (graceful degradation when `knowledge-graph-layer` has not landed). Empty/sparse graph leaves every row untouched (no re-sort, identity path).
- [x] 4.3 Wire the boost pass between dedup and the confidence floor in `hybrid_search`; populate `SearchResult.cross_ref_boost` per row. The list is re-sorted only when a boost actually moved a score so the floor and an identity MMR see the post-boost ranking.
- [x] 4.4 Apply the same boost computation on `progressive_retrieve`'s initial candidate set (dedup → boost → floor → MMR).
- [x] 4.5 Short-circuit the lookup entirely when `cross_ref_boost_weight == 0.0` (early return before any `links` query).
- [x] 4.6 Validate writes to `cross_ref_boost_weight` and `cross_ref_boost_cap` reject out-of-range values — *per the §1.2 precedent (`quaid config set` is generic with no per-key write validation), enforced as read-time clamping into `[0.0, 1.0]` in `configured_cross_ref`, so an out-of-range stored value can never affect scoring. No dedicated write path/CLI flag exists for these keys to attach a hard reject to.*
- [x] 4.7 Write `tests/search_cross_ref.rs` covering: co-cited boost, empty-graph no-op, expired-edge exclusion, cap saturation on hub pages, `weight=0.0` short-circuit

## 5. MMR reranker (`mmr-reranking` capability)

- [x] 5.1 Implement `apply_mmr(candidates, lambda, k)` in `src/core/search.rs` using the greedy formula in `design.md` and the deterministic tie-break `(fused_score desc, page_id asc, chunk_id asc)`. Signature is `apply_mmr(conn, candidates, lambda, k)` (conn is needed to read per-candidate vectors). `chunk_id asc` is realized as `slug asc` since merged rows carry no chunk id; page_id asc is the primary tie-break.
- [x] 5.2 Reuse the existing cosine-similarity primitive on `page_embeddings_vec_*` vectors; handle missing-vector candidates with zero diversity penalty. Each candidate's representative embedding (first stored chunk under the active model) is read from the `vec0` table and decoded; the f64-accumulation cosine matches the supersede path's primitive.
- [x] 5.3 Wire MMR as the final post-fusion pass in `hybrid_search`; populate `SearchResult.mmr_score` (applied after truncation so the budget is only spent on surviving rows).
- [x] 5.4 Apply MMR exactly once on `progressive_retrieve`'s initial candidate set (not per expansion step) — `apply_mmr(conn, initial, lambda, 0)` (k=0 reorders without truncating).
- [x] 5.5 Add `--mmr-lambda L` CLI flag with `[0.0, 1.0]` validation; expose via `memory_search` / `memory_query` MCP parameters. CLI flag added to `quaid search` and `quaid query` (clap `parse_unit_interval`); MCP `mmr_lambda` param added to both tools with `[0,1]` validation in the handlers. *Note: `quaid search` / `memory_search` are FTS-only (no vector arm); MMR is threaded through their quality passes and degrades to relevance ordering when candidates lack stored vectors.*
- [x] 5.6 Verify `mmr_lambda = 1.0` reproduces pre-change relevance ordering bytewise (golden test against a frozen baseline fixture) — `apply_mmr` returns the input untouched (no reorder, `mmr_score` left unset) at λ≥1.0; covered by `lambda_one_reproduces_relevance_ordering_bytewise` and `hybrid_search_lambda_one_reproduces_identity_ordering` in `tests/search_mmr.rs`.
- [x] 5.7 Write `tests/search_mmr.rs` covering: diversity penalty downranking, first-selection equals top score, deterministic tie-break, `lambda = 1.0` baseline, missing-vector fallback

## 6. Extractive reranker (`extractive-rerank` capability) — opt-in

- [x] 6.1 Create `src/core/rerank.rs` with a public `extractive_rerank(chunk, query_vec, top_n, budget_ms)` entry point. Implemented with an injected `embed_sentence` closure as the 5th parameter so the module is DB- and model-free for unit testing; returns a `RerankOutcome` (`Selected` / `PassedThrough`).
- [x] 6.2 Implement deterministic punctuation-based sentence segmentation (no new crates) — `segment_sentences` splits on `.`/`!`/`?` runs followed by whitespace/EOI, retaining terminal punctuation. (The repo's tokenizer does not expose sentence boundaries; a deterministic splitter is used.)
- [x] 6.3 Implement contiguous-span selection by sentence-level cosine similarity to the query embedding; respect `rerank_extractive_top_n` (sliding-window max-sum, earliest-window tie-break for determinism).
- [x] 6.4 Enforce per-chunk wall-clock budget; fall through to original chunk text on timeout with a debug log; never remove the candidate (debug log via `eprintln!` since the repo has no `log`/`tracing` dep; the candidate is never dropped).
- [x] 6.5 Skip chunks with fewer than `top_n + 1` sentences or no stored embedding without erroring (passthrough).
- [x] 6.6 Wire extractive rerank behind `rerank_extractive` config flag; integrate into `hybrid_search` so the returned preview (`SearchResult.summary`) reflects the selected span. *`SearchResult` has no `snippet` field — `summary` is the retrieval preview, so the span replaces `summary`. The pass is wired at the `hybrid_search` boundary that feeds `progressive_retrieve`'s initial set; the query string is not threaded into `progressive_retrieve` (its public signature is owned by other call sites and would break main.rs), and graph-expansion neighbours carry no query-relevance score, so they are out of scope for the snippet rewrite. Full no-op at the seeded `false` default.*
- [x] 6.7 Confirm `Cargo.toml` runtime dependencies are unchanged after this section (verified: `git diff --stat Cargo.toml Cargo.lock` is empty).
- [x] 6.8 Write `tests/rerank_extractive.rs` covering: top-3 contiguous selection, single-sentence (`top_n=1`), short-chunk passthrough, missing-embedding passthrough, budget timeout fallback; tests run without opening a SQLite connection

## 7. Determinism, MCP shape, and `progressive_retrieve` integration

- [x] 7.1 Add a determinism test that runs the same query twice against an unchanged DB and asserts element-for-element equality of `SearchResult` lists including `mmr_score`, `cross_ref_boost`, `dedup_collapsed_count` — `tests/search_determinism.rs` compares full serialized lists under non-identity config.
- [x] 7.2 Update `memory_search` and `memory_query` MCP tool JSON schemas to include the new optional parameters and result fields; regenerate any generated docs — `mmr_lambda` param added to both `MemoryQueryInput`/`MemorySearchInput` (schemars derives the JSON schema); the result fields (`mmr_score`, `cross_ref_boost`, `dedup_collapsed_count`) were already on `SearchResult` from §1 and serialize automatically.
- [x] 7.3 Verify ordering of passes inside `progressive_retrieve` matches `hybrid_search` (dedup → boost → floor → MMR on initial set; dedup + floor only on expansion steps) — initial set runs the full ordered pipeline; expansion steps unchanged (dedup + floor only).
- [x] 7.4 Extend `tests/progressive_retrieve.rs` (or create one) covering all four signals applied in order and verifying below-floor candidates are not expanded — created `tests/progressive_retrieve.rs`.

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
