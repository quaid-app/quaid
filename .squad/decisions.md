# Amy — conversation memory release docs

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Context:** `v0.18.0` release-doc truth pass for `conversation-memory-foundations`
- **Decision:** Public release docs must split the shipped `v0.17.0` state from the branch-prep `v0.18.0` state, and must call out the tool-count delta explicitly (`v0.17.0` = 19 MCP tools, `v0.18.0` branch = 22).
- **Why:** The branch adds `memory_add_turn`, `memory_close_session`, and `memory_close_action`, but GitHub Releases and `install.sh` still resolve to the published `v0.17.0` tag until `v0.18.0` exists. Treating those as the same state makes install docs, release copy, and tool-count claims untruthful.


# Bender — conversation memory coverage debug

- **Date:** 2026-05-04T07:22:12.881+08:00
- **Context:** Scruffy reported the conversation-memory branch as both test-red and honestly below the 90% coverage bar after commits `a348e7f`, `684931c`, and `9d1b20e`.
- **Decision:** Treat the coverage alarm as a validation artifact, not a branch-wide coverage collapse. Fix the red suite first by restoring persisted `quaid_id` canonicalization on read outputs, then measure coverage with the same CI-style `cargo llvm-cov --lcov` path plus `cargo llvm-cov report --summary-only`.
- **Why:** The failing suite test was real: `memory_get` exposed the raw stored frontmatter JSON after an update omitted `quaid_id`, so the persisted UUID survived in `pages.uuid` but disappeared from `frontmatter.quaid_id`. After the read-path fix, the branch still measured 92.01% total line coverage and 90.18% total region coverage, so the honest position is above 90%.
- **Guardrail:** For persisted identity fields stored outside agent-editable frontmatter, every read surface that emits canonical page JSON must re-inject the persisted value rather than trusting the sparse stored frontmatter map.


# Bender decision: conversation-memory supersede race fix

- Timestamp: 2026-05-04T07:22:12.881+08:00
- Scope: `conversation-memory-foundations` tasks `2.2`-`2.5`
- Decision: `src/commands/put.rs` now stages the successor row and claims the predecessor head inside the same still-open SQLite write transaction before recovery-sentinel, tempfile, and rename work begins. The existing transactional `reconcile_supersede_chain` call stays in place after rename as the race backstop.
- Why: two different successor slugs could both preflight the same head and the loser surfaced `SupersedeConflictError` only after rename, which made the rejection contract dishonest because vault bytes could already be on disk.
- Trade-off: this keeps the SQLite writer transaction open across the Unix write-through seam. That wider single-writer window is accepted for this slice because it is the requested safe direction and it preserves the invariant that a rejected non-head supersede attempt does not mutate the vault.


# Bender SLM Validation — Findings

**Author:** Bender (Tester)
**Date:** 2026-05-05T06:49Z
**Change:** `slm-extraction-and-correction` (proposal #2)
**Branch audited:** `feat/slm-conversation-mem` (current working tree state)

---

## What I Verified

### ✅ PASSED — Schema v9 foundations (proposal #1 carry-forward)

| Check | Result |
|---|---|
| `correction_sessions` table present with correct `status` CHECK constraint | ✅ |
| `correction_sessions.exchange_log` CHECK (`json_valid` + `json_type = 'array'`) | ✅ |
| `idx_correction_open` partial index on `status = 'open'` | ✅ |
| `extraction_queue` `trigger_kind` and `status` CHECK constraints | ✅ |
| All 12 extraction/fact-resolution config keys seeded | ✅ |
| `SCHEMA_VERSION = 9` in `db.rs` | ✅ |
| `config.version = '9'` seeded | ✅ |
| v8 DB rejected at open with re-init message | ✅ |
| `tests/extraction_queue.rs` — 7 tests all green | ✅ |
| `tests/supersede_chain.rs` — 2 tests green | ✅ |
| `tests/conversation_turn_capture.rs` — 15 tests green | ✅ |
| `memory_add_turn` enqueues when `extraction.enabled = true` | ✅ |
| `memory_close_session` triggers `session_close` job | ✅ |

### 🐛 BUG FIXED — `open_is_idempotent` stale assertion

`db::tests::open_is_idempotent` was asserting `PRAGMA user_version == 8` after the
second `db::open()`. Because `set_version()` runs on every `open_connection()` call and
sets `user_version = SCHEMA_VERSION`, the re-open correctly produces 9.
The assertion was left at 8 from the v8→v9 bump.

**Fix applied:** Changed `assert_eq!(version, 8)` → `assert_eq!(version, 9)`.
**Test now passes.**

---

## ❌ NOT IMPLEMENTED — Implementation lane must clear these

Everything below is spec'd in tasks.md but absent from the repository.
These represent 100% of proposal #2's deliverable surface.

### 2. SLM Runtime (tasks 2.x)
- `src/core/conversation/slm.rs` does not exist.
- No `SlmRunner`, no `catch_unwind` boundary, no lazy-load gate.
- **Risk:** Without the panic boundary, a Phi-3.5 crash propagates to the MCP serve loop.
  The design requires `catch_unwind` isolation.

### 3. Model lifecycle (tasks 3.x)
- `src/core/conversation/model_lifecycle.rs` does not exist.
- No download, no atomic install, no SHA-256 integrity check.
- **Risk:** `quaid extraction enable` is a documented user entry point and doesn't exist.
  CLI truthfulness claim in the proposal is false until this lands.

### 4. CLI extraction commands (tasks 4.x)
- `src/commands/extraction.rs` does not exist.
- `src/commands/model.rs` does not exist.
- Neither is registered in `src/commands/mod.rs` or `src/main.rs`.
- `quaid extraction enable | disable | status` and `quaid model pull` produce
  "unknown subcommand" errors today.
- **Risk:** All CLI truthfulness claims in the proposal are false.

### 5 + 6. Extraction worker + output parser (tasks 5.x, 6.x)
- `src/core/conversation/extractor.rs` does not exist.
- No window selection, no SLM call, no JSON parser.
- **Risk:** `extraction_queue` rows pile up forever with no worker to drain them.
  Any session that enqueues extraction jobs just leaks queue rows.
  The queue janitor (task 11.x) also doesn't exist.

### 7. Per-fact resolution (tasks 7.x)
- `src/core/conversation/supersede.rs` (new) does not exist.
- No dedup/supersede/coexist decision logic.
- **Risk:** Zero fact pages are ever written to the vault.
  LoCoMo / LongMemEval scores remain at 0.0% baseline.

### 8. Fact-page write step (tasks 8.x)
- No `write_fact` function exists.
- No vault file output path.
- **Risk:** Extraction worker (when it lands) has no way to persist results.

### 9. Cursor advance + queue accounting (tasks 9.x)
- No post-job cursor write, no `last_extracted_turn` advance.
- **Risk:** Without the deliberate cursor-before-done ordering, crash safety guarantee is
  absent. Re-runs would have no dedup path either (supersede.rs missing).

### 10. Idle-timer auto-close (tasks 10.x)
- No `idle_close_ms` timer in the MCP serve loop.
- **Risk:** Abandoned sessions never get their tail turns extracted unless the user
  explicitly calls `memory_close_session`.

### 11. Janitor (tasks 11.x)
- No hourly janitor for done/failed queue rows or expired correction sessions.
- **Risk:** Both `extraction_queue` and `correction_sessions` grow unboundedly under
  production use.

### 12. Correction dialogue (tasks 12.x)
- `src/core/conversation/correction.rs` does not exist.
- `memory_correct` and `memory_correct_continue` are not registered in `src/mcp/server.rs`.
- **Risk:** Bounded correction dialogue is entirely absent. Clients that call
  `memory_correct` receive an "unknown tool" MCP error.

### 13. `quaid extract` CLI (tasks 13.x)
- `src/commands/extract.rs` does not exist.
- **Risk:** Manual re-extraction and `--force` reset are unavailable.

### 14. DAB §8 benchmark gate (tasks 14.x)
- No LoCoMo adapter, no LongMemEval sub-section, no §8 in the DAB harness.
- **Risk:** No regression gate for extraction quality; LoCoMo/LongMemEval remain
  at 0.1% / 0.0% and are not tracked.

### 15. Integration tests (tasks 15.x)
- `tests/airgap_extraction.rs` — missing.
- `tests/extraction_idempotency.rs` — missing.
- `benches/extraction.rs` — missing.
- End-to-end smoke test (15.4) — missing.

---

## Airgap / Runtime Claims

The proposal states: "single static binary, fully airgapped." This claim is *conditionally
true* today:
- **Correct for the existing binary:** BGE-small-en-v1.5 is embedded at build time; no
  network calls are needed for semantic search.
- **False for extraction:** `quaid extraction enable` (unimplemented) would trigger a
  model download. Until task 3.x lands with a working download gate and `enable` CLI,
  users have no way to obtain or cache the Phi-3.5 model — meaning extraction is both
  gate-blocked and network-dependent at first use.
- **The airgap claim for the extraction path cannot be validated without task 3.x.**

---

## Summary Assessment

Proposal #1 (conversation-memory-foundations): **fully landed, all tests green post-fix.**
Proposal #2 (slm-extraction-and-correction): **0 of 14 task groups implemented.**

The schema is v9 and the queue foundations are correct. Everything that rides on top of
them — the SLM runtime, the worker, the fact writer, the correction dialogue, all CLI
commands, and all benchmark gates — has not been written. The implementation lane must
complete tasks 2–15 before this change can be marked honest.

---

## Tasks Updated in tasks.md

- No proposal tasks marked complete. The only verified-complete items are the v8/v9
  schema tests which were part of proposal #1 carry-forward (already in tasks.md tasks
  1.x, which were already ticked as done by the implementation lane).
- The stale `open_is_idempotent` test fix is a test-discipline repair, not a task-unit
  close.


# bender: Unix coverage fix — self-write dedup race

**Date:** 2026-05-05T06:49:17+08:00  
**Branch:** feat/slm-conversation-mem  
**Commit:** 697273f  

## Root cause

`classify_watch_event_only_suppresses_rename_when_source_is_not_markdown_or_is_self_write`
is a `#[cfg(unix)]` test that shares the global `PROCESS_REGISTRIES.self_write_dedup` map
with 15+ other tests that call `init_process_registries()`. Under `cargo llvm-cov` (Coverage
CI job), coverage instrumentation slows each test enough that the window between the single
up-front `remember_self_write_path_at` call and the second `classify_watch_event` call
(Case 2) is large enough for a concurrent test's `init_process_registries()` to clear the map.

CI evidence: `src/core/vault_sync.rs:8034:9 — assertion failed: classify_watch_event(...).is_empty()`.
The regular Test job passes (no instrumentation, narrower window).

## Decision

Fix in test logic only. No production code change needed.

Re-call `remember_self_write_path_at` immediately before the Case 2 `classify_watch_event`
invocation. This shrinks the race window to a single function-call boundary (~microseconds).

**Why Case 1 is immune:** `should_suppress_self_write_rename` returns `false` whether or not
the target registry entry is present, because the markdown source (`notes/from.md`) has no
matching entry — the function falls through to `maybe_suppress_self_write_event(source_path)`
which returns `false` for an unregistered path. The 3-event assertion holds either way.

**Why Case 2 is not:** suppression requires the target entry to be present. Without it,
the function returns `false` and a `DirtyPath` event is emitted.

## Alternative considered

Adding `serial_test` crate + `#[serial]` attribute across all registry-touching tests.
Rejected: adds a new dependency and touches ~15 test functions for a localised issue.

## Outcome

- Commit 697273f pushed to `feat/slm-conversation-mem`
- `cargo check` clean, `cargo fmt --check` clean
- CI Coverage job unblocked pending next run
- No production code changed; narrowest honest test-logic fix


# Fry decision — conversation memory close action

- Timestamp: 2026-05-04T07:22:12.881+08:00
- Change: conversation-memory-foundations
- Scope: tasks 9.1-9.5

## Decision

Keep `memory_close_action` on the narrow MCP contract `{slug, status, note?}` and prove optimistic-concurrency conflicts with an internal pre-write test seam instead of widening the public tool schema.

## Why

- The OpenSpec slice only commits to slug-based action closure.
- Collection-aware slug resolution already gives the handler the routing it needs.
- The pre-write seam gives a deterministic conflict proof without adding user-visible knobs.


# Fry — conversation memory queue foundations

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Decision:** For `memory.location = dedicated-collection`, auto-create a sibling collection named `<write-target>-memory` rooted at `<write-target-root>-quaid-memory` on first use.
- **Why:** This keeps conversation/extracted paths isolated from the main vault without inventing another config key in this slice, and avoids nesting the dedicated collection under the live vault root.
- **Implication:** Future MCP/CLI surfaces should treat that derived collection contract as the current truthful default unless a later OpenSpec explicitly introduces user-configurable naming or root overrides.


---
recorded_at: 2026-05-04T07:22:12.881+08:00
author: Fry
change: conversation-memory-foundations
topic: supersede-retrieval-surface
---

# Decision

`memory_get` should return structured JSON for the supersede-chain slice instead of rendered markdown so the caller can reliably read `superseded_by` and `supersedes` pointers without reparsing frontmatter text.

# Why

- The OpenSpec requirement for task 3.5 is about machine-readable chain traversal metadata, not presentation.
- MCP callers need a stable successor pointer surface; embedding it only in rendered markdown would force brittle text parsing.
- The CLI `get` surface remains markdown-oriented, so this narrows the structured change to MCP where it is needed.

# Consequence

- MCP consumers now get canonical slugs plus explicit `superseded_by` / `supersedes` fields.
- Future chain-aware tooling can build on `memory_get` without another response-shape change.


---
recorded_at: 2026-05-04T07:22:12.881+08:00
author: Fry
change: conversation-memory-foundations
topic: session-tool-contract
---

# Decision

Wave 2 session tooling should persist `closed_at` in conversation frontmatter and store namespace-qualified queue session keys internally whenever the public `session_id` is only namespace-local.

# Why

- `memory_close_session` must return the original close timestamp on idempotent re-close, which is not recoverable truthfully from file mtime or queue state alone.
- The current `extraction_queue` schema has only `session_id`, so raw namespace-local ids would collapse unrelated `alpha/main` and `beta/main` sessions onto one pending row.
- Keeping the qualification internal preserves the public MCP contract (`session_id` stays namespace-local) while protecting queue semantics and future worker routing.

# Consequence

- Conversation files remain the source of truth for session lifecycle because `closed_at` lives with the session frontmatter.
- Queue producers and future workers must treat `extraction_queue.session_id` as an internal routing key, not blindly as the public caller-facing session id.


# Fry — SLM first batch boundary

- Date: 2026-05-05
- Change: `slm-extraction-and-correction`

## Decision

Land the first truthful batch as the v9 schema/config reset only: `correction_sessions`, extraction/fact-resolution config defaults, schema-version bump, and the rejection/acceptance tests that prove fresh v9 bootstrap and fail-closed v8 reopen behavior.

## Why

- Every later SLM/control/worker slice depends on the persisted schema and defaults being stable first.
- The branch is already dirty in nearby conversation/runtime files, so keeping Batch 1 to schema + tests avoids widening into active seams before the base contract is locked.
- This keeps the branch moving toward v0.19.0 with a reviewable, low-blast-radius slice that future runtime/CLI work can build on.

## Follow-up

- Next batch should start at runtime/model lifecycle wiring (`2.*` / `3.*`) or the thinnest CLI plumbing that consumes the new defaults without broadening into worker/correction orchestration prematurely.


# Fry — SLM model lifecycle batch decision

- Date: 2026-05-05
- Change: `slm-extraction-and-correction`

## Decision

Land the model-cache plumbing around a manifest-verified install path:

1. Resolve friendly aliases (`phi-3.5-mini`, `gemma-3-1b`, `gemma-3-4b`) to pinned Hugging Face repos/revisions.
2. Download required model artifacts into a temporary cache directory first.
3. Verify SHA-256 from source headers when Hugging Face exposes one (notably safetensor blobs), and persist a local `manifest.json` with computed hashes for every downloaded file.
4. Promote the cache with a final rename only after the manifest verifies cleanly, and delete failed temp installs.

## Why

This keeps the landed slice truthful without pretending every upstream metadata file comes with a server-side SHA-256. Large weight blobs still get source-backed hash verification, while the local manifest gives Quaid a deterministic cache-integrity check for later opens and re-pulls. The temp-dir + rename install path also closes the partial-cache seam needed by `quaid extraction enable` and `quaid model pull`.


---
recorded_at: 2026-05-04T07:22:12.881+08:00
author: Fry
change: release-v0.18.0
topic: manifest-and-doc-truth
---

# Decision

The `v0.18.0` release-bound commit should move the Cargo manifest surface to `0.18.0` and, in the same pass, repair every release-facing link or status line that still points at moved docs or an older upcoming tag.

# Why

- `release.yml` hard-fails when `Cargo.toml` does not match the pushed tag, so the branch is not truthfully releasable until the manifest and lockfile both carry `0.18.0`.
- Public install and upgrade guidance still participates in the release lane: a tag can succeed while release notes, README/download instructions, or upgrade docs still point at missing files like the old root `MIGRATION.md`.
- Keeping the version bump and the doc/link repair in one coherent release-lane commit prevents a half-prepared state where tagging would pass CI but ship broken release references.

# Consequence

- Future release prep should audit workflow release-note links, README/install docs, and web upgrade docs alongside the version bump.
- The branch can now truthfully stay in “preparing `v0.18.0` / latest public tag still older” mode until the actual tag and GitHub Release are cut.


# Leela — conversation-memory conflict resolution

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **PR:** #153 (`feat/slm-conversation-mem`)
- **Scope:** Resolve six OpenSpec add/add conflicts against `main`

## Decision

Keep the conflict resolutions on the truth-repaired branch versions of the six `conversation-memory-foundations` artifacts.

## Why

`main` carries earlier draft copies of the same change that still describe a v7→v8 schema bump, `pages.kind`, unchecked tasks, and broader future-slice claims. The branch copies were already updated to the shipped reality: schema v8 was the landed baseline before the remaining slices, all 70 tasks are complete, and the narrower conversation-routing / fixed lease-expiry truths are explicitly documented.

## Applied rule

1. Resolve the six add/add conflicts to the artifact text that matches the shipped implementation, not the first version that reached `main`.
2. Preserve completed checkbox history and truth notes that explain the landed baseline and narrowed seams.
3. Treat the merge as documentation-truth repair only; no unrelated code or `.squad/` churn enters the commit.


## Leela — conversation-memory-foundations next waves

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Batch the remaining `conversation-memory-foundations` scope (`4.1`–`12.5`) into three execution waves so the file contract freezes before public tool wiring, and the watcher/edit-history seam stays isolated until the capture path is green.

### Wave 1 — conversation file contract + root resolution

- **Tasks:** `4.1`–`4.6`, `11.1`–`11.2`
- **Goal:** Freeze `Turn` / `ConversationFile`, canonical render shape, malformed-parse contract, multi-day ordinal continuation, and the shared vault-root resolver before any writer, queue, or MCP surface depends on them.
- **Dependencies:** Approved `2.2`–`2.5` and `3.1`–`3.7` only.
- **Parallelism:** `4.1`–`4.3` can proceed alongside `11.1`–`11.2`; `4.4`–`4.6` wait for both seams to settle.
- **Reviewer / pre-gate notes:**
  - **Professor** pre-gates any deviation from the current spec's frontmatter keys, heading shape, or path contract.
  - Do **not** widen into watcher/file-edit work in this wave.

### Wave 2 — capture request path + queue-backed session tools

- **Tasks:** `5.1`–`5.5`, `6.1`–`6.6`, `7.1`–`7.6`, `8.1`–`8.6`, `11.3`–`11.4`, `12.1`–`12.3`
- **Goal:** Land one coherent synchronous capture surface: append + fsync durability, queue collapse/lease semantics, `memory_add_turn`, `memory_close_session`, dedicated-collection first-use routing, and the first end-to-end conversation tests.
- **Dependencies:** Wave 1 complete.
- **Parallelism:** After `append_turn` and queue APIs settle, MCP wiring (`7.x`, `8.x`) can split from integration-test assembly (`12.1`–`12.3`), but `12.1`–`12.3` do not close until both tools and config-path behavior are green.
- **Reviewer / pre-gate notes:**
  - **Scruffy** pre-gates the latency and concurrency harness before anyone claims `5.5`, `6.6`, or `7.5`.
  - **Professor** gates MCP contract shape, error mapping, and queue precedence/lease truth.
  - Keep proposal-2 runtime out of scope: no worker, no idle-close daemon, no SLM calls.

### Wave 3 — extracted-fact mutation + history-preservation closure

- **Tasks:** `9.1`–`9.5`, `10.1`–`10.7`, `12.4`–`12.5`
- **Goal:** Finish the remaining mutator surface: `memory_close_action` as the lone in-place lifecycle update, plus file-edit-aware supersede/history preservation for extracted facts, then close with final supersede/file-edit integration proofs.
- **Dependencies:** Wave 1 complete; Wave 2 green for shared path/config seams; `10.x` also depends on the approved supersede/retrieval behavior from `2.2`–`3.7`.
- **Parallelism:** `9.x` and `10.x` may run in separate lanes only after reviewer gates are cleared; `12.4`–`12.5` stay last as the change-closure proofs.
- **Reviewer / pre-gate notes:**
  - **Nibbler** is the mandatory pre-gate reviewer before any `10.x` implementation starts because this widens the watcher path, extracted-root routing, and optional `_history` writes.
  - **Professor** gates the `9.x` optimistic-concurrency contract and performs final mutation-surface rereview.
  - **Scruffy** owns watcher-regression and file-edit supersede coverage on `12.5`.
  - If `10.x` is rejected, follow reviewer lockout rules and reassign the revision to a different author.


### 2026-05-04T07:22:12.881+08:00: conversation-memory supersede race routing
**By:** Leela
**What:** Do not narrow the `conversation-memory-foundations` supersede/retrieval wording first. The next revision should close the same-target contender race by moving the effective supersede gate/claim ahead of any write-through file install for all contenders, while keeping the transaction-time reconcile as the final backstop. Route the revision to Bender; Fry and Mom remain locked out, Professor stays the gate reviewer, and Scruffy should be pulled in for the concurrency proof.
**Why:** The current spec/tasks already promise a single direct successor and rejection of the second supersede write; narrowing now would redefine a real user-visible integrity failure rather than fix it. Bender is the best remaining fit because the blocker is a destructive race on the real write-through seam, and his lane is breaking shaky assumptions, validating rollback-free integrity, and defending regressions honestly.


## Leela — conversation-memory foundations Wave 1 truth repair

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Truth-repair the Wave 1 OpenSpec artifacts to describe the shipped queue lease recovery as a fixed 300-second window and the shipped `memory.location` routing/tests as conversation-root-only.

## Why

- `src/core/conversation/queue.rs` hardcodes `DEFAULT_LEASE_EXPIRY_SECONDS = 300`; there is no lease-expiry config key or runtime config read.
- `src/core/conversation/turn_writer.rs` and `tests/conversation_turn_capture.rs` only resolve and prove conversation-file placement under `memory.location`.
- Leaving broader wording in checked tasks/spec text keeps the Wave 1 closure dishonest even though the underlying code is correct for the narrower shipped slice.

## Scope preserved

- No product code changes are part of this repair.
- Future extracted-root routing remains with the later extracted-fact/file-edit work; this repair only narrows wording to the shipped Wave 1 surface.


# Leela — slm-extraction-and-correction execution slices

**Date:** 2026-05-05T06:49:17.593+08:00  
**Requested by:** macro88  
**Change:** slm-extraction-and-correction

## Decision

Do **not** route this change on the current dirty `feat/slm-conversation-mem` checkout. Reset routing to a refreshed branch from current `origin/main` / `v0.18.0`, then execute in reviewable waves:

1. **Schema v9 baseline** — close `1.1–1.6` together only: `correction_sessions`, partial index, config defaults, `SCHEMA_VERSION = 9`, and schema-version tests in one atomic batch.
2. **Model cache / download plumbing** — close `3.1–3.6` together: alias resolution, atomic install, integrity cleanup, cache layout tests.
3. **SLM runtime + strict parse contract** — close `2.1–2.7` with `6.1–6.5`: loader, deterministic inference, panic isolation, typed JSON parser, mixed-validity handling.
4. **Fact resolution + vault write seam** — close `7.1–8.5` together only after adversarial review of extracted-file writes and supersede routing.
5. **Worker orchestration / replay surfaces** — close `5.1–5.7` with `9.1–9.4`, then `10.1–11.4`, then CLI replay/status items (`4.4`, `13.1–13.6`) once the write path is stable.

`12.1–12.8` should wait until wave 4 is proven, because correction commits are forced supersedes over the same write path. `14.1–15.4` are release-blocking endgame work, not early-slice closure material.

Open a **new draft PR** for this change after waves 1–3 are green and reviewed. Do not reuse merged PR #153. Hold `v0.19.0` until merged `main` re-validates serial tests plus `cargo llvm-cov` above 90% and the benchmark/integration lane is green.

## Why

- `feat/slm-conversation-mem` is **ahead 2 / behind 18** versus `origin/feat/slm-conversation-mem`, while `origin/main` already contains merged PR **#153** and tag **`v0.18.0`**. Continuing here risks replaying foundation-era commits and release-lane confusion.
- The current dirty tree overlaps the new change in `src/core/db.rs`, `src/core/conversation/turn_writer.rs`, and `tests/conversation_turn_capture.rs`. Even though the visible diffs are formatting-only, they sit in the exact schema/session files the first SLM slices must edit, so they are merge noise and false-conflict fuel.
- Extracted-fact writing and correction dialogue both depend on the existing watcher + add-only supersede chain. Those are stateful mutation surfaces, so Nibbler should gate them before closure claims.

## Reviewer gates

- **Professor first:** wave 1 schema/reset review before more runtime work lands.
- **Professor second:** wave 2/3 API and panic-boundary review before draft PR opens.
- **Nibbler pre-gate:** required before wave 4 (`7.*`, `8.*`) and again before correction dialogue (`12.*`).
- **Scruffy:** after each landed wave, rerun serial tests first, then explicit `cargo llvm-cov` confirmation; coverage >90% is still a human gate.
- **Zapp:** draft PR upkeep once waves 1–3 are merged into a coherent branch; `v0.19.0` only after final mainline validation.


---
recorded_at: 2026-05-04T07:22:12.881+08:00
author: Leela
change: release-v0.18.0
topic: remote-head-reintegration
---

# Decision

Integrate the `v0.18.0` release-prep side-lane commits onto `feat/slm-conversation-mem` from a clean sibling worktree rooted at `origin/feat/slm-conversation-mem`, then update PR #153 so it states that conversation-memory foundations are complete and only review, CI, and release-lane completion remain.

# Why

- The parked `D:\repos\quaid` checkout is dirty and stale, so it is not a trustworthy place to merge or push release-bound work.
- Fry's manifest/release-lane prep and Amy's doc-truth pass were stacked off an older branch point; cherry-picking onto the current remote PR head preserves later fmt/clippy fixes already on `feat/slm-conversation-mem`.
- With all 70/70 OpenSpec tasks closed, the PR body must stop implying any product seam is still in flight; the only honest remaining work is reviewer sign-off, CI, and the eventual release cut.

# Consequence

- `feat/slm-conversation-mem` remains the single truthful integration branch for `v0.18.0`, but no tag or GitHub Release should be created until review and CI clear.
- Future release-lane reintegration should treat the remote PR head, not a parked local checkout, as the source of truth whenever side-lane commits need to be folded back in.


---
timestamp: 2026-05-04T07:22:12.881+08:00
author: Mom
change: conversation-memory-foundations
topic: file-edit supersede closure
---

- Preserve the manual-edit chain by inserting one archived predecessor row and rewiring any prior predecessor to point at that archive before updating the live head.
- Treat whitespace-only extracted edits as semantic no-ops: no page mutation, no raw-import rotation, no file-state refresh.
- Exclude `extracted/_history/**/*.md` from watcher dirty-path classification and reconciler ingestion so opt-in sidecars cannot become live pages or self-archive recursively.


## Mom — conversation-memory-foundations slice 2 revision

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Keep supersede-chain validation in two places on the put path: preflight it before any Unix vault rename machinery starts, and keep the existing transactional reconcile as the final race backstop.

## Why

- Preflight is what makes the non-head supersede refusal honest on the real write-through seam; otherwise the vault can mutate before the typed conflict returns.
- The transactional reconcile still has to guard the DB edge because another writer can change chain state after preflight and before commit.

## Evidence

- `src/commands/put.rs` now validates `supersedes` before sentinel/tempfile/rename work.
- The new Unix test proves rejected non-head supersedes leave vault bytes, active raw-import bytes, and recovery state unchanged while still surfacing `SupersedeConflictError`.


## Mom — conversation-memory foundations Wave 1 revision

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Use explicit ownership and explicit sentinels for the Wave 1 seams: queue completion/failure must be bound to the current dequeue attempt, same-session turn appends must hold a per-session cross-process file lock, and rendered turn metadata must use an explicit `json turn-metadata` fence instead of being inferred from any trailing JSON block.

## Why

- Lease expiry reuses the same queue row, so `job_id` alone cannot prove the caller still owns the live claim.
- A process-local mutex is not enough for file-backed turn ordinals; the serialization proof has to hold when two OS processes race the same session.
- Trailing JSON content is valid user content. If metadata is inferred from shape alone, the canonical parser strips real content.

## Evidence

- `src/core/conversation/queue.rs` now rejects `mark_done` / `mark_failed` when the caller's attempt no longer matches the live `running` row.
- `src/core/conversation/turn_writer.rs` now pairs the existing in-process mutex with a per-session cross-process file lock, and `tests/conversation_turn_capture.rs` proves the second process blocks until the first releases it.
- `src/core/conversation/format.rs` now renders metadata with ` ```json turn-metadata`, and tests prove a bare trailing JSON fence remains content.


# Mom — future schema mismatch must fail closed

- **Date:** 2026-05-05
- **Scope:** `src/core/db.rs` schema-version gate

## Decision

Treat **any** schema-version mismatch as a hard stop at open time, not just older databases.

## Why

Allowing `schema_version > SCHEMA_VERSION` lets an older binary attach to a newer database shape and do normal open work against an unsupported schema. That is a fail-open seam, not a compatibility feature.

## Required proof

- Preflight/open rejects `schema_version != SCHEMA_VERSION`
- Regression seeds a future version (currently `10`) and proves open/init refuse before creating current-version tables or rewriting stored version metadata


---
timestamp: 2026-05-04T07:22:12.881+08:00
author: Mom
change: conversation-memory-foundations
topic: whitespace-noop rename tracking
---

- Treat rename-only extracted whitespace no-ops as tracked-path moves, not deletions.
- Preserve the existing page/raw-import state, but move the `file_state` row onto the new relative path so future reconciles still see the file as tracked.
- Prove the seam with an `apply_reingest` test that renames an extracted preference without changing bytes, then asserts the new path is still classified as `unchanged`.


# Nibbler — conversation-memory file-edit pregate

- **Date:** 2026-05-04T07:22:12.881+08:00
- **Requested by:** macro88
- **Change:** conversation-memory-foundations

## Decision

Tasks `10.1`-`10.7` and `12.4`-`12.5` are still a red gate. The already-landed supersede and turn-capture coverage passes, but the risky file-edit/history slice is not honestly closed until it proves the watcher can preserve truth without forking the chain, fabricating history on whitespace saves, or re-ingesting its own disk sidecars.

## Blocking seams

### 1. Archive-before-overwrite must happen inside the same atomic edit path

`src/core/reconciler.rs:2498-2629` currently re-ingests modified files by updating the existing page row in place, then rotating `raw_imports` and `file_state`. If the file-edit handler runs after that overwrite, the prior truth is already gone. The safe bar is: snapshot the current head, create the archived predecessor, update the live head, and persist the associated raw/file state as one fail-closed unit.

### 2. Manual edits to an already-chained head must keep the chain linear

The dangerous case is not a singleton fact; it is `A -> B` followed by a manual edit to `B`. The safe result is `A -> B--archived-<ts> -> B`, not `A -> B` plus `B--archived-<ts> -> B`. `src/core/graph.rs:346-365` will happily expose multiple predecessors for one head, so `12.5` must assert the edit rewires the predecessor instead of silently creating a fork.

### 3. Whitespace-only saves must be total no-ops

Not just "no archive row." A trailing newline, editor reflow, or other formatting-only save must leave the page count, live page version, `raw_imports`, `file_state`, and graph shape unchanged. Anything weaker will turn ordinary Obsidian save traffic into fake historical corrections.

### 4. The handler must stay extracted-only, type-gated, and honest about routing scope

The new path is only safe for extracted fact types (`decision`, `preference`, `fact`, `action_item`). Conversation files and ordinary notes must keep the normal vault-sync path. Also, the change cannot pretend extracted-root routing already follows `memory.location`; the task truth notes say conversation routing shipped first and extracted-root routing is still part of this future slice.

### 5. `history_on_disk` must not create watcher recursion or sidecar ingestion

Writing archives to `extracted/_history/*.md` under the watched tree is a trap. If those sidecars are not explicitly bypassed, self-write-suppressed, or otherwise excluded from normal reconcile, the watcher can ingest them as live pages or even archive the archive. The opt-in only counts as safe if the sidecar exists for humans without becoming a second source of truth.

## Test gate before landing

- `12.4` must still prove the public `A -> B -> C` chain story at graph + head-only retrieval surfaces.
- `12.5` must simulate a manual edit of an already-chained extracted page and assert:
  - exactly one predecessor hop remains after the edit,
  - default retrieval returns only the live head,
  - `include_superseded` exposes the archived hop,
  - whitespace-only edits are full no-ops,
  - `history_on_disk=true` writes the sidecar without the watcher ingesting or re-archiving it.

## Current evidence

`cargo test --quiet --test supersede_chain --test conversation_turn_capture` passes on the current branch. That is useful baseline evidence for the landed waves, but it does not close the remaining file-edit/history seam.


# Nibbler — concurrent supersede race analysis

- **Date:** 2026-05-04T07:22:12.881+08:00
- **Requested by:** macro88
- **Change:** conversation-memory-foundations

## Decision

The remaining supersede race is still open. Two different successor slugs can both pass `validate_supersede_target(...)` against the same head, because that check is only a read-time preflight and the existing mutex is keyed to the destination slug path, not the semantic supersede target. On Unix write-through, the loser can therefore create its sentinel, tempfile, and renamed vault file before `persist_page_record(...)` finally loses the `UPDATE ... WHERE superseded_by IS NULL` compare-and-swap.

## Actual failure mode

`src/commands/put.rs` currently runs the non-head refusal preflight before `persist_with_vault_write(...)`, but the authoritative supersede CAS still lives inside `persist_page_record(...)`, after the rename/fsync path. When two contenders race on the same head:

1. contender B and contender C both read A as a head during preflight;
2. their per-slug locks do not interact because `facts/b.md` and `facts/c.md` are different paths;
3. both direct-write paths can install bytes on disk for their own new slug;
4. the winner commits its page row and flips `A.superseded_by`;
5. the loser then hits `reconcile_supersede_chain(...)` after rename, so the caller gets a post-rename recovery failure instead of a clean typed supersede conflict, and the vault has already been mutated by a write that should have been rejected.

That is not an honest "reject non-head supersede without mutation" contract.

## Required invariant

Before any vault bytes are installed for a superseding write, the contender must hold an exclusive, still-reversible claim on the predecessor head. If that claim cannot be acquired, the call must fail with the typed supersede conflict before sentinel creation, tempfile creation, rename, raw-import rotation, or any other vault-visible mutation.

## Tightest safe fix strategy

Do not rely on destination-path locking plus preflight. Move the authoritative head claim to the pre-rename phase and keep it inside the same open SQLite write transaction that will later finalize the write:

- start the write transaction before the rename/install step;
- perform the OCC row work and the authoritative supersede compare-and-swap for the predecessor while that transaction is open;
- keep the transaction uncommitted during rename/fsync;
- only after the filesystem install succeeds should file-state/raw-import bookkeeping and transaction commit complete;
- on any pre-commit failure, roll the transaction back so the head claim disappears with it.

Because the repo already operates under a single-writer model, holding the SQLite write transaction across the rename is the narrowest credible serialization surface. It blocks the second contender before vault mutation instead of letting it discover the conflict after its bytes are already on disk.

## Review outcome

- **Status:** REJECT until this race is closed or the task wording is narrowed
- **Why:** the current implementation still allows a rejected non-head supersede attempt to mutate the vault under concurrent contenders


# Nibbler — conversation-memory Wave 1 seam analysis

- **Date:** 2026-05-04T07:22:12.881+08:00
- **Requested by:** macro88
- **Change:** conversation-memory-foundations

## Decision

Wave 1 is not honestly "fully implemented." The landed core tests pass, but the three Professor seams are real, and one of them is a format-level ambiguity that should narrow scope instead of getting hand-waved as a small parser fix.

## Seam 1 — stale leased job completion by bare `job_id`

`dequeue()` can recycle an expired `running` row back to `running` again on the same row id, but `mark_done()` still finalizes by `id` alone and `mark_failed()` only gates on `id + status='running'`. That lets a stale worker from an older lease complete or fail the newer claim.

### Tightest safe invariant

Every dequeue claim must mint a fresh lease identity, and only that exact live claim may transition the row out of `running`. A bare row id is never enough once lease expiry can reissue the same row.

### Honest consequence

This seam is a direct blocker for `6.5`. The safe repair is a per-claim token or generation carried through dequeue and required by completion/failure transitions.

## Seam 2 — same-session append serialization is only in-process

`append_turn()` serializes by a process-local `OnceLock<Mutex<...>>`, then computes ordinals from the filesystem and appends with ordinary file writes. A second process can still race file creation, ordinal assignment, or same-file append ordering.

### Tightest safe invariant

For a given `{memory root, namespace, session_id}`, ordinal assignment and durable append must be linearized across all writers that can touch that vault, not just threads inside one process.

### Honest consequence

If the team wants to keep task `5.5` as written, this needs a real cross-process exclusion mechanism held across snapshot + create/append + fsync. If that is out of scope for Wave 1, the task and closure note must narrow to single-process serialization only.

## Seam 3 — parser metadata misclassifies trailing JSON fences

`split_content_and_metadata()` treats any terminal ```json ... ``` block as metadata. That means ordinary turn content that naturally ends with a JSON example cannot round-trip; the parser silently steals content into `metadata`.

### Tightest safe invariant

Metadata must be unambiguously distinguishable from user content. A trailing JSON fence may count as metadata only when the file format gives it an explicit, non-content marker.

### Honest consequence

This is the seam that should force scope narrowing instead of a heuristic patch. The current canonical format is ambiguous on its face, so "parse it smarter" is not a credible closure. Either change the format to add an explicit metadata sentinel, or narrow Wave 1 so opaque metadata round-trip is not claimed for arbitrary content that ends in fenced JSON.

## Review outcome

- **Status:** REJECT any "fully implemented" claim for Wave 1
- **Why:** two concurrency invariants are still underpowered, and the metadata fence contract is ambiguous enough to require either redesign or narrowed scope


# Nibbler — model lifecycle / airgap risk memo

- **Timestamp:** 2026-05-05T06:49:17.593+08:00
- **Requested by:** macro88
- **Scope:** upcoming `3.1–3.6` model lifecycle + extraction enablement surface
- **Verdict:** **REJECT current closure bar until the risks below are explicitly closed**

## Blocking findings

1. **Schema/version mismatch still fails open for newer databases.**
   - `src\core\db.rs:127-133` and `src\core\db.rs:206-211` only reject `schema_version < SCHEMA_VERSION`.
   - A future DB with `schema_version > 9` is allowed through open, which means an older binary can attach to a newer schema instead of failing closed.
   - Acceptance bar: reject on any schema-version mismatch (`!=`), and prove it with a regression that seeds `schema_version = 10` and asserts the binary exits before doing normal open work.

2. **The current “integrity check” is not anchored trust for SLM downloads.**
   - `src\core\conversation\model_lifecycle.rs:391-438` accepts the expected SHA-256 from response headers (`ETag`, `x-sha256`, etc.) and compares the file against that value.
   - That only proves the bytes match what the server said in that response. It does **not** prove the curated alias cache contains the intended model if the remote host, mirror, or overridden base URL is malicious.
   - Acceptance bar: for shipped aliases (`phi-3.5-mini`, `gemma-3-*`), pin expected file hashes in source and verify against those pins. If raw repo IDs stay supported, the guarantee must be explicitly downgraded for them and never described as the same integrity level.

3. **“Enable once, then airgapped forever” is still unproved and easy to accidentally violate in the runtime loader.**
   - `src\commands\extraction.rs:28-36` makes download explicit at enable time, but nothing in the landed surface yet proves the future SLM load path will refuse network and use only a verified local cache.
   - The upcoming runtime can still accidentally call `download_model()` (or equivalent fetch logic) when `extraction.enabled = true` and the cache is missing/corrupt, silently breaking the explicit-download promise.
   - Acceptance bar: runtime load must be local-only; missing/invalid cache must runtime-disable extraction with an actionable reason; add a test that enables once, blocks network, deletes or corrupts cache, then proves the daemon does **not** fetch and instead reports disabled/failure state.

4. **Partial-download cleanup is only proved on returned-error paths, not on crash/interruption paths.**
   - `src\core\conversation\model_lifecycle.rs:288-316` installs via temp dir + rename, which avoids promoting a partial cache, but cleanup only happens when the function returns an error normally.
   - A kill/crash during download leaves `.alias-download-*` directories behind. That is not a correctness break for cache selection, but it is an unclosed disk-growth seam and weakens the “partial download cleanup” claim.
   - Acceptance bar: either narrow the claim to “no partial cache promotion” only, or add stale-temp-dir cleanup on next install/startup and prove it with an interrupted-download regression.

5. **The current Windows proof lane for lifecycle/integrity is not honest.**
   - `tests\model_lifecycle.rs:175` uses a non-blocking mock socket and panics on `WouldBlock`; on this Windows lane, `cargo test --no-default-features --features bundled,online-model --test model_lifecycle` currently fails in the stale-cache and bad-integrity cases.
   - That means the claimed guarantees around cache recovery and integrity failure cleanup are not presently proved on Windows.
   - Acceptance bar: replace or harden the mock server so the targeted lifecycle tests pass reliably on Windows before those guarantees are declared closed.

## Required reviewer bar for Fry

Fry does **not** get approval for `3.1–3.6` unless all of the following are true:

1. Explicit enable/download is the **only** networked path.
2. Runtime load is local-only and fail-closed.
3. Curated aliases use pinned expected hashes, not header-echo trust.
4. Schema mismatch rejects both older and newer DB versions.
5. Partial-download behavior is either honestly narrowed or crash-cleaned.
6. Windows-targeted lifecycle tests pass and cover: cache integrity failure, stale cache recovery, no silent runtime fetch, and future-schema rejection.


# Professor decision — conversation memory close action review

- Timestamp: 2026-05-04T07:22:12.881+08:00
- Change: conversation-memory-foundations
- Scope: tasks 9.1-9.5
- Commit: ecd5513

## Decision

Approve Fry's `memory_close_action` slice.

## Why

- The MCP surface matches the spec-sized contract exactly: `{slug, status, note?}` with `{updated_at, version}` in the response.
- The implementation is action-item-only, updates `status` in place through the existing expected-version write path, and appends the optional note without widening the public interface.
- Failure handling is honest for this slice: invalid statuses are rejected at the boundary, non-`action_item` targets return `KindError`, and the conflict proof shows the stale closer loses cleanly with `ConflictError` while the competing writer's state remains stored.

## Verification

- Read proposal, design, tasks, and the conversation-turn-capture spec for the `memory_close_action` contract.
- Inspected `src/mcp/server.rs` in commit `ecd5513`, including helper validation/mapping and the focused tests for update, `KindError`, invalid status, and OCC conflict.
- Re-ran `cargo test -q memory_close_action -- --nocapture` and `cargo test -j 1`; both passed on this lane, consistent with Scruffy's reported verification posture for this slice.


---
timestamp: 2026-05-04T07:22:12.881+08:00
agent: Professor
topic: conversation-memory file-edit/history review
status: approved
---

# Decision

Approve the conversation-memory file-edit/history slice as landed across `b84e8b1` and `8eb8ec7` for tasks `10.1`-`10.7` and `12.4`-`12.5`.

# Basis

- Manual extracted-file edits keep one linear supersede chain by inserting one archived predecessor and rewiring any older predecessor onto that archive before the live row is rewritten.
- Whitespace-only saves are treated as true no-ops in both the manual-edit handler and the reconciler/full-hash diff paths, so there is no archive/version/raw-import/file-state churn.
- Handling stays extracted-only and type-gated to `decision`, `preference`, `fact`, and `action_item`.
- `_history` sidecars are excluded from both reconcile ignore handling and watch-event classification, preventing reingest loops.
- Targeted coverage is honest: `tests/file_edit_supersede.rs`, `tests/supersede_chain.rs`, reconciler whitespace tests, and watcher-sidecar tests all prove the shipped seam directly.

# Validation

- `cargo test --quiet --test file_edit_supersede --test supersede_chain`
- `cargo test --quiet extracted_path_detection_recognizes_namespace_and_history_sidecars`


## Professor — conversation-memory-foundations slice 1 re-review

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations  
**Commits:** a1ceae8, 6f51f2b

## Decision

APPROVE the re-reviewed first-slice artifact. Leela's truth-repair closes the only prior blocking issue: the OpenSpec artifacts now describe the already-landed v8 baseline honestly, using `pages.type`, the guarded session index wording, and the correct "remaining work starts at 2.2" boundary.

## Why

The earlier rejection was explicitly limited to contract truth. That mismatch is now repaired across the proposal, design, tasks, and the affected specs, and the checked tasks remain marked as already-landed baseline work rather than pretending a fresh schema bump is still pending. The shipped code in `src/schema.sql` and `src/core/db.rs` matches the rewritten artifacts.

## Evidence

- The artifacts no longer describe `pages(kind, ...)` or an unguarded session-id index.
- The current schema/code still show schema version 8, `pages.superseded_by`, the head-only index on `pages.type`, the guarded session index, `extraction_queue`, and the related baseline tests/config defaults.
- `cargo test --quiet -j 1` passed during re-review.


## Professor — conversation-memory-foundations slice 2 Bender review

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations  
**Commit:** d98e010

## Decision

APPROVE Bender's race-fix revision for slice `2.2`-`2.5` / `3.1`-`3.7`.

## Why

- The prior blocker is honestly closed. `src/commands/put.rs` now stages the successor row and claims the predecessor head inside the same still-open SQLite write transaction before sentinel/tempfile/rename work starts, so a losing contender can no longer install vault bytes before the supersede conflict is known.
- Keeping `reconcile_supersede_chain(...)` again at commit time is acceptable here because it reuses that same still-open transaction window. It is now an idempotent backstop, not a second late semantic gate that can observe a post-rename race opened by another writer.
- The broader retrieval/export surface for `3.1`-`3.7` remains coherent and already had honest coverage: head-only search/query/progressive defaults, `--include-superseded` opt-in, `memory_get` successor metadata, `memory_graph` supersede edges, and migrate/export round-trip behavior all line up with the current spec/tasks wording.

## Evidence

- `src/commands/put.rs` now opens the write transaction before the Unix write-through seam, calls `stage_page_record(...)`, and only then proceeds to recovery sentinel, tempfile, rename, fsync, and final commit via `commit_staged_page_record(...)`.
- The new Unix test hook blocks after the supersede claim and before write-through work, which is the right seam for proving the loser never creates vault bytes, raw-import ownership, or recovery residue while still surfacing `SupersedeConflictError`.
- Existing slice coverage still backs the rest of the slice: `tests/supersede_chain.rs`, `src/core/migrate.rs` round-trip coverage, and the retrieval plumbing in `src/core/search.rs`, `src/core/progressive.rs`, `src/mcp/server.rs`, `src/commands/search.rs`, `src/commands/query.rs`, `src/commands/get.rs`, and `src/core/graph.rs`.

## Validation

- Passed `cargo check --quiet`.
- Passed `cargo test --quiet supersede_chain -- --nocapture`.
- Passed targeted portable supersede tests covering chain linkage, non-head rejection, and migrate/export round-trip.
- Host note: this Windows review lane cannot execute the new Unix-only contender test directly, so approval rests on the code-path review plus the deterministic proof now landed at the correct seam.


## Professor — conversation-memory-foundations slice 2 re-review

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Reject Mom's rerevision for slice `2.2`-`2.5` / `3.1`-`3.7`.

## Why

- Mom fixed the static stale-target seam: `put` now preflights `supersedes` before Unix sentinel/tempfile/rename work, and the new Unix test honestly proves that an already-non-head target does not mutate vault bytes or active raw-import bytes before returning `SupersedeConflictError`.
- But the slice still overclaims the broader rejection guarantee. The authoritative supersede check still happens later in `reconcile_supersede_chain()` after rename, and `with_write_slug_lock()` serializes only the destination slug path, not the supersede target. Two concurrent writers can therefore both preflight against the same head, one can win the chain update, and the loser can still hit `SupersedeConflictError` only after its file bytes were installed.
- That means Professor's original integrity objection is narrowed but not closed: the repair covers deterministic non-head attempts, not the race where a target becomes non-head between preflight and commit.

## Evidence

- `src/commands/put.rs` preflights with `supersede::validate_supersede_target(...)` before `persist_with_vault_write(...)`, but the final chain mutation and conflict detection still happen in `persist_page_record()` via `supersede::reconcile_supersede_chain(...)` after the Unix rename path.
- `src/core/vault_sync.rs` `with_write_slug_lock()` keys the mutex by `collection_id:relative_path`, so competing successors to the same prior page do not share a lock unless they write the same destination slug.
- Validation run on this branch passed for `cargo check --quiet`, `cargo test --quiet supersede_chain`, and `cargo test --quiet superseding_non_head_page_is_rejected_without_partial_write`, but there is no proof covering the concurrent same-target race.


## Professor — conversation-memory-foundations slice 2 review

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations  
**Commit:** a348e7f

## Decision

REJECT Fry's supersede/retrieval slice for tasks 2.2-2.5 and 3.1-3.7.

## Highest-priority issue

`memory_put` / `put_from_string` still performs supersede-chain validation too late on the Unix vault-write path. `persist_with_vault_write()` renames the target markdown file into place first, and only then `persist_page_record()` calls `supersede::reconcile_supersede_chain(...)`. If that reconciliation rejects a non-head supersede, the DB transaction rolls back but the vault file is already mutated and the surfaced error becomes `PostRenameRecoveryPendingError`, not the intended typed supersede conflict.

## Why this blocks approval

- Task 2.3 says non-head supersede writes are rejected with a typed caller-visible error.
- Task 2.5 claims honest atomicity coverage, but the current proof only checks DB rows on the local lane; it does not prove the write was blocked before the source-of-truth file changed.
- On shipped Unix write-through behavior, the observable outcome is partial mutation plus recovery mode, which is materially different from a clean supersede rejection.

## Required repair direction

- Move supersede-target/head validation ahead of the rename/write-through step, or
- add a compensating rollback that restores the prior file before returning, and
- add tests that prove the rejected non-head supersede leaves the vault file and raw-import/source state unchanged on the real write-through path.


## Professor — conversation-memory foundations Wave 1 re-review

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Approve the Wave 1 artifact at commit `bbcb084`.

## Why

Leela's truth-repair closes the only remaining blocker from the prior rejection: the checked artifacts now describe the shipped Wave 1 contract exactly.

- `openspec/changes/conversation-memory-foundations/tasks.md`, `proposal.md`, `design.md`, and `specs/extraction-queue/spec.md` now consistently state that lease expiry is a fixed 300-second recovery window in this wave, matching `src/core/conversation/queue.rs` and its tests.
- The same artifacts now consistently limit `memory.location` routing/tests to conversation-file placement, matching `src/core/conversation/turn_writer.rs` and `tests/conversation_turn_capture.rs`.
- The previously repaired implementation seams remain closed: conversation metadata parsing uses the explicit `json turn-metadata` sentinel, same-session appends serialize across processes, and queue completion/failure is bound to the current dequeue attempt so stale workers cannot finalize a re-leased row.

I revalidated the landed slice with `cargo check --quiet`, `cargo test --test conversation_turn_capture --quiet`, `cargo test --test extraction_queue --quiet`, and `cargo test --test supersede_chain --quiet`.

## Consequence

Wave 1 is **APPROVED** for the requested scope (`4.1`-`6.6`, `11.1`-`11.4`). Leela's artifact repair is sufficient; no further revision is required for this checkpoint.


## Professor — conversation-memory foundations Wave 1 review

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations  
**Commits:** 041976f, 82bc2b9

## Decision

REJECT the Wave 1 artifact for scope `4.1`-`6.6` and `11.1`-`11.4`.

## Highest-priority issue

**Extraction-queue lease ownership is not safe after expiry.** `dequeue()` can recycle an expired `running` row back into service, but `mark_done()` still finalizes by `job_id` alone. A stale worker that wakes up after lease recovery can therefore mark a newer claim `done`, violating the lease/retry contract for task `6.5` and the queue spec's crash-recovery semantics.

## Other blocking issues

- `append_turn()` only serializes same-session writers with an in-process mutex. That does not satisfy the task `5.5` requirement for same-session serialization across processes, and the current coverage never exercises concurrent same-session writers.
- The canonical parser treats any trailing ````json` fence in turn content as metadata. A valid turn whose content naturally ends with a JSON code block cannot round-trip canonically; the parser strips that content into `metadata`.

## Evidence

- `src/core/conversation/queue.rs` reuses expired rows in `recover_expired_leases()` / `dequeue()`, but `mark_done()` updates `WHERE id = ?1` with no claim token or generation check.
- `src/core/conversation/turn_writer.rs` uses a process-local `OnceLock<Mutex<...>>` and plain `File::create` / `OpenOptions::append(true)` for same-session writes, so a second process can race ordinal assignment or file creation.
- `src/core/conversation/format.rs` infers metadata by scanning backward for a trailing ````json` fence, which is ambiguous with ordinary JSON content.
- Validation passed: `cargo test --quiet --test conversation_turn_capture --test extraction_queue` and `cargo test --quiet -j 1`.

## Lockout

Fry and Scruffy may not author the next revision of this rejected Wave 1 artifact. The next revision must be independently produced by a different agent.


### 2026-05-04T07:22:12.881+08:00: Professor review — conversation-memory Wave 2

**By:** Professor  
**What:** Approved Wave 2 as landed across `b7a0b2d` and `e2fcb65` for tasks `7.1`-`8.6` and `12.1`-`12.3`.  
**Why:** The shipped surface now matches the scoped contract: `memory_add_turn` and `memory_close_session` are wired on the MCP path, queue scheduling/error mapping is present, close is idempotent, namespace-local session ids are isolated at both file and queue seams, multi-day ordinals continue correctly, and the end-to-end tests cover file creation, queue collapse, ingestion, close behavior, midnight rollover, and namespace separation. Targeted validation also passed, including the ignored release latency gate for `tests/turn_latency.rs`.

**Decision:** APPROVE. No revision lockout applies because this artifact is not rejected.


# Professor — Schema v9 first-slice review

- Date: 2026-05-05
- Change: `slm-extraction-and-correction`
- Commit: `9f5a6f9`
- Outcome: APPROVED

## Decision

Approve the first slice. The schema bump is fail-closed for v8 databases, `correction_sessions` lands with the promised defaults and partial index, and the queue/turn-capture seams now carry the namespace and lease-generation invariants this branch needs before worker logic lands.

## Why

- `src/core/db.rs` rejects v8 before running v9 bootstrap DDL, so the pre-release no-migration policy remains honest and low-risk.
- `src/schema.sql` matches the slice contract: `correction_sessions`, `idx_correction_open`, extraction/fact-resolution defaults, and `config.version = 9`.
- `src/core/conversation/queue.rs` uses attempts as the stale-lease generation guard; `mark_done` / `mark_failed` fail closed on stale claims, which is the right foundation before any worker starts finalizing jobs.
- Conversation capture keeps `last_extracted_turn`/`last_extracted_at` in the on-disk format and namespaces queue keys as `namespace::session_id`, closing the known collision seam for same session ids across namespaces.
- `tasks.md` is not over-checked: only `1.*` is marked done, which matches the landed scope. The queue hardening in this commit is extra foundation work, not a false task closure for later runtime/worker/correction items.

## Validation reviewed

- `cargo test --quiet fresh_v9_schema --lib`
- `cargo test --quiet open_with_model_rejects_v8_database_before_creating_v9_tables --lib`
- `cargo test --quiet init_rejects_v8_database_before_creating_v9_tables --lib`
- `cargo test --quiet --test extraction_queue`
- `cargo test --quiet --test conversation_turn_capture`

## What can proceed next

Proceed to the next narrow slice: runtime/model lifecycle and control-surface plumbing (`2.*` / `3.*`, optionally the thinnest `4.*` wiring). Do not widen into worker-side fact writing or correction orchestration until that model-loading contract is landed and reviewed.


# Scruffy — conversation-memory close-action test decision

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Scope:** `conversation-memory-foundations` tasks `9.1`-`9.5`
- **Decision:** Prove `memory_close_action` with focused MCP-handler tests in `src\mcp\server.rs`, and keep the conflict path deterministic by routing the public tool through a private helper that exposes a pre-write seam only to tests.
- **Why:** The slice is mostly MCP orchestration over existing OCC machinery, so handler-local tests cover the real branches without inventing a broad integration harness. The pre-write seam lets the test land a competing write first, then assert the stale close returns `ConflictError` and does not leak the requested lifecycle status or note into the stored action item.
- **Coverage note:** This lane covers success (`status` update + note append), `KindError`, invalid status rejection, and deterministic OCC conflict while preserving repo-wide line coverage above the 90% floor on Windows.


2026-05-04T07:22:12.881+08:00

- Decision: Treat `extracted/_history/**` as a reserved sidecar path in file-edit coverage proofs and verify repeated manual edits preserve one linear supersede chain (`old predecessor -> new archive -> head`).
- Why: The risky regression is silent history forking or accidental sidecar re-ingest, not happy-path head creation.
- Test impact: Keep one focused integration file that covers chain relinking, whitespace no-op, type/path gating, and sidecar behavior under the Windows coverage lane.


# Scruffy — conversation-memory queue/core coverage decision

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Scope:** `conversation-memory-foundations` tasks 4.1-6.6 and 11.1-11.4
- **Decision:** Cover the slice at the core seams, not through premature MCP wiring: keep round-trip and parse failures in `src\core\conversation\format.rs`, append/path/layout proofs in `tests\conversation_turn_capture.rs`, and queue collapse/order/retry/lease proofs in `tests\extraction_queue.rs`.
- **Why:** This slice is plumbing. If the tests wait for `memory_add_turn` / `memory_close_session` tool wiring, coverage will lag the landed behavior and the branch will look under-tested for the wrong reason. The dedicated-collection path is therefore proved through the core root resolver and append path, with the current implementation creating a companion `*-memory` collection/root on first use.
- **Coverage note:** On this Windows lane, `cargo test -j 1` passes and `cargo llvm-cov --lib --tests --summary-only --no-clean -j 1` reports 90.02% total line coverage, so the slice clears the requested floor without pretending branch coverage was rerun.


# Scruffy — conversation-memory race-fix coverage decision

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Scope:** `conversation-memory-foundations` tasks 2.2-2.5 after commit `d98e010`
- **Decision:** Treat the branch as still above the requested honest coverage floor based on the practical Windows lane (`cargo test -j 1` and `RUST_TEST_THREADS=1 cargo llvm-cov --lib --tests --summary-only -j 1`), but do not claim a full local branch-coverage rerun or a locally executed deterministic race regression from this environment.
- **Why:** The measured repo-wide line coverage is 90.18%, and `src\commands\put.rs` remains well-covered at 94.26% line coverage after the race fix. But `cargo llvm-cov --branch` on the available stable toolchain fails because branch coverage is nightly-only, and the new contender test in `src\commands\put.rs` is `#[cfg(unix)]`, so this Windows lane cannot honestly say it re-executed that exact race proof.
- **Test note:** No extra test was added in this lane because the missing proof is environmental, not a missing branch in the committed test suite.


# Scruffy — conversation-memory slice 2 test decision

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Scope:** `conversation-memory-foundations` tasks 2.2-2.5 / 3.1-3.7
- **Decision:** Treat the existing text-query supersede integration as necessary but insufficient. Keep dedicated proofs for exact-slug head filtering, progressive expansion refusing superseded neighbours by default, and graph traversal surfacing `superseded_by` edges distinctly.
- **Why:** Those branches are where this slice can look covered while still lying: exact-slug query paths bypass the generic recall path, progressive retrieval can accidentally reintroduce historical pages during expansion, and graph traversal needs its own proof that supersede edges are first-class.
- **Coverage note:** Current honest coverage is far below 90% for the branch, so this slice should be reported truthfully rather than treated as "covered enough" by the existing broad suite.


# Scruffy — conversation-memory Wave 1 rerecheck

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Scope:** `conversation-memory-foundations` tasks 4.1-6.6 after commit `5c88104`
- **Decision:** Keep the Wave 1 truth note closed without adding more tests in this lane. The full Windows rerecheck still clears the requested floor (`cargo test -j 1` passes; `RUST_TEST_THREADS=1 cargo llvm-cov --lib --tests --summary-only -j 1` reports 90.01% total line coverage), and the three revision seams already have direct proof in the landed suite.
- **Why:** The new queue lease-token path is covered by stale-claim tests in `tests\\extraction_queue.rs`, the same-session cross-process serialization path is covered by the child-process lock test in `tests\\conversation_turn_capture.rs`, and the explicit metadata-fence path is covered by both round-trip and bare-trailing-JSON preservation tests in `src\\core\\conversation\\format.rs`. The remaining misses in those files are mostly config/error helpers and platform branches, so adding filler tests here would not make the task truth any more honest.
- **Coverage note:** This is a truthful Windows/stable rerecheck only. It does not claim nightly branch coverage, and it does not pretend to execute Unix-only lock behavior from this host.


---
agent: scruffy
date: 2026-05-04T07:22:12.881+08:00
change: conversation-memory-foundations
---

# Decision: namespace-isolated queue proofs use composite internal session keys

For Wave 2 conversation-memory coverage, I treated queue isolation as an internal storage concern rather than widening the public MCP contract. The proof lane assumes extraction rows are keyed internally as `<namespace>::<session_id>` while file paths remain `<namespace>/conversations/<date>/<session-id>.md`.

Why:
- the queue schema in this wave still stores a single `session_id` text field
- namespace isolation must prove "same session id, different namespace" does not collapse to one pending row
- keeping the composite key internal avoids inventing a new public session identifier format

Test impact:
- end-to-end namespace isolation checks should assert two pending rows, not one collapsed row
- close-session and add-turn queue assertions should use the effective internal key only when they are inspecting raw queue rows directly


# Zapp conversation-memory draft PR

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Change:** `conversation-memory-foundations`
- **Scope:** Draft PR truth boundary for `feat/slm-conversation-mem`

## Decision

Open a draft PR against `main`, but scope the claim to the pushed schema/supersede foundations slice plus the OpenSpec truth repair only.

## Why

The branch compare currently carries broader roadmap and planning ancestry than the implementation that is actually landed on `feat/slm-conversation-mem`. Narrowing the PR body to the pushed slice keeps reviewer, docs, and launch messaging aligned with reality while the larger conversation-memory change is still in progress.

## Guardrails

- State explicitly that the larger `conversation-memory-foundations` change is still in progress.
- Do not claim `memory_add_turn`, queue worker or extraction runtime behavior, or release readiness.
- Keep the PR in draft until the wider implementation actually lands and is pushed.


---
recorded_at: 2026-05-04T07:22:12.881+08:00
author: Zapp
change: conversation-memory-foundations
topic: pr-153-after-bender-race-fix
---

# Decision

Refresh draft PR #153 so it claims the supersede/retrieval slice as approved after Bender's race-fix follow-up, while still saying the broader `conversation-memory-foundations` change remains in progress from task `4.1` onward.

# Why

- The pushed branch now includes the original supersede/retrieval landing plus the follow-up fixes that closed the rejected supersede preflight hole, deepened retrieval proofs, restored canonical page UUID reads, and sealed the concurrent successor-claim race.
- The OpenSpec artifacts already truthfully mark tasks `2.*` and `3.*` complete and show remaining implementation starting at `4.1`, so the PR body should mirror that boundary instead of sounding like the whole change is approved.
- GitHub still reports the PR as conflicted, and merge simulation against current `main` reproduces add/add conflicts in the five `conversation-memory-foundations` OpenSpec files, so the body should restate that status rather than implying the lane is merge-ready.

# Consequence

- PR #153 stays draft and does not claim `memory_add_turn`, session-close tools, conversation files, extraction workers, file-edit correction flow, or release readiness.
- The truthful next merge action remains a narrow refresh from `main` plus resolution of these OpenSpec conflicts: `design.md`, `proposal.md`, `specs/add-only-supersede-chain/spec.md`, `specs/conversation-turn-capture/spec.md`, and `tasks.md`.


## Zapp — conversation-memory draft PR final-wave refresh

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Refresh draft PR #153 so it truthfully says Wave 2 is now approved, then split the remaining product wave in the body: `memory_close_action` is the active in-flight seam, while the file-edit/history-preservation slice stays pre-gated and explicitly unclaimed. Keep the PR draft-only and carry the freshly reproduced OpenSpec conflict count.

## Why

Professor already approved Wave 2 across `b7a0b2d` and `e2fcb65`, so leaving the body at the older "Wave 2 in flight" boundary would now understate shipped progress. But Leela's wave plan still keeps task `10.x` behind Nibbler's pre-gate, so the honest refresh cannot present the whole final wave as landing together; it has to separate the active `memory_close_action` seam from the still-blocked file-edit/history slice while reporting the current six-file spec conflict list against `main`.


---
recorded_at: 2026-05-04T07:22:12.881+08:00
author: Zapp
change: conversation-memory-foundations
topic: pr-153-last-product-slice
---

# Decision

Refresh draft PR #153 so it says `memory_close_action` is approved and the only remaining product scope is the file-edit/history-preservation slice, which is the active landing seam under Nibbler's pre-gated constraints rather than a shipped claim.

# Why

- Professor approved the `memory_close_action` slice at commit `ecd5513`, and Scruffy's focused coverage confirms the narrow MCP/OCC contract, so keeping that seam in "in flight" copy would now be stale.
- The remaining open tasks are the file-edit/history seam (`10.x`, `12.4`, `12.5`), and Nibbler already defined the non-negotiable landing constraints: archive-before-overwrite in one fail-closed path, linear-chain preservation on edited heads, whitespace-only total no-ops, extracted/type gating, and no `_history` watcher recursion.
- A fresh merge simulation against current `main` still reproduces six OpenSpec add/add conflicts, so the draft should stay draft and report that exact count without implying the final slice is merge-ready.


---
recorded_at: 2026-05-04T07:22:12.881+08:00
author: Zapp
change: conversation-memory-foundations
topic: pr-153-refresh-and-merge-state
---

# Decision

Draft PR #153 should claim only the live v8 baseline, commit `a348e7f`'s supersede/retrieval slice, and the matching OpenSpec truth repair; its current `mergeable_state: dirty` is a real conflict with `main`, not stale metadata.

# Why

- PR #153's pushed head is `a348e7f`, and that commit lands the supersede/retrieval slice across write paths, retrieval filters, MCP, CLI, migrate/export, and `tests/supersede_chain.rs`.
- GitHub reports the PR as `CONFLICTING`, and merge simulation against `main` reproduces add/add conflicts in the `conversation-memory-foundations` OpenSpec files already present on both branches.
- The smallest truthful next move is to refresh the branch from `main` and resolve those OpenSpec files without widening the draft's product claims.

# Consequence

- The draft PR body now matches the pushed branch truthfully.
- The coordinator should not mark the PR ready for review yet.
- Minimal next action: merge or rebase `main` into `feat/slm-conversation-mem` and resolve these five OpenSpec conflicts: `design.md`, `proposal.md`, `specs/add-only-supersede-chain/spec.md`, `specs/conversation-turn-capture/spec.md`, and `tasks.md`.


## Zapp — conversation-memory draft PR Wave 2 refresh

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Refresh draft PR #153 so it says Wave 1 is approved and complete, names Wave 2 as the current in-flight scope (`memory_add_turn`, `memory_close_session`, and the first end-to-end conversation integration tests), stays draft, and reports the freshly reproduced OpenSpec conflict list against `main`.

## Why

Professor already approved the Wave 1 artifact, so leaving the draft body framed around the older checkpoint would understate the branch's real progress. A fresh merge simulation now shows six spec-only add/add conflicts rather than the previously listed five, so the truthful update must move both the scope boundary and the conflict count together.


# Zapp — SLM PR / release surface decision

- **Date:** 2026-05-05
- **Decision:** Do **not** reuse `feat/slm-conversation-mem` as the next draft-PR head. Its remote ref already backed merged PR #153, while the local head now diverges with a smaller, coherent `v0.18.0` release-truth slice. Publish that slice under a fresh head ref, and keep any future `v0.19.0` PR claims blocked until extraction/correction code actually lands.
- **Why:** A truthful draft PR must describe only the pushed scope. The active branch currently contains manifest + release-doc truth work for the pending `v0.18.0` release lane, not the `slm-extraction-and-correction` implementation proposed for the next lane. Reusing the merged head name would blur those scopes and misstate what is actually ready for review.
- **Consequences:** Open the draft PR now for the pushed `v0.18.0` release-prep slice only, with explicit non-claims for SLM extraction/correction. For `v0.19.0`, keep the pre-tag truth checklist ready: Cargo version parity, release workflow/tag gate, README + getting-started + install docs wording, roadmap status, MCP tool-count copy, and release asset contract all need a fresh truth pass once the implementation branch is real.


### 2026-04-28: Professor Batch 1 watcher-reliability pre-gate — REJECT current closure

**By:** Professor  
**What:** Rejected Batch 1 watcher-reliability closure plan as written due to three blocking contradictions.  
**Why:** Overflow recovery authorization contract must reuse existing `ActiveLease`, not bypass it; `memory_collections` frozen 13-field schema cannot widen without explicit 13.6 reopen; `WatcherMode` semantics contradictory with unreachable `"inactive"` variant.

**Decisions:**
- **D-B1-1:** Overflow recovery operation mode (`OverflowRecovery`) is acceptable as a `FullHashReconcileMode` label, but authorization must remain `FullHashReconcileAuthorization::ActiveLease { lease_session_id }`. No new authorization variant exists.
- **D-B1-2:** `memory_collections` 13.6 frozen 13-field schema must not widen under Batch 1. Watcher health can expand CLI `quaid collection info` only. MCP widening deferred pending explicit 13.6 reopen with design + test updates.
- **D-B1-3:** `WatcherMode` must be truthfully defined: either `Native | Poll | Crashed` only with `null` for non-active/Windows, or `"inactive"` is a real surfaced state with precise definition. No ambiguous mixed contract accepted.

**Verdict:** REJECT Batch 1 closure. Awaiting scope repair. Batch 1 not honestly closable; v0.10.0 not shippable until resolved.

**Result:** Rejection recorded. Leela repair in progress.

---

### 2026-04-29T21:29:11.071+08:00: User directive
**By:** macro88 (via Copilot)
**What:** Start implementation branches from main/origin-main, not from an existing release or dirty branch.
**Why:** User request — captured for team memory


# Fry Batch 3 implementation

- Date: 2026-04-29T20:33:01.970+08:00
- Decision: Keep `page_uuid` dual-read (`quaid_id` first, legacy `memory_id` fallback) but canonicalize every render/write/export to `quaid_id`.
- Why: Batch 3 needs an explicit migration target for files missing `quaid_id`, while existing vault content and fixtures can still be ingested safely during the transition. Reusing `put_from_string` for write-back preserves the rename-before-commit and raw_import invariants without maintaining a second file writer.


# Fry Batch 3 Recon

- **Timestamp:** 2026-04-29T20:33:01.970+08:00
- **Change:** `vault-sync-engine`
- **Scope:** Batch 3 reconnaissance and execution order for `v0.12.0`

## Decision

Implement Batch 3 in this order:

1. Settle the UUID/frontmatter contract seam first (`memory_id` vs `quaid_id`) and thread that decision through the shared UUID helpers.
2. Extract or reuse the existing rename-before-commit writer path so UUID write-back uses the same sentinel + dedup + post-rename stat + single-tx `file_state`/`raw_imports` rotation as `memory_put`.
3. Add bulk-write guard helpers for WriteAdmin/offline-live-owner checks before exposing new CLI entrypoints.
4. Only then wire `collection migrate-uuids` and `collection add --write-quaid-id`, followed immediately by task-aligned tests and OpenSpec checkbox updates.

## Why

The current tree has three coupled seams: the frontmatter key name is still `memory_id`, the only production rename-before-commit implementation lives in `src\commands\put.rs`, and live-owner refusal currently reports only `owner_session_id` even though pid/host data already exists in `serve_sessions`. Landing CLI surface changes before those seams are resolved would either duplicate the write path, produce dishonest task closures, or force follow-up churn across tests and error text.

This ordering also keeps task checkboxes honest. `5a.5`/`17.5ww*` become true when the shared writer path is real, `17.5ii9` becomes true when the live-owner helper is wired, and `5a.5a`/`9.2a` only close once the CLI commands are actually surfaced on top of those lower-level guarantees.

## Notes

- Existing write-gate semantics already live in `src\core\vault_sync.rs::ensure_collection_write_allowed`; Batch 3 should preserve the restoring/`needs_full_sync` fail-closed behavior for WriteAdmin flows.
- The reconciler preflight already points operators at `quaid collection migrate-uuids`, so the new CLI should be added in the same lane as the real write-back implementation, not earlier.


# Leela Batch 3 lane

- **Timestamp:** 2026-04-29T20:33:01.970+08:00
- **Change:** `vault-sync-engine`
- **Scope:** Safe execution lane + release sequencing for Batch 3 / `v0.12.0`

## Decision

Use a separate clean worktree for Batch 3 implementation and release prep staging.

- **Keep untouched:** `D:\repos\quaid` on `release/v0.11.0` (dirty `.squad/` state, ahead of origin by 1 commit)
- **Implementation lane:** `D:\repos\quaid-vault-sync-batch3-v0120`
- **Branch:** `spec/vault-sync-engine-batch3-v0120`
- **Base:** `origin/main` at `fdc20a0` (`Cargo.toml` = `0.11.6`, latest published release = `v0.11.6`)

## Why

The current checkout is not safe for Batch 3 or release work: it is on an old release branch, has uncommitted `.squad/` changes, and diverges from `origin/main`. A sibling worktree isolates implementation from that state and lines Batch 3 up with the real release base.

## Release sequencing constraints

1. Batch 3 is still open in OpenSpec (`5a.5`, `5a.5a`, `9.2a`, `5a.7`, `17.5ww`, `17.5ww2`, `17.5ww3`, `17.5ii9`), so `v0.12.0` cannot ship yet.
2. The next release must not start from `release/v0.11.0`; it must start from the clean main-based lane and merge back to `main` first, per the user instruction.
3. `Cargo.toml` on main is still `0.11.6`; a `v0.12.0` tag would require a version bump before tagging.
4. Release automation is tag-driven (`.github\workflows\release.yml`) and fails if the tag/version mismatch or the 17-file release manifest is incomplete.
5. Coverage over 90% must be checked explicitly via `cargo llvm-cov`; CI reports coverage but does not fail the build for dipping below the requested threshold.

## Routing

- **Fry first:** implement Batch 3 in the clean worktree using the existing recon order (UUID/frontmatter seam → shared write path → live-owner guard → CLI surfaces/tests/OpenSpec checkboxes).
- **Professor second:** review `src\core\vault_sync.rs`, `src\core\page_uuid.rs`, and any shared writer extraction before merge.
- **Scruffy third:** run targeted test/coverage passes and verify the coverage report stays above 90%.
- **Nibbler fourth:** adversarial review of serve-live refusal, write-gate enforcement, and rename-before-commit safety.
- **Zapp last:** only after PR review is complete, comments are resolved, CI is green, and the branch is merged to `main`; then run the release checklist and tag/release `v0.12.0` from the merged state.


# Leela Batch 3 branch ancestry

- **Timestamp:** 2026-04-29T21:29:11.071+08:00
- **Change:** `vault-sync-engine`
- **Scope:** Batch 3 branch ancestry and conflict-risk only

## Decision

No branch-base recovery is needed.

The Batch 3 worktree branch `spec/vault-sync-engine-batch3-v0120` was created from `origin/main` at `fdc20a0` and then received one Batch 3 commit (`4401ed7`). It was not started from `origin/release/v0.11.0`.

## Why

- The branch reflog records the branch creation point as `origin/main`.
- `HEAD` is a single-child commit on top of `fdc20a0`, which is `origin/main` / `v0.11.6`.
- Relative to `origin/release/v0.11.0`, the branch is three commits ahead because it includes the newer main-line commits plus the Batch 3 commit; that is expected ancestry, not evidence of a wrong starting branch.

## Recovery action

Do not rebase, cherry-pick, or rebuild the branch for base-branch reasons. If merge conflicts appear later, treat them as normal forward-integration conflicts from subsequent changes, not as fallout from starting Batch 3 on the wrong base.


# Nibbler Batch 3 review

- **Date:** 2026-04-29T20:33:01.970+08:00
- **Requested by:** macro88
- **Verdict:** REJECT

## Decision

Batch 3 safety is not acceptable to ship.

## Blocking findings

1. `collection add --write-quaid-id` does not truly refuse live serve ownership for the same vault root. The guard is keyed to the newly created `collection_id`, while `collections.root_path` is not unique and `add()` only rejects duplicate names. A second collection row can point at the same canonical root and run bulk UUID rewrites while serve still owns the original row.
2. The bulk UUID rewrite path does not hold an offline owner lease for the duration of the batch. `run_uuid_write_back()` only performs a one-time `ensure_no_live_serve_owner()` preflight, and `collection add --write-quaid-id` drops the fresh-attach short-lived lease before starting the rewrite. A serve session can claim ownership after preflight and race the rewrite mid-batch.
3. The completion claims overstate proof. The landed tests cover `migrate-uuids` live-owner refusal for an existing collection, dry-run, and permission skip, but they do not prove `collection add --write-quaid-id` refusal, the same-root alias case, or the missing lease/race seam.

## Rejected artifacts

- `D:\repos\quaid-vault-sync-batch3-v0120\src\commands\collection.rs`
- `D:\repos\quaid-vault-sync-batch3-v0120\src\core\vault_sync.rs`
- `D:\repos\quaid-vault-sync-batch3-v0120\tests\collection_cli_truth.rs`
- `D:\repos\quaid-vault-sync-batch3-v0120\openspec\changes\vault-sync-engine\tasks.md` (the checked closure claims for `5a.5a`, `9.2a`, `12.6b`, `17.5ii9`)


# Professor Batch 3 Review

**Date:** 2026-04-29T20:33:01.970+08:00
**Reviewer:** Professor
**Verdict:** REJECT

## Blocking findings

1. `src\commands\collection.rs` / `src\core\vault_sync.rs`
   - Batch 3 closes `12.6b` and `17.5ii9` in `openspec\changes\vault-sync-engine\tasks.md`, and the implementation plan says the refusal must name pid/host **and instruct the operator to stop serve first**.
   - The landed `ServeOwnsCollectionError` now includes pid/host, but neither the error text nor the CLI handler adds the required operator guidance. Current tests only assert the tag plus pid/host, so the claimed task closure is not truthful.

## Non-blocking notes

- The shared rename-before-commit seam reuse is honest: `write_quaid_id_to_file(...)` delegates to `put::put_from_string(...)`, so UUID write-back rides the existing sentinel/tempfile/rename/fsync/post-rename-stat/single-tx path instead of introducing a parallel writer.
- The frozen `brain_collections` MCP contract stays closed: `failing_jobs` remains skipped from serialization and the exact-key test still enforces the existing field set.


## 2026-04-29T20:33:01.970+08:00 — Batch 3 coverage lane split

- Keep the Batch 3 proof on the real seams:
  - `src/core/vault_sync.rs` owns atomic UUID write-back, read-only skip, `file_state`/`raw_imports` rotation, and live-owner refusal helpers.
  - `src/commands/collection.rs` owns `collection add --write-quaid-id` / `collection migrate-uuids --dry-run` routing, restoring-state/write-gate checks, and summary shaping.
  - `tests/collection_cli_truth.rs` owns subprocess truth: exit codes, JSON summary, plain-text operator guidance, and serve-live refusal wording.
- Treat `tests/command_surface_coverage.rs` as a last-mile dispatch smoke only; do not spend Batch 3 effort there until the real helper and CLI truth seams are locked.
- Windows iteration should stay cheap: targeted tests first, then `cargo llvm-cov --lib --tests --summary-only --no-clean -j 1`, then `cargo llvm-cov report --json --output-path target\llvm-cov-report.json` for missed-line movement.



# Mom Batch 3 Revision

## Mom Batch 3 Revision

- **Date:** 2026-04-29T21:29:11.071+08:00
- **Decision:** Treat bulk UUID rewrite ownership as a canonical-root seam, not a single-row seam.

### Why

`collection_owners` is keyed by `collection_id`, but `collections.root_path` is not unique. That means `collection add --write-quaid-id` can create an alias row that points at the same vault root while serve still owns a different row, and a row-scoped preflight/lease is not enough to keep the watcher out.

### Applied rule

1. Before `collection add --write-quaid-id` inserts the alias row, preflight the canonical root and fail closed if any same-root row has a live serve owner.
2. For non-dry-run bulk UUID rewrites, acquire one short-lived offline session across **all** collection rows sharing the canonical root and hold it for the entire rewrite loop.
3. Keep the operator-facing refusal honest: tell them to stop serve first, run the bulk rewrite offline, then restart serve.

### Scope

This is intentionally narrow to bulk UUID rewrites (`migrate-uuids` and `collection add --write-quaid-id`). It does not widen generic duplicate-root policy or imply that all collection commands are now root-unique.



# Nibbler Batch 3 rereview

# Nibbler Batch 3 rereview

- **Timestamp:** 2026-04-29T21:29:11.071+08:00
- **Worktree:** `D:\repos\quaid-vault-sync-batch3-v0120`
- **Branch/Commit:** `spec/vault-sync-engine-batch3-v0120` @ `67f4091`
- **Verdict:** **APPROVE**

## Why

1. The same-root alias bypass is closed in both directions:
   - `collection add --write-quaid-id` now refuses before inserting a second row when any same-root alias is live-owned.
   - Bulk UUID rewrite refusal now resolves live ownership by canonical root, not only the target collection row.
2. The offline race is closed at the right seam:
   - non-dry-run UUID write-back acquires a short-lived owner lease covering every same-root collection row before the rewrite loop starts;
   - helper coverage proves the root-scoped lease claims aliases together and cleans up after drop.
3. The operator-facing story is now honest:
   - refusal text explicitly tells operators to stop serve first, rerun offline, then restart serve;
   - task closure notes were narrowed to the actual proof: same-root alias refusal plus a root-scoped lease/source-invariant seam, not a broader claim.

## Residual non-blocking risks

- The end-to-end refusal tests remain Unix-gated, so on a Windows host the rerun evidence comes from helper/unit proof rather than executing the CLI path directly. That matches the current Unix-only command surface, but it is still narrower evidence than a native Unix validation lane.



# Professor Batch 3 re-review

# Professor Batch 3 re-review

- **Date:** 2026-04-29T21:29:11.071+08:00
- **Requested by:** macro88
- **Verdict:** APPROVE
- **Revision reviewed:** 67f4091 on spec/vault-sync-engine-batch3-v0120

## Decision

The revised Batch 3 implementation now honestly closes the prior rejection findings.

## What changed enough to pass

1. collection add --write-quaid-id now refuses before inserting an alias row when any same-root collection is live-owned. The refusal is root-scoped rather than keyed only to the newly created row.
2. Non-dry-run bulk UUID rewrite now acquires a short-lived owner lease across every collection row sharing the canonical root before the rewrite loop begins, so serve cannot claim an alias mid-batch.
3. Operator-facing refusal text now includes the required stop serve first guidance, and the tests/proof seam were updated to cover that wording and the root-scoped lease ordering.
4. openspec/changes/vault-sync-engine/tasks.md no longer overclaims the repaired seam: the revised notes explicitly tie closure to same-root alias refusal, root-scoped lease coverage, and the stop-serve guidance.

## Non-blocking follow-ups

- None.






---
---
timestamp: 2026-04-29T21:29:11.071+08:00
requested_by: macro88
worktree: D:\repos\quaid-v0.12.0-release
branch: release/v0.12.0
head: 90f888ab48fd7e36869b84757a04c5abecffa8ef
topic: v0.12.0 docs/release truth review
---

# Decision: APPROVE `release/v0.12.0` docs truth

## Verdict

APPROVE

## Why

1. `Cargo.toml` is bumped to `0.12.0`, and the public install surfaces now treat `v0.12.0` as branch-prep state rather than pretending the tag is already published.
2. `README.md`, `docs/getting-started.md`, `docs/roadmap.md`, and `website/src/content/docs/tutorials/install.mdx` now truthfully describe the shipped Batch 3 UUID slice: opt-in `quaid collection add --write-quaid-id`, offline `quaid collection migrate-uuids [--dry-run]`, UUID-migration preflight before restore/remap, and `memory_put` preserving `quaid_id`.
3. The docs match the implementation boundary: bulk UUID rewrites are Unix-only and offline, while preserved-UUID behavior is covered on the write/read path.

## Blocking findings

None.

## Non-blocking polish

- Optional: mirror the getting-started page's explicit "Unix-only bulk rewrite" caveat into the README Batch 3 mention so every top-level surface carries the same constraint wording.

---
# Leela decision — v0.12.0 merge lane

- **Timestamp:** 2026-04-29T21:29:11.071+08:00
- **Requested by:** macro88
- **PR:** `#123`
- **Scope:** `release/v0.12.0` final merge lane

## Decision

Clear only the real merge blockers inside the release branch, then merge normally. That meant fixing the flaky env-var test race, adding coverage for the env-guard restore path so `codecov/patch` cleared, accepting the docs correction raised in review, resolving the review threads, and explicitly avoiding an admin merge.

## Why

- The branch itself was already the intended release-prep lane and was only blocked by merge policy.
- The failing `Test` / `codecov/patch` gate and the unresolved review conversations were all scoped to the branch and could be repaired surgically without reopening release scope.
- Admin merge would have hidden a real quality gate failure and violated the no-bypass rule already established for merge-lane work.

## Outcome

- PR `#123` merged cleanly into `main`.
- The exact `main` SHA to tag for `v0.12.0` is `5a8bdf068bf54be52f9b2bc661af34056473221a`.




# Fry Batch 4 gap audit

- **Date:** 2026-04-30T06:37:20.531+08:00
- **Change:** `vault-sync-engine`
- **Scope:** Read-only Batch 4 audit for tasks `12.1`, `12.6`, `12.6a`, `12.6b`, `12.7`

## Decision

Do **not** start Batch 4 implementation on this branch state yet. The rename-before-commit core is close, but `12.6b` is blocked by missing Batch 3 UUID-write surfaces, and the remaining `12.1` gap is still a real source-seam issue rather than a checkbox cleanup.

## Guardrails

1. Keep the Unix platform gate narrow; do **not** widen Windows vault-write support as part of this slice.
2. Keep `memory_collections` on the frozen 13-field MCP schema; no Batch 4 work should add fields there.

## Task 12.1 — full 13-step rename-before-commit

### Already implemented

- Shared writer core exists and is used by both CLI and MCP through `src\commands\put.rs::put_from_string(...)` and `persist_with_vault_write(...)` (`src\commands\put.rs:100-191`, `342-623`).
- The current writer already covers most of the design sequence:
  - step 1 CAS / write gate: `resolve_slug_for_op`, `ensure_collection_vault_write_allowed`, `check_update_expected_version` (`src\commands\put.rs:109-117`, `376-381`; `src\core\vault_sync.rs:556-577`)
  - step 3 precondition: `check_fs_precondition_before_sentinel(...)` (`src\commands\put.rs:382-387`; `src\core\vault_sync.rs:667-674`)
  - step 4 sha256: `prepared.sha256` (`src\commands\put.rs:166`, `372-375`)
  - steps 5-6 sentinel + tempfile fsync: `create_recovery_sentinel(...)`, `create_tempfile(...)` (`src\commands\put.rs:390`, `424-438`, `652-719`)
  - step 7 symlink guard: `stat_at_nofollow(...)` check before rename (`src\commands\put.rs:439-451`)
  - step 8 dedup insert: `insert_write_dedup(...)` + `remember_self_write_path(...)` (`src\commands\put.rs:467-489`)
  - steps 9-11 rename, parent fsync, post-rename stat/inode/hash guard (`src\commands\put.rs:506-595`)
  - steps 12-13 single SQLite tx + sentinel unlink (`src\commands\put.rs:597-623`)

### Partially implemented

- The filesystem precondition logic itself is good and tested (`src\core\vault_sync.rs:581-700`, `5259-5402`), but it is still wired as a separate helper that reopens the root / parent rather than operating on the final trusted parent fd that the writer later uses.
- Post-rename abort handling is already fail-closed and sentinel-backed (`src\commands\put.rs:750-778`), so the recovery model is mostly correct even before the last seam is repaired.

### Still missing

- **Step 2 is not design-complete.** `walk_to_parent(...)` has no `create_dirs=true` mode (`src\core\fs_safety.rs:58-132`), and the writer still falls back to path-based `fs::create_dir_all(parent)` before reopening the parent fd (`src\commands\put.rs:392-412`). That is the main remaining `12.1` gap.
- The actual step ordering is still split: precondition runs through `check_fs_precondition_before_sentinel(...)` before the final parent fd is opened for writing (`src\commands\put.rs:382-387` vs `399-412`), instead of one exact fd-relative sequence.
- The symlink refusal path still returns a generic I/O error string (`"target path is a symlink"`) rather than a dedicated typed write error (`src\commands\put.rs:439-449`).
- The implementation-plan pointer is stale: it says audit `put_from_string` in `vault_sync.rs`, but the production writer lives in `src\commands\put.rs`.

### Tests that already exist

- Precondition/OCC before sentinel: `unix_update_without_expected_version_conflicts_before_sentinel_creation`, `unix_stale_expected_version_conflicts_before_sentinel_creation`, `unix_external_delete_conflicts_before_sentinel_creation`, `unix_external_create_conflicts_before_sentinel_creation`, `unix_fresh_create_succeeds_without_existing_file_state` (`src\commands\put.rs:1221-1347`)
- Failure matrix and recovery: sentinel failure, pre-rename failure, rename failure, parent fsync failure, foreign rename, commit busy recovery, foreign-rename + startup recovery (`src\commands\put.rs:1462-1754`)
- Filesystem-precondition behavior: fast path, ctime self-heal, hash mismatch, same-size external rewrite (`src\core\vault_sync.rs:5259-5402`)

### Tests still missing

- Explicit tempfile `fsync` failure coverage (today there is no dedicated hook for the tempfile fsync branch)
- Explicit post-rename `stat` failure coverage
- Explicit dedup-insert collision / duplicate-entry failure coverage
- Typed symlink-escape coverage (today only the raw error string is present)

## Task 12.6 — mandatory `expected_version` everywhere

### Already implemented

- MCP enforces the contract up front:
  - existing page + missing `expected_version` → conflict (`src\mcp\server.rs:589-615`, tests at `1651-1673`, `1677-1707`)
  - stale `expected_version` → conflict (`src\mcp\server.rs:589-615`, tests at `1711-1740`)
  - create with unexpected `expected_version` → conflict (`src\mcp\server.rs:597-604`, tests at `1814-1828`)
- The Unix CLI/write-through core also enforces missing/stale update versions before sentinel creation (`src\commands\put.rs:376-381`, tests at `1221-1280`).
- CLI help text already documents the intended rule: `--expected-version` required for Unix updates, optional for creates (`src\main.rs:41-46`).

### Partially implemented

- The real OCC rule is already present for the shipped MCP and direct Unix CLI path, so this task is mostly a truth-closure task rather than a missing-core-logic task.

### Still missing

- The contract is not yet closed through the deferred live-routing path from `12.6a`; `quaid put` still writes directly regardless of serve ownership.
- There is still a non-Unix fallback path and test that allow unconditional update semantics (`src\commands\put.rs:323-339`, `1780-1792`). Do **not** widen platform support to “fix” this; instead keep the Unix gate truthful and keep Batch 4 scoped to vault-write surfaces only.

### Tests that already exist

- MCP OCC tests: `src\mcp\server.rs:1651-1828`
- Unix CLI-core OCC tests: `src\commands\put.rs:1221-1280`

### Tests still missing

- A serve-owned CLI-routing test proving the same OCC contract still holds once `12.6a` is implemented

## Task 12.6a — `quaid put` live-owner/offline routing

### Already implemented

- Core owner-lease infrastructure exists:
  - `acquire_owner_lease(...)` / `owner_session_id(...)` (`src\core\vault_sync.rs:1865-1910`)
  - tests for refusing a live foreign owner and reclaiming stale residue (`src\core\vault_sync.rs:6422-6492`)

### Partially implemented

- `ServeOwnsCollectionError` exists, but it only carries `owner_session_id`, not the `pid/host` detail required by the Batch 4 wording (`src\core\vault_sync.rs:307-310`).

### Still missing

- `quaid put` is still direct-dispatch only:
  - `main.rs` sends `Commands::Put` straight to `commands::put::run(...)` (`src\main.rs:301-305`)
  - `commands::put::run(...)` only applies the Unix gate, reads stdin, and calls `put_from_string(...)` (`src\commands\put.rs:90-97`)
  - there is **no** live-owner detection, no refusal instructing “use MCP or stop serve”, no offline temporary lease/heartbeat wrapper, and no IPC path
- This task must stay in the refuse-or-offline shape only; do not reopen Batch 5 IPC work here.

### Tests that already exist

- Only lower-level lease helper tests in `vault_sync.rs` (`6422-6492`)

### Tests still missing

- `quaid put` refuses while a live serve owner exists
- `quaid put` acquires/releases an offline owner lease when no live owner exists
- refusal message includes pid/host once the error surface is repaired

## Task 12.6b — bulk rewrite routing

### Already implemented

- Nothing user-facing for this task is actually implemented yet.

### Partially implemented

- The branch has prerequisite clues only:
  - restore/reconcile status text already tells operators to run `migrate-uuids work` in the trivial-content halt case (`src\commands\collection.rs:3000-3005`)
  - Batch 3 tasks remain open in `tasks.md` (`openspec\changes\vault-sync-engine\tasks.md:116-121`, `174`, `236`, `373`, `418-419`)

### Still missing

- `CollectionAction` still has **no** `MigrateUuids` variant (`src\commands\collection.rs:19-55`)
- `CollectionAddArgs` still uses the old `write_memory_id` field name, and `add(...)` explicitly rejects it as deferred (`src\commands\collection.rs:58-67`, `234-237`)
- There is a direct defer-test proving the flag is still blocked (`src\commands\collection.rs:1790-1812`)
- No live-owner refusal exists for bulk UUID rewrites because the bulk UUID rewrite commands themselves do not exist yet
- Even if they did exist, the current `ServeOwnsCollectionError` cannot yet name pid/host

### Batch 3 stale/incomplete callout

- `tasks.md` is honest that Batch 3 remains open (`5a.5`, `5a.5a`, `9.2a`, `17.5ii9`, `17.5ww`, `17.5ww2` are still unchecked), but the current `implementation_plan.md` is stale where it says Batch 3 bulk-write routing “already implements” the `12.6b` refusal (`openspec\changes\vault-sync-engine\implementation_plan.md:221`).
- That stale assumption is contradicted by the live code in `src\commands\collection.rs`, which still rejects `--write-quaid-id` and exposes no `migrate-uuids` command.

### Tests that already exist

- Only the defer test: `add_rejects_write_memory_id_before_creating_collection_row` (`src\commands\collection.rs:1790-1812`)

### Tests still missing

- `migrate-uuids` offline success
- `migrate-uuids --dry-run` no-op
- `collection add --write-quaid-id` live-owner refusal
- bulk refusal message naming pid/host and stop-serve guidance

## Task 12.7 — tests

### What already exists

- Strong direct coverage already exists for:
  - OCC-before-sentinel and filesystem-precondition cases (`src\commands\put.rs:1221-1347`)
  - per-slug mutex behavior (`src\commands\put.rs:1351-1458`)
  - sentinel/pre-rename/rename cleanup (`src\commands\put.rs:1462-1538`)
  - parent-fsync failure (`src\commands\put.rs:1578-1615`)
  - foreign rename / concurrent rename (`src\commands\put.rs:1619-1653`)
  - commit failure and sentinel-driven startup recovery (`src\commands\put.rs:1657-1754`)
  - MCP-side OCC / no-vault-mutation assertions (`src\mcp\server.rs:1651-1828`)

### What is still missing

- explicit tempfile fsync failure
- explicit post-rename stat failure
- explicit dedup-entry collision
- CLI live-owner routing tests (`12.6a`)
- bulk UUID rewrite routing tests (`12.6b`, blocked by missing Batch 3 commands)

## Concrete implementation checklist once branch state is corrected

1. **Do not touch platform scope or MCP schema.**
   - Keep the Unix gate closed.
   - Keep `memory_collections` frozen at 13 fields.
2. **Repair Batch 3 first; Batch 4 depends on it.**
   - Add `CollectionAction::MigrateUuids { name, dry_run }`
   - Rename `write_memory_id` to the truthful `write_quaid_id`
   - Implement the actual bulk UUID writer by reusing the production writer path, not a second file rewrite path
   - Add the live-owner refusal for those bulk commands, with pid/host detail
   - Mark Batch 3 tasks immediately as each one is truly done
3. **Finish the real `12.1` seam.**
   - Replace the path-based `fs::create_dir_all(...)` fallback with an fd-relative parent-directory creation/walk flow
   - Unify the write sequence so the precondition and rename operate on the same trusted parent-fd path
   - Add a typed symlink-escape error instead of a generic I/O string
4. **Implement `12.6a` in the narrowed Batch 4 shape only.**
   - Before direct `quaid put`, detect a live owner from `collection_owners` + `serve_sessions`
   - If live owner exists, refuse and instruct the operator to use MCP or stop serve
   - If no live owner exists, acquire a temporary offline lease + heartbeat around the direct write, then release it
5. **Close `12.7` with the missing failure tests.**
   - tempfile fsync failure
   - post-rename stat failure
   - dedup collision
   - CLI live-owner refusal / offline lease flow
   - bulk UUID rewrite routing once Batch 3 surfaces exist
6. **Protect the >90% coverage bar during the implementation lane.**
   - keep new tests inline with the touched modules
   - rerun the existing coverage command after Batch 3 + Batch 4 land together


---
created_at: 2026-04-30T06:37:20.531+08:00
author: Leela
type: routing-decision
subject: Batch 4 execution lane — recovery path from stale checkout
---

# Decision: Batch 4 Branch Routing and Recovery Path

## Context

The current working directory (`D:\repos\quaid`) is parked on `release/v0.11.0`, which is
12 commits ahead of `origin/release/v0.11.0` (all Scribe log commits) and is **not on main**.
`origin/main` is at `v0.12.0` (SHA `5a8bdf0`). The local tasks.md shows Batch 3 items as
open only because the stale branch predates the Batch 3 merge — all Batch 3 closures
(`5a.5`, `5a.5a`, `9.2a`, `5a.7`, `17.5ww`, `17.5ww2`, `17.5ww3`, `17.5ii9`, `12.6b`, `17.5www`)
are confirmed closed on `origin/main`. No `v0.13.0` tag or `release/v0.13.0` branch exists.
There are 2 modified `.squad/` files and 1 untracked `.squad/` health report in the working tree.

## Decision

**Batch 4 work begins in a sibling worktree created from `origin/main`.**

The `D:\repos\quaid` checkout is NOT touched for Batch 4 code work. The stale
`release/v0.11.0` working tree's dirty files are low-risk (`.squad/` only) and do not
conflict with a sibling worktree's object store.

### Worktree setup

```powershell
cd D:\repos\quaid
git fetch origin main --tags
git worktree add ..\quaid-vault-sync-batch4-v0130 -b spec/vault-sync-engine-batch4-v0130 origin/main
```

Starting SHA: `5a8bdf0` (tagged `v0.12.0`, confirmed clean).

### Batch 4 task scope

Open tasks on `origin/main`:
- `12.1` — complete the 13-step rename-before-commit sequence (audit `put_from_string` against all 13 steps; wire steps 2 `walk_to_parent`, 3 `check_fs_precondition`, 7 symlink defense-in-depth, and 8 dedup insert timing on ALL vault-byte write paths)
- `12.6` — mandatory `expected_version` enforcement audit across MCP + CLI (no blind-update escape hatch)
- `12.6a` — CLI write routing for `quaid put` single-file (refuse with `ServeOwnsCollectionError` when live owner exists; offline lease path when no live owner)
- `12.6b` — **ALREADY CLOSED** on main (Batch 3 Mom revision). Verify guard in place; no re-implementation needed.
- `12.7` — tests covering every rename-before-commit failure mode (tempfile fsync error, parent fsync error, commit error, foreign rename in window, concurrent dedup entries, external write mid-precondition)

### Agent assignments

| Agent | Task |
|-------|------|
| Fry | Implements 12.1, 12.6, 12.6a, 12.7 in the sibling worktree |
| Scruffy | Monitors unit test coverage ≥ 90% throughout |
| Professor | Code peer review of 12.1 (security-adjacent) and 12.6 (contract enforcement) |
| Nibbler | Adversarial review of 12.6a (CLI write routing, live-owner detection) |
| Bender | End-to-end validation pass after Fry signals implementation complete |
| Amy | Documentation review for any new error types or CLI changes |
| Zapp | Release lane: `release/v0.13.0` → PR → merge to main → tag `v0.13.0` after all gates clear |

### Gate sequence before code begins

1. ✅ No active reviewer gate (all prior Batch 3 gates cleared at v0.12.0 merge)
2. ✅ No v0.13.0 tag collision
3. ✅ `origin/main` is clean at `5a8bdf0`
4. ✅ Batch 3 closures verified on `origin/main` — no re-closure needed
5. **Required before first commit:** Fry creates the worktree as specified above

### Gate sequence before release

1. `cargo test` green in the worktree
2. Coverage ≥ 90% confirmed by Scruffy (CI publishes coverage evidence; Scruffy must confirm manually)
3. Professor and Nibbler approve (no admin-merge around reviewer gates — lesson from v0.12.0)
4. All review threads resolved
5. `release/v0.13.0` branch PR opened against `main`
6. PR merged cleanly
7. Zapp creates annotated tag `v0.13.0` from merge SHA and pushes it

### Constraints

- **Do NOT merge Batch 4 into or from `release/v0.11.0`** — that branch is dead.
- **Do NOT touch the 3 dirty files in `D:\repos\quaid`** during Batch 4 — they are Scribe artifacts and should be committed or pruned separately by Scribe.
- Tasks `12.6c`–`12.6g` (IPC socket) are **Batch 5 scope** — do not pull them into Batch 4.
- `12.6b` is already closed; Batch 4 only needs to verify the guard is present, not re-implement it.

## Risk flags

- `12.1` is security-adjacent (rename-before-commit discipline). Professor must review before merge, not after.
- The coverage threshold is not CI-enforced — human confirmation required before Zapp starts release lane.
- `now.md` is stale (updated 2026-04-25). The active branch field says `spec/vault-sync-engine` but actual work branch is a sibling worktree. No action needed for Batch 4 execution, but Scribe should update `now.md` after Batch 4 lands.


---
created_at: 2026-04-30T06:37:20.531+08:00
author: Scruffy
type: testing-decision
subject: Batch 4 coverage baseline and closure guard
---

# Decision: Batch 4 coverage baseline and truthful closure gate

## Context

A read-only Batch 4 assessment on `D:\repos\quaid` found that the current repo-wide Rust
coverage baseline is **89.47%** from
`cargo llvm-cov --lib --tests --summary-only --no-clean -j 1`.

The Batch 4 lane is uneven:

- `src\core\vault_sync.rs` — 83.22% line coverage
- `src\commands\put.rs` — 95.70%
- `src\commands\collection.rs` — 91.70%
- `src\mcp\server.rs` — 96.90%

The same assessment also confirmed that Batch 4 routing tasks are still genuinely open:
`quaid put` does not yet perform live-owner routing, `ServeOwnsCollectionError` still lacks
the pid/host detail required by the spec, `--write-quaid-id` is still explicitly deferred,
and there is no `migrate-uuids` collection subcommand in the current command surface.

## Decision

**Do not claim Batch 4 is above 90% or closure-complete unless validation includes both:**

1. a fresh `cargo llvm-cov --lib --tests --summary-only --no-clean -j 1` run, and
2. a refreshed `target\llvm-cov-report.json` via
   `cargo llvm-cov report --json --output-path target\llvm-cov-report.json`.

**Do not close `12.6`, `12.6a`, `12.6b`, or `12.7` on the current surface.**

## Rationale

- The repo is already below the stated 90% bar before any Batch 4 code lands.
- `vault_sync.rs` is the dominant coverage risk, so touching it without direct backfill is
  likely to worsen both patch and project coverage.
- The current codebase has good low-level OCC and rename-failure proof, but it still lacks the
  live-owner routing and bulk UUID rewrite surfaces needed for truthful closure of the open
  Batch 4 tasks.

## Lean validation path

For Batch 4 implementation work, the lean honest path is:

1. targeted Rust tests for `src\commands\put.rs` and `src\core\vault_sync.rs`
2. any new CLI truth tests needed for live-owner refusal / offline lease flow
3. final coverage rerun with the two-command llvm-cov loop above

This keeps scope tight while still proving the real Batch 4 contract.

# Bender — conversation memory baseline

- **Date:** 2026-05-04T07:22:12.881+08:00
- **Decision:** Do not call the conversation-memory branch release-ready yet, even though the current baseline clears the requested line-coverage bar.
- **Why now:** The measured baseline is good enough on code health (`cargo llvm-cov report` = 92.11% TOTAL line coverage; default coverage run, online-feature tests, clippy, cargo check, release-asset parity, and install-release seam all passed), but the release lane still has two hard gates: `Cargo.toml` is still `0.17.0`, so the tag-driven `release.yml` would reject `v0.18.0`, and the >90% coverage requirement still depends on explicit human confirmation because CI only reports coverage. Local `tests/install_profile.sh` failures are permission-semantics noise from the Windows bash / NTFS environment, not evidence that the Linux/macOS release asset contract is broken.
- **Next gate:** Let implementation continue, but do not open or merge a release-bound PR until the version bump is in the actual release candidate commit and someone reruns `cargo llvm-cov report` on the final tree to re-confirm the line-coverage floor.

# Fry — conversation-memory-foundations schema slice

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Implement the first conversation-memory schema slice as a strict v8 foundation patch on top of the existing `pages.type` model, not by renaming the column to `kind` or introducing a migration lane. The new session-expression index must guard `json_extract(...)` with `json_valid(frontmatter)` so malformed-frontmatter rows remain tolerated while the new v8 artefacts are present.

## Why

The repo already ships `SCHEMA_VERSION = 8`, so the honest minimal slice is to add the new `superseded_by`/`extraction_queue` artefacts, strengthen tests, and keep v7 databases on the existing schema-mismatch/re-init path. A raw `json_extract(frontmatter, '$.session_id')` expression index broke existing malformed-frontmatter tolerance in unit tests, so the guarded form is the safe way to land the session lookup seam without widening this slice into frontmatter-cleanup or migration work.

# Fry — Batch 7 PR opening gate

**Date:** 2026-05-02T21:49:40.366+08:00  
**Requested by:** macro88  
**Change:** vault-sync-engine

## Decision

Open the Batch 7 product PR from `sync-engine/batch-7` to `main` after committing and pushing the non-`.squad` branch work. Merge remains blocked until review feedback exists and is fully resolved in a later pass.

## Why

This records the explicit review gate for the Batch 7 lane and keeps the release handoff truthful: `v0.17.0` is still deferred until the PR lands and post-merge validation is rerun on `main`.

# Leela — conversation-memory-foundations batching gate

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Do not fan implementation past the already-started schema edits until the OpenSpec artifacts are truth-repaired and routing is reset. Treat the schema work as a v8 → v9 change until proven otherwise, resolve the `pages.type` versus `pages.kind` DDL mismatch in the artifacts before more Section 1 work, and require Nibbler pre-gate on the watcher/file-edit slice before Fry starts task 10. Open the draft PR after the corrected preflight slice plus the first stable implementation slices land (`1.1–2.5` and `11.1–11.2`), not at the end of the 70-task change.

## Why

The repo already advertises schema version 8 in code and schema, while the change artifacts still describe a v7 → v8 reset. The current tasks also specify `idx_pages_supersede_head ON pages(kind, superseded_by)` even though the live table stores that field as `type`, so leaving the artifacts unchanged would make the first batch lie about what is actually shipping. The branch is already dirty with partial work on this change, so the safe routing move is to pause widening, repair the truth in the specs/tasks, then continue under explicit reviewer and coverage gates.

# Leela — conversation-memory-foundations truth repair

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Truth-repair this change so it explicitly treats schema v8 plus the landed first plumbing slice (`pages.superseded_by`, the head/session indexes, `extraction_queue`, config defaults, and `Page.superseded_by`) as the current baseline. Rewrite stale `pages.kind` references to `pages.type`, and keep tasks `1.1`–`1.8` / `2.1` checked by rephrasing them as already-landed baseline work. Remaining implementation scope starts at `2.2`; no additional schema bump is in scope.

## Why

The live repo already ships the first slice, so leaving the artifacts on a planned `v7 → v8` bump and `pages(kind, superseded_by)` would make reviewers and implementers work against a false baseline. Reframing the checked tasks keeps scope unchanged while making OpenSpec honest about what is already landed versus what remains.

# Professor — conversation-memory-foundations slice 1 review

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations  
**Commit:** a1ceae8

## Decision

Reject Fry's first slice for tasks 1.1-1.8 and 2.1. The code lands the narrow `pages.type` + guarded-`json_valid(frontmatter)` variant, but the OpenSpec artifacts still mark done against the older `pages(kind, ...)` and raw `json_extract(frontmatter, ...)` wording, so the shipped contract and the checked task text are out of sync.

## Highest-priority issue

**Spec/task truth mismatch:** `proposal.md` and `tasks.md` still describe the wrong schema contract for the checked items. This slice is only reviewable after those artifacts are rewritten to match what actually shipped.

## Gate outcome

- **Professor:** REJECT
- **Reason:** schema truth / task honesty failure, not a code correctness failure
- **Lockout:** Fry may not author the next revision of this rejected artifact

## Evidence

- `src/schema.sql` ships `idx_pages_supersede_head ON pages(type, superseded_by)` and guards the session index with `json_valid(frontmatter)`.
- `openspec/changes/conversation-memory-foundations/proposal.md` and `tasks.md` still describe `pages(kind, superseded_by)` and an unguarded `json_extract(frontmatter, '$.session_id')`.
- `cargo test --quiet -j 1` passed during review, so the rejection is about contract truth, not failing tests.

## 2026-05-04T07:22:12.881+08:00 — Conversation-memory slice 1 test gate

- `src\core\db.rs` already carries the high-value slice-1 proofs: schema v8 artefacts/defaults, `superseded_by` foreign-key enforcement, `extraction_queue` CHECK failures, and v7 rejection on open/init.
- The practical seam to keep green while Fry widens the slice is every hand-built `Page` fixture. When `Page` gains a field, update those fixtures in the same commit and add one serde-backcompat test proving legacy payloads still deserialize with the new field defaulted.
- Coordinator gate nuance: run `cargo test --quiet -j 1` with `RUST_TEST_THREADS=1` before `cargo llvm-cov --lib --tests --summary-only --no-clean -j 1` (also with `RUST_TEST_THREADS=1`). The plain serial test pass flushes fixture drift and the `commands::embed` ordering flake early; otherwise the coverage lane fails late on compile-only or order-sensitive targets and muddies the real coverage signal.

# Zapp — conversation memory release lane

- **Date:** 2026-05-04T07:22:12.881+08:00
- **Decision:** Do not open the draft PR for `feat/slm-conversation-mem` yet.
- **Why now:** The branch has no remote tracking ref or PR, the working tree mixes uncommitted implementation work with unrelated doc moves, and the public release surfaces are still stale (`v0.15.0` language, 17-tool copy, `roadmap.md` references, and `MIGRATION.md` links if that move lands).
- **Earliest safe moment:** After the branch is pushed with a coherent commit set, the draft body can truthfully describe the landed slice, and the public docs/release references are repaired. `Cargo.toml` should only move to `0.18.0` on the actual release-bound commit that will be tagged.
