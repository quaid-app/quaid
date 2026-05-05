## Context

This is the second of two proposals splitting Phase 5 of `docs/roadmap_v3.md`. The first proposal (`conversation-memory-foundations`) lands the plumbing — turn capture, conversation files, the `extraction_queue` table with UPSERT-collapse enqueue semantics, and the ADD-only supersede chain. With it, Quaid can record conversations and supersede page versions, but no facts are produced. This proposal layers the SLM-driven extraction worker, fact-page writing, and the `memory_correct` dialogue on top.

Source-of-truth design doc: `docs/superpowers/specs/2026-05-03-phase5-conversation-memory-design.md`. The brainstorm captured there resolves all five v3-roadmap design questions — this proposal implements the half of the resolution that was held back from proposal #1.

Constraints shaping this change:

- The product positioning is "single static binary, fully airgapped." A 2 GB SLM cannot ship in the binary; it must be downloaded once and loaded at runtime. The download must be visible and intentional, not a silent side effect.
- `candle-transformers` already exists in the dependency tree (used by BGE for embeddings). Adding Phi-3.5 means enabling features, not introducing a new inference stack.
- The `pages` table is the single page store. Extracted facts must be ordinary pages, not a parallel storage system, so all existing retrieval surfaces (`memory_search`, `memory_query`, `memory_graph`) work without modification.
- The vault is the source of truth (decision in proposal #1). The worker writes facts as markdown files and lets the Phase 4 watcher ingest — never directly to the database.
- LoCoMo and LongMemEval are at 0.1% / 0.0%; Mem0 v3 sits near 90%. The benchmark lift bet rides on this proposal.

## Goals / Non-Goals

**Goals:**

- Run Phi-3.5 Mini in-process inside `quaid serve` via candle, lazy-loaded only when extraction is enabled.
- Make the model-download lifecycle explicit: `quaid extraction enable` triggers the fetch with progress UI; daemons never silently download.
- Drain the `extraction_queue` with a single in-process worker that produces typed facts as markdown files in the vault.
- Define the SLM extraction prompt and JSON-only output contract; build a defensive parser that tolerates accidental fences without inviting LLM jailbreaks.
- Resolve each fact with a fail-closed policy: `0` matching heads → coexist, `1` matching head + trustworthy semantic embeddings → thresholded dedup/supersede/coexist, `>1` matching heads or untrustworthy embedding evidence → typed refusal. The resulting accepted write still falls back to the supersede chain delivered by proposal #1.
- Provide a bounded `memory_correct` MCP tool (max 3 SLM exchanges) so wrong facts can be repaired through dialogue.
- Provide CLI surfaces for runtime control (`quaid extraction enable | disable | status`, `quaid model pull`, `quaid extract <session> [--force]`, `quaid extract --all`).
- Hit ≥ 40% LoCoMo and ≥ 40% LongMemEval. Add a DAB §8 Conversation Memory regression gate alongside the existing §4 gate.

**Non-Goals:**

- Multi-worker concurrency. Single in-process worker is sufficient for v1.
- A web UI for browsing or correcting facts. Vault + Obsidian is the supported path.
- Synchronous fact extraction on the request path. Callers that need a flush can call `memory_close_session` (proposal #1) and observe via `quaid extraction status`.
- LLM-assisted contradiction detection (DAB §7). Tracked under the existing `contradiction-semantic-gate` spec.
- Cross-namespace fact recall.
- Bundling model weights in the binary. The binary stays single-file; weights live in the model cache.
- Ollama / external runtime support. Reconsider only if candle proves inadequate for Phi-3.5 in production.
- Auto-detection of the user's hardware to pick a model. The user picks via `extraction.model_alias`.

## Decisions

### Decision 1 — In-process candle inference, lazy-loaded.

The brainstorm chose in-process candle integration over a sidecar daemon, on-demand subprocess, or external runtime. Reasons recapped: candle is already in the dep tree; lazy-load means non-extraction users pay zero memory cost; the airgapped binary positioning forbids "first install Ollama"; sidecar IPC plumbing is multi-week effort solving a problem (process isolation for an inference call that rarely panics) that doesn't exist yet. Trade-off: a Phi-3.5 panic takes down the *worker* but a `catch_unwind` boundary keeps `quaid serve` alive — non-extraction MCP tools continue to function.

### Decision 2 — Eager model download via `quaid extraction enable`.

The brainstorm chose B+D from Q9: eager fetch as the primary path, manual `quaid model pull <alias>` as a fallback for power users / CI / environments that need pre-staged caches. Why eager: airgap is the product promise, and a silent first-extraction download breaks that promise at the worst moment (when the user is waiting for their first fact). Eager fetch makes the network call visible at the moment the user opted in; failures surface fast with actionable errors. Why never silently download from the daemon: even with `extraction.enabled = true`, if the model file is missing at runtime we runtime-disable rather than auto-fetch. The lifecycle proof seam in this batch is a local cache loader that validates a verified cache and never calls download code; the full `slm.rs` runtime still remains deferred. The user can always re-run `quaid extraction enable` to recover.

Curated aliases are stronger than raw repo ids: the shipped alias table pins source digests for every downloaded file (LFS weights via SHA-256, Git-tracked files via blob object ids), while raw repo ids remain manifest-verified only.

### Decision 3 — Single in-process worker, debounced + close-triggered + idle-timer.

One worker keeps Phi-3.5 single-instance and avoids GPU/RAM contention. The worker drains the queue defined by proposal #1; this proposal adds the runtime side. The trigger model is the brainstorm's Q4 answer (D): debounce in normal flow, immediate enqueue on `memory_close_session`, plus an idle-timer auto-close after `extraction.idle_close_ms` (default 60s) so a session that's silently abandoned still gets its tail extracted.

The worker advances the conversation file's frontmatter cursor (`last_extracted_turn`) on success **before** marking the queue job `done`. This ordering is deliberate: a crash between the cursor write and the queue-done write re-runs the same window on restart, and `fact-resolution`'s dedup path drops the duplicates. Crash safety is bought by ordering, not by transactions.

### Decision 4 — Hybrid type-specific frontmatter + prose body for facts.

The brainstorm chose B from Q6. Each fact kind has a small handful of required structured fields (`about` / `chose` / `what`) plus a prose summary written by the SLM. Structured fields make dedup/supersede a cheap indexed lookup; prose preserves the context that makes the page useful for FTS5 / vec0 retrieval. Strict triples were considered and rejected: many conversational facts don't fit a single `(subject, predicate, object)` cleanly, and forcing them into one strips the context LoCoMo questions need.

Common frontmatter (`session_id`, `source_turns`, `extracted_at`, `extracted_by`, `supersedes`, `corrected_via`) is the same across all four kinds. The `corrected_via` field is set by the correction surfaces — `explicit` for `memory_correct`, `file_edit` for the file-edit-aware supersede handler from proposal #1.

### Decision 5 — Resolution thresholds (0.92 / 0.4) are configurable from day one for unique-head semantic comparisons.

`fact_resolution.dedup_cosine_min` (default `0.92`) and `fact_resolution.supersede_cosine_min` (default `0.4`) are config keys, not constants. Reason: these thresholds depend on the embedding model and corpus characteristics. We pick reasonable initial values and allow benchmark-driven tuning without a release. The defaults match the brainstorm.

The "key match + low cosine → coexist" path (cosine < 0.4 with same key) is a deliberate carve-out only when there is exactly one matching head and the embedding evidence is trustworthy. Once same-key multi-head partitions already exist, the policy fails closed instead of picking the highest cosine head. Hash-shim pseudo-embeddings also fail closed rather than being treated as semantic proof.

### Decision 6 — Worker writes vault files; the watcher does the DB write.

The worker never inserts into the page table directly. It writes the markdown file at `<vault>/extracted/<type-plural>/<slug>.md`, and the Phase 4 watcher (already running) ingests it into pages, FTS5, and vec0. Single write path. Trade-off: a small lag between fact-file write and queryability (the watcher is debounced); we accept this — it's bounded by the same vault-sync latency every other write inherits, and the cost is amortised across the existing sync infrastructure.

The supersede chain mutation (atomic two-end update of `supersedes` / `superseded_by`) is handled by the page-write logic delivered in proposal #1; the worker just sets `supersedes: <prior_slug>` in the new file's frontmatter and lets the existing path do the rest.

### Decision 7 — Bounded correction dialogue, max 3 SLM exchanges.

The brainstorm chose B+D from the correction-feature question. `memory_correct` initiates; `memory_correct_continue` advances or abandons. The SLM correction-mode prompt enforces three structured outcomes per turn: commit / clarify / abandon. Hard cap at 3 exchanges; the third exchange must commit or the session abandons with `reason: turn_cap_reached`. Small janitor purges expired open sessions hourly.

The correction prompt is **not** a chat prompt. The SLM is told its job is to produce a corrected fact, not to converse. Most simple corrections commit in one shot; ambiguous corrections like the brainstorm's "Business Admin vs CS+Business Systems major" example get one or two clarifying exchanges before committing.

### Decision 8 — Idempotency under `--force` is by structural equivalence, not byte equality.

`quaid extract <session> --force` resets the cursor to 0 and re-runs extraction. The result should be the same supersede chain shape (same heads, same key partitioning) but the prose body of each fact may vary — the SLM is not deterministic. We test idempotency by comparing structured frontmatter keys and chain topology, not byte equality of files. This is enough for the LoCoMo/LongMemEval benchmark contract.

When `--force` re-runs, existing facts that match the new outputs are de-duplicated (cosine > 0.92) rather than re-written. So the vault doesn't grow on re-extraction.

### Decision 9 — DAB §8 Conversation Memory regression gate ships with this proposal.

DAB §4 today is the existing semantic-search gate. Adding §8 (Conversation Memory) backed by the LoCoMo adapter makes regression in extraction quality visible the same way regression in semantic search is. The §8 gate lives in the existing benchmark harness; this proposal adds the section, the LoCoMo adapter wiring, and the CI integration.

## Risks / Trade-offs

| Risk | Mitigation |
|---|---|
| Phi-3.5 hallucinates facts not in the conversation. | Prompt explicitly constrains "facts must be supported by the windowed turns; do not infer beyond what was said." `extracted_by` field provides audit trail for retroactive analysis. Phase 7 active enrichment can later run a verification pass. |
| First extraction's lazy-load adds 5–15 s latency. | Documented; the user sees `quaid extraction status` reporting "loading" during that window. Subsequent extractions are warm. |
| 2 GB resident memory while the SLM is loaded. | Documented. `extraction.enabled = false` is the default, so docs-only users pay nothing. Gemma 3 1B (~600 MB) is the documented lower-resource alternative via `extraction.model_alias`. |
| SLM panic crashes the daemon. | `catch_unwind` boundary marks the job retriable, runtime-disables extraction, leaves serve running. Recovery is a manual `quaid extraction enable`. |
| Interrupted model downloads leak temp directories. | Atomic rename still prevents partial cache promotion, and later installs scavenge stale `.alias-download-*` directories while leaving fresh in-progress dirs alone. |
| Cosine thresholds (0.92 / 0.4) are wrong for the user's corpus. | Both values are config-tunable from day one; benchmark-driven tuning doesn't require a release. Low-cosine coexist remains available for unique-head partitions, while ambiguous multi-head partitions and hash-shim/unavailable embeddings now fail closed instead of silently mutating history. |
| Worker advances cursor before queue-done write; crash in between re-runs the window. | Deliberate ordering; deduplication via `fact-resolution` ensures re-run produces no duplicates. Tested in `tests/extraction_idempotency.rs`. |
| `memory_correct` 3-turn cap forces abandon on hard corrections. | Cap chosen to keep dialogue bounded; users with hard corrections can either edit the file directly (handled by proposal #1's file-edit-aware supersede) or open a normal conversation that mentions the correction (caught by next extraction pass). |
| Disk usage grows under high-volume usage. | Quantified in the design doc: ~250–400 MB DB after a year of 100 turns/day. Phase 8 / `#134` scale work is on the roadmap for higher volumes. |
| Re-extraction with a future SLM swap could produce different fact partitioning. | Acceptable: `--force` is the explicit way to re-extract under a new model. Incremental extraction continues with whatever model is currently loaded. |

## Migration Plan

1. Bump `SCHEMA_VERSION` and `quaid_config.schema_version` to v9.
2. Add the `correction_sessions` table and `idx_correction_open` partial index in `src/schema.sql`.
3. The schema-mismatch error already gives users the re-init recipe; no new tooling needed.
4. There is no data backfill: facts written under v8 (which there are none of, because v8 doesn't have the worker) remain valid heads. v9 just adds a new table.
5. Rollback: revert the schema bump and the schema.sql change. The new MCP tools (`memory_correct`, `memory_correct_continue`) and CLI subcommands disappear with the binary revert.

## Open Questions

None. The brainstorm covered all five v3-roadmap design questions; the resulting design doc resolves them. The split between proposals #1 and #2 is documented in the proposal.md of each. Future Phase 6 / Phase 7 work (entity extraction, active enrichment) builds on this proposal but is not gated by any of its decisions.
