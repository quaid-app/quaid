## ADDED Requirements

### Requirement: Search and FTS entry points use a Default-able parameter struct

Public search and FTS5 entry points in `src/core/fts.rs` and
`src/core/search.rs` SHALL accept their per-call options through a single
`#[derive(Default, Clone)]` struct rather than as positional arguments
or as a fan-out of `_with_<flag>` function variants. New filters
(future: `min_score`, additional facets) SHALL be added by extending
the struct, not by introducing a new sibling function.

The two parameter structs are:

- `FtsQuery<'a>` in `src/core/fts.rs`, covering: `query: &'a str`,
  `wing: Option<&'a str>`, `collection: Option<i64>`,
  `namespace: Option<&'a str>`, `include_superseded: bool`,
  `canonical: bool`, `limit: usize`.
- `HybridSearch<'a>` in `src/core/search.rs`, covering the same fields
  used by the hybrid path (no `canonical`-specific function name; the
  flag lives on the struct).

Both structs SHALL implement `Default` so callers can write the
struct-literal idiom `FtsQuery { query, namespace: Some(ns),
..Default::default() }` and `HybridSearch { query, include_superseded:
false, ..Default::default() }` without naming every field.

#### Scenario: Calling search_fts with only the required field

- **WHEN** a caller invokes `search_fts(&conn, FtsQuery { query: "rust", ..Default::default() })`
- **THEN** the call compiles, executes the same SQL the previous
  `search_fts(query, None, None, &conn, 0)` shape produced (with
  `limit: 0` interpreted identically), and returns the same result set
  in the same order

#### Scenario: Calling hybrid_search with namespace and canonical filters

- **WHEN** a caller invokes `hybrid_search(&conn, HybridSearch { query: "AI founder", namespace: Some("test-ns"), canonical: true, limit: 10, ..Default::default() })`
- **THEN** the call returns the same results as the previous
  `hybrid_search_canonical_with_namespace("AI founder", None, None, Some("test-ns"), false, &conn, 10)`
  invocation, in the same order

#### Scenario: Adding a new filter does not introduce a new function variant

- **WHEN** a future change adds a new filter (e.g. `min_score: Option<f64>`) to FTS or hybrid search
- **THEN** the new filter SHALL be added as a field on `FtsQuery` /
  `HybridSearch` with a sensible default, and SHALL NOT be exposed via
  a new `*_with_min_score` function

### Requirement: Public search functions stay below the too_many_arguments threshold

No public function in `src/core/fts.rs` or `src/core/search.rs` SHALL
carry a `#[allow(clippy::too_many_arguments)]` annotation. The
parameter-struct idiom from the requirement above is the means by which
this is enforced — public functions accept at most `&Connection` plus
one query/options struct.

#### Scenario: Auditing the search modules for too_many_arguments allowances

- **WHEN** a reviewer runs `grep -rn "#\\[allow(clippy::too_many_arguments)\\]" src/core/fts.rs src/core/search.rs`
- **THEN** the command returns zero matches

### Requirement: Removed un-namespaced search variants stay removed

The historical un-namespaced public variants `search_fts_with_namespace`,
`search_fts_canonical`, `search_fts_canonical_with_namespace`,
`search_fts_canonical_with_namespace_filtered`,
`search_fts_tiered_with_namespace`,
`search_fts_tiered_with_namespace_filtered`,
`search_fts_canonical_tiered`,
`search_fts_canonical_tiered_with_namespace`,
`search_fts_canonical_tiered_with_namespace_filtered`,
`hybrid_search_with_namespace`, `hybrid_search_canonical`, and
`hybrid_search_canonical_with_namespace` SHALL NOT be reintroduced.
The struct-of-options pattern subsumes every previous variant; adding
back a function-name-encoded flag would re-introduce the maintenance
burden the change set out to remove.

#### Scenario: Reviewer searches for a deleted variant by name

- **WHEN** a reviewer runs `grep -rn "fn search_fts_with_namespace\\|fn hybrid_search_canonical" src/`
- **THEN** the command returns zero matches in production source

### Requirement: Behavior preservation across the variant-collapse refactor

The struct-of-options refactor SHALL NOT change the result set, ordering,
or error contract of any FTS or hybrid search call. Every pre-existing
test that exercised one of the historical 12 FTS or 4 hybrid variants
SHALL continue to pass after the refactor with assertions unchanged
(only the call-site syntax updated).

The internal FTS5 SQL builder
(`src/core/fts.rs::search_fts_internal`) and the tiered AND→OR fallback
(`src/core/fts.rs::search_fts_tiered_internal`) SHALL retain identical
SQL output, identical parameter binding order, and identical handling
of empty queries, FTS5 syntax errors, wing/collection/namespace
filters, the `include_superseded` flag, and canonical-vs-bare slug
formatting. The hybrid path's exact-slug short-circuit, sanitization
gate, vector-search merge, and `read_merge_strategy` interaction
SHALL be unchanged.

The expert-vs-natural-language contract from the existing
`search-natural-language-safety` capability SHALL continue to hold:
`search_fts` propagates invalid FTS5 syntax as `Err`; the
`sanitize_fts_query` helper is applied by callers (`commands/search.rs`
unless `--raw`, `mcp/server.rs::memory_search`, and `hybrid_search`
on the natural-language path) before invoking `search_fts` or
`search_fts_tiered`.

#### Scenario: Question mark in natural-language hybrid query still succeeds

- **WHEN** a caller invokes `hybrid_search(&conn, HybridSearch { query: "what is rust?", limit: 1000, ..Default::default() })`
- **THEN** the call returns `Ok(...)` with the same result set as the
  previous `hybrid_search("what is rust?", None, None, false, &conn, 1000)`
  call

#### Scenario: Invalid FTS5 syntax through search_fts still returns an error

- **WHEN** a caller invokes `search_fts(&conn, FtsQuery { query: "rust?", limit: 1000, ..Default::default() })`
- **THEN** the call returns `Err(SearchError::...)`, matching the
  pre-refactor behavior of `search_fts("rust?", None, None, &conn, 1000)`

#### Scenario: Tiered AND→OR fallback path still recovers compound-term recall

- **WHEN** a caller invokes `search_fts_tiered(&conn, FtsQuery { query: "neural network inference", limit: 1000, ..Default::default() })`
  against a corpus where no single page contains all three tokens but
  multiple pages contain individual tokens
- **THEN** the call returns the OR-fallback result set (the same set
  the previous `search_fts_tiered("neural network inference", None,
  None, &conn, 1000)` returned), in the same order

#### Scenario: Namespace-aware hybrid query preserves global-namespace fallback

- **WHEN** a caller invokes `hybrid_search(&conn, HybridSearch { query: "sharedtoken privatetoken", namespace: Some("test-ns"), canonical: true, limit: 10, ..Default::default() })`
- **THEN** the result set includes both the `test-ns`-namespaced page
  and any global-namespace (`namespace = ''`) match, identical to the
  pre-refactor `hybrid_search_canonical_with_namespace` behavior

### Requirement: Public search modules carry no dead-code allowances on retained variants

After the refactor, `src/core/fts.rs` and `src/core/search.rs` SHALL
contain zero `#[allow(dead_code)]` annotations. Variants tied to
removed un-namespaced fan-outs SHALL be deleted with the function;
any function that survives the refactor SHALL be reachable from
production callers, integration tests, or both, with no suppression
required.

The three dead-code allowances that live outside the search modules —
`src/core/graph.rs::neighborhood_graph`,
`src/core/assertions.rs::check_assertions`, and `src/core/db.rs::open` —
SHALL be audited as part of this change. Each that is retained SHALL
either be migrated to `#[expect(dead_code, reason = "...")]` (when the
`add-rust-lints-and-ci-gate` change has landed first) or carry a
`// reason: ...` comment that names the future caller or test entry
point that justifies keeping it. Any allowance whose justification is
"used by a deleted un-namespaced variant" SHALL be removed along with
the function it guards.

#### Scenario: Reviewer audits search modules for dead-code allowances

- **WHEN** a reviewer runs `grep -n "#\\[allow(dead_code)\\]" src/core/fts.rs src/core/search.rs`
- **THEN** the command returns zero matches

#### Scenario: Surviving dead-code allowance has a reason

- **WHEN** a reviewer inspects a `#[allow(dead_code)]` (or
  `#[expect(dead_code, ...)]`) annotation that survives the change in
  `src/core/graph.rs`, `src/core/assertions.rs`, or `src/core/db.rs`
- **THEN** the annotation either carries a `reason = "..."` argument
  (for `#[expect]`) or is preceded by a `// reason: ...` comment line
  identifying the caller, test, or documented future use that
  justifies retention
