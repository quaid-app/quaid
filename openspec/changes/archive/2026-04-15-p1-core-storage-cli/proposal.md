---
id: p1-core-storage-cli
title: "Phase 1: Core Storage, CLI, Search, and MCP"
status: proposed
type: feature
phase: 1
owner: fry
reviewers: [leela, professor, nibbler]
created: 2026-04-13
depends_on: sprint-0-repo-scaffold
---

# Phase 1: Core Storage, CLI, Search, and MCP

## What

Implement the smallest complete slice that proves GigaBrain's value proposition:
- SQLite brain initialisation with full v4 schema
- Core CRUD commands: `init`, `get`, `put`, `list`, `stats`
- FTS5 full-text search (`search`)
- Candle + BGE-small-en-v1.5 local embeddings (`embed`, `query`)
- Hybrid search: SMS exact-match short-circuit + set-union merge of FTS5 + vector results
- `import` / `export` (normalized markdown round-trip)
- `compact` (WAL checkpoint)
- MCP stdio server with 5 core tools: `brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`
- Transactional ingest with SHA-256 idempotency key + `ingest_log`
- Embedded default skills (ingest + query at minimum)
- Round-trip test, corpus-reality tests, static binary verification

## Why

This is the ship gate for v0.1.0. Without it, GigaBrain is a spec. With it, a real user can:
1. Import their existing markdown brain
2. Search it semantically and by keyword
3. Export it without data loss
4. Hook any MCP-compatible agent to it via `gbrain serve`

Everything in Phase 2 (graph, assertions, progressive retrieval) builds on this. Phase 2 does not start until Phase 1 passes all ship gates.

## Workstreams

**Week 1 — Foundation:**
- `src/core/types.rs` — all structs
- `src/core/db.rs` — connection, schema init, WAL, sqlite-vec load
- `src/core/markdown.rs` — frontmatter parse, compiled_truth/timeline split, summary, render
- `src/core/palace.rs` — `derive_wing()`, `derive_room()`
- `src/commands/init.rs`, `get.rs`, `put.rs`, `list.rs`, `stats.rs`, `tags.rs`, `link.rs`
- Unit tests: markdown round-trip, frontmatter parse

**Week 2 — Search:**
- `src/core/fts.rs` — FTS5 search, BM25 scoring, wing filter
- `src/core/inference.rs` — candle init, embed, vector search
- `src/core/chunking.rs` — temporal sub-chunking (sections + timeline entries)
- `src/core/search.rs` — hybrid: SMS + palace pre-filter + FTS5 + vec0 + set-union merge
- `src/core/progressive.rs` — token-budget expansion
- `src/commands/search.rs`, `embed.rs`, `query.rs`
- Unit tests: set-union vs RRF merge, token counting

**Week 3 — Ingest + MCP:**
- `src/core/novelty.rs` — Jaccard + cosine dedup
- `src/core/migrate.rs` — `import_dir()`, `export_dir()`, `validate_roundtrip()`
- `src/commands/import.rs`, `export.rs`, `ingest.rs`, `timeline.rs`
- `src/mcp/server.rs` — 5 core MCP tools
- `src/commands/serve.rs`
- Round-trip tests

**Week 4 — Polish:**
- `src/commands/config.rs`, `version.rs`, `compact.rs`
- `--json` output on all commands
- Full unit test suite
- Embedded skills finalized

## Ship Gate

All must pass before Phase 2 begins:
1. `cargo test` passes
2. `gbrain import <corpus>` → `gbrain export` → semantic diff = 0
3. `gbrain serve` connects to Claude Code with all 5 MCP tools
4. Static binary: `ldd` confirms no dynamic dependencies on Linux musl build
5. BEIR nDCG@10 baseline established (no regression gate yet — establish baseline)
6. OCC enforcement: all write commands and MCP write tools use version-checked updates (see below)

## Concurrency: Optimistic Concurrency Control (OCC)

All write paths — `put`, `import`, `ingest`, and MCP `brain_put` — must enforce
optimistic concurrency control using the `version` column on `pages`:

1. **Read-before-write**: every write operation reads the current `version` of the
   target page (or expects `version = 0` for new pages).
2. **Compare-and-swap**: the UPDATE statement includes `WHERE version = :expected_version`.
   If zero rows are affected, the write is rejected with a conflict error.
3. **Version bump**: on success, `version` is incremented atomically in the same UPDATE.
4. **MCP contract**: `brain_put` accepts an optional `expected_version` field. If
   provided, OCC is enforced. If omitted (first write / create), the write proceeds
   unconditionally. `brain_put` responses always include the resulting `version`.
5. **Conflict error**: write conflicts return a structured error (CLI exit code 1,
   MCP JSON-RPC error with code `-32009`) containing the current `version` so the
   caller can retry with a merge.

This prevents silent lost updates in multi-agent MCP sessions where concurrent
tools may write to the same page.

## Reviewer Gates

- **Professor**: code review on `db.rs`, `search.rs`, `inference.rs`
- **Nibbler**: adversarial review on MCP server (OCC enforcement, injection)
- **Bender**: end-to-end round-trip validation sign-off
- **Scruffy**: unit test coverage on markdown parser and search merge logic
