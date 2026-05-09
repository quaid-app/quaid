## 1. Baseline & branch setup

- [x] 1.1 Create branch `refactor/collapse-search-fn-variants` off `main`.
- [x] 1.2 Capture pre-refactor `cargo test --all-targets` output to `/tmp/quaid-pretest.txt` (full pass list, used to diff against post-refactor).
- [x] 1.3 `grep -rn "search_fts\|hybrid_search" src/ tests/ --include="*.rs" > /tmp/quaid-precallsites.txt` and verify the file matches the inventory in `proposal.md` (catches any new caller introduced since the proposal was written).

## 2. Define `FtsQuery<'a>` and the new `core/fts.rs` public surface

- [x] 2.1 In `src/core/fts.rs`, declare `pub struct FtsQuery<'a>` with fields `query: &'a str`, `wing: Option<&'a str>`, `collection: Option<i64>`, `namespace: Option<&'a str>`, `include_superseded: bool`, `canonical: bool`, `limit: usize`, and `#[derive(Default, Clone)]`.
- [x] 2.2 Add rustdoc on `FtsQuery` showing the canonical call-site idiom (`FtsQuery { query, namespace: Some(ns), ..Default::default() }`) per Decision 7. Document each field's semantics in one line.
- [x] 2.3 Add the new public function `pub fn search_fts(conn: &Connection, q: FtsQuery<'_>) -> Result<Vec<SearchResult>, SearchError>`. Body delegates to the existing private `search_fts_internal` with arguments unpacked from `q`. Function-level rustdoc states the expert FTS5 contract (invalid syntax → `Err`) and links to `FtsQuery` for field documentation.
- [x] 2.4 Add the new public function `pub fn search_fts_tiered(conn: &Connection, q: FtsQuery<'_>) -> Result<Vec<SearchResult>, SearchError>`. Body delegates to the existing private `search_fts_tiered_internal`. Function-level rustdoc states the sanitized-input precondition.
- [x] 2.5 Compile (`cargo build --all-targets`). Combined with §7 deletions in this PR — the new struct-accepting names directly replace the old shim chains; intermediate coexistence skipped to keep the diff small.

## 3. Define `HybridSearch<'a>` and the new `core/search.rs` public surface

- [x] 3.1 In `src/core/search.rs`, declare `pub struct HybridSearch<'a>` with fields `query: &'a str`, `wing: Option<&'a str>`, `collection: Option<i64>`, `namespace: Option<&'a str>`, `include_superseded: bool`, `canonical: bool`, `limit: usize`, and `#[derive(Default, Clone)]`.
- [x] 3.2 Add rustdoc on `HybridSearch` showing the call-site idiom and documenting each field, mirroring 2.2.
- [x] 3.3 Add the new public function `pub fn hybrid_search(conn: &Connection, q: HybridSearch<'_>) -> Result<Vec<SearchResult>, SearchError>`. Body inlines what `hybrid_search_impl` currently does, calling the new struct-form `search_fts_tiered` from §2.4 (replacing both old `search_fts_canonical_tiered_with_namespace_filtered` and `search_fts_tiered_with_namespace_filtered` calls with one branch on `q.canonical`).
- [x] 3.4 Compile.

## 4. Migrate production callers

- [x] 4.1 Update `src/commands/query.rs:36` (`hybrid_search_canonical_with_namespace(...)` → `hybrid_search(&conn, HybridSearch { query: ..., wing: ..., collection: ..., namespace: ..., include_superseded: ..., canonical: true, limit: ... })`).
- [x] 4.2 Update `src/commands/search.rs:29` (`search_fts_canonical_with_namespace_filtered(...)` → `search_fts(&conn, FtsQuery { canonical: true, ... })`) and `src/commands/search.rs:39` (`search_fts_canonical_tiered_with_namespace_filtered(...)` → `search_fts_tiered(&conn, FtsQuery { canonical: true, ... })`). Adjust the import block at the top of the file accordingly.
- [x] 4.3 Update `src/mcp/server.rs:1198` (`hybrid_search_canonical_with_namespace(...)` → `hybrid_search(...)` with `canonical: true`) and `src/mcp/server.rs:1266` (`search_fts_canonical_with_namespace_filtered(...)` → `search_fts(...)` with `canonical: true`). Adjust imports.
- [x] 4.4 Compile and run `cargo test --lib` to catch any production-side regression early.

## 5. Migrate inline test callers in `core/fts.rs` and `core/search.rs`

- [x] 5.1 Rewrite every `search_fts(...)` call in the `core/fts.rs` `mod tests` block (~20 sites: empty-DB, basic match, wing filter, BM25 ranking, sanitize, expert FTS5 semantics, `search_fts_tiered` regressions) to the struct form. Each call's behavioral expectations remain unchanged.
- [x] 5.2 Rewrite every `search_fts_tiered(...)` call in the same `mod tests` block to the struct form.
- [x] 5.3 Rewrite every `hybrid_search(...)`, `hybrid_search_canonical(...)`, and `hybrid_search_canonical_with_namespace(...)` call in the `core/search.rs` `mod tests` block (~25 sites) to the struct form.
- [x] 5.4 Update the test imports at the top of each `mod tests` block: drop the removed names, add `FtsQuery` and `HybridSearch` where used.
- [x] 5.5 Run `cargo test --lib` and confirm pass count matches `/tmp/quaid-pretest.txt` for the lib tests.

## 6. Migrate integration test callers under `tests/`

- [x] 6.1 Update `tests/beir_eval.rs:318` and `tests/beir_eval.rs:408` (two `hybrid_search(...)` sites) to the struct form. Update imports.
- [x] 6.2 Update `tests/corpus_reality.rs:148`, `:179`, `:419` (`hybrid_search`) and `:464` (`search_fts`) to the struct form. Update imports.
- [x] 6.3 Update `tests/namespace_isolation.rs:20`, `:30`, `:40` (`hybrid_search_canonical_with_namespace`) to the struct form (`canonical: true`). Update imports.
- [x] 6.4 Update `tests/watcher_core.rs:316` (`fts::search_fts_canonical_tiered`) to the struct form (`search_fts_tiered` with `canonical: true`). Update imports.
- [x] 6.5 Run `cargo test --all-targets` and confirm pass count and pass list match `/tmp/quaid-pretest.txt`. (1357 passed pre and post; only timing variance in diff.)

## 7. Delete the old public variants in `core/fts.rs`

Note: deletions were performed inline with §2 — the new struct-accepting `search_fts`/`search_fts_tiered` directly replaced the old function bodies, and the other 10 wrappers were removed in the same edit. Tasks below describe the resulting state.

- [x] 7.1 Delete `pub fn search_fts_with_namespace` and its `#[allow(dead_code)]` annotation.
- [x] 7.2 Delete `pub fn search_fts_canonical` and its `#[allow(dead_code)]` annotation.
- [x] 7.3 Delete `pub fn search_fts_canonical_with_namespace`.
- [x] 7.4 Delete `pub fn search_fts_canonical_with_namespace_filtered`.
- [x] 7.5 Delete the original `pub fn search_fts` body and its `#[cfg_attr(not(test), allow(dead_code))]` — replaced by the new struct-accepting function from §2.3 (which now claims the name `search_fts`).
- [x] 7.6 Delete `pub fn search_fts_tiered` (positional-arg form) — replaced by §2.4's new function (which now claims the name `search_fts_tiered`).
- [x] 7.7 Delete `pub fn search_fts_tiered_with_namespace`.
- [x] 7.8 Delete `pub fn search_fts_tiered_with_namespace_filtered`.
- [x] 7.9 Delete `pub fn search_fts_canonical_tiered`.
- [x] 7.10 Delete `pub fn search_fts_canonical_tiered_with_namespace`.
- [x] 7.11 Delete `pub fn search_fts_canonical_tiered_with_namespace_filtered`.
- [x] 7.12 Reduce `#[expect(clippy::too_many_arguments)]` on `search_fts_tiered_internal` and `search_fts_internal` to a tighter `// reason:` form. Both private `_internal` functions still trip the lint on their own (8 args), so per the proposal's allowance, keep one `#[expect(...)]` per fn with `reason = "private SQL builder; public surface uses the FtsQuery struct"`. The public surface itself carries no such annotation. (Pre-refactor baseline used `#[expect(...)]` rather than `#[allow(...)]`, so no allow→expect migration needed.)
- [x] 7.13 `cargo build --all-targets` and `cargo test --all-targets` — both green, no warnings beyond the pre-refactor baseline.

## 8. Delete the old public variants in `core/search.rs`

- [x] 8.1 Delete `pub fn hybrid_search_with_namespace` and its `#[allow(dead_code)]` (was actually `#[expect(dead_code, ...)]` in the pre-refactor source).
- [x] 8.2 Delete `pub fn hybrid_search_canonical` and its `#[allow(dead_code)]`.
- [x] 8.3 Delete `pub fn hybrid_search_canonical_with_namespace`.
- [x] 8.4 Delete the original `pub fn hybrid_search` body and its `#[allow(dead_code)]` — replaced by §3.3's new struct-accepting function.
- [x] 8.5 Delete `fn hybrid_search_impl` and its `#[expect(clippy::too_many_arguments)]` — its body is folded into `hybrid_search` (§3.3). The `exact_slug_result_with_namespace` call from the old `_impl` is invoked from the new `hybrid_search` with the same arguments.
- [x] 8.6 `cargo build --all-targets` and `cargo test --all-targets` — both green.

## 9. Audit the three out-of-module `#[allow(dead_code)]` allowances

- [x] 9.1 Inspect `src/core/graph.rs` (`neighborhood_graph`). Found no `#[allow(dead_code)]` annotation on `main` (already removed by an earlier refactor — function has both production callers in `src/commands/graph.rs`/`src/mcp/server.rs` (the `_for_page` variant) and direct test coverage in `core/graph.rs::tests`). Nothing to change.
- [x] 9.2 Inspect `src/core/assertions.rs` (`check_assertions`). Same — no `#[allow(dead_code)]` remains. Function is called from `src/commands/validate.rs` (`fn check_assertions` wrapper) and from `core/assertions.rs::tests`. Nothing to change.
- [x] 9.3 Inspect `src/core/db.rs::open`. No `#[allow(dead_code)]` annotation on `main` — function is reached from many callers (default-model entry into `open_with_model`). Nothing to change.
- [x] 9.4 `grep -rn "#\[allow(dead_code)\]" src/core/fts.rs src/core/search.rs` returns zero. ✓
- [x] 9.5 `grep -rn "#\[allow(clippy::too_many_arguments)\]" src/core/fts.rs src/core/search.rs` returns zero. ✓ (Two `#[expect(...)]` annotations remain on the private `_internal` builders, per task 7.12 allowance — these are not `#[allow(...)]` and not on the public surface.)
- [x] 9.6 `grep -rn 'fn search_fts_with_namespace\|fn hybrid_search_canonical\|fn search_fts_canonical_with_namespace\|fn search_fts_tiered_with_namespace' src/ --include="*.rs"` returns only test-function names inside `#[cfg(test)] mod tests` blocks (e.g. `fn hybrid_search_canonical_preserves_collection_prefix_on_tiered_fts_recall`), not production `pub fn` definitions. ✓

## 10. Verify behavior preservation

- [x] 10.1 `cargo test --all-targets` — pre-refactor `/tmp/quaid-pretest.txt` (1357 passed) matches post-refactor `/tmp/quaid-posttest.txt` (1357 passed). Diff is timing-variance only.
- [x] 10.2 The four spec-scenario tests pass individually after the refactor: `search_fts_returns_error_on_invalid_fts5_syntax`, `search_tiered_compound_term_falls_back_to_or_when_and_empty`, `hybrid_search_namespace_filter_includes_global_and_excludes_other_namespaces`, `hybrid_search_accepts_question_mark_in_query`.
- [x] 10.3 `cargo clippy --all-targets -- -D warnings` is clean. (`--all-features --locked` form fails on a pre-existing `compile_error!` between the mutually-exclusive `embedded-model` and `online-model` feature flags — verified pre-existing on `main`, unrelated to this change.)
- [x] 10.4 `cargo fmt --all -- --check` passes (no diff).

## 11. Documentation

- [x] 11.1 Rustdoc on `FtsQuery` and `HybridSearch` shows the `..Default::default()` idiom inside an ```` ```ignore ```` code block. (Skipped the visual `cargo doc --open` confirmation — the rendered form is straightforward `rustdoc` markdown, no custom directives.)
- [x] 11.2 No stale intra-doc links to deleted variant names — `grep` confirmed.
- [x] 11.3 `CLAUDE.md` architecture table references `core/fts.rs` and `core/search.rs` by name only, not by individual function names; no update needed.

## 12. Commit, PR, and OpenSpec archive

- [ ] 12.1 Commit changes in two logical commits: (a) "Add `FtsQuery`/`HybridSearch` structs, migrate callers" (steps 2–6); (b) "Delete telescoping search/FTS variants and audit dead-code allowances" (steps 7–9). This keeps the diff bisectable.
- [ ] 12.2 Open a PR titled "Collapse telescoping search/FTS function variants into struct-of-options API". Link to `docs/CODE_REVIEW.md §1.6` and to this OpenSpec change directory. (User opens PRs themselves — agent prepares commits only.)
- [ ] 12.3 In the PR body, paste the line-count delta from `cloc src/core/fts.rs src/core/search.rs` before vs. after. Actual delta: net +95 LOC across the two files (250 added / 220 deleted in `fts.rs`; 251 added / 186 deleted in `search.rs`). The original ~–200 LOC target was a nominal forecast assuming positional one-line test calls would survive; the rustfmt-canonical multi-line struct-literal form for ~50 test callsites contributes ~150 LOC of growth that offsets the public-surface deletion. The actual API simplification (12+4 → 2+1 public functions, zero `#[allow]` on public surface) is the substantive win.
- [ ] 12.4 After merge, run `openspec archive collapse-search-fn-variants`.
