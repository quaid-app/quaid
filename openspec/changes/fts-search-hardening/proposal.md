---
id: fts-search-hardening
title: "search: harden CLI and MCP FTS input for natural-language queries"
status: proposed
type: bug
owner: professor
reviewers: [leela, fry]
created: 2026-04-17
closes: ["#52", "#53"]
references: ["#56"]
---

## Why

Doug's v0.9.1 benchmark surfaced a release-blocking usability bug in the explicit FTS path:
`gbrain search` and MCP `brain_search` still send raw user text into SQLite FTS5, so ordinary
queries like `what is CLARITY?`, `it's a stablecoin`, `50% fee reduction`, and
`gpt-5.4 codex model` can crash or return invalid output. The repo already hardens the hybrid
path, which means the gap is not core search capability but inconsistent interface behavior on
the explicit CLI + MCP search surfaces.

## What Changes

- Add a shared natural-language-safe FTS search path for the explicit CLI command and MCP
  `brain_search` tool so punctuation and dotted-version tokens no longer surface FTS5 parser
  failures to end users.
- Preserve the raw `search_fts` primitive for internal and low-level callers that intentionally
  need exact FTS5 semantics; move the hardening to the user-facing interface boundary instead of
  mutating every search call site.
- Add regression coverage at the command and MCP layers for the benchmark-reported failure cases,
  including `?`, `'`, `%`, and dotted version-number tokens.
- Add a v0.9.4 validation gate that reruns Doug's benchmark FTS slice and requires zero
  crash/parse-error failures before the lane is considered complete.

## Capabilities

### New Capabilities
- `fts-search-input-hardening`: explicit CLI and MCP FTS search surfaces accept natural-language
  input safely and consistently.
- `search-benchmark-regressions`: benchmark-visible search failures are locked down with
  command-surface regressions and rerun validation.

### Modified Capabilities
- None.

## Impact

- `src/core/fts.rs`: shared interface-level hardening helper(s) layered above raw `search_fts`
- `src/commands/search.rs`: explicit CLI `search` path
- `src/mcp/server.rs`: MCP `brain_search` handler and related tests
- `tests\` and/or command-surface tests: new regression coverage for benchmark inputs
- User-facing contract for explicit search becomes "natural-language safe" instead of "raw FTS5
  syntax may leak through"
