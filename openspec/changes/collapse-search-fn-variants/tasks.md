## 1. Baseline & branch setup

- [ ] 1.1 Create branch `refactor/collapse-search-fn-variants` off `main`.
- [ ] 1.2 Capture pre-refactor `cargo test --all-targets` output to `/tmp/quaid-pretest.txt` (full pass list, used to diff against post-refactor).
- [ ] 1.3 `grep -rn "search_fts\|hybrid_search" src/ tests/ --include="*.rs" > /tmp/quaid-precallsites.txt` and verify the file matches the inventory in `proposal.md` (catches any new caller introduced since the proposal was written).

## 2. Define `FtsQuery<'a>` and the new `core/fts.rs` public surface

- [ ] 2.1 In `src/core/fts.rs`, declare `pub struct FtsQuery<'a>` with fields `query: &'a str`, `wing: Option<&'a str>`, `collection: Option<i64>`, `namespace: Option<&'a str>`, `include_superseded: bool`, `canonical: bool`, `limit: usize`, and `#[derive(Default, Clone)]`.
- [ ] 2.2 Add rustdoc on `FtsQuery` showing the canonical call-site idiom (`FtsQuery { query, namespace: Some(ns), ..Default::default() }`) per Decision 7. Document each field's semantics in one line.
- [ ] 2.3 Add the new public function `pub fn search_fts(conn: &Connection, q: FtsQuery<'_>) -> Result<Vec<SearchResult>, SearchError>`. Body delegates to the existing private `search_fts_internal` with arguments unpacked from `q`. Function-level rustdoc states the expert FTS5 contract (invalid syntax → `Err`) and links to `FtsQuery` for field documentation.
- [ ] 2.4 Add the new public function `pub fn search_fts_tiered(conn: &Connection, q: FtsQuery<'_>) -> Result<Vec<SearchResult>, SearchError>`. Body delegates to the existing private `search_fts_tiered_internal`. Function-level rustdoc states the sanitized-input precondition.
- [ ] 2.5 Compile (`cargo build --all-targets`). New surface coexists with old at this point — both must compile.

## 3. Define `HybridSearch<'a>` and the new `core/search.rs` public surface

- [ ] 3.1 In `src/core/search.rs`, declare `pub struct HybridSearch<'a>` with fields `query: &'a str`, `wing: Option<&'a str>`, `collection: Option<i64>`, `namespace: Option<&'a str>`, `include_superseded: bool`, `canonical: bool`, `limit: usize`, and `#[derive(Default, Clone)]`.
- [ ] 3.2 Add rustdoc on `HybridSearch` showing the call-site idiom and documenting each field, mirroring 2.2.
- [ ] 3.3 Add the new public function `pub fn hybrid_search(conn: &Connection, q: HybridSearch<'_>) -> Result<Vec<SearchResult>, SearchError>`. Body inlines what `hybrid_search_impl` currently does, calling the new struct-form `search_fts_tiered` from §2.4 (replacing both old `search_fts_canonical_tiered_with_namespace_filtered` and `search_fts_tiered_with_namespace_filtered` calls with one branch on `q.canonical`).
- [ ] 3.4 Compile.

## 4. Migrate production callers

- [ ] 4.1 Update `src/commands/query.rs:36` (`hybrid_search_canonical_with_namespace(...)` → `hybrid_search(&conn, HybridSearch { query: ..., wing: ..., collection: ..., namespace: ..., include_superseded: ..., canonical: true, limit: ... })`).
- [ ] 4.2 Update `src/commands/search.rs:29` (`search_fts_canonical_with_namespace_filtered(...)` → `search_fts(&conn, FtsQuery { canonical: true, ... })`) and `src/commands/search.rs:39` (`search_fts_canonical_tiered_with_namespace_filtered(...)` → `search_fts_tiered(&conn, FtsQuery { canonical: true, ... })`). Adjust the import block at the top of the file accordingly.
- [ ] 4.3 Update `src/mcp/server.rs:1198` (`hybrid_search_canonical_with_namespace(...)` → `hybrid_search(...)` with `canonical: true`) and `src/mcp/server.rs:1266` (`search_fts_canonical_with_namespace_filtered(...)` → `search_fts(...)` with `canonical: true`). Adjust imports.
- [ ] 4.4 Compile and run `cargo test --lib` to catch any production-side regression early.

## 5. Migrate inline test callers in `core/fts.rs` and `core/search.rs`

- [ ] 5.1 Rewrite every `search_fts(...)` call in the `core/fts.rs` `mod tests` block (~20 sites: empty-DB, basic match, wing filter, BM25 ranking, sanitize, expert FTS5 semantics, `search_fts_tiered` regressions) to the struct form. Each call's behavioral expectations remain unchanged.
- [ ] 5.2 Rewrite every `search_fts_tiered(...)` call in the same `mod tests` block to the struct form.
- [ ] 5.3 Rewrite every `hybrid_search(...)`, `hybrid_search_canonical(...)`, and `hybrid_search_canonical_with_namespace(...)` call in the `core/search.rs` `mod tests` block (~25 sites) to the struct form.
- [ ] 5.4 Update the test imports at the top of each `mod tests` block: drop the removed names, add `FtsQuery` and `HybridSearch` where used.
- [ ] 5.5 Run `cargo test --lib` and confirm pass count matches `/tmp/quaid-pretest.txt` for the lib tests.

## 6. Migrate integration test callers under `tests/`

- [ ] 6.1 Update `tests/beir_eval.rs:318` and `tests/beir_eval.rs:408` (two `hybrid_search(...)` sites) to the struct form. Update imports.
- [ ] 6.2 Update `tests/corpus_reality.rs:148`, `:179`, `:419` (`hybrid_search`) and `:464` (`search_fts`) to the struct form. Update imports.
- [ ] 6.3 Update `tests/namespace_isolation.rs:20`, `:30`, `:40` (`hybrid_search_canonical_with_namespace`) to the struct form (`canonical: true`). Update imports.
- [ ] 6.4 Update `tests/watcher_core.rs:316` (`fts::search_fts_canonical_tiered`) to the struct form (`search_fts_tiered` with `canonical: true`). Update imports.
- [ ] 6.5 Run `cargo test --all-targets` and confirm pass count and pass list match `/tmp/quaid-pretest.txt`.

## 7. Delete the old public variants in `core/fts.rs`

- [ ] 7.1 Delete `pub fn search_fts_with_namespace` ([core/fts.rs:99](../../../src/core/fts.rs#L99)) and its `#[allow(dead_code)]` annotation.
- [ ] 7.2 Delete `pub fn search_fts_canonical` ([core/fts.rs:120](../../../src/core/fts.rs#L120)) and its `#[allow(dead_code)]` annotation.
- [ ] 7.3 Delete `pub fn search_fts_canonical_with_namespace` ([core/fts.rs:131](../../../src/core/fts.rs#L131)).
- [ ] 7.4 Delete `pub fn search_fts_canonical_with_namespace_filtered` ([core/fts.rs:150](../../../src/core/fts.rs#L150)).
- [ ] 7.5 Delete the original `pub fn search_fts` body ([core/fts.rs:87](../../../src/core/fts.rs#L87)) and its `#[cfg_attr(not(test), allow(dead_code))]` — replaced by the new struct-accepting function from §2.3 (which now claims the name `search_fts`).
- [ ] 7.6 Delete `pub fn search_fts_tiered` ([core/fts.rs:192](../../../src/core/fts.rs#L192)) and its `#[allow(dead_code)]` — replaced by §2.4's new function (which now claims the name `search_fts_tiered`).
- [ ] 7.7 Delete `pub fn search_fts_tiered_with_namespace` ([core/fts.rs:210](../../../src/core/fts.rs#L210)).
- [ ] 7.8 Delete `pub fn search_fts_tiered_with_namespace_filtered` ([core/fts.rs:229](../../../src/core/fts.rs#L229)).
- [ ] 7.9 Delete `pub fn search_fts_canonical_tiered` ([core/fts.rs:253](../../../src/core/fts.rs#L253)) and its `#[allow(dead_code)]`.
- [ ] 7.10 Delete `pub fn search_fts_canonical_tiered_with_namespace` ([core/fts.rs:271](../../../src/core/fts.rs#L271)).
- [ ] 7.11 Delete `pub fn search_fts_canonical_tiered_with_namespace_filtered` ([core/fts.rs:290](../../../src/core/fts.rs#L290)).
- [ ] 7.12 Remove `#[allow(clippy::too_many_arguments)]` from `search_fts_tiered_internal` ([core/fts.rs:311](../../../src/core/fts.rs#L311)) and `search_fts_internal` ([core/fts.rs:354](../../../src/core/fts.rs#L354)). The signatures stay the same — these are private builders the new public functions delegate into — but the public surface no longer triggers the warning, so the suppression is no longer load-bearing. Confirm clippy stays green: if the private `_internal` fns themselves still trip the lint they were already silencing pre-refactor, keep one suppression per fn with a `// reason: private SQL builder; public surface uses FtsQuery struct.` comment.
- [ ] 7.13 `cargo build --all-targets` and `cargo test --all-targets` — both green, no warnings beyond the pre-refactor baseline.

## 8. Delete the old public variants in `core/search.rs`

- [ ] 8.1 Delete `pub fn hybrid_search_with_namespace` ([core/search.rs:41](../../../src/core/search.rs#L41)) and its `#[allow(dead_code)]`.
- [ ] 8.2 Delete `pub fn hybrid_search_canonical` ([core/search.rs:64](../../../src/core/search.rs#L64)) and its `#[allow(dead_code)]`.
- [ ] 8.3 Delete `pub fn hybrid_search_canonical_with_namespace` ([core/search.rs:84](../../../src/core/search.rs#L84)).
- [ ] 8.4 Delete the original `pub fn hybrid_search` body ([core/search.rs:20](../../../src/core/search.rs#L20)) and its `#[allow(dead_code)]` — replaced by §3.3's new struct-accepting function.
- [ ] 8.5 Delete `fn hybrid_search_impl` ([core/search.rs:106](../../../src/core/search.rs#L106)) and its `#[allow(clippy::too_many_arguments)]` — its body is folded into `hybrid_search` (§3.3). Confirm the `exact_slug_result_with_namespace` call from the old `_impl` is invoked from the new `hybrid_search` with the same arguments.
- [ ] 8.6 `cargo build --all-targets` and `cargo test --all-targets` — both green.

## 9. Audit the three out-of-module `#[allow(dead_code)]` allowances

- [ ] 9.1 Inspect `src/core/graph.rs:69` (`neighborhood_graph`). Confirm whether any non-test caller exists (`grep -rn "neighborhood_graph" src/ tests/`). If retained: prepend `// reason: <test or future caller>` on the line above the `#[allow(dead_code)]`. If unreachable from production *and* tests: delete the function.
- [ ] 9.2 Inspect `src/core/assertions.rs:112` (`check_assertions`). Same treatment as 9.1.
- [ ] 9.3 Inspect `src/core/db.rs:93` (`open`). The default-model legacy entry — confirm it's reached via `default_db_path()` callers. Add `// reason: legacy default-model entry; preserved for callers that don't pass a model alias` (or the actual reason from the code) above the `#[allow(dead_code)]`.
- [ ] 9.4 Run `grep -rn "#\\[allow(dead_code)\\]" src/core/fts.rs src/core/search.rs` — must return zero. (`search-api-shape` spec scenario.)
- [ ] 9.5 Run `grep -rn "#\\[allow(clippy::too_many_arguments)\\]" src/core/fts.rs src/core/search.rs` — must return zero. (`search-api-shape` spec scenario.)
- [ ] 9.6 Run `grep -rn "fn search_fts_with_namespace\\|fn hybrid_search_canonical\\|fn search_fts_canonical_with_namespace\\|fn search_fts_tiered_with_namespace" src/ --include="*.rs"` — must return zero matches in production source. (Removed-variants scenario.)

## 10. Verify behavior preservation

- [ ] 10.1 Run `cargo test --all-targets 2>&1 | tee /tmp/quaid-posttest.txt`. Diff `/tmp/quaid-pretest.txt` against `/tmp/quaid-posttest.txt`: pass list and pass count must be identical.
- [ ] 10.2 Spot-check the four spec-scenario tests by running each individually and confirming both "before" and "after" produce the same result/error: `search_fts_returns_error_on_invalid_fts5_syntax`, `search_tiered_compound_term_falls_back_to_or_when_and_empty`, `hybrid_search_namespace_filter_includes_global_and_excludes_other_namespaces`, `hybrid_search_accepts_question_mark_in_query`.
- [ ] 10.3 Run `cargo clippy --all-targets --all-features --locked -- -D warnings`. Must pass with no new warnings beyond the pre-refactor baseline. (If `add-rust-lints-and-ci-gate` has landed, this is the strict gate; if not, it's the looser default-clippy gate already in CI.)
- [ ] 10.4 Run `cargo fmt --all -- --check`. Must pass.

## 11. Documentation

- [ ] 11.1 Confirm the rustdoc on `FtsQuery` and `HybridSearch` shows the `..Default::default()` call-site idiom per Decision 7. Render with `cargo doc --no-deps --open` and visually confirm the example renders as code, not prose.
- [ ] 11.2 Update any stale function-level rustdoc on the deleted variants if remnants survived (search for `[`search_fts_with_namespace`]` and similar broken intra-doc links in the surviving rustdoc; replace with `[`search_fts`]` or `[`FtsQuery`]`).
- [ ] 11.3 Confirm `CLAUDE.md` references to `core/fts.rs` and `core/search.rs` (the architecture table) still match the new shape — no doc update needed unless a name in the table changed (it should not).

## 12. Commit, PR, and OpenSpec archive

- [ ] 12.1 Commit changes in two logical commits: (a) "Add `FtsQuery`/`HybridSearch` structs, migrate callers" (steps 2–6); (b) "Delete telescoping search/FTS variants and audit dead-code allowances" (steps 7–9). This keeps the diff bisectable.
- [ ] 12.2 Open a PR titled "Collapse telescoping search/FTS function variants into struct-of-options API". Link to `docs/CODE_REVIEW.md §1.6` and to this OpenSpec change directory.
- [ ] 12.3 In the PR body, paste the line-count delta from `cloc src/core/fts.rs src/core/search.rs` before vs. after, confirming the ~–200 LOC target from the original review.
- [ ] 12.4 After merge, run `openspec archive collapse-search-fn-variants` (or the workflow's archive command) to mark the change complete and write the spec into `openspec/specs/search-api-shape/`.
