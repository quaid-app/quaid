# Phase 3 Execution Checklist — Skills, Benchmarks, and CLI Polish

**Lead:** Fry
**Implementers:** Fry (CLI/MCP/benchmarks), Leela (skills content review), Kif (benchmark harness)
**Reviewers:** Professor (validate/MCP correctness), Nibbler (MCP adversarial), Scruffy (benchmark coverage)

---

## 1. Fry — Skills completion (SKILL.md authoring)

- [x] 1.1 Author `skills/briefing/SKILL.md`: define "what shifted" report workflow — list changed pages via `truth_updated_at`/`timeline_updated_at` within lookback window, summarize new pages, include unresolved contradictions from `gbrain check --all`, top knowledge gaps from `gbrain gaps`, and upcoming timeline entries. Define output format (markdown sections), configurable lookback (default 1 day), and example agent invocation sequence.
- [x] 1.2 Author `skills/alerts/SKILL.md`: define interrupt-driven alert triggers — new contradiction detected (high priority), knowledge gap resolved (low), page stale >30 days (timeline_updated_at > truth_updated_at by 30+ days) with >5 inbound links (medium), embedding drift detected (low). Define priority levels (critical/high/medium/low), delivery mechanism (stdout structured output), deduplication rules (same alert type + same slug within 24h = suppress), and suppression configuration.
- [x] 1.3 Author `skills/research/SKILL.md`: define knowledge gap resolution workflow — fetch unresolved gaps via `gbrain gaps --limit 10`, assess sensitivity (internal-only by default), generate research queries from gap context, ingest findings via ingest skill, mark gap resolved via `brain_gap_approve` + resolution slug. Define sensitivity escalation workflow (`internal` → `external` requires explicit approval), redacted query generation (strip entity names), and Exa integration patterns.
- [x] 1.4 Author `skills/upgrade/SKILL.md`: define agent-guided update workflow — check current version via `gbrain version`, fetch latest release metadata from GitHub Releases API, download binary + SHA-256 checksum, verify checksum before replacing, run `gbrain validate --all` after upgrade, update skills if new versions bundled. Define rollback procedure (keep previous binary as `.bak`), version pinning rules (skills declare minimum binary version), and failure modes.
- [x] 1.5 Author `skills/enrich/SKILL.md`: define external data enrichment patterns — Crustdata (company/person professional data: funding, headcount, roles), Exa (web search + content extraction), Partiful (event/social data). Define storage flow (enrichment → `brain_raw` → extract facts → update `compiled_truth` + `assertions`), conflict resolution (enrichment contradicts existing truth → log contradiction, don't auto-overwrite), and rate limiting guidance.

## 2. Fry — CLI stub implementation (validate, call, pipe, skills doctor)

- [x] 2.1 Implement `src/commands/validate.rs`: modular integrity checker with `--links` (check link interval non-overlap via `SELECT` for overlapping `valid_from`/`valid_until` on same from_page_id+to_page_id+relationship, temporal ordering `valid_from <= valid_until`, referential integrity from_page_id/to_page_id exist in pages), `--assertions` (dedup check for duplicate subject+predicate+object with overlapping validity, supersession chain where supersedes_id references valid assertion, dangling page_id), `--embeddings` (exactly one `active=1` in `embedding_models`, all `page_embeddings.model_id` = active model ID, all vec_rowids resolve in active vec table), `--all` runs all. Return structured `Vec<Violation>`. Exit 0 clean, exit 1 violations.
- [x] 2.2 Add `--json` output to `validate.rs`: when `--json` flag is set, output `{"passed": bool, "checks": ["links","assertions","embeddings"], "violations": [{"check":"links","type":"dangling_reference","details":{...}},...]}`.
- [x] 2.3 Implement `src/commands/call.rs`: parse tool name and JSON input, match tool name to MCP handler function, deserialize input to the tool's input struct, invoke handler, serialize result to stdout. Support all 16 MCP tools. Print `{"error": "unknown tool: <name>"}` to stderr for unknown tools, exit 1.
- [x] 2.4 Implement `src/commands/pipe.rs`: read stdin line-by-line, parse each line as `{"tool": "<name>", "input": {...}}`, invoke the tool handler (reuse call.rs dispatch logic), write result as one JSON line to stdout. On parse error or unknown tool, write `{"error": "..."}` to stdout (not stderr — pipe protocol). Continue until EOF.
- [x] 2.5 Implement `src/commands/skills.rs` `List` action: scan skill resolution order (embedded → `~/.gbrain/skills/` → `./skills/`), list active skills with source path. Support `--json` for structured output.
- [x] 2.6 Implement `src/commands/skills.rs` `Doctor` action: for each skill, compute SHA-256 of resolved content, detect shadowing (external overrides embedded), verify skill format (YAML frontmatter present, required fields `name` and `description`). Output resolution table. Support `--json`.
- [x] 2.7 Wire `validate`, `call`, `pipe`, and `skills` commands in `src/main.rs`: pass `--json` flag to validate. Pass `Connection` to call and pipe. Ensure clap subcommands are correctly dispatched.

## 3. Fry — MCP Phase 3 tools (brain_gap, brain_gaps, brain_stats, brain_raw)

- [x] 3.1 Add `BrainGapInput { query: String, context: Option<String> }` struct and `brain_gap` tool method in `src/mcp/server.rs`. Validate query non-empty (reject with `-32602`). Delegate to `core::gaps::log_gap(query, context, None, conn)`. Return `{"id": <gap_id>, "query_hash": "<hash>"}`.
- [x] 3.2 Add `BrainGapsInput { resolved: Option<bool>, limit: Option<u32> }` struct and `brain_gaps` tool method. Default resolved=false, limit=20, max 1000. Delegate to `core::gaps::list_gaps`. Return JSON array of gap objects.
- [x] 3.3 Add `BrainStatsInput {}` (no fields) struct and `brain_stats` tool method. Query `SELECT COUNT(*) FROM pages`, `links`, `assertions`, `knowledge_gaps WHERE resolved_at IS NULL`, `page_embeddings`. Query `embedding_models WHERE active=1` for active model name. Use `PRAGMA page_count * PRAGMA page_size` for db_size_bytes. Return JSON object.
- [x] 3.4 Add `BrainRawInput { slug: String, source: String, data: serde_json::Value }` struct and `brain_raw` tool method. Validate slug with `validate_slug`. Look up page by slug (return `-32001` if not found). `INSERT INTO raw_data (page_id, source, data, fetched_at) VALUES (?, ?, ?, datetime('now'))`. Return `{"id": <row_id>}`.
- [x] 3.5 Write MCP tests: `brain_gap` with empty query returns `-32602`; `brain_gap` stores gap with NULL query_text and internal sensitivity; duplicate `brain_gap` is idempotent; `brain_gaps` returns array with limit; `brain_stats` returns all expected fields; `brain_raw` with unknown slug returns `-32001`; `brain_raw` with valid slug stores row.

## 4. Fry — --json coverage audit and completion

- [x] 4.1 Audit all commands in `src/commands/` for `--json` support: verify that every `run()` function that receives a `json: bool` parameter actually uses it. List any commands that ignore the flag or don't accept it.
- [x] 4.2 Fix any commands where `--json` is accepted but not implemented: add `serde_json::to_string_pretty` output path for the command's result data.
- [x] 4.3 Ensure `skills list`, `skills doctor`, and `validate` pass `--json` from `cli.json` in main.rs dispatch.

## 5. Kif — Benchmark harness: offline CI gates

- [ ] 5.1 Create `benchmarks/datasets.lock` (TOML): pin BEIR commit hash (beir-cellar/beir), LongMemEval commit hash (xiaowu0162/LongMemEval), LoCoMo commit hash (snap-research/locomo). Pin Ragas version in `benchmarks/requirements.txt`.
- [ ] 5.2 Create `benchmarks/prep_datasets.sh`: download pinned datasets to `benchmarks/datasets/` (gitignored). Verify SHA-256 of downloaded archives. Skip download if cached. Add `benchmarks/datasets/` to `.gitignore`.
- [ ] 5.3 Implement `benchmarks/beir_eval.rs` (or `tests/beir_eval.rs`): load NQ+FiQA from `benchmarks/datasets/`, import into a temp brain.db, run query set, compute nDCG@10, compare against `benchmarks/baselines/beir.json`. Assert regression < 2%. Use `#[ignore]` attribute for CI opt-in via `cargo test -- --ignored`.
- [ ] 5.4 Implement corpus-reality integration tests in `tests/corpus_reality.rs`: test import completeness (all fixture files → pages), SMS retrieval (exact slug → top-1), timeline retrieval (known fact → top-5), duplicate ingest (no duplicates), conflicting ingest (contradiction detected), idempotent round-trip (export → reimport → export → diff=0), latency (100 queries, assert p95 < 250ms on release build).
- [ ] 5.5 Implement concurrency stress tests in `tests/concurrency_stress.rs`: parallel OCC (4 threads, same slug, stale version → 1 success + 3 ConflictError), duplicate ingest (2 threads, same source → 1 success), WAL compact under load (compact during read → both succeed). Use `std::thread::spawn` and `Arc<Mutex<Connection>>` for shared DB access.
- [ ] 5.6 Implement embedding migration test in `tests/embedding_migration.rs`: embed with default model, record query results, re-embed (simulate model B by re-running embed), verify results come from new embeddings, rollback active flag, verify original results. Assert zero cross-model contamination.
- [ ] 5.7 Create `benchmarks/baselines/beir.json`: record initial nDCG@10 baseline after first BEIR run. This becomes the regression anchor.

## 6. Kif — Benchmark harness: advisory benchmarks (Python)

- [ ] 6.1 Create `benchmarks/requirements.txt`: pin `ragas`, `datasets`, `openai`, `langchain` versions. Add instructions for Ollama as local LLM alternative.
- [ ] 6.2 Implement `benchmarks/longmemeval_adapter.py`: load LongMemEval dataset, convert sessions to gbrain pages via `gbrain import`, run retrieval queries via `gbrain query --json`, evaluate R@5 using LongMemEval's official `evaluate_qa.py`. Report results.
- [ ] 6.3 Implement `benchmarks/locomo_eval.py`: load LoCoMo dataset, import conversations, run retrieval queries, compute F1 on single-iteration retrieval, compare against FTS5-only baseline (`gbrain search` results). Report delta.
- [ ] 6.4 Implement `benchmarks/ragas_eval.py`: run progressive retrieval queries (`gbrain query --depth auto --json`), extract context and answers, evaluate with Ragas (context_precision, context_recall, faithfulness). Support both OpenAI and Ollama as LLM judge.

## 7. Fry — CI integration for benchmark gates

- [ ] 7.1 Add benchmark CI job to `.github/workflows/ci.yml`: run `cargo test --test corpus_reality --test concurrency_stress --test embedding_migration` on every PR. These are offline and mandatory.
- [ ] 7.2 Add BEIR regression job (separate workflow or CI job): runs on release branches and manual trigger. Downloads pinned datasets, runs `cargo test --test beir_eval -- --ignored`, fails if regression > 2%.
- [ ] 7.3 Document advisory benchmark workflow in `benchmarks/README.md`: how to run LongMemEval, LoCoMo, Ragas locally. Required API keys, Ollama setup, expected runtimes.

## 8. Cross-checks and reviewer gates

- [ ] 8.1 Professor reviews `validate.rs` integrity check SQL for correctness: interval overlap detection, referential integrity queries, embedding model resolution.
- [ ] 8.2 Nibbler reviews `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw` MCP tools for adversarial edge cases: gap injection, stats information leakage, raw data size limits.
- [x] 8.3 Leela reviews all 5 SKILL.md files for completeness, clarity, and agent-executability: can an agent follow each skill end-to-end without ambiguity?
- [ ] 8.4 Scruffy verifies benchmark harnesses produce reproducible results: re-run each offline benchmark twice and confirm identical scores.
- [ ] 8.5 `cargo test` — all existing tests pass plus new validate/call/pipe/skills/MCP tests.
- [ ] 8.6 `cargo clippy -- -D warnings` — zero warnings.
- [ ] 8.7 `cargo fmt --check` — clean.

## Ship Gate

All must pass before Phase 3 is marked complete:
1. Zero `todo!()` stubs in `src/commands/`
2. All 8 SKILL.md files are production-ready (no "Stub" markers)
3. 16 MCP tools registered and tested
4. `gbrain validate --all` runs successfully on a clean brain
5. `gbrain skills doctor` shows correct resolution order
6. Offline benchmarks (corpus-reality, concurrency, embedding migration) pass in CI
7. BEIR nDCG@10 baseline established with < 2% regression gate
8. `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` all clean
