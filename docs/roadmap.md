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

**Release:** `v0.1.0`

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

**Release:** `v0.2.0`

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

## Phase 3 — Polish, Skills, and Benchmarks ✅

**Status: Complete**
**Branch:** `phase3/p3-skills-benchmarks` → PR #31
**OpenSpec:** [`openspec/changes/archive/2026-04-17-p3-skills-benchmarks/`](../openspec/changes/archive/2026-04-17-p3-skills-benchmarks/)
**Depends on:** Phase 2 ship gate

Phase 3 was delivered in two OpenSpec slices:

- **`p3-polish-benchmarks`** — release readiness, coverage CI, docs polish. Shipped on this branch.
- **`p3-skills-benchmarks`** — skills completion, benchmark harnesses, CLI polish, MCP Phase 3 tools. This PR.

**Delivered:**
- 5 production-ready agent skills: `briefing`, `alerts`, `research`, `upgrade`, `enrich` — all 8 skills are now production-ready
- CLI stub completion: `validate --all/--links/--assertions/--embeddings`, `call`, `pipe`, `skills list`, `skills doctor`
- MCP Phase 3 tools: `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw` — 16 tools total
- `--json` flag coverage across all commands
- Benchmark harnesses: BEIR (nDCG@10), corpus-reality, concurrency stress, embedding migration, LongMemEval, LoCoMo, Ragas
- CI benchmark gate wiring in `.github/workflows/ci.yml`

**Gate:** All offline benchmark targets met; `v0.9.2` dual-channel release artifacts built and verified. `v0.9.4` adds FTS5 search hardening and assertion extraction tightening (see Version targets below).

---

## Deliberate Deferrals

These are known design choices that are _not_ oversights:

| Deferral | Reasoning |
| -------- | --------- |
| Public npm publication | Packaging and postinstall are implemented, but public publication still depends on registry ownership and `NPM_TOKEN` release automation. |
| Homebrew tap, winget, or other package managers | Same dependency as npm. Tracked as future distribution work. |
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
| `v0.9.2` | Phase 3 — full skill suite + benchmarks + dual BGE-small release channels |
| `v0.9.4` | FTS5 search hardening (`sanitize_fts_query`, `--raw` bypass, JSON errors) + assertion extraction tightening (scope to `## Assertions` sections + frontmatter; #55 remains a post-ship rerun gate) |

---

## vault-sync-engine — Collections, Live-Sync, and Write Safety 🔄

**Status: In progress**
**Branch:** `spec/vault-sync-engine`
**OpenSpec:** [`openspec/changes/vault-sync-engine/`](../openspec/changes/vault-sync-engine/)

Adds vault-as-collection attachment, a file watcher, a stat-diff reconciler, quarantine lifecycle, and a fully safe write-through path for `brain_put` on Unix.

### What has landed

- **Schema v5** — `collections`, `file_state`, `raw_imports`, `embedding_jobs`, quarantine indexes; v4 brains refuse with re-init instructions
- **Collection management** — `gbrain collection add|list|info|sync|restore|restore-reset|reconcile-reset`
- **Ignore patterns** — `gbrain collection ignore add|remove|list|clear --confirm`; atomic-parse `.gbrainignore` with mirror refresh; built-in defaults (`.git/**`, `node_modules/**`, etc.) always applied
- **Quarantine lifecycle** — `gbrain collection quarantine list|export|discard`; auto-sweep TTL (`GBRAIN_QUARANTINE_TTL_DAYS`, default 30); pages with DB-only state (links, assertions, knowledge gaps, contradictions, raw_data) are quarantined rather than hard-deleted; `discard --force` or post-export discard available
- **Reconciler** — stat-diff walk, UUID identity resolution, rename detection (native pair → UUID match → content-hash uniqueness), delete-vs-quarantine classifier, 500-file batch commit
- **File watcher** — one `notify` watcher per active collection in `gbrain serve`; 1.5 s debounce (`GBRAIN_WATCH_DEBOUNCE_MS`); reconcile-backed flushes; path+hash self-write suppression with TTL expiry
- **Write-through `brain_put`** *(Unix)* — full rename-before-commit sequence (recovery sentinel → tempfile → `renameat` → fsync parent dir → single SQLite tx); mandatory `expected_version` for updates; `check_fs_precondition` four-field CAS
- **Write interlock** — `state='restoring'` or `needs_full_sync=1` blocks all mutating CLI/MCP ops with `CollectionRestoringError`
- **Offline restore** — `gbrain collection restore <name> <target>` → Tx-A → atomic rename → Tx-B; `sync --finalize-pending` drives full-hash reconcile and reopens writes
- **`brain_collections` MCP tool** — frozen 13-field per-collection object; truthful state, recovery, and ignore-diagnostic surfacing (17 MCP tools total)
- **Collection filter** — `brain_search`, `brain_query`, `brain_list` accept an optional `collection` filter; default to the sole active collection when exactly one exists
- **Collection-aware slug routing** — all slug-bearing CLI/MCP surfaces accept `<collection>::<slug>`; ambiguous bare slugs return a stable `AmbiguityError` with candidates

### Explicitly deferred (not yet shipped)

| Item | Why deferred |
| ---- | ------------ |
| `quarantine restore` | Requires crash-durable post-unlink cleanup and a no-replace install path; reopened until a safe slice lands |
| IPC socket write proxying (`12.6*`) | Full trust-boundary design for `SO_PEERCRED` peer auth still in progress |
| Per-event-type watcher handlers (`6.5–6.11`) | Create/Modify/Delete/Rename handlers; overflow recovery, `.gbrainignore` live reload, and watcher supervisor not yet wired |
| Embedding job queue (`8.*`) | Async background embedding worker not yet implemented |
| `gbrain collection remove` | Detach + optional purge not yet implemented |
| `gbrain stats` per-collection augmentation | Per-collection row + aggregate totals pending |
| Online restore handshake (`17.5pp/qq*`) | Live-serve ack protocol not yet implemented |
| Opt-in UUID write-back (`5a.5`, `migrate-uuids`) | `--write-gbrain-id` and `migrate-uuids` CLI not yet implemented |
| Legacy `gbrain import` removal (`15.*`) | Import path remains until reconciler covers all ingest use cases |
