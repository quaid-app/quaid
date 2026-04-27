## Context

Phases 1 and 2 are complete. The binary handles storage, search, graph, assertions,
progressive retrieval, novelty checking, palace filtering, and 12 MCP tools. Phase 3
(`p3-polish-benchmarks`) shipped release readiness, coverage, and docs polish.

What remains: the agent-facing intelligence layer (skills), the evaluation infrastructure
(benchmarks), and CLI surface completion (validate, call, pipe, skills doctor, --json, MCP tools).

**Current state:**
- 8 SKILL.md files exist; 3 are production-ready (ingest, query, maintain), 5 are stubs
- 4 CLI commands are `todo!()`: validate, call, pipe, skills (partially)
- 4 MCP tools are unimplemented: memory_gap, memory_gaps, memory_stats, memory_raw
- `--json` flag exists globally but some commands may not fully honour it
- benchmarks/ has a README with Phase 1 baseline but no executable harnesses
- version.rs works (prints version)

**Constraints:**
- Zero network at runtime (except `--features online-model`)
- Single static binary
- Skills are markdown, not compiled code
- Benchmark datasets must be pinned (commit hashes in `datasets.lock`)
- API-dependent benchmarks are advisory, not CI gates

## Goals / Non-Goals

**Goals:**
- All SKILL.md files are production-ready and agent-testable
- All CLI commands are implemented (zero `todo!()` stubs)
- MCP tool surface matches spec (16 tools total)
- Offline benchmarks run in CI and block releases on regression
- `--json` produces valid JSON on every command

**Non-Goals:**
- GPU acceleration
- Room-level palace filtering
- npm distribution
- LLM-integrated contradiction detection

## Decisions

### 1. Skills are pure markdown — no binary changes required

**Decision:** Skills are SKILL.md files consumed by agents at session start. Authoring
production skills requires no Rust code changes. The `skills doctor` command inspects
resolution order (embedded → `~/.quaid/skills/` → working directory) and content hashes.

**Rationale:** "Thin harness, fat skills" — the spec principle. Agent intelligence should
not be compiled into the binary.

### 2. validate command: modular check architecture

**Decision:** `quaid validate` runs check modules independently:
- `--links`: non-overlapping intervals, temporal ordering, referential integrity
- `--assertions`: dedup, supersession chain validity, dangling references
- `--embeddings`: active model exists, all chunks reference active model, vec_rowid resolution
- `--all`: runs all checks
- `--referential`: pages referenced by links/assertions exist

Each check returns a list of violations. Exit code 0 = clean, 1 = violations found.
JSON mode outputs structured violation objects.

**Rationale:** Users and the upgrade skill need targeted checks. Running all checks on a
large memory is slow; targeted checks are fast.

### 3. call command: direct MCP tool invocation

**Decision:** `quaid call <TOOL> <JSON>` invokes the MCP tool handler directly without
starting the MCP server. It deserializes the JSON input, calls the tool function, and
prints the result. This is the "GL pattern" from the spec.

**Rationale:** Enables shell scripting with MCP tools without an MCP client. Useful for
`quaid call memory_raw '{"slug":"...","source":"meeting","data":{...}}'`.

### 4. pipe mode: JSONL on stdin/stdout

**Decision:** `quaid pipe` reads one JSON object per line from stdin. Each object is
`{"tool": "<tool_name>", "input": {...}}`. Results are written as one JSON object per line
to stdout. Errors are JSON objects with an `error` field.

**Rationale:** Shell pipelines (`cat commands.jsonl | quaid pipe`) enable batch processing
without MCP protocol overhead.

### 5. Benchmark harness: Rust for offline, Python for advisory

**Decision:**
- **Offline gates** (BEIR, corpus-reality, concurrency, embedding migration, round-trip):
  Rust integration tests in `benchmarks/` or `tests/`. Run via `cargo test`.
- **Advisory benchmarks** (LongMemEval, LoCoMo, Ragas): Python scripts with adapters.
  Run manually. Require API keys (OpenAI for LLM judge, or local Ollama).

**Rationale:** Offline gates must run in CI without external dependencies. Python is the
standard for ML evaluation frameworks (Ragas, LongMemEval official scripts).

### 6. MCP Phase 3 tools: memory_gap, memory_gaps, memory_stats, memory_raw

**Decision:**
- `memory_gap`: wraps `core::gaps::log_gap()`. Accepts query string and context. Always
  stores with `sensitivity = 'internal'`. Returns gap ID.
- `memory_gaps`: wraps `core::gaps::list_gaps()`. Accepts `resolved` bool and `limit`.
  Returns JSON array.
- `memory_stats`: wraps `commands::stats::run()` in JSON mode. Returns page count, link
  count, assertion count, contradiction count, gap count, embedding count.
- `memory_raw`: INSERT into `raw_data` table. Accepts slug, source, and arbitrary JSON data.
  Returns row ID.

**Error codes:** `-32001` for not-found, `-32003` for DB errors, consistent with Phase 2.

### 7. Dataset pinning strategy

**Decision:** All benchmark datasets are pinned via commit hash in `benchmarks/datasets.lock`
(TOML format). A prep script downloads pinned versions to `benchmarks/datasets/` (gitignored).
CI caches the datasets directory.

**Rationale:** Reproducibility. Floating HEAD references produce non-deterministic benchmarks.

## Migration Plan

No schema migration. All changes are additive:
- New SKILL.md content (overwrite stubs)
- New CLI command implementations (replace `todo!()`)
- New MCP tool registrations
- New benchmark files

## Open Questions

1. **memory_gap_approve**: The spec defines a separate approval tool for gap sensitivity
   escalation. Should this be a Phase 3 MCP tool or deferred? **Decision: defer.**
   The research skill documents the workflow; the tool can be added when needed.

2. **Concurrency stress test infrastructure**: Should parallel writer tests use threads
   or processes? **Decision: threads** — simpler, tests OCC at the rusqlite level.

3. **Ragas model**: Should we mandate Ollama for local Ragas evaluation or accept API keys?
   **Decision: accept both.** Document Ollama as the default, API key as fallback.
