# GigaBrain Benchmarks

This directory contains the benchmark harness for GigaBrain.

## Overview

Benchmarks are split into two categories:

### Offline CI Gates (mandatory — block release)

These run entirely locally with no API keys required.

| Benchmark | Metric | Gate |
|-----------|--------|------|
| BEIR (NQ + FiQA subset) | nDCG@10 | No regression > 2% vs baseline |
| Corpus-reality tests | Pass/fail | All scenarios must pass |
| Concurrency stress | OCC invariants | Zero data corruption |
| Embedding migration | Cross-model contamination | Zero contamination |
| Round-trip integrity (semantic) | Semantic diff | Must be zero |
| Round-trip integrity (byte-exact) | Byte diff | Must be zero |
| Static binary verification | `ldd` / `file` | Must be statically linked |

### Advisory Benchmarks (run manually before major releases)

| Benchmark | Metric | Target |
|-----------|--------|--------|
| LongMemEval | R@5 | ≥ 85% |
| LoCoMo | F1 | ≥ +30% over FTS5 baseline |
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
- **Implementation:** `gbrain query` with default parameters

### Query Set & Results

| # | Query | Expected Relevant | Top-1 Result | Hit@1 | Hit@3 | nDCG@10 |
|---|-------|-------------------|--------------|-------|-------|---------|
| 1 | who founded brex | people/pedro-franceschi OR people/henrique-dubugras | people/pedro-franceschi | ✓ | ✓ | 1.0000 |
| 2 | technology company developer tools | companies/acme | companies/acme | ✓ | ✓ | 1.0000 |
| 3 | knowledge brain sqlite embeddings | projects/gigabrain | projects/gigabrain | ✓ | ✓ | 1.0000 |
| 4 | corporate card fintech startup | companies/brex | companies/brex | ✓ | ✓ | 1.0000 |
| 5 | brazilian entrepreneur yc | people/pedro-franceschi OR people/henrique-dubugras | people/henrique-dubugras | ✓ | ✓ | 1.0000 |
| 6 | rust sqlite vector search | projects/gigabrain | projects/gigabrain | ✓ | ✓ | 1.0000 |
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

## Structure (to be created in Phase 3)

```
benchmarks/
├── datasets.lock           # Pinned dataset commit hashes
├── beir_eval.rs            # BEIR nDCG@10 harness
├── longmemeval_adapter.py  # LongMemEval format adapter
├── locomo_eval.py          # LoCoMo evaluation
├── ragas_eval.py           # Ragas quality metrics
├── requirements.txt        # Python deps for advisory benchmarks
└── datasets/               # gitignored — downloaded by prep scripts
```

## Dataset Pinning

All datasets MUST be pinned to specific commit hashes in `datasets.lock`.
Never use HEAD or floating references in CI.

## Running Benchmarks

### Phase 1 (Current)

```bash
# Reproduce Phase 1 baseline
./target/release/gbrain --db bench_brain.db init
./target/release/gbrain --db bench_brain.db import tests/fixtures/
./target/release/gbrain --db bench_brain.db query "who founded brex"
# ... repeat for all 8 queries
```

### Phase 3 (Planned)

```bash
cargo test --test beir_eval
cargo test --test corpus_reality
cargo test --test concurrency_stress
cargo test --test embedding_migration
cargo test --test roundtrip
```
