# Graph-aware retrieval — acceptance gate

This document encodes the release acceptance gate for the knowledge-graph
layer (OpenSpec change `knowledge-graph-layer`). It is the operational
form of the spec requirement *"Acceptance is gated on benchmark
improvements"* in `specs/graph-aware-retrieval/spec.md`.

**Status as of 2026-05-13:** Baseline and post-change measurements have
**not** been performed in this change. Numbers must be recorded before
graph-aware retrieval is accepted as a default-on capability.

## Thresholds

A release shipping graph-aware retrieval as default-on (`config.graph_depth
≥ 1`) MUST satisfy both of the following against a reproducible bge-small
baseline:

| Benchmark | Metric | Threshold |
| --------- | ------ | --------- |
| DAB §4 Semantic / Hybrid | absolute score | ≥ 35 / 50 |
| DAB §4 Semantic / Hybrid | delta vs bge-small baseline | ≥ +8 points |
| MSMARCO P@5 | delta vs bge-small baseline | ≥ +5 points |

If either threshold misses, the release MAY still ship the autowiring,
path-output, and `quaid graph extract-entities` features, but the graph
expansion default MUST be turned off:

```sql
UPDATE config SET value = '0' WHERE key = 'graph_depth';
```

The schema-side default of `1` stays as the *aspirational* default for a
future release that passes the gate.

## Reproducible measurement procedure

The bge-small baseline and the post-change run share a single procedure
so deltas are meaningful:

1. **Build** the release binary in the `embedded-model` (airgapped)
   channel: `cargo build --release`.
2. **Initialize** a fresh memory: `quaid init ./bench.db`.
3. **Disable** graph expansion for the baseline: `quaid config set
   graph_depth 0 --db ./bench.db`.
4. **Run** the DAB §4 corpus + queries (see `benchmarks/dab_section8.py`
   for the §8 wrapper that uses the same real-pipeline helpers) and
   record the score; commit numerics to `benchmarks/baselines/`.
5. **Run** MSMARCO P@5 against the pinned corpus subset and record.
6. **Enable** graph expansion: `quaid config set graph_depth 1 --db
   ./bench.db`.
7. **Re-run** both benchmarks and compute deltas.

The corpus and query lists MUST be pinned in `benchmarks/datasets.lock`
before publishing numerics, mirroring the existing BEIR gating
convention.

## Recording results

Append measured runs to a new `benchmarks/baselines/graph_retrieval.json`
file alongside `beir.json` and `conversation_memory.json`. Each entry must
record: `date`, `embedding_model`, `corpus`, `graph_depth`,
`dab_section4_score`, `msmarco_p5`, and the binary commit SHA.

## Why this is gated as a documented procedure rather than measured numbers

The Wave 7 implementation pass did not have access to representative
hardware or the pinned DAB §4 / MSMARCO corpora under release CI. Rather
than fabricate numbers, this gate is encoded as a release-time procedure
consistent with the existing manual / advisory benchmarks in
`benchmarks/README.md`. The first release that ships the graph layer is
expected to run the procedure above, record the numerics, and flip
`graph_depth` to `0` if the gate misses.
