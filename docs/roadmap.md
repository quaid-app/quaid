# GigaBrain Roadmap

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
**Branch:** `phase2/p2-intelligence-layer`
**Depends on:** Phase 1 ship gate

**Release:** `v0.2.0` — tag pending. All ship gates passed.

Phase 2 adds cross-reference traversal, temporal reasoning, and memory-consolidation capabilities that separate GigaBrain from a glorified FTS5 wrapper.

**Deliverables:**
- Temporal links with validity windows: `gbrain link`, `gbrain link` close via `--valid-until`
- N-hop graph neighbourhood traversal: `gbrain graph <slug> --depth N --temporal active|all [--json]`
- Assertions table with provenance + heuristic contradiction detection: `gbrain check [--slug SLUG] [--all] [--json]`
- Progressive retrieval with full token-budget gating: `gbrain query "..." --depth auto`
- Novelty checking — ingest skips near-duplicate content (Jaccard ≥ 0.85 or cosine above threshold)
- Palace room classification via `##`-heading-based `derive_room` in `src/core/palace.rs`
- Knowledge gap detection and listing: `gbrain gaps [--resolved] [--limit N] [--json]`; auto-logged on low-result queries
- Work-context page types: `decision`, `commitment`, `action_item`
- Full MCP write surface with optimistic concurrency (version check on `brain_put`)
- MCP Phase 2 tools: `brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`, `brain_check`, `brain_timeline`, `brain_tags`

**Key modules added:**
- `src/core/graph.rs` — N-hop BFS over links with temporal filtering
- `src/core/assertions.rs` — contradiction detection via SQL assertion comparison
- `src/core/gaps.rs` — gap logging and resolution
- `src/commands/graph.rs`, `src/commands/check.rs`, `src/commands/gaps.rs`

**Ship gate (all passed — Phase 3 unblocked):**
1. `cargo test` passes
2. Graph BFS returns correct N-hop neighbourhood with temporal filtering
3. `gbrain check --all` detects conflicting assertions
4. Novelty check rejects near-duplicate ingest (Jaccard ≥ 0.85)
5. All Phase 2 MCP tools respond correctly
6. No regression on BEIR baseline

---

## Phase 3 — Polish, Skills, and Benchmarks 🔄

**Status: In progress**
**Branch:** `phase3/p3-skills-benchmarks`
**OpenSpec:** [`openspec/changes/p3-skills-benchmarks/`](../openspec/changes/p3-skills-benchmarks/)
**Depends on:** Phase 2 ship gate

Phase 3 is delivered in two OpenSpec slices:

- **`p3-polish-benchmarks`** — release readiness, coverage CI, docs polish. **Already shipped** on this branch.
- **`p3-skills-benchmarks`** — skills completion, benchmark harnesses, CLI polish, MCP Phase 3 tools. **This branch.**

**Completed in this branch:**
- 5 production-ready agent skills: `briefing`, `alerts`, `research`, `upgrade`, `enrich`
- CLI stub completion: `validate --all/--links/--assertions/--embeddings`, `call`, `pipe`, `skills list`, `skills doctor`
- MCP Phase 3 tools: `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw` (16 tools total)
- `--json` flag coverage across all commands
- Benchmark harnesses: BEIR (nDCG@10), corpus-reality, concurrency stress, embedding migration, LongMemEval, LoCoMo, Ragas

**Pending before ship gate:**
- CI benchmark gate wiring: offline benchmark jobs in `.github/workflows/ci.yml` (tasks 7.1–7.2)
- Nibbler adversarial review: `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw`
- Scruffy benchmark reproducibility sign-off
- `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` all clean

**Gate:** All offline benchmark targets met; `v1.0.0` release artifacts built and verified static.

---

## Deliberate Deferrals

These are known design choices that are _not_ oversights:

| Deferral | Reasoning |
| -------- | --------- |
| npm global installation (`npm install -g gbrain`) | Requires npm packaging, registry account, and publish pipeline. Deferred until core release contract is fully polished. Will be proposed as a separate change. |
| One-command curl installer | Adds operational surface area, signing concerns, and support burden. Deferred follow-on — not part of the v0.1.0 or v1.0.0 release scope. |
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
| `v1.0.0` | Phase 3 — full skill suite + benchmarks + release pipeline |
