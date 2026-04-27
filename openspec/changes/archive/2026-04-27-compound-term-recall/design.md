## Context

`search_fts` in `src/core/fts.rs` passes the query string verbatim into a
`WHERE page_fts MATCH ?1` clause. FTS5 interprets space-separated tokens with
**implicit AND semantics** — all tokens must appear in a document for it to match.

This is correct and desirable when the user types exact terminology that appears
verbatim in documents. But for compound noun phrases ("neural network inference",
"machine learning model"), the document corpus typically uses overlapping but not
identical phrasing. A document about "deep learning inference engines" contains
`inference` and `learning` but not `neural network inference` as a co-occurrence,
so the AND pass misses it entirely.

`hybrid_search` in `src/core/search.rs` partially mitigates this via vector search
(which is semantic and finds paraphrases), but `quaid search` is FTS-only and has
no vector fallback. DAB benchmark §4 P04b exposed this gap at v0.9.5.

The `fts5-search-robustness` change (targeting #52/#53) sanitizes punctuation.
That fix is orthogonal: a sanitized compound query is syntactically valid but still
semantically zero-result under AND semantics. Both fixes must land to fully solve
the search quality problem.

## Goals / Non-Goals

**Goals:**
- `quaid search "neural network inference"` returns results from an ML corpus instead
  of empty results when the corpus contains related but not verbatim-matching documents.
- Precision is preserved: when AND finds results, they rank above OR-only matches.
- `quaid search --raw` retains pure expert FTS5 behaviour with no OR expansion.
- `hybrid_search` FTS arm gets the same recall improvement.

**Non-Goals:**
- Changing the `search_fts` function signature or its documented AND-semantics contract
  (it is the expert interface; its contract must not change).
- Adding synonym expansion, stemming, or query rewriting beyond simple token OR.
- Changing the vector search arm — it already handles semantic recall.
- Changing `quaid search --raw` in any way.

## Decisions

### 1. AND-first tiered approach, not always-OR

**Decision:** `search_fts_tiered` runs AND pass first; OR fallback is only triggered
when AND returns zero results.

**Rationale:** AND results are higher precision — they represent documents where all
query terms actually appear together. Always-OR degrades result quality for simple
queries where exact co-occurrence is meaningful. The tiered approach maximises recall
without sacrificing precision when precision is achievable.

### 2. OR expansion via new helper, not inside sanitize_fts_query

**Decision:** Add a separate `expand_fts_query_or(sanitized: &str) -> String` helper.
`sanitize_fts_query` is not changed.

**Rationale:** `sanitize_fts_query` has a single responsibility: make a query FTS5
parse-safe. Conflating OR expansion with sanitization would blur the architectural
layers. The helper is pure text transformation and trivially testable in isolation.

### 3. Tiered logic lives in fts.rs, not the command layer

**Decision:** `search_fts_tiered` is a new public function in `src/core/fts.rs`.
Command and search callers swap in `search_fts_tiered` for `search_fts` on the
default path.

**Rationale:** The tiered logic is a reusable search-quality behaviour — it belongs
in the library layer, not duplicated per-caller. Keeping it in `fts.rs` means future
callers (MCP tools, batch import, etc.) can opt in trivially.

### 4. No OR fallback for MCP memory_search in this change

**Decision:** MCP `memory_search` is not changed in this change. It calls `search_fts`
after sanitization (per the `fts5-search-robustness` change) and does not get the
tiered upgrade here.

**Rationale:** MCP consumers that need compound-term recall should use `memory_query`
(hybrid search), not `memory_search` (FTS-only). Upgrading `memory_search` to tiered
is low-cost and can be added later; it is out of scope to keep this change minimal.

## Risks / Trade-offs

- [OR broadens results] OR fallback may surface loosely-related results for vague queries.
  Mitigation: OR only triggers on zero-AND results; if AND finds anything, those rank
  first. For genuinely absent content, OR is no worse than the current zero-result state.
- [Token quality] For queries with very short tokens (stop words), OR fallback may over-
  retrieve. Mitigation: `sanitize_fts_query` already strips punctuation; FTS5's porter
  tokenizer handles common stop words at index time. Acceptable trade-off at current scope.
- [Behavioral change for hybrid_search] The FTS arm change slightly alters merge set for
  `quaid query`. Vector results were already providing recall; FTS OR adds a second recall
  signal. Score ordering is not affected (BM25 ranks OR hits lower than AND hits naturally).
