---
id: fts5-search-robustness
title: "search: make quaid search safe for natural-language input"
status: proposed
type: bug
owner: fry
reviewers: [leela, professor]
created: 2026-04-19
closes: ["#52", "#53"]
---

# search: make quaid search safe for natural-language input

## Why

`quaid search` passes the raw query string directly to `search_fts`, which is the expert
FTS5 interface and propagates parse errors without sanitization. Natural-language queries
containing `?`, `'`, `%`, or dotted version strings like `gpt-5.4` cause hard SQLite FTS5
syntax errors or, with `--json`, produce non-JSON output. Beta tester doug-aillm filed
issues #52 and #53 after a DAB benchmark run on v0.9.1. The fix in PR #43 (merged to
v0.9.2) sanitizes input only in the `quaid query` / `hybrid_search` path — the raw
`quaid search` command and the MCP `memory_search` tool remain unprotected.

## What Changes

### 1. `src/commands/search.rs` — sanitize before calling search_fts

Apply `sanitize_fts_query(query)` to the input before passing it to `search_fts`.
This converts all non-alphanumeric characters to spaces and quotes bare FTS5 boolean
keywords, making `quaid search "what is CLARITY?"` and `quaid search "gpt-5.4"` safe.

Users who want raw FTS5 syntax (quoted phrases, boolean operators, wildcard `*`) must use
the documented `--raw` flag (new, see below) to opt out of sanitization.

### 2. `src/commands/search.rs` — add `--raw` flag to preserve expert FTS5 access

Add a `--raw` boolean flag. When `--raw` is set, the query is passed unsanitized directly
to `search_fts`. This preserves full FTS5 expert access for power users while making the
default path safe for natural language.

### 3. `src/mcp/server.rs` — sanitize `memory_search` tool input

In the `memory_search` MCP tool handler, apply `sanitize_fts_query` to the incoming query
parameter before passing to `search_fts`. MCP consumers are typically agents sending
natural-language queries, not FTS5 experts. No `--raw` equivalent for MCP in this change.

### 4. `src/commands/search.rs` — graceful `--json` error output

When `search_fts` returns an error (e.g. via `--raw` with malformed FTS5), and `--json`
is active, output `{"error": "<message>"}` to stdout instead of propagating the error to
stderr with no JSON output. This makes `quaid search --json` safe for automated consumers.

### 5. `src/core/fts.rs` — documentation update

Update the `search_fts` doc comment to clarify that `src/commands/search.rs` now sanitizes
by default, and that the `--raw` flag bypasses sanitization for expert callers.

## Capabilities

### New Capabilities
- `search-natural-language-safety`: `quaid search` handles natural-language queries safely
  by default; `--raw` flag available for expert FTS5 syntax access.

### Modified Capabilities
- `fts5-search`: Default interface sanitizes input; raw FTS5 access moved behind `--raw` flag.

## Impact

- `src/commands/search.rs`: apply `sanitize_fts_query` by default; add `--raw` flag;
  emit `{"error": ...}` JSON on error when `--json` is active.
- `src/mcp/server.rs`: apply `sanitize_fts_query` to `memory_search` query parameter.
- `src/core/fts.rs`: doc comment update only.
- No schema changes, no new dependencies.
