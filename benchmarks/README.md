# Quaid Benchmarks

This directory contains the benchmark harness for Quaid.

## Overview

Benchmarks are split into two categories:

### Offline CI Gates (mandatory — block release)

These run entirely locally with no API keys required. They are wired into CI to run on every PR.

| Benchmark | Metric | Gate |
|-----------|--------|------|
| BEIR (NQ + FiQA subset) | nDCG@10 | No regression > 2% vs baseline |
| Corpus-reality tests | Pass/fail | All scenarios must pass |
| Concurrency stress | OCC invariants | Zero data corruption |
| Embedding migration | Cross-model contamination | Zero contamination |
| Round-trip integrity (semantic) | Semantic diff | Must be zero |
| Round-trip integrity (byte-exact) | Byte diff | Must be zero |
| Static binary verification | `ldd` / `file` | Must be statically linked |

### Manual / advisory benchmark gates

| Benchmark | Metric | Target / Gate |
|-----------|--------|---------------|
| DAB §8 Conversation Memory | LoCoMo token-F1 + LongMemEval answer-hit@5 | No subsection regression > 3.0 points vs baseline on full representative-hardware runs |
| LongMemEval (raw-page path) | R@5 | ≥ 85% |
| LoCoMo (raw-page path) | F1 delta vs FTS5 | ≥ +30% |
| Ragas | context_precision, context_recall | Advisory |

---

## Phase 1 Baseline — BEIR-style nDCG@10 Proxy

**Established:** 2026-04-15
**Embedding Model:** SHA-256 hash-based shim (non-semantic, deterministic)
**Note:** This baseline uses hash-based embeddings. Full semantic evaluation with BGE-small-en-v1.5 will be recorded after T14 completes.

### Methodology

- **Corpus:** 5 fixture pages from `tests/fixtures/` (2 people, 2 companies, 1 project)
- **Queries:** 8 synthetic queries with ground-truth relevance judgments
- **Metric:** nDCG@10 (normalized discounted cumulative gain at rank 10, binary relevance)
- **Search mode:** Hybrid (FTS5 + vector, set-union merge)
- **Implementation:** `quaid query` with default parameters

### Query Set & Results

| # | Query | Expected Relevant | Top-1 Result | Hit@1 | Hit@3 | nDCG@10 |
|---|-------|-------------------|--------------|-------|-------|---------|
| 1 | who founded brex | people/pedro-franceschi OR people/henrique-dubugras | people/pedro-franceschi | ✓ | ✓ | 1.0000 |
| 2 | technology company developer tools | companies/acme | companies/acme | ✓ | ✓ | 1.0000 |
| 3 | quaid memory sqlite embeddings | projects/quaid | projects/quaid | ✓ | ✓ | 1.0000 |
| 4 | corporate card fintech startup | companies/brex | companies/brex | ✓ | ✓ | 1.0000 |
| 5 | brazilian entrepreneur yc | people/pedro-franceschi OR people/henrique-dubugras | people/henrique-dubugras | ✓ | ✓ | 1.0000 |
| 6 | rust sqlite vector search | projects/quaid | projects/quaid | ✓ | ✓ | 1.0000 |
| 7 | developer productivity apis | companies/acme | companies/acme | ✓ | ✓ | 1.0000 |
| 8 | brex cto technical leadership | people/henrique-dubugras | people/henrique-dubugras | ✓ | ✓ | 1.0000 |

### Aggregate Metrics

- **Hit@1:** 8/8 = **100.0%**
- **Hit@3:** 8/8 = **100.0%**
- **nDCG@10:** **1.0000**

### Interpretation

Phase 1 establishes a perfect baseline on a small synthetic corpus. This is expected given:
1. Only 5 pages in the corpus (limited noise)
2. Queries designed to have clear ground-truth mappings
3. Hash-based embeddings still capture lexical overlap effectively on this scale

**Next Steps:**
- Establish semantic baseline with BGE-small-en-v1.5 after T14 completes
- Expand to BEIR subsets (NFCorpus, FiQA, NQ) for adversarial evaluation in Phase 3
- Set regression gates: no more than 2% drop in nDCG@10 on expanded corpus

### Latency Benchmarks (Phase 1)

| Operation | p50 (wall-clock) | Notes |
|-----------|------------------|-------|
| FTS5 search (5 docs) | ~155ms | Cold start, includes DB open |
| Hybrid query (5 docs) | ~420ms | Cold start, includes embedding |
| Import (5 files) | ~3.7s | Full ingest + embedding pipeline |

*Latencies measured on development container (Linux x86_64), release build, single iteration. p50 approximated from 5-iteration samples.*

---

## Structure

```
benchmarks/
├── datasets.lock               # Pinned dataset commit hashes and archive SHAs
├── prep_datasets.sh            # Download and verify pinned datasets
├── requirements.txt            # Python deps for advisory benchmarks
├── baselines/
│   ├── beir.json               # Regression anchor: nDCG@10 baseline
│   └── conversation_memory.json # DAB §8 regression anchor
├── conversation_memory_common.py # Shared real-pipeline helpers for §8 runs
├── dab_section8.py             # DAB §8 wrapper: LoCoMo + LongMemEval
├── beir_eval.rs  → tests/      # BEIR nDCG@10 harness (cargo test --test beir_eval)
├── longmemeval_adapter.py      # LongMemEval R@5 adapter
├── locomo_eval.py              # LoCoMo F1 vs FTS5 baseline
├── ragas_eval.py               # Ragas advisory quality metrics
└── datasets/                   # gitignored — downloaded by prep_datasets.sh
```

## Dataset Pinning

All datasets MUST be pinned to specific commit hashes or archive SHA-256 hashes
in `benchmarks/datasets.lock`. Never use HEAD or floating references in CI.

To update pins after downloading new archive versions:

```bash
./benchmarks/prep_datasets.sh --compute-hashes
# Update SHA-256 values in datasets.lock
```

---

## Running Benchmarks

### Phase 1 (Reproduce synthetic baseline)

```bash
# Reproduce Phase 1 baseline — 5 fixture pages, 8 synthetic queries
cargo build --release
./target/release/quaid --db bench_memory.db init
./target/release/quaid --db bench_memory.db import tests/fixtures/
./target/release/quaid --db bench_memory.db query "who founded brex"
# ... repeat for all 8 queries in baselines/beir.json
```

### Phase 3 — Offline CI gates (mandatory, no API keys)

These run in CI on every PR. They use the fixture corpus and library APIs directly.

```bash
# All offline tests (runs automatically in CI)
cargo test --test corpus_reality
cargo test --test concurrency_stress
cargo test --test embedding_migration
cargo test --test roundtrip_semantic
cargo test --test roundtrip_raw

# BEIR regression (requires datasets — manual or release-branch CI)
./benchmarks/prep_datasets.sh nq fiqa
cargo test --test beir_eval -- --ignored

# DAB §8 Conversation Memory
# Full regression gate: run manually on representative Unix hardware with local model cache
./benchmarks/prep_datasets.sh locomo longmemeval
python benchmarks/dab_section8.py --json

# Extraction per-window latency gate
# Manual representative-hardware run; hosted Windows/macOS Intel environments skip
cargo bench --bench extraction
```

### DAB §8 — Conversation Memory gate

This section is the conversation-memory benchmark surface for the SLM extractor.

- **Harness:** `benchmarks/dab_section8.py`
- **Subsections:** LoCoMo + LongMemEval
- **Real path exercised:** `memory_add_turn` → `memory_close_session` → `quaid serve` worker → extracted fact pages → `quaid query` / `quaid search`
- **Baseline anchor:** `benchmarks/baselines/conversation_memory.json`
- **Regression rule:** once baselines are established, neither subsection may drop by more than **3.0 percentage points** version-over-version

#### Truth boundary

- The authoritative gate is a **full** run on **representative Unix hardware** with **no `--limit`**.
- GitHub-hosted runners are not representative hardware for this path. The repo therefore ships a **manual smoke hook** (`.github/workflows/conversation-memory-benchmarks.yml`) instead of claiming an always-on CI gate.
- Until a full representative-hardware baseline is recorded, the gate reports `pending-baseline` rather than pass/fail.
- LongMemEval's dataset evidence ids point at session-level documents, not extracted fact slugs. The in-repo §8 subsection therefore uses a truthful proxy metric: **answer-hit@5** on retrieved fact pages (hit = top-5 max token-F1 ≥ 0.5 against the gold answer).

#### Commands

```bash
# Full representative-hardware run (authoritative gate)
./benchmarks/prep_datasets.sh locomo longmemeval
python benchmarks/dab_section8.py --json

# Hosted-runner / smoke run (informational only; skips regression comparison)
python benchmarks/dab_section8.py --limit 25 --json

# Run just one subsection
python benchmarks/dab_section8.py --dataset locomo
python benchmarks/dab_section8.py --dataset longmemeval

# Manual extraction latency gate (requires staged local model cache)
cargo bench --bench extraction
```

### Phase 3 — Advisory benchmarks (manual, before major releases)

Advisory benchmarks require API keys or a local Ollama instance and downloaded datasets.
They are informational — results track quality but do not block releases.

#### Prerequisites

```bash
# 1. Build the binary
cargo build --release

# 2. Download pinned datasets
./benchmarks/prep_datasets.sh          # all datasets (~600 MB)
./benchmarks/prep_datasets.sh locomo   # LoCoMo only (~50 MB)
./benchmarks/prep_datasets.sh longmemeval  # LongMemEval only

# 3. Install Python dependencies
python -m venv .venv && source .venv/bin/activate
pip install -r benchmarks/requirements.txt
```

#### LongMemEval — Multi-session memory (R@5 target: ≥ 85%)

```bash
# With OpenAI LLM judge (optional for answer scoring)
OPENAI_API_KEY=sk-... python benchmarks/longmemeval_adapter.py

# Point at an existing brain
python benchmarks/longmemeval_adapter.py --db ~/memory.db --limit 200

# JSON output for logging
python benchmarks/longmemeval_adapter.py --json > results/longmemeval.json

# Exercise the real conversation-memory extractor instead of page import
python benchmarks/longmemeval_adapter.py --mode conversation-memory --limit 25 --json
```

Expected runtime: ~5–20 minutes depending on corpus size and query limit.
No API key required for retrieval evaluation (only for answer grading).

#### LoCoMo — Conversational memory (pre-extraction advisory: ≥ +30% F1 delta over FTS5; §8 gate: ≥ 40% absolute F1 on extraction-produced fact pages)

```bash
# Evaluate hybrid vs FTS5 baseline
python benchmarks/locomo_eval.py

# FTS5 baseline only
python benchmarks/locomo_eval.py --baseline-only

# Point at existing brain
python benchmarks/locomo_eval.py --db ~/memory.db --limit 50 --json

# Exercise the real conversation-memory extractor instead of page import
python benchmarks/locomo_eval.py --mode conversation-memory --limit 25 --json
```

Expected runtime: ~2–10 minutes.
No API key required — uses token-level F1, not LLM judge.

#### Ragas — Answer quality metrics (advisory, no gate)

Ragas requires an LLM judge (OpenAI or Ollama). It evaluates:
- `context_precision` — are retrieved contexts relevant to the query?
- `context_recall` — do retrieved contexts cover the expected answer?
- `faithfulness` — does the answer stay grounded in context?
- `answer_relevancy` — is the answer relevant to the question?

```bash
# With OpenAI (gpt-4o-mini by default)
OPENAI_API_KEY=sk-... python benchmarks/ragas_eval.py

# With local Ollama (run: ollama pull llama3.2 first)
ollama pull llama3.2
python benchmarks/ragas_eval.py --llm ollama

# Dry run — no API calls
python benchmarks/ragas_eval.py --dry-run

# Custom brain
OPENAI_API_KEY=sk-... python benchmarks/ragas_eval.py --db ~/memory.db --limit 20
```

**Ollama setup:**
```bash
# Install: https://ollama.ai
curl -fsSL https://ollama.ai/install.sh | sh
ollama pull llama3.2        # ~2 GB download
ollama serve                # starts on port 11434
# Then:
python benchmarks/ragas_eval.py --llm ollama
```

Expected runtime: ~5–30 minutes depending on LLM latency and query count.

---

## BEIR Regression Gate

The BEIR gate is designed to run on release branches via a dedicated CI job in `.github/workflows/ci.yml`. **Note:** CI wiring for the benchmark jobs (tasks 7.1–7.2) is pending — run manually until the workflow is updated.

```bash
# Download pinned datasets first
./benchmarks/prep_datasets.sh nq fiqa

# Run regression check (fails if nDCG@10 drops > 2% from baseline)
cargo test --test beir_eval -- --ignored

# Update baseline after intentional improvement
# Edit benchmarks/baselines/beir.json with new nDCG@10 values
```

Baseline anchor: `benchmarks/baselines/beir.json`
Regression threshold: 2% drop in nDCG@10 fails the gate.

## Conversation Memory Regression Gate

Baseline anchor: `benchmarks/baselines/conversation_memory.json`
Regression threshold: 3.0 percentage-point drop in either §8 subsection fails the gate.
Current status: baseline pending until the first full representative-hardware run is recorded.
