# Quaid — Agent Instructions

Persistent memory for AI agents. SQLite + FTS5 + local vector embeddings. One static binary.

## What this is

Quaid stores your knowledge as structured pages in a single SQLite file (`memory.db`).
Pages have a `compiled_truth` section (always-current intelligence) and a `timeline` section
(append-only evidence). This is Garry Tan's compiled knowledge model in a database.

## Before you start

Read `skills/` — all workflow intelligence lives in SKILL.md files there.

- `skills/ingest/SKILL.md` — how to ingest meeting notes, articles, documents
- `skills/query/SKILL.md` — how to search and synthesize across memory
- `skills/maintain/SKILL.md` — how to detect contradictions and clean orphans
- `skills/briefing/SKILL.md` — how to generate daily briefings
- `skills/research/SKILL.md` — how to resolve knowledge gaps

## Key commands

```bash
quaid init ~/.quaid/memory.db          # create new memory store
quaid collection add work ~/vault      # attach a live-sync markdown collection
quaid ingest /path/to/note.md          # ingest a single markdown file
quaid query "who knows X?"     # hybrid semantic query
quaid search "keyword"         # FTS5 full-text search
quaid get people/alice          # read a page
quaid put people/alice < page.md  # write a page
quaid link people/alice companies/acme --relationship works_at --valid-from 2024-01
quaid graph people/alice --depth 2
quaid graph extract-entities          # opt-in backfill: writes assertions
quaid query "..." --hops 2     # override config.graph_depth for one call
quaid check --all               # contradiction detection
quaid gaps                      # knowledge gaps
quaid serve                     # start MCP server
```

## Architecture

- `src/core/` — library modules (DB, search, embeddings, parsing, entity extraction)
- `src/commands/` — one file per CLI command (includes `quaid graph extract-entities`)
- `src/mcp/server.rs` — MCP stdio server
- `src/schema.sql` — current DDL (embedded via include_str!); **v10 baseline**, no v9 → v10 migration
- `skills/*/SKILL.md` — fat markdown skill files
- `docs/graph.md` — knowledge-graph layer reference (frontmatter autowiring, wikilinks, entity-pattern extraction, graph-aware retrieval, path explanations)

## Constraints

- Single writer. No auth. No multi-tenant.
- `memory_put` uses optimistic concurrency (`expected_version`). Re-fetch before writing.
- `memory_gap` always creates gaps with `sensitivity = 'internal'`. There is no MCP tool for escalation; sensitivity is escalated through the research skill's approval workflow (`skills/research/SKILL.md`), which records an approval before any external use.
- Ingest is idempotent for exact-byte duplicates and keeps the active source in `raw_imports`.

## Testing rules

- Add new test coverage under organized files in `tests/...`; do not add new inline `#[cfg(test)] mod tests` blocks to production source files in `src/...`.
- Prefer subprocess or public-API integration tests for CLI/parser/runtime behavior. Do not add test-only production seams just to improve coverage unless the seam is part of the production design.

## Tech stack

Rust + rusqlite (bundled SQLite) + sqlite-vec + candle (BGE-small-en-v1.5) + clap + rmcp
