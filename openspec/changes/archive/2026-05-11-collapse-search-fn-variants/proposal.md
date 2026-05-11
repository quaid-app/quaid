## Why

[src/core/fts.rs](../../../src/core/fts.rs) exposes **12 public functions** that
are progressively thicker shims around one private `search_fts_internal`:
`search_fts`, `search_fts_with_namespace`, `search_fts_canonical`,
`search_fts_canonical_with_namespace`,
`search_fts_canonical_with_namespace_filtered`, `search_fts_tiered`,
`search_fts_tiered_with_namespace`, `search_fts_tiered_with_namespace_filtered`,
`search_fts_canonical_tiered`, `search_fts_canonical_tiered_with_namespace`,
`search_fts_canonical_tiered_with_namespace_filtered`, plus
`expand_fts_query_or`. Two carry `#[allow(clippy::too_many_arguments)]`
([fts.rs:311](../../../src/core/fts.rs#L311),
[fts.rs:354](../../../src/core/fts.rs#L354)) to silence the 7-positional-arg
warning. Four carry `#[allow(dead_code)]`
([fts.rs:98](../../../src/core/fts.rs#L98),
[fts.rs:119](../../../src/core/fts.rs#L119),
[fts.rs:191](../../../src/core/fts.rs#L191),
[fts.rs:252](../../../src/core/fts.rs#L252)) because nothing outside tests
calls the un-namespaced variants any more.

[src/core/search.rs](../../../src/core/search.rs) has the same shape — four
variants of `hybrid_search` (`hybrid_search`, `hybrid_search_with_namespace`,
`hybrid_search_canonical`, `hybrid_search_canonical_with_namespace`) plus
the private `hybrid_search_impl`, with three `#[allow(dead_code)]` and one
`#[allow(clippy::too_many_arguments)]`.

This is the textbook "telescoping function" smell called out in
[docs/CODE_REVIEW.md §1.6 and §5.1](../../../docs/CODE_REVIEW.md): every new
filter (canonical slug, namespace, include-superseded) doubled the surface
area instead of extending it. The result is hard for callers to choose
between, hard to grep for, and the next flag (e.g. `min_score`) would
double the surface again. Replacing both fan-outs with a single
`Default`-able parameter struct collapses ~200 LOC, eliminates every
`too_many_arguments` and dead-code shim in these two files, and makes the
call-site idiom self-documenting:
`FtsQuery { query, namespace: Some(ns), ..Default::default() }`.

This is proposal #6 in
[docs/temp_IMPL_PLAN.md](../../../docs/temp_IMPL_PLAN.md). It is independent
of the file-split and inline-test extraction proposals and can ship in
parallel.

## What Changes

- Introduce `FtsQuery<'a>` (in `src/core/fts.rs`) and `HybridSearch<'a>`
  (in `src/core/search.rs`) — `#[derive(Default, Clone)]` parameter
  structs covering every dimension the current variants encode: `query`,
  `wing`, `collection`, `namespace`, `include_superseded`, `canonical`,
  `limit` (plus `tiered: bool` for FTS, since both `search_fts` and
  `search_fts_tiered` exist as legitimate distinct entry points).
- Replace the 12 `search_fts*` variants with **two** public functions —
  `search_fts(conn, q)` (single-pass FTS5; expert callers and the AND
  arm of tiered search use this) and `search_fts_tiered(conn, q)`
  (AND-then-OR fallback for natural-language recall) — each accepting
  `FtsQuery<'_>`. Canonical-slug behavior moves from the function name
  into `FtsQuery::canonical: bool`. The `expand_fts_query_or` helper
  stays unchanged — it's a string utility, not a search variant.
  `sanitize_fts_query` also stays unchanged.
- Replace the 4 `hybrid_search*` variants with **one** public function —
  `hybrid_search(conn, q)` — accepting `HybridSearch<'_>`. The private
  `hybrid_search_impl` is folded into `hybrid_search` since the public
  wrapper no longer adds value.
- Update every caller in [src/commands/](../../../src/commands/),
  [src/mcp/](../../../src/mcp/), and [tests/](../../../tests/) to the
  struct-literal form. Concrete call-site list (verified via grep):
  [src/commands/query.rs:36](../../../src/commands/query.rs#L36),
  [src/commands/search.rs:29](../../../src/commands/search.rs#L29),
  [src/commands/search.rs:39](../../../src/commands/search.rs#L39),
  [src/mcp/server.rs:1198](../../../src/mcp/server.rs#L1198),
  [src/mcp/server.rs:1266](../../../src/mcp/server.rs#L1266),
  [src/mcp/server.rs:3033](../../../src/mcp/server.rs#L3033) (test),
  plus `tests/beir_eval.rs`, `tests/corpus_reality.rs`,
  `tests/namespace_isolation.rs`, `tests/watcher_core.rs`, and the inline
  test modules in `core/fts.rs` and `core/search.rs`.
- Delete every `#[allow(clippy::too_many_arguments)]` in these two files
  (4 occurrences). Delete every `#[allow(dead_code)]` whose function is
  removed (the un-namespaced variants — at least 6 of the 7 in these two
  files). Audit the remaining 3 dead-code allowances flagged in
  [docs/CODE_REVIEW.md §3.1](../../../docs/CODE_REVIEW.md):
  [src/core/graph.rs:69](../../../src/core/graph.rs#L69)
  (`neighborhood_graph`),
  [src/core/assertions.rs:112](../../../src/core/assertions.rs#L112)
  (`check_assertions`),
  [src/core/db.rs:93](../../../src/core/db.rs#L93) (`open`). Anything
  kept gets a `// reason:` comment per the
  `add-rust-lints-and-ci-gate` proposal's `#[expect(...)]` convention,
  or is migrated to `#[expect(dead_code, reason = "…")]` if/when that
  proposal lands first.
- **BREAKING (internal-only):** every removed `pub fn` is a crate-public
  surface change. There are no external consumers of these functions
  (`pub` in this crate ≠ exported library API; `lib.rs` re-exports a
  narrow surface and these functions are reached only from within the
  crate plus integration tests in this repo). No CLI or MCP behavior
  changes — same queries return the same results in the same order.

Explicitly **out of scope**: no behavior change to FTS5 sanitization,
tiered AND→OR fallback logic, vector-search merge strategy, exact-slug
short-circuit, or namespace filtering. The internal SQL is unchanged —
the same `search_fts_internal` body executes for every code path. Only
the function-surface routing changes.

## Capabilities

### New Capabilities
- `search-api-shape`: Codifies the struct-of-options idiom for
  search/FTS entry points in this crate — what the parameter structs
  contain, the `..Default::default()` call-site idiom, the rule that
  new filters extend the struct rather than adding a `_with_<flag>`
  function variant, and the prohibition on `#[allow(clippy::too_many_arguments)]`
  on public search functions.

### Modified Capabilities
<!-- None — `search-natural-language-safety` references `search_fts` by
name, and that name is preserved (only its signature changes). The
sanitize/raw/JSON-error requirements continue to hold verbatim because
the internal SQL is unchanged. -->

## Impact

- **Affected files (production):**
  [src/core/fts.rs](../../../src/core/fts.rs) (12 fns → 2 fns + struct,
  ~150 LOC delta),
  [src/core/search.rs](../../../src/core/search.rs) (4 fns → 1 fn +
  struct, ~80 LOC delta),
  [src/commands/query.rs](../../../src/commands/query.rs),
  [src/commands/search.rs](../../../src/commands/search.rs),
  [src/mcp/server.rs](../../../src/mcp/server.rs) (2 production
  call-sites + 1 test call-site).
- **Affected files (tests):**
  [tests/beir_eval.rs](../../../tests/beir_eval.rs),
  [tests/corpus_reality.rs](../../../tests/corpus_reality.rs),
  [tests/namespace_isolation.rs](../../../tests/namespace_isolation.rs),
  [tests/watcher_core.rs](../../../tests/watcher_core.rs), plus the
  inline `mod tests` blocks in `core/fts.rs` and `core/search.rs`.
- **LOC delta:** ~–200 net per
  [docs/CODE_REVIEW.md §8 row 7](../../../docs/CODE_REVIEW.md). The
  internal-routing functions and shim chains evaporate; struct
  declarations add a few dozen lines.
- **APIs:** no new `pub` items added to the library export surface
  (`lib.rs`). Crate-internal `pub fn` surface shrinks; struct types
  `FtsQuery` / `HybridSearch` become the new crate-internal entry
  points.
- **Behavior:** none changed. Every existing test (`cargo test`)
  continues to pass with identical assertion outcomes — the same
  queries return the same results in the same order. This is the hard
  acceptance bar.
- **Dependencies:** none added or removed.
- **Risk:** low. The diff is mechanical (call-site updates + signature
  collapse), test coverage already exercises every variant, and the
  internal SQL builder
  ([fts.rs:355–442](../../../src/core/fts.rs#L355-L442)) is untouched.
  The `Default`-able struct keeps every existing call expressible.
- **Coordination:** independent of `add-rust-lints-and-ci-gate`,
  `extract-inline-tests-to-integration`,
  `decompose-vault-sync-module`, `decompose-mcp-server-module`, and
  `remove-production-panic-paths`. Touches files those changes don't,
  and the dead-code-allowance audit overlaps cleanly:
  `add-rust-lints-and-ci-gate` migrates `#[allow]` → `#[expect]`
  globally; this change deletes the un-namespaced variants those
  allowances guarded so the migration converges on a smaller surface.
