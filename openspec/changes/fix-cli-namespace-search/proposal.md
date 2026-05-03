---
id: fix-cli-namespace-search
title: "Fix CLI namespace filtering for quaid search"
status: proposed
type: bugfix
owner: doug
reviewers: []
created: 2026-05-03
closes: ["#145"]
---

# Fix CLI namespace filtering for quaid search

## Why

GitHub issue #145 reports that `quaid search --namespace workns "bitcoin"` returns pages
from every namespace instead of only pages from namespace `workns` plus global pages in
namespace `""`.

This violates the completed `openspec/changes/namespace-isolation/` contract. That change
explicitly says omitted CLI/MCP namespace reads default to global-only, while explicit
namespace reads include only the requested namespace and global pages. `search` is one of
the commands that namespace-isolation added `--namespace` to, so its behavior must match
`query`, `get`, and `list`.

The high-risk gap is the CLI search path:

1. `src/main.rs` parses `Search { namespace, .. }`.
2. `src/main.rs` calls `commands::search::run(..., namespace.as_deref().or(Some("")), ...)`.
3. `src/commands/search.rs::run()` validates and normalizes the namespace, then calls the
   FTS5 canonical helpers.
4. `src/core/fts.rs::search_fts_canonical_with_namespace()` and
   `search_fts_canonical_tiered_with_namespace()` must append the namespace predicate.

If any step passes `None` for an explicit namespace, the core FTS helper intentionally
performs an unfiltered search across all namespaces. That is useful for internal callers,
but it is not the CLI read default or explicit CLI namespace behavior.

## What Changes

- Preserve the CLI contract that `quaid search --namespace <id> <query>` passes
  `Some("<id>")` to the FTS5 namespace-aware helper.
- Preserve the omitted CLI contract that `quaid search <query>` passes `Some("")`, not
  `None`, so omitted namespace means global-only rather than all namespaces.
- Ensure both default sanitized search and `--raw` search use namespace-aware canonical
  helper functions:
  - sanitized path: `search_fts_canonical_tiered_with_namespace`
  - raw path: `search_fts_canonical_with_namespace`
- Ensure `search_fts_canonical_with_namespace` implements:
  - `namespace = ""`: `AND p.namespace = ?`
  - `namespace = "workns"`: `AND (p.namespace = ? OR p.namespace = '')`
  - `namespace = None`: no namespace predicate, reserved for deliberate all-namespace
    internal calls
- Add a CLI integration regression test that reproduces issue #145 with three matching
  pages: global, requested namespace, and unrelated namespace.

## Capabilities

### Modified Capabilities

- `namespace-isolation`: `quaid search --namespace <id>` filters FTS5 results to the
  requested namespace plus global pages, and omitted `--namespace` remains global-only.

## Impact

- `src/commands/search.rs`: audit and fix namespace normalization and helper selection if
  it regressed.
- `src/core/fts.rs`: audit and fix namespace predicate construction if needed.
- `tests/search_hardening.rs` or `tests/namespace_isolation.rs`: add a CLI integration
  regression test for issue #145.
- No schema changes.
- No release-channel-specific behavior: airgapped and online binaries compile the same
  CLI and FTS5 namespace filtering code. The model feature flags only affect embedding
  loading and vector search, not `quaid search`.
