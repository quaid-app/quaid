---
id: compound-term-recall
title: "search: tiered OR fallback for compound-term FTS queries"
status: implemented
type: bug
owner: fry
reviewers: [leela, professor]
created: 2026-05-01
closes: ["#67", "#69"]
---

# search: tiered OR fallback for compound-term FTS queries

## Why

`quaid search "neural network inference"` returns zero results even on an AI/ML-heavy corpus.
After `sanitize_fts_query`, the string becomes `neural network inference` — valid FTS5 syntax —
but FTS5 applies **implicit AND semantics**: all three tokens must co-occur in a single document
for any match. Documents that use synonymous phrasing ("machine learning model",
"inference engine", "embedding model") are never returned, even though they are directly
relevant.

Issues #67 and #69 (filed by doug-aillm during DAB v1.0 benchmark §4 Semantic/Hybrid, v0.9.5)
are identical reports of this behaviour. Both show paraphrase pair P04b returning zero results,
dropping the §4 benchmark score significantly.

This is **distinct** from the `fts5-search-robustness` change (closes #52/#53), which prevents
parse crashes on special characters. A query like `neural network inference` is syntactically
valid FTS5; it simply returns no results because of AND semantics, not a parse error. The two
fixes are complementary and can be shipped independently.

## What Changes

### 1. `src/core/fts.rs` — OR-expansion helper

Add `expand_fts_query_or(sanitized: &str) -> String`.

Converts a sanitized multi-word query into an FTS5 OR chain:
`neural network inference` → `neural OR network OR inference`.

Single-word queries are returned unchanged (no OR chain needed).

### 2. `src/core/fts.rs` — tiered recall search

Add `search_fts_tiered(query, wing_filter, collection_filter, conn, limit)`.

**Algorithm:**
1. Run `search_fts(sanitized_and, ...)` — exact implicit-AND pass.
2. If results are non-empty, return them immediately (AND precision wins).
3. If AND pass returns empty AND the query has ≥ 2 tokens, run
   `search_fts(or_expanded, ...)` — OR fallback for recall.
4. Return OR results (may also be empty for genuinely absent content).

The function signature mirrors `search_fts` exactly — callers can swap in without
additional wiring changes.

### 3. `src/commands/search.rs` — use tiered search on non-raw path

Replace the `search_fts` call on the default (non-`--raw`) code path with
`search_fts_tiered`. The `--raw` path continues to call `search_fts` directly,
preserving expert FTS5 semantics (AND or explicit OR is the caller's choice).

### 4. `src/core/search.rs` — use tiered search in hybrid_search FTS arm

In `hybrid_search_impl`, replace `search_fts` / `search_fts_canonical` with
`search_fts_tiered` / `search_fts_canonical_tiered`. The vector arm is unchanged;
tiered recall only affects the FTS component of the merge.

### 5. `src/core/fts.rs` — documentation update

Update doc comments on `search_fts`, `search_fts_tiered`, and `sanitize_fts_query`
to explain the three-layer architecture: (a) sanitize, (b) AND pass, (c) OR fallback.

## Capabilities

### New Capabilities
- `fts-compound-recall`: multi-token FTS queries now fall back to OR union when the
  AND pass returns no results, recovering paraphrase and synonym recall.

### Modified Capabilities
- `fts5-search`: default path uses tiered AND→OR strategy; `--raw` retains pure FTS5.
- `hybrid-search`: FTS arm gains tiered recall; vector arm is unchanged.

## Impact

- `src/core/fts.rs`: new `expand_fts_query_or` helper + `search_fts_tiered` +
  `search_fts_canonical_tiered` functions; doc comment updates.
- `src/commands/search.rs`: swap `search_fts` → `search_fts_tiered` on non-raw path.
- `src/core/search.rs`: swap FTS calls → tiered variants.
- No schema changes. No new dependencies.
