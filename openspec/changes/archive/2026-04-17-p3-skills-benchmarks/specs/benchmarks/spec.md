## ADDED Requirements

### Requirement: BEIR retrieval regression gate

The BEIR benchmark harness SHALL evaluate hybrid search quality on NQ and FiQA subsets
using nDCG@10. It SHALL run entirely offline with no API keys. Results SHALL be compared
against a pinned baseline; regression greater than 2% SHALL fail the CI gate.

#### Scenario: BEIR evaluation run
- **WHEN** `cargo test --test beir_eval` is executed
- **THEN** the harness imports the pinned NQ+FiQA corpus, runs queries, computes nDCG@10,
  and compares against the baseline in `benchmarks/baselines/beir.json`

#### Scenario: Regression detected
- **WHEN** nDCG@10 drops by more than 2% from baseline
- **THEN** the test fails with a message showing current vs baseline scores

### Requirement: LongMemEval multi-session memory benchmark

The LongMemEval adapter SHALL convert gbrain queries to LongMemEval format and evaluate
R@5 (Recall at 5). Target: ≥ 85%. This is advisory (requires API key for LLM judge).

#### Scenario: LongMemEval evaluation
- **WHEN** `python benchmarks/longmemeval_adapter.py` is executed with `OPENAI_API_KEY` set
- **THEN** it imports LongMemEval sessions, runs retrieval queries through gbrain, and
  reports R@5

### Requirement: LoCoMo conversational memory benchmark

The LoCoMo adapter SHALL evaluate F1 on single-iteration retrieval. Target: ≥ +30% over
naive FTS5 baseline. This is advisory (API-dependent).

#### Scenario: LoCoMo evaluation
- **WHEN** `python benchmarks/locomo_eval.py` is executed
- **THEN** it imports LoCoMo conversations, evaluates hybrid search F1, and compares
  against FTS5-only baseline

### Requirement: Ragas answer quality metrics

The Ragas harness SHALL evaluate context_precision, context_recall, and faithfulness for
progressive retrieval results. Advisory, not a release gate.

#### Scenario: Ragas evaluation
- **WHEN** `python benchmarks/ragas_eval.py` is executed
- **THEN** it runs progressive retrieval queries and evaluates with Ragas metrics

### Requirement: Corpus-reality tests

Corpus-reality tests SHALL validate:
1. Import 7K+ file corpus → zero page loss
2. Known entity retrieval → correct page in top 1 (SMS test)
3. Known timeline fact → correct entry in top 5
4. Duplicate ingest → no duplicate timeline/assertions
5. Conflicting source ingest → contradiction detected
6. Normalized export → reimport → export → semantic diff = 0
7. 100 queries → p95 latency < 250ms

#### Scenario: Import completeness
- **WHEN** a 7K+ file corpus is imported
- **THEN** `gbrain stats` shows the same number of pages as source files

#### Scenario: Idempotent round-trip
- **WHEN** a corpus is exported, reimported, and re-exported
- **THEN** semantic diff between the two exports is zero

### Requirement: Concurrency stress tests

Concurrency tests SHALL validate OCC safety invariants:
1. 4 threads calling `brain_put` on same slug with stale version → all but one get ConflictError
2. 2 threads ingesting same source → exactly one succeeds
3. kill -9 before COMMIT → clean state on retry
4. `gbrain compact` during open reader → both succeed

#### Scenario: Parallel OCC enforcement
- **WHEN** 4 threads write to the same page with stale `expected_version`
- **THEN** exactly one succeeds, three receive ConflictError, zero data corruption

### Requirement: Embedding migration correctness

Embedding migration tests SHALL verify:
1. Embed with model A, run 20 queries, record top-5
2. Re-embed with model B, flip active flag
3. Same 20 queries → results from model B only
4. Rollback to model A → original results return

#### Scenario: Zero cross-model contamination
- **WHEN** the active model is switched from A to B
- **THEN** no results from model A's vec table leak through

### Requirement: Dataset pinning

All benchmark datasets SHALL be pinned to specific commit hashes in
`benchmarks/datasets.lock`. A prep script SHALL download pinned versions to
`benchmarks/datasets/` (gitignored). CI SHALL cache this directory.

#### Scenario: Dataset preparation
- **WHEN** `./benchmarks/prep_datasets.sh` is executed
- **THEN** all datasets are downloaded at their pinned versions to `benchmarks/datasets/`
