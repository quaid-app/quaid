## 1. Pre-release schema reset to v9

- [x] 1.1 Update `src/schema.sql`: add `correction_sessions` table with columns and CHECK constraints per the `correction-dialogue` spec
- [x] 1.2 Add partial index `idx_correction_open ON correction_sessions(status, expires_at) WHERE status = 'open'`
- [x] 1.3 Seed config defaults in the existing `config` table: `extraction.enabled='false'`, `extraction.model_alias='phi-3.5-mini'`, `extraction.window_turns='5'`, `extraction.debounce_ms='5000'`, `extraction.idle_close_ms='60000'`, `extraction.retention_days='30'`, `fact_resolution.dedup_cosine_min='0.92'`, `fact_resolution.supersede_cosine_min='0.4'`
- [x] 1.4 Bump `SCHEMA_VERSION`, `quaid_config.schema_version`, and schema-version tests to `9`
- [x] 1.5 Verify no v8 → v9 migration or rollback path is added; schema-version mismatches continue to fail with the schema-mismatch/re-init message
- [x] 1.6 Unit tests: fresh v9 schema accepts the new artefacts; v8 and future-schema DBs are rejected at open; CHECK constraints on `correction_sessions.status` enforced

## 2. SLM runtime — candle Phi-3.5 wrapper

- [ ] 2.1 Add `phi3` feature to the `candle-transformers` dependency in `Cargo.toml`
- [ ] 2.2 Create `src/core/conversation/slm.rs` with `SlmRunner` struct holding tokenizer, model, and inference config
- [ ] 2.3 Implement `SlmRunner::load(alias: &str) -> Result<SlmRunner>` that resolves the alias to a local model directory, loads tokenizer + safetensors, and constructs the candle Phi3 model
- [ ] 2.4 Implement `SlmRunner::infer(prompt: &str, max_tokens: usize) -> Result<String>` with deterministic sampling (temperature 0 or near-zero) for reproducibility
- [ ] 2.5 Wrap `infer` in a `catch_unwind` boundary; on panic return a typed error rather than propagating
- [ ] 2.6 Lazy-load gate: the daemon holds an `Option<SlmRunner>` initialized on first use, with an interior mutex; subsequent inferences reuse the loaded instance
- [ ] 2.7 Tests: `tests/slm_runtime.rs` loads a tiny test model fixture, runs a deterministic prompt, asserts output

## 3. Model lifecycle — download + cache + verification

- [x] 3.1 Create `src/core/conversation/model_lifecycle.rs` with the local model cache layout (`~/.quaid/models/<alias>/{tokenizer.json,model.safetensors,...}`)
- [x] 3.2 Implement `download_model(alias: &str, progress: impl ProgressReporter) -> Result<PathBuf>` that resolves alias to a Hugging Face repo, downloads files into the cache, and runs per-file source-pinned integrity checks for curated aliases using the shipped mixed-digest pin table (each pinned file is verified by either SHA-256 or git-blob-SHA1, including SHA-256-pinned Gemma tokenizer artifacts); for raw repo-id downloads, verifies against server-supplied ETag SHA-256 where available
- [x] 3.3 Resolve aliases: `phi-3.5-mini` → `microsoft/Phi-3.5-mini-instruct` (with the appropriate quantised variant), `gemma-3-1b`, `gemma-3-4b`, plus any non-alias passes through as a raw repo id
- [x] 3.4 Atomic install + stale-temp scavenging: download into a temp directory, verify, then rename into the final cache location to avoid partial-cache states, and scavenge stale `.alias-download-*` leftovers on later installs
- [x] 3.5 On integrity-check failure, remove the partial download and return an actionable error
- [x] 3.6 Tests: `tests/model_lifecycle.rs` covers alias resolution, atomic install, stale-temp scavenging, partial-download recovery, local-only cache load / no-silent-fetch, and integrity verification (using a mock HTTP server / fixture model)

## 4. CLI — `quaid extraction` and `quaid model` subcommands

- [x] 4.1 Create `src/commands/extraction.rs` implementing `enable | disable | status`
- [x] 4.2 `quaid extraction enable`: validate the configured model alias, run `download_model` with progress UI, on success update `quaid_config.extraction.enabled = true`; on failure leave the flag unchanged and print the actionable error
- [x] 4.3 `quaid extraction disable`: update `quaid_config.extraction.enabled = false` (does not delete cached model files)
- [ ] 4.4 `quaid extraction status`: query daemon (or local DB if daemon is not running) for model state, queue depth, active-session list, last-extraction-at per session, and recent failed jobs; format as the human-readable summary described in the spec
- [x] 4.5 Create `src/commands/model.rs` implementing `quaid model pull <alias>` that calls `download_model` without changing `extraction.enabled`
- [x] 4.6 Wire the new subcommands into `src/main.rs` clap dispatch
- [ ] 4.7 Tests: `tests/cli_extraction.rs` covers enable success / failure paths, `model pull` does not flip the flag, status output shape

## 5. Extraction worker — window selection + SLM call

- [ ] 5.1 Create `src/core/conversation/extractor.rs::Worker` struct with handles to the queue, SLM runner, and vault writer
- [ ] 5.2 Implement the worker loop: poll dequeue at a configurable cadence (default 1s), process one job at a time, sleep when no jobs are available
- [ ] 5.3 Window selection: read the conversation file, parse cursor `C` and `last`, compute `[C+1, last]` new turns, slice into windows of `extraction.window_turns`, with up to `window_turns - new_count` lookback turns when new turns are sparse
- [ ] 5.4 For `trigger_kind = 'session_close'` jobs with no new turns, run a single window over the most recent `window_turns` turns purely as context (cursor remains unchanged)
- [ ] 5.5 Build the SLM prompt per the `fact-extraction-schema` spec: system prompt + user prompt with new-turns and lookback-context delimited
- [ ] 5.6 Invoke `SlmRunner::infer` with `max_tokens = 2048` (configurable later)
- [ ] 5.7 Tests: `tests/extraction_window.rs` covers window slicing with sufficient new turns, with sparse new turns, and the session_close empty-window case

## 6. Output parsing — strict JSON contract

- [ ] 6.1 Define `ExtractionResponse { facts: Vec<RawFact> }` and `RawFact` enum (one variant per kind) in `src/core/types.rs` with `serde(tag = "kind")`
- [ ] 6.2 Implement `parse_response(raw: &str) -> Result<ExtractionResponse>` that: strips leading/trailing whitespace, strips accidental ```json fences, then `serde_json::from_str`
- [ ] 6.3 Reject any `RawFact` whose required type-specific fields are missing; reject unknown kinds; record validation errors at the per-fact level so other facts in the same response can still proceed
- [ ] 6.4 Increment `extraction_queue.attempts` on parse failure; mark `failed` after `extraction.max_retries` (default 3) per the proposal-#1 contract
- [ ] 6.5 Tests: `tests/slm_prompt_parsing.rs` golden-file coverage of bare JSON, fenced JSON, JSON with leading commentary (rejected), unknown kind (rejected), missing required field (rejected), mixed-validity facts (partial accept)

## 7. Per-fact resolution

- [ ] 7.1 Create `src/core/conversation/supersede.rs::resolve(raw_fact, conn) -> Result<Resolution>` returning one of `Drop`, `Supersede(prior_slug)`, `Coexist`
- [ ] 7.2 Head-only key-match query: select pages where `kind = ? AND superseded_by IS NULL AND json_extract(frontmatter, '$.<type_key>') = ?`
- [ ] 7.3 Compute prose-embedding cosine between the new fact's `summary` and each candidate head's body; reuse the existing embedding pipeline
- [ ] 7.4 Apply rules: cosine > `dedup_cosine_min` → Drop; cosine in `[supersede_cosine_min, dedup_cosine_min]` against best-match → Supersede; otherwise → Coexist
- [ ] 7.5 Multi-match: pick the head with highest cosine to the new fact; resolution evaluates only against that head; other matching heads remain unchanged
- [ ] 7.6 Wrap resolution + write step in a single transaction so head-lookup, decision, and write are consistent
- [ ] 7.7 Tests: `tests/fact_resolution.rs` covers all five rule branches plus multi-match disambiguation; ensures non-head pages are excluded from candidates

## 8. Fact-page write step

- [ ] 8.1 Implement `write_fact(resolution, raw_fact, conn) -> Result<FactWriteResult>`:
  - **Drop**: no file write; log structured event with `decision = drop` and the matched head's slug
  - **Supersede**: derive new slug (`<key>-<4char-hash>` or similar); render markdown file with `supersedes: <prior_slug>`, `corrected_via: null`, source-turn references; write to `<vault>/extracted/<type-plural>/<slug>.md`
  - **Coexist**: render and write similarly with `supersedes: null`
- [ ] 8.2 Slug generator: derive from kind + type-key + 4-char SHA-256 hash; collision-avoidance via append-counter loop bounded to a few attempts
- [ ] 8.3 Namespace path nesting: when namespaces are in use, write under `<vault>/<namespace>/extracted/...`
- [ ] 8.4 No direct page-table writes from this path: the existing Phase 4 vault watcher ingests the file
- [ ] 8.5 Tests: `tests/fact_write.rs` covers each resolution branch produces the right file (or no file); slug collision; namespace path; supersede frontmatter triggers proposal-#1's atomic two-end update

## 9. Cursor advance + queue accounting

- [ ] 9.1 After all windows for a job are processed successfully, update the conversation file's `last_extracted_turn` to the highest ordinal in the just-processed new-turns range and `last_extracted_at` to current time
- [ ] 9.2 Persist the cursor write before transitioning the queue job to `done` (deliberate ordering for crash safety)
- [ ] 9.3 On any window failure, do not advance the cursor; let the queue's retry logic re-claim the job on next dequeue
- [ ] 9.4 Tests: `tests/extraction_worker.rs` covers cursor advance on success, cursor unchanged on failure, crash-recovery via lease expiry re-runs the same window without producing duplicates (verified by fact-resolution dedup)

## 10. Idle-timer auto-close

- [ ] 10.1 Maintain in-memory `HashMap<(namespace, session_id), Instant>` of last-turn arrival times
- [ ] 10.2 On `memory_add_turn`, update the map; on session_close, remove the entry
- [ ] 10.3 Background task ticks every 10s, scans for entries older than `extraction.idle_close_ms`, and for each: enqueues a `session_close` job, updates the day-file's `status = closed`, removes the entry from the map
- [ ] 10.4 Tests: `tests/idle_close.rs` simulates time passage, verifies enqueue + status update at the right moment, verifies activity resets the timer

## 11. Janitor — purge old queue rows + expire correction sessions

- [ ] 11.1 Add an hourly janitor task to `quaid serve` that runs both purges in a single tick
- [ ] 11.2 Purge: delete `extraction_queue` rows where `status IN ('done', 'failed') AND enqueued_at < (now - extraction.retention_days days)`
- [ ] 11.3 Expire: update `correction_sessions` rows where `status = 'open' AND expires_at < now()` to `status = 'expired'`
- [ ] 11.4 Tests: `tests/janitor.rs` covers both behaviours and verifies pending/running rows are never purged regardless of age

## 12. Correction dialogue — `memory_correct` MCP tool

- [ ] 12.1 Create `src/core/conversation/correction.rs` with `Correction` struct holding session id, fact slug, exchange log, turn budget
- [ ] 12.2 Implement `start_correction(fact_slug, correction_text) -> Result<CorrectionStep>`: validate the slug is a head fact-kind page; insert `correction_sessions` row with `status: open`, `expires_at: now + 1h`, `turns_used: 1`, `exchange_log: [{user: correction_text}]`; build correction-mode prompt; invoke SLM; return next step
- [ ] 12.3 Implement `continue_correction(correction_id, response_or_abandon) -> Result<CorrectionStep>`: validate session is `open` and not expired; append exchange to log; on `abandon`, transition to `abandoned` and return without SLM; on `response`, increment `turns_used`, invoke SLM with full exchange context, return next step
- [ ] 12.4 Hard cap: when `turns_used >= 3`, the next non-commit SLM output forces `status: abandoned` with `reason: turn_cap_reached`
- [ ] 12.5 Correction-mode SLM prompt template: enforces commit / clarify / abandon outcomes only; output is JSON of shape `{"outcome": "commit"|"clarify"|"abandon", ...}`
- [ ] 12.6 On commit: parse the corrected fact (same JSON contract as extraction); resolve via `supersede.rs` (forced supersede path — corrections always supersede the original); write the new fact with `corrected_via: explicit` in frontmatter
- [ ] 12.7 Register `memory_correct` and `memory_correct_continue` MCP tools in `src/mcp/server.rs`
- [ ] 12.8 Tests: `tests/memory_correct.rs` covers one-shot commit, clarify-then-commit, explicit abandon, turn-cap-abandon, expired-session continuation rejection, non-head fact rejection, non-fact-kind slug rejection

## 13. CLI — `quaid extract <session>` and `--all`

- [ ] 13.1 Create `src/commands/extract.rs` implementing `quaid extract <session-id> [--force]` and `quaid extract --all [--since <date>]`
- [ ] 13.2 Bare `extract <session>`: enqueue an immediate `manual` job for the session
- [ ] 13.3 `extract <session> --force`: reset `last_extracted_turn = 0` across all of the session's day-files, then enqueue
- [ ] 13.4 `extract --all`: iterate sessions in the active namespace; with `--since`, restrict to sessions with at least one day-file dated on or after the cutoff
- [ ] 13.5 Output: print enqueued session ids and a hint that progress is observable via `quaid extraction status`
- [ ] 13.6 Tests: `tests/cli_extract.rs` covers all four flag combinations

## 14. DAB §8 Conversation Memory benchmark gate

- [ ] 14.1 Extend the existing DAB harness to add a §8 Conversation Memory section
- [ ] 14.2 Wire the LoCoMo adapter (or its existing harness equivalent) into §8: load fixture sessions, ingest via `memory_add_turn`, close sessions, run the LoCoMo test queries against the resulting fact set, score
- [ ] 14.3 Repeat for LongMemEval as a parallel sub-section
- [ ] 14.4 Set the regression gate: §8 Conversation Memory must not regress more than 3 points version-over-version (matching §4's existing tolerance)
- [ ] 14.5 Add a CI run that exercises the full §8 path on representative hardware

## 15. Integration tests + benchmarks

- [ ] 15.1 `tests/airgap_extraction.rs`: run extraction enable + a turn add + extraction in a network-namespace-isolated environment; assert zero outbound network calls after enable completes
- [ ] 15.2 `tests/extraction_idempotency.rs`: extract a session, then `--force` re-extract; verify the resulting head set is structurally equivalent (same `(kind, type_key)` partitioning, same chain shape)
- [ ] 15.3 `benches/extraction.rs`: per-window p95 latency under representative load — assert `< 3s` on M1/M2 Mac fixtures and `< 8s` on x86_64 Linux fixtures
- [ ] 15.4 End-to-end smoke test: capture a 50-turn session via `memory_add_turn`, close it, wait for extraction, assert at least one fact page exists in `<vault>/extracted/`, assert `memory_search` over the conversation topic returns the fact
