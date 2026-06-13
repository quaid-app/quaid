---
title: "Quaid Product Roadmap"
source: quaid-app/quaid
date: 2026-05-14T10:44:54.579+00:00
tags: [quaid, roadmap, product, planning, memory-systems]
aliases: [quaid-roadmap]
---

# Quaid Product Roadmap

**Last updated:** 2026-05-14T10:44:54.579+00:00
**Latest public release:** v0.22.3
**Main branch manifest:** v0.22.3
**Next release lane:** not tagged yet
**Benchmark status:** DAB v1 release gate is 94.4% on `v0.22.2`; conversation-memory and scale benchmarks remain the main public gap.

---

## Vision

Quaid is local-first persistent memory for AI agents. A single static binary wrapping SQLite + FTS5 + vector embeddings that runs fully airgapped. The north star: a private, offline, compounding knowledge system that gets smarter as you use it. No cloud. No API keys. No data leaving the machine.

Release truth matters here:

- GitHub Releases and `install.sh` currently resolve to **`v0.22.3`**.
- `Cargo.toml` on `main` also now reads **`0.22.3`**, so there is no separate public `v0.23.0` lane waiting to be tagged.
- The roadmap below mixes **already shipped foundations** with **follow-on work that is still branch-only or not started**. Those states are called out explicitly.

---

## Completed Phases

### Phase 1 - Core Storage, CLI, Search, MCP (v0.1.0) ✅
FTS5 + vector hybrid search, 5 MCP tools, markdown import/export.

### Phase 2 - Intelligence Layer (v0.2.0) ✅  
Temporal links, assertions, contradiction detection, progressive retrieval, 16 MCP tools.

### Phase 3 - Skills, Benchmarks, CLI Polish (v0.9.2) ✅
All 8 skills production-ready, BEIR regression gate, benchmark harnesses (LoCoMo, LongMemEval, BEAM adapters).

### Phase 4 - Vault Sync Engine (v0.9.x → v0.17.0) ✅ Shipped core surface
Multi-collection live filesystem sync, namespace isolation, collection health reporting, and the reviewed Unix same-root single-file live-write path are all in the latest public release. `quaid collection add/sync` replaced `quaid import`.

**Still deferred:** Online restore handshake, Windows quarantine restore, and the remaining restore hardening follow-ons.

---

## Phase 5 - Conversation memory foundations + SLM extraction ✅ Shipped in v0.19.0

**Shipped: v0.19.0**
**Target: >40% LoCoMo, >40% LongMemEval**
**Issues: #137 (namespace, shipped), #105 (conversation memory), #135 (contradiction resolution)**

### What it needed to achieve

The single biggest gap vs Mem0/GBrain: Quaid stores raw conversation turns as documents. Fact extraction doesn't happen at write time. When asked "What degree did I graduate with?", Quaid can't answer from "Business Administration, spent 4 years at it" buried in casual conversation.

### Shipped surface (v0.19.0)

**Namespace isolation** ✅ Shipped in `v0.16.0` — multiple agents and sessions share one DB without bleed.

**Conversation ingestion and session controls**
- `memory_add_turn` accepts `{session_id, role, content, timestamp?, metadata?}`
- `memory_close_session` forces the final queue flush for a session
- `memory_close_action` updates `action_item` pages in place
- Conversation turns are written to namespace-aware day files under `conversations/YYYY-MM-DD/<session-id>.md`
- Request-path latency stays non-blocking; the request does file append + fsync + queue work only

**Queue and retrieval foundations**
- `extraction_queue` is landed with debounce and session-close triggers
- ADD-only supersede chains are landed for page history
- Head-only retrieval is now the default, with `--include-superseded` / `include_superseded` as the opt-in escape hatch
- `memory_get` and `memory_graph` surface supersede relationships directly

**Release truth**
- All Phase 5 work shipped in `v0.19.0` with the extraction worker, `quaid extraction status`, `quaid extract`, DAB §8 benchmark wiring, and the extraction/integration proof surfaces

### Success criteria
- LoCoMo benchmark score > 40% (from 0.1% baseline)
- LongMemEval score > 40% (from 0.0% baseline)
- DAB §8 Conversation Memory harness exists with LoCoMo + LongMemEval subsections and a documented no >3-point regression rule once representative-hardware baselines are recorded
- Fully airgapped - no API calls after model download
- Non-blocking - ingest latency unchanged
- `quaid query "what did we decide about X last week"` returns relevant facts

**Current truth boundary:** the repo carries a manual/hosted-smoke DAB §8 hook. The authoritative regression gate still requires a full representative Unix hardware run because the always-on CI fleet does not match that hardware profile yet.

---

## Phase 5b - Daemon runtime and HTTP/SSE transport ✅ Shipped in v0.21.0

**Shipped: v0.21.0**
**Issues: #175 (multi-agent HTTP transport), #177 (standalone extraction worker)**

### What it achieved

`quaid serve` coupled the background runtime to the stdio MCP transport: when the MCP client closed stdin, the workers died with it. This phase separates the runtime from the transport so vault sync, the extraction worker, and all supervised duties survive MCP client disconnects.

### Shipped surface (v0.21.0)

- `quaid daemon run` — foreground entry point for launchd/systemd; owns the full background runtime and never opens stdio MCP; optionally hosts HTTP/SSE via `--http`
- `quaid daemon install|uninstall|start|stop|restart|status|logs` — platform service lifecycle (macOS launchd, Linux systemd)
- `quaid status` — top-level process overview (session type, PID, DB path, transports, activity)
- `quaid serve --http` — opt-in HTTP/SSE MCP transport on loopback (v1: `--trust-loopback` required; non-loopback binds and `--token-file` are refused in v1); stdio behavior unchanged
- Session-type expansion: `daemon`, `serve_host`, `serve`, `cli` — exactly one process holds the runtime-host lease per database
- `RuntimeOwnsCollectionError` replaces `ServeOwnsCollectionError` in ownership predicates and error payloads
- No new MCP tools; the 24-tool surface is unchanged

### Release truth
- First shipped in `v0.21.0`; GitHub Releases and `install.sh` now resolve to `v0.22.3`.

---

## Phase 6 - Knowledge graph foundations ✅ Shipped foundation in v0.22.0

**Shipped foundation:** `v0.22.0`  
**Latest published release carrying it:** `v0.22.3`  
**Issues still open for follow-ons:** #107, #72, #133, #74

### What already landed

The knowledge-graph phase is no longer hypothetical. The current public release already ships the foundational graph layer:

- **Structured frontmatter + autowiring.** YAML frontmatter now round-trips as structured JSON, and `links:`, `parent:`, `children:`, and `related:` fields create derived graph edges automatically.
- **Wikilink autowiring.** `[[slug]]` references create and clean up derived `wiki_link` edges on page rewrite.
- **Graph path output.** `memory_graph` and `quaid graph` now return `paths` so agents and humans can explain how a node was reached.
- **Opt-in graph-aware retrieval knobs.** `graph_depth`, `graph_distance_decay`, `graph_expansion_max`, and `--hops N` are all landed, but retrieval expansion stays default-off until the benchmark gate is cleared.
- **Entity-pattern backfill command.** `quaid graph extract-entities` is available now, but it writes assertions only; it does not yet promote durable entity edges.

### What is still not shipped

These items remain future work and should not be described as part of the current public release:

- **No `memory_graph_query(entity, hops)` MCP tool yet.**
- **No durable entity-pattern edge promotion yet.** The extractor writes assertions, not persistent graph edges.
- **No default-on graph expansion yet.** The shipped release keeps `graph_depth = 0` by default pending the documented benchmark gate.
- **No active enrichment yet.** That stays in Phase 7.

### Success criteria for the remaining follow-ons
- Entity-centric graph query results in <200 ms for the supported 1-hop path
- Durable entity edges that persist across sessions without requiring manual linking
- Default-on graph expansion only after the DAB §4 / MSMARCO gate is published as passing
- A public benchmark story that reflects the shipped graph layer rather than the pre-graph baseline

---

## Phase 7 - Intelligence Layer

**Target: Active memory, production agent trust**
**Issues: #136 (active enrichment), #75 (noise reduction), #76 (context compression)**

### What it needs to achieve

With the graph foundation and conversation memory in place, the system can start being proactive rather than purely reactive.

### Core requirements

**Active memory enrichment** (#136)
- When new content arrives about entity X, existing memories about X update
- GBrain v0.23.0 headline feature
- Trigger: new ingest → entity resolution → graph update → related memory enrichment
- Configurable: passive mode (no enrichment) vs active mode
- Background queue: enrichment doesn't block ingest

**Noise reduction + deduplication** (#75)
- Result deduplication (same slug, different retrieval paths)
- Relevance filtering: suppress very low-confidence matches
- Most valuable at corpus scale (10K+ pages)

**Context compression** (#76)
- REFRAG-style chunk compression before sending to LLM
- Reduce tokens sent without sacrificing quality
- High ROI at production scale API cost

### Key design decisions for OpenSpec
- Enrichment propagation depth: 1-hop or N-hop?
- Does enrichment run synchronously or strictly background?
- Deduplication: at retrieval time or at ingest time?
- What's the "I trust this agent's memory tomorrow" composite score?

---

## Scale & Performance (Parallel Track)

**Issues: #134 (large corpus performance)**

### What it needs to achieve

Current import: ~437s for 350 pages. GBrain operates at 75,000 files. Quaid has never been validated at that scale.

### Benchmark targets

| Scale | Import target | FTS p95 | Semantic p95 |
|-------|--------------|---------|-------------|
| 1K pages | <30s | <50ms | <300ms |
| 10K pages | <5min | <100ms | <500ms |
| 50K pages | <30min | <200ms | <1s |

### Investigation areas
- Incremental indexing (only re-index changed pages)
- Parallel embedding (multi-threaded)
- Index sharding by PARA category or date
- Lazy embedding (embed on query miss, not at ingest)

### Key design decision
- Does this require schema changes, or is it pure optimization?
- What's the interaction with the live watcher (Batch 7)?

---

## Personal Eval Framework (Future)

**Inspired by GBrain v0.25**

GBrain v0.25 shipped personal evals against real user queries. The philosophical debate:
- GBrain: "Evals on your real workload is the only honest signal, public benchmarks are theatre"
- Quaid: Open benchmarks (DAB, LoCoMo, LongMemEval, BEAM) for cross-system comparison

Both are right in different contexts. A future feature worth considering:

`quaid eval --against-history` - run your N most recent queries against current binary, compare to stored baseline, report regressions. Each user gets their own personalized regression detector.

Not on the current roadmap but worth an OpenSpec after the daemon surface is stable.

---

## Build Order Summary

This table is now only for **remaining work** after the shipped phases above.

| Priority | Issue | Remaining feature | Depends on |
|----------|-------|-------------------|-----------|
| 1 | #134 | Large corpus performance | — |
| 2 | #107 | Durable entity extraction follow-on | #105 (coordinate) |
| 3 | #72 | Self-wiring graph follow-on and edge promotion | #107 |
| 4 | #133 | Entity-centric graph query API | #72, #107 |
| 5 | #74 | Multi-hop traversal | #133 |
| 6 | #136 | Active memory enrichment | #107, #133 |
| 7 | #75, #76 | Noise reduction, context compression | Any |

---

## Roadmap feasibility verdicts (#73, #76, #136, #167, #173)

Five open issues previously had no review coverage and no feasibility signal, so
the team could not tell cheap extensions from architecture changes. The verdicts
below record where each one actually sits and — critically — the **blocking
primitive** for each, so that a future change touching that primitive re-triggers
this review. Evidence file:line references are accurate as of commit `92b9018`
and will drift; treat them as starting points, not a permanent index.

| Issue | Verdict | Blocking primitive (re-trigger review when this changes) |
|-------|---------|----------------------------------------------------------|
| **#73** Minion queue / async jobs | **Mostly already built.** Two SQLite-backed queues already exist with lease-expiry recovery, retry caps, and status polling: `extraction_queue` (`src/core/conversation/queue.rs`) and `embedding_jobs` (`src/schema.sql:374-392`), both drained by the daemon. Generalize them into **one `jobs` table** rather than building a third queue. | No generic `jobs` table. Today payload columns are hardcoded and `trigger_kind` is `CHECK`-constrained (`src/schema.sql:401`); there is no `quaid jobs` CLI or MCP job surface (`src/mcp/server.rs` exposes none). |
| **#76** REFRAG-style context compression | **Infeasible as written.** Quaid returns *text* over MCP to closed-decoder clients, so REFRAG's chunk→dense-embedding splicing has nowhere to land — the decoder is not ours. Fold into the rerank change as **extractive compression** (already specced and #76 explicitly deferred in `openspec/changes/retrieval-quality-rerank/proposal.md`). | Closed-decoder MCP boundary (no hidden-state handoff). The viable form is the `extractive-rerank` capability in the rerank proposal. |
| **#167** Image-to-memory | **Blocked.** Today the vault watcher ignores non-markdown files (`src/core/vault_sync/watcher.rs`); companion markdown indexes, images do not. The proposed "skill intercepts via vault watcher event" hook **does not exist** — skills are prose, not daemon plugins. Needs **#73 first** (generalized queue) or explicit agent-driven triggering. | No watcher→jobs ingest hook for non-md files. Depends on #73's generalized `jobs` queue. |
| **#136** Active memory enrichment | **Heaviest lift.** Entity extraction is regex-only, 5 ms-budgeted, and deliberately writes **no `links` rows** (`src/core/entities.rs`); cosine-gated supersede exists only inside the conversation path (`src/core/conversation/supersede.rs`). The "new content about X updates existing memories about X" loop has nothing to stand on. | No entity→pages index and no ingest fan-out. Also depends on durable entity edges (#107/#72) and on #73 for background propagation. |
| **#173** git-sync | **Mostly feasible, but spec the gaps first.** `.git/**` is builtin-ignored (`src/core/ignore_patterns.rs:30`), the reconciler absorbs external bulk diffs with sha256/inode rename detection, and page identity travels in `quaid_id` frontmatter (`src/core/page_uuid.rs`). **Two real gaps:** (1) DB-only state — programmatic links, `raw_data`, contradictions, and gaps — never travels via git; (2) `raw_imports` byte-exact restore (`src/core/raw_imports.rs`) is **per-machine**, so `collection restore` over a checkout diverges from git HEAD. `collection sync` also refuses while a daemon owns the collection. Spec the DB-only-state and restore-vs-checkout semantics **before** building any `quaid sync`. | DB-only state has no git-travelling representation; `raw_imports` restore is per-machine. Daemon-owns-collection refusal (`src/commands/collection.rs`). Duplicate-uuid halt (Area 6) must be fixed first. |

**Sequencing implication.** Do **#73 first** — it unlocks #167's ingest trigger and #136's background propagation. **Fold #76 into the rerank change** (close it as "extractive compression"). **Spec #173's DB-only-state and restore-vs-checkout semantics** before any `quaid sync` command lands.

Next concrete steps (not yet done):
1. Propose a `jobs` table generalizing `queue.rs` lease semantics (`job_type` + JSON payload) with a `quaid jobs` / MCP status surface. The `jobs` proposal inherits `queue.rs` lease/retry tests as its acceptance template.
2. Close #76 as "extractive compression," referencing the rerank spec.
3. Draft #173 design covering daemon-running vs stopped pull paths and conflict-marker quarantine.

> Verdict bias caveat: these verdicts can bias toward the current architecture.
> The per-issue *blocking primitive* column is the mitigation — when a listed
> primitive changes, the corresponding verdict must be re-reviewed.

---

## Related Resources

- Roadmap page on docs site: quaid.app/contributing/roadmap/
- Benchmark results: benchmark.quaid.app
- Competitive intel: [[gbrain-v0.23.0-openclaw-integration]], [[mit-recursive-language-models-rlm]]
- Issue tracker: github.com/quaid-app/quaid/issues
