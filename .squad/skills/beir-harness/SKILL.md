# Skill: BEIR Harness Pattern (Rust)

## What it solves

How to wire a BEIR-style retrieval benchmark in Rust integration tests.

## Pattern

```
tests/beir_eval.rs
  - #[ignore] on all dataset-dependent tests
  - nDCG computation unit tests always run (no #[ignore])
  - Load corpus.jsonl, queries.jsonl, qrels/test.tsv
  - Import into temp brain via import_dir()
  - embed::run() to index
  - hybrid_search() for each query → ranked slugs
  - Compute mean nDCG@10
  - Compare against baselines/beir.json with 2% regression threshold
```

## Key functions

```rust
fn dcg(retrieved: &[String], relevant: &HashMap<String, u32>, k: usize) -> f64
fn idcg(relevant: &HashMap<String, u32>, k: usize) -> f64
fn ndcg_at_k(retrieved, relevant, k) -> f64
fn mean_ndcg_at_10(results: &[(Vec<String>, HashMap<String, u32>)]) -> f64
```

## FTS5 query caution

FTS5 treats `-term` as NOT-term. Avoid leading hyphens in benchmark queries:
- ❌ "Co-founded Brex" → FTS5 error (Co AND NOT founded)
- ✓ "Brex founder Dubugras" → clean keyword search

## Baseline file format

```json
{
  "regression_threshold_pct": 2.0,
  "baselines": {
    "fiqa": { "ndcg_at_10": 0.412, "status": "established" }
  }
}
```

## First-run workflow

1. `./benchmarks/prep_datasets.sh fiqa`
2. `cargo test --test beir_eval -- --ignored fiqa`
3. Record nDCG@10 in `benchmarks/baselines/beir.json`
4. Subsequent runs assert `score >= baseline * 0.98`
