---
id: p3-skills-benchmarks
title: "Phase 3: Skills Completion, Benchmark Harnesses, and CLI Polish"
status: complete
type: feature
phase: 3
owner: fry
reviewers: [leela, kif, professor, nibbler, scruffy]
created: 2026-04-17
depends_on: p2-intelligence-layer
---

# Phase 3: Skills Completion, Benchmark Harnesses, and CLI Polish

## Why

Phases 1 and 2 shipped the core storage, search, graph, assertions, progressive retrieval,
and MCP tools. Phase 3 (`p3-polish-benchmarks`) handled release readiness, coverage, and
docs polish. What remains is the **intelligence layer that makes Quaid useful to agents**
and the **evaluation infrastructure that proves it works**.

Right now:
- Five of eight SKILL.md files are stubs (briefing, alerts, research, upgrade, enrich).
  Agents can't use them.
- Four CLI commands are `todo!()` stubs: `validate`, `call`, `pipe`, `skills doctor`.
- Four MCP tools from the spec are unimplemented: `memory_gap`, `memory_gaps`, `memory_stats`,
  `memory_raw`.
- No benchmark harness exists beyond a Phase 1 nDCG@10 proxy on 5 fixture pages.
- `--json` output coverage is incomplete across some commands.

Without this work, Quaid has a solid engine but no flight manual and no instruments.

## What Changes

### 1. Skills Completion (5 skills)

Author production-ready SKILL.md files for:
- **Briefing**: "what shifted" daily report — changed pages, new contradictions, open gaps
- **Alerts**: interrupt-driven notifications — new contradictions, stale pages, resolved gaps
- **Research**: knowledge gap resolution — prioritize gaps, generate queries, ingest findings
- **Upgrade**: agent-guided binary + skill updates — version check, download, verify, validate
- **Enrichment**: external data integration — Crustdata, Exa, Partiful patterns

Implement `quaid skills doctor` to verify skill resolution order and content hashes.

### 2. CLI Stub Completion

Replace all `todo!()` stubs with working implementations:
- `quaid validate --all` — referential integrity, stale embeddings, broken links, assertion dedup
- `quaid validate --links` / `--assertions` / `--embeddings` — targeted checks
- `quaid call <TOOL> <JSON>` — raw MCP tool invocation (GL pattern)
- `quaid pipe` — JSONL streaming mode for scripting
- `quaid skills list` / `quaid skills doctor` — skill inspection
- Ensure `--json` flag produces structured output on every command

### 3. MCP Phase 3 Surface

Implement remaining MCP tools from the spec:
- `memory_gap` — log a knowledge gap (with privacy-safe query_hash)
- `memory_gaps` — list unresolved/resolved gaps
- `memory_stats` — memory statistics (page count, link count, contradiction count, etc.)
- `memory_raw` — store raw structured data (API responses, JSON) for a page

### 4. Benchmark Harnesses

Build evaluation infrastructure:
- **BEIR (NQ + FiQA)**: offline nDCG@10 regression gate — Rust binary in `benchmarks/`
- **LongMemEval**: R@5 multi-session memory — Python adapter
- **LoCoMo**: F1 conversational memory — Python adapter
- **Ragas**: context_precision/recall/faithfulness — Python adapter
- **Corpus-reality tests**: import 7K+ files, retrieval correctness, idempotent round-trip
- **Concurrency stress tests**: parallel OCC, duplicate ingest, kill-before-commit
- **Embedding migration tests**: zero cross-model contamination
- CI gates: offline benchmarks block release, API-dependent are advisory

## Capabilities

### New Capabilities
- `skills-doctor`: Skill resolution order inspection and content hash verification
- `validate`: Database integrity checking (links, assertions, embeddings, referential)
- `call`: Raw MCP tool invocation from CLI
- `pipe`: JSONL streaming for shell pipelines
- `memory_gap` / `memory_gaps` / `memory_stats` / `memory_raw`: MCP tool surface completion
- `benchmark-harness`: Reproducible retrieval quality and safety evaluation
- `briefing-skill` / `alerts-skill` / `research-skill` / `upgrade-skill` / `enrich-skill`: Agent workflow skills

### Modified Capabilities
- `--json`: Extended to all commands that don't yet support it

## Non-Goals

- npm global distribution or simplified installer UX (deferred per p3-polish-benchmarks)
- Room-level palace filtering (deferred until benchmarks prove value)
- LLM-assisted contradiction detection (agent-side, not binary)
- WASM compilation
- Overnight consolidation cycle (agent configuration, not binary)
- GPU acceleration / CUDA / Metal (deferred)

## Impact

- `skills/*/SKILL.md` — 5 stub files rewritten to production content
- `src/commands/validate.rs`, `call.rs`, `pipe.rs`, `skills.rs` — stub → implementation
- `src/mcp/server.rs` — 4 new MCP tools
- `benchmarks/` — evaluation harness files
- `.github/workflows/ci.yml` — benchmark CI gates
- `src/main.rs` — `--json` wiring for any missing commands

## Risks / Trade-offs

**Risk: BEIR harness build time in CI** → Mitigation: run benchmarks as a separate CI job,
not blocking PR merge. Only block releases.

**Risk: LongMemEval/LoCoMo/Ragas require API keys** → Mitigation: classify as advisory.
Run manually before major releases, not in CI.

**Risk: Skill content quality** → Mitigation: skills are markdown consumed by agents.
Quality validated by agent testing, not unit tests. Review by Leela and Professor.

**Risk: validate --embeddings performance on large brains** → Mitigation: implement with
streaming row-by-row checks, not full table load. Add `--limit` for partial checks.
