---
title: "Quaid Product Roadmap"
source: quaid-app/quaid
date: 2026-05-04
tags: [quaid, roadmap, product, planning, memory-systems]
aliases: [quaid-roadmap]
---

# Quaid Product Roadmap

**Last updated:** May 4, 2026
**Latest public release:** v0.17.0
**Current release lane:** v0.18.0
**Benchmark baseline:** DAB v1 213/215 (99%), LoCoMo 0.1%, LongMemEval 0.0%, BEAM 0.0%

---

## Vision

Quaid is local-first persistent memory for AI agents. A single static binary wrapping SQLite + FTS5 + vector embeddings that runs fully airgapped. The north star: a private, offline, compounding knowledge system that gets smarter as you use it. No cloud. No API keys. No data leaving the machine.

The gap vs competitors (Mem0 v3, GBrain, Hindsight) is documented and honest:

| Benchmark | Quaid | Mem0 v3 | Status |
|-----------|-------|---------|--------|
| Infrastructure (DAB v1) | 99% | ~70% est | Quaid leads |
| LoCoMo | 0.1% | 91.6% | Gap = issue #105 |
| LongMemEval | 0.0% | 93.4% | Gap = issue #105 |
| BEAM 100K | 0.0% | 64.1% | Gap = issues #105, #107 |

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

## Phase 5 - Conversation memory foundations + SLM extraction

**Priority: current release lane — foundations landed, extractor follow-on next**
**Target: >40% LoCoMo, >40% LongMemEval**
**Issues: #137 (namespace, shipped), #105 (conversation memory), #135 (contradiction resolution)**

### What it needs to achieve

The single biggest gap vs Mem0/GBrain: Quaid stores raw conversation turns as documents. Fact extraction doesn't happen at write time. When asked "What degree did I graduate with?", Quaid can't answer from "Business Administration, spent 4 years at it" buried in casual conversation.

### Landed on the `v0.18.0` branch

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
- The latest public binaries and `install.sh` still publish `v0.17.0`
- The upcoming `v0.18.0` tag is the release that will publish the 3 new conversation-memory MCP tools

### Still remaining after foundations

**SLM-based fact extraction** (runs in background)
- Local SLM: Phi-3.5 Mini (MIT, ~2GB) as default. Configurable to Gemma 3 1B/4B.
- Fully airgapped: download once at setup, zero network calls after
- Context window: 3-5 turns (single turns are meaningless without context)
- Extracts: decisions, preferences, facts, tasks
- Stores as typed pages: `decision`, `preference`, `fact`, `action_item`
- Config: `quaid config set extraction.enabled true`

**Contradiction resolution** (#135)
- Supersede stale facts: "I used to work at X, now Y"
- `memory_put(content, supersedes=<prev_id>?)`
- Version chains: temporal-latest query by default, historical on flag

### Success criteria
- LoCoMo benchmark score > 40% (from 0.1% baseline)
- LongMemEval score > 40% (from 0.0% baseline)
- Fully airgapped - no API calls after model download
- Non-blocking - ingest latency unchanged
- `quaid query "what did we decide about X last week"` returns relevant facts

### Next design / delivery questions
- What is the final 3-5 turn extraction boundary? (Strict session window, time window, or both?)
- How does contradiction resolution interact with ADD-only immutability once extracted facts begin superseding each other automatically?
- Model download: lazy (first use) or eager (at `extraction.enabled`)?
- What structured format should extracted facts persist with, and what confidence metadata should survive review?

---

## Phase 6 - Knowledge Graph

**Target: GBrain-competitive entity linking, graph traversal**
**Issues: #107 (entity extraction), #72 (self-wiring graph), #133 (graph traversal API), #74 (multi-hop)**

### What it needs to achieve

GBrain v0.23.0 headline: "Your brain's people, companies, and concepts now all benefit from what it learns from you." Entity graph that enriches over time. This is the second major gap after conversation memory.

### Core requirements

**Zero-LLM entity extraction at write time** (#107)
- Extract people, companies, concepts from ingested pages
- Zero LLM required - pure heuristic/NLP extraction
- Can share the SLM pipeline from Phase 5 if SLM is enabled
- Entities get canonical IDs, are linked across pages

**Self-wiring knowledge graph** (#72)
- YAML frontmatter as first-class graph edges
- Wikilinks and tags auto-generate edges
- Historical GBrain data: +28% graph search performance, -53% noisy results

**Graph traversal query API** (#133)
- `memory_graph_query(entity, hops)` MCP tool
- "Find all pages connected to entity X"
- Multi-hop traversal (1-3 hops configurable)
- Edge type weighting: explicit wikilink > tag co-occurrence > title mention

**Multi-hop traversal** (#74)
- Configurable depth: `--depth 1|2|3|auto`
- Combine semantic similarity with relationship distance scoring

### Success criteria
- `memory_graph_query` returns entity-adjacent pages in <200ms (1-hop)
- Graph edges persist across sessions
- Entity extraction runs without LLM call (zero marginal cost)
- DAB v2.1 §4 Knowledge Graph score improves from ~0

### Key design decisions for OpenSpec
- Entity extraction: heuristic-only vs SLM-optional?
- Graph storage: new table vs extending links table?
- How do graph edges interact with the existing `memory_link` surface?
- Namespace isolation for graph queries - does entity graph span namespaces?

---

## Phase 7 - Intelligence Layer

**Target: Active memory, production agent trust**
**Issues: #136 (active enrichment), #75 (noise reduction), #76 (context compression)**

### What it needs to achieve

With entity graph and conversation memory in place, the system can start being proactive rather than purely reactive.

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

Not on the current roadmap but worth an OpenSpec after the `v0.18.0` foundations and the extraction follow-on are both shipped.

---

## Build Order Summary

| Priority | Issue | Feature | Depends on |
|----------|-------|---------|-----------|
| 1 | #137 ✅ | Namespace isolation | — |
| 2 | #134 | Large corpus performance | — |
| 3 | #105 | Conversation memory foundations (`v0.18.0` lane) + SLM extraction follow-on | #137 |
| 4 | #135 | Contradiction resolution | #105 |
| 5 | #107 | Entity extraction | #105 (coordinate) |
| 6 | #72 | Self-wiring knowledge graph | #107 |
| 7 | #133 | Graph traversal query API | #72, #107 |
| 8 | #74 | Multi-hop traversal | #133 |
| 9 | #136 | Active memory enrichment | #107, #133 |
| 10 | #75, #76 | Noise reduction, context compression | Any |

---

## Related Resources

- Roadmap page on docs site: quaid.app/contributing/roadmap/
- Benchmark results: benchmark.quaid.app
- Competitive intel: [[gbrain-v0.23.0-openclaw-integration]], [[mit-recursive-language-models-rlm]]
- Issue tracker: github.com/quaid-app/quaid/issues
