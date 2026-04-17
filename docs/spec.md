---
title: GigaBrain - Personal Knowledge Brain
type: project
created: 2026-04-06
updated: 2026-04-13
status: spec-complete-v4
tags:
  - open-source
  - knowledge-base
  - sqlite
  - rag
  - rust
  - mcp
  - thin-harness
  - fat-skills
  - candle
sources:
  - https://gist.github.com/garrytan/49c88e83cf8d7ae95e087426368809cb
  - Author architecture notes (2026-04-06)
  - Architecture Review (2026-04-08)
  - Rowboat knowledge graph insight (2026-04-09)
---

# GigaBrain - Personal Knowledge Brain

> Open-source personal knowledge brain. SQLite + FTS5 + vector embeddings in one file. Thin CLI harness, fat skill files. MCP-ready from day one. Runs anywhere. No API keys, no internet, no Docker. Truly static single binary.

Inspired by Garry Tan's GBrain work, with this spec adapting similar goals to a local-first Rust + SQLite architecture intended for portable, offline use.

- **Status:** Spec complete v4 — ready to build core (see [Phased Delivery](#phased-delivery))
- **Repo (planned):** GitHub.com/[owner]/gbrain
- **License (planned):** MIT
- **Origin:** Inspired by Garry Tan's GBrain spec (2026-04-05), then extended with architecture improvements (2026-04-06) and memory research integration (2026-04-08)
- **v1 differentiator over Garry's spec:** Local embeddings, Rust binary instead of TypeScript/Bun, true zero-dependency single binary
- **v2 additions (Apr 2026 research):** Set-union hybrid search, palace-style hierarchical filtering, progressive retrieval, selective ingestion, temporal knowledge graph, contradiction detection, four-tier memory consolidation. Techniques sourced from MemPalace (96.6% R@5 LongMemEval), OMNIMEM (+411% F1), agentmemory (92% token reduction).
- **v3 additions (Architecture Review):** Exact-Match Short-Circuit (SMS) search, Temporal Sub-chunking for timelines, Assertions table for heuristic contradiction detection, strict Optimistic Concurrency on MCP writes, true zero-dependency static linking via `candle` (replacing `fastembed`/ONNX).
- **v4 additions (Community Research + Garry v0.8.0):** Knowledge gap detection (`knowledge_gaps` table + `brain_gap` MCP tool), graph neighborhood traversal (`brain_graph`), `original` page type for user's own thinking, standardised source attribution format with authority hierarchy, filing disambiguation rules, richer person templates, new skills (upgrade, alerts, research), and PGLite convergence validation. Informed by community prototypes, public discussion, and Garry Tan's v0.8.0 GBrain skillpack analysis.

---

## Table of Contents

1. [The Problem](#the-problem) *(includes Non-Goals)*
2. [The Solution](#the-solution)
3. [Architecture Overview](#architecture-overview)
4. [Technology Stack](#technology-stack) *(v3: candle replaces fastembed)*
5. [Database Schema](#database-schema) *(v4: + knowledge_gaps table, original page type)*
6. [CLI Reference](#cli-reference) *(v4: + graph, gaps commands)*
7. [MCP Server](#mcp-server) *(v4: + brain_graph, brain_gap, brain_gaps)*
8. [Hybrid Search](#hybrid-search) *(v3: SMS short-circuit + set-union + palace filtering)*
9. [Progressive Retrieval](#progressive-retrieval) *(v2: token-budget-gated expansion)*
10. [Ingest Pipeline](#ingest-pipeline) *(v3: assertions + sub-chunking + idempotency)*
11. [Migration Plan](#migration-plan) *(v4: + original type mapping)*
12. [Export and Round-trip](#export-and-round-trip)
13. [Skills (Fat Markdown)](#skills-fat-markdown) *(v4: + alerts, research, upgrade skills; source attribution; filing disambiguation)*
14. [Repository Structure](#repository-structure) *(v4: + graph.rs, gaps.rs, 3 new skills)*
15. [Build and Release](#build-and-release)
16. [Phased Delivery](#phased-delivery) *(v4: knowledge_gaps in P1, graph in P2, gaps/alerts/research/upgrade in P3)*
17. [Implementation Roadmap](#implementation-roadmap) *(v4: + graph, gaps, new skills)*
18. [Benchmarks and Release Gates](#benchmarks-and-release-gates) *(corpus-reality + LongMemEval, LoCoMo, BEIR, Ragas)*
19. [Design Decisions](#design-decisions) *(v4: + SQLite vs PGLite, links-as-graph-layer)*
20. [Schema Versioning and DB Migration](#schema-versioning-and-db-migration)
21. [Security and Data Sensitivity](#security-and-data-sensitivity)
22. [Error Handling and Graceful Degradation](#error-handling-and-graceful-degradation)
23. [Comparison Table](#comparison-table) *(v4: updated for Garry v0.8.0 + PGLite, + graph/gap rows)*
24. [Open Questions](#open-questions) *(v2: new questions added)*
25. [Spec History](#spec-history)

---

## The Problem

Git doesn't scale past ~5,000 markdown files. At 7,471 files and 2.3GB, a wiki-brain directory becomes slow to clone, painful to search, and unusable for structured queries. Full-text search requires grep. Semantic search requires an external vector database. Cross-references are just markdown links with no queryable graph.

The compiled-truth + timeline architecture (Karpathy-style: always-current intelligence above the line, append-only evidence below the line) is the right knowledge model - it just needs a real database underneath.

Additionally: every existing knowledge-base tool (Obsidian, Notion, RAG frameworks) either requires a GUI, locks data in a SaaS platform, or needs an internet connection and API keys to do anything useful. An agent-first world needs a knowledge layer that:

- Lives in a single logical database (one `.db` file; WAL sidecars during operation, compactable to single file for transport)
- Does full-text + semantic search natively
- Exposes an MCP server for any AI client
- Works on a plane, in an air-gapped environment, with no ongoing API costs
- Is fast, small, and has zero runtime dependencies

### Non-Goals (v1)

- **Not a collaborative platform** — single-user, single-writer. No auth, no RBAC, no multi-tenant.
- **Not a sync product** — no real-time replication, no CRDTs, no cloud sync. `rsync`/`scp` is the transport.
- **Not a full graph database** — typed links with temporal validity, not arbitrary traversals or Cypher queries.
- **Not a general note-taking app** — structured knowledge pages, not freeform notes. Use Obsidian for that.
- **Not a document warehouse** — pages are compiled intelligence, not raw file storage. Raw data goes in `raw_data` table.
- **Not a semantic contradiction oracle** — heuristic detection via assertions, not LLM-powered reasoning. The binary is dumb.
- **Not multimodal** — text only. Images, audio, video are not indexed or embedded.

---

## The Solution

A single Rust CLI distributed in two BGE-small channels — airgapped embedded (~180MB) or online (~90MB) — wrapping:

- **SQLite** with WAL mode - single logical database (`brain.db` + WAL sidecars while live; `gbrain compact` checkpoints to true single file for transport/backup)
- **FTS5** - full-text search, built into SQLite
- **sqlite-vec** - vector similarity search as a SQLite extension, statically linked
- **candle + BGE-small-en-v1.5** - pure-Rust ML framework running a local embedding model, no ONNX runtime dependencies
- **MCP stdio server** - any MCP-compliant client can search, read, write, and ingest
- **Fat skills** - intelligence lives in markdown SKILL.md files, not in code

One `cargo build --release --target x86_64-unknown-linux-musl`. One truly static binary. Drop it anywhere and run it.

---

## Architecture Overview

```
╔══════════════════════════════════════════════════════════════╗
║                        CONSUMERS                             ║
╠══════════════════════════════════════════════════════════════╣
║                                                              ║
║   Claude Code       OpenClaw / Doug     Any MCP Client      ║
║   (via MCP)         (via MCP/CLI)       (via MCP)           ║
║        │                  │                   │              ║
║        └──────────┬────────────────────┘       │             ║
║                   │                            │             ║
║   ┌───────────────▼──────────┐  ┌─────────────▼──────────┐ ║
║   │      MCP Server          │  │         CLI             │ ║
║   │   (stdio transport)      │  │    bin/gbrain           │ ║
║   │    gbrain serve          │  │  single Rust binary     │ ║
║   └───────────────┬──────────┘  └─────────────┬──────────┘ ║
║                   │                            │             ║
║                   └──────────────┬─────────────┘            ║
║                                  │                           ║
║              ┌───────────────────▼──────────────┐           ║
║              │            gbrain-core            │           ║
║              │              (Rust)               │           ║
║              │                                   │           ║
║              │  ┌──────────────────────────────┐ │           ║
║              │  │  db.rs        (rusqlite)     │ │           ║
║              │  │  fts.rs       (FTS5 queries) │ │           ║
║              │  │  inference.rs (candle ML)    │ │           ║
║              │  │  search.rs    (SMS + union)  │ │           ║
║              │  │  progressive.rs (retrieval)  │ │           ║
║              │  │  palace.rs    (wing/room)    │ │           ║
║              │  │  novelty.rs   (dedup)        │ │           ║
║              │  │  assertions.rs(heuristics)   │ │           ║
║              │  │  chunking.rs  (temporal)     │ │           ║
║              │  │  markdown.rs  (parse/render) │ │           ║
║              │  │  links.rs     (temporal KG)  │ │           ║
║              │  │  migrate.rs   (import/export)│ │           ║
║              │  └──────────────────────────────┘ │           ║
║              └───────────────────┬────────────────┘          ║
║                                  │                           ║
║              ┌───────────────────▼──────────────┐           ║
║              │           SQLite DB               │           ║
║              │           brain.db                │           ║
║              │                                   │           ║
║              │  ┌──────────────────────────────┐ │           ║
║              │  │  pages                        │ │           ║
║              │  │  page_fts      (FTS5 vtable)  │ │           ║
║              │  │  page_embeddings (vec0 vtable)│ │           ║
║              │  │  links                        │ │           ║
║              │  │  assertions                   │ │           ║
║              │  │  tags                         │ │           ║
║              │  │  raw_data                     │ │           ║
║              │  │  timeline_entries             │ │           ║
║              │  │  ingest_log                   │ │           ║
║              │  │  config                       │ │           ║
║              │  └──────────────────────────────┘ │           ║
║              └───────────────────────────────────┘          ║
║                                                              ║
╠══════════════════════════════════════════════════════════════╣
║                    SKILLS (Fat Markdown)                     ║
╠══════════════════════════════════════════════════════════════╣
║                                                              ║
║  skills/ingest/SKILL.md   — meeting/doc/article ingestion   ║
║  skills/query/SKILL.md    — search + synthesis              ║
║  skills/maintain/SKILL.md — lint, contradictions, orphans   ║
║  skills/enrich/SKILL.md   — external API enrichment         ║
║  skills/briefing/SKILL.md — daily briefing compilation      ║
║  skills/alerts/SKILL.md   — interrupt-driven notifications  ║
║  skills/research/SKILL.md — knowledge gap resolution        ║
║  skills/upgrade/SKILL.md  — agent-guided binary upgrades    ║
║                                                              ║
╚══════════════════════════════════════════════════════════════╝
```

### Core Philosophy

**Thin harness, fat skills.** The binary is plumbing. The intelligence lives in SKILL.md files. Claude Code, OpenClaw, or any agent reads SKILL.md at session start and knows every workflow, heuristic, and edge case without that logic being compiled into the binary. Default skills are embedded in the binary via `include_str!()` and extracted to `~/.gbrain/skills/` on first run. External skill files in the working directory override embedded defaults. `gbrain skills doctor` shows active resolution order and content hashes.

**Above the line / Below the line.** Every knowledge page has two zones:
- **compiled_truth** - Always current. Rewritten when new info arrives. The intelligence assessment. The "what we know now."
- **timeline** - Append-only. Never rewritten. The evidence base. The "what happened and when."

The horizontal rule (`---`) is the boundary. Reconstructed on export.

**Single logical database, total ownership.** `brain.db` is the database. During operation, SQLite WAL mode creates `-wal` and `-shm` sidecars for write performance. Run `gbrain compact` to checkpoint back to a true single file for transport. The practical artifact is: binary + DB + skill pack (embedded defaults, optional overrides). No connection strings. No Docker. No managed database. No API keys required at runtime.

---

## Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Language | **Rust** | Single binary via `cargo build --release`. No runtime. No GC pauses. Memory safe. Cross-compiles cleanly. |
| Database | **rusqlite** with `bundled` feature | SQLite compiled into the binary. Zero system dependency. `bundled` cargo feature = self-contained. |
| Full-text search | **FTS5** | Built into SQLite. Porter stemmer + unicode61 tokenizer. Handles 100K+ documents trivially. |
| Vector search | **sqlite-vec** (statically linked) | Alex Garcia's sqlite-vec extension. Stores float32 arrays as BLOBs. Native cosine similarity. Same DB, same query. v0.1+ stable. Statically linked via rusqlite. |
| Embeddings | **candle + BGE-small-en-v1.5** | HuggingFace's pure-Rust ML framework. Unlike `fastembed` (ONNX runtime), `candle` allows true `musl` static compilation. Weights embedded via `include_bytes!()` for zero-network binary. |
| CLI | **clap** | Industry standard Rust CLI framework. Auto-generates help text. |
| MCP server | **rmcp** | Rust MCP crate. Stdio transport. |
| Markdown | **pulldown-cmark** + **gray-matter** port | Fast CommonMark parser. Frontmatter parsing via custom YAML header extraction. |
| JSON/YAML | **serde_json** / **serde_yaml** | Standard serialization. |

### Why Rust over TypeScript/Bun (Garry's original stack)

| | Garry's GBrain (TypeScript/Bun) | This spec (Rust) |
|---|---|---|
| Binary size | ~10MB (Bun compiled) | ~90MB (includes model weights) |
| Embeddings | text-embedding-3-small (OpenAI API, costs money, needs internet) | BGE-small-en-v1.5 via candle (local, free, fast, pure Rust) |
| Internet required | Yes (for embeddings) | No |
| API keys required | Yes (OPENAI_API_KEY) | No |
| Cross-compile | Bun's compile works but CGO complications | `cargo cross` or GitHub Actions matrix = trivial |
| Memory | Node/Bun overhead | Minimal, no GC |
| sqlite-vec linking | Native addon complications with Bun | `rusqlite` bundled feature handles it cleanly |
| Air-gapped use | No | Yes |

**The key differentiator:** runs on a plane, in a datacenter without egress, on a client machine with no API keys configured. "Runs on client" is the real value.

---

## Database Schema

```sql
-- brain.db schema
-- GigaBrain v4

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

-- Load sqlite-vec extension (statically linked in Rust binary)
-- SELECT load_extension('./vec0');  -- handled at db init in Rust

-- ============================================================
-- pages: the core content table
-- ============================================================
CREATE TABLE IF NOT EXISTS pages (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    slug            TEXT    NOT NULL UNIQUE,    -- e.g. "people/pedro-franceschi"
    type            TEXT    NOT NULL,            -- person, company, deal, project, concept, original, source, media, decision, commitment, action_item, area, resource, archive, journal
    title           TEXT    NOT NULL,
    summary         TEXT    NOT NULL DEFAULT '', -- executive summary (blockquote at top of compiled_truth)
    compiled_truth  TEXT    NOT NULL DEFAULT '', -- markdown, above the line
    timeline        TEXT    NOT NULL DEFAULT '', -- markdown, below the line
    frontmatter     TEXT    NOT NULL DEFAULT '{}',-- JSON blob (original YAML converted)
    wing            TEXT    NOT NULL DEFAULT '', -- palace hierarchy: entity grouping (auto-derived from slug, override via frontmatter)
    room            TEXT    NOT NULL DEFAULT '', -- palace hierarchy: topic within wing (derived from section headers or frontmatter)
    version         INTEGER NOT NULL DEFAULT 1,    -- optimistic concurrency: incremented on every write
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),         -- bumped on ANY page-scoped mutation
    truth_updated_at TEXT   NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),         -- bumped ONLY when compiled_truth changes
    timeline_updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_pages_type ON pages(type);
CREATE INDEX IF NOT EXISTS idx_pages_slug ON pages(slug);
CREATE INDEX IF NOT EXISTS idx_pages_updated ON pages(updated_at);
CREATE INDEX IF NOT EXISTS idx_pages_wing ON pages(wing);
CREATE INDEX IF NOT EXISTS idx_pages_wing_room ON pages(wing, room);

-- ============================================================
-- page_fts: full-text search over compiled_truth + timeline
-- ============================================================
CREATE VIRTUAL TABLE IF NOT EXISTS page_fts USING fts5(
    title,
    slug,
    compiled_truth,
    timeline,
    content='pages',
    content_rowid='id',
    tokenize='porter unicode61'
);

-- Triggers to keep FTS in sync
CREATE TRIGGER IF NOT EXISTS pages_ai AFTER INSERT ON pages BEGIN
    INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
    VALUES (new.id, new.title, new.slug, new.compiled_truth, new.timeline);
END;

CREATE TRIGGER IF NOT EXISTS pages_ad AFTER DELETE ON pages BEGIN
    INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
    VALUES ('delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline);
END;

CREATE TRIGGER IF NOT EXISTS pages_au AFTER UPDATE ON pages BEGIN
    INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
    VALUES ('delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline);
    INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
    VALUES (new.id, new.title, new.slug, new.compiled_truth, new.timeline);
END;

-- ============================================================
-- page_embeddings: vector embeddings via sqlite-vec
-- vec0 virtual table for native cosine similarity
-- ============================================================
-- ── Embedding model registry (single source of truth for active model) ──
-- Each model gets its own vec0 table (dimension is baked into the virtual table).
-- The embedding_models table is the ONLY authoritative selector for the active model.
-- The config keys 'embedding_model' and 'embedding_dimensions' are derived from this
-- table at init and are read-only aliases — never write them directly.
CREATE TABLE IF NOT EXISTS embedding_models (
    name       TEXT PRIMARY KEY,              -- e.g. 'bge-small-en-v1.5'
    dimensions INTEGER NOT NULL,              -- e.g. 384
    vec_table  TEXT NOT NULL UNIQUE,          -- e.g. 'page_embeddings_vec_384'
    active     INTEGER NOT NULL DEFAULT 0,    -- 1 = currently used for writes
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Enforce exactly one active model at all times.
-- Partial unique index: only rows where active=1 participate in the constraint.
-- Zero active rows → init/migration must seed one. Multiple active rows → rejected.
CREATE UNIQUE INDEX IF NOT EXISTS idx_embedding_models_one_active
    ON embedding_models(active) WHERE active = 1;

-- Seed the default model at init (MUST happen before any embedding operation):
--   INSERT INTO embedding_models (name, dimensions, vec_table, active)
--   VALUES ('bge-small-en-v1.5', 384, 'page_embeddings_vec_384', 1);
-- On model upgrade: INSERT new model with active=0, re-embed all pages under it,
-- then in a single transaction: UPDATE old model SET active=0, UPDATE new model SET active=1.
-- The unique index guarantees at most one active=1 row at any point.
-- Old table is kept until explicitly dropped, so rollback is safe.

-- Default model — vec table created dynamically at init based on registered dimensions.
-- Example for BGE-small (384-dim):
--   CREATE VIRTUAL TABLE IF NOT EXISTS page_embeddings_vec_384 USING vec0(
--       embedding float[384]
--   );
-- On model upgrade (e.g. to 768-dim), a new vec table is created and a re-embed
-- migration populates it before flipping the active flag. Old table is kept until
-- explicitly dropped, so rollback is safe.

-- Metadata for each embedding chunk — model-scoped to avoid rowid collisions.
-- Each model's vec table has its own rowid space. page_embeddings stores the
-- vec_rowid for the specific model's vec table, NOT a shared autoincrement.
CREATE TABLE IF NOT EXISTS page_embeddings (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,  -- internal metadata ID (NOT vec rowid)
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    model           TEXT    NOT NULL DEFAULT 'bge-small-en-v1.5' REFERENCES embedding_models(name),
    vec_rowid       INTEGER NOT NULL,       -- rowid in the model's vec table (model-scoped)
    chunk_type      TEXT    NOT NULL,       -- 'truth_section' | 'timeline_entry'
    chunk_index     INTEGER NOT NULL,       -- 0-based index within page
    chunk_text      TEXT    NOT NULL,       -- the text that was embedded
    content_hash    TEXT    NOT NULL,       -- SHA-256 of chunk_text (skip re-embed if unchanged)
    token_count     INTEGER NOT NULL,       -- approximate token count (whitespace-split)
    heading_path    TEXT    NOT NULL DEFAULT '',  -- e.g. "## State" or "## Timeline > 2024-03-01"
    last_embedded_at TEXT   NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(model, vec_rowid)               -- one metadata row per vector per model
);
-- Join: page_embeddings.vec_rowid = <model_vec_table>.rowid
--        WHERE page_embeddings.model = <active_model>
-- On model migration: new rows created for new model, old model rows kept for rollback.
-- Cutover: flip embedding_models.active, queries switch to new model's rows.
-- Rollback: flip back, old rows + old vec table still intact.

CREATE INDEX IF NOT EXISTS idx_embeddings_page ON page_embeddings(page_id);
CREATE INDEX IF NOT EXISTS idx_embeddings_model ON page_embeddings(model);
CREATE INDEX IF NOT EXISTS idx_embeddings_lookup ON page_embeddings(model, page_id, chunk_index);

-- ============================================================
-- links: cross-references between pages
-- ============================================================
-- Surrogate ID is the stable target for link-close operations.
-- No UNIQUE on (from, to, relationship, valid_from) — multiple intervals with
-- unknown start dates are allowed. Dedup and non-overlap enforced in app logic.
-- brain_link_close targets by link ID, not by date columns.
CREATE TABLE IF NOT EXISTS links (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    from_page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    to_page_id   INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    relationship TEXT    NOT NULL DEFAULT 'related',
    context      TEXT    NOT NULL DEFAULT '',
    valid_from   TEXT    DEFAULT NULL,
    valid_until  TEXT    DEFAULT NULL,
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    -- Temporal ordering: valid_until must be after valid_from when both are known
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
);

CREATE INDEX IF NOT EXISTS idx_links_from ON links(from_page_id);
CREATE INDEX IF NOT EXISTS idx_links_to   ON links(to_page_id);
CREATE INDEX IF NOT EXISTS idx_links_current ON links(valid_until);  -- fast filter for current-only queries

-- ============================================================
-- assertions: heuristic contradiction detection
-- Populated by agents during Tier 2 ingest. Enables pure-SQL
-- consistency checks without burning LLM tokens.
-- ============================================================
-- Surrogate ID is the stable target for supersession.
-- No UNIQUE on (page_id, subject, predicate, object, valid_from) — multiple intervals
-- with unknown start dates are allowed. Tier 2 rewrites supersede old assertions by
-- setting valid_until on the prior row AND pointing supersedes_id to it.
-- Contradiction detection: SELECT WHERE valid_until IS NULL (current beliefs only).
-- Dedup enforced in application logic during ingest.
CREATE TABLE IF NOT EXISTS assertions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    subject         TEXT    NOT NULL,
    predicate       TEXT    NOT NULL,
    object          TEXT    NOT NULL,
    valid_from      TEXT    DEFAULT NULL,
    valid_until     TEXT    DEFAULT NULL,
    supersedes_id   INTEGER DEFAULT NULL REFERENCES assertions(id),
    confidence      REAL    DEFAULT 1.0,
    asserted_by     TEXT    NOT NULL DEFAULT 'agent',
    source_ref      TEXT    NOT NULL DEFAULT '',
    evidence_text   TEXT    NOT NULL DEFAULT '',
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from),
    CHECK (asserted_by IN ('agent', 'manual', 'import', 'enrichment'))
);

CREATE INDEX IF NOT EXISTS idx_assertions_subj ON assertions(subject);
CREATE INDEX IF NOT EXISTS idx_assertions_pred ON assertions(predicate);

-- ============================================================
-- tags
-- ============================================================
CREATE TABLE IF NOT EXISTS tags (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    tag     TEXT    NOT NULL,
    UNIQUE(page_id, tag)
);

CREATE INDEX IF NOT EXISTS idx_tags_tag     ON tags(tag);
CREATE INDEX IF NOT EXISTS idx_tags_page_id ON tags(page_id);

-- ============================================================
-- raw_data: sidecar data (replaces .raw/ JSON files)
-- ============================================================
CREATE TABLE IF NOT EXISTS raw_data (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    source     TEXT    NOT NULL,    -- "crustdata", "happenstance", "exa", "partiful"
    data       TEXT    NOT NULL,    -- full JSON response
    fetched_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(page_id, source)         -- one row per source per page, overwrite on re-enrich
);

CREATE INDEX IF NOT EXISTS idx_raw_data_page ON raw_data(page_id);

-- ============================================================
-- timeline_entries: structured timeline (supplements markdown)
-- ============================================================
CREATE TABLE IF NOT EXISTS timeline_entries (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    date    TEXT    NOT NULL,               -- ISO 8601: YYYY-MM-DD
    source  TEXT    NOT NULL DEFAULT '',    -- "meeting", "email", "manual", etc.
    summary      TEXT    NOT NULL,               -- one-line summary
    summary_hash TEXT    NOT NULL DEFAULT '',    -- SHA-256 of summary for dedupe
    detail       TEXT    NOT NULL DEFAULT '',    -- full markdown detail
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(page_id, date, summary_hash)          -- prevent replay duplicates
);

CREATE INDEX IF NOT EXISTS idx_timeline_page ON timeline_entries(page_id);
CREATE INDEX IF NOT EXISTS idx_timeline_date ON timeline_entries(date);

-- ============================================================
-- raw_imports: original file bytes for byte-exact round-trip
-- ============================================================
CREATE TABLE IF NOT EXISTS raw_imports (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    import_id  TEXT    NOT NULL,            -- identifies the import batch
    is_active  INTEGER NOT NULL DEFAULT 1,  -- 1 = current snapshot for this page, 0 = historical
    raw_bytes  BLOB   NOT NULL,            -- original file content, byte-for-byte
    file_path  TEXT   NOT NULL,            -- relative path within the import source
    created_at TEXT   NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(page_id, import_id)
);
-- On re-import: set is_active=0 on prior rows for the same page_id, then insert new row.
-- Raw export uses the active snapshot by default, or a specific import_id if provided.

CREATE INDEX IF NOT EXISTS idx_raw_imports_page ON raw_imports(page_id);
CREATE INDEX IF NOT EXISTS idx_raw_imports_active ON raw_imports(page_id, is_active) WHERE is_active = 1;

-- Import manifest: tracks each import batch for rollback/audit
CREATE TABLE IF NOT EXISTS import_manifest (
    import_id   TEXT PRIMARY KEY,           -- UUID or timestamp-based batch ID
    source_dir  TEXT NOT NULL,              -- original import path
    page_count  INTEGER NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- ============================================================
-- ingest_log: audit trail for all ingest operations
-- ============================================================
CREATE TABLE IF NOT EXISTS ingest_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    ingest_key    TEXT NOT NULL UNIQUE,              -- SHA-256 of source content; idempotency key
    source_type   TEXT NOT NULL,                     -- "meeting", "article", "doc", "conversation", "import"
    source_ref    TEXT NOT NULL,                     -- meeting ID, URL, file path, etc.
    pages_updated TEXT NOT NULL DEFAULT '[]',        -- JSON array of page slugs
    summary       TEXT NOT NULL DEFAULT '',
    completed_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
-- Fully transactional idempotency protocol:
--   1. BEGIN TRANSACTION
--   2. SELECT ingest_key WHERE ingest_key = ? → if row exists, ROLLBACK and skip (already done)
--   3. Run all mutations (page puts, links, timeline, assertions, embeddings)
--   4. INSERT ingest_log row (inside the same transaction)
--   5. COMMIT
-- The ingest_log row is written atomically with all mutations. If the process
-- crashes before COMMIT, the entire transaction rolls back — including the
-- ingest_log row — so the key never appears and the retry starts clean.
-- No stale-row reclamation needed. No two-phase protocol. Just SQLite ACID.

-- ============================================================
-- config: brain-level settings
-- ============================================================
CREATE TABLE IF NOT EXISTS config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO config (key, value) VALUES
    ('version',              '4'),
    -- embedding_model and embedding_dimensions are derived from embedding_models
    -- at startup and kept in sync automatically. Do not write them directly.
    ('embedding_model',      'bge-small-en-v1.5'),   -- read-only alias, derived from embedding_models WHERE active=1
    ('embedding_dimensions', '384'),                  -- read-only alias, derived from embedding_models WHERE active=1
    ('chunk_strategy',       'section'),            -- "page", "section", or "paragraph"
    ('search_merge_strategy','set-union'),           -- "set-union" (default) or "rrf" (fallback)
    ('default_token_budget', '4000');                -- default for progressive retrieval queries

-- ============================================================
-- contradictions: detected inconsistencies across pages
-- ============================================================
CREATE TABLE IF NOT EXISTS contradictions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id       INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    other_page_id INTEGER REFERENCES pages(id) ON DELETE SET NULL,
    type          TEXT    NOT NULL,       -- "temporal", "cross_page", "stale"
    description   TEXT    NOT NULL,
    detected_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    resolved_at   TEXT    DEFAULT NULL    -- NULL = unresolved
);

CREATE INDEX IF NOT EXISTS idx_contradictions_page ON contradictions(page_id);
CREATE INDEX IF NOT EXISTS idx_contradictions_unresolved ON contradictions(resolved_at) WHERE resolved_at IS NULL;

-- ============================================================
-- knowledge_gaps: queries the brain couldn't answer well
-- Privacy-safe by default: raw query text is NOT retained
-- unless explicitly approved.  Only query_hash is stored on
-- detection; query_text is populated post-approval.
-- ============================================================
CREATE TABLE IF NOT EXISTS knowledge_gaps (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    query_hash       TEXT    NOT NULL,   -- SHA-256 of original query, always stored
    query_text       TEXT    DEFAULT NULL,  -- raw text retained only after approval
    context          TEXT    NOT NULL DEFAULT '',
    confidence_score REAL    DEFAULT NULL,
    sensitivity      TEXT    NOT NULL DEFAULT 'internal',
    approved_by      TEXT    DEFAULT NULL,
    approved_at      TEXT    DEFAULT NULL,
    redacted_query   TEXT    DEFAULT NULL,
    resolved_at      TEXT    DEFAULT NULL,
    resolved_by_slug TEXT    DEFAULT NULL,
    detected_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (sensitivity IN ('internal', 'external', 'redacted')),
    CHECK (query_text IS NULL OR (approved_by IS NOT NULL AND approved_at IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS idx_gaps_unresolved ON knowledge_gaps(resolved_at) WHERE resolved_at IS NULL;
```

### Schema Notes

- **All text fields:** UTF-8
- **All timestamps:** ISO 8601 (`YYYY-MM-DDTHH:MM:SSZ` for timestamps, `YYYY-MM-DD` for dates)
- **Embeddings:** BGE-small-en-v1.5 produces 384-dimensional float32 vectors. sqlite-vec stores them natively in `vec0` virtual table. 384 floats × 4 bytes = 1,536 bytes per chunk (vs 6,144 bytes for OpenAI's 1536-dim model - 4x smaller)
- **JSON fields** (`frontmatter`, `data`, `pages_updated`): stored as TEXT, parsed in application layer
- **Slugs** include directory prefix: `people/pedro-franceschi`, `companies/river-ai`, `deals/river-ai-series-a`
- **Embedding model registry (single source of truth):** Each model gets its own vec0 table (dimension is baked in). The `embedding_models` table is the ONLY authoritative selector for the active model — a partial unique index (`WHERE active = 1`) enforces that exactly one model is active at any time. The `config` keys `embedding_model` and `embedding_dimensions` are derived from the registry at startup and are read-only aliases. Switching models: register new model with `active=0`, run `gbrain embed --all` to populate its vec table, then in a single transaction flip `active=0` on old and `active=1` on new. Old table kept for rollback. `gbrain validate --embeddings` checks: (a) exactly one active model exists, (b) all chunks reference the active model, (c) all vec_rowids resolve correctly.
- **Assertions:** A temporal fact table with provenance (`subject, predicate, object, valid_from, valid_until, supersedes_id, asserted_by, source_ref, evidence_text`). Each assertion has a surrogate `id`. Tier 2 rewrites supersede old assertions by setting `valid_until` on the prior row AND pointing `supersedes_id` to it in the new row. Contradiction detection queries only current beliefs (`valid_until IS NULL`). `valid_from` is nullable (NULL = unknown). Dedup enforced in application logic.
- **Chunk types:** `page_embeddings.chunk_type` tracks whether a chunk is a `truth_section` or `timeline_entry`. Timeline entries are embedded individually for hyper-specific temporal retrieval.
- **Palace hierarchy (`wing`, `room`):** Auto-derived from slug by default. `people/pedro-franceschi` → wing: `pedro-franceschi`, room: derived from section headers (State, Assessment, etc.). Override via frontmatter `wing:` and `room:` fields. Wings map to MemPalace's concept of entity groupings; rooms map to topic sub-areas within an entity.
- **Summary:** Extracted from the first blockquote (`> ...`) in `compiled_truth` during `put`/`ingest`. Used by progressive retrieval to serve lightweight results without loading full pages.
- **Temporal links (`valid_from`, `valid_until`, `relationship`):** Links carry typed relationships and temporal validity windows. `valid_until IS NULL` = currently active. `valid_from` is nullable (NULL = unknown start date). Each link has a surrogate `id` used by `brain_link_close` for unambiguous targeting. Multiple intervals with unknown start dates are allowed — dedup and non-overlap enforced in application logic.
- **Freshness timestamps:** `updated_at` bumped on any page-scoped mutation. `truth_updated_at` bumped only when `compiled_truth` changes (Tiers 2-4). `timeline_updated_at` bumped only when timeline content or `timeline_entries` change (Tier 1). Staleness = `timeline_updated_at` > `truth_updated_at` by 30+ days.
- **Contradictions:** Detected by `gbrain check` (CLI) and `brain_check` (MCP). Stored with `resolved_at` for tracking. Unresolved contradictions surface in briefings and maintenance reports.
- **Knowledge gaps:** Logged by `brain_gap` (MCP) when `brain_query` returns no results or only low-confidence matches (below configurable threshold). **Privacy-safe by default:** only a SHA-256 `query_hash` is stored at detection time; raw `query_text` is `NULL` until explicitly approved. `confidence_score` stores the highest search score from the triggering query. **Sensitivity is always `internal` at creation — `brain_gap` does not accept a sensitivity parameter.** Escalation to `redacted` or `external` requires a separate `brain_gap_approve` call that records `approved_by`, `approved_at`, and optionally populates `query_text` and `redacted_query` (the anonymised version for `redacted` mode). A CHECK constraint enforces that `query_text` can only be non-NULL when an approval audit trail exists. The research skill (`skills/research/SKILL.md`) refuses external calls for any gap without an approval record. When a gap is resolved via ingest, `resolved_at` and `resolved_by_slug` are set. Unresolved gaps surface in briefings alongside contradictions.

---

## CLI Reference

The CLI is a thin dispatcher. Each command maps to a handler in `src/commands/`.

```
USAGE:
    gbrain [OPTIONS] <COMMAND>

OPTIONS:
    --db <PATH>      Path to brain.db [env: GBRAIN_DB] [default: ./brain.db]
    --json           Output JSON instead of human-readable text
    --version        Print version
    --tools-json     Print MCP tool discovery JSON

COMMANDS:
    init [PATH]                     Create a new brain.db
    get <SLUG>                      Read a page by slug
    put <SLUG> [FILE]               Write/update a page (stdin or file)
    search <QUERY>                  FTS5 full-text search
    query <QUESTION>                Hybrid semantic search (FTS5 + vector, set-union merge)
      --depth <LEVEL>               summary|section|full|auto [default: auto]
      --token-budget <N>            Max tokens to return [default: from config]
      --wing <WING>                 Filter to specific palace wing
    ingest <FILE>                   Ingest a source document
      --type <TYPE>                 meeting|article|doc|conversation
    link <FROM> <TO>                Create cross-reference
      --context <TEXT>              Sentence containing the link
      --relationship <TYPE>         Relationship type (works_at, founded, invested_in, board_member, related)
      --valid-from <DATE>           When relationship started (ISO 8601)
      --valid-until <DATE>          When relationship ended (NULL = still current)
    link-close <LINK_ID>              Close a temporal link interval by ID
      --valid-until <DATE>          When relationship ended (required)
    links <SLUG>                    List all outbound links with IDs (for discovering link IDs)
      --temporal <current|historical|all>  Filter by temporal state [default: all]
      --json                        Include link IDs, valid_from, valid_until in output
    unlink <FROM> <TO>              Remove cross-reference entirely
      --relationship <TYPE>         Specific relationship to remove (default: all)
    backlinks <SLUG>                Show pages that link TO this slug (includes link IDs)
      --temporal <current|historical|all>  Filter by temporal state [default: current]
    graph <SLUG>                    N-hop neighborhood graph (pages + links as JSON)
      --depth <N>                   Hops from center node [default: 2]
      --temporal <current|historical|all>  Filter links by temporal state [default: current]
      --limit <N>                   Max nodes to return [default: 50]
    check [SLUG]                    Run contradiction detection
      --all                         Check entire brain
      --type <temporal|cross_page|stale>  Filter by check type
    gaps                            List unresolved knowledge gaps
      --limit <N>                   Max results [default: 20]
      --resolved                    Include resolved gaps
    tags <SLUG>                     List tags for a page
    tag <SLUG> <TAG>                Add a tag
    untag <SLUG> <TAG>              Remove a tag
    timeline <SLUG>                 Show timeline entries
    timeline-add <SLUG>             Add a structured timeline entry
      --date <YYYY-MM-DD>           Date of entry (required)
      --summary <TEXT>              One-line summary (required)
      --source <SOURCE>             Source identifier (e.g. "meeting/123")
      --detail <TEXT>               Full markdown detail
    list                            List pages with filters
      --type <TYPE>                 Filter by page type
      --tag <TAG>                   Filter by tag
      --limit <N>                   Max results [default: 50]
      --sort <updated|created|title> Sort order [default: updated]
    stats                           Brain statistics
    export [--dir <PATH>]           Export to markdown directory [default: ./export/]
      --raw --import-id <ID>        Byte-exact export from specific import batch
    import <DIR>                    Import from markdown directory
    compact                         Checkpoint WAL → single file (for transport/backup)
    embed [<SLUG>]                  Generate/regenerate embeddings
      --all                         Embed all pages
      --stale                       Only pages updated since last embedding
    config get <KEY>                Read a config value
    config set <KEY> <VALUE>        Write a config value
    config list                     List all config
    serve                           Start MCP server (stdio transport)
    call <TOOL> <JSON>              Raw tool call (GL pattern)
    pipe                            JSONL pipe mode (one JSON object per line)
    validate                        Run integrity checks on brain.db
      --links                       Check link interval non-overlap, temporal ordering
      --assertions                  Check assertion dedup, supersession chains
      --embeddings                  Check all chunks have valid vec_rowids in active model
      --all                         Run all integrity checks
    skills                          List active skills (embedded + external)
    skills doctor                   Verify skill resolution, show versions/hashes
    version                         Version info
```

### DB path resolution

1. `--db /path/to/brain.db` flag (highest priority)
2. `GBRAIN_DB` environment variable
3. `./brain.db` in current directory (default)

### Output formats

- **Default:** Human-readable markdown/text (Claude-friendly)
- **`--json`:** JSON for programmatic use
- **`gbrain pipe`:** JSONL streaming mode
- **`gbrain --tools-json`:** MCP tool discovery format (compatible with Claude Code tool use)

### Usage examples

```bash
# Create a new brain
$ gbrain init ~/my-brain.db

# Import existing markdown directory
$ gbrain import /data/brain/ --db ~/brain.db
Importing 7,471 files...
  people:    1,222 pages
  companies:   847 pages
  deals:       234 pages
  ...
  links: 14,329 cross-references extracted
  raw_data: 892 sidecar files loaded
  timeline_entries: 23,441 entries parsed
Done. brain.db: 487MB
Generating embeddings (7,471 pages, section strategy)...
  Embedded 22,847 chunks in 3m 14s
brain.db (with embeddings): 521MB
Validation: 7,471 files → 7,471 pages ✓

# Full-text search
$ gbrain search "River AI"
people/ali-partovi.md    (score: 12.3)  ...River AI board member since 2024...
companies/river-ai.md    (score: 45.7)  ...River AI is building...

# Semantic query
$ gbrain query "who knows Jensen Huang?"
Searching 7,471 pages (FTS5 + vec0, set-union merge)...
people/ali-partovi.md      — mentioned NVIDIA partnership (score: 0.89)
people/ilya-sutskever.md   — co-presented at NeurIPS (score: 0.84)
people/marc-andreessen.md  — board connection via Meta (score: 0.81)

# Read a page
$ gbrain get people/pedro-franceschi
---
title: Pedro Franceschi
type: person
...
---
# Pedro Franceschi
> Co-founder and CEO of Brex. YC alum (W17)...

# Write/update a page
$ cat updated-pedro.md | gbrain put people/pedro-franceschi

# Stats
$ gbrain stats
Pages: 7,471
  people:    1,222
  companies:   847
  deals:       234
  ...
Links: 14,329
Tags: 8,892
Raw data: 892
Timeline entries: 23,441
Embeddings: 22,847 chunks (bge-small-en-v1.5)
DB size: 521MB

# Start MCP server
$ gbrain serve
GigaBrain MCP server running (stdio)
Model: bge-small-en-v1.5 (384-dim, local)
DB: /Users/garry/brain.db (521MB, 7471 pages)
Tools: search, get, put, ingest, link, query, timeline, tags, list, stats
```

---

## MCP Server

### Transport

Stdio (standard MCP). The client spawns `gbrain serve` as a subprocess and communicates via stdin/stdout JSON-RPC 2.0.

### Claude Code config (`~/.claude/mcp.json`)

```json
{
  "mcpServers": {
    "gbrain": {
      "command": "gbrain",
      "args": ["serve", "--db", "/path/to/brain.db"]
    }
  }
}
```

### Tools

| Tool | Description | Parameters |
|------|-------------|------------|
| `brain_search` | FTS5 full-text search | `{ query: string, type?: string, wing?: string, limit?: number }` |
| `brain_query` | Hybrid search (SMS + set-union + progressive retrieval) | `{ question: string, depth?: "summary"\|"section"\|"full"\|"auto", token_budget?: number, wing?: string, limit?: number }` |
| `brain_get` | Read a page by slug | `{ slug: string }` |
| `brain_put` | Write/update a page (auto-extracts summary + palace metadata) | `{ slug: string, content: string, expected_version: number, assertions?: Array<{subject, predicate, object, valid_from?, asserted_by?, source_ref?, evidence_text?}> }` or `{ slug: string, compiled_truth?: string, timeline_append?: string, frontmatter?: object, expected_version: number }` |
| `brain_ingest` | Ingest a source document — **single transactional mutation** | `{ content: string, source_type: string, source_ref: string, force?: boolean, pages: Array<{slug, content, expected_version, assertions?, links?, timeline_entries?, tags?}> }` |
| `brain_link` | Create a new link interval (returns link ID) | `{ from: string, to: string, relationship?: string, context?: string, valid_from?: string, page_version: number }` |
| `brain_links` | List outbound links with IDs | `{ slug: string, temporal?: "current"\|"historical"\|"all" }` |
| `brain_link_close` | Close an existing link interval by ID | `{ link_id: number, valid_until: string, page_version: number }` |
| `brain_unlink` | Remove cross-reference entirely | `{ from: string, to: string, relationship?: string, page_version: number }` |
| `brain_timeline` | Get timeline entries | `{ slug: string, limit?: number }` |
| `brain_timeline_add` | Add timeline entry | `{ slug: string, date: string, summary: string, source?: string, detail?: string, page_version: number }` |
| `brain_tags` | List tags for a page | `{ slug: string }` |
| `brain_tag` | Add/remove tag | `{ slug: string, tag: string, remove?: boolean, page_version: number }` |
| `brain_list` | List pages with filters | `{ type?: string, tag?: string, wing?: string, limit?: number, sort?: string }` |
| `brain_backlinks` | Pages linking to a slug (temporal filtering) | `{ slug: string, temporal?: "current"\|"historical"\|"all" }` |
| `brain_graph` | N-hop neighborhood graph (pages + links as JSON) | `{ slug: string, depth?: number, temporal?: "current"\|"historical"\|"all", limit?: number }` |
| `brain_check` | Run contradiction detection | `{ slug?: string, type?: "temporal"\|"cross_page"\|"stale", resolve?: string }` |
| `brain_gap` | Log a knowledge gap (always created as `internal` — no caller override) | `{ query_text: string, context?: string, confidence_score?: number }` |
| `brain_gap_approve` | Escalate gap sensitivity (audited — records approver + timestamp) | `{ gap_id: number, sensitivity: "redacted"\|"external", approver: string, redacted_query?: string }` |
| `brain_gaps` | List unresolved knowledge gaps | `{ limit?: number, include_resolved?: boolean }` |
| `brain_stats` | Brain statistics (includes contradiction + gap counts) | `{}` |
| `brain_raw` | Read/write raw enrichment data | `{ slug: string, source?: string, data?: object, page_version: number }` |

**Concurrency Model:**

- **`brain_put`** requires `expected_version`. Mismatch → MCP Tool Error (conflict). Agent must `brain_get`, merge, retry.
- **`brain_ingest`** is a single server-side transactional mutation. The agent passes all page updates, links, timeline entries, assertions, and tags in one call. The server wraps everything in a SQLite transaction, checks `expected_version` for each page, and either commits all or rolls back all. This eliminates the window where side tables (links, timeline, tags) can desync from page content.
- **All page-scoped mutators** (`brain_link`, `brain_unlink`, `brain_timeline_add`, `brain_tag`, `brain_raw`) require `page_version`. The server verifies it matches before mutating and bumps `pages.version` on success. Mismatch → same conflict error as `brain_put`. No write path bypasses the version check.

### Resources

| Resource | URI | Description |
|----------|-----|-------------|
| Page | `brain://pages/{slug}` | Full page content as markdown |
| Index | `brain://index` | All page slugs grouped by type |

### Prompts

| Prompt | Description |
|--------|-------------|
| `brain_briefing` | Compile a briefing from current brain state |
| `brain_ingest_meeting` | Guide for ingesting a meeting transcript |

---

## Hybrid Search (Exact-Match Short-Circuit + Set-Union)

The core search experience uses a 4-step pipeline designed to prevent semantic noise from burying exact keyword matches. FTS5 + vector fan-out, merged using **set-union** (default) or RRF (fallback). Palace-style pre-filtering narrows the search space. SMS (exact-match short-circuit) ensures title/slug matches always rank first.

### Why set-union over RRF

UNC's AutoResearchClaw pipeline (Apr 2026) tested RRF-style score re-ranking and found it **degrades performance** — score-based re-ranking disrupts the semantic ordering that dense retrieval already established. Their discovery: set-union merging (keep vector ranking intact, append BM25-only results) delivered +44% F1 in a single iteration on LoCoMo benchmark. Ablation confirmed: removing the BM25 hybrid component = -14% F1. The sparse results add value, but only when they don't interfere with dense ranking.

Config: `search_merge_strategy` in the config table. Default `set-union`, fallback `rrf` for A/B testing on different corpora.

### Algorithm

```
query = "who knows Jensen Huang?"

Step 0: Palace pre-filter (intent classification)
──────────────────────────────────────────────────
Classify query intent → target wing(s) + room(s).
Rule-based: extract entity names from query, match against page slugs/titles.
If match found: constrain Steps 1-2 to matching wing(s).
If no match: search all pages (no filter).

Example: "who knows Jensen Huang?" → wing filter: '%jensen-huang%'
Example: "what's our thesis on River AI?" → wing filter: '%river-ai%'
Example: "all YC founders in batch W25" → no wing filter (cross-cutting query)

When an LLM agent is driving the query (via MCP), the agent can pass explicit
wing/room filters based on skills/query/SKILL.md guidance.

Step 1: SMS (Exact-Match Short-Circuit) — ABSOLUTE RANKING
────────────────────────────────────────────────────────────────
-- If the query exactly matches a page title or slug, it jumps to the top.
-- This prevents semantic fuzziness from burying the obvious result.
SELECT id, title, slug FROM page_fts
WHERE title MATCH ? OR slug MATCH ?
LIMIT 5;
-- Result: exact_results[] — these appear first in the final output, always.

Step 2: Vector similarity search (candle + sqlite-vec) — PRIMARY RANKING
────────────────────────────────────────────────────────────────
-- Embed the query with pure-Rust candle (local, in-process)
query_embedding = candle_embed("who knows Jensen Huang?")

-- Resolve active model's vec table from the embedding_models registry
-- (e.g. active model = 'bge-small-en-v1.5' → vec_table = 'page_embeddings_vec_384')
active_model = SELECT name, vec_table FROM embedding_models WHERE active = 1;

-- cosine similarity via sqlite-vec, with optional palace filter
-- NOTE: vec table name is resolved at runtime from active_model.vec_table
-- Join key: pev.rowid = pe.vec_rowid (NOT pe.id — id is internal metadata key)
SELECT pe.page_id, pe.chunk_text,
       vec_distance_cosine(pev.embedding, ?) AS vec_score
FROM {active_model.vec_table} pev
JOIN page_embeddings pe ON pev.rowid = pe.vec_rowid
                       AND pe.model = {active_model.name}
JOIN pages p ON pe.page_id = p.id
WHERE (? IS NULL OR p.wing LIKE ?)  -- palace filter (NULL = no filter)
ORDER BY vec_score
LIMIT 50;

-- Deduplicate: if same page_id appears multiple times, keep highest-scoring chunk
-- Result: vec_results[] with original ranking preserved

Step 3: FTS5 keyword search — SUPPLEMENTARY
────────────────────────────────────────────
SELECT pages.id, pages.slug, pages.title,
       bm25(page_fts) AS fts_score
FROM page_fts
JOIN pages ON pages.id = page_fts.rowid
WHERE page_fts MATCH ?
  AND (? IS NULL OR pages.wing LIKE ?)  -- same palace filter
ORDER BY fts_score
LIMIT 50;

-- Result: fts_results[]

Step 4: Set-union merge (default)
──────────────────────────────────
-- Start with SMS exact matches (always first)
merged = exact_results.clone()

-- Then vector results in their original order (primary ranking)
for result in vec_results:
    if result.page_id NOT IN merged:
        merged.append(result)

-- Then FTS5-only results (those NOT in vector set) at the end
for result in fts_results:
    if result.page_id NOT IN merged:
        merged.append(result)

-- Apply lightweight boosts only to break ties within the appended FTS-only set:
--   +0.01 if page type matches question intent
--   +0.01 if updated_at within last 30 days

Step 4 (alt): RRF merge (fallback, config: search_merge_strategy = "rrf")
───────────────────────────────────────────────────────────────────────────
SMS exact matches still go first. Then for remaining pages, compute:
  rrf_score = 1/(k + rank_vec) + 1/(k + rank_fts)
  where k = 60  (standard RRF constant)
If a page appears in only one set: rrf_score = 1/(k + rank)
Sort by rrf_score DESC.

Step 5: Final ranking + fetch
──────────────────────────────
Top results returned with:
  - slug
  - title
  - summary (executive blockquote — for progressive retrieval Stage 1)
  - relevant excerpt (chunk_text from best-matching vector chunk, or FTS snippet)
  - score
  - type
  - wing, room

For deeper results, see Progressive Retrieval below.
```

### Palace filtering impact

MemPalace's published ablation: base retrieval at 60.9% R@5 → 94.8% with wing+room filtering (+34%). gbrain's palace filter is derived from slug structure rather than a separate palace DB, but the principle is identical: constrain the search space before running expensive similarity queries.

### Performance targets

- SMS exact match: < 5ms (FTS5 title/slug lookup)
- Palace filter classification: < 5ms (regex match against slug index)
- FTS5 search: < 50ms for 100K pages
- Vector search: < 200ms for 50K chunks (sqlite-vec with float32[384])
- Full hybrid query (SMS + palace + vector + FTS5 + merge): < 250ms
- Embedding generation (query): < 20ms (BGE-small-en-v1.5 via candle on CPU, already loaded)

---

## Progressive Retrieval

Token-budget-gated expansion. Instead of returning full pages and hoping the agent handles context management, gbrain controls how much content it serves based on a configurable token budget.

### Why this matters

OMNIMEM's ablation (AutoResearchClaw, Apr 2026): removing progressive retrieval = -17% F1 — the largest single component contribution. agentmemory reports 92% fewer tokens vs dumping everything into context. The pattern: serve summaries first, expand on demand, stop when the budget is consumed.

### Depth levels

| Level | What's returned | Tokens per result | Use case |
|-------|----------------|-------------------|----------|
| `summary` | Title + executive summary blockquote | ~50-100 | Quick scan, "do we know this person?" |
| `section` | Best-matching chunk from vector search | ~200-500 | Targeted retrieval, "what's the latest on X?" |
| `full` | Complete `compiled_truth` | ~500-2000 | Deep read, preparing for a meeting |
| `auto` | Start at summary, expand top results until token budget consumed | varies | Default — agent doesn't need to choose |

### Token budget

Default: 4000 tokens (configurable in `config` table as `default_token_budget`). Override per-query via the `token_budget` parameter.

### Algorithm (auto mode)

```
1. Run hybrid search → ranked results[]
2. For each result (in rank order):
   a. Include summary (title + blockquote). Add to running token count.
   b. If token_budget - running_count > 500:
      Include best-matching section/chunk. Add to running count.
   c. If token_budget - running_count > 1500 AND result is top-3:
      Include full compiled_truth. Add to running count.
   d. If running_count >= token_budget: stop expanding, return.
3. Return results with their expansion level noted.
```

### MCP integration

The `brain_query` tool gains `depth` and `token_budget` parameters:

```json
{
  "question": "who knows Jensen Huang?",
  "token_budget": 4000,
  "depth": "auto",
  "wing": null
}
```

Response includes expansion metadata:

```json
{
  "results": [
    {
      "slug": "people/ali-partovi",
      "title": "Ali Partovi",
      "depth": "full",
      "summary": "Co-founder of Neo. NVIDIA board connection via...",
      "content": "...full compiled_truth...",
      "score": 0.89,
      "tokens_used": 1200
    },
    {
      "slug": "people/ilya-sutskever",
      "title": "Ilya Sutskever",
      "depth": "section",
      "summary": "Co-founder of SSI. Previously Chief Scientist at OpenAI...",
      "content": "...best matching chunk...",
      "score": 0.84,
      "tokens_used": 450
    }
  ],
  "total_tokens": 3800,
  "budget_remaining": 200
}

---

## Ingest Pipeline

```
Source document (meeting notes, article, transcript)
            │
            ▼
    gbrain ingest <file> --type meeting
            │
            ├─→ Begin ingest transaction
            │     - Compute stable ingest key: SHA-256(source file content)
            │     - BEGIN TRANSACTION
            │     - SELECT from ingest_log WHERE ingest_key = ? → if exists, ROLLBACK + skip
            │     - All writes below happen inside this transaction
            │     - ingest_log row is INSERT'd at the end, inside the same transaction
            │     - COMMIT writes everything atomically (mutations + log row)
            │     - Crash before COMMIT → full rollback, key never persists, retry starts clean
            │
            ├─→ Parse source (Claude Code reads ingest/SKILL.md, follows workflow)
            │     - Identify: participants, companies, topics, decisions, action items
            │
            ├─→ For each entity mentioned (four-tier consolidation):
            │     ├─ gbrain get <slug>   → exists? update using tier rules:
            │     │     Tier 1: Append raw evidence to timeline (ALWAYS — never gated by novelty)
            │     │     Tier 2: Novelty check THEN update State section facts if changed
            │     │       ├─ Jaccard similarity vs existing compiled_truth
            │     │       │   > 0.85 → skip derived rewrite (duplicate content)
            │     │       ├─ Embedding cosine similarity vs existing chunks
            │     │       │   > 0.95 → warn (near-duplicate, allow override with --force)
            │     │       └─ Below thresholds → proceed with Tiers 2-4
            │     │     Tier 3: Re-evaluate Assessment concepts if facts shifted
            │     │     Tier 4: Rewrite executive summary blockquote if picture changed
            │     └─ doesn't exist?     → gbrain put <slug> (create from template)
            │     Note: page writes use optimistic concurrency — each page has a version
            │     counter; the update fails if the version changed since read (concurrent
            │     agent detected). The caller retries from the read step.
            │
            ├─→ Extract summary from first blockquote → pages.summary field
            │
            ├─→ Derive palace metadata
            │     wing: from slug (people/pedro → pedro-franceschi)
            │     room: from section headers or frontmatter override
            │
            ├─→ Extract and create links (with temporal metadata)
            │     gbrain link <from> <to> --relationship "works_at" --valid-from "2024-01-15" --context "sentence..."
            │
            ├─→ Add structured timeline entries
            │     gbrain timeline-add <slug> --date YYYY-MM-DD --summary "..."
            │     Dedupe: (page_id, date, summary_hash) UNIQUE constraint prevents replay duplicates
            │
            ├─→ Update embeddings for modified pages
            │     gbrain embed <slug>   (or --stale to batch)
            │
            ├─→ Auto-log to ingest_log table (keyed by ingest SHA)
            │
            └─→ Commit transaction
```

The `gbrain ingest` command receives the raw source file. The actual intelligence — how to parse a meeting transcript, which entities get pages, how to rewrite compiled_truth, when to append vs rewrite — lives in `skills/ingest/SKILL.md`. The binary handles novelty checking (Jaccard + cosine, scoped to Tiers 2-4 only — Tier 1 evidence is never suppressed) and palace metadata derivation. Everything else is skill-driven.

### Novelty check implementation

```rust
fn check_novelty(new_content: &str, existing_page: &Page, db: &Connection) -> NoveltyResult {
    // 1. Jaccard similarity between new compiled_truth and existing
    let jaccard = jaccard_similarity(new_content, &existing_page.compiled_truth);
    if jaccard > 0.85 {
        return NoveltyResult::DuplicateDerived;   // skip Tiers 2-4 (derived rewrites)
    }                                              // Tier 1 (timeline/links) ALWAYS proceeds

    // 2. Embedding cosine similarity between new content and existing chunks
    let query_embedding = embed(new_content);
    let max_sim = max_chunk_similarity(&query_embedding, existing_page.id, db);
    if max_sim > 0.95 {
        return NoveltyResult::NearDuplicate;  // warn on Tiers 2-4, proceed with --force
    }                                         // Tier 1 ALWAYS proceeds

    NoveltyResult::Novel                      // all tiers proceed
}
```

**Critical invariant:** Novelty checks only gate derived rewrites (Tiers 2-4: State, Assessment, summary). Tier 1 (raw timeline evidence, links, assertions) is **always** written regardless of similarity score. This prevents silent evidence loss from recurring meetings or incremental notes where compiled_truth looks similar but new timeline events contain material information.

Jaccard similarity uses token-level overlap (whitespace-split, lowercased). Fast and effective for detecting copy-paste or minimal-edit duplicates. The embedding check catches semantic duplicates where wording differs but meaning is identical.

---

## Migration Plan

Importing an existing markdown brain (7,471 files at `/data/brain/`):

### Type mapping

`gbrain import` resolves page types in three tiers:

1. **Frontmatter `type:` wins** when present and non-blank.
2. **Top-level folder inference** applies when `type:` is absent, blank, or null.
3. **Fallback to `concept`** if no folder rule matches.

Supported folder inference rules:

```
Top-level folder (case-insensitive; numeric prefixes like `1. ` stripped) → page type:
  Projects / 1. Projects   → project
  Areas / 2. Areas         → area
  Resources / 3. Resources → resource
  Archives / 4. Archives   → archive
  Journal / Journals       → journal
  People                   → person
  Companies / Orgs         → company
```

Any other top-level folder — or a file imported from the vault root — defaults to `concept`.

### Parse algorithm

```rust
fn parse_markdown_file(content: &str, file_path: &Path) -> ParsedPage {
    // 1. Extract YAML frontmatter (between first --- and second ---)
    let (frontmatter_str, body) = split_frontmatter(content);
    let frontmatter: serde_yaml::Value = serde_yaml::from_str(&frontmatter_str)?;

    // 2. Split body at first horizontal rule (--- on its own line, after frontmatter)
    //    This separates compiled_truth from timeline
    let (compiled_truth, timeline) = match body.find("\n---\n") {
        Some(idx) => (body[..idx].trim(), body[idx + 5..].trim()),
        None      => (body.trim(), ""),
    };

    // 3. Extract slug from file path
    //    /data/brain/people/pedro-franceschi.md → "people/pedro-franceschi"
    let slug = file_path
        .strip_prefix(base_dir)?
        .with_extension("")
        .to_string_lossy()
        .to_string();

    ParsedPage { slug, frontmatter, compiled_truth, timeline }
}
```

### Link extraction

```rust
// Wiki-style links: [Display Text](../people/name.md)
// Convert to slugs: "people/name"
let link_re = Regex::new(r"\[([^\]]+)\]\((\.\./)?([[\w/-]+)\.md\)")?;

// For each match: record from_slug, to_slug, surrounding sentence as context
// Resolve relative paths against the source file's directory
```

### Timeline parsing

```rust
// Timeline line format: - **YYYY-MM-DD** | Source — Summary. Detail.
let timeline_re = Regex::new(
    r"^- \*\*(\d{4}-\d{2}-\d{2})\*\*\s*\|\s*([^—]+)—\s*(.+)$"
)?;
// Each match → { date, source, summary }
// Multi-line continuation (indented) → detail field
```

### Sidecar files

```rust
// For people/pedro-franceschi.md, check people/.raw/pedro-franceschi.json
// Format: { "sources": { "crustdata": {...}, "happenstance": {...} } }
// → raw_data rows: (page_id, "crustdata", JSON), (page_id, "happenstance", JSON)
```

### Transaction safety

```rust
conn.execute("BEGIN TRANSACTION", [])?;
// Insert all pages
// Insert all tags (after all pages exist for FK resolution)
// Insert all links (resolve slugs → page IDs)
// Insert timeline entries
// Insert raw data
conn.execute("COMMIT", [])?;
```

### Embedding generation (post-import)

```bash
$ gbrain embed --all
```

Chunks `compiled_truth` at `##` header boundaries (section strategy, `chunk_type: 'truth_section'`).
Chunks `timeline` at individual entries — each `- **YYYY-MM-DD**` line becomes its own chunk (`chunk_type: 'timeline_entry'`). This prevents long timelines from becoming a single oversized chunk and enables hyper-specific temporal retrieval ("What did X do in March 2024?").
- Pages without headers: chunk at ~500-token boundaries
- Target: ~200-800 tokens per truth section chunk, ~50-200 per timeline entry
- BGE-small-en-v1.5 on CPU: ~7,500 pages × ~3 chunks avg = ~22,500 embeddings
- Estimated time: 3-5 minutes on Apple Silicon, 8-12 minutes on Intel

### Validation

```bash
# Count pages in DB vs files on disk — must match
# Count links vs parsed wiki links — must match
# Spot-check 10 random pages: export → diff against original
# Report any discrepancies
$ gbrain import /data/brain/ --validate-only   # dry-run validation without writing
```

### Special files

- `index.md` → stored in `config` table as `original_index`
- `log.md` → parsed and inserted into `ingest_log` table
- `schema.md` → stored in `config` table as `original_schema`
- `README.md` → ignored during import

---

## Export and Round-trip

The export command reconstructs the directory structure from DB state. The round-trip
contract is **semantic equivalence**, not byte-for-byte identity — frontmatter key
ordering, trailing whitespace, and YAML formatting may differ from the original source.

To support rollback and diffing, import stores the raw source bytes of every file in
`raw_imports` (keyed by page_id + import_id). These are **immutable snapshots** — they
are never updated when pages are mutated after import. This is intentional: raw exports
represent the state at import time, not current state.

**Important:** `--raw` export requires `--import-id` and only emits bytes from that
specific import batch. There is no "current state" raw export — use normalized export
for current state. This prevents the dangerous split-brain where raw export silently
discards post-import edits.

```rust
fn export_page(page: &Page, mode: ExportMode) -> String {
    match mode {
        // Raw mode: byte-for-byte faithful to a specific import batch (required)
        ExportMode::Raw { import_id } => {
            get_raw_import(&page.id, &import_id)
                .expect("no raw snapshot for this page in the specified import batch")
        }
        // Normalized mode: reconstructed from current DB columns (default)
        ExportMode::Normalized => export_normalized(page),
    }
}

fn export_normalized(page: &Page) -> String {
    // 1. Reconstruct YAML frontmatter from frontmatter JSON
    let frontmatter = serde_yaml::to_string(
        &serde_json::from_str::<serde_yaml::Value>(&page.frontmatter)?
    )?;

    // 2. Reconstruct body
    let mut body = page.compiled_truth.clone();
    if !page.timeline.is_empty() {
        body.push_str("\n\n---\n\n");
        body.push_str(&page.timeline);
    }

    // 3. Combine
    format!("---\n{}---\n\n{}\n", frontmatter, body)
}

// Write to: <export-dir>/<slug>.md
// Reconstruct .raw/ sidecars from raw_data table
// Regenerate index.md from page list
```

### Round-trip validation

```bash
# Semantic validation (default): normalized fields must match
$ gbrain export --dir /tmp/brain-export/
$ gbrain validate --original /data/brain/ --exported /tmp/brain-export/
# Checks: same pages, same frontmatter keys/values, same compiled_truth, same timeline entries

# Byte-exact validation: requires --import-id (raw exports are immutable snapshots)
$ gbrain export --raw --import-id <ID> --dir /tmp/brain-export-raw/
$ diff -r /data/brain/ /tmp/brain-export-raw/   # should be empty

# --raw without --import-id is an error (prevents accidental stale-byte export)
$ gbrain export --raw --dir /tmp/brain-export-raw/
# Error: --raw requires --import-id. Use normalized export for current state.
```

Semantic validation is the primary correctness test. Byte-exact round-trip is available
via `--raw` for rollback scenarios but is not the default contract.

---

## Skills (Fat Markdown)

Skills live in `skills/` at the repo root. Each is a standalone SKILL.md that Claude Code, OpenClaw, or any agent reads and follows. No logic is compiled into the binary.

---

### skills/ingest/SKILL.md

```markdown
---
name: gbrain-ingest
description: |
  Ingest meetings, articles, docs, and conversations into the brain.
  Follows the compiled truth + timeline architecture: update existing
  pages with new info, create pages for new entities, maintain cross-references.
---

# Ingest Skill

## Workflow

1. **Read the source.** Meeting transcript, article, document, conversation log.
   Identify: participants, companies, topics, decisions, commitments, action items.

2. **For each entity mentioned (four-tier consolidation):**
   - `gbrain get <slug>` — does a page exist? Note the `version` number.
   - **If yes:** Apply tier-by-tier update:
     - **Tier 1 (Raw Evidence):** Append to timeline. Always. The exact words, dates, sources. Never summarised. → `gbrain timeline-add`
     - **Tier 2 (Extracted Facts):** Update State section with structured assertions derived from evidence. Rewrite when facts change. Extract strict factual assertions (roles, statuses, locations) for the `assertions` table. When a fact changes, set `valid_until` on the old assertion before inserting the new one. → e.g., `{subject: "Pedro Franceschi", predicate: "is_ceo_of", object: "Brex", valid_from: "2018-01-01"}`
     - **Tier 3 (Synthesised Concepts):** Re-evaluate Assessment section if underlying facts shifted. Cross-reference patterns across linked pages. → e.g., "Brex's leadership team has been stable since 2024, with Pedro driving the enterprise pivot"
     - **Tier 4 (Narrative Intelligence):** Rewrite executive summary blockquote ONLY if the overall picture changed. This is the one-sentence answer to "what do we know and why does it matter?" → e.g., `> Pedro Franceschi runs Brex. Strong enterprise traction.`
     - `gbrain put <slug>` with updated content, `expected_version` from step 2, and `assertions` array. If ConflictError, re-read and merge. The system auto-extracts the summary from the blockquote and derives palace metadata.
   - **If no:** Create page using the appropriate template (see templates below).
     `gbrain put <slug>` with new content and `assertions`.

3. **Extract work-context entities.**
   - For each **decision** made: create/update a `decisions/<slug>` page. Link to stakeholders and projects.
     Record assertion: `{subject: "<decision-slug>", predicate: "decided_by", object: "<person>", valid_from: "<date>"}`
   - For each **commitment** with an owner and deadline: create/update a `commitments/<slug>` page.
     Record assertion: `{subject: "<person>", predicate: "committed_to", object: "<deliverable>", valid_from: "<date>"}`
     If a prior commitment shifted (new deadline, changed scope), set `valid_until` on the old assertion and create a new one. Update the commitment page's State section.
   - For each **action item** assigned: create/update an `actions/<slug>` page.
   - Link all work-context entities to the people, companies, and projects involved.

4. **Extract and create links (with temporal metadata).**
   - For every entity-to-entity reference:
     `gbrain link <from> <to> --relationship "works_at" --valid-from "2024-01-15" --context "..."`.
   - Links are bidirectional in meaning but stored directionally. Create both if both pages exist.
   - When evidence shows a relationship ended, close the specific interval by its link ID:
     `gbrain link-close <link_id> --valid-until "2026-03-01"`.
     The link ID is returned by `brain_link` on creation and by `brain_backlinks` on query.
     The binary enforces non-overlapping intervals: rejects close if it would create overlap.

5. **Parse timeline entries.**
   - For each datable event in the source:
     `gbrain timeline-add <slug> --date YYYY-MM-DD --summary "..." --source "meeting/123"`

6. **Log the ingest.**
   - The system auto-logs to ingest_log. Verify with `gbrain stats`.

7. **Refresh embeddings.**
   - After all puts: `gbrain embed --stale`
   - This ensures search reflects the new content immediately.

8. **Handle raw data.**
   - If the source includes structured data (API responses, JSON):
     `gbrain call brain_raw '{"slug":"...","source":"meeting","data":{...}}'`

## Entry criteria

Not everything gets a page. The bar:
- Anyone you met 1:1 or in a small group: YES
- YC staff, partners, active batch founders: YES
- Companies discussed in deal context: YES
- **Decisions** that affect multiple people or projects: YES
- **Commitments** with a specific owner and deadline: YES
- **Action items** assigned to a person with a due date: YES
- Casual mentions with no substance: NO
- Vague intentions without owner or deadline: NO (wait until they crystallize)
- Create the page only if its existence serves future retrieval.

## Quality rules

- Executive summary (blockquote at top) must reflect latest state
- State section gets REWRITTEN, not appended to
- Timeline is append-only, reverse-chronological (newest first)
- Open Threads: add new items, remove resolved ones (move to timeline)
- Every wiki link uses relative path format: `[Name](../people/name.md)`

## Source attribution format

Every fact in compiled_truth needs inline source citation with full provenance. A tweet reference without a URL is a broken citation — this is the #1 failure mode at scale.

Standard format:
```
[Source: User, direct message, 2026-04-07]
[Source: Meeting "Team Sync" #12345, 2026-04-03](meetings/2026-04-03-team-sync.md)
[Source: X/@handle, topic, 2026-04-05](https://x.com/handle/status/ID)
[Source: email from Name re Subject, 2026-04-05]
[Source: Crustdata LinkedIn enrichment, 2026-04-07]
[Source: Wall Street Journal, 2026-04-05](https://wsj.com/...)
```

### Source authority hierarchy

When sources conflict, note the contradiction in compiled_truth with BOTH citations. Never silently pick one.

1. **User's direct statements** (highest authority)
2. **Primary sources** (meetings, emails, direct conversations)
3. **Enrichment APIs** (Crustdata, Happenstance, Captain)
4. **Web search results**
5. **Social media posts** (lowest — require URL, context, date)

## Filing disambiguation

When filing entities, apply these rules to prevent duplicate pages in wrong categories:

| Question | Answer |
|----------|--------|
| Could you teach it as a framework? | → `concepts/` |
| Is it the user's own idea, synthesis, or observation? | → `originals/` |
| Could you build it, but nobody is working on it yet? | → Use `ideas/` directory (stored as `project` type with `status: idea` frontmatter) |
| Is someone actively building it? | → `projects/` |
| About them as a human? | → `people/` |
| About the organisation? | → `companies/` (both pages link to each other) |
| Nothing fits? | → Flag for human review. The schema may need to evolve. |

**Key rule for originals:** Capture the user's EXACT phrasing. The language IS the insight. Use their words for the slug. `meatsuit-maintenance-tax.md` not `biological-needs-maintenance-overhead.md`. The vividness IS the concept.

## Page templates

### Person

```markdown
---
title: First Last
type: person
tags: []
linkedin: ""
twitter: ""
score: 0
---
# First Last

> [Executive summary in one sentence.]

## State

**As of YYYY-MM-DD:** [What we know now. Current role, context, relationship status.]

## Assessment

[Intelligence assessment. What makes this person interesting or relevant.]

## Open Threads

- [ ] [Action item or open question]

---

## Timeline

- **YYYY-MM-DD** | meeting — [What happened]
```

#### Optional enrichment sections (Tier 1 contacts — 5+ interactions)

For high-engagement contacts, expand the person template with these sections between Assessment and Open Threads when evidence supports them. Don't add speculatively — each section requires direct observation or sourced claims.

```markdown
## What They Believe
- [Belief] — observed: [source, date]
- [Belief] — self-described: [interview/bio, date]
- [Belief] — inferred: [pattern across N interactions, confidence: high/medium/low]

## What They're Building
Current projects, recent ships, product direction.

## Hobby Horses
Topics they return to obsessively across conversations.

## Trajectory
Ascending, plateauing, pivoting, declining? Evidence-based, not speculative.

## Network
- **Close to:** People frequently seen with
- **Crew:** Which cluster or community
```

### Company

```markdown
---
title: Company Name
type: company
tags: []
domain: ""
linkedin: ""
stage: ""
---
# Company Name

> [What they do in one sentence.]

## State

**As of YYYY-MM-DD:** [Current status, funding, relevant context.]

## Assessment

[Thesis, opportunity, concerns.]

---

## Timeline

- **YYYY-MM-DD** | source — [Event]
```

### Decision

```markdown
---
title: "Decision: [Short description]"
type: decision
tags: []
date: YYYY-MM-DD
status: active           # active | superseded | reversed
stakeholders: []         # slugs of people involved
---
# Decision: [Short description]

> [One-sentence summary of what was decided and why.]

## Context

**Decided YYYY-MM-DD** by [who]. Discussed in [meeting/thread/email].

[What problem this solves. What alternatives were considered.]

## Implications

- [What changes as a result]
- [What depends on this holding]

## Open Threads

- [ ] [Follow-up or risk to monitor]

---

## Timeline

- **YYYY-MM-DD** | meeting — Original decision made
```

### Commitment

```markdown
---
title: "Commitment: [Who] → [What] by [When]"
type: commitment
tags: []
owner: ""                # slug of person who committed
due: YYYY-MM-DD
status: open             # open | completed | shifted | dropped
---
# Commitment: [Who] → [What] by [When]

> [Owner] committed to [deliverable] by [date]. Status: [open/completed/shifted/dropped].

## State

**As of YYYY-MM-DD:** [Current status. On track? Shifted? Why?]

## Context

Made during [meeting/conversation]. Linked to [decision/project].

---

## Timeline

- **YYYY-MM-DD** | meeting — Commitment made
```

### Action Item

```markdown
---
title: "Action: [Short description]"
type: action_item
tags: []
owner: ""                # slug of person responsible
due: YYYY-MM-DD
status: open             # open | done | blocked | dropped
priority: normal         # low | normal | high | urgent
---
# Action: [Short description]

> [Owner] to [do what] by [when]. Priority: [level].

## Context

From [meeting/conversation/decision]. Blocked by: [nothing / dependency].

---

## Timeline

- **YYYY-MM-DD** | source — Created
```

### Original (user's own thinking)

```markdown
---
title: "[Exact user phrasing]"
type: original
tags: []
origin: ""               # meeting, conversation, shower thought, riff on [concept]
confidence: high         # high | medium | low | speculative
---
# [Exact user phrasing]

> [One-sentence summary of the original idea in the user's voice.]

## The Idea

[Full articulation. Use the user's exact words wherever possible.
The language IS the insight — preserve phrasing, metaphors, and framing.]

## Why It Matters

[What this connects to. What it explains. What it predicts.]

## Connections

- Shaped by: [people who influenced the thinking]
- Played out at: [companies/projects where it applied]
- Discussed in: [meetings/conversations]
- Builds on: [other originals, concepts]

---

## Timeline

- **YYYY-MM-DD** | source — First articulated
```
```

---

### skills/query/SKILL.md

```markdown
---
name: gbrain-query
description: |
  Answer questions from the brain using FTS5 + semantic search + structured queries.
  Synthesize across multiple pages. Cite sources.
---

# Query Skill

## Strategy: Four-layer search

1. **Palace-filtered hybrid search** — `gbrain query "<question>" --wing "<entity>"` —
   the primary search path. Set-union merging with palace pre-filtering.
   Best for: most questions. The wing filter narrows the search space before
   vector + FTS5 run, dramatically improving precision.

2. **FTS5 keyword search** — `gbrain search "<query>"` — fast, exact matches.
   Best for: names, company names, specific terms, known slugs.

3. **Semantic vector search** — `gbrain query "<question>"` (no wing filter) — meaning-based.
   Best for: cross-cutting queries where the entity isn't known upfront.

4. **Structured queries** — `gbrain list --type person --tag yc-alum` +
   `gbrain backlinks <slug> --temporal current` — relational navigation.
   Best for: "all YC founders in batch W25", "who currently works at X?"

## Workflow

1. Decompose the question into search strategies.
2. Identify target wing(s) from entity names in the question.
3. Run hybrid query with palace filter and progressive retrieval:
   `gbrain query "<question>" --wing "<entity>" --depth auto --token-budget 4000`
4. Review summaries first (Tier 4 narrative). Expand to sections/full only if needed.
5. For temporal questions, check link validity:
   `gbrain backlinks <slug> --temporal current` vs `--temporal historical`
6. Before surfacing, verify with contradiction check:
   `gbrain check <slug>` — flag any unresolved contradictions in the answer.
7. Synthesize answer with citations: `[Pedro Franceschi](people/pedro-franceschi)`
8. If the answer is valuable enough to persist, consider creating a new source page.

## When you don't know

Say so. "The brain doesn't have info on X" is better than hallucinating.
Suggest enrichment: "Want me to research X and add them?"
```

---

### skills/maintain/SKILL.md

```markdown
---
name: gbrain-maintain
description: |
  Periodic brain maintenance. Find contradictions, stale info, orphan pages,
  missing cross-references. Keep the knowledge graph healthy.
---

# Maintain Skill

## Lint checks (run every few days)

1. **Contradiction detection** — Run `gbrain check --all` to detect:
   - **Link vs Assertion:** Current assertions (`valid_until IS NULL`) like `(Pedro, is_ceo, Brex)` where the link to `companies/brex` has a `valid_until` in the past. Pure SQL join.
   - **Temporal contradictions:** Page says "left X in 2025" but link to X has no `valid_until`. Compiled_truth dates vs link validity windows.
   - **Cross-page contradictions:** Multiple current assertions (`valid_until IS NULL`) with the same subject+predicate but different objects across pages. Superseded assertions (with `valid_until` set) are excluded — they're history, not contradictions.
   - **Staleness:** Pages where `timeline_updated_at` > `truth_updated_at` by 30+ days. These need Tier 2-4 rewrites.
   All findings stored in the `contradictions` table. Surface unresolved items in briefings.

2. **Stale info** — Pages where `timeline_updated_at` > `truth_updated_at` by 30+ days.
   Compiled truth is stale relative to new evidence. These need Tier 2 consolidation.

3. **Orphan pages** — `gbrain backlinks <slug>` = 0 inbound links.
   Either add links from related pages or flag for deletion.

4. **Missing cross-references** — Scan compiled_truth for mentions of known
   page titles that aren't formally linked. Add via `gbrain link` with relationship type.

5. **Dead links** — For each link, verify both pages still exist.

6. **Temporal link audit** — Links with `valid_until IS NULL` where evidence
   suggests the relationship ended. Flag for invalidation.

7. **Open thread audit** — Items older than 30 days. Resolved items
   still listed as open.

8. **Tag consistency** — Normalize: lowercase, hyphens. Merge near-duplicates.

9. **Palace metadata audit** — Pages with empty `wing` or `room` fields.
   Derive from slug structure and section headers.

10. **Embedding freshness** — Pages updated since last embedding:
    `gbrain embed --stale`

## Output

Write maintenance report:
`gbrain put sources/maintenance-YYYY-MM-DD` with findings and actions taken.
Include: contradictions found/resolved, stale pages rewritten, orphans linked/flagged, temporal links invalidated.
```

---

### skills/enrich/SKILL.md

```markdown
---
name: gbrain-enrich
description: |
  Enrich person and company pages from external sources.
  Crustdata, Happenstance, Exa, Captain (Pitchbook). Validation rules enforced.
---

# Enrich Skill

## Sources

| Source | Best for | Auth |
|--------|----------|------|
| Crustdata | LinkedIn profile data (90+ fields) | API key |
| Happenstance | Career history, network search | Credits |
| Exa | Web search, articles, mentions | API key |
| Captain/Pitchbook | Company financials, deals, investors | API key |

## Person enrichment workflow

1. Find LinkedIn URL (frontmatter, contacts, or Happenstance search)
2. Hit Crustdata: `GET /screener/person/enrich?linkedin_profile_url=...`
   - Auth: `Token` (NOT Bearer!)
3. Validate before writing:
   - Connection count < 20 → likely wrong person. Save raw_data with flag, don't update page.
   - Name mismatch (different last name) → skip.
4. Store raw: `gbrain call brain_raw '{"slug":"people/name","source":"crustdata","data":{...}}'`
5. Distill to page: update compiled_truth with location, title, company, career arc, top skills.
   DO NOT dump full 90-field data into the page.

## Batch rules

- Checkpoint every 20 items
- Exponential backoff on 429s: 10s → 20s → 40s → ... → 5min cap
- Dry-run: `--dry-run` shows what would be enriched
- Never re-enrich already-enriched pages (check raw_data table first)
```

---

### skills/briefing/SKILL.md

```markdown
---
name: gbrain-briefing
description: |
  Compile a daily briefing from brain state plus real-time sources.
  What changed, what's coming, who's waiting, what needs attention.
---

# Briefing Skill

## Briefing structure

1. **Calendar** — Today's meetings. For each: pull brain pages for participants using progressive retrieval (`--depth summary`).
2. **Active deals** — `gbrain list --type deal --tag active`
3. **Commitments due** — `gbrain list --type commitment --tag open` filtered to due within 7 days. Flag overdue.
4. **Action items** — `gbrain list --type action_item --tag open` sorted by priority + due date.
5. **What shifted overnight** — Query assertions where `valid_until` was set in the last 24h (superseded facts, shifted commitments, reversed decisions). This is the "overnight shift report."
6. **Open threads** — Scan pages for time-sensitive Open Threads items.
7. **Unresolved contradictions** — `gbrain check --all` → surface any unresolved items from the `contradictions` table.
8. **Recent brain changes** — `gbrain list --sort updated` filtered to last 24h.
9. **People in play** — Person pages updated in last 7 days with score ≥ 3.
10. **Stale alerts** — Pages flagged by maintain skill (including temporal link issues).

## Output

Write briefing to `sources/briefing-YYYY-MM-DD`.
Return formatted markdown suitable for Telegram delivery.
Alert-worthy items are handled by the alerts skill, not the briefing.
```

---

### skills/alerts/SKILL.md

```markdown
---
name: gbrain-alerts
description: |
  Interrupt-driven notification thresholds. Defines what warrants an
  immediate push notification vs. waiting for the next scheduled briefing.
  Pairs with the briefing skill — briefing is pull, alerts are push.
---

# Alerts Skill

## Alert tiers

### Immediate alert (push via Telegram within minutes)
- High-priority entity first appears (e.g. RIFT first post, major competitor launch)
- Extreme price moves (BTC +/-15% in an hour, portfolio company token event)
- Legislative/regulatory events (CLARITY Act floor vote, SEC filing deadline)
- Commitment overdue by 24h+ with no update
- Contradiction detected on a page updated in last 24h (active deal or person)

### Next briefing (include in next scheduled digest)
- New followers or engagement on our posts
- Non-urgent replies to monitored threads
- Routine enrichment completions
- Knowledge gaps detected by `brain_gap`
- Stale pages flagged by maintain skill

### Silent log (no notification, recorded in timeline)
- Routine social media engagement
- Minor price moves within normal range
- Link saves and bookmarks
- Background enrichment data updates

## How it works

1. Events arrive via agent monitoring (cron, webhooks, MCP tools).
2. Agent classifies event against the tier definitions above.
3. **Immediate:** Format for Telegram delivery (short, no markdown tables, action-oriented). Push immediately.
4. **Next briefing:** Write to `sources/alerts-queue-YYYY-MM-DD` for the briefing skill to pick up.
5. **Silent:** Write to relevant page timeline via `gbrain timeline-add`. No notification.

## Customisation

Alert thresholds are in this skill file, not in the binary. Adjust tiers by editing this file.
The agent should surface its classification reasoning if borderline ("I classified this as next-briefing because...").
```

---

### skills/research/SKILL.md

```markdown
---
name: gbrain-research
description: |
  Resolve knowledge gaps logged by brain_gap. Run on schedule or on demand.
  Respects sensitivity classification: internal gaps resolved from existing
  brain only, redacted gaps anonymised before external queries, external gaps
  may use web search and enrichment APIs. Default sensitivity is internal.
---

# Research Skill

## Workflow

1. **Read gap log.** `gbrain gaps` — list unresolved knowledge gaps.

2. **Prioritise.** Rank by:
   - Age (older gaps first — they've been unresolved longest)
   - Context (gaps from high-priority queries rank higher)
   - Frequency (same query_text appearing multiple times = high demand)

3. **Check sensitivity classification and approval before researching.**

   | Sensitivity | Approval required? | Allowed research methods |
   |-------------|-------------------|------------------------|
   | `internal` (default) | No | Search existing brain pages only (`brain_query`, `brain_search`). No network calls. If the brain can't answer it, leave the gap unresolved and note "requires external research — escalate sensitivity via `brain_gap_approve` to proceed." |
   | `redacted` | Yes (`brain_gap_approve` with `approved_by`, `redacted_query`) | External search permitted using ONLY the `redacted_query` stored in the approval record (entity names, deal terms, dollar amounts stripped). **Never send the original `query_text` externally.** If no `redacted_query` exists in the approval record, refuse and ask for one. |
   | `external` | Yes (`brain_gap_approve` with `approved_by`) | External search permitted with the original query text. Use only for non-sensitive topics (public companies, open-source projects, general concepts). |

   **Hard rule:** The research skill MUST verify that `approved_by IS NOT NULL AND approved_at IS NOT NULL` before any external call. If the approval record is missing (which shouldn't happen due to the CHECK constraint, but defense in depth), treat as `internal`.

4. **Research each gap (per sensitivity rules above).**
   - `internal`: re-query brain with alternate phrasing, check backlinks, scan related pages
   - `redacted`/`external`: Web search (Exa, Brave) for the (redacted or original) query text
   - If entity-related: check enrichment APIs (Crustdata, Happenstance) — `external` only
   - If topic-related: search for recent articles, papers, threads
   - Compile findings into a draft page or update to existing page

4. **Ingest findings.** Follow `skills/ingest/SKILL.md` — standard four-tier consolidation.
   - New entity discovered → create page via `gbrain put`
   - Existing entity enriched → update via `brain_ingest` with `expected_version`
   - Topic research → create `concepts/` or `sources/` page

5. **Resolve gap.** After successful ingest:
   - The system marks the gap resolved with the slug of the page that filled it
   - If research yields nothing useful, add a note to the gap context and leave unresolved
   - Re-check after 7 days (topics evolve, new sources appear)

6. **Report.** Write research summary to `sources/research-YYYY-MM-DD` for audit trail.

## When to run

- **Scheduled:** Daily, after the morning briefing compile. Process top 5 unresolved gaps.
- **On demand:** Agent notices a gap during conversation and wants to fill it immediately.
- **Batch:** Weekly deep run — process all gaps older than 3 days.

## Quality rules

- Never fabricate. If research yields no results, say so.
- Cite every source using the standard attribution format (see ingest skill).
- Prefer primary sources over social media summaries.
- If a gap is genuinely unanswerable (e.g. "what is X planning internally?"), mark it as `context: "unanswerable — requires insider access"` and leave unresolved.
```

---

### skills/upgrade/SKILL.md

```markdown
---
name: gbrain-upgrade
description: |
  Agent-guided upgrade path for the gbrain binary and skills.
  Inspired by Garry Tan's v0.8.0 "just ask your agent to upgrade" pattern.
  The skill file IS the upgrade guide — the binary handles mechanics.
---

# Upgrade Skill

## Pre-upgrade checklist

1. **Check current version:** `gbrain version`
2. **Record the resolved DB path:** `echo "${GBRAIN_DB:-./brain.db}"` — needed for rollback.
3. **Stop MCP server if running:** `pgrep -f "gbrain serve" && echo "STOP: kill gbrain serve before upgrading"`
4. **Backup:** `gbrain compact` then manual backup if desired (the binary creates its own WAL-safe backup during migration)
5. **Validate current state:** `gbrain validate --all` — fix any issues before upgrading
6. **Check for new version:** query GitHub releases API for latest version tag
7. **Record current binary path:** `which gbrain` — needed for rollback

## Upgrade steps

1. **Download new binary to a staging path (NOT directly to the install location):**
   ```bash
   TARGET_VERSION="v0.2.0"  # always pin an explicit version, never use 'latest' unverified
   PLATFORM="$(uname -s | tr A-Z a-z)-$(uname -m)"
   STAGING="/tmp/gbrain-${TARGET_VERSION}"

   # Download binary and checksum file
   curl -fsSL "https://github.com/[owner]/gbrain/releases/download/${TARGET_VERSION}/gbrain-${PLATFORM}" \
     -o "${STAGING}"
   curl -fsSL "https://github.com/[owner]/gbrain/releases/download/${TARGET_VERSION}/gbrain-${PLATFORM}.sha256" \
     -o "${STAGING}.sha256"
   ```

2. **Verify integrity before installing:**
   ```bash
   # Verify SHA-256 checksum matches published hash
   echo "$(cat ${STAGING}.sha256)  ${STAGING}" | shasum -a 256 --check
   # If check fails: STOP. Do not install. Report the mismatch.
   ```

3. **Preserve the current binary for rollback, then install:**
   ```bash
   INSTALL_PATH="$(which gbrain)"
   cp "${INSTALL_PATH}" "${INSTALL_PATH}.rollback"
   cp "${STAGING}" "${INSTALL_PATH}" && chmod +x "${INSTALL_PATH}"
   ```

4. **Run migrations:** `gbrain version` — the binary auto-migrates on startup if needed.
   It creates a WAL-safe backup via `VACUUM INTO` to `brain.db.backup-v{N}` before any schema migration.
   Migration does not proceed until backup succeeds.

5. **Validate post-migration:**
   - `gbrain validate --all` — all integrity checks pass
   - `gbrain stats` — page counts match pre-upgrade
   - `gbrain embed --stale` — re-embed any pages affected by model changes

6. **Update skills:** Pull latest `skills/` from the repo. External skill files
   in the working directory override embedded defaults.
   `gbrain skills doctor` — verify resolution order and content hashes.

7. **Verify round-trip:** `gbrain import --validate-only` if upgrading from a version
   with schema changes. Confirms no data loss.

8. **Clean up:** Remove staging file and rollback binary if everything passed.
   ```bash
   rm -f "${STAGING}" "${STAGING}.sha256"
   # Keep rollback binary for 7 days, then clean: rm "${INSTALL_PATH}.rollback"
   ```

9. **Report:** Tell the user what changed — version number, any schema migrations run,
   any skills updated, any action required.

## Rollback

If anything goes wrong:

1. **Stop all clients.** Ensure no `gbrain serve` process is running. Check:
   `pgrep -f "gbrain serve"` — kill any running MCP servers before restoring.

2. **Restore prior binary:**
   `cp "$(which gbrain).rollback" "$(which gbrain)"`

3. **Restore pre-migration DB backup (WAL-safe):**
   The backup file is the resolved DB path — respect `--db`/`GBRAIN_DB` if set.
   ```bash
   # Resolve the actual DB path (same logic as the binary uses)
   DB_PATH="${GBRAIN_DB:-./brain.db}"

   # Delete WAL sidecars — they contain post-migration state that would
   # replay into the restored backup and corrupt the rollback.
   rm -f "${DB_PATH}-wal" "${DB_PATH}-shm"

   # Restore the backup over the actual DB path
   cp "${DB_PATH}.backup-v{N}" "${DB_PATH}"
   ```
   **Critical:** You MUST delete `-wal` and `-shm` before restoring. Without this,
   SQLite will replay the WAL on next open, re-applying the migration you're trying to undo.

4. **Verify:** `gbrain version` — should show the pre-upgrade version.
   `gbrain validate --all` — confirm DB integrity.

5. Report what failed for debugging.

## CI release requirements

Every GitHub release MUST publish:
- Platform binaries: `gbrain-linux-x86_64`, `gbrain-darwin-arm64`, etc.
- SHA-256 checksums: `gbrain-linux-x86_64.sha256`, `gbrain-darwin-arm64.sha256`, etc.
- Each `.sha256` file contains the hex digest only (no filename), one line.
- The release workflow generates checksums in CI, not locally, to prevent tampering.
```

---

## Repository Structure

```
gbrain/
├── README.md               # Project overview + quick start
├── CLAUDE.md               # Claude Code session instructions
├── AGENTS.md               # Generic agent session instructions
├── LICENSE                 # MIT
├── Cargo.toml
├── Cargo.lock
│
├── bin/                    # Compiled binaries (gitignored, built in CI)
│   ├── gbrain-darwin-arm64
│   ├── gbrain-darwin-x86_64
│   └── gbrain-linux-x86_64
│
├── src/
│   ├── main.rs             # Entry point: arg parsing + command dispatch (clap)
│   ├── commands/
│   │   ├── mod.rs
│   │   ├── get.rs
│   │   ├── put.rs
│   │   ├── search.rs
│   │   ├── query.rs
│   │   ├── ingest.rs
│   │   ├── link.rs
│   │   ├── tags.rs
│   │   ├── timeline.rs
│   │   ├── list.rs
│   │   ├── stats.rs
│   │   ├── export.rs
│   │   ├── import.rs
│   │   ├── embed.rs
│   │   ├── graph.rs        # N-hop neighborhood traversal
│   │   ├── gaps.rs         # Knowledge gap list/management
│   │   ├── serve.rs
│   │   ├── call.rs
│   │   ├── init.rs
│   │   ├── config.rs
│   │   └── version.rs
│   ├── core/
│   │   ├── mod.rs
│   │   ├── db.rs           # Database: open(), schema init, WAL, sqlite-vec load
│   │   ├── fts.rs          # FTS5: search_fts(query, wing_filter) → ranked results
│   │   ├── inference.rs    # candle init, embed(text), search_vec(query, k, wing_filter)
│   │   ├── search.rs       # hybrid_search(query): SMS + palace filter + FTS5 + vec + set-union merge
│   │   ├── progressive.rs  # progressive_retrieve(results, token_budget, depth) → expanded results
│   │   ├── novelty.rs      # check_novelty(content, page): Jaccard + cosine dedup
│   │   ├── assertions.rs   # heuristic contradiction detection via assertions table
│   │   ├── graph.rs        # neighborhood_graph(slug, depth): N-hop BFS over links table
│   │   ├── gaps.rs         # knowledge gap detection and resolution tracking
│   │   ├── chunking.rs     # temporal sub-chunking: truth sections + timeline entries
│   │   ├── palace.rs       # derive_wing(slug), derive_room(content), classify_intent(query)
│   │   ├── markdown.rs     # parse_frontmatter(), split_content(), extract_summary(), render_page()
│   │   ├── links.rs        # extract_links(), resolve_slug(), temporal validity
│   │   ├── migrate.rs      # import_dir(), export_dir(), validate_roundtrip()
│   │   └── types.rs        # Page, Link, Tag, TimelineEntry, SearchResult, Contradiction, KnowledgeGap, etc.
│   ├── mcp/
│   │   ├── mod.rs
│   │   └── server.rs       # MCP stdio server: tool definitions + handlers
│   └── schema.sql          # DDL (embedded in db.rs via include_str!, also standalone)
│
├── skills/
│   ├── ingest/SKILL.md
│   ├── query/SKILL.md
│   ├── maintain/SKILL.md
│   ├── enrich/SKILL.md
│   ├── briefing/SKILL.md
│   ├── alerts/SKILL.md
│   ├── research/SKILL.md
│   └── upgrade/SKILL.md
│
├── benchmarks/
│   ├── longmemeval.rs      # LongMemEval multi-session memory (R@5 ≥ 85%)
│   ├── locomo.rs           # LoCoMo conversational memory (F1 regression)
│   ├── beir_subset.rs      # BEIR retrieval regression (nDCG@10)
│   ├── ragas_eval.rs       # Ragas answer quality (context_precision, recall)
│   └── datasets/           # gitignored, downloaded by prep script
│
├── tests/
│   ├── roundtrip_semantic.rs # import → normalized export → semantic validate (MUST pass)
│   ├── roundtrip_raw.rs     # import → raw export by import_id → byte-exact diff (MUST pass)
│   ├── fts.rs              # FTS5 search correctness
│   ├── inference.rs        # candle embedding + vector search quality
│   ├── links.rs            # link extraction + temporal interval enforcement
│   ├── mcp.rs              # MCP tool call correctness
│   └── fixtures/
│       ├── person.md
│       ├── company.md
│       └── .raw/person.json
│
└── .github/
    └── workflows/
        ├── ci.yml          # cargo test + cargo build --release
        └── release.yml     # cross-compile matrix → GitHub release assets
```

### CLAUDE.md (embedded)

```markdown
# GigaBrain

Personal knowledge brain. SQLite + FTS5 + local vector embeddings. One binary.

## Architecture

Thin CLI (src/main.rs) dispatches to commands (src/commands/).
Core library (src/core/) handles DB, search, embeddings, markdown parsing.
Skills (skills/) are fat markdown files - all intelligence lives there.

## Key files

- `src/core/db.rs`         — rusqlite connection, schema init, WAL, sqlite-vec load
- `src/core/fts.rs`        — FTS5 search: `search_fts(query, wing_filter, db)` → ranked results
- `src/core/inference.rs`  — candle model init, `embed(text)`, `search_vec(query, k, wing_filter, db)`
- `src/core/search.rs`     — `hybrid_search(query, db)`: SMS + palace filter + FTS5 + vec + set-union merge
- `src/core/progressive.rs`— `progressive_retrieve(results, budget, depth)`: token-budget expansion
- `src/core/novelty.rs`    — `check_novelty(content, page, db)`: Jaccard + cosine dedup
- `src/core/assertions.rs`  — `check_assertions(slug, db)`: heuristic contradiction detection via SQL
- `src/core/graph.rs`      — `neighborhood_graph(slug, depth, db)`: N-hop BFS over links table
- `src/core/gaps.rs`       — `log_gap(query, context, score, db)`, `list_gaps(db)`: knowledge gap tracking
- `src/core/chunking.rs`   — temporal sub-chunking: truth sections + individual timeline entries
- `src/core/palace.rs`     — `derive_wing(slug)`, `derive_room(content)`, `classify_intent(query)`
- `src/core/markdown.rs`   — parse frontmatter, split compiled_truth/timeline, extract_summary, render
- `src/mcp/server.rs`      — MCP stdio server exposing all tools (including brain_graph, brain_gap, brain_check)

## Build

```bash
cargo build --release
# Output: target/release/gbrain (airgapped channel — default)

# Cross-compile
cargo install cross
cross build --release --target aarch64-apple-darwin
cross build --release --target x86_64-unknown-linux-musl
```

## Test

```bash
cargo test
# Key tests: tests/roundtrip_semantic.rs (normalized export validate) + tests/roundtrip_raw.rs (byte-exact diff by import_id).
```

## Embedding model

BGE-small-en-v1.5 via candle (pure Rust). 384 dimensions. `v0.9.1` ships two
compile-time channels:

- `embedded-model` — airgapped channel (default): `include_bytes!()` model assets embedded at build time
- `online-model` — online channel: downloads/caches BGE-small on first semantic use

## Skills

Read skills/ before doing brain operations. They contain all workflow logic.
```

---

## Build and Release

### Cargo.toml (core dependencies)

```toml
[package]
name = "gbrain"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "gbrain"
path = "src/main.rs"

[dependencies]
# CLI
clap = { version = "4", features = ["derive"] }

# Database - bundled = SQLite compiled into binary, no system dep
rusqlite = { version = "0.31", features = ["bundled"] }

# sqlite-vec - vector search as SQLite extension, statically linked
sqlite-vec = "0.1"   # or inline via rusqlite loadable extension

# Embeddings - pure Rust ML, no ONNX runtime, true static linking
candle-core = "0.8"
candle-nn = "0.8"
candle-transformers = "0.8"
safetensors = "0.4"
tokenizers = "0.20"

# MCP server
rmcp = "0.1"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# Markdown
pulldown-cmark = "0.11"

# Regex
regex = "1"

# Async runtime (for MCP server)
tokio = { version = "1", features = ["full"] }

# Error handling
anyhow = "1"
thiserror = "1"

[features]
default = ["bundled", "embedded-model"]
bundled = ["rusqlite/bundled"]
embedded-model = []                      # airgapped channel (default): include_bytes!() model weights into binary
online-model = ["dep:reqwest"]           # online channel: download/cache BGE-small on first use
```

### Build commands

```bash
# Development
cargo build

# Release (optimized)
cargo build --release

# Release (airgapped channel — default; embeds BGE-small for offline use)
cargo build --release

# Release — online channel (downloads/caches BGE-small on first semantic use)
cargo build --release --no-default-features --features bundled,online-model

# Run tests
cargo test

# Cross-compile — candle enables true musl static linking
cross build --release --target aarch64-apple-darwin          # macOS ARM
cross build --release --target x86_64-apple-darwin           # macOS Intel
cross build --release --target x86_64-unknown-linux-musl     # Linux x86_64 (static)
cross build --release --target aarch64-unknown-linux-musl    # Linux ARM64 (static)

# Install locally
cargo install --path .
```

### CI/CD (GitHub Actions)

```yaml
# .github/workflows/release.yml
strategy:
  matrix:
    include:
      - target: aarch64-apple-darwin
        os: macos-latest
      - target: x86_64-apple-darwin
        os: macos-latest
      - target: x86_64-unknown-linux-musl    # static binary — no glibc dependency
        os: ubuntu-latest
      - target: aarch64-unknown-linux-musl   # static binary — no glibc dependency
        os: ubuntu-latest

# Post-build verification (release gate):
# - run: file target/${{ matrix.target }}/release/gbrain
# - run: ldd target/${{ matrix.target }}/release/gbrain 2>&1 | grep -q "not a dynamic" || exit 1  # Linux
# - run: otool -L target/${{ matrix.target }}/release/gbrain | grep -qv "\.dylib" || true           # macOS (system libs OK)

# Post-build: generate SHA-256 checksums for integrity verification
# - run: shasum -a 256 target/${{ matrix.target }}/release/gbrain | awk '{print $1}' > gbrain-${{ matrix.target }}.sha256
# Publish both binary and .sha256 file as release assets
```

Release artifacts published to GitHub Releases on tag push. Each release includes platform binaries and SHA-256 checksum files (generated in CI, not locally). Install via the upgrade skill (see `skills/upgrade/SKILL.md`) which handles version pinning and checksum verification. Quick install for first-time users:

```bash
VERSION="v0.1.0"
PLATFORM="darwin-arm64"
curl -fsSL "https://github.com/[owner]/gbrain/releases/download/${VERSION}/gbrain-${PLATFORM}" -o /tmp/gbrain
curl -fsSL "https://github.com/[owner]/gbrain/releases/download/${VERSION}/gbrain-${PLATFORM}.sha256" -o /tmp/gbrain.sha256
echo "$(cat /tmp/gbrain.sha256)  /tmp/gbrain" | shasum -a 256 --check
cp /tmp/gbrain /usr/local/bin/gbrain && chmod +x /usr/local/bin/gbrain
```

---

## Phased Delivery

The spec describes the full vision. Build in phases — earn the right to add complexity.

### Phase 1: Core (ship this first)

The smallest thing that proves the value proposition:

- `gbrain init`, `get`, `put`, `list`, `stats`
- `pages` table with `version`, split timestamps
- `knowledge_gaps` table (schema only — tools in Phase 3)
- `original` page type (in type mapping, template in ingest skill)
- FTS5 search (`gbrain search`)
- Candle embeddings + sqlite-vec (`gbrain embed`, `gbrain query`)
- SMS exact-match short-circuit
- Basic set-union hybrid search (no palace filtering yet)
- `gbrain import` / `gbrain export` (normalized only)
- `gbrain compact` (WAL checkpoint)
- MCP server with `brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`
- Transactional ingest with idempotency
- Embedded default skills (including source attribution format and filing disambiguation in ingest skill)
- Round-trip test, corpus-reality tests, static binary verification

**Ship gate:** imports a real corpus, retrieves correctly, exports faithfully, binary is static.

### Phase 2: Intelligence Layer

- Temporal links (`brain_link`, `brain_link_close`, backlinks with `--temporal`)
- Graph neighborhood traversal (`brain_graph`, `gbrain graph`)
- Assertions with provenance
- Contradiction detection (`gbrain check`)
- Progressive retrieval with token budgets
- Novelty checking (Tiers 2-4 gating)
- Work-context entities (decision, commitment, action_item)
- Palace wing filtering (validate with benchmarks before committing to room-level)
- Full MCP write surface with version checks
- Optional person template enrichment sections (What They Believe, Hobby Horses, Trajectory, Network) for Tier 1 contacts

### Phase 3: Polish + Benchmarks

- Briefing skill with "what shifted" report
- Alerts skill (interrupt-driven notifications vs scheduled briefings)
- Research skill (knowledge gap resolution)
- Knowledge gap detection (`brain_gap`, `brain_gaps` MCP tools, `gbrain gaps` CLI)
- Upgrade skill (agent-guided binary + skill updates)
- Enrichment skill
- LongMemEval, LoCoMo, BEIR, Ragas benchmarks
- `gbrain skills doctor`
- `gbrain validate --all` integrity checker
- `--json` output on all commands
- `pipe` mode
- CI/CD release pipeline with all gates

### Deliberate Deferrals

- **First-class chunks table:** The current `page_embeddings` table serves as both chunk metadata and embedding join table. This is intentionally not split into a separate `chunks` table for v1 — the enriched columns (content_hash, token_count, heading_path) are sufficient. If progressive retrieval, re-embedding, or chunk lifecycle management becomes painful at scale, promote chunks to their own table in a future version. This is a deliberate deferral, not an oversight.
- **Room-level palace filtering:** Deferred until benchmarks on real corpus prove it helps. Wing-only in v1.
- **LLM-assisted contradiction detection:** Binary stays dumb. Cross-page semantic reasoning happens via the maintain skill.
- **WASM compilation:** Rust has strong WASM support. PGLite proves browser portability is viable. If we ever need gbrain in a browser or serverless context, WASM is the path. Not a current priority.
- **Overnight consolidation cycle:** Garry's DREAMS.md pattern (overnight entity sweep, enrichment, citation fixing) is powerful but is agent configuration, not gbrain binary. Could be a skill added post-v1.

---

## Implementation Roadmap

### Week 1 - Foundation

- [ ] `cargo init` + `Cargo.toml` with dependencies
- [ ] `src/core/types.rs` — Page (with summary, wing, room), Link (with relationship, valid_from, valid_until), Tag, TimelineEntry, SearchResult, Contradiction, KnowledgeGap structs
- [ ] `src/core/db.rs` — rusqlite connection, schema DDL via `include_str!`, WAL, sqlite-vec load
- [ ] `src/core/markdown.rs` — `parse_frontmatter()`, `split_content()`, `extract_summary()`, `render_page()`
- [ ] `src/core/palace.rs` — `derive_wing(slug)`, `derive_room(content)` — auto-derive palace metadata from slug structure and section headers
- [ ] Unit tests for markdown parsing (round-trip frontmatter, compiled_truth/timeline split, summary extraction)
- [ ] `src/main.rs` — clap CLI scaffold, command dispatch
- [ ] `src/commands/init.rs` — create new brain.db (v4 schema with palace + temporal + contradictions + knowledge_gaps)
- [ ] `src/commands/get.rs` — read page by slug
- [ ] `src/commands/put.rs` — write/update page (auto-extract summary, auto-derive wing/room)
- [ ] `src/commands/list.rs` — list pages with filters (including `--wing`)
- [ ] `src/commands/stats.rs` — brain statistics (including contradiction count)
- [ ] `src/commands/tags.rs` + tag/untag — tag operations
- [ ] `src/commands/link.rs` + unlink + backlinks — with `--relationship`, `--valid-from`, `--valid-until`, `--temporal` flags

**Checkpoint:** `gbrain init`, `gbrain put` (with auto-summary/palace), `gbrain get`, `gbrain list`, `gbrain stats`, `gbrain link --relationship works_at --valid-from 2024-01-15` all working.

### Week 2 - Search + Progressive Retrieval

- [ ] `src/core/fts.rs` — FTS5 search logic, BM25 scoring, palace wing/room filter
- [ ] `src/commands/search.rs` — full-text search command with `--wing` filter
- [ ] `src/core/inference.rs` — candle model init, `embed(text)`, batch embedding, palace-filtered vector search
- [ ] `src/core/chunking.rs` — temporal sub-chunking: truth sections at `##` boundaries, timeline entries individually
- [ ] `src/commands/embed.rs` — generate/refresh embeddings, `--all` and `--stale` flags
- [ ] `src/core/search.rs` — hybrid search: SMS + palace pre-filter + FTS5 + vec0 fan-out + **set-union merge** (default) with RRF fallback via config flag
- [ ] `src/core/progressive.rs` — `progressive_retrieve(results, token_budget, depth)`: summary → section → full expansion gated by token budget
- [ ] `src/commands/query.rs` — semantic query command with `--depth`, `--token-budget`, `--wing` flags
- [ ] Unit tests for set-union vs RRF merge correctness, progressive retrieval token counting

**Checkpoint:** `gbrain search "River AI"` and `gbrain query "who knows Jensen Huang?" --depth auto --token-budget 4000` both return ranked results with progressive expansion. Set-union merge is default, RRF switchable via `gbrain config set search_merge_strategy rrf`.

### Week 3 - Ingest + MCP + Integrity

- [ ] `src/core/links.rs` — extract wiki-links from markdown, resolve slugs, temporal validity management
- [ ] `src/core/novelty.rs` — `check_novelty()`: Jaccard similarity + cosine similarity dedup
- [ ] `src/core/migrate.rs` — `import_dir()`: recursive scan, parse, transaction insert, validate (populate wing/room/summary during import)
- [ ] `src/commands/import.rs` — full migration command (derive palace metadata for all imported pages)
- [ ] `src/commands/export.rs` — reconstruct markdown directory
- [ ] `tests/roundtrip_semantic.rs` — import → normalized export → semantic validate (primary correctness test)
- [ ] `tests/roundtrip_raw.rs` — import → `export --raw --import-id` → byte-exact diff
- [ ] `src/commands/timeline.rs` — read timeline entries
- [ ] `src/commands/ingest.rs` — source document ingestion command (with novelty check, `--force` override)
- [ ] `src/core/assertions.rs` — heuristic contradiction detection via assertions table: link vs assertion, temporal staleness
- [ ] `src/core/graph.rs` — `neighborhood_graph(slug, depth, db)`: N-hop BFS over links table with temporal filtering
- [ ] `src/core/gaps.rs` — `log_gap()`, `list_gaps()`, `resolve_gap()`: knowledge gap tracking
- [ ] `src/commands/check.rs` — `gbrain check [SLUG] --all --type temporal|cross_page|stale`
- [ ] `src/commands/graph.rs` — `gbrain graph <SLUG> --depth N --temporal current|historical|all`
- [ ] `src/commands/gaps.rs` — `gbrain gaps --limit N --resolved`
- [ ] `src/mcp/server.rs` — MCP stdio server with all tools (including `brain_graph`, `brain_gap`, `brain_gaps`, `brain_check`, progressive `brain_query`, temporal `brain_backlinks`)
- [ ] `src/commands/serve.rs` — start MCP server

**Checkpoint:** `gbrain import /data/brain/` completes with zero diff (palace metadata populated). `gbrain ingest` rejects duplicates (Jaccard > 0.85). `gbrain check --all` detects temporal contradictions. `gbrain graph people/pedro-franceschi --depth 2` returns the 2-hop neighborhood as JSON. `gbrain serve` connects to Claude Code with all 20 MCP tools.

### Week 4 - Polish + Release

- [ ] `src/commands/call.rs` — raw tool call (GL pattern)
- [ ] `src/commands/config.rs` — config get/set/list (including `search_merge_strategy`, `default_token_budget`)
- [ ] `src/commands/version.rs`
- [ ] `--tools-json` output (MCP tool discovery)
- [ ] `pipe` mode (JSONL streaming)
- [ ] `--json` output flag on all commands
- [ ] Full test suite: fts.rs, inference.rs, links.rs, mcp.rs, novelty.rs, assertions.rs, progressive.rs, palace.rs, chunking.rs, graph.rs, gaps.rs
- [ ] `skills/` markdown files finalized (four-tier consolidation + source attribution + filing disambiguation in ingest, palace-aware query, contradiction-aware maintain/briefing, alerts, research, upgrade)
- [ ] `CLAUDE.md`, `AGENTS.md`, `README.md`
- [ ] CI/CD: `cargo test` + cross-compile matrix → GitHub Releases
- [ ] `gbrain import --validate-only` dry-run mode
- [ ] `gbrain embed --stale` incremental re-embedding

**Checkpoint:** Full test suite passes. Cross-compiled binaries on GitHub Releases. Round-trip validated against production brain. Contradiction detection runs clean on imported data.

### Week 5 - Release Gates (corpus-reality first, leaderboards second)

- [ ] `benchmarks/` directory with dataset prep scripts, pinned dataset versions, and evaluation harness

#### Offline CI gates (mandatory, no API keys, fully local)

- [ ] **BEIR (retrieval subset)** — Retrieval regression gate. Runs entirely offline.
  - Dataset: `https://github.com/beir-cellar/beir` — pin to specific commit hash in `benchmarks/datasets.lock`
  - Subsets: NQ + FiQA (closest to personal KB workload)
  - Metrics: nDCG@10. Target: no regression > 2% between releases
  - Harness: custom Rust binary in `benchmarks/beir_eval.rs` — embeds corpus, runs queries, compares to baseline
- [ ] **Corpus-reality tests** (see below) — fully local, no LLM judge

#### API-dependent evaluation (optional, run manually or in separate CI job)

- [ ] **LongMemEval** — Multi-session memory benchmark.
  - Dataset: `https://github.com/xiaowu0162/LongMemEval` — pin commit in `benchmarks/datasets.lock`
  - Harness: official `evaluate_qa.py` (requires `OPENAI_API_KEY` for LLM judge)
  - Metrics: R@5 (Recall at 5). Target: ≥ 85%
  - Adapter: `benchmarks/longmemeval_adapter.py` converts gbrain queries to LongMemEval format
- [ ] **LoCoMo** — Long conversational memory benchmark.
  - Dataset: `https://github.com/snap-research/locomo` — pin commit in `benchmarks/datasets.lock`
  - Harness: official evaluation scripts (API-dependent)
  - Metrics: F1 on single-iteration retrieval. Target: ≥ +30% over naive FTS5 baseline
- [ ] **Ragas** — Answer and context quality metrics for progressive retrieval.
  - Framework: `https://docs.ragas.io/` — pin version in `benchmarks/requirements.txt`
  - Metrics: context_precision, context_recall, faithfulness
  - Note: Ragas requires an LLM judge. Use local Ollama model or API key. Results are advisory, not a release gate.
- [ ] **Corpus-reality tests** — The benchmarks that actually matter for users.
  - Import a messy real markdown corpus (7K+ files) → verify zero page loss
  - Retrieve a known entity by name → correct page in top 1 (SMS test)
  - Retrieve a known fact from timeline → correct entry within top 5 (temporal sub-chunk test)
  - Ingest the same source twice → no duplicate timeline entries, no duplicate assertions
  - Ingest two conflicting sources → contradiction detected
  - Normalized export → reimport → normalized export → semantic diff = zero (idempotent round-trip)
  - Run 100 queries against imported corpus → measure p50/p95 latency (target: p95 < 250ms)
- [ ] **Concurrency and crash-safety stress tests** — CI gate for safety invariants.
  - Parallel writers: 4 threads calling `brain_put` on the same slug with stale `expected_version` → all but one must get ConflictError, zero data corruption
  - Overlapping ingest: 2 threads ingesting the same source simultaneously → exactly one succeeds (idempotency key), zero duplicate timeline/assertion rows
  - Kill-before-commit: start ingest, `kill -9` before COMMIT, retry → clean state, no partial mutations, ingest_log has no stale rows
  - WAL compact under load: run `gbrain compact` while a reader holds an open query → compact succeeds, reader gets consistent snapshot
  - Invariants: monotonic `pages.version`, no lost timeline rows, no duplicate side-table rows across all stress scenarios
- [ ] **Embedding model migration correctness** — CI gate for vec search contract.
  - Embed corpus with model A, run 20 queries, record top-5 results per query
  - Register model B (same dimensions, different weights — or re-embed with same model under a new name)
  - Re-embed all pages under model B, flip active flag
  - Run same 20 queries → verify: (a) all results come from model B's vec table, (b) no vec_rowid/id confusion, (c) no stale model A results leak through
  - Rollback: flip active flag back to model A → verify original top-5 results return identically
  - Gate: zero cross-model contamination across all queries
- [ ] **Round-trip integrity** — CI gate (two separate tests).
  - **Semantic:** Import corpus → normalized export → `gbrain validate` against original. Checks same pages, same frontmatter keys/values, same compiled_truth, same timeline entries. MUST pass.
  - **Byte-exact:** Import corpus → `export --raw --import-id <ID>` → `diff -r` against original source. MUST pass. Only valid for the specific import batch, not after mutations.
- [ ] **Static binary verification** — CI release gate.
  - `ldd` / `file` / `otool` on every release artifact. Reject any binary with dynamic library dependencies.
  - Gate: `file gbrain-linux-x86_64 | grep "statically linked"` must succeed.

**Checkpoint:** All offline CI gates pass: BEIR nDCG regression, corpus-reality tests, concurrency stress tests, round-trip integrity (both semantic and raw), static binary verification. API-dependent benchmarks (LongMemEval R@5 ≥ 85%, LoCoMo F1, Ragas) run manually before major releases. A failing offline gate blocks the release; API-dependent benchmarks are advisory.

**Philosophy:** Corpus-reality beats benchmark theater. The leaderboard benchmarks validate architectural decisions; the corpus tests validate that the tool actually works for its intended user.

---

## Design Decisions

### Why SQLite over Postgres/Qdrant/Chroma

**Postgres:** Better for multi-user writes, replication, row-level security. None of those apply here. This is one person's brain. One writer, many readers. SQLite's sweet spot is exactly this workload.

**Qdrant/Chroma/Pinecone:** External services. Require network. Require containers or API keys. A personal brain shouldn't need a sidecar container or a paid API to do semantic search. sqlite-vec gives native cosine similarity in the same file, same connection, same query.

**The fundamental principle:** `brain.db` is a 500MB file you can `scp`, `rsync`, back up to S3, or carry on a USB stick. No connection strings. No Docker. No managed database.

### Why candle over fastembed (ONNX)

The primary promise of this tool is a "single binary that runs anywhere." `fastembed` relies on the ONNX runtime. Linking `libonnxruntime` statically — especially across `musl` for Linux or various macOS architectures — is fragile and often results in missing shared object errors at runtime.

**candle** is HuggingFace's pure-Rust ML framework. By using candle to load safetensors for BGE-small-en-v1.5, we achieve true 100% static linking with zero C-dependencies (other than SQLite, which is bundled via `rusqlite`).

### Why local embeddings over OpenAI

Garry's spec uses `text-embedding-3-small` (OpenAI API, 1536 dims, $0.02/1M tokens). Reasonable for a server-side tool. The problem: it requires internet access and an API key at embedding time.

**BGE-small-en-v1.5 via candle:**
- Pure Rust inference, no ONNX runtime
- 384 dimensions (4x smaller than OpenAI's 1536-dim - smaller DB, faster search)
- Quality: excellent for personal knowledge base retrieval tasks (competitive with OpenAI small)
- No internet required (model weights embedded in binary by default)
- No API key
- No cost
- Runs in-process

**Trade-off:** Slightly lower quality on some retrieval benchmarks vs OpenAI text-embedding-3-large. Acceptable for a personal knowledge base where recall@10 matters more than recall@1.

**Future:** The `model` column in `page_embeddings` and `embedding_models` registry table allow swapping models. Upgrade path: register new model, run `gbrain embed --all`, flip active flag.

### Why Rust over TypeScript/Bun

Garry's spec is TypeScript/Bun. That's a solid choice for his use case (server, API keys available). Rust is better for this spec because:

1. `rusqlite --features bundled` = SQLite compiled into the binary. No system SQLite version issues.
2. sqlite-vec statically linked = no native extension loading complications
3. candle is pure Rust = in-process ML inference, no ONNX runtime, true musl static linking
4. `cargo cross` = trivial cross-compilation to arm64 + x86_64 macOS and Linux
5. No runtime (no Bun, no Node installed on target machine)
6. Lower memory footprint

Bun's compiled binary is ~10MB. This binary is ~90MB (including model weights). The 80MB difference is the cost of zero runtime dependencies. Worth it for a personal tool deployed on client machines.

### Why RRF over weighted sum

Garry's spec uses `FTS5 score × 0.4 + vector similarity × 0.6`. This requires choosing weights and normalizing scores across two different scoring systems with different distributions.

RRF (Reciprocal Rank Fusion) is more robust: it only uses rank position, not raw scores. No normalization needed. No magic numbers. Works well empirically. Standard in hybrid search literature (SIGIR 2009).

Formula: `RRF(d) = Σ 1/(k + rank(d, r))` where k=60, summed over result sets r.

### Chunk strategy: section-level

- **Per-page:** Too coarse. A 5,000-word person page has many distinct topics. Vector of whole page loses specificity.
- **Per-paragraph:** Too fine. Loses context. Short paragraphs embed poorly.
- **Per-section (`##` headers):** Right balance. Each `## State`, `## Assessment`, `## Timeline` section becomes a chunk. ~200-800 tokens. Good quality, good retrieval precision.
- **Fallback:** Pages without headers chunk at ~500-token boundaries.

### Multiple brains

A different DB file = a different brain. No application-level complexity.

```bash
GBRAIN_DB=/path/to/work.db gbrain stats
GBRAIN_DB=/path/to/personal.db gbrain serve --port 3001
```

### Why set-union over RRF (v2 change)

v1 spec used RRF (Reciprocal Rank Fusion). UNC's AutoResearchClaw pipeline (Apr 2026) tested score-based re-ranking approaches and found they degrade performance — disrupting the semantic ordering dense retrieval already established. Set-union (vector results first in original order, FTS5-only results appended) delivered +44% F1 on LoCoMo in a single iteration. Ablation confirmed the gain. RRF is retained as a config fallback for A/B testing.

### Why exact-match short-circuit (SMS)

Set-union merging starts with vector results, which means searching for "Pedro Franceschi" might not return that person's actual page first if other pages have higher embedding similarity. In a personal brain, searching for a specific name MUST return that entity's page at #1. SMS guarantees title and slug exact matches bypass semantic fuzziness entirely.

### Why palace filtering (v2 addition)

MemPalace's ablation: 60.9% → 94.8% R@5 with wing+room pre-filtering (+34%). Constraining the search space before running expensive vector queries is cheaper and more effective than post-hoc re-ranking. gbrain's palace metadata is auto-derived from slug structure (zero manual effort) with frontmatter override for custom taxonomies.

**Caveat:** The +34% improvement is from MemPalace's synthetic benchmark, not validated on gbrain's corpus. Wing-level filtering ships in v1 as a low-cost bet (auto-derived, zero manual effort). Room-level filtering is deferred until benchmark results on real data confirm it helps. If benchmarks show palace filtering doesn't materially improve retrieval on a personal knowledge corpus, demote it to optional.

### Why progressive retrieval (v2 addition)

OMNIMEM's ablation: removing progressive retrieval = -17% F1 (largest single component). Serving full pages is wasteful when the agent only needs a summary to decide relevance. Token-budget-gated expansion lets the system serve the right amount of content for the context window available.

### Why temporal links (v2 addition)

Knowledge graphs without temporal validity can't distinguish "who works at Brex?" from "who ever worked at Brex?". MemPalace's temporal triples (valid_from/valid_until) enable this distinction. Contradiction detection builds directly on temporal metadata — you can't catch "page says he left, but the link is still active" without it.

### Why selective ingestion (v2 addition)

OMNIMEM's three principles: selective ingestion, multimodal atomic units, progressive retrieval. Jaccard overlap + cosine similarity dedup at ingest time prevents the corpus from accumulating noise. Cleaner corpus = better retrieval precision without any search algorithm changes.

### Why SQLite over PGLite (v4 validation)

Garry Tan's GBrain v0.8.0 (Apr 2026) moved from Supabase to PGLite — an in-process Postgres that runs in a browser or Node.js via WASM. Same principle as our SQLite choice: zero external dependencies, fully local. Three independent teams in the same week (us with SQLite, Garry with PGLite, @ansubkhan with Fastify/SQLite) converged on local embedded databases for agent memory. The architecture is validated.

| | SQLite (gbrain) | PGLite (Garry's GBrain) |
|---|---|---|
| Transport | `cp brain.db` / `scp` / USB stick | Requires WASM runtime to read |
| Runtime | None (statically linked into Rust binary) | Node.js/Bun + WASM |
| True single file | Yes (after `gbrain compact`) | No (PGLite has its own data directory) |
| Vector search | sqlite-vec (statically linked) | pgvector via WASM |
| Cross-compile | `cargo cross` to any musl target | Requires WASM-compatible platform |
| Browser portability | No (desktop binary) | Yes (WASM) |

Both are good choices for their respective stacks. We chose SQLite because `cp brain.db` is the entire backup and migration story. PGLite's browser portability is a future option for us via Rust→WASM compilation, but not a current priority.

### The links table as a graph layer (v4 positioning)

gbrain's `links` table with typed relationships and temporal validity windows is a knowledge graph without Neo4j. Combined with `brain_graph` (N-hop neighborhood traversal), `brain_backlinks` (temporal filtering), and palace-style hierarchy (wing/room), gbrain provides GraphRAG capabilities in a single SQLite file — no separate graph database required.

This is worth calling out because the "separate graph + vector store" problem is a known pain point in the GraphRAG community. Every implementation (nano-graphrag, LangChain GraphRAG, etc.) makes you choose between a graph database and a vector store. gbrain doesn't — wikilink traversal in the links table, vector similarity in sqlite-vec, FTS5 keyword search, all in one file, one connection, one query.

### No file watcher (v1)

The brain is written by AI agents using the CLI or MCP. There's no "file on disk changed" event to watch for. `gbrain import` and `gbrain put` are explicit writes. A `gbrain watch` command that syncs a markdown directory to the DB is a v2 feature.

---

## Schema Versioning and DB Migration

The `config` table stores `version` (currently `'4'`). On startup, the binary compares the DB schema version to its own expected version:

| Scenario | Behavior |
|----------|----------|
| DB version == binary version | Normal operation |
| DB version < binary version | Acquire exclusive write lock, WAL-safe backup, then run entire migration chain + version bump in a single transaction. Rollback on any error. The binary ships with a migration chain (v1→v2, v2→v3, etc.). **No concurrent writers during migration.** |
| DB version > binary version | Refuse to open. Print error: "brain.db is version N, but this binary supports up to version M. Upgrade gbrain." |

Migrations are tested by importing a fixture brain at each prior schema version and verifying post-migration integrity via `gbrain validate --all`.

```rust
fn migrate(conn: &Connection, db_path: &Path) -> Result<()> {
    let db_version: u32 = get_config(conn, "version")?.parse()?;
    let target_version: u32 = SCHEMA_VERSION;  // compiled into binary

    if db_version > target_version {
        bail!("brain.db v{} is newer than this binary (supports up to v{})", db_version, target_version);
    }

    if db_version == target_version {
        return Ok(());  // no migration needed
    }

    // Step 1: Acquire exclusive write lock BEFORE backup.
    // BEGIN IMMEDIATE prevents concurrent writers from committing between
    // backup and migration, ensuring the backup is a consistent snapshot.
    conn.execute("BEGIN IMMEDIATE", [])?;

    // Step 2: WAL-safe backup inside the exclusive lock.
    // VACUUM INTO runs as a read operation on the current snapshot,
    // producing a standalone copy with all WAL content checkpointed.
    let backup_path = db_path.with_extension(format!("db.backup-v{}", db_version));
    if let Err(e) = conn.execute(
        &format!("VACUUM INTO '{}'", backup_path.display()), []
    ) {
        conn.execute("ROLLBACK", [])?;
        bail!("WAL-safe backup failed, migration aborted: {}", e);
    }

    // Step 3: Run entire migration chain inside the same transaction.
    // If ANY step fails, the entire chain rolls back — no partial migration.
    for step in (db_version + 1)..=target_version {
        if let Err(e) = conn.execute_batch(MIGRATIONS[step]) {
            conn.execute("ROLLBACK", [])?;
            bail!("Migration v{}→v{} failed at step {}, rolled back: {}",
                  db_version, target_version, step, e);
        }
    }

    // Step 4: Bump version inside the same transaction.
    set_config(conn, "version", &target_version.to_string())?;

    // Step 5: Commit atomically. All migrations + version bump succeed or none do.
    conn.execute("COMMIT", [])?;
    Ok(())
}
```

**Migration protocol:**

1. **Exclusive lock first:** `BEGIN IMMEDIATE` blocks all concurrent writers before the backup snapshot. No writes can interleave between backup and migration.
2. **WAL-safe backup under lock:** `VACUUM INTO` produces a standalone, fully-checkpointed copy at `{db_path}.backup-v{N}`. Because the exclusive lock is held, the backup is guaranteed to reflect the exact state that migration will operate on.
3. **Atomic migration chain:** All migration steps + version bump run inside the same transaction. If any step fails, `ROLLBACK` restores the DB to its pre-migration state — no partial migration possible.
4. **Rollback safety:** The backup file is a complete, self-contained SQLite database (no WAL sidecars). Restoring it requires replacing the DB file AND deleting any `-wal`/`-shm` sidecars (see rollback procedure below).

**Important:** Migration requires no concurrent clients. The MCP server should not be running during migration. The CLI handles this naturally (single process), but the upgrade skill should verify no `gbrain serve` process is active before proceeding.

---

## Security and Data Sensitivity

brain.db contains sensitive personal intelligence: deal assessments, people evaluations, relationship context, business strategy. The security model:

**At rest:**
- brain.db is a regular file. Protect it with filesystem permissions (`chmod 600`).
- For encryption at rest, use OS-level full-disk encryption (FileVault, LUKS) or SQLite's SEE extension (commercial). gbrain does not implement its own encryption — that's a footgun.
- `gbrain compact` before transport to ensure no WAL sidecar contains unencrypted data.

**In transit:**
- MCP server runs on stdio (local pipes only). No network listener. No remote access by default.
- `scp`/`rsync` for transfer. Use encrypted channels.

**Operational:**
- No telemetry. No analytics. No phone-home. The gbrain binary itself makes zero network calls at runtime.
- Skills are local markdown files. The binary does not exfiltrate data.
- **Network boundary for agent-driven skills:** The enrichment skill (`skills/enrich/SKILL.md`) and research skill (`skills/research/SKILL.md`) instruct the agent to call external APIs (Crustdata, Exa, Brave, Happenstance). These network calls are made by the agent, not by the gbrain binary — but the effect is the same: brain content (queries, entity names) can reach third-party services. The `knowledge_gaps.sensitivity` field controls this: gaps default to `internal` (no external research), and must be explicitly upgraded to `redacted` or `external` before the research skill will issue network calls. Raw query text (`query_text`) is never retained at detection time — only a `query_hash` is stored; `query_text` is populated only after explicit approval. Agents must respect this classification.
- `gbrain export` writes plaintext markdown. Treat export directories with the same sensitivity as the DB.
- `.env` files or API keys for enrichment skills (Crustdata, Exa) are the user's responsibility. gbrain never stores them in brain.db.

**Non-goals:** gbrain does not implement user auth, access control, audit logging, or data classification. It is a single-user tool on a single machine. If the machine is compromised, the brain is compromised.

---

## Error Handling and Graceful Degradation

| Failure | Behavior |
|---------|----------|
| Candle model fails to load | Fatal on startup. Binary refuses to serve if embeddings can't work. Clear error message with path to model weights. |
| sqlite-vec not available | Fall back to pure-Rust cosine similarity (O(n) scan). Log warning. Performance degrades but search still works. |
| Skill file missing | Use embedded default. If embedded default also missing (shouldn't happen), warn and continue — the binary still works for read/write/search, just without agent workflows. |
| DB file corrupt | `gbrain validate --all` reports errors. `gbrain` refuses destructive operations on a corrupt DB. Recommend restore from backup. |
| WAL sidecar missing | SQLite handles this — rolls back uncommitted transactions, creates fresh WAL. No data loss for committed writes. |
| Disk full during write | SQLite transaction rolls back cleanly. No partial state. Error surfaced to caller. |
| Concurrent writer conflict | ConflictError returned to caller with current version. Caller retries. No data corruption. |

**Principle:** Fail loud, fail safe. Never silently corrupt. Never silently drop data. If in doubt, refuse the operation and tell the user why.

---

## Comparison Table

| | Garry's GBrain (v0.8) | This spec (v4) | MemPalace | agentmemory | Obsidian | Notion |
|---|---|---|---|---|---|---|
| Language | TypeScript/Bun | Rust | Python | Node.js | Electron | Web/Cloud |
| Binary | ~10MB + API dep | ~90MB self-contained | pip install | npm install | Heavy app | SaaS |
| Embeddings | PGLite (local, v0.8) | BGE-small local (free) | ChromaDB (local) | BM25 + vector | Plugin | Built-in AI |
| Storage | PGLite (local Postgres) | Single SQLite | ChromaDB + SQLite | SQLite | Markdown files | Cloud DB |
| Search | FTS5 + vector + weighted | Set-union + palace filter + progressive | Palace-filtered semantic | Triple-stream (BM25+vec+KG) | Plugin | Cloud |
| Graph traversal | No | Yes (N-hop `brain_graph`) | No | Yes (KG edges) | No | No |
| Retrieval | Full pages | Progressive (budget-gated) | Closet → drawer drill-down | Context injection + budget | Manual | Full pages |
| Dedup/novelty | None | Jaccard + cosine | None published | TTL + importance eviction | None | None |
| Temporal graph | No | Yes (valid_from/until) | Yes (KG triples) | Yes (versioned) | No | No |
| Contradiction detection | No | Yes (CLI + MCP) | Yes (fact_checker.py) | Yes (cascading staleness) | No | No |
| Knowledge gap detection | No | Yes (`brain_gap` + research skill) | No | No | No | No |
| Memory tiers | 2 (truth + timeline) | 4 (evidence → fact → concept → narrative) | 4 (L0-L3) | 4 (observation → narrative) | 1 (flat notes) | 1 (pages) |
| Knowledge model | Compiled truth + timeline | Compiled truth + timeline (tiered) | Palace (wings/halls/rooms) | Knowledge graph | Flat notes | Pages + DBs |
| LongMemEval | Not published | Est. 85-92% R@5 | 96.6% R@5 (raw mode) | 64% Recall@10 | N/A | N/A |
| Air-gapped | No (PGLite OK, but Bun runtime) | Yes (true static binary) | Yes | No (needs MCP client) | Yes | No |
| API keys | None (v0.8) | None | None | None | None | None |
| Backup story | PGLite data dir | `cp brain.db` | ChromaDB dir | SQLite file | rsync | Cloud |

---

## Open Questions

### 1. Embed model weights into binary?

**Option A:** `include_bytes!()` the ONNX model file into the binary at compile time
- Pros: Truly zero-dependency, no network ever, guaranteed version consistency
- Cons: ~90MB binary, model updates require recompile

**Option B:** fastembed downloads BGE-small to `~/.cache/fastembed/` on first run (33MB download, then cached)
- Pros: Smaller binary distribution (~55MB vs ~90MB), model updates independent of binary
- Cons: Requires internet on first run, breaks offline/air-gapped deployment

**Decision:** Ship Option A (embedded weights) as the default — the spec's core promise is
zero-dependency, offline-first operation. Option B available via `--features online-model`
for users who prefer smaller binaries and accept the network dependency.

### 2. sqlite-vec linking mechanism — RESOLVED

**Decision:** Compile `vec0.c` directly into rusqlite via a custom build script. sqlite-vec is a single C file (~3K lines) with no external dependencies beyond SQLite itself. The approach:

1. Add `vec0.c` and `vec0.h` to `src/vendor/sqlite-vec/`
2. In `build.rs`, compile `vec0.c` with `cc::Build` and link it into the rusqlite bundled SQLite
3. Register the extension at runtime via `sqlite3_auto_extension` (no `load_extension` call needed)

This produces a single statically linked binary on all targets including `musl`. The approach is proven by other SQLite extension projects (e.g., sqlean bundles extensions the same way).

**Fallback:** If vec0.c integration is blocked, pure-Rust cosine similarity (O(n) scan over all embeddings) is acceptable for < 100K chunks. Performance is ~50ms for 50K chunks on Apple Silicon — within the 200ms target.

**CI verification:** Every release build runs `ldd` (Linux) / `otool -L` (macOS) / `file` to confirm no dynamic dependencies. This is a release gate — see Week 5 benchmarks.

### 3. Embedding dimension: 384 vs higher

BGE-small-en-v1.5 = 384 dims. BGE-base = 768 dims. BGE-large = 1024 dims.
- Larger = better quality, larger DB, slower search
- 384 is fast and good enough for personal knowledge base retrieval
- Config table and `model` column make it trivial to switch

### 4. MCP server: which rmcp version?

`rmcp` is newer and less battle-tested than `@modelcontextprotocol/sdk` (used in Garry's spec). Alternatives:
- Implement MCP protocol directly (it's JSON-RPC 2.0 over stdio, relatively simple)
- Use a thin wrapper layer

### 5. Ingest command UX

Does `gbrain ingest <file>` attempt to parse entities itself (requiring an LLM call), or is it a pass-through that stores the file and expects the agent (via MCP `brain_ingest` tool) to do the parsing?

**Recommendation:** `gbrain ingest` stores the raw source + auto-logs it + runs novelty check. The actual entity extraction and page creation happens via the agent following `skills/ingest/SKILL.md`. This keeps the binary dumb and the intelligence in markdown.

### 6. Palace room derivation granularity (v2)

Wing derivation from slug is straightforward. Room derivation from section headers is less obvious. Options:
- **Section-based:** Each `## Header` in compiled_truth becomes a room. Simple but creates many rooms per page.
- **Fixed enum:** Map to standardised halls (facts, events, discoveries, preferences, advice) like MemPalace. More structured but requires classification.
- **Hybrid:** Use fixed halls for embedding metadata (which hall does this chunk belong to?), section headers for page-level room. Best of both worlds but more complex.

**Recommendation:** Start with wing-only filtering (v2.0). Add room-level filtering as v2.1 once we have empirical data on palace filter effectiveness with real corpus.

### 7. Contradiction detection: heuristic vs LLM-assisted (v2) — RESOLVED

**Decision:** Heuristic-only in the binary, powered by the `assertions` table. Agents populate temporal assertions during Tier 2 ingest (`{subject, predicate, object, valid_from, valid_until}`). Fact changes supersede old assertions (`valid_until` set). The binary runs pure-SQL consistency checks on current beliefs only (`valid_until IS NULL`):
- **Link vs Assertion:** Current assertions where the corresponding link has `valid_until` in the past.
- **Cross-page conflicts:** Multiple current assertions with same subject+predicate but different objects.
- **Temporal staleness:** `timeline_updated_at` > `truth_updated_at` by more than 30 days.

LLM-assisted cross-page checks happen via the maintain skill. Binary stays dumb.

---

## Spec History

| Date | Event |
|------|-------|
| 2026-04-05 | Garry Tan specs GBrain v1 (TypeScript/Bun, OpenAI embeddings). Inspired by hitting git scaling limits at 7,471 files / 2.3GB. |
| 2026-04-05 | Garry posts spec to GitHub Gist. Architecture: SQLite + FTS5 + vector, thin CLI, fat skills, MCP-first. |
| 2026-04-06 | Initial architecture review of Garry's spec. Key improvements identified: Rust over TypeScript, local BGE-small-en-v1.5 over OpenAI embeddings, sqlite-vec over pure-JS cosine similarity, RRF over weighted sum. |
| 2026-04-06 | Full standalone spec written (v1). Incorporates Garry's schema verbatim (adapted for 384-dim vectors), all CLI commands, all skill files. Adds: Rust implementation details, cross-compile CI, embedding decision rationale. |
| 2026-04-08 | Memory research integration (v2). Incorporated techniques from MemPalace (500/500 LongMemEval), UNC AutoResearchClaw/OMNIMEM (+411% F1), agentmemory (92% token reduction), and Obsidian Mind. Seven changes: (1) palace-style hierarchical filtering (+34% retrieval), (2) set-union hybrid search replacing RRF (+44% F1), (3) progressive retrieval with token budgets, (4) selective ingestion via Jaccard/cosine novelty checks, (5) temporal knowledge graph with validity windows, (6) contradiction detection (CLI + MCP), (7) four-tier memory consolidation in ingest skill. Schema version bumped to v2. |
| 2026-04-08 | **v3 Architecture Review.** Five changes: (1) Exact-Match Short-Circuit (SMS) ensures title/slug matches always rank first, (2) Temporal sub-chunking embeds timeline entries individually instead of as one blob, (3) Assertions table enables pure-SQL heuristic contradiction detection, (4) Strict optimistic concurrency with `expected_version` on MCP writes, (5) Switched from `fastembed`/ONNX to pure-Rust `candle` for true `musl` static binary. Also: embedding model registry for safe model upgrades, temporal link uniqueness fix, ingest idempotency, semantic round-trip contract, raw_imports table. |
| 2026-04-09 | **Work-context entity types.** Inspired by Rowboat (rowboatlabs/rowboat) knowledge-graph-vs-wiki insight: added `decision`, `commitment`, `action_item` as first-class page types with templates. Updated ingest skill to extract work-context entities from meetings. Updated briefing skill with commitments due, action items, and "what shifted overnight" report (superseded assertions in last 24h). |
| 2026-04-09 | **External review integration.** Accepted 11 of 12 findings from adversarial review. Added: Non-Goals section, Phased Delivery (core → intelligence → polish), WAL/single-file honesty (`gbrain compact`), embedded skills with external override + `skills doctor`, enriched chunk model (content_hash, token_count, heading_path, last_embedded_at), assertion provenance (asserted_by, source_ref, evidence_text), `brain_link_close` for targeted temporal closure, corpus-reality benchmarks alongside leaderboard benchmarks, palace filtering caveat. Rejected: `valid_from = 'unknown'` sentinel replacement (alternatives add complexity for marginal benefit). |
| 2026-04-13 | **v4 Community Research + Garry v0.8.0 integration.** Reviewed new research notes from Apr 9-12. Eight spec changes: (1) `knowledge_gaps` table + `brain_gap`/`brain_gaps` MCP tools for self-improving knowledge base — agent detects what it doesn't know and logs it for research skill resolution. (2) `brain_graph` MCP tool + `gbrain graph` CLI for N-hop neighborhood traversal — returns pages + links as JSON for UI/graph visualization. (3) `original` as first-class page type with template — distinguishes the user's own thinking from world concepts. (4) Standardised source attribution format with authority hierarchy — inline citation format, URL requirement for social refs, source conflict rules. (5) Filing disambiguation rules in ingest skill — concept vs original vs idea vs project decision tree. (6) Richer person template with optional enrichment sections (What They Believe, Hobby Horses, Trajectory, Network) for Tier 1 contacts. (7) Three new skills: `alerts/SKILL.md` (interrupt-driven vs scheduled notifications), `research/SKILL.md` (knowledge gap resolution), `upgrade/SKILL.md` (agent-guided binary + skill updates inspired by Garry's v0.8.0 auto-upgrade UX). (8) "Why SQLite over PGLite" design decision + links-table-as-graph-layer positioning. Comparison table updated for Garry v0.8.0 + PGLite. Schema version bumped to v4. |

---

*This spec is designed to stand alone. Everything needed to build GigaBrain is above — no prior context required. It is explicitly inspired by Garry Tan's GBrain work while pursuing a Rust + SQLite implementation with different deployment trade-offs. v4 integrates memory research from MemPalace, OMNIMEM, and agentmemory, plus community research and Garry Tan's v0.8.0 GBrain skillpack analysis. Architecture additions: knowledge gap detection, graph traversal, source attribution standards, filing disambiguation, and three new skills (alerts, research, upgrade).*
