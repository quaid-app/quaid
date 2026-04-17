## 1. Shared search hardening

- [ ] 1.1 Add a shared natural-language-safe FTS wrapper near `src/core/fts.rs` that sanitizes
      explicit search input, short-circuits empty-after-sanitize queries, and delegates actual
      execution to raw `search_fts`.
- [ ] 1.2 Update `src/commands/search.rs` to use the shared wrapper for both text and `--json`
      output so punctuation and dotted-version queries exit 0 with normal result semantics.
- [ ] 1.3 Update `src/mcp/server.rs` `brain_search` to use the same wrapper and stop surfacing
      natural-language punctuation as `invalid search query` errors.

## 2. Regression coverage

- [ ] 2.1 Add command-surface regression tests for `what is CLARITY?`, `it's a stablecoin`,
      `50% fee reduction`, and `gpt-5.4 codex model --json`.
- [ ] 2.2 Add MCP regression tests proving `brain_search` returns valid JSON for the same
      punctuation and dotted-version inputs.
- [ ] 2.3 Extend integration coverage so the explicit search path—not just `sanitize_fts_query()`
      or `hybrid_search()`—is exercised end-to-end.

## 3. Release gating

- [ ] 3.1 Update user-facing help/docs to describe `search` and `brain_search` as
      natural-language-safe interfaces for v0.9.4.
- [ ] 3.2 Run the benchmark validation commands from the v0.9.4 triage and confirm zero FTS
      crash/parse-error failures in the rerun slice.
- [ ] 3.3 Collect reviewer sign-off: Leela on shared-surface design coherence and Professor on
      command-surface regressions plus rerun evidence.
