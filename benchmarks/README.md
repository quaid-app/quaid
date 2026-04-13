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

## Running Offline Gates

```bash
# After Phase 3 implementation:
cargo test --test beir_eval
cargo test --test corpus_reality
cargo test --test concurrency_stress
cargo test --test embedding_migration
cargo test --test roundtrip
```
