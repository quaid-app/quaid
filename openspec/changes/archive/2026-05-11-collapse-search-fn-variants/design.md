## Context

[src/core/fts.rs](../../../src/core/fts.rs) and
[src/core/search.rs](../../../src/core/search.rs) ended up in their
current shape through additive evolution: each new filter (canonical
slug, namespace, include-superseded) was hung off a *new* function name
rather than added to the existing entry point. The result is twelve
near-identical FTS variants and four near-identical hybrid variants,
all routing into one private `_internal` shim. The telescoping is
visible in the call sites — production code only ever calls the
maximally-flagged variants
([src/commands/query.rs:36](../../../src/commands/query.rs#L36),
[src/commands/search.rs:29](../../../src/commands/search.rs#L29),
[src/mcp/server.rs:1198](../../../src/mcp/server.rs#L1198),
[src/mcp/server.rs:1266](../../../src/mcp/server.rs#L1266)) while the
un-namespaced shorter names exist purely as test-facing convenience.

The four `#[allow(clippy::too_many_arguments)]` and seven
`#[allow(dead_code)]` annotations across these two files
([fts.rs:98,119,191,252,311,354](../../../src/core/fts.rs);
[search.rs:19,40,63,105](../../../src/core/search.rs)) are the
load-bearing tell that the surface is structurally wrong, not just
verbose.

The original code review captured the prescription verbatim
([docs/CODE_REVIEW.md §1.6](../../../docs/CODE_REVIEW.md), reproduced in
[docs/temp_IMPL_PLAN.md](../../../docs/temp_IMPL_PLAN.md) row 7):
collapse the twelve+four to a single entry point per concern, parameter
struct with `Default`, `..Default::default()` at every call site.

This proposal is the implementation of that prescription. It is the
only one of the six code-review follow-ups that touches `core/fts.rs`
and `core/search.rs` substantively, so it can ship in parallel with
the file-split, panic-removal, lints/CI, and inline-test extraction
proposals.

## Goals / Non-Goals

**Goals:**

- Replace the 12 public `search_fts*` variants with two public functions
  (`search_fts` for single-pass FTS5, `search_fts_tiered` for the
  AND→OR fallback path), each accepting `&Connection` plus a single
  `FtsQuery<'_>` value.
- Replace the 4 public `hybrid_search*` variants with one public
  function (`hybrid_search`) accepting `&Connection` plus a single
  `HybridSearch<'_>` value. Fold the private `hybrid_search_impl` into
  `hybrid_search` since the wrapper layer no longer adds anything.
- Eliminate every `#[allow(clippy::too_many_arguments)]` in the two
  files (4 occurrences) and every `#[allow(dead_code)]` whose function
  is removed by the variant collapse.
- Audit and close out the three dead-code allowances flagged in
  [docs/CODE_REVIEW.md §3.1](../../../docs/CODE_REVIEW.md) that live
  outside these two files
  ([core/graph.rs:69](../../../src/core/graph.rs#L69),
  [core/assertions.rs:112](../../../src/core/assertions.rs#L112),
  [core/db.rs:93](../../../src/core/db.rs#L93)) — keep with a
  `// reason:` comment, or delete with the function.
- Update every call site in `src/commands/`, `src/mcp/`, and `tests/`
  to the struct-literal form. Document the canonical idiom
  (`FtsQuery { query, namespace: Some(ns), ..Default::default() }`) in
  rustdoc on the parameter struct itself, where IDE hover-help will
  surface it.
- Preserve every existing test outcome — same queries, same results,
  same ordering, same error contract. The diff is mechanical at every
  call site, and the SQL builder body is untouched.

**Non-Goals:**

- No change to FTS5 sanitization
  ([fts.rs::sanitize_fts_query](../../../src/core/fts.rs#L33)). The
  existing `search-natural-language-safety` capability is preserved as
  written; that spec mentions `search_fts` by name and that name is
  retained on the new struct-accepting function.
- No change to the tiered AND→OR fallback algorithm. Same precision-
  first behavior, same OR fallback when AND returns empty and the
  sanitized query has more than one token, same single-token short-
  circuit.
- No change to the hybrid path's exact-slug short-circuit
  ([search.rs::exact_slug_query](../../../src/core/search.rs#L214)) or
  vector-search merge (set-union vs RRF) logic.
- No new functionality. No new filter (e.g. `min_score`, no new
  faceting). The struct is sized to the *existing* parameter set.
- No file split of `core/fts.rs` or `core/search.rs`. Their length is
  driven by the inline `mod tests` block, which is the
  `extract-inline-tests-to-integration` proposal's domain.
- No change to `lib.rs` re-exports or the external library API
  surface. The functions being collapsed are crate-public, not
  library-public.
- No migration to `#[expect(...)]`. That is the
  `add-rust-lints-and-ci-gate` proposal's job; this change deletes
  rather than re-tags the `#[allow(dead_code)]` annotations whose
  functions are removed, and where allowances are *retained* (the
  three out-of-module ones) we keep the `#[allow]` form plus a
  `// reason:` comment so the conventions converge cleanly when
  proposal #1 lands.

## Decisions

### Decision 1: One struct per module, not a shared `SearchQuery`

`FtsQuery` lives in `src/core/fts.rs`; `HybridSearch` lives in
`src/core/search.rs`. They have overlapping fields but different
concerns: `FtsQuery` is consumed by the SQL builder directly, while
`HybridSearch` orchestrates exact-slug + FTS-tiered + vector + merge.

Considered but rejected: a single `SearchOptions` struct shared between
the two modules. Rejected because (a) it would couple two layers that
are otherwise independent — `core/search.rs` would need to know
`FtsQuery` field semantics and vice versa — and (b) the two layers
will diverge: `HybridSearch` will eventually grow merge-strategy
and vector-`k` fields that have no meaning to FTS, and `FtsQuery` will
grow FTS5-specific options (e.g. raw-vs-sanitized, BM25-tuning) that
have no meaning to hybrid. Two structs lets each evolve independently.
The struct-literal idiom is the same shape at every call site, so the
"two structs" choice has no ergonomic cost.

### Decision 2: Canonical-slug behavior moves to a `bool` field, not a separate function

The historical `search_fts` vs `search_fts_canonical` split encoded the
output-slug format (`"slug"` vs `"<collection>::slug"`) in the function
name. The new shape moves it to `FtsQuery::canonical: bool` (default
`false`). Same for `HybridSearch::canonical`.

Considered but rejected: keeping `_canonical`-suffixed functions because
"canonical" is a *semantic* output difference, not a filter. Rejected
because the only distinction is which `slug_expr` is interpolated into
the SQL prefix
([fts.rs:370–379](../../../src/core/fts.rs#L370-L379)) and whether
hybrid uses the canonical exact-slug helper. Both branches already
exist in the `_internal` shims behind a `canonical_slug: bool` flag —
the public surface was already structurally a boolean, just dressed up
in two function names. Promoting the flag to the struct exposes that
truth.

### Decision 3: Keep `search_fts` and `search_fts_tiered` as two distinct entry points, not a single `tiered: bool` field

Both `search_fts` (single-pass FTS5, propagates invalid syntax as `Err`)
and `search_fts_tiered` (AND→OR fallback, takes a sanitized query) have
**different contracts**, not just different settings:

- `search_fts` is the expert interface; invalid FTS5 syntax is `Err`,
  which is depended upon by `tests/fts.rs::search_fts_returns_error_on_invalid_fts5_syntax`.
- `search_fts_tiered` requires a *sanitized* input; it sees its
  contract violated if a caller passes raw FTS5 syntax.

Considered but rejected: collapsing the two into `search_fts(...,
FtsQuery { tiered: true, ... })`. Rejected because it would smuggle a
contract change behind a `bool` and obscure the precondition about
sanitization. Two entry points, each accepting `FtsQuery<'_>`, keeps
the contracts visible at the call site. This still drops 10 of the 12
public functions (every `_with_namespace*`, `_canonical*`, `_filtered`
shim collapses), which is the LOC win the original review predicted.

### Decision 4: Fold `hybrid_search_impl` into `hybrid_search`

`hybrid_search_impl` exists today only because the four public
variants needed somewhere to land
([search.rs:106](../../../src/core/search.rs#L106)). Once the four
collapse to one, the `_impl` private wrapper has no purpose. The body
moves up into `hybrid_search` and the private name is deleted.

### Decision 5: `#[allow(dead_code)]` audit — delete-with-function vs retain-with-reason

Of the 9 dead-code allowances tracked in
[docs/CODE_REVIEW.md §3.1](../../../docs/CODE_REVIEW.md):

- 6 are on the un-namespaced search variants in
  [core/fts.rs](../../../src/core/fts.rs) and
  [core/search.rs](../../../src/core/search.rs). These get **deleted**
  along with their host function — the variant-collapse subsumes them.
- 3 are on functions outside the search modules:
  [`neighborhood_graph` (graph.rs:69)](../../../src/core/graph.rs#L69),
  [`check_assertions` (assertions.rs:112)](../../../src/core/assertions.rs#L112),
  [`open` (db.rs:93)](../../../src/core/db.rs#L93). These are tested
  via integration tests and/or kept for a documented purpose
  (`db::open` is the legacy default-model entry preserved for backward
  compatibility with `default_db_path()` callers). Each gets a
  one-line `// reason: ...` comment naming the test or future use that
  justifies retention. They keep `#[allow(...)]` for now;
  `add-rust-lints-and-ci-gate` will sweep the whole repo to
  `#[expect(...)]` later.

This split avoids over-reaching: the change owns the search-module
deletions because they fall out of the variant collapse, but it does
not pre-empt proposal #1's repo-wide `#[allow]`→`#[expect]` migration.

### Decision 6: No CLI- or MCP-visible behavior change

The hard acceptance bar is "every existing test passes with no
assertion edits beyond updating the call shape." Concretely:

- Each pre-existing inline test in `core/fts.rs` (~25 tests) is
  rewritten to use `FtsQuery { ..Default::default() }`, with the same
  query string and assertion outcomes.
- Each pre-existing inline test in `core/search.rs` (~30 tests) uses
  `HybridSearch { ..Default::default() }`.
- Each external integration test
  ([tests/beir_eval.rs](../../../tests/beir_eval.rs),
  [tests/corpus_reality.rs](../../../tests/corpus_reality.rs),
  [tests/namespace_isolation.rs](../../../tests/namespace_isolation.rs),
  [tests/watcher_core.rs](../../../tests/watcher_core.rs)) is updated
  call-by-call.

The diff is mechanical. Verification: `cargo test` runs green before
and after; spot-check a representative few (e.g.
`hybrid_search_uses_tiered_fts_recall_when_and_and_vector_are_empty`,
`search_fts_returns_error_on_invalid_fts5_syntax`,
`hybrid_search_namespace_filter_includes_global_and_excludes_other_namespaces`)
to confirm assertion outcomes are byte-identical.

### Decision 7: Document the call-site idiom on the struct, not the function

`pub struct FtsQuery` carries the rustdoc that shows
`FtsQuery { query, namespace: Some(ns), ..Default::default() }` as the
canonical call site. Function-level docs on `search_fts` and
`search_fts_tiered` link to the struct rather than re-listing fields.

This puts the idiom where IDE hover-help (`r-a`) will show it when a
user is filling in fields, instead of on the function where they only
see `q: FtsQuery<'_>` and have to chase the type to learn what's
inside.

## Risks / Trade-offs

- **Risk: Default values for `bool` fields could mask a caller bug at
  the moment of conversion.** A pre-refactor caller passing
  `include_superseded = false` and a post-refactor caller forgetting
  to set the field both end up with `false` — but if the original
  caller was passing `true` and the conversion mis-types `false`, the
  behavior changes silently.
  → **Mitigation:** the call-site update is mechanical and
  field-order-preserving; reviewers diff the post-refactor call against
  the pre-refactor positional args one parameter at a time. Tests
  exercise both the `true` and `false` branches of every flag
  (`exact_slug_result_canonical_hides_superseded_pages_by_default`
  covers `include_superseded = false`,
  `exact_slug_result_canonical_hides_superseded_pages_by_default`'s
  `historical_result` branch covers `true`).

- **Risk: A struct with optional fields invites callers to forget
  `..Default::default()` and only set some fields.** If `Default` is
  derived on a struct with non-`Copy` fields (`&'a str` is `Copy` but a
  hypothetical future `Vec<...>`-typed filter would not be), refactor
  callers may lock into shapes that won't compile under future
  extensions.
  → **Mitigation:** the rustdoc on the struct *requires* the
  `..Default::default()` form. Code review enforces the rule. The
  `search-api-shape` spec captures it as a normative requirement.

- **Trade-off: Two structs (`FtsQuery`, `HybridSearch`) instead of one
  shared `SearchOptions`.** Slightly more code (two `derive(Default)`
  declarations instead of one), but the modules stay decoupled. See
  Decision 1.

- **Trade-off: `search_fts` and `search_fts_tiered` are two functions
  instead of one with `tiered: bool`.** Slightly more public surface
  than the absolute minimum, but preserves the "expert vs sanitized"
  contract distinction. See Decision 3.

- **Risk: External tests under `tests/` that I haven't enumerated.**
  Running `grep -rn "search_fts\|hybrid_search" tests/` returned hits
  in four files; if the working tree or a feature branch adds a fifth,
  it needs to be updated too.
  → **Mitigation:** `cargo build --tests` will fail loudly on any
  missed call site (the public function signatures change, so old
  positional calls won't compile). CI `cargo test --all-targets` is
  the safety net.

- **Risk: The `add-rust-lints-and-ci-gate` proposal may land first and
  migrate `#[allow(...)]` → `#[expect(...)]` repo-wide.** If so, the
  three retained dead-code allowances in `core/graph.rs`,
  `core/assertions.rs`, `core/db.rs` will be `#[expect(dead_code,
  reason = "...")]` by the time this change touches them, not
  `#[allow(...)]`.
  → **Mitigation:** the audit step accepts both forms — keep the
  `#[expect(...)]` if present (it already has the `reason`), or add
  the `// reason:` comment if still on `#[allow(...)]`. The
  `search-api-shape` spec is written to accommodate either annotation
  syntax.

- **Risk: A reviewer assumes the surface change is library-API
  breaking.** It is not — `lib.rs` does not re-export
  `search_fts` or `hybrid_search`, and there are no external
  consumers (this is a binary crate plus a thin library used by its
  own integration tests).
  → **Mitigation:** the proposal's "Impact" section explicitly notes
  `BREAKING (internal-only)` and the design's Non-Goals call out "no
  change to `lib.rs` re-exports."

## Migration Plan

This is a single-PR mechanical refactor. No phased rollout.

1. **Branch off `main` (or current default).** No coordination with
   other in-flight proposals required — this proposal touches files
   that no other in-flight change touches at the function-body level.
2. **Land in this order within the PR:**
   1. Define `pub struct FtsQuery<'a>` in `core/fts.rs` (with rustdoc
      showing the call-site idiom).
   2. Add the new `pub fn search_fts(conn, q)` and `pub fn
      search_fts_tiered(conn, q)` next to the existing variants
      (temporarily compiling alongside them).
   3. Update every caller (`src/commands/`, `src/mcp/`, `src/core/`,
      and inline `mod tests` blocks in `core/fts.rs` and
      `core/search.rs`, plus integration tests under `tests/`) to use
      the new entry point.
   4. Delete the 10 old `search_fts*` public variants and the
      `#[allow(clippy::too_many_arguments)]` / `#[allow(dead_code)]`
      annotations they justified. Keep `search_fts_internal` and
      `search_fts_tiered_internal` as the private SQL builders with
      their original signatures (these are not the public surface).
   5. Repeat (a)–(d) for `core/search.rs`: define `HybridSearch<'a>`,
      add `pub fn hybrid_search(conn, q)`, fold
      `hybrid_search_impl` into the body, update callers, delete the
      three old public variants.
   6. Audit the three out-of-module `#[allow(dead_code)]`
      occurrences. Add `// reason: ...` comments. Delete any whose
      sole justification was a now-removed un-namespaced search
      variant.
   7. Run `cargo clippy --all-targets --all-features --locked -- -D
      warnings` and `cargo test --all-targets` (matching the
      `add-rust-lints-and-ci-gate` PR's CI command). Both must pass
      with zero warnings.
3. **Verification before merge:**
   - Diff the `cargo test` JSON output (`cargo test -- -Z
     unstable-options --format json` on nightly, or just stderr line
     count on stable) before and after the refactor; the test set
     and pass count must be unchanged.
   - Spot-check one representative scenario per requirement in
     `specs/search-api-shape/spec.md`: build a one-liner that
     evaluates the pre-refactor call and the post-refactor struct
     call against the same fixture and confirms identical output.
4. **Rollback:** revert the merge commit. There is no DB schema, no
   wire format, no on-disk artifact, no runtime config touched by this
   change — it is signature-and-call-site only.

## Open Questions

- Should the inline test moves implied by step 2(c)–(d) wait for the
  `extract-inline-tests-to-integration` proposal, or proceed in this PR?
  → **Recommendation:** rewrite the inline tests in place (keep them
  inline), let the inline-test proposal move them later. Mixing the
  signature change with a file-location move makes review harder. If
  the inline-test PR lands first, this PR rewrites the moved tests at
  their new location with no extra work.
- `expand_fts_query_or` and `sanitize_fts_query` are utilities, not
  search variants, but `expand_fts_query_or` is `pub` and currently
  only used inside `core/fts.rs`. Should it be downgraded to
  `pub(crate)` or `pub(super)`?
  → **Recommendation:** out of scope for this proposal. Visibility
  tightening is its own pass and would couple this change to a
  proposal it doesn't need.
