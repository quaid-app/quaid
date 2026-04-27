# Quaid — Agent Instructions

Persistent memory for AI agents. SQLite + FTS5 + local vector embeddings. One static binary.

## What this is

Quaid stores your knowledge as structured pages in a single SQLite file (`memory.db`).
Pages have a `compiled_truth` section (always-current intelligence) and a `timeline` section
(append-only evidence). This is Karpathy's compiled knowledge model in a database.

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
quaid import /path/to/notes/   # import markdown directory
quaid query "who knows X?"     # hybrid semantic query
quaid search "keyword"         # FTS5 full-text search
quaid get people/alice          # read a page
quaid put people/alice < page.md  # write a page
quaid link people/alice companies/acme --relationship works_at --valid-from 2024-01
quaid graph people/alice --depth 2
quaid check --all               # contradiction detection
quaid gaps                      # knowledge gaps
quaid serve                     # start MCP server
```

## Architecture

- `src/core/` — library modules (DB, search, embeddings, parsing)
- `src/commands/` — one file per CLI command
- `src/mcp/server.rs` — MCP stdio server
- `src/schema.sql` — v4 DDL (embedded via include_str!)
- `skills/*/SKILL.md` — fat markdown skill files

## Constraints

- Single writer. No auth. No multi-tenant.
- `memory_put` uses optimistic concurrency (`expected_version`). Re-fetch before writing.
- `memory_gap` always creates gaps with `sensitivity = 'internal'`. Escalation requires `memory_gap_approve`.
- Ingest is idempotent: SHA-256 of source content is the idempotency key.

## Tech stack

Rust + rusqlite (bundled SQLite) + sqlite-vec + candle (BGE-small-en-v1.5) + clap + rmcp
