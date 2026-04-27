## Context

`src/commands/search.rs::run()` calls `search_fts(query, ...)` directly. `search_fts` in
`src/core/fts.rs` is the raw FTS5 interface: it passes the query string verbatim into a
`WHERE page_fts MATCH ?1` clause and propagates any SQLite parse error as `Err`. FTS5
treats `?`, `*`, `+`, `(`, `)`, `"` as syntax operators and `'` as a SQL string delimiter.
Queries containing these characters produce SQLite FTS5 syntax errors.

`sanitize_fts_query` already exists in `src/core/fts.rs` and is applied by `hybrid_search`
in `src/core/search.rs` before calling `search_fts`. The `quaid query` command (which uses
`hybrid_search`) is therefore safe. `quaid search` and the MCP `memory_search` tool are not.

When `quaid search --json` is active and `search_fts` returns an error, the `?` operator
in `run()` propagates the error to the top-level error handler, which prints to stderr.
Stdout has no JSON output, so downstream JSON consumers receive malformed/empty output.

## Goals / Non-Goals

**Goals:**
- `quaid search` handles natural-language queries without crashing on punctuation or
  special characters by default.
- `quaid search --json` always produces valid JSON, even on error paths.
- The MCP `memory_search` tool handles natural-language agent queries safely.
- Expert FTS5 users retain full syntax access via an explicit `--raw` flag.

**Non-Goals:**
- Adding `--raw` to MCP `memory_search` (agents are natural-language callers by design).
- Changing the `search_fts` function itself — it stays as the raw expert interface.
- Adding Unicode or language-specific tokenization improvements.
- Changing `quaid query` — it already sanitizes.

## Decisions

### 1. Sanitize in the command layer, not inside search_fts

**Decision:** Apply `sanitize_fts_query` in `src/commands/search.rs::run()` and in the
MCP `memory_search` handler, not inside `search_fts`.

**Rationale:** `search_fts` is intentionally the raw expert interface — its documented
contract is "propagates FTS5 syntax errors." Changing its contract would break any caller
relying on expert FTS5 syntax (e.g., future CLI commands or test helpers). The sanitization
belongs at the consumer layer.

### 2. --raw flag to opt out of sanitization

**Decision:** Add a `--raw` boolean flag to `quaid search`. When set, skip sanitization
and pass the query verbatim to `search_fts`. Default is sanitized (natural-language safe).

**Rationale:** Some users want to write expert FTS5 queries like `"exact phrase" AND term*`.
A `--raw` flag preserves that capability explicitly rather than removing it silently.
The flag is named `--raw` (not `--expert` or `--no-sanitize`) to be maximally explicit.

### 3. JSON error envelope instead of propagated error

**Decision:** When `--json` is active and an error occurs (only reachable via `--raw`
with malformed FTS5), output `{"error": "<message>"}` to stdout instead of propagating.

**Rationale:** JSON consumers (scripts, agents) cannot parse stderr. A `{"error": ...}`
envelope is the standard API error pattern and makes the `--json` contract reliable.
Without `--raw`, this path is never triggered — sanitized queries don't produce FTS5 errors.

### 4. No --raw equivalent for MCP memory_search

**Decision:** MCP `memory_search` always sanitizes. No raw mode for MCP.

**Rationale:** MCP consumers are agents sending natural-language queries. There is no
scenario where an MCP client would need raw FTS5 expert syntax. Keeping MCP always-safe
simplifies the tool contract and avoids exposing error paths to agent consumers.

## Risks / Trade-offs

- [Silent query alteration] Sanitization drops characters silently. A user who types
  `gpt-5.4` gets `gpt 5 4` in FTS5 → Acceptable: the tokens still find relevant content.
  The `--raw` opt-out covers edge cases where exact syntax matters.
- [--raw is easily forgotten] Power users may not know about `--raw` → Mitigation: document
  in `quaid search --help` that default mode sanitizes, `--raw` for FTS5 expert syntax.
