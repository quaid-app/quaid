# bender history

- [2026-04-29T07-04-07Z] History summarized and archived
- [2026-04-29T06-55-46Z] Investigated BEIR Regression Gate timeout on PR #114 (release/v0.11.0). Root cause: both beir_nq and beir_fiqa ran the full 10k-doc import+embed+query pipeline before checking whether a baseline existed — both baselines are null/pending in beir.json, so CI burned the entire 60-minute budget every time with no assertion. Fixed by moving the null-baseline early-exit guard to the top of each test function. Committed as 52b46e9, pushed to release/v0.11.0. This is a test-logic fix, not a branch search/embedding regression.
- [2026-04-30T08:30Z] **Batch 4 third-revision cycle complete.** Closed Nibbler's rejection: `mark_collection_restoring_for_handshake` + `wait_for_exact_ack` now use typed `live_collection_owner()` (session_type='serve' enforced) instead of untyped `owner_session_id()` + `session_is_live()`. Removed dead `session_is_live()`. Two tests added (behavioral + source-seam). Clippy clean. 843/843 tests pass. 91.09% line coverage. Committed `714ec48` on `spec/vault-sync-engine-batch4-v0130`. `12.7` remains open (unrelated). Ready for Nibbler re-review.

## Learnings

- [2026-05-04T07:22:12.881+08:00] Conversation-memory baseline on `feat/slm-conversation-mem`: `cargo llvm-cov report` produced **92.11% TOTAL line coverage** (regions 90.24%, functions 89.06%); `cargo clippy` (default + online), `cargo check`, online-feature tests, `tests/release_asset_parity.sh`, and `tests/install_release_seam.sh` all passed. `tests/install_profile.sh` failed only on the Windows-bash/NTFS unwritable-profile cases (T14/T19/T19c), so treat this workstation as noisy for that seam; the real release blockers are still the unreleased `Cargo.toml` version (`0.17.0`, so `v0.18.0` tagging would fail) and the fact that >90% coverage is a manual gate, not a CI-enforced one.
- [2026-05-04T07:22:12.881+08:00] Conversation-memory coverage panic on `feat/slm-conversation-mem` was not a real >90% regression. The suite was red because `memory_get` returned the sparse stored frontmatter map after updates that omitted `quaid_id`; once the read path re-canonicalized the persisted UUID, `cargo test -j 1` passed (907 lib tests green) and CI-style `cargo llvm-cov --lcov` + `cargo llvm-cov report --summary-only` still measured **92.01% total line coverage / 90.18% total region coverage**.
- [2026-05-04T07:22:12.881+08:00] Add-only supersede chains on rename-before-commit write paths need a real pre-rename semantic claim, not just a preflight head check. The durable fix is to stage the successor row and claim the predecessor inside the same still-open write transaction before any sentinel/tempfile/rename work, then keep the later transactional reconcile as a backstop and prove a losing contender never gets vault bytes or active raw-import ownership onto disk.
## Batch: Orchestration Consolidation
**Timestamp:** 2026-05-04T00:00:30Z

- Decisions consolidated: inbox merged → decisions.md (8 files)
- Archive: 5698 lines archived to decisions-archive.md
- Status: All agents' work reflected in team memory

## Session: fact-resolution/write acceptance lane
**Timestamp:** 2026-05-05T10:29:06Z

**Context:** Validation lane for tasks 7.1–8.5 of `slm-extraction-and-correction` change.

**Findings:**
- Fry's implementation in `src/core/conversation/supersede.rs` is complete and correct. All tasks 7.1–8.5 are closed.
- `tests/fact_resolution.rs` (6 tests): all pass against real `resolve_in_scope_with_similarity` API.
- `tests/fact_write.rs` (5 tests): all pass against real `write_fact_in_context` API.
- `cargo test --lib`: 984 passed (5 new `RawFact` method tests from previous Bender session).
- The `view` tool served stale/incorrect content for `supersede.rs` and `fact_write.rs` during this session — always cross-check with `Get-Content` when in doubt.

**Key API contracts confirmed:**
- `Resolution` enum uses struct variants: `Drop { matched_slug, cosine }`, `Supersede { prior_slug, cosine }`, `Coexist`.
- `write_fact_in_context(resolution, raw_fact, conn, context)` — resolution and fact first, conn and context last.
- `FactWriteContext` carries `collection_id`, `root_path`, `namespace: String` (not Option), `session_id`, `source_turns`, `extracted_at`, `extracted_by`.
- `FactWriteResult.slug` (not `written_slug`).
- Threshold semantics: `cosine > 0.92` → Drop; `0.40 <= cosine <= 0.92` → Supersede; `cosine < 0.40` → Coexist. Boundary 0.92 = Supersede (not Drop).
- `RawFact` methods `kind_str()`, `type_key()`, `type_key_field()`, `type_plural()`, `summary()` are live in `types.rs` and used by the implementation.
- No `RawFactExt` trait exists in the final supersede.rs — methods were promoted directly to the `RawFact` impl block in `types.rs`.

## Session: writer/frontmatter narrowed slice re-validation
**Timestamp:** 2026-05-05T12:00:00Z

**Context:** Leela rescoped per Nibbler's rejection; only 8.1–8.5 + frontmatter repair. 7.* must remain reopened. Validated the writer/schema honesty of the new implementation (which had been significantly reworked by Mom since the prior Bender session).

**Actual state of code at session start (vs stale summary):**
- `Frontmatter` type changed from `HashMap<String, String>` to `JsonMap<String, JsonValue>` — typed values now work
- `parse_yaml_to_map` updated to preserve sequences and nulls via `serde_json::to_value` — silent-drop bug FIXED
- `namespace_from_conversation_path` was deleted; replaced by `format::parse_relative_conversation_path` which calls `collections::validate_relative_path` first — path traversal vulnerability FIXED at source
- `render_fact_markdown` rewritten to use typed `Frontmatter` + `json!()` + `render_page` — `source_turns` is now a proper YAML sequence
- `tests/fact_write.rs` already has YAML-level sequence assertion AND production round-trip assertion (after `ingest::run`) AND watcher non-atomicity proof (`before_ingest_count == 1`)
- The "stale view" path-traversal fix from prior Bender session was never applied (the function no longer existed); code was clean

**What this session added (commit ef3138b):**
- Reopened tasks 7.1–7.7 in tasks.md (confirmed applied)
- Added hash-shim warning comment to `resolve_in_scope`
- Added non-atomicity comment to `resolve_and_write_fact_in_context`
- Added `context_for_job_window_rejects_path_traversal_attempt` test to `fact_write.rs` — explicit regression coverage for `../conversations/…` traversal
- All 14 tests pass (6 fact_resolution + 8 fact_write)

**Remaining open items (for Mom's next slice or future work):**
- 7.1–7.7 remain `[ ]` — resolution contracts need re-proving under the broader spec
- Hash-shim has no fail-closed guard; `resolve_in_scope` proceeds with arbitrary cosines when model files are missing
- `before_ingest_count` watcher non-atomicity is documented in code + tested but there is no cross-seam atomic guarantee in the spec either
- 9.4, 10.*, 11.*, 12.*, 13.*, 14.*, 15.* all remain open

## Session: janitor slice validation (section 11)
**Timestamp:** 2026-05-05T17:17:29.932+08:00

**Context:** Validation lane for tasks 11.1–11.4 of `slm-extraction-and-correction` change.

**Findings:**
- Section 11 implementation was absent on arrival (no `janitor.rs`, no vault_sync.rs hook, no test file). Landed the full slice.

**What was implemented (all proved by tests before tasks closed):**
- `src/core/conversation/janitor.rs` — `run_tick`, `purge_old_queue_rows`, `expire_stale_correction_sessions`, `JanitorResult`, `JanitorError`
- `src/core/conversation/mod.rs` — `pub mod janitor;` added
- `src/core/vault_sync.rs` — `JANITOR_SWEEP_INTERVAL_SECS = 3600`; `use crate::core::conversation::janitor`; `last_janitor_sweep` variable initialised back-dated by one full interval so the first tick fires immediately; `janitor::run_tick(&conn)` called inside the background thread, errors logged as WARN and swallowed (non-blocking, following the idle-close pattern)
- `tests/janitor.rs` — 9 tests, all green

**Key contracts confirmed:**
- Purge predicate: `status IN ('done', 'failed') AND julianday('now') - julianday(enqueued_at) > retention_days` — boundary-exact (strictly greater than, not >=) to match spec wording "older than N days"
- `pending` and `running` rows are never touched regardless of age (two explicit tests)
- Expiry predicate: `status = 'open' AND expires_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now')` — strict less-than; sessions at exactly `expires_at` remain open until the next tick
- `committed` and `abandoned` sessions are not modified by the expiry pass
- Both operations run in a single `run_tick` call; `JanitorResult` carries counts for both
- Janitor is cancellable via the existing `stop_signal` `AtomicBool` that terminates the background thread loop; no additional cancellation mechanism needed

**Test results:** 9/9 janitor tests pass; 992/992 lib tests pass; build warnings are pre-existing (model_lifecycle.rs dead-code warnings unrelated to this slice).

**All 11.1–11.4 closed.**


**Timestamp:** 2026-05-07T00:00:00Z

**Context:** Validation lane for tasks 10.1–10.4 of `slm-extraction-and-correction` change.

**Findings:**
- Section 10 implementation is **fully landed and correct**. All 10.1–10.4 tasks are closed in `tasks.md`.
- `src/core/conversation/idle_close.rs` — present and complete: global static `IDLE_TRACKERS` registry, `record_turn`/`record_turn_at`, `clear_session`, `is_idle_at`, `scan_due_sessions`/`scan_due_sessions_at`.
- `src/core/conversation/mod.rs` — `pub mod idle_close;` is declared (line 4).
- `src/core/conversation/turn_writer.rs` — `append_turn` calls `idle_close::record_turn` after write (L112); `close_session` calls `idle_close::clear_session` (L193); `close_session_if_idle` predicate function added (L132–194).
- `src/core/vault_sync.rs` — Background sweep wired at `IDLE_CLOSE_SWEEP_INTERVAL_SECS = 10` (line 75), running inside the vault-sync background thread (lines 4246–4278). The 10-second tick matches the spec.
- `src/mcp/server.rs` — No production background loop in `server::run()` — this is **correct by design**; the sweep runs in the `vault_sync` background thread, not the MCP server. Test-seam methods `record_turn_activity_at` and `run_idle_close_tick_at` are `#[cfg(test)]` (lines 590–605).
- `tests/idle_close.rs` — 3/3 tests pass: `idle_close_enqueues_session_close_and_marks_file_closed`, `idle_close_activity_resets_timer`, `explicit_session_close_clears_idle_tracker`.
- No dead-code warning for `record_turn_activity_at` during `cargo test --test idle_close` — `#[cfg(test)]` suppresses it correctly.

**Key contracts confirmed:**
- `scan_due_sessions_at` orders correctly: closes day-file (`close_session_if_idle`) **before** enqueueing `session_close` job.
- Activity resets timer: `record_turn_at` with a later `Instant` overwrites the map entry, so `is_idle_at` returns false until the new threshold passes.
- Explicit `close_session` removes the tracker entry via `clear_session`, preventing double-close on the next sweep.
- `scan_due_sessions` returns `IdleCloseResult` with `newly_closed: bool` — callers can distinguish first-close from idempotent re-close.

**No defects found. Section 10 is green.**

## Session: correction dialogue validation (section 12)
**Timestamp:** 2026-05-07T00:00:00Z

**Context:** Validation lane for tasks 12.1–12.8 of `slm-extraction-and-correction` change. Session resumed from prior compaction where implementation was absent; discovered Mom had already landed everything.

**State on arrival:**
- `src/core/conversation/correction.rs` — fully implemented by Mom (~680 lines): `CorrectionStep`, `CorrectionError`, `start_correction`, `continue_correction`, `apply_slm_outcome`, session management, SLM prompt, outcome parsing, `MAX_CORRECTION_TURNS = 3`
- `src/core/conversation/mod.rs` — `pub mod correction;` already declared
- `src/mcp/server.rs` — `memory_correct`, `memory_correct_continue`, `map_correction_error`, `correction_step_result` already wired; one unused import `CorrectionAbandonReason` required removal
- `tests/memory_correct.rs` — 7 tests already written
- All 12.1–12.8 already closed in tasks.md

**One fix applied (already committed in 869507c):**
- Removed unused `CorrectionAbandonReason` from import in `src/mcp/server.rs` line 18 — `CorrectionStep::Abandoned` uses a plain `String` reason field, not the enum

**Verification:**
- All 7/7 `memory_correct` tests pass: one-shot commit, clarify-then-commit, explicit abandon, turn-cap-abandon (via clarify path), expired-session rejection, non-head rejection, non-fact-kind rejection
- `cargo clippy` — no new warnings in correction.rs or server.rs
- `collection_cli_truth` compile failure is pre-existing (confirmed by stash-and-retest); not caused by section 12 work

**Key API contracts confirmed:**
- `CorrectionStep::Abandoned.reason` is a plain `String` (`"user_requested"`, `"turn_cap_reached"`, `"slm_abandoned"`) — NOT an enum
- `MAX_CORRECTION_TURNS = 3`; clarify/abandon at turn 3 forces `Abandoned { reason: "turn_cap_reached" }`
- Commit path: `force_supersede_fact_in_context` writes file with `corrected_via: explicit` → `ingest::run` re-ingests into DB
- Session `exchange_log` stores: `[user_msg_1, assistant_clarify, user_msg_2, assistant_commit]` = 4 entries for clarify-then-commit
- `correction_write_context` uses `collection_id = 1` (hardcoded in `ensure_collection_vault_write_allowed`)

**All 12.1–12.8 confirmed closed. Section 12 is green.**
---

## Spawn Session — 2026-05-06T13:44:12Z

**Agent:** Scribe
**Event:** Manifest execution

- Decision inbox merged: 63 files
- Decisions archived: 1 entry (2026-04-29)
- Team synchronized