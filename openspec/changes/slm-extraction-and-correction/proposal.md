## Why

This is the second of two proposals splitting Phase 5 of `docs/roadmap_v3.md`. The first proposal (`conversation-memory-foundations`) lands the plumbing — turn capture, conversation files, the extraction job queue, and the ADD-only supersede chain — but writes no facts. This proposal layers the actual SLM-based fact extraction, fact-page writing, and correction surface on top of those foundations. Without it, Quaid records conversations but cannot answer LoCoMo-style questions like "what did we decide about X last week" because no structured facts are produced. With it, the LoCoMo and LongMemEval benchmark scores move from 0.1% / 0.0% baseline toward the ≥ 40% targets. Issues `#105`, `#135`.

## What Changes

- Integrate Phi-3.5 Mini as the default local SLM via `candle-transformers`, lazy-loaded in-process inside `quaid serve` only when extraction is enabled. Configurable to other small local models (Gemma 3 1B, Gemma 3 4B, or any Hugging Face model id) via `extraction.model_alias`.
- Add a `quaid extraction enable` / `disable` CLI subcommand pair. `enable` flips `extraction.enabled = true` and **eagerly downloads** the configured model with progress UI; if the download fails the flag stays unflipped. Curated aliases use source-pinned artifact digests, while raw repo ids remain a weaker manifest-only path. Add a `quaid model pull <alias>` subcommand for manual / power-user / CI workflows.
- Implement the **extraction worker** that drains the `extraction_queue` table (delivered by proposal #1): pull the next due job, slice the windowed turns out of the conversation file (`extraction.window_turns` new turns plus lookback context as needed), call Phi-3.5, parse the JSON output, and apply per-fact resolution.
- Define the **SLM extraction prompt** (system + user) that emits typed facts of kinds `decision`, `preference`, `fact`, `action_item` with the hybrid type-specific frontmatter + prose body schema. Constrain the SLM to JSON-only output, no markdown fences, no inference beyond the windowed turns.
- Implement **per-fact resolution** (dedup vs supersede vs coexist) keyed on type-specific structured frontmatter (`about` / `chose` / `what`) plus prose-embedding cosine. Cosine > 0.92 → drop as duplicate; cosine in `[0.4, 0.92]` and same key → supersede the existing head; cosine < 0.4 with key match → coexist (different aspects sharing the key); no key match → coexist as a fresh head.
- Write extracted facts as markdown files to `<vault>/extracted/<type>/<slug>.md` so they ride the existing vault sync engine into the page table. **No direct DB write from the extraction path** — one write path, the same one users see.
- Idle-timer logic: a session with no turns for `extraction.idle_close_ms` (default 60s) automatically transitions to a `session_close` extraction job to flush tail turns, even without an explicit `memory_close_session` call.
- Add `memory_correct` and `memory_correct_continue` MCP tools — a bounded dialogue (max 3 SLM exchanges) for repairing wrong facts. Backed by a new `correction_sessions` table. Outputs supersede the original via the chain delivered by proposal #1, with `corrected_via: explicit` set on the new head.
- Add a `quaid extract <session-id> [--force]` and `quaid extract --all [--since <date>]` CLI surface for re-extracting sessions when the SLM is upgraded or extraction is re-tuned.
- Add `quaid extraction status` CLI command that reports model state, queue depth, active sessions, last-extraction-at per session, and recent failed jobs.
- Add a `correction_sessions` table to back the bounded-dialogue tool, plus an hourly janitor that purges expired open correction sessions and `done` extraction-queue rows older than N days.
- DAB harness gains a §8 Conversation Memory section that scores multi-session recall against the LoCoMo adapter, landing alongside this proposal so we can track regression in the same way DAB §4 currently is.
- **BREAKING (pre-release)**: schema bump from v8 to v9 — adds the `correction_sessions` table. No automatic migration per the existing pre-release no-auto-migration policy.

## Capabilities

### New Capabilities

- `slm-runtime`: Lazy-loaded in-process Phi-3.5 (or alternate small local model) inference via candle, the model-download lifecycle (`quaid extraction enable`, `quaid model pull <alias>`), the `extraction.enabled` master gate, the local-only cache-load seam that refuses silent fetches, panic-boundary isolation, and the configurable model alias.
- `extraction-worker`: The queue-draining worker — window selection (new turns + lookback context), prompt construction, SLM invocation, JSON-only output parsing with per-fact validation error collection, retry/fail accounting for whole-response parse failures (delegating to `extraction-queue`), and the after-extraction frontmatter cursor advance on the source conversation file.
- `fact-extraction-schema`: The hybrid type-specific frontmatter + prose body shape for `decision`, `preference`, `fact`, and `action_item` pages, including the SLM output contract that produces these shapes and the path layout under `<vault>/extracted/<type>/<slug>.md`.
- `fact-resolution`: The per-fact dedup vs supersede vs coexist decision logic — exact lookup by `(kind, type_key)` plus prose-embedding cosine in defined ranges — and the write step that produces a vault file picked up by Phase 4 vault sync.
- `correction-dialogue`: The bounded `memory_correct` and `memory_correct_continue` MCP tools, the `correction_sessions` table, the constrained correction-mode SLM prompt (commit / clarify / abandon outcomes only), the 3-turn cap, the 1h expiry janitor, and the `corrected_via: explicit` annotation on resulting head pages.
- `extraction-control-cli`: The CLI surface — `quaid extraction enable | disable | status`, `quaid model pull <alias>`, `quaid extract <session-id> [--force]`, `quaid extract --all [--since <date>]`.

### Modified Capabilities

- `extraction-queue`: Adds the runtime worker contract — how the worker claims, drains, and accounts retries/failures — and the lease-expiry behaviour required when a worker crashes mid-job. Proposal #1 specs the table, the enqueue UPSERT, the dequeue ordering, and the lease window; this proposal adds the worker side and the handful of consequential additions: idle-timer auto-`session_close`, the `done`-row janitor, and the cursor-advance-on-success contract that ties the worker back to the conversation file's frontmatter cursor.

## Impact

- **Code**:
  - `src/core/conversation/slm.rs` (new): candle-transformers Phi-3 wrapper, lazy-load on first job, prompt builder, output parser, `catch_unwind` panic boundary.
  - `src/core/conversation/extractor.rs` (new): the worker — pull job, slice window, run SLM, parse, resolve, write.
  - `src/core/conversation/supersede.rs` (new): per-fact resolution logic (dedup / supersede / coexist) keyed on type-specific frontmatter + prose embedding cosine.
  - `src/core/conversation/correction.rs` (new): `memory_correct` + `memory_correct_continue` orchestration, continuation tracking via the `correction_sessions` table.
  - `src/core/conversation/model_lifecycle.rs` (new): `quaid extraction enable` + `quaid model pull <alias>` plumbing, on-disk model cache management, model-id verification at daemon open.
  - `src/commands/extraction.rs` (new): the `quaid extraction` CLI subcommand group.
  - `src/commands/extract.rs` (new): the `quaid extract <session-id>` re-extraction CLI.
  - `src/commands/model.rs` (new): the `quaid model pull <alias>` CLI.
  - `src/core/db.rs`: schema bump to v9; `correction_sessions` table + `idx_correction_open` partial index.
  - `src/core/types.rs`: `RawFact`, `ExtractionResponse`, `CorrectionSession`, `WindowedTurns` types.
  - `src/mcp/server.rs`: register `memory_correct`, `memory_correct_continue`.
  - `src/schema.sql`: embedded DDL updated to v9.
  - `Cargo.toml`: keep the existing `candle-transformers` dependency baseline and use its built-in Phi-3 module surface; there is no separate Phi-3 Cargo feature to toggle on 0.8.x.
- **Schema**: bump `SCHEMA_VERSION` / `quaid_config.schema_version` to v9. Add `correction_sessions` table per the design doc, plus partial index `idx_correction_open ON correction_sessions(status, expires_at) WHERE status = 'open'`. No migration path.
- **Config**: New keys in the existing mutable `config` table — `extraction.enabled` (default `false`), `extraction.model_alias` (default `phi-3.5-mini`), `extraction.window_turns` (default `5`), `extraction.debounce_ms` (default `5000`), `extraction.idle_close_ms` (default `60000`).
- **Migration**: None. Pre-release no-auto-migration policy.
- **Tests**:
  - `tests/extraction_worker.rs`: worker drains queue, advances cursor, idempotent re-run with `--force` produces stable supersede chain.
  - `tests/slm_prompt_parsing.rs`: golden-file tests for prompt construction; defensive parsing of accidental ```json fences and whitespace.
  - `tests/fact_resolution.rs`: dedup at cosine > 0.92, supersede at cosine in [0.4, 0.92] with key match, coexist on key match + cosine < 0.4, coexist on no key match, multi-match disambiguation by highest cosine.
  - `tests/memory_correct.rs`: bounded dialogue commits in ≤ 3 turns; clarification path; abandon path; expiry after 1h.
  - `tests/airgap_extraction.rs`: zero network calls after `quaid extraction enable` succeeds (executed under network-namespace isolation).
  - `tests/extraction_idempotency.rs`: `quaid extract <session> --force` from cursor=0 produces the same supersede chain as initial extraction (modulo SLM nondeterminism, which the test allows for via fact-set equivalence rather than byte-equal comparison).
  - `benches/extraction.rs`: per-window p95 < 3s on M1/M2 Mac, < 8s on x86_64 Linux, on representative input.
- **Dependencies**: No new runtime crates. `candle-transformers` is already in the dependency tree for BGE, and Quaid uses that crate's built-in Phi-3 module surface without any extra Cargo feature gate. The model weights are downloaded at `quaid extraction enable` time, not bundled in the binary.
- **Performance**: SLM inference is the dominant cost; budget per-window p95 is `< 3s` on M1/M2 Mac and `< 8s` on x86_64 Linux. Memory: ~2 GB resident while the SLM is loaded; with `extraction.enabled = false` (the default), zero extra memory cost.
- **Benchmarks**: LoCoMo ≥ 40% (from 0.1% baseline). LongMemEval ≥ 40% (from 0.0% baseline). DAB §8 Conversation Memory section added as a regression gate alongside DAB §4.
