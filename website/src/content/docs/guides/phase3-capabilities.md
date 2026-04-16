---
title: Phase 3 Capabilities
description: Skills completion, CLI polish, new MCP tools, and benchmark harnesses — the Phase 3 feature set shipped through v0.9.1.
sidebar:
  badge:
    text: v0.9.1
    variant: success
---

Phase 3 completes GigaBrain's flight manual and instruments: all 8 agent skills are production-ready, four new MCP tools round out the 16-tool surface, and four previously-stubbed CLI commands are fully implemented.

---

## Production-ready skills

All 8 SKILL.md files are authored, reviewed, and agent-executable. Skills are markdown files embedded in the binary and extracted to `~/.gbrain/skills/` on first run. Drop a custom `SKILL.md` in your working directory to override any default.

| Skill | What it does |
| ----- | ------------ |
| `skills/ingest/` | Meeting transcripts, articles, documents — novelty-check, parse, write |
| `skills/query/` | Search and synthesis workflows — hybrid query, progressive retrieval |
| `skills/maintain/` | Lint, contradiction detection, orphan page cleanup |
| `skills/briefing/` | "What shifted" daily report — changed pages, new contradictions, open gaps |
| `skills/alerts/` | Interrupt-driven notifications — new contradictions, stale pages, resolved gaps |
| `skills/research/` | Knowledge gap resolution — prioritize gaps, ingest findings, mark resolved |
| `skills/upgrade/` | Agent-guided binary and skill updates — version check, download, verify, validate |
| `skills/enrich/` | External data integration — Crustdata, Exa, Partiful enrichment patterns |

### Inspect skills

```bash
# List active skills and their sources
gbrain skills

# Show resolution order and content hashes
gbrain skills doctor
```

The `doctor` subcommand shows whether a skill is embedded, user-overridden, or shadowed, and verifies the YAML frontmatter is intact.

---

## New CLI commands

### `validate` — database integrity checker

```bash
# Run all checks
gbrain validate --all

# Targeted checks
gbrain validate --links        # referential integrity + temporal ordering
gbrain validate --assertions   # dedup, supersession chains, dangling page_id
gbrain validate --embeddings   # exactly one active model; all embeddings use active model

# JSON output
gbrain validate --all --json
```

**Example JSON output:**
```json
{
  "passed": true,
  "checks": ["links", "assertions", "embeddings"],
  "violations": []
}
```

If violations are found, `validate` exits with code 1:
```json
{
  "passed": false,
  "checks": ["links"],
  "violations": [
    {
      "check": "links",
      "type": "dangling_reference",
      "details": { "link_id": 42, "to_slug": "people/missing" }
    }
  ]
}
```

### `call` — raw MCP tool invocation

Invoke any of the 16 MCP tools directly from the command line. Useful for debugging and scripting.

```bash
# Search via MCP tool
gbrain call brain_search '{"query":"river ai","limit":10}'

# Log a knowledge gap
gbrain call brain_gap '{"query":"who funds acme?","context":"company research"}'

# Get brain statistics
gbrain call brain_stats '{}'
```

Unknown tool names exit with code 1 and print `{"error": "unknown tool: <name>"}` to stderr.

### `pipe` — JSONL streaming mode

Read one JSON request per line from stdin, invoke the named tool, write one JSON result per line to stdout. Useful for shell pipelines and batch automation.

```bash
# Request format (one per line):
# {"tool": "<name>", "input": {...}}

echo '{"tool":"brain_search","input":{"query":"machine learning"}}' | gbrain pipe

# Batch requests from a file
cat requests.jsonl | gbrain pipe > results.jsonl
```

On parse errors or unknown tools, `pipe` writes `{"error": "..."}` to stdout (not stderr) and continues reading.

---

## New MCP tools (Phase 3)

Four new tools complete the 16-tool MCP surface. See the [MCP Server guide](/guides/mcp-server/) for full call examples.

| Tool | Description |
| ---- | ----------- |
| `brain_gap` | Log a knowledge gap with optional context |
| `brain_gaps` | List unresolved (or resolved) knowledge gaps |
| `brain_stats` | Brain statistics: page count, link count, contradiction count, db size |
| `brain_raw` | Store raw structured data (API responses, JSON) attached to a page |

---

## Benchmark harnesses

### Offline benchmarks (CI-gated)

These run on every PR and are mandatory gates:

```bash
# Run all offline benchmark tests
cargo test --test corpus_reality
cargo test --test concurrency_stress
cargo test --test embedding_migration
```

| Benchmark | What it measures |
| --------- | ---------------- |
| `corpus_reality` | Import completeness, SMS retrieval, timeline retrieval, idempotent round-trip, FTS5 coverage |
| `concurrency_stress` | Parallel OCC, duplicate ingest, WAL compact under load |
| `embedding_migration` | Zero cross-model contamination when switching embedding models |

### BEIR regression gate (release-branch only)

```bash
# Download pinned datasets first
bash benchmarks/prep_datasets.sh

# Run BEIR nDCG@10 regression (--ignored = offline flag)
cargo test --test beir_eval -- --ignored
```

Fails if nDCG@10 regresses more than 2% against `benchmarks/baselines/beir.json`.

### Advisory benchmarks (Python, API-key optional)

These run manually before major releases and support Ollama as a no-key judge:

```bash
cd benchmarks

# LongMemEval — R@5 multi-session memory (target ≥ 85%)
python longmemeval_adapter.py --db ~/brain.db --split dev --limit 100

# LoCoMo — F1 conversational memory (target: +30% vs FTS5 baseline)
python locomo_eval.py --db ~/brain.db

# Ragas — context precision/recall/faithfulness (advisory, no threshold)
python ragas_eval.py --db ~/brain.db --judge ollama   # no API key required
```

See `benchmarks/README.md` for prerequisites, Ollama setup, and expected runtimes.

---

## Related

- [CLI Reference](/reference/cli/) — complete flag and subcommand reference
- [MCP Server](/guides/mcp-server/) — all 16 MCP tool examples
- [Intelligence Layer](/guides/intelligence-layer/) — Phase 2 graph, contradiction, and gap features
- [Roadmap](/contributing/roadmap/) — full phase delivery plan
