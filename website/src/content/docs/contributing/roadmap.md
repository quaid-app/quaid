---
title: Roadmap
description: Phased delivery plan with explicit ship gates.
---

GigaBrain is built in phases. Each phase has a hard ship gate — no phase begins until the previous one passes.

---

## Sprint 0 — Repository Scaffold ✅

**Status: Complete**

Sprint 0 establishes the full repository structure before any core implementation begins. It produces no runnable binary, but everything that follows depends on what it puts in place.

**Deliverables:**
- `Cargo.toml` with all declared dependencies (Rust + rusqlite + sqlite-vec + candle + clap + rmcp)
- Module stubs in `src/` — `src/core/`, `src/commands/`, `src/mcp/`
- `src/schema.sql` — full v4 DDL (pages, FTS5, vectors, links, assertions, knowledge gaps)
- `skills/*/SKILL.md` stubs for all 8 skill categories
- `tests/fixtures/` — sample page fixtures
- `benchmarks/README.md`
- `CLAUDE.md` and `AGENTS.md` — context files for any agent spawned in this repo
- `.github/workflows/ci.yml` — `cargo check` + `cargo test` on every PR
- `.github/workflows/release.yml` — cross-compile matrix → GitHub Releases on tag push

**Gate:** `cargo check` passes; CI triggers on PR; all spec directories exist.

---

## Phase 1 — Core Storage, CLI, Search, and MCP ✅

**Status: Complete**  
**Owner:** Fry  
**Depends on:** Sprint 0

**Release:** `v0.1.0` — tag pending. All ship gates passed; pushing the `v0.1.0` tag triggers the release workflow.

The smallest complete slice that proves GigaBrain's value proposition. When Phase 1 ships, a real user can import their markdown brain, search it semantically and by keyword, export without data loss, and connect any MCP-compatible agent via `gbrain serve`.

**Workstream 1 — Foundation (Week 1):**
- All core types (`src/core/types.rs`)
- Database init, WAL, sqlite-vec load (`src/core/db.rs`)
- Markdown frontmatter parsing, compiled-truth/timeline split (`src/core/markdown.rs`)
- Palace wing/room derivation (`src/core/palace.rs`)
- CLI commands: `init`, `get`, `put`, `list`, `stats`, `tags`, `link`

**Workstream 2 — Search (Week 2):**
- FTS5 search with BM25 scoring (`src/core/fts.rs`)
- Candle embeddings + vector search (`src/core/inference.rs`)
- Hybrid search: SMS exact-match short-circuit + set-union merge of FTS5 + vector (`src/core/search.rs`)
- Progressive retrieval with token-budget gating (`src/core/progressive.rs`)
- CLI commands: `search`, `embed`, `query`

**Workstream 3 — Ingest and MCP (Week 3):**
- Novelty checking — Jaccard + cosine dedup (`src/core/novelty.rs`)
- `import` / `export` with normalized markdown round-trip (`src/core/migrate.rs`)
- MCP stdio server with 5 core tools: `brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`
- CLI command: `serve`

**Workstream 4 — Polish (Week 4):**
- `config`, `version`, `compact` commands
- `--json` output on all commands
- Full unit test suite
- Embedded skills finalized

**Ship gate (all passed — Phase 2 unblocked):**
1. `cargo test` passes
2. `gbrain import <corpus>` → `gbrain export` → semantic diff = 0
3. `gbrain serve` connects to Claude Code with all 5 MCP tools responding correctly
4. Static binary: `ldd` confirms no dynamic dependencies on Linux musl build
5. BEIR nDCG@10 baseline established

---

## Phase 2 — Intelligence Layer ✅

**Status: Complete**  
**Release:** `v0.2.0`  
**Depends on:** Phase 1 ship gate

**Planned scope:**
- Temporal links: `brain_link`, `brain_link_close`, backlinks with `--temporal`
- Graph neighbourhood traversal: `brain_graph`, `gbrain graph`
- Assertions with provenance
- Contradiction detection: `gbrain check`
- Progressive retrieval with token budgets (full implementation)
- Novelty checking tiers 2–4
- Work-context page types: `decision`, `commitment`, `action_item`
- Palace wing filtering (validated against benchmarks before committing to room-level)
- Full MCP write surface with version checks (optimistic concurrency enforcement)
- Optional person template enrichment sections for tier-1 contacts

**Gate:** All Phase 1 gates remain green; Phase 2 feature tests pass; no regression on BEIR baseline.

---

## Phase 3 — Skills, Benchmarks, and CLI Polish ✅

**Status: Complete**  
**Release:** `v1.0.0`  
**Depends on:** Phase 2 ship gate

**Delivered scope:**
- Release readiness: GitHub Release workflow hardening, checksum verification, and a reviewable public release checklist
- Free coverage reporting on pushes to `main` and PRs targeting `main`
- Docs polish: honest README and public docs for current status, supported install paths, and deferred work
- Docs-site build/deploy and navigation improvements
- All 8 skills production-ready (`briefing`, `alerts`, `research`, `upgrade`, `enrich`, `ingest`, `query`, `maintain`)
- `gbrain skills doctor` — skill resolution order and content hash verification
- `gbrain validate --all` — database integrity checker (links, assertions, embeddings)
- `gbrain call <TOOL> <JSON>` — raw MCP tool invocation from CLI
- `gbrain pipe` — JSONL streaming mode for shell pipelines
- 4 new MCP tools: `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw` (total: 16 tools)
- `--json` output on all commands
- Benchmark harnesses: BEIR nDCG@10 regression gate, corpus-reality, concurrency stress, embedding migration (offline, CI-gated)
- Advisory benchmarks: LongMemEval, LoCoMo, Ragas (Python adapters, API-key optional)

**Ship gate (all passed — v1.0.0 unblocked):**
1. Zero `todo!()` stubs in `src/commands/`
2. All 8 SKILL.md files are production-ready
3. 16 MCP tools registered and tested
4. `gbrain validate --all` runs successfully on a clean brain
5. `gbrain skills doctor` shows correct resolution order
6. Offline benchmarks (corpus-reality, concurrency, embedding migration) pass in CI
7. BEIR nDCG@10 baseline established with < 2% regression gate
8. `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` all clean

---

## Deliberate Deferrals

These are known design choices that are _not_ oversights:

| Deferral | Reasoning |
| -------- | --------- |
| npm global install (`npm install -g gbrain`) | Planned follow-on. npm packaging requires its own approved scope and release contract before it can ship. Not in this release. |
| Homebrew tap, winget, or other package managers | Same dependency as npm. Tracked as future distribution work. Not in this release. |
| `curl \| sh` one-command installer | Requires a stable hosted script and a hardened install contract. Tracked as future work. Not in this release. |
| First-class `chunks` table | `page_embeddings` columns are sufficient for v1. Promote if progressive retrieval lifecycle becomes painful. |
| Room-level palace filtering | Deferred until benchmarks on a real corpus prove it helps. Wing-only in v1. |
| LLM-assisted contradiction detection | The binary stays dumb. Cross-page reasoning lives in the maintain skill. |
| WASM compilation | Viable in principle (Rust has strong WASM support). Not a v1 priority. |
| Overnight consolidation cycle | Powerful agent workflow (Karpathy-style DREAMS pattern). Better as a post-v1 skill than a binary feature. |
| Collaborative / multi-user | Single-writer by design. No auth, no RBAC, no CRDTs. |

---

## Version targets

| Tag | What ships |
| --- | ---------- |
| `v0.1.0` | Phase 1 — core storage, CLI, search, MCP |
| `v0.2.0` | Phase 2 — intelligence layer |
| `v1.0.0` | Phase 3 — full skill suite + benchmarks + release pipeline ✅ |
