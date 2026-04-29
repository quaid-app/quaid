# Amy decision inbox — v0.11.0 doc truth repair

- **Date:** 2026-04-28T21:46:33.929+08:00
- **Requested by:** macro88

## Decision

For a release branch that exists before the GitHub tag is published, user-facing docs must separate:

1. **The branch target** — what the upcoming release is preparing (here: `v0.11.0` / Batch 2).
2. **The latest published tag** — what GitHub Releases downloads and `install.sh` can actually install today.

## Applied rule

- Status and release-summary prose may describe the upcoming slice on the branch.
- Install snippets that fetch GitHub Release assets must use `<published-tag>` placeholders or explicit "latest published tag" wording until the release workflow completes.
- If readers need the unreleased branch behavior, docs should point them to a source build instead of implying the tag already exists.

## Why

This branch already carries the Batch 2 embedding-worker work, but a literal `v0.11.0` install/download command would be false until the tag is live. Separating "branch target" from "published asset" keeps README, getting-started, and docs-site install guidance truthful during the release lane.


# Decision: Clippy type_complexity fix — release/v0.11.0

**Date:** 2026-04-28T21:46:33.929+08:00  
**Author:** Bender  
**Branch:** release/v0.11.0  
**Commit:** e42b7b3

## Decision

Introduce a `type` alias `EmbeddingJobRow = (i64, i64, String, i64, i64)` in `src/core/vault_sync.rs` to resolve the `clippy::type_complexity` lint error that was blocking the `cargo clippy --all-targets -- -D warnings` gate.

## Rationale

- Professor rejected the release because `cargo clippy --all-targets -D warnings` was red.
- The sole blocker was the return type of `load_embedding_job_candidates` — a 5-tuple inside `Vec` inside `Result` crosses Clippy's complexity threshold.
- A `type` alias is the minimal, behavior-preserving fix. Using `#[allow(clippy::type_complexity)]` was rejected as suppression rather than resolution.
- No OpenSpec tasks affected — this is a lint-only fix with no semantic change.

## Scope

- **Changed:** `src/core/vault_sync.rs` — 4-line addition (doc comment + type alias line + blank line + updated fn signature).
- **Not changed:** Any test, OpenSpec task, documentation, or other source file.

## Gate verification

`cargo clippy --all-targets -- -D warnings` → exits 0 after fix.


# Leela Batch 2 Revision Decision

- **Timestamp:** 2026-04-28T21:46:33.929+08:00
- **Change:** `vault-sync-engine`
- **Scope:** Batch 2 release revision for `v0.11.0`

## Decision

1. **Schema v7 parity is mandatory across embedded DDL and open-path checks.** `src/schema.sql` now seeds `config.version='7'` so a crash-partial fresh database is not misclassified as legacy by `preflight_existing_schema()` while `src/core/db.rs` already enforces `SCHEMA_VERSION = 7`.
2. **Task 8.6 stays closed only under the accepted frozen-MCP contract.** `memory_collections` remains frozen at 13 fields; failed embedding jobs are surfaced via `quaid collection info` (plain text and `--json`) rather than widening MCP in a release patch.

## Why

- Professor's rejection named the schema-version mismatch as the highest-priority release blocker; leaving the DDL seed at `6` while the open path enforced `7` made the schema bump non-atomic.
- The accepted OpenSpec contract after Batch 1 repair freezes `memory_collections`. Reintroducing `failing_jobs` there would silently reopen task 13.6 and break the exact-key test in `src/mcp/server.rs`.
- Hiding `failing_jobs` from `quaid collection info --json` was still a truth gap, because JSON output is part of the public CLI surface.

## Files

- `src/schema.sql`
- `src/core/db.rs`
- `src/commands/collection.rs`
- `tests/collection_cli_truth.rs`
- `openspec/changes/vault-sync-engine/implementation_plan.md`
- `openspec/changes/vault-sync-engine/tasks.md`


# Decision: v7 bootstrap crash-window recovery stays fail-closed outside fresh bootstrap state

**Date:** 2026-04-28T21:46:33.929+08:00  
**By:** Mom  
**Status:** Proposed

## Context

Professor rejected the prior Batch 2 revision because `preflight_existing_schema()` accepted a crash-partial fresh v7 database while `open_with_model()` still rejected the same database when `quaid_config` was empty. That left the real bootstrap crash window unresolved.

## Decision

1. Treat an empty `quaid_config` as recoverable **only** when the database is still bootstrap-fresh:
   - exactly the seeded default collection exists
   - no user rows exist in mutable runtime tables (`pages`, `raw_imports`, `file_state`, `embedding_jobs`, links/assertions/gaps, etc.)
   - `embedding_models` has at most one active row and no inactive leftovers
2. If `embedding_models` already has an active row, use that row as the authoritative model hint when rebuilding `quaid_config`.
3. Any non-fresh database with an empty `quaid_config` still fails closed and requires re-init / operator intervention.

## Why

This closes the real crash window without silently “repairing” databases that already carry user state. It also avoids trusting the legacy `config.embedding_model` seed, which is only a bootstrap default and can disagree with the true runtime-selected model.

## Proof

- `src/core/db.rs` regression tests now prove both `open_with_model()` and `init()` recover the fresh crash-partial v7 state.
- A separate regression keeps the fail-closed path for databases that already contain page data.


2026-04-28T21:46:33.929+08:00 — Professor final Batch 2 / release v0.11.0 gate

Decision: REJECT the current Batch 2 + release-lane artifact.

Why:
- The v7 schema bump is not atomic/self-consistent: `src/schema.sql` still seeds `config.version = ''6''` while `src/core/db.rs` enforces `SCHEMA_VERSION = 7`. A crash after schema bootstrap but before legacy-config repair can leave a fresh DB that self-rejects as legacy on next open.
- Task 8.6 is marked done prematurely. `failing_jobs` is hidden from both `memory_collections` and `quaid collection info --json` via `#[serde(skip_serializing)]`, so the promised machine-readable observability contract is not actually shipped.
- Release workflow prep is incomplete: `.github/workflows/release.yml` now links `docs/getting-started.md`, but that document still advertises `v0.10.0` / Batch 1.

Highest-priority fix:
1. Make the v7 schema bootstrap self-consistent in the embedded DDL / open path, then rerun the Batch 2 review with task 8.6 honesty and release-doc alignment repaired.

Lockout:
- Fry is locked out of the next revision of this rejected Batch 2 / release artifact.


# Professor Batch 2 Re-review

- **Timestamp:** 2026-04-28T21:46:33.929+08:00
- **Change:** `vault-sync-engine`
- **Scope:** Batch 2 / `v0.11.0` release blocker rereview
- **Verdict:** REJECT

## Decision

The release blocker is **not** cleared.

### Highest-priority blocker

`src\schema.sql` now seeds the legacy `config.version` to `7`, but the reopen path in `src\core\db.rs` still rejects a fresh crash-partial v7 database when `quaid_config` exists but is empty. `preflight_existing_schema()` now passes that database, yet `open_with_model()` falls through `read_quaid_config() == None` to `Schema` re-init failure, so the bootstrap crash window remains user-visible.

## Secondary findings

1. **Task 8.6 truth repair is otherwise acceptable.** `quaid collection info --json` now surfaces `failing_jobs`, while `memory_collections` stays on the frozen 13-field MCP contract.
2. **Release docs are mostly repaired, but not fully.** `README.md`, `docs/getting-started.md`, and `website/src/content/docs/tutorials/install.mdx` now distinguish the unreleased branch target from the latest published tag, but `docs/roadmap.md` still incorrectly says `memory_collections` surfaces `failing_jobs`.

## Why this stays blocked

- The prior top blocker was crash-safety of the schema bump. A release cannot proceed while a newly initialized DB can still strand itself in a self-rejecting state after a narrow crash.
- The remaining roadmap mismatch is lower severity than the reopen bug, but it confirms the release lane is not yet fully truthful.

## Required next repair

1. Make the fresh-v7 bootstrap path reopenable when `quaid_config` is absent/empty but the embedded schema is already current.
2. Add a regression that proves `db::open()` or `open_with_model()` succeeds on that crash-partial state, not just that preflight no longer misclassifies it.
3. Correct the roadmap wording so `failing_jobs` is described as CLI-only for this release.


## 2026-04-28T21:46:33.929+08:00 — Professor final release gate for v0.11.0

**Verdict:** REJECT

### What cleared

- The prior blocking findings are materially closed:
  - v7 schema seed/runtime mismatch is repaired (`src/schema.sql` seeds version 7 and `src/core/db.rs` reopens crash-partial fresh DBs cleanly).
  - The crash-partial bootstrap window now has direct regression coverage in `src/core/db.rs`.
  - Batch 2 truth drift is repaired: `memory_collections` stays frozen, while `quaid collection info` surfaces `failing_jobs`.
  - Release docs and roadmap are now truthful about the unpublished `v0.11.0` lane and the frozen MCP contract.
  - Coverage evidence clears the requested bar (`cargo llvm-cov report` shows 90.77% line coverage).

### Remaining blocker

- The branch is **not** release-landable because the required CI lint gate is red:
  - `cargo clippy --quiet -- -D warnings` fails on `src/core/vault_sync.rs:3347` with `clippy::type_complexity`.
  - Repo CI (`.github/workflows/ci.yml`) treats Clippy warnings as errors for release gating.

### Why this blocks release

Behaviorally, Batch 2 is now honest and well-covered for 8.1–8.6, 17.5ee, and 17.5ff. But the actual artifact still cannot enter the release execution lane while the enforced lint gate fails on touched code. Fix the `load_embedding_job_candidates` return-type complexity (or equivalently factor it into a named type), rerun Clippy, and this gate can be reconsidered.


# Zapp decision inbox — Batch 2 roadmap truth

- **Date:** 2026-04-28T21:46:33.929+08:00
- **Requested by:** macro88

## Decision

For the unreleased `v0.11.0` lane, roadmap copy must describe the accepted Batch 2 diagnostic split exactly:

1. `memory_collections` stays frozen at the 13-field MCP schema and only surfaces `embedding_queue_depth`.
2. `failing_jobs` is a CLI diagnostic exposed via `quaid collection info` rather than a new `memory_collections` field.

## Why

Professor's verdict explicitly rejected roadmap wording that implied `memory_collections` exposed `failing_jobs`. The approved OpenSpec Batch 2 closure and the exact-key contract in `src/mcp/server.rs` keep MCP frozen, while `src/commands/collection.rs` is the truthful public surface for failed-job reporting.

## Files

- `docs/roadmap.md`
- `openspec/changes/vault-sync-engine/implementation_plan.md`
- `openspec/changes/vault-sync-engine/tasks.md`
- `src/mcp/server.rs`
- `src/commands/collection.rs`


---

### 2026-04-23T18:01:18+08:00: User directive
**By:** macro88 (via Copilot)
**What:** Once the current work is done and pushed remote, start the next session to (1) achieve 90%+ code coverage and 100% pass, (2) update project docs completely, (3) update the public docs to be best in class, (4) get a PR in and merged, (5) release v0.9.6, (6) after merge to main, clean up unused files such as leftover root .md files on a feature branch and open a cleanup PR, and (7) check all issues and close what is fixed or no longer relevant/actionable.
**Why:** User request — captured for team memory


# Fry — 13.3 CLI slug parity decision

- **Date:** 2026-04-24
- **Change:** `vault-sync-engine`
- **Scope:** Task `13.3` only

## Decision

For CLI surfaces, apply collection-aware slug resolution at the command boundary, then emit canonical page references as `<collection>::<slug>` anywhere the CLI returns a page-facing slug.

## Why

N1 made MCP the reference contract for collection-aware routing and canonical page references. Matching that contract in CLI output closes the real parity seam for users and reviewers without widening into the explicitly deferred `13.5` collection-filter/defaults work or `13.6` new MCP-tool work.

## Applied pattern

1. Resolve user input once with the existing collection-aware resolver.
2. Convert subsequent lookups to `(collection_id, slug)` or `page_id` so explicit routes and duplicate bare slugs stay unambiguous.
3. Canonicalize only at the CLI boundary when rendering page-referencing output.

## Notes

- `check --all` needed the same keyed approach internally so duplicate slugs across collections no longer collide.
- `query`, `search`, and `list` remain out of the deferred collection-filter scope; this slice only makes their returned slugs canonical.


# 2026-04-24: Vault-sync-engine Batch 13.5 read-filter default rule

**By:** Fry

**What:** Implement `brain_search`, `brain_query`, and `brain_list` collection filtering through one shared read-helper with this exact precedence: explicit `collection` wins; otherwise the sole active collection is the effective filter when exactly one collection is active; otherwise fall back to the write-target collection.

**Why:** The planning trap for 13.5 was widening the default into “all active collections otherwise.” Recording the helper rule keeps the slice aligned with the reviewed scope, makes the behavior testable in one place, and avoids reintroducing the ambiguity when later MCP read surfaces grow collection-aware filters.


# Fry Decision - 13.6 `brain_collections` projection truth

## What

For the `13.6` + `17.5ddd` slice, `brain_collections` is implemented as a read-only projection helper in `src\core\vault_sync.rs` that returns the frozen design schema directly, including:

- `root_path = null` whenever `state != "active"`
- parsed tagged-union `ignore_parse_errors`
- `integrity_blocked` as the string-or-null discriminator
- `recovery_in_progress` backed by a narrow process-local runtime registry during `complete_attach(...)`

## Why

The design contract is stricter than the existing CLI `collection info` surface. Reusing the CLI status summary would have overreported deprecated meanings (`manifest_incomplete_pending`, `post_tx_b_attach_pending`) and would not have distinguished queued recovery from actively running attach recovery.

The runtime registry addition is intentionally narrow: it does not widen mutator behavior or watcher semantics, it only gives the read-only MCP projection truthful visibility into the already-existing attach/recovery work.

## Consequences

- MCP callers now get the exact frozen field set for this slice without shelling out to CLI info.
- Future work on `13.5`, watcher health, queue health, or broader runtime surfacing should extend this projection helper instead of inventing a second collection-status schema.


# Fry post-batch coverage decision

- **Date:** 2026-04-25
- **By:** Fry

## What

Treat `reconciler::has_db_only_state(...)` as the single authoritative hard-delete gate for the current quarantine surface. `quarantine discard` now consults that shared predicate before deleting, while `db_only_state_counts(...)` remains only for user-facing error/detail output.

## Why

The recent coverage pass needed one truthful invariant across all destructive paths: reconciler missing-file handling, quarantine discard, and TTL sweep must all agree on the same five-branch preservation rule. Centralizing the delete decision on the shared predicate keeps the deferred restore surface untouched, improves reviewer confidence, and gives us a stable source-level invariant test for future regression catches.


# Fry — quarantine slice decision

- **Date:** 2026-04-25
- **Scope:** vault-sync-engine narrow quarantine lifecycle batch

## Decision

Interpret the `17.5i` / `9.8` discard contract from the current design/spec, not the stale task wording: a quarantined page with DB-only state may be discarded **either** with explicit `--force` **or** after a successful same-quarantine-epoch `quarantine export`.

## Why

The design and capability spec both describe `--force OR prior export`, while the task line for `17.5i` still reads like export is required even when `--force` is present. For this batch I implemented same-epoch export tracking with a dedicated `quarantine_exports` table, kept `--force` as the immediate destructive override, and left `quarantine audit` plus restore overwrite/export-conflict policy deferred.

## Consequences

- `collection quarantine discard <slug>` now succeeds without `--force` only after a matching current-epoch export.
- `collection quarantine discard <slug> --force` remains the explicit destructive bypass.
- If the team wants stricter discard semantics later, that should be a deliberate spec/task repair, not an accidental interpretation change inside this batch.


# Fry — Vault Sync Batch H decisions

Date: 2026-04-22
Change: `vault-sync-engine`
Batch: H

## Decision 1: Drift-capture authorization stays closed and identity-carrying

- `full_hash_reconcile` now uses explicit `RestoreDriftCapture` / `RemapDriftCapture` modes.
- Authorization remains a closed enum and now carries caller identity:
  - `RestoreCommand { restore_command_id }`
  - `RestoreLease { lease_session_id }`
  - `ActiveLease { lease_session_id }`
  - `AttachCommand { attach_command_id }`
- Validation fails before opening the root or walking files.

**Why:** This preserves Batch G’s fail-closed surface while allowing Batch H’s non-active drift-capture use cases. The caller, not `full_hash_reconcile`, decides whether drift is acceptable.

## Decision 2: Canonical trivial-content predicate is single-sourced

- Added shared helper logic so rename inference and UUID-migration preflight use the same rule:
  - non-trivial = body after frontmatter is non-empty and `>= 64` bytes
  - trivial = body after frontmatter is empty or `< 64` bytes

**Why:** This closes the review seam Professor called out. Restore/remap preflights and rename inference now make the same call on short/template notes.

## Decision 3: Fresh-attach clears the write gate only after dedicated full-hash success

- Added `fresh_attach_reconcile_and_activate()` which runs `full_hash_reconcile` in `FreshAttach` mode and only then flips the collection back to `state='active'` with `needs_full_sync=0`.

**Why:** Batch H is allowed to land the core fresh-attach seam, but not the higher-level watcher/supervisor choreography. This keeps the write-gate sequencing honest without over-claiming the remaining attach orchestration work.


# Fry — Vault Sync Batch I decisions

Date: 2026-04-22
Change: `vault-sync-engine`
Batch: I

## Decision 1: `collection_owners` is authoritative; collection lease IDs are authorization mirrors only

- Restore/remap ownership resolution now reads the live owner exclusively from `collection_owners`.
- The existing `collections.active_lease_session_id` / `restore_lease_session_id` columns remain only as persisted identity mirrors for the already-approved full-hash authorization layer.

**Why:** This keeps ownership single-sourced while avoiding a risky rewrite of the Batch H authorization surface.

## Decision 2: Serve runtime owns both exact acking and RCRT recovery

- `gbrain serve` now registers a `serve_sessions` lease, heartbeats it, sweeps stale sessions, writes exact `(session_id, reload_generation)` release acks only for post-startup generation bumps, and runs the restoring-collection retry pass.
- Fresh serve startup seeds its observed generation map from current DB state, so it does not impersonate an already-restoring predecessor.

**Why:** Batch I needed a single runtime actor for release/reattach without dragging watcher product scope into the slice.

## Decision 3: The write gate is enforced through shared collection-aware slug resolution

- Shared helpers now resolve explicit and bare slugs against collections and enforce the fail-closed gate when `state='restoring'` OR `needs_full_sync=1`.
- The gate now covers CLI/MCP `put`, `check`, `link`, `tags` mutations, `timeline-add`, `brain_raw`, and slug-bound `brain_gap`, while slug-less `brain_gap` remains intentionally allowed.

**Why:** The adversarial seam was not just page-byte writes; any mutator that can land DB state during restore/remap needed the same collection-aware interlock.


# Decision: Fry Batch K2 implementation notes

**Author:** Fry  
**Date:** 2026-04-23

## Decision

For Batch K2, keep plain `gbrain collection sync <name>` closed and use **only** `gbrain collection sync <name> --finalize-pending` as the explicit CLI completion path for offline restore after Tx-B.

## Why

This preserves the pre-gate constraint against turning plain sync into a generic recovery multiplexer while still allowing `17.11` to be proven through a real CLI chain. It also gives `collection info` one truthful operator target for both pending-finalize and post-Tx-B attach-pending restore states.

## Consequences

- Offline restore now reaches an honest CLI-completable path without relying on serve/RCRT for the `17.11` proof.
- Post-Tx-B restore state must surface as attach-pending, not generic restoring.
- Terminal integrity failures remain reset-required; finalize never auto-clears `integrity_failed_at`.


### 2026-04-25: Vault-sync watcher core stays on the existing serve runtime loop

**By:** Fry

**What:** Landed the watcher core as a narrow extension of `start_serve_runtime()` instead of introducing a separate watcher supervisor or worker service. Each active collection now gets one `notify` watcher, a bounded `tokio::mpsc` queue, a debounce buffer, and flushes through the existing reconciler path. Self-write echo suppression also stays in the existing process-global runtime registries as a shared path+hash+instant map.

**Why:** This slice needed to prove watcher ingestion and dedup behavior without widening into the deferred supervision / health / ignore-reload / broader mutation choreography work. Reusing the current serve loop keeps the code reviewer-friendly, preserves one reconciliation truth path, and lets `gbrain put` and watcher suppression share the same process-local state without IPC.


---
date: 2026-04-25
author: hermes
status: inbox
---

# Hermes: Public Docs Refresh Decisions

## Decision 1: Homepage serve snippet replaced with accurate stdio example

**What:** Removed the fake terminal output (`"Server active at http://localhost:8080"`) from `index.mdx` and replaced it with a three-command sequence (`init` → `import` → `serve`) with no fabricated output.

**Why:** `gbrain serve` is a stdio MCP server, not an HTTP server. The previous snippet misled new users about the transport layer. The new snippet is accurate and also surfaces the `import` step that was missing from the homepage funnel.

## Decision 2: Version pinned in install.mdx updated to v0.9.4

**What:** Both the GitHub Releases `VERSION` variable and the installer `GBRAIN_VERSION` pins updated from `v0.9.2` → `v0.9.4` throughout `install.mdx`.

**Why:** v0.9.4 is the current release. Version examples pointing at older releases create confusion and silently install older binaries.

## Decision 3: Schema v5 replaces v4 in docs prose

**What:** `getting-started.mdx` step 01 description and `contributing.md` repo layout updated from "v4 schema" → "v5 schema".

**Why:** The vault-sync-engine branch ships schema v5. Advertising v4 understates what `gbrain init` creates and could confuse contributors looking at the actual `schema.sql`.

## Decision 4: brain_collections added to mcp-server guide as vault-sync-engine section

**What:** Added a "vault-sync-engine — Collections and write safety" table to the Available Tools section in `mcp-server.md`, documenting `brain_collections` with a full response shape example.

**Why:** `brain_collections` is the 17th MCP tool and its seam is closed (task 13.6 merged). Omitting it from the MCP guide means anyone who discovers it via `gbrain call brain_collections '{}'` has no public reference. The tool count (16 → 17) updated in `phase3-capabilities.md` and `getting-started.mdx` accordingly.

## Decision 5: vault-sync-engine section added to roadmap

**What:** Added a full vault-sync-engine section to `roadmap.md` listing what's landed, what's deferred, and why the gate is open.

**Why:** The README roadmap already acknowledged vault-sync-engine. The docs-site roadmap was silent on it. Visitors reading docs see Phase 3 as the end of the story, which understates the project's current trajectory. The new section is honest: restore and IPC are explicitly called out as deferred.

## Decision 6: Quarantine restore and IPC NOT documented

**What:** No docs reference quarantine `restore`, IPC sockets, or `17.5pp/qq` work.

**Why:** Per task instructions ("do not advertise unfinished restore/IPC work as available") and per the Bender truth repair that backed restore out of the live surface. These remain deferred until crash-durable cleanup and no-replace install land.


# Decision: M1b-i and M1b-ii Repairs

**Author:** Leela  
**Date:** 2026-05-XX  
**Context:** Nibbler rejected M1b-i; Professor + Nibbler rejected M1b-ii. Bender locked out of M1b-i revision; Fry locked out of M1b-ii revision.

---

## M1b-i: Re-scope accepted — production behavior in command layer acknowledged

**Nibbler's rejection:** M1b-i closure note claimed "Explicit mutator matrix in `src/mcp/server.rs`" but the write-gate enforcement for `brain_link` and `brain_check` lives in library functions in `commands/link.rs` and `commands/check.rs`, not in MCP server.rs. These are production behavior changes, not proof-only tests.

**Decision:** Re-scope 17.5s5 to explicitly own those production behavior changes. The gates are correct and appropriate behavior; the bookkeeping was dishonest. Updated tasks.md:

- **17.5s2 note** — Added repair note acknowledging that `brain_link` and `brain_check` gates live in the command layer, not solely in `server.rs`.
- **17.5s5 note** — Updated to explicitly re-scope as a behavior lane: `commands/link.rs::run_silent` calls `ensure_collection_write_allowed` for both from/to collection IDs; `commands/check.rs::execute_check` calls `resolve_slug_for_op` + `ensure_collection_write_allowed` (slug-mode) or `ensure_all_collections_write_allowed` (all-mode). These are real behavior changes, not proof-only tests.
- Added **17.5s6** to own the M1b-ii repair (see below).

**No code changes required for M1b-i.** The behavior is correct. The fix is truthfulness in tasks.md.

---

## M1b-ii: Collection interlock ordering fixed in brain_put

**Professor + Nibbler's rejection:** `brain_put` in `server.rs` ran OCC/precondition prevalidation (version/existence checks) BEFORE the collection write-gate. A blocked collection (`state='restoring'` OR `needs_full_sync=1`) with a pre-existing page at the slug would return a `-32009` version-conflict error instead of `-32002 CollectionRestoringError`. Similarly, a blocked collection with a non-existent page and `expected_version` supplied would return "does not exist at version N" instead of `CollectionRestoringError`. Not Unix-only.

**Decision:** Fix the ordering. `CollectionRestoringError` / write-gate MUST win over any OCC/precondition conflict.

**Code change in `src/mcp/server.rs`:**  
Added `vault_sync::ensure_collection_write_allowed(&db, resolved.collection_id)` immediately after `resolve_slug_for_op` and **before** the OCC prevalidation block in `brain_put`. This is a cross-platform call (pure DB state query, no `#[cfg(unix)]` needed). The prevalidation itself is retained — it provides useful error messages with `current_version` in error data for callers.

**New ordering-proof tests added:**
- `brain_put_collection_interlock_wins_over_update_without_expected_version` — page exists, collection restoring, `expected_version=None` → `-32002 CollectionRestoringError` (not "already exists")
- `brain_put_collection_interlock_wins_over_ghost_expected_version` — page absent, collection restoring, `expected_version=Some(1)` → `-32002 CollectionRestoringError` (not "does not exist at version 1")

**All 87 MCP server tests pass.** All 16 check/link command tests pass.

---

## Deferred items (unchanged)

The following remain deferred as per the existing tasks.md boundaries:
- No full `12.1`, `12.4`, `12.5`, `12.6*`, `12.7`
- No dedup `7.x`
- No happy-path `17.5k`
- No IPC / live routing / generic startup healing
- No `17.5pp`, `17.5qq`, `17.5rr–vv6`

---

## Rule for future handlers

Any MCP handler that does OCC/precondition checking AFTER `resolve_slug_for_op` **MUST** call `ensure_collection_write_allowed` (or `ensure_collection_vault_write_allowed` if writable-flag checking is also needed) **BEFORE** any version/existence prevalidation. The collection write-gate always wins.


# M2a / M2b Gate Reconciliation

**Author:** Leela  
**Date:** 2026-04-25  
**Context:** Professor and Nibbler returned conflicting gate results on the proposed M2a and M2b batches. This memo resolves those conflicts into two concrete, gateable replacement slices.

---

## What the Reviewers Actually Agreed On

Reading past the surface disagreement, both reviewers share the same underlying position on every contested point — they just used different framing:

| Point | Professor | Nibbler | Actual Consensus |
|-------|-----------|---------|-----------------|
| `2.4a2` + `17.16` | ✅ Approve | ✅ (implicit, not contested) | Include in M2a |
| `17.16a` scope | Vault-byte mutators only | Vault-byte write entry points only | Same scope; task wording is the bug |
| `12.5` status | Approve as a task | Already live for brain_put/gbrain put | Both right: it's live at vault-byte scope; the task just needs a closure note |
| `12.4` + narrow `17.5k` | Approve (replaces M2b) | Approve | Include in M2b |
| `17.17e` | No — not yet | Yes — with vault-byte caveats | Dispute resolves by enforcing task-text scope |

---

## The Real Bug in the Original Proposals

The original M2a overloaded `17.16a` with the phrase "every mutating command." Code inspection refutes this:

- `ensure_collection_vault_write_allowed` (the `writable=0` gate) is called by `commands/put.rs::put_from_string` and by `commands/collection.rs` for the CLI path. Both `gbrain put` and `brain_put` hit it.
- `brain_link`, `brain_check`, `brain_raw`, etc. call `ensure_collection_write_allowed`, which checks `state=restoring` and `needs_full_sync` only — **not `writable=0`**.
- The test `brain_put_refuses_when_collection_is_read_only` (tagged `17.5qq11`) already exists and passes. `put_refuses_read_only_collection` in `commands/collection.rs` covers the CLI path.

So `12.5` is **substantively complete for vault-byte write paths** and `17.16a` as written is a false claim. Both reviewers saw this; they stated it differently.

---

## Recommendation

### M2a' — Platform Gate + Vault-Byte Read-Only Closure

**Batch name:** M2a'

**Include:**
- `2.4a2` — `#[cfg(windows)]` handlers return `UnsupportedPlatformError` from `gbrain serve`, `gbrain put`, and the vault-sync `collection` subcommands. Offline commands may still run.
- `17.16` — Integration: Windows platform gate returns `UnsupportedPlatformError`.
- `17.16a` (narrowed) — Integration: vault-byte write entry points (`gbrain put` + `brain_put` via `put_from_string`) refuse `writable=0` with `CollectionReadOnlyError`. **Task wording must be corrected before implementation: remove "every mutating command"; replace with "vault-byte write entry points (gbrain put + brain_put via put_from_string)."** Existing unit tests (`brain_put_refuses_when_collection_is_read_only`, `put_refuses_read_only_collection`) may satisfy this if reviewers accept them as sufficient integration-level evidence; otherwise add a single integration test covering both paths together.

**`12.5` — treat as already substantively complete; wording/proof cleanup only:**  
The gate exists in production code at `vault_sync::ensure_collection_vault_write_allowed` and is already exercised by two tests. **`12.5` should be closed with a tasks.md closure note** stating: "CollectionReadOnlyError enforcement live for vault-byte write paths (gbrain put / brain_put via put_from_string). DB-only mutators (brain_link, brain_check, brain_raw, etc.) are explicitly out of scope per the K1 repair ruling on 9.2b; that deferred surface has no current batch target." No new production code is required for `12.5`. This closure note update is the only action needed.

**Defer:**  
Everything else. `12.5` broader mutator coverage has no batch target and is not blocked on anything in this slice.

**Why this resolves the conflict:**  
Professor's approval stands — the vault-byte scope is exactly what Professor conditioned approval on. Nibbler's rejection is resolved — the "every mutating command" overclaim is removed and `12.5` is correctly treated as already-live at the proved scope.

**Caveats:**
- `2.4a2` must not claim `2.4c` (symlink-skipping walk integration) is done.
- `17.16a` closure note must name the actual files and functions that enforce the behavior (`vault_sync::ensure_collection_vault_write_allowed`, called from `put_from_string`), per M1b-repair discipline.
- The broader `writable=0` mutator surface (brain_link, brain_check, brain_raw) remains explicitly deferred with no batch target.

---

### M2b' — Per-Slug Mutex + Mechanical Write-Through Proof

**Batch name:** M2b'

**Sequence:** After M2a' (the platform gate makes the Unix-only scope explicit and keeps M2b' clean).

**Include:**
- `12.4` — Per-slug async mutex serializes within-process writes. Not a substitute for DB CAS.
- `17.5k` (narrow) — `brain_put` mechanical happy path on Unix: `expected_version` check → precondition → sentinel → tempfile → rename → parent-fsync → single-tx commit succeeds on the correct-version path. **Closure note must state explicitly:** "dedup echo suppressed" claim is deferred to post-7.x. This task proves only the mechanical sequence.
- `17.17e` — Named invariant: the enumerated vault-byte write entry points (`brain_put` create-with-existing, `brain_put` update, CLI `gbrain put`) check `expected_version` BEFORE any tempfile, dedup insert, FS mutation, or DB mutation. **Closure note must name the actual enforcement sites** (12.3 closure, `put_from_string` ordering, pre-sentinel CAS check) and must not claim any DB-only mutator is in scope.

**Defer:**  
Full `17.5k` echo suppression (post-7.x), `12.6*`, `12.7`, `7.x`, watcher, IPC, and broader `17.17e` coverage of non-vault-byte mutators.

**Why this resolves the conflict:**  
Nibbler's approval holds — `12.4` + narrow `17.5k` + `17.17e` with Unix-only, within-process, not-a-CAS-substitute, no-dedup-echo/IPC/watcher/live-serve caveats is exactly what Nibbler approved.  
Professor's rejection is addressed — `17.17e` as written in tasks.md already enumerates specific vault-byte entry points (`brain_put create-with-existing, brain_put update, CLI gbrain put`). The dispute was about overclaiming scope. With the closure note explicitly bounded to that enumerated set and naming the actual enforcement sites, Professor's concern is met. If Professor still objects after seeing this scoping, `17.17e` should be deferred to a post-M2b' wording-only fix, but it should not block `12.4` + narrow `17.5k`.

**Caveats:**
- `12.4` closure note: "within-process only, not a substitute for DB CAS; per-slug granularity means different-slug concurrent writes are never blocked."
- `17.5k` closure note: "dedup echo claim deferred to post-7.x. This task proves the mechanical sequence only."
- `17.17e` closure note: "vault-byte write entry points only, as enumerated in the task text. DB-only mutators not in scope."
- M2b' does not claim `brain_put` is production-ready for live serve. Live serve safety requires dedup (7.x) and IPC routing (12.6*), both deferred.
- `17.17e` is proof-only. No production gate may appear silently under a "tests-only" claim without naming where production enforcement lives.

---

## Summary: What Happens to Each Original Task ID

| Task | Disposition |
|------|-------------|
| `2.4a2` | ✅ In M2a' — implement Windows platform gate |
| `12.5` | ✅ **Close with wording/proof cleanup only** — vault-byte path already live; tasks.md closure note required; no new code |
| `17.16` | ✅ In M2a' — integration test for Windows gate |
| `17.16a` | ✅ In M2a' — **task wording must be narrowed first**; vault-byte paths only |
| `12.4` | ✅ In M2b' — implement per-slug async mutex |
| `17.5k` (narrow) | ✅ In M2b' — mechanical happy-path proof; no dedup echo claim |
| `17.17e` | ✅ In M2b' — named invariant test scoped to enumerated vault-byte entry points; if Professor still objects, defer to post-M2b' wording fix (does not block `12.4` + `17.5k`) |

---

## Pre-Implementation Actions Required Before M2a' Can Start

1. **tasks.md wording fix for `17.16a`:** Replace "every mutating command" with "vault-byte write entry points (gbrain put + brain_put via put_from_string)."
2. **tasks.md closure note for `12.5`:** Add the vault-byte scope confirmation and DB-only mutator deferral note above.

These are tasks.md edits only. No code gate required before these edits; Fry may land them in the same PR as M2a' implementation.

---

## What Remains Deferred After M2a' + M2b'

No change from the M1b deferral list. The IPC pre-review (Nibbler adversarial gate for `12.6*`) should be opened as a parallel track concurrent with M2 landing. No M2 sub-slice unblocks IPC.


# Next Slice After 13.3 — Leela Sequencing Decision

**Date:** 2026-04-24  
**Author:** Leela  
**Branch:** spec/vault-sync-engine  
**Preceding closed batch:** 13.3 (CLI slug parity)

---

## Decision

**Next slice: `13.6` + `17.5ddd`**

---

## Included Task IDs

| ID | Description |
|----|-------------|
| `13.6` | New `brain_collections` MCP tool — returns per-collection object per design.md § `brain_collections` schema |
| `17.5ddd` | Proof: `brain_collections` response shape matches design.md schema exactly |

---

## Deferred Task IDs (explicit)

| ID | Reason for deferral |
|----|---------------------|
| `13.5` | Complex default-filter semantics ("write-target in single-writer setups, all collections otherwise") introduce non-trivial routing logic and risk of widening; should land after `brain_collections` (13.6) establishes the collection-state ground truth |
| `17.5ss` | Slug resolution proof tests — standalone proof cluster, separate slice |
| `17.5tt` | Same |
| `17.5uu` | Same |
| `17.5vv` through `17.5vv6` | Same |
| `14.1`, `14.2` | `gbrain stats` augmentation — different surface, not a natural co-traveler |
| `17.5ddd` (without 13.6) | Not a standalone proof; must ship with the tool |
| All `6.*` (watcher) | Still deferred |
| All `7.*` (dedup) | Still deferred |
| All `8.*` (embedding queue) | Still deferred |
| `11.9`, `12.6*` (IPC) | Still deferred |
| `9.8`, `9.9` (quarantine commands) | Still deferred |

---

## Why This Is the Right Next Slice

**Narrowest truthful increment after 13.3:**  
13.6 is a pure read-only MCP tool addition. It does not modify any existing tool, introduces no mutation path, requires no write-target semantics, and has no IPC/watcher/dedup dependency. The proof surface is exactly one test (`17.5ddd`) that validates schema fidelity against the design.md spec.

**Why not 13.5 first?**  
13.5 introduces collection-filter semantics on three existing search/list tools, including a default-filter decision ("write-target in single-writer setups, all collections otherwise"). That default encodes state logic about collection ownership that `brain_collections` (13.6) will expose. Landing 13.6 first establishes the observable state model before 13.5 filters on it. Reversing the order risks the 13.5 default logic being under-tested or overclaiming what single-writer detection means.

**Why not 17.5ss–vv proof cluster?**  
These are proof tests for section 2 `parse_slug` resolution rules. They are unrelated to the MCP output surface and should be scoped as their own proof-closure slice — analogous to how M1b-i/ii were proof closure after the implementation landed in M1a. Mixing them with 13.6 would blur the reviewer focus from "does `brain_collections` match the spec shape" to "do 6 resolution rule tests also pass."

**Reviewer-clean landability:**  
- Professor: verifies `brain_collections` response object fields match design.md schema exactly (13.6 + 17.5ddd is a clean spec-fidelity review, no edge-case resolution logic)  
- Nibbler: verifies no unintended state leakage (collection internals not over-exposed; read-only guard confirmed)  
- Scruffy: proof coverage on 17.5ddd shape test; no new mutation paths to track

The increment is standalone: it does not require any prior open item, does not require any deferred surface to be meaningful, and the reviewer workload is scoped to a single new read-only tool.

---

## Ownership

| Role | Agent |
|------|-------|
| Implementation owner | Fry |
| Gate reviewers | Professor (spec shape), Nibbler (state exposure / read-only invariant) |
| Proof coverage | Scruffy |

---

## Post-Landing

After 13.6 closes, the natural follow-on is the slug-resolution proof cluster (`17.5ss`–`17.5vv6`) as a standalone proof slice, which will then unblock 13.5's default-filter semantics with a validated resolution ground truth underneath it.


# Next Slice After N1 — Sequencing Gate

**Author:** Leela  
**Date:** 2026-04-24  
**Branch:** spec/vault-sync-engine  
**Anchor commit:** `532c972` — Close N1 MCP slug routing truth

---

## Decision

**Include in next batch (N2): `13.3` only.**

**Defer: `13.5`, `13.6`.**

---

## What N1 Left Open

N1 closed the slug-routing seam on MCP surfaces only:
- `13.1` — MCP slug-bearing handlers resolve collection-aware input ✅
- `13.2` — MCP responses emit canonical `<collection>::<slug>` ✅
- `13.4` — `AmbiguityError` payload shape is stable ✅

Still open: `13.3` (CLI parity), `13.5` (collection filter on search/query/list), `13.6` (`brain_collections` tool).

---

## Why `13.3` Is the Right Next Slice

**Library seam is already proven.** `collections::parse_slug` handles both bare slugs and `<collection>::<slug>` and was exercised in N1. `resolve_slug_for_op` wraps it for write ops. No new design decisions are needed.

**The gap is CLI routing inconsistency, not a design gap.** Several CLI commands do NOT route through `resolve_slug_for_op` and instead pass slug strings directly to bare-slug DB queries:

- `graph.rs` → `neighborhood_graph(slug, ...)` → `WHERE slug = ?1` — no collection-awareness
- Similar pattern likely in `backlinks`, `embed`, and any CLI command that calls graph/FTS core directly

These commands will silently fail or return wrong results on a `<collection>::<slug>` input. 13.3 closes this by ensuring every slug-bearing CLI entry point routes through collection-aware resolution consistently.

**CLI output canonical form is also open.** The 13.2 closure was MCP-only. CLI output (e.g., `gbrain graph`, `gbrain get --json`) does not yet emit canonical `<collection>::<slug>` addresses. This is part of 13.3 scope.

**Reviewer-clean landability.** 13.3 is provably narrow:
- No new DB tables or queries
- No new design decisions — same contract as N1
- Professor can verify that CLI canonical output matches the N1 MCP canonical form contract
- Nibbler can verify no ambiguity-resolution regression on multi-collection CLI inputs
- Scruffy adds integration tests for `<collection>::<slug>` input on each slug-bearing CLI command

---

## Why `13.5` Is Deferred

13.5 adds an optional `collection` filter parameter to `brain_search`, `brain_query`, and `brain_list`. This requires:
- New parameter wiring (MCP schema change)
- Default-behavior logic: "filter by write-target in single-writer, all collections otherwise" — a new design decision not yet proven in any prior batch
- New DB query path (WHERE clause extension for collection-scoped FTS/vector search)

This is broader scope than CLI parity. It belongs after 13.3 closes the routing consistency gap.

---

## Why `13.6` Is Deferred

13.6 introduces a new `brain_collections` MCP tool. This requires:
- New tool registration in `server.rs`
- New DB query returning the per-collection object schema from `design.md §brain_collections`
- Potentially new fields not yet surfaced in any prior batch

Most scope in group 13. Correctly deferred until 13.3 and 13.5 are closed.

---

## Ownership

| Role     | Assignment |
|----------|-----------|
| Owner    | Fry       |
| Reviewer | Professor — verify CLI canonical output matches MCP N1 contract |
| Reviewer | Nibbler — verify multi-collection ambiguity regression on CLI surfaces |
| Test     | Scruffy — integration tests for `<collection>::<slug>` CLI inputs |

---

## Clean Stop Point

If no safe narrow slice existed, the clean stop would be after N1. But 13.3 is safe and narrow — it applies a proven seam to an adjacent surface with no new design.

---

## Explicit Deferral List

| Task | Reason |
|------|--------|
| `13.5` | New filter param + default-behavior logic; more scope than CLI parity |
| `13.6` | New MCP tool + new DB query surface; highest scope in group 13 |
| `14.*` | Stats update; independent of slug routing; correctly queued after group 13 |
| `15.*` | Legacy ingest removal; depends on `16.*` docs; queued for post-sync cleanup phase |
| `16.*` | Documentation pass; end-of-branch gate |


---
leela_id: next-gate-2026-04-25
issue_date: 2026-04-25T03:45:00Z
branch_state: after-commit-43d2117
closed_scope: |
  13.5, 13.6, 9.10/9.11, 17.5aa5, watcher core (6.1-6.4, 7.1-7.4, 7.6)
deferred_scope: |
  watcher overflow/supervision/health/live ignore reload,
  remaining dedup 7.5,
  broader watcher choreography,
  IPC/live routing, online restore handshake,
  destructive restore/remap surfaces (Phase 4, Tx-A),
  bulk UUID mutations (ServeOwnsCollection gates only, no proxy),
  embedding queue full suite,
  legacy ingest removal,
  documentation,
  follow-up stubs (daemon-install, openclaw-skill)
---

# Next Truthful Slice: Dedup Completion + Quarantine Delete Classifier

## Exactly One Next Slice

**Task IDs to close now:**
- `7.5` — Failure handlers: dedup removal after rename failure or post-rename abort
- `17.5g7` — Quarantine export: dump all five DB-only-state categories as JSON
- `17.5h` — Auto-sweep TTL: discard clean quarantined pages, preserve DB-only-state
- `17.5i` — Quarantine discard `--force`: require exported JSON for DB-only-state
- `17.5j` — Quarantine restore: re-ingest and reactivate file_state
- `9.8` — `gbrain collection quarantine {list,restore,discard,export,audit}`
- `9.9` — Auto-sweep TTL: configurable retention, preserve DB-only-state pages
- `9.9b` — `gbrain collection info` surfaces quarantine count

**Test tasks to close:**
- `17.4` — `.gbrainignore` atomic parse unit tests
- `17.5g7` — quarantine export JSON proof
- `17.5h` — auto-sweep TTL discard/preserve proof
- `17.5i` — discard --force with DB-only-state proof
- `17.5j` — quarantine restore re-ingest proof
- `17.5z` — gbrainignore parse failure preserves mirror
- `17.5y` — gbrainignore valid edit triggers reconcile
- `17.5aa` — gbrainignore absent-file semantics

## Files Likely Involved

**Core implementation:**
- `src/core/vault_sync.rs` — dedup remove-on-failure helpers
- `src/core/reconciler.rs` — update failure unlink path
- `src/commands/put.rs` — add sentinel unlink and dedup cleanup on failure
- `src/core/quarantine.rs` — **new file** — export, classify, TTL sweep
- `src/commands/collection.rs` — add `quarantine` subcommand + `info` quarantine count
- `src/mcp/server.rs` — expose quarantine list + stats

**Testing:**
- `tests/quarantine_lifecycle.rs` — **new file** — export/discard/restore round-trip
- `tests/dedup_failure_cleanup.rs` — **new file** — dedup removal after sentinel failure
- `tests/ignore_atomic_parse.rs` — existing, add negative cases

## What Remains Deferred

**Strictly out of scope to keep this honest:**
- Watcher overflow recovery (`needs_full_sync` set, recovery task polling) — `6.7a`
- Watcher health/supervision/restart logic — `6.10`, `6.11`
- Live `.gbrainignore` reload during serve — `6.8`
- Full dedup echo suppression beyond `17.5bb-dd` narrow proof — `7.5` full failure suite
- IPC socket, write routing, proxy mode — `12.6a`-`12.6g`
- Online restore handshake `(session_id, reload_generation)` ack — `17.5pp`-`17.5qq`
- Remap Phase 4 bijection verification — `17.5ii4`-`17.5ii5`
- Embedding job queue + worker — §8 full, `17.5ee`-`17.5gg`
- `migrate-uuids` + `--write-gbrain-id` bulk UUID rewrite — `9.2a`, `12.6b`, `17.5ii9`
- Legacy `gbrain import` removal — `15.1`-`15.4`
- Documentation refresh — §16

## Why This Slice Is Next

1. **Completes the dedup contract** — Watcher core lands dedup on happy path (`17.5bb-dd`), but failure handling (`7.5`) is unfinished. This slice closes the vault-byte write dedup story end-to-end before any broader watcher mutation or recovery machinery runs.

2. **Closes quarantine lifecycle** — Reconciler and restore-recovery both produce quarantined pages; we have no way to export, inspect, discard, or restore them yet. CLI and MCP surfaces are bare. Auto-sweep would silently delete recoverable pages. This slice is the operator-facing quarantine resolution that validates the five-category preserve logic downstream.

3. **Validates has_db_only_state predicate** — Export walks all five categories; discard-force refuses without exported JSON; auto-sweep preserves DB-only-state. This is the first real exercise of the quarantine delete-vs-preserve classifier (`has_db_only_state`) in anger. Bugs found here are cheaper than discovering them during a production restore when data is already at risk.

4. **Low risk, high signal** — No platform gates, no concurrency, no IPC, no new system integration. Pure CLI + SQL + JSON export. Reviewers can reason about it without watcher supervision, overflow recovery, or online handshake machinery in flight.

5. **Truthful stopping point** — Stops before watcher overflow (`6.7a`), live ignore reload (`6.8`), and broader watcher choreography. None of those can ship until dedup and quarantine are proven; this slice makes that boundary explicit.

## Reviewer-Friendly Story

The slice is: _Operator gets to inspect, export, and recover quarantined pages without data loss. Auto-sweep respects the five DB-only-state categories so recovery-worthy pages never disappear on TTL. Dedup failure path cleans up its own state so no phantom entries block reconciliation._

## GitHub Issues

None directly cited in the spec, but this enables:
- Safe operator recovery from partial restore failures
- Validation of the quarantine preserve logic that protects recovery-linked data
- Honest dedup failure semantics instead of leaving orphaned entries

## Architecture Alignment

- **Fry's lane** (implementation): dedup cleanup + quarantine export/discard/restore + auto-sweep TTL
- **Professor's lane** (review): five-category delete-vs-quarantine logic, operator guidance docs
- **Nibbler's lane** (test): quarantine lifecycle round-trip + failure cleanup edge cases
- **Scruffy's lane** (test): auto-sweep TTL with DB-only-state preservation + missing-page dedup cleanup

This slice lands in exactly the sequence Fry can execute after watcher core without waiting on online handshake or IPC machinery.



# Decision: next slice after watcher core (commit 43d2117): brain_put rename-before-commit seam

**Author:** Leela  
**Date:** 2026-04-25T03:45:00Z  
**Status:** Recommendation — requires fresh Professor + Nibbler pre-gate before implementation

---

## Current state (post-watcher-core)

- **Completed:** Watcher core slice (tasks `6.1–6.4`, `7.1–7.4`, `7.6`) landed as commit 43d2117.
  - Per-collection notify watcher + bounded queue + 1.5s debounce
  - Reconcile-backed flushes + path+hash dedup set with 5s TTL
  - No watcher overflow recovery, health, live ignore reload, dedup failure-removal, or broader choreography
  
- **Explicitly deferred from watcher core:**
  - Overflow recovery (`6.7a`), supervision/health (`6.10–6.11`), live ignore reload (`6.8`), dedup failure cleanup (`7.5`)
  - Broader watcher-mutation choreography, stats expansion, IPC/live routing
  - Wider destructive restore surfaces

- **M1b-i and M1b-ii already landed:** Write-interlock fixes for collection state check ordering (before OCC), ensuring `CollectionRestoringError` wins over version-conflict errors.

---

## Recommended next slice

**Theme:** `brain_put` rename-before-commit seam (Unix-only, single-file)

### Exact task IDs to close

- `12.2` — Filesystem precondition fast/slow path (stat compare, hash on mismatch, self-heal, conflict refusal)
- `12.3` — Mandatory `expected_version` on updates (creates may omit)
- `12.4` — Per-slug async mutex (vault-byte writes only)
- `12.4a–12.4d` — Failure handlers (pre-sentinel, sentinel-creation, pre-rename, rename, post-rename)
- `12.5` — Enforce `CollectionReadOnlyError` on read-only collections
- `17.5k–17.5r` — 13 proof tests:
  - `17.5k` happy path (tempfile → rename → tx commit)
  - `17.5l` stale `expected_version` before any FS mutation
  - `17.5m–17.5r` precondition paths, conflicts, external mutations

### Exact files involved

- `src/core/writer.rs` — Rename-before-commit orchestration (steps 2–12, minus IPC)
- `src/core/file_state.rs` — Precondition comparison helpers
- `src/mcp/server.rs` — `brain_put` handler (collection interlock positioning per M1b-ii fix)
- `src/commands/put.rs` — CLI sentinel lifecycle
- `tests/vault_write.rs` or equivalent — 13 mechanical tests

### Explicitly deferred

- IPC write proxying (`12.6a–12.6g`, socket placement/auth/peer-verify)
- Bulk-rewrite routing refusal (`12.6b`, `migrate-uuids` / `--write-gbrain-id` blocking on live serve)
- Broader integration with live serve (`17.10` online handshake)
- Watcher overflow recovery (`6.7a` — depends on watcher first)
- Background raw_imports GC (`5.4f` — needs serve scheduler)
- UUID write-back and `migrate-uuids` (`5a.5–5a.5a`)
- Full `brain_put` destructive-edge integration and crash recovery (Group 11 follow-up)

---

## Why this is the best next gate

1. **Unblocks agent workflows.** MCP `brain_put` and CLI `gbrain put` are the primary vault-byte mutation surfaces. This slice enables safe single-file writes without IPC complexity; IPC becomes a serve-integration follow-up, not a hard blocker.

2. **Reuses M1b-ii interlock fix immediately.** The collection state check–before–OCC ordering fix that just landed pairs directly with this slice's precondition enforcement. Together they form a coherent "write-gate + precondition + tempfile + dedup + rename + commit" narrative reviewers can assess holistically.

3. **Minimal scope, maximum confidence.** This is Unix-only and constrained to steps 2–12 (minus IPC). The logic is fully specified in spec and agent-writes design. Proof tests are mechanical: stat fields, version conflicts, dedup, concurrent foreign rename, external delete/create. No new state machine, no negotiation protocol, no service integration.

4. **Honest deferred boundary.** Stops before IPC because serve integration (socket bind, peer auth, proxy) is Group 11 scope. Stops before integration tests with live serve because they require serve/RCRT handshake. Stops before watcher overflow because overflow handling depends on watcher existing first. Does not leave half-built surfaces.

5. **Natural precedent for watcher overflow.** Once `brain_put` rename-before-commit lands solid, the overflow handler becomes straightforward: set `needs_full_sync=1` on bounded-channel overflow, RCRT runs full reconcile. This slice completes the write-path foundation for that next work.

6. **Addresses linked GitHub issues (verify in repo).** Likely addresses any open tickets for "agent write support," "MCP write safety," or "`brain_put` production readiness."

---

## Implementation owner & reviewers

**Owner:** Fry  
**Reviewers:**
- **Professor:** Precondition fast/slow logic, OCC ordering, CAS mandatory contract, interlock sequencing
- **Nibbler:** Dedup consistency, sentinel lifecycle, concurrent-rename edge case, adversarial stat-field scenarios

---

## Pre-gate checklist

Both reviewers must confirm:
- [ ] IPC, bulk-rewrite refusal, watcher overflow remain deferred
- [ ] `12.2–12.5` scope is crisp and includes NO serve integration, handshake work, or Group 11 materials
- [ ] M1b-ii interlock ordering fix is in tree and pair-able with precondition sequencing
- [ ] Proof tests (`17.5k–17.5r`) are mechanical and do not require watcher or serve fixtures


# Note Alignment — 9.8 Restore Ordering Overclaim

**Date:** 2026-04-25  
**Author:** Leela  
**Scope:** Wording-only; no production code changed.

## Decision

Corrected the closure note in `openspec/changes/vault-sync-engine/tasks.md` line 180, item (4).

**Old (overclaim):**
> restore commits DB updates first in transaction then writes temp file + rename, rolling back DB on any failure.

**New (truthful):**
> restore stages DB changes in an open transaction, writes the temp file, renames it into place, then commits the transaction; on any failure before the rename the transaction rolls back and the temp file is unlinked; on any post-rename failure the target is best-effort unlinked so the DB rollback leaves no residue on disk.

## Rationale

`src/core/quarantine.rs` (lines 424–508) shows the actual ordering:
1. `unchecked_transaction()` opens the tx
2. DB mutations are staged (DELETE file_state, UPDATE pages, INSERT embedding_jobs, DELETE quarantine_exports) — **not yet committed**
3. Temp file created, written, synced, renamed into place
4. Post-rename: fsync parent, stat, hash, upsert_file_state — each failure does `best-effort unlinkat(target)`
5. `tx.commit()` — DB commit is the **last** step

The prior note implied DB commit preceded the file write, which is the reverse of the actual order.

## Scope of check

- `now.md` — no matching overclaim found; the existing bullet already correctly states "post-rename failures now best-effort unlink the target file so no residue is left on disk."
- No task IDs widened or narrowed.
- No production code changed.


# Leela — Post-Quarantine-Batch Plan

**Date:** 2026-04-25  
**Context:** Quarantine truth repair (Bender) is the last committed stop-point. `quarantine restore` has been backed out; `9.8` (restore arm only) and `17.5j` are reopened. Watcher core is landed (`6.1–6.4`, `7.1–7.6`). No next vault-sync slice is active.

---

## Truthful stop-point assessment

The landed seam is clean on these surfaces:
- `quarantine list|export|discard`, TTL sweep, info count, dedup cleanup  
- Watcher runtime: one watcher per collection, bounded queue, 1.5s debounce, reconcile-backed flush, self-write dedup with TTL  
- All collection CLI and MCP surfaces through batch `17.5aa5`

The open seam requiring the narrowest safe next step:
- **`9.8` (restore arm)** — refused at CLI with a deferred-surface error; needs crash-durable post-unlink cleanup and a no-replace install path before it can be re-opened  
- **`17.5j`** — quarantine restore integration test; remains open until the restore seam lands

---

## Recommended next slice: Quarantine Restore Narrow Fix

**Scope (narrow — do not widen):**
1. Crash-durable post-unlink cleanup: parent `fsync` after every `unlink` in the restore unlink path (the specific Nibbler blocker)
2. No-replace install semantics: verify the install target is absent at `renameat` time — not just at pre-check — to prevent concurrent-creation clobber (the second Nibbler blocker)
3. `17.5j`: integration test that quarantine restore re-ingests the page and reactivates the `file_state` row

**Explicit out of scope for this slice:**
- Watcher mutation handlers (`6.5`, `6.6`, `6.7`, `6.7a`, `6.8–6.11`)
- Embedding queue (`8.*`)
- IPC socket and CLI write routing (`11.9`, `12.6*`)
- UUID write-back (`5a.5`, `5a.5a`)
- Stats/init/legacy-removal (`14.*`, `10.*`, `15.*`)
- Live/background workers, remap online/offline tests (`17.5qq*`, `17.5pp`)

**Owner:** Fry  
**Reviewers:** Nibbler (primary — named the specific blockers), Professor (pre-gate on crash-durability contract before body starts)  
**Test:** Scruffy (`17.5j` proof map)  
**Gate required:** Professor must sign off on the exact crash-durability contract (which fsyncs are mandatory, which are best-effort, and what the observable invariant is at recovery time) BEFORE Fry writes the body.

---

## Post-batch coverage and docs: parallel, not blocking

Coverage backfill and docs do NOT block the quarantine restore slice. They run in parallel. Neither should gate on the other.

### Coverage backfill (Scruffy)
These tests cover already-landed spec items with no new implementation required:

| Test | Covers |
|------|--------|
| `17.4` | `.gbrainignore` atomic parse — three-way semantics |
| `17.5rr` | Schema-consistency: DB-only-state pages survive hard-delete path |
| `17.5ss` | Bare-slug resolution in single vs. multi-collection brain |
| `17.5tt` | `WriteCreate` resolves to write-target or `AmbiguityError` |
| `17.5uu` | `WriteUpdate` requires exactly one owner |
| `17.5vv` | `WriteAdmin` rejects bare-slug form |
| `17.5vv2–vv6` | Collection name `::`-reject, external address resolution, interlocks |
| `17.17c` | `raw_imports_active_singular` named invariant |
| `17.17d` | `quarantine_db_state_predicate_complete` named invariant |

Owner: **Scruffy** (test harness specialist)  
Reviewer: **Professor** (design correctness on invariant proofs), **Nibbler** (adversarial edge cases on `17.17d`)

### Docs (Section 16)
Owner: any available agent (Fry post-slice, or a dedicated doc pass)  
These are not gated by implementation and can land at any point after the quarantine batch:
- `16.1` README: remove `gbrain import`, document `gbrain collection add`  
- `16.2` getting-started: vault + collections workflow  
- `16.3` spec.md: v5 schema + live sync  
- `16.4` AGENTS.md + skills: remove `gbrain import` / `import_dir` references  
- `16.5` CLAUDE.md: architecture section update  
- `16.7` env var documentation  
- `16.8` five DB-only-state categories + quarantine resolution flow  

**Routing:** `16.1–16.8` should land together as a single doc-cleanup batch, coordinated with or after the `15.*` legacy-ingest removal (which has its own "SHALL NOT merge until §16 doc updates are complete" gate).

---

## Do not open yet

The following surfaces should remain deferred until the quarantine seam is fully closed (restore landed + `17.5j` passing):

- **Watcher mutation handlers** (`6.5–6.11`): the watcher runtime is live but has no mutation handlers. Opening these before quarantine restore is closed creates a partial restore/watcher interplay that hasn't been reviewed.
- **IPC / CLI write routing** (`11.9`, `12.6*`): large security surface, needs Nibbler pre-gate, not the right time.
- **Embedding queue** (`8.*`): serve-side worker infrastructure, orthogonal to quarantine close-out.
- **UUID write-back** (`5a.5*`): rename-before-commit discipline, IPC dependency, defer.
- **Section 10 init cleanup** (`10.*`): can land as part of the legacy-ingest removal batch (`15.*`).

---

## Sequencing summary

```
NOW (parallel):
  └── Fry: quarantine restore narrow fix (9.8 restore + 17.5j)
       └── GATE: Professor pre-gate on crash-durability contract (required before body)
       └── REVIEW: Nibbler adversarial (primary)
  └── Scruffy: coverage backfill (17.4, 17.5rr, 17.5ss–vv6, 17.17c, 17.17d)
       └── REVIEW: Professor (design), Nibbler (adversarial edge cases on 17.17d)
  └── Doc agent: section 16 doc cleanup batch (16.1–16.8)

AFTER quarantine restore + coverage close:
  └── Pick: watcher mutation handlers (6.5–6.7a) OR embedding queue (8.*) — not both
  └── DO NOT pick: IPC, UUID write-back, remap online tests
```

---

## Why this sequencing

1. **The quarantine seam is half-open.** Bender's truth repair was correct to back out restore, but leaving `9.8` open indefinitely means every future slice carries a known gap in the lifecycle. Closing it narrow and fast is lower risk than widening to watcher handlers with an open quarantine promise.

2. **Coverage and docs are cheap, independent, and overdue.** `17.17c` and `17.17d` are named invariant tests that cover raw_imports atomicity and quarantine predicate completeness — two data-loss surfaces that are already live in the tree. These should not wait on any implementation slice.

3. **Parallel is correct here.** The restore slice and coverage/docs have zero shared code paths. Serializing them creates dead time without safety benefit.


# Decision: Post-M1a Next Slice — Batch M1b (split into M1b-i and M1b-ii)

**Author:** Leela  
**Date:** 2026-04-24  
**Context:** M1a closed the writer-side sentinel crash core (`12.1a`, `12.4aa–d`, `17.5t/u/u2/v`). The full `12.1` write-through sequence, precondition/CAS, mutex, IPC, dedup, and watcher remain explicitly deferred.

---

## Recommended Batch Name

**M1b**, split into two sub-slices:

- **M1b-i — Write-interlock test closure** (test-only, no new code)
- **M1b-ii — Precondition + CAS hardening** (new implementation + tests)

---

## M1b-i: Write-interlock test closure

### Exact task IDs to include

| ID | Description |
|----|-------------|
| `17.5s2` | Write-interlock refuses ALL mutating ops during `state='restoring'` OR `needs_full_sync=1` |
| `17.5s3` | Slug-less `brain_gap` succeeds during `restoring` (Read carve-out) |
| `17.5s4` | Slug-bound `brain_gap` is refused during `restoring` |
| `17.5s5` | `brain_link`/`brain_check`/`brain_raw` refused during `restoring` with `CollectionRestoringError` |

### Why this is safe and valuable now

Task `11.8` (write interlock) was completed in an earlier batch. These four tests exercise already-landed code. They require zero new implementation, carry zero regression risk, and close a visible gap between the code truth and test coverage. This is the fastest possible next gate.

### Caveats

- No new code. If a test fails, it reveals a pre-existing bug in `11.8`; do not extend the slice to fix it — log a separate repair.

---

## M1b-ii: Precondition + CAS hardening

### Exact task IDs to include

| ID | Description |
|----|-------------|
| `12.2` | `check_fs_precondition`: fast path (all four stat fields match); slow path (hash on mismatch); hash-match self-heals stat fields; hash-mismatch → `ConflictError`; `ExternalDelete`/`ExternalCreate`/`FreshCreate` cases defined |
| `12.3` | Enforce mandatory `expected_version` for updates; only pure creates may omit |
| `12.4a` | Pre-sentinel failure path: if precondition or CAS check fails before sentinel creation, no vault mutation, no DB mutation, return error |
| `17.5l` | Stale `expected_version` rejected before any FS mutation |
| `17.5m` | Fast path: all four stat fields match → proceed without re-hash |
| `17.5n` | Slow path: stat mismatch + hash match → self-heal stat fields, proceed |
| `17.5o` | Slow path: stat mismatch + hash mismatch → `ConflictError` |
| `17.5p` | External rewrite preserving `(mtime,size,inode)` but changing `ctime` → caught by slow path |
| `17.5q` | External delete → `ConflictError` |
| `17.5r` | External create → `ConflictError` |
| `17.5s` | Fresh create succeeds when target absent and no `file_state` row |

### Why this is the best next slice

M1a proved the crash core: sentinel durability, post-rename sentinel retention, and startup recovery consumption. The remaining full `12.1` sequence has three distinct concerns: (1) **precondition/CAS** (steps 1–3: `expected_version`, `walk_to_parent`, `check_fs_precondition`), (2) **dedup** (step 8), and (3) **live routing + IPC** (`12.6a–g`). None of (2) or (3) are pre-requisites for (1). Implementing precondition/CAS next is therefore:

- **Coherent**: closes the safety gap between the write gate (M1a crash core) and the actual "is this file still what I expect?" verification.
- **Independent**: no dependency on dedup set (`7.x`), watcher (`6.x`), IPC (`11.9`, `12.6*`), or embedding queue (`8.x`).
- **Reviewable**: a single new function (`check_fs_precondition`), a mandatory-version gate, one pre-sentinel failure path, and eight tests. No async, no IPC, no network.
- **Gatable**: passes are definitive; each test covers a single conditional branch with no shared state.

### Exact task IDs to defer (from section 12 and related)

| ID | Reason |
|----|--------|
| `12.1` (full) | Depends on dedup (`7.x`) + IPC routing (`12.6*`); precondition/CAS is a prior dependency |
| `12.4` (full mutex/CAS) | Per-slug async mutex requires within-process concurrency design; separate proof seam |
| `12.5` | `CollectionReadOnlyError` when `writable=0` — already enforced by K1 (`17.5qq11` / `ensure_collection_vault_write_allowed`); this checkbox needs audit to confirm it is already proved, not new work |
| `12.6a–g` | All IPC routing and socket security; requires Nibbler pre-implementation review |
| `12.7` | Full happy-path + all-failure-modes integration sweep; blocked until dedup + IPC land |
| `17.5k` | Happy path with dedup echo suppression; requires `7.x` dedup set |
| `17.5bb–ee` | Dedup/embedding/watcher tests; require `7.x`, `8.x`, `6.x` respectively |
| `17.5w`, `17.5x` | `needs_full_sync` trigger + overflow recovery worker; require watcher (`6.x`) |
| `7.x` | Dedup set; natural batch after M1b-ii |
| `8.x` | Embedding queue and worker; natural batch after dedup |
| `17.5pp`, `17.5qq*` | Online restore handshake + IPC tests; depend on `11.9`, `12.6*` |
| `17.5ii4`, `17.5ii5`, `17.5ii9` | Remap Phase 4 bijection + bulk UUID / IPC live constraints |

---

## Ordering and gate

```
M1b-i  (test-only, fast) → can land independently at any time
M1b-ii (precondition/CAS) → may proceed immediately after M1b-i review passes
```

M1b-i has no implementation risk and should not block M1b-ii if M1b-i is slow to review. They share no changed code. They CAN be reviewed in parallel.

---

## Truthfulness caveats required

1. **M1b-ii does NOT close `12.1` (full).** After M1b-ii lands, the full rename-before-commit sequence still has dedup (step 8), live routing (12.6), and full-coverage integration tests (12.7) outstanding. Do not describe M1b-ii as "write-through complete."
2. **`12.4` (per-slug async mutex) remains open.** M1b-ii does not add within-process write serialization; concurrent writes from the same process remain race-prone until `12.4` lands.
3. **Unix-only scope.** `12.2`/`12.3`/`12.4a` are part of the fd-relative write path. Windows platform gate (`17.16`) is still deferred.
4. **`12.5` is not new work in this batch.** K1 (`17.5qq11`) already enforces `CollectionReadOnlyError` for vault-byte writes. Before closing `12.5`, confirm the audit passes — do not re-implement or overclaim coverage for non-vault-byte mutators.
5. **`17.5k` (happy-path with dedup) cannot land here.** The dedup insert at step 8 is still absent. Do not claim the full happy-path test is passing until `7.x` lands.

---

## Summary judgment

M1a proved sentinel durability under crash. M1b-i closes the interlock test gap cheaply. M1b-ii extends the write path with precondition/CAS — the only seam that can safely proceed without IPC, dedup, or watcher infrastructure. Together they form a narrow, honest, independently testable advance toward the full `12.1` contract without overclaiming it.


# Post-M1b Next Slice Decision

**Author:** Leela  
**Date:** 2026-04-24  
**Context:** Batches M1b-i and M1b-ii are closed. This memo recommends the next coherent implementation slice and documents the gateable sub-slice split.

---

## Where We Stand After M1b

The write-through seam (`12.x`) now has the following closed:

| Task | Surface | Closed by |
|------|---------|-----------|
| `12.1a` | Sentinel crash core | M1a |
| `12.2` | `check_fs_precondition` (self-heal stat, hash-mismatch ConflictError) | M1b-ii |
| `12.3` | Mandatory `expected_version` for updates | M1b-ii |
| `12.4a` | Pre-sentinel failure aborts with no vault/DB mutation | M1b-ii |
| `12.4aa/b/c/d` | Full failure-path proofs | M1a/M1b |
| `17.5l-s` | CAS + precondition proofs | M1b-ii |
| `17.5s2-s5` | Write-interlock matrix (restoring + needs_full_sync) | M1b-i |

What is **not** closed:
- `12.4` — per-slug async mutex (within-process write serialization)
- `12.5` — `CollectionReadOnlyError` on `writable=0`
- `12.6*` — IPC socket, proxy routing, peer auth _(major new surface)_
- `12.7` — comprehensive write-through integration tests _(depends on 12.6)_
- `7.x` — self-write dedup set _(watcher-dependent)_
- `17.5k` — full happy-path test including dedup echo suppression _(depends on 7.x)_

The mechanical write chain (steps 1–13 of the spec's rename-before-commit sequence) is **almost provable now** — the only missing piece before an end-to-end mechanical proof is the per-slug mutex (`12.4`). IPC/routing and dedup are separate surfaces that do not block the mechanical proof.

---

## Recommendation: Split M2 into M2a and M2b

### M2a — Platform Safety + Read-Only Gate

**Purpose:** Close two independent safety items with zero new implementation risk.

**Tasks included:**
- `2.4a2` — `#[cfg(windows)]` handlers return `UnsupportedPlatformError` for `gbrain serve`, `gbrain put`, and all `gbrain collection` vault-sync subcommands. Offline commands may still run.
- `12.5` — Enforce `CollectionReadOnlyError` when `collections.writable=0` on any vault-byte write path.
- `17.16` — Integration: Windows platform gate returns `UnsupportedPlatformError`.
- `17.16a` — Integration: non-writable collection refuses every mutating command with `CollectionReadOnlyError`.

**Tasks deferred from M2a:** Everything else.

**Why this is coherent:**  
Both tasks are defensive closures. `2.4a2` prevents silent Windows CI success on paths whose safety depends on Unix `O_NOFOLLOW`/`renameat` semantics. `12.5` closes a guard the DB already supports (`writable=0` column exists and is set by the capability probe in `9.2b`) but the write path does not yet enforce. Neither task touches the mutex, IPC, dedup, or any new state machine. This is a single-reviewer batch — Professor or Nibbler alone can gate it.

**Caveats for M2a:**
- `2.4a2` must NOT claim `2.4c` (symlink-skipping walk integration) is done — that's a reconciler change, separate seam.
- `12.5` covers vault-byte writes only (`brain_put` / `gbrain put`). DB-only mutators (`brain_link`, `brain_check`, etc.) are not subject to `CollectionReadOnly`; that ruling is already canonical in the K1 repair note on `9.2b`.

---

### M2b — Per-Slug Mutex + Mechanical Write-Through Proof

**Purpose:** Land the last missing implementation primitive for within-process write safety, then prove the complete mechanical write chain end-to-end.

**Tasks included:**
- `12.4` — Per-slug async mutex: `Arc<Mutex<()>>` (or `tokio::sync::Mutex`) keyed by `collection_id::slug`, acquired before the write begins and released after full write-through completes (or fails). Explicitly NOT a substitute for DB CAS.
- `17.5k` (narrow split) — `brain_put` mechanical happy path: `expected_version` check → precondition → sentinel → tempfile → rename → parent-fsync → single-tx commit succeeds on the correct-version path. **Closure note must explicitly state:** the "dedup echo suppressed" claim is deferred to post-`7.x`; this task proves only the mechanical sequence.
- `17.17e` — Named invariant: every mutating entry point checks `expected_version` BEFORE any tempfile, dedup insert, FS mutation, or DB mutation. Pairs naturally with `12.4` proof ordering.

**Tasks deferred from M2b:** Full `17.5k` echo suppression (post-7.x), `12.6*`, `12.7`, `7.x`, watcher, IPC.

**Why this is coherent:**  
`12.4` is the single remaining implementation unit before the mechanical write path is complete. Without it, two concurrent MCP `brain_put` calls to the same slug can both pass CAS validation and then race on the filesystem. With it, within-process serialization is provable. `17.5k` (narrow) and `17.17e` are the direct proofs for this seam. The mutex is self-contained: it does not require IPC, the dedup set, or the watcher. The three tasks form one reviewable unit.

**Caveats for M2b:**
- `12.4` closure note must state: "within-process only, not a substitute for DB CAS; per-slug granularity means different-slug concurrent writes are never blocked."
- `17.5k` closure note must not claim "dedup echo suppressed" — that language requires `7.x` (watcher-side dedup consultation). The annotation must be explicit: "dedup echo claim deferred to post-7.x batch."
- M2b does NOT claim `brain_put` is production-ready for live serve. Live serve safety requires echo suppression (7.x) and IPC routing (12.6*), both explicitly deferred.
- `17.17e` is a proof-only task; no production code gate appears under a "tests-only" claim without explicitly naming where the production enforcement lives (follow M1b-repair discipline).

---

## What Is Explicitly Deferred After M2

These items remain open after M2a + M2b and must not be claimed in either batch:

| Deferred item | Dependency / reason |
|---------------|---------------------|
| Full `17.5k` echo suppression | Requires `7.x` dedup set + watcher |
| `12.6*` IPC socket, proxy, peer auth | Major new surface; Nibbler adversarial pre-review required BEFORE Fry starts |
| `12.7` comprehensive write-through tests | Depends on `12.6*` being in place |
| `7.x` self-write dedup set | Watcher-dependent (`7.3` requires watcher to consult set) |
| `17.5k` dedup echo claim | Watcher-dependent |
| `17.5w/x` needs_full_sync healing + overflow gate | Watcher-dependent |
| `17.5y-aa*` `.gbrainignore` watcher tests | Watcher/serve-dependent |
| `5a.5/5a.5a` opt-in UUID write-back | Can be scoped once M2b mutex is in place; separate batch |
| `2.4c` symlink-skipping walk integration | Reconciler seam, separate surface |
| `4.6` periodic full-hash audit | Background task, needs serve/Group 8 |
| `9.6`, `9.8`, `9.9`, `9.10` remaining collection commands | Lower priority; no blocking proof obligation |
| Groups 13–17 (collection-aware slug parsing, stats, legacy removal, docs, named invariants) | Post-landing work; wait for an appropriate branch stop point |
| `17.17a-d` remaining named invariant tests | Can be parceled into the batch that closes the surface they audit |
| `18.x` daemon-install / openclaw stubs | Follow-up OpenSpec work |

---

## Sequencing

```
M2a  (platform gate + read-only)   →   M2b  (mutex + mechanical proof)
                                    ↓
                             IPC pre-review (Nibbler)
                                    ↓
                           M3: IPC + routing (12.6*)
                                    ↓
                        M4: Dedup + full 17.5k (post-7.x)
```

M2a and M2b are independent of each other — they can be reviewed in parallel by different reviewers if desired. M2a has zero implementation risk and can land first. M2b requires one implementation unit (`12.4` mutex) and can be gated separately.

Neither M2a nor M2b unblocks IPC (`12.6*`). IPC requires Nibbler adversarial pre-review before any implementation begins — that gate should be opened as a separate pre-review task concurrent with M2 landing.

---

## Reviewer Routing

| Sub-slice | Owner | Reviewers |
|-----------|-------|-----------|
| M2a | Fry | Professor (primary), Nibbler (optional spot-check on 12.5 write-gate ordering) |
| M2b | Fry | Professor (12.4 mutex design + 17.17e invariant), Nibbler (12.4 concurrency adversarial) |
| IPC pre-review | Nibbler | Leela gate-approves before Fry starts 12.6* |


# Decision: Quarantine Third Revision — Vault-Byte Gate + Post-Rename Residue

**Date:** 2026-04-25  
**Author:** Leela (third revision, locked out of Fry and Mom)  
**Artifact:** `src/core/quarantine.rs`

## Problem

Two confirmed blockers from re-review prevented `9.8` closure:

1. **Read-only gate bypass:** `restore_quarantined_page` called `ensure_collection_write_allowed` which only checks `state == Restoring || needs_full_sync`. This bypassed the vault-byte writable gate (`collections.writable=0`). A read-only collection could have pages restored to its filesystem.

2. **Post-rename residue:** After a successful `renameat` (temp → target), any failure in `put_parent_sync`, `stat_file`, `hash_file`, `upsert_file_state`, or `tx.commit` left the file at the final target path on disk while the DB transaction was dropped (page remained quarantined). This created unreferenced bytes on disk.

3. **Overclaiming notes:** `tasks.md` 9.8 and `now.md` claimed "failure leaves no residue" and "all operator-facing default seam blockers closed" — both were false given the above two bugs.

## Decision

**Fix the blockers narrowly; do not widen scope.**

### Production changes (`src/core/quarantine.rs`):

1. Replace `vault_sync::ensure_collection_write_allowed` with `vault_sync::ensure_collection_vault_write_allowed` in `restore_quarantined_page`. This is a 1-line change that adds the `collection.writable` check alongside the existing `state/needs_full_sync` check.

2. Replace the straight `?`-propagation in the post-rename window with explicit error arms that call `fs_safety::unlinkat_parent_fd(&parent_fd, Path::new(target_name))` (best-effort) before returning. Covers `put_parent_sync`, `stat_file`, `hash_file`, `upsert_file_state`, and `tx.commit`.

### Test added:
`tests/quarantine_revision_fixes.rs::blocker_1_third_revision_restore_refuses_read_only_collection` — verifies `CollectionReadOnlyError` is surfaced and no file is written when the collection has `writable=0`. `#[cfg(unix)]`.

Post-rename residue fix is code-only (triggering a post-rename failure in integration requires mocking; structural fix is sufficient here).

### Notes corrected:
- `tasks.md` 9.8 note updated with third-revision repair block naming both fixes.
- `now.md` quarantine line updated to accurately describe the third-revision state.

## Rationale

The smallest truthful repair: two surgical code changes + one test + corrected notes. No scope widening. All previously-good seams preserved. The batch is reviewer-friendly because the diff is minimal and each change maps directly to a named blocker.

## Known host-specific false-fails (unchanged, pre-existing)

`init_rejects_nonexistent_parent_directory` and `open_rejects_nonexistent_parent_dir` fail on this host due to stale residue at `D:\nonexistent\dir` (Windows path exists). These are unrelated to this revision.


# Leela Decision — Vault Sync Batch H Repair

Date: 2026-04-22
Change: `openspec/changes/vault-sync-engine`

## Decision

Bind Batch H's restore/remap full-hash bypass to persisted collection owner identity and fail closed when the owner is absent or mismatched.

## Why

The rejected seam was success-shaped: mode + caller class + any non-empty token was enough to authorize drift capture. For a pre-destruction bypass, that is too open. The narrowest safe repair is to compare the presented identity against authoritative state already attached to the collection lifecycle.

## Implementation shape

- Persist three owner fields on `collections`:
  - `active_lease_session_id`
  - `restore_command_id`
  - `restore_lease_session_id`
- Require exact identity match for:
  - `RemapRoot` / `RemapDriftCapture` with `ActiveLease`
  - `Restore` / `RestoreDriftCapture` with `RestoreCommand` or `RestoreLease`
- Reject before any root open or walk when:
  - caller identity is empty
  - persisted owner identity is missing
  - caller identity does not equal persisted owner identity
- Keep `FreshAttach` separate; it remains attach-command scoped and does not reuse restore/remap owner matching.

## Notes

- `tasks.md` wording was already truthful after the repair and did not need another edit.
- Validation passed with both required command lines after the change.


# Leela — Vault Sync Batch I Repair

Date: 2026-04-22
Change: `vault-sync-engine`

## Decision

Keep Batch I fail-closed: legacy compatibility mutators must honor the same restore/full-sync write gate, and offline restore/remap must stop at Tx-B / pending full-sync so only RCRT can reopen writes.

## Why

The rejected slice still had two silent reopen paths: old ingest/import commands could write during restore, and offline restore/remap called attach completion directly. The narrow truthful repair is to block the legacy writers up front and leave offline flows in `state='restoring'` + `needs_full_sync=1` until serve-owned RCRT takes the lease and completes attach.

## Implementation shape

- Add the shared `ensure_all_collections_write_allowed()` interlock to:
  - `src/commands/ingest.rs`
  - `src/core/migrate.rs::import_dir()` (write mode only; validate-only remains read-only)
- Remove direct offline `complete_attach(...)` calls from:
  - `begin_restore(..., online = false)`
  - `remap_collection(..., online = false)`
- Strengthen `unregister_session()` so it clears `collections.active_lease_session_id` and `collections.restore_lease_session_id` whenever that session is released.
- Keep plain `gbrain collection sync <name>` explicitly deferred in `tasks.md` instead of overstating support.

## Validation

- `cargo test --quiet`
- `GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model`


# Leela — Vault Sync Batch J Repair Decision

**Date:** 2026-04-23  
**Author:** Leela (Lead)  
**Context:** Batch J rejection cycle — Nibbler and Professor both rejected on the same seam: `gbrain collection sync <name> --finalize-pending` emitted exit 0 / `status: ok` for blocked `FinalizeOutcome` variants.

---

## Decision: Fail-Close CLI Truth for `--finalize-pending`

### Problem

`sync()` in `src/commands/collection.rs` unconditionally called `render_success` after `finalize_pending_restore()` returned, regardless of whether the outcome was `Finalized`, `Deferred`, `ManifestIncomplete`, `IntegrityFailed`, `Aborted`, or `NoPendingWork`. This gave automation a false green: exit 0, `"status": "ok"`, even though the collection remained blocked and was never finalized.

### Fix

Match on `FinalizeOutcome` in the `--finalize-pending` branch:

- `Finalized` | `OrphanRecovered` → `render_success` with `"status": "ok"` (unchanged success path)
- All other variants → `bail!("FinalizePendingBlockedError: collection={name} outcome={blocked:?} collection remains blocked and was not finalized")`

This ensures:
1. Non-zero exit for every blocked outcome
2. Error text explicitly states the collection is blocked / not finalized
3. The outcome variant name is included for diagnostics

### Scope Boundary

- **Changed:** `src/commands/collection.rs` — `sync()` finalize_pending branch only
- **Added:** Two CLI truth tests in `tests/collection_cli_truth.rs`: `NoPendingWork` path and `Deferred` path
- **Updated:** `openspec/changes/vault-sync-engine/tasks.md` — repair note appended to task 17.5ll2
- **Not touched:** MCP server, Phase 4 remap, any deferred destructive-path work, existing approved Batch J invariants (bare no-flag sync, short-lived lease discipline, terminal reconcile halts)

### Validation

Both test commands passed clean:
- `cargo test --quiet` — all tests pass
- `GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model` — all tests pass

### Invariant Now Enforced

> `gbrain collection sync <name> --finalize-pending` MUST return non-zero exit and emit text containing "FinalizePendingBlockedError" and "remains blocked"/"not finalized" for any outcome other than `Finalized` or `OrphanRecovered`.


# Decision: vault-sync-engine Batch K Rescope — Reconciled Recommendation

**Author:** Leela  
**Date:** 2026-04-23  
**Session:** vault-sync-engine Batch K — reconciling Professor and Nibbler rejected splits  

---

## Context

Two reviewers both rejected the original combined Batch K boundary but proposed different safer splits:

- **Nibbler's split:** K1 = restore integrity proofs (`1.1b`, `1.1c`, `17.5kk3`, `17.5ll3`, `17.5ll4`, `17.5ll5`, `17.5mm`) first; K2 = collection add + read-only surface + `17.11`.
- **Professor's split:** K1 = collection add scaffolding + read-only truth (`1.1b`, `1.1c`, `9.2`, `9.2b`, `9.3`, `17.5qq10`, `17.5qq11`) first; K2 = offline restore integrity closure (`17.5kk3`, `17.5ll3`, `17.5ll4`, `17.5ll5`, `17.5mm`, `17.11`).

The dispute is entirely about ordering, not about whether both clusters belong to the same pair of batches. The disagreement turns on one question: can the restore integrity proofs run honestly without first landing the shared write gate and fixing the offline `restore_command_id` persistence hole?

---

## Recommendation

**Adopt Professor's split exactly.**

### Batch K1 — Collection Add Scaffolding + Read-Only Truth

**Label:** `vault-sync-engine-batch-k1`

**Include now:**
- `1.1b` — Extend `log_gap()` / `brain_gap` to accept optional slug; populate `knowledge_gaps.page_id`; update `Gap` struct and list/resolve responses; unit tests for slug and slug-less variants
- `1.1c` — Classify `brain_gap` by variant: slug-bound = `WriteUpdate` (subject to `CollectionRestoringError` interlock); slug-less = `Read` (no interlock); unit test covers both during `state='restoring'`
- `9.2` — `gbrain collection add <name> <path>`: validate name (no `::`), open `root_fd` with `O_NOFOLLOW`, persist row, run initial reconcile under short-lived `collection_owners` lease
- `9.2b` — Capability probe: attempt a tempfile write; EACCES/EROFS → `writable=0` + WARN; subsequent writes refuse with `CollectionReadOnlyError` via shared write gate
- `9.3` — `gbrain collection list` diagnostics surface
- `17.5qq10` — `collection add` capability probe sets `writable=0` on EACCES/EROFS and WARNs
- `17.5qq11` — `CollectionReadOnlyError` refuses writes when `writable=0` on every mutating path

### Batch K2 — Offline Restore Integrity Closure

**Defer to next batch:**
- `17.5kk3` — Tx-B failure leaves `pending_root_path` set; generic recovery does NOT clear it
- `17.5ll3` — Successor process with different restore-command identity cannot bypass fresh-heartbeat gate
- `17.5ll4` — `pending_manifest_incomplete_at` retries successfully within window
- `17.5ll5` — `pending_manifest_incomplete_at` escalates to `integrity_failed_at` after TTL
- `17.5mm` — Restore manifest tamper → `IntegrityFailed` + `restore-reset` flow
- `17.11` — Integration: offline restore finalizes via CLI (end-to-end proof)

K2 must also include the offline `begin_restore` code fix (persist `restore_command_id` + wire originator identity before exiting) and the `restore_reset()` scope tightening. These are production code changes, not test-only.

---

## Why Professor's K1 is safer than Nibbler's K1

### 1. The offline restore identity gate has a live implementation hole

`begin_restore` in the offline path (lines 1068–1098 of `vault_sync.rs`) does NOT persist `restore_command_id`. The online path explicitly sets it (lines 1030–1049). This means `finalize_pending_restore()` → `caller_is_originator` is structurally `false` for every offline-originated `RestoreOriginator` call, making the identity gate vacuous for the offline path.

Under Nibbler's K1, task `17.5ll3` ("successor with a different restore-command identity cannot bypass the fresh-heartbeat gate") would be written against a code path where the originator ALSO cannot claim its identity. The test might pass for the wrong reason — it would appear to prove identity gating when it is actually proving that no party can claim originator authority at all. That is false confidence, not integrity proof.

Running those tests in K2, after the offline path is fixed to persist `restore_command_id`, means the proof is honest.

### 2. `restore_reset()` is over-permissive today

`restore_reset()` (lines 1199–1225) unconditionally clears ALL restore state — including `integrity_failed_at`, `pending_manifest_incomplete_at`, `pending_root_path`, and `restore_command_id` — without any caller identity check or state precondition guard. Running `17.5mm` (manifest tamper → `IntegrityFailed` → `restore-reset` required) before this is tightened certifies a recovery flow that the code does not yet bound correctly.

### 3. `writable=0` is not yet a shared write gate

`9.2b` and `17.5qq11` are Professor's non-negotiables for K1 because the restore state machine tests exercise write paths. Proving that writes are blocked during restore state is only honest if those same write paths refuse via the shared `CollectionReadOnlyError` gate. Without `9.2b`/`17.5qq11` landing first, the restore tests prove a partial write-block that does not cover all mutating surfaces.

### 4. `1.1c` write interlock must precede restore tests

Slug-bound `brain_gap` during `state='restoring'` must be classified as `WriteUpdate` and subject to `CollectionRestoringError`. This lands in K1. If restore integrity tests run before this classification is wired, a slug-bound gap write during restore is silently allowed — one of the interlock seams the restore tests are supposed to certify is not yet closed.

### 5. `17.11` requires real `collection add` — both reviewers agree

Both Professor and Nibbler independently moved `17.11` to the same batch as `collection add`. Under Nibbler's split, the K1 restore proofs must use DB fixtures; `17.11` defers to K2 anyway. This means Nibbler's K1 only gains the ability to run `17.5kk3`/`ll3`–`ll5`/`mm` against fixture state before the scaffolding is honest. Professor's split defers those tests to K2 and lets them exercise the real CLI path.

### 6. Nibbler's adversarial focus is preserved, just in the right batch

Nibbler's three core seams (identity theft prevention, manifest tamper, Tx-B failure residue) are all K2 tasks under Professor's split. Nibbler's adversarial pre-gate is still mandatory — it simply targets K2 at the right moment, after the code holes are fixed, rather than K1 while the offline path is still broken.

---

## Non-negotiable implementation constraints for Batch K1

These are Professor's constraints, confirmed:

1. **Fresh-attach command, not `import_dir()` alias.** `gbrain collection add` must be a first-class fresh-attach path — not a wrapper around the legacy ingest pipeline.

2. **Fail before row creation on root/ignore validation.** If the collection name is invalid, if `O_NOFOLLOW` root open fails, if symlink validation fails, or if `.gbrainignore` atomic parse fails — the command must refuse before any `collections` row is written.

3. **Short-lived lease discipline on initial reconcile.** The initial reconcile inside `collection add` must register a session, acquire `collection_owners`, heartbeat through the reconcile, and release on every exit path (success and error). Zero lease residue after abort.

4. **`state='active'` only on reconcile success.** Collection row may exist with intermediate state during reconcile; only a fully completed reconcile flips `state='active'`, `needs_full_sync=0`, and `last_sync_at`.

5. **Read-only by default.** No `gbrain_id` write-back, no watcher start, no serve widening. `--write-gbrain-id` deferred to later batch (blocked on 5a.5).

6. **Probe downgrade only for true permission signals.** `EACCES`/`EROFS`-class errors → `writable=0` with WARN. All other probe failures → command aborts, no row created.

7. **`CollectionReadOnlyError` via shared write gate.** The refusal must be enforced at the shared low-level mutator entrypoint, not only inside `collection add`. CLI and MCP mutating paths must all hit the same gate.

8. **`collection list` output must match spec.** If spec/task column list differs from what is implemented, repair the artifact before claiming the surface done.

---

## Required proof tasks for Batch K1

- Direct CLI/integration proof that `collection add` acquires and releases short-lived `collection_owners` lease around initial reconcile
- Proof that `needs_full_sync=0` is only set after successful reconcile (not on partial add failure)
- Probe downgrade test (`writable=0`) on EACCES/EROFS root
- Shared write-refusal proof: `CollectionReadOnlyError` from a mutating path after `writable=0` is persisted
- `brain_gap` slug-bound vs slug-less restoring-state proof (`1.1c`)
- No partial-row residue after add fails post-insert but pre-reconcile
- Probe artifact cleanup (`.gbrain-probe-*` tempfiles) on abort

---

## Gating for Batch K1

| Gate | Required | Scope |
|------|----------|-------|
| **Professor pre-implementation gate** | YES — mandatory | Confirm `collection add` lease acquisition design; confirm `CollectionReadOnlyError` shared write gate contract before Fry starts the body |
| **Nibbler adversarial pre-gate** | YES — mandatory | Add-time lease ownership; probe artifact residue; `CollectionReadOnlyError` shared gate coverage across all mutators |

---

## Entry criteria for Batch K2

K2 must not start until:

1. K1 is landed and gated.
2. The offline `begin_restore` code is fixed to persist `restore_command_id` (generate, write, and thread through to `finalize_pending_restore` as `RestoreOriginator`).
3. `restore_reset()` is tightened: state precondition guard added so unconditional clear is replaced with explicit operator-confirmed reset only from `integrity_failed` / operator-initiated paths.
4. The implementation holes above are in the tree before the K2 proofs are written.

K2 gating: Nibbler adversarial pre-gate on identity theft prevention, manifest tamper, and Tx-B failure residue. Professor reconfirms before K2 implementation starts.

---

## Confidence

High. The Professor/Nibbler disagreement is purely about which cluster comes first. The deciding factor — the live offline `restore_command_id` persistence hole — is verifiable in the current tree. Restore integrity tests written against a broken identity gate produce false confidence, not proof. Professor's sequence (add + write gate first, then honest restore tests) is the only ordering where the proofs prove what they claim.


# K2 Truth Gap Repair — Leela

**Date:** 2026-04-23  
**Batch:** K2 (`vault-sync-engine`)  
**Author:** Leela

---

## Decision

`17.11` ("Integration: offline restore finalizes via CLI") is **genuinely complete** in K2 and has been marked `[x]` in `openspec/changes/vault-sync-engine/tasks.md`.

## Reasoning

Scruffy kept `17.11` deferred with the reasoning "attach completion still depends on serve/RCRT rather than a pure end-to-end CLI completion path to an active collection." This was accurate at Batch I when `complete_attach` did not exist in the CLI finalize path.

By K2, the implementation in `src/core/vault_sync.rs` chains:

```
finalize_pending_restore_via_cli
  → finalize_pending_restore    (Tx-B: verifies manifest, clears pending_root_path)
  → complete_attach             (full_hash_reconcile_authorized + state='active' + needs_full_sync=0)
```

`complete_attach` runs entirely within the CLI process under a short-lived `collection_owners` lease. There is **no serve/RCRT dependency** for the offline `--finalize-pending` path. The RCRT path (`run_rcrt_pass`) also calls `complete_attach` for serve-initiated recovery, but the CLI path is fully self-contained.

The end-to-end proof is `offline_restore_can_complete_via_explicit_cli_finalize_path` in `tests/collection_cli_truth.rs` (`#[cfg(unix)]`). It:
1. Creates a fixture collection with a page + raw_import
2. Calls real binary: `gbrain collection restore work <target>`
3. Confirms `blocked_state=pending_attach`, `suggested_command=gbrain collection sync work --finalize-pending`
4. Calls real binary: `gbrain collection sync work --finalize-pending`
5. Confirms `finalize_pending=Attached` (exit 0) and DB: `state=active`, `root_path=<target>`, `needs_full_sync=0`, no pending fields, file bytes present on disk

This IS the "real end-to-end CLI proof reaching an honestly active collection without widening plain sync" that the original deferred note was waiting for.

## Files Changed

- `openspec/changes/vault-sync-engine/tasks.md`: `17.11` marked `[x]`; deferred proof boundary note replaced with K2 completion note

## K2 Task Completion Summary

| Task ID | Status | Notes |
|---------|--------|-------|
| 17.5kk3 | ✅ Complete | Proved by Scruffy |
| 17.5ll3  | ✅ Complete | Proved by Scruffy |
| 17.5ll4  | ✅ Complete | Proved by Scruffy |
| 17.5ll5  | ✅ Complete | Proved by Scruffy |
| 17.5mm   | ✅ Complete | Proved by Scruffy |
| 17.11    | ✅ Complete | Proved — deferred note superseded; CLI path independent of serve/RCRT |

## Validation

- `cargo test --quiet` — all tests pass ✅
- `GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model` — all tests pass ✅

## Rule

Deferred notes contain a premise. When the implementation advances in a way that invalidates the premise, the deferred note becomes a false claim and must be superseded. The proof reviewer's job at gate time is to re-check every deferred note against the current code, not carry forward a conclusion from an earlier implementation state.


# Decision: vault-sync-engine Batch L Rescope — Reconciliation of Professor / Nibbler Gate Outcomes

**Author:** Leela  
**Date:** 2026-04-23  
**Status:** Recommendation — requires Professor re-confirmation + Nibbler adversarial pre-gate before implementation  
**Supersedes:** `leela-vault-sync-next-batch-4.md` (prior Batch L proposal)

---

## Background

K2 closed the offline restore integrity matrix. The previously proposed Batch L (`11.1 + 11.4 + 17.5ll + 17.13`) was:

- **Professor:** APPROVED as a startup-recovery closure slice, with a conditional split trigger: if implementation requires a new online-handshake change, broader supervisor rewrites, or a second finalize path, split into `L1 = 11.1 + 11.4` and `L2 = 17.5ll + 17.13`.
- **Nibbler:** REJECTED as mixing two distinct recovery authorities. Proposed split: `L1 = 17.5ll + 17.13 + minimal 11.1`; `L2 = 11.4 + 17.12`.

These split lines do not align — Professor's conditional split groups by infrastructure vs. proof; Nibbler's split groups by recovery authority. The tiebreaker is recovery-authority coherence, not infrastructure vs. proof coherence.

---

## Recommendation

### Batch Label

**`vault-sync-engine-batch-l1`** — Restore-Orphan Startup Recovery

---

### Include Now (Batch L1)

| ID | Description | Scope boundary |
|----|-------------|----------------|
| `11.1` (partial) | Initialize process-global RCRT/supervisor-handle registry and dedup set. **Explicitly excludes** sentinel directory initialization — that portion belongs to L2. | Only the registries that RCRT and supervisor spawn depend on at startup. |
| `17.5ll` | Restore orphan recovery: originator dead, RCRT finalizes as `StartupRecovery`. | Unit/integration: one recovery authority, one failure mode. |
| `17.13` | Integration: crash mid-restore between rename and Tx-B — RCRT finalizes on next serve start. | The integration proof that closes the crash-to-recovery story for restore orphans. |

---

### Defer to Batch L2 (Sentinel Startup Recovery)

| ID | Description | Reason deferred |
|----|-------------|-----------------|
| `11.1` (sentinel portion) | Recovery sentinel directory initialization. | Belongs with the sentinel reader it serves, not the RCRT path. |
| `11.4` | Recover from `brain_put` recovery sentinels: set `needs_full_sync=1`, unlink after reconcile. | Different recovery authority, different parser correctness requirements, different cleanup rules. Needs its own proof boundary. |
| `17.12` | Integration: crash mid-write — startup recovery ingests disk bytes; DB converges. | This is the honest companion proof for `11.4`. Landing `11.4` without `17.12` would leave sentinel recovery unproven. |
| `2.4a2` | Windows platform gate (`UnsupportedPlatformError` on vault-sync commands). | Orthogonal to both recovery slices. Keep out of both until a standalone batch is appropriate. |

---

## Why This Split Is Safer Than the Original L Boundary

### 1. One recovery authority per batch

RCRT orphan finalize and sentinel dirty-signal recovery are different state machines with different failure modes:

- **RCRT orphan recovery** (L1): "Was the originator's process actually dead? Is the heartbeat stale past the 15s threshold? May I finalize using the persisted `restore_command_id` and route through `StartupRecovery`?" The entire proof pivots on identity (who owned the restore) and liveness (is the originator actually gone).

- **Sentinel recovery** (L2): "Is there a malformed, partially written, or abandoned sentinel file in the sentinel directory? Did the `brain_put` rename-before-commit sequence crash? What does correct sentinel parsing require to avoid a false-clean signal?" The entire proof pivots on file parser correctness and the monotonic non-destructive rule (sentinel absence must not clear dirtiness).

Bundling both means a test failure tells you "one of the two recovery authorities is wrong" — not which one. The proof record is polluted. Each authority deserves its own pass/fail surface.

### 2. `17.12` is `11.4`'s companion proof — not `11.1`'s or `17.13`'s

The Batch F learning is directly applicable: landing write infrastructure without its proof is a data-loss surface. `11.4` without `17.12` would leave sentinel recovery in the same state `full_hash_reconcile` was before Batch G: code in tree, proof absent, false confidence. The safer pattern is always: proof and infrastructure land together when the subject is a recovery/mutation surface.

### 3. Professor's conditional split trigger was met

Professor's gate document stated: "Split immediately if implementation needs a second finalize/attach code path distinct from the current helper seams." `11.4` (sentinel unlink/reconcile trigger) does not go through `finalize_pending_restore`; it is a distinct dirty-signal surface. It meets the trigger for split even under Professor's own standard.

### 4. `11.1` partial is cleaner than it sounds

`11.1` initializes three things: supervisor-handle registry, dedup set, and sentinel directory. The first two are prerequisites for RCRT (`11.5` is already checked, meaning `11.1`'s RCRT/supervisor portion may be partially wired already). The sentinel directory portion is a prerequisite only for `11.4`. Keeping L1's `11.1` scope to RCRT/supervisor registries is a clean cut: L1's startup order remains `registry init (RCRT/supervisor) → RCRT pass → supervisor spawn`, and L2 adds `sentinel directory init → sentinel sweep` before RCRT in its own batch.

---

## Non-Negotiable Implementation Constraints for Batch L1

1. **Startup order: `registry init → RCRT → supervisor spawn`.** No supervisor/watcher handle spawns for any collection until the startup RCRT pass has had first refusal on that collection. This is enforceable, auditable, and must appear as a sequential call site, not implied by initialization order.

2. **Strict 15-second stale-command threshold as a single named constant.** One shared helper/constant used by every startup recovery decision. No PID probes, no `start_time` reads, no new identity surface. Decision = persisted `restore_command_id` + heartbeat age only.

3. **Canonical `StartupRecovery` finalize path — no inline SQL, no shortcuts.** Route through `finalize_pending_restore(..., FinalizeCaller::StartupRecovery { ... })` then the existing attach-completion seam. If that helper cannot handle the startup-recovery case, fix the helper; do not add a parallel path.

4. **RCRT defers, not errors, when the originator heartbeat is fresh.** `StartupRecovery` must return a deferred/blocked result when `pending_command_heartbeat_at` is within the 15s threshold. This must be covered by a test: fresh heartbeat → no finalize.

5. **`11.1` partial scope must be explicit in the implementation.** The sentinel directory must NOT be initialized in L1. If `11.1` as written bundles all three initializations, tasks.md must be split into `11.1a` (RCRT/supervisor registries) and `11.1b` (sentinel directory) before Fry starts. The split prevents an accidental "I'll wire it while I'm in here" drift that would silently pre-empt L2.

6. **Initialization failure of process-global registries is fatal to serve startup.** No partial-initialization fallback. If the registry fails to initialize, serve must exit before any collection is attached.

7. **No success-shaped lies.** If RCRT cannot finalize/attach a collection at startup, the collection must remain in a blocked, observable state. Do not silently clear `pending_root_path`, `restore_command_id`, or `needs_full_sync` without completing the finalize+attach sequence.

---

## Minimum Proof Required (L1 Only)

1. **Dead-orphan proof** (`17.5ll`, `17.13`): Stale heartbeat + orphaned pending restore → RCRT finalizes exactly once as `StartupRecovery` before any supervisor spawns.
2. **Fresh-heartbeat defer proof**: Fresh heartbeat at startup → RCRT returns deferred, collection stays blocked, no finalize attempt.
3. **Startup order proof**: Direct test or tightly-scoped instrumentation proving `registry init` precedes RCRT and RCRT precedes supervisor spawn for any affected collection.
4. **No-broadening proof**: L1's proof claim is restricted to restore-orphan recovery. `17.5ll` and `17.13` must not silently certify generic `needs_full_sync=1` attach, remap startup, or sentinel-triggered reconcile.
5. **Foreign-owner / stale-session proof**: Stale or foreign `serve_sessions` residue cannot satisfy the RCRT recovery condition; ownership truth must come from `collection_owners`.

---

## Gating

| Gate | Required | Rationale |
|------|----------|-----------|
| **Professor re-confirmation of L1 boundary** | YES — mandatory | Professor approved the full `11.1 + 11.4 + 17.5ll + 17.13` scope. The rescoped L1 is narrower but also differs from what he gated: `11.1` is now partial and `11.4`/`17.12` are deferred. Professor must confirm (a) the partial `11.1` cut line is correct, (b) the startup order constraint is still satisfiable without sentinel directory init, and (c) L2 is a complete, self-standing follow-on. |
| **Nibbler adversarial pre-gate of L1** | YES — mandatory | Nibbler proposed this split, but the partial `11.1` handling is new. Nibbler must confirm (a) the `11.1` task split into registry-only vs. sentinel-directory is clean with no sentinel leakage into L1, (b) `17.5ll`'s fresh-heartbeat defer proof is sufficient to cover the premature-RCRT-firing adversarial seam, and (c) the no-broadening claim for `17.13` is enforceable without `11.4` present. |

Both gates are required before Fry writes a single line of L1 implementation code.

---

## Entry Criteria for Batch L2 (Sentinel Startup Recovery)

L2 must not start until:

1. L1 is landed and gated.
2. `11.1` partial (RCRT/supervisor registries) is confirmed clean in L1.
3. Professor has confirmed the sentinel directory `11.1b` scope.
4. Nibbler has pre-gated the sentinel reader correctness requirements: strict parsing, monotonic non-destructive dirty signal, sentinel-unlink-only-after-reconcile-success, malformed-sentinel → dirty + no convergence claim.

---

## Confidence

High. The split follows Nibbler's adversarial proposal exactly on recovery-authority boundaries, respects Professor's non-negotiable startup order, and applies the team's standing "one recovery story per batch" routing rule established in the Batch J rescope learning. The only novel element is the `11.1` partial split, which needs both reviewers to confirm the cut line before implementation.


# Leela Recommendation — Vault Sync Next Batch After I

## Proposed batch

**Batch J — Plain Sync + Restore/Integrity Proof Closure**

## Task IDs

- Operator entrypoint completion:
  - `9.5`
- Ownership / lease proof closure:
  - `17.5hh`
  - `17.5hh2`
  - `17.5hh3`
  - `17.5hh4`
- Restore / remap / finalize proof closure:
  - `17.5ii`
  - `17.5ii4`
  - `17.5ii5`
  - `17.5kk3`
  - `17.5ll`
  - `17.5ll3`
  - `17.5ll4`
  - `17.5ll5`
  - `17.5pp`
  - `17.5qq`
  - `17.5qq2`
  - `17.5qq3`
  - `17.5qq4`
  - `17.5qq5`
  - `17.5qq6`
  - `17.5qq7`
  - `17.5qq8`
  - `17.9`
  - `17.10`
  - `17.11`
  - `17.13`
- Integrity-halt / operator-surfacing closure:
  - `17.5mm`
  - `17.5nn`
  - `17.5oo`
  - `17.5oo3`

## Why this should come next

Batch I finished the dangerous restore/remap orchestration seam, but the implementation is still missing the normal operator path: plain `gbrain collection sync <name>` (`9.5`) still hard-errors. That leaves the change in an awkward in-between state where the exceptional recovery paths exist, but the routine stat-diff/reconciler path is not yet available.

This is the tightest coherent follow-on because it stays on the exact same state machine:

1. **Close the ordinary operator path.**
   - `9.5` is the missing day-to-day entrypoint for collection reconciliation.
   - It directly depends on the Batch I restore/remap/RCRT wiring already in place and should reuse that ownership + heartbeat contract rather than start a new surface.
2. **Convert Batch I’s orchestration into reviewable proof.**
   - The open `17.5hh*`, `17.5ii*`, `17.5kk3`, `17.5ll*`, `17.5pp`, `17.5qq*`, `17.9`–`17.13` tasks are the unclaimed evidence that leases, finalize paths, remap attach, crash recovery, and the write-gate all behave correctly under real failure modes.
3. **Finish the integrity story before widening scope.**
   - `17.5mm`, `17.5nn`, `17.5oo`, and `17.5oo3` close the remaining integrity-blocked/operator-guidance loop that Batch I introduced (`restore-reset`, `reconcile-reset`, RCRT skip rules, and `brain_collections` surfacing).

## Safety / review considerations

- This batch is still a **destructive-path** batch even though much of the remaining work is proof-oriented.
- It touches restore finalization, remap attach, owner lease transitions, and integrity-blocked recovery states.
- Keeping it limited to `9.5` plus the remaining restore/remap/integrity proof tasks makes review sharp: one seam, one operator entrypoint, one failure model.

## Gating advice

- **Professor pre-implementation gate: REQUIRED.**
  - He should approve the exact Batch J boundary before coding starts, especially the scope of `9.5` (plain sync only, not broader collection UX) and the authority/lease expectations exercised by `17.5hh*`, `17.5pp`, and `17.5qq*`.
- **Nibbler adversarial review: MANDATORY.**
  - The open work is concentrated in failure-mode proof for restore/remap, ownership transfer, finalize recovery, and integrity halts. That is exactly the class of seam where adversarial review matters.

## Why the other plausible batches should wait

- **CLI write routing / IPC / bulk UUID rewrite (`12.6a`, `12.6b`, `17.5ii9`, `17.5ii10`, `17.5ii11`, `17.5ii12`) should wait.**
  - That opens a new write-surface and security seam before the current restore/sync seam has its default operator path and failure proofs closed.
- **Collection breadth (`9.2*`, `9.3`, `9.6`, `9.8*`, `9.10`, `9.11`) should wait.**
  - Useful surface area, but mostly breadth/polish compared with the still-open core reconcile/recovery seam.
- **Watcher/live-sync breadth (`6.*`, `7.*`, `8.*`, `17.5aaa2+`, `17.7`, `17.8`, `17.14`) should wait.**
  - That is the next major serve slice, but it is safer to enter it after the restore/remap/sync state machine is fully operator-complete and adversarially proven.


# Decision: vault-sync-engine Batch K Scope

**Author:** Leela  
**Date:** 2026-04-23  
**Session:** vault-sync-engine post-Batch-J next-batch planning  

---

## Context

Batch J closed with: plain `gbrain collection sync <name>` as active-root reconcile only, short-lived lease discipline, terminal reconcile halts (`reconcile_halted_at`), truthful CLI blocked-state diagnostics, and fail-closed `--finalize-pending` CLI truth. 

Still explicitly deferred after Batch J:
- Destructive restore/remap path closure (integration proofs, not library code — the library is in place)
- Finalize/handshake/integrity matrix beyond the narrowed CLI truth surface (17.5kk3, ll3–ll5, mm, pp/qq series)
- MCP widening for collection surfaces
- Group 6 (watcher), Group 7 (dedup set), Group 8 (embedding queue), Group 12 (brain_put), Group 13 (MCP slug parsing), Groups 15–18

---

## Decision: Batch K = Collection Add + Offline Restore Integrity Matrix

### Proposed label

**Batch K — `gbrain collection add` + Offline Restore Integrity Matrix**

---

### Task IDs in scope

**Group 1 — gap logging completions (small, complete the write-interlock seam):**
- `1.1b` — Extend `log_gap()` / `brain_gap` to accept optional slug; populate `knowledge_gaps.page_id`; update `Gap` struct and list/resolve responses; unit tests for slug and slug-less variants and `has_db_only_state` effect
- `1.1c` — Classify `brain_gap` by variant: slug-bound = `WriteUpdate` (subject to `CollectionRestoringError` interlock); slug-less = `Read` (no interlock). Unit test covers both during `state='restoring'`

**Group 9 — operator entry point:**
- `9.2` — `gbrain collection add <name> <path> [--writable/--read-only] [--watcher-mode]`: validate name (no `::`), open `root_fd` with `O_NOFOLLOW`, persist row, run initial reconciliation. Defers `--write-gbrain-id` (blocked on 5a.5/12.*)
- `9.2b` — Capability probe: attempt a tempfile write inside the root; if EACCES/EROFS, set `collections.writable=0` with WARN; subsequent writes refuse with `CollectionReadOnlyError`
- `9.3` — `gbrain collection list` prints `name | state | writable | write_target | root_path | page_count | last_sync_at | queue_depth`

**Group 17 — restore integrity matrix tests (the deferred finalize/handshake proof surface):**
- `17.5kk3` — Tx-B failure leaves `pending_root_path` set; generic recovery worker does NOT clear the flag
- `17.5ll3` — Successor process with a different restore-command identity cannot bypass the fresh-heartbeat gate
- `17.5ll4` — `pending_manifest_incomplete_at` retries successfully within the retry window
- `17.5ll5` — `pending_manifest_incomplete_at` escalates to `integrity_failed_at` after TTL
- `17.5mm` — Restore manifest tamper detected → `IntegrityFailed` + `restore-reset` flow
- `17.5qq10` — `collection add` capability probe sets `writable=0` on EACCES/EROFS and WARNs
- `17.5qq11` — `CollectionReadOnlyError` refuses writes when `writable=0`

**Group 17 — offline restore end-to-end integration proof:**
- `17.11` — Integration: offline restore finalizes via CLI (the deferred end-to-end proof from Batch I; requires `collection add` to scaffold the target collection)

---

### Deferred from Batch K

- `9.2a` (`--write-gbrain-id` on `collection add`) — blocked on 5a.5 UUID write-back, which requires Group 12 rename-before-commit primitives
- `17.5pp` / `17.5qq` / `17.5qq2`+ (online restore handshake) — blocked on `11.9` (IPC socket), which is not yet implemented; the full handshake proof sequence requires a live serve-side IPC socket
- `17.5ii4` / `17.5ii5` (remap Phase 4 bijection / Phase 1 non-zero drift proofs) — deferred until the remap destructive path has separate integration test infrastructure
- `17.5qq3` / `17.5qq4` (remap online/offline mode) — same dependency as above
- Group 6 (watcher), Group 7 (dedup), Group 8 (embedding queue), Group 12 (brain_put), Group 13 (MCP slug parsing) — none of these have dependency on Batch K; all remain deferred

---

## Why this batch comes next

### Dependency on Batch J

Batch J landed two things that Batch K directly builds on:
1. **`--finalize-pending` CLI truth** — Batch K adds the deep integration test (17.11) that proves the offline restore path exercises `finalize_pending_restore()` honestly end-to-end. Without the Batch J fail-closed behavior, this proof would be misleading.
2. **Terminal reconcile-halt discipline** — Batch K's restore-integrity tests (17.5ll3–ll5, 17.5mm) prove that the `RCRT` path does not bypass the identity checks that were made fail-closed in Batch J's architecture. The reconcile-halt precedent from Batch J establishes the pattern Batch K's tests validate.

### Why `collection add` belongs here

`17.11` (offline restore end-to-end integration proof) requires a collection to exist in the DB. Without `gbrain collection add`, every integration test for restore must use a fabricated DB fixture. Adding the real command here makes the end-to-end proof honest and reusable for future integration tests in Groups 6, 8, 11, and 12.

Additionally, `1.1b` and `1.1c` are the last open items that complete the write-interlock described in `11.8` (which is `[x]` done) — the interlock is incomplete until slug-bound `brain_gap` has its `CollectionRestoringError` classification wired.

### Why the restore integrity matrix goes here and not later

Tasks 17.5kk3, ll3–ll5, mm are the "finalize/handshake/integrity matrix closure beyond the narrowed CLI truth surface" that was explicitly deferred from Batch J. The library code (RCRT, restore-command identity, `pending_manifest_incomplete_at` escalation, integrity-failed path) was all landed in Batches H and I. These tasks are proof-writing, not new-code-writing. Deferring them further creates an increasing gap between implementation and proof, which is exactly the failure mode that required Batch J's narrow repair.

### Coherence of the batch boundary

The batch answers one question: **"Does the offline restore lifecycle work correctly end-to-end, including all non-happy-path recovery branches?"** All tasks in scope either scaffold the proof (collection add), complete interlock correctness (1.1b/c), or prove the restore state machine (17.* integrity cluster). No new non-trivial production code paths are added outside of `collection add` and the gap-logging completions.

---

## Highest-risk seams

### 1. `17.5ll3` — restore-command identity gate (PRIMARY)

This test must prove that a *different* `restore_command_id` (i.e., a successor CLI process that didn't originate the restore) cannot finalize as `RestoreOriginator`. The code path exists in the RCRT and `finalize_pending_restore()`, but the proof has never been exercised. If the identity gate has a bypass — e.g., NULL `restore_command_id` passes — this silently permits unauthorized finalization.

**Mitigation:** Nibbler adversarial review required on the identity gate implementation before this test is accepted as passing.

### 2. `17.5mm` — manifest tamper detection

The `pending_restore_manifest` is stored in the DB. Any path that reads this manifest and does not verify its integrity before proceeding with the destructive step is a data-corruption vector. The test must prove that a tampered manifest triggers `IntegrityFailed` and that `restore-reset` is the required recovery path — not silent acceptance.

**Mitigation:** Nibbler must verify the manifest verification code path in `finalize_pending_restore()` before this test is accepted.

### 3. `9.2` — initial reconciliation on `collection add`

Running initial reconciliation inside `collection add` touches the same write path as `gbrain collection sync`, but without the lease/heartbeat discipline that Batch J added. The initial reconcile must acquire the same short-lived `collection_owners` lease with heartbeat, or the offline concurrency guarantee breaks on first-add.

**Mitigation:** Professor should confirm the lease acquisition design for the initial reconcile in `collection add` before Fry starts the body.

---

## Gating advice

| Gate | Required? | Reason |
|------|-----------|--------|
| **Professor pre-implementation gate** | YES | Sign off on (a) lease acquisition design for `collection add` initial reconcile and (b) identity gate contract in `17.5ll3` before Fry starts those implementations |
| **Nibbler adversarial review** | YES (mandatory) | Must review: restore-command identity gate (`17.5ll3`), manifest tamper detection (`17.5mm`), and Tx-B failure residue path (`17.5kk3`) before merge |

---

## Why alternative next batches should wait

**Alternative A: Online restore handshake (17.5pp/qq)** — Blocked on `11.9` (IPC socket), which is not yet implemented. Cannot be proven without a real live serve process with socket-level peer auth. Wait for the IPC batch (likely after brain_put).

**Alternative B: Watcher pipeline (Group 6)** — Watcher requires the serve process globals from `11.1` (not done) and the dedup set from Group 7 (not done). It also requires `collection add` to exist for test fixtures. This batch should come after Batch K, not before.

**Alternative C: brain_put rename-before-commit (Group 12)** — The most complex and highest-risk remaining batch. Requires IPC socket (11.9), dedup set (7.*), per-slug async mutex (12.4), and 13-step crash-safety sequence. It deserves a separate focused batch and Nibbler pre-implementation security review. It should not be mixed with restore integration work.

**Alternative D: MCP slug parsing (Group 13)** — Depends on most of Groups 9, 11, and 12 being wired. Too early.

---

## Confidence

High. Every task in Batch K either tests code that is already in the tree (restore integrity matrix) or is a natural predecessor to future Groups 6/11/12 integration work (collection add). No task requires a new unsafe seam that isn't already approved. The batch is bounded: it doesn't touch IPC, watcher, brain_put, or MCP surfaces.


# Decision: vault-sync-engine Next Batch After K2 — Batch L Recommendation

**Author:** Leela  
**Date:** 2026-04-23  
**Session:** vault-sync-engine post-K2 batch planning  

---

## Context

K2 closed the offline restore integrity matrix: restore-command identity is persisted, `restore_reset()` is tightened, Tx-B residue is authoritative, manifest tamper is handled, and the CLI can finalize via `collection sync --finalize-pending`. Three seams were explicitly deferred out of K2: startup/orphan recovery, online handshake, and broader post-Tx-B topology.

The RCRT code itself (`9.7d`) is already in the tree and marked complete, and `11.5` (run RCRT before spawning supervisors) is also checked. What's missing is the prove-it layer: the process-global registries that RCRT depends on at startup (`11.1`), the startup sentinel recovery surface (`11.4`), and the integration tests that prove RCRT actually recovers orphaned restores (`17.5ll`, `17.13`).

---

## Recommendation

**Batch Label:** `vault-sync-engine-batch-l`  
**Theme:** Startup & Orphan Recovery Closure

---

## Task IDs

| ID | Description |
|----|-------------|
| `11.1` | Initialize process-global registries: `supervisor_handles`, dedup set, recovery sentinel directory |
| `11.4` | Recover from `brain_put` recovery sentinels directory on startup: `needs_full_sync=1` per affected collection; unlink each sentinel after reconcile |
| `17.5ll` | Orphan recovery: originator dead, RCRT finalizes as `StartupRecovery` |
| `17.13` | Integration: crash mid-restore between rename and Tx-B — RCRT finalizes on next serve start |

**Optional co-traveler (low-risk, self-contained):**

| ID | Description |
|----|-------------|
| `2.4a2` | Windows platform gate: `UnsupportedPlatformError` from serve/put/collection vault-sync commands on `#[cfg(windows)]` |

`2.4a2` has zero dependencies on the startup recovery seam and is a compiler-level `cfg` change, not logic. It can be included to close a long-open stub without adding risk. If the batch feels too wide, defer it.

---

## Rationale

### 1. K2 proved the happy offline path; L proves the crash path

K2 showed that when the originator process is live and holds its identity, it can finalize a pending restore via explicit CLI. `17.5ll` is the complementary proof: what happens when the originator process dies without finalizing? The RCRT-as-`StartupRecovery` path is the entire point of having an RCRT. Without this test, the restore state machine has a one-sided proof. The code is in the tree; the proof is what's missing.

### 2. `11.1` is a prerequisite that should have been tighter in the code

`supervisor_handles`, the dedup set, and the recovery sentinel directory are process-global registries. Right now `11.5` (run RCRT at startup) and `11.7` (supervisor spawn) are marked complete, but `11.1` is not. That implies those features are wired to either zero-initialized in-place or implicit globals. The registry initialization task closes the honest boundary: `serve` has an explicit startup sequence with typed ownership. `17.5ll` needs this to be real or its fixture environment is underspecified.

### 3. `11.4` connects the sentinel directory to the startup recovery loop

`brain_put`'s rename-before-commit (Group 12) is deferred, but the sentinel discipline is already in scope: Step 5 of the rename-before-commit sequence creates a recovery sentinel. Task `11.4` is about reading those sentinels at startup and triggering `needs_full_sync=1`. This is a tight seam: the sentinel directory is initialized in `11.1`; `11.4` wires the recovery reader at startup. Without `11.4`, the sentinel directory is initialized but never drained. Including it now keeps these two infrastructure tasks coherent.

### 4. `17.13` closes the crash-to-recovery integration loop

`17.5ll` is a unit-level proof of the orphan recovery path. `17.13` is the integration-level proof: simulate crash mid-restore (rename done, Tx-B not yet committed), start serve, observe RCRT finalizes. This is the scenario the whole "fail closed, recover on next serve start" design was built for. Including it in Batch L makes the batch a complete story, not a partial one.

### 5. This is lower-risk than all alternatives

| Alternative | Why it should wait |
|-------------|-------------------|
| **Online handshake + IPC** (`11.9`, `12.6a–g`, `17.5pp`, `17.5qq` series) | Requires Nibbler adversarial pre-gate on IPC security design before any code is written. IPC socket placement, bind-time audit, peer verification, and the full cross-UID attack test suite (`17.5ii10–12`) are a separate major surface. Should not open until startup recovery is solid. |
| **`brain_put` rename-before-commit** (Group 12) | The CLI routing logic (`12.6a`) detects a live serve owner and proxies writes over IPC. This makes Group 12 a dependent of IPC. Until IPC exists, brain_put write-through is a partial implementation. |
| **Watcher** (Group 6) | Depends on serve infrastructure being stable and supervisor_handles being initialized. Should follow startup recovery and IPC, not precede them. |
| **Collection quarantine/ignore CLI** (`9.8`–`9.10`) | Standalone polish that can land at any time but does not close any open safety seam. Correct to defer. |

---

## Gating

| Gate | Required | Scope |
|------|----------|-------|
| **Professor pre-implementation gate** | YES — mandatory | Confirm the startup orchestration contract: registry initialization order (`11.1` before `11.5`), sentinel directory lifecycle (create-on-serve-start, drain-on-startup, unlink-after-reconcile), and that RCRT-as-`StartupRecovery` doesn't accidentally impersonate or replay state from a still-live originator. |
| **Nibbler adversarial review** | YES — mandatory | The `StartupRecovery` path in RCRT is a state machine transition with identity implications. Nibbler must verify: (a) RCRT correctly detects "originator dead" vs "originator slow" (heartbeat age threshold); (b) RCRT cannot be triggered while the originator's session is still in `serve_sessions`; (c) `17.5ll` covers the false-positive case where RCRT fires prematurely and a slow originator later tries to finalize. |

---

## Entry Criteria for Batch L

Batch L can start immediately after K2 is landed and gated. No additional prerequisites.

---

## Entry Criteria for Batch M (Online Handshake + IPC)

Batch M must not start until:
1. Batch L is landed and gated.
2. Nibbler has pre-gated the IPC security design (`11.9`, `12.6c–g`) independently of the implementation — the adversarial test list (`17.5ii10–12`, `17.5ii12`) must be reviewed before Fry writes a single line of socket code.
3. Professor confirms the CLI write routing contract (`12.6a`): offline-direct vs. IPC-proxy detection logic, and the `ServeOwnsCollectionError` bulk-rewrite refusal boundary.

---

## Confidence

High. The K2 deferral explicitly named "startup/orphan recovery" as the top-of-queue item. The code (RCRT) is in the tree; the proof (17.5ll, 17.13) and the structural initialization (11.1, 11.4) are what remains. This batch has no new implementation surface, no new I/O channels, and no new security boundary — it is purely closing the proof layer on already-committed design decisions.


# Decision: vault-sync-engine Batch H Scope

**Author:** Leela  
**Date:** 2026-04-22  
**Session:** vault-sync-engine post-Batch-G scoping  

---

## Context

Batch G delivered: `full_hash_reconcile` (real implementation with mode/auth contract), `render_page` UUID preservation (5a.6), `brain_put` UUID round-trip (5a.7 partial), and the zero-active `raw_imports` InvariantViolationError guard in `apply_reingest` (5.4h repair). The Batch G history note explicitly flagged: "Deferred: 5.8* (Batch H, now unblocked by 4.4)".

Restore/remap defense is the highest-risk remaining unimplemented piece of the reconciler core that now has all prerequisites satisfied.

---

## Decision: Batch H = Restore/Remap Safety Pipeline (Phases 0–3) + Fresh-Attach

### Task IDs in scope

**Core pipeline:**
- `5.8a0` — UUID-migration preflight (runs FIRST before RO-mount gate; DB scan, O(page_count))
- `5.8a` — RO-mount gate (`statvfs` / `MNT_RDONLY`; binary refusal with two acceptance paths)
- `5.8a2` — dirty-preflight guard (`is_collection_dirty` + sentinel directory check)
- `5.8b` — Phase 1: drift capture (`full_hash_reconcile` in `synchronous_drift_capture` mode, bypassing `state='active'` gate via lease / `restore_command_id`)
- `5.8c` — Phase 2: stability check (two successive stat-only snapshots; retry up to `GBRAIN_RESTORE_STABILITY_MAX_ITERS`)
- `5.8d` — Phase 3: pre-destruction fence (final stat-only walk; diff → abort via standard abort-path resume)
- `5.8d2` — TOCTOU recheck (re-evaluate `is_collection_dirty` on fresh connection between Phase 2 and destructive step)
- `5.9` — Fresh-attach / first-use-after-detach → `full_hash_reconcile`

**Deferred from Batch G (5.4h remainder):**
- The `--allow-rerender` CLI flag (audit-logged WARN operator override for InvariantViolationError)

**Tests in scope:**
- `17.5aaa` — zero-active `raw_imports` → `InvariantViolationError`; `--allow-rerender` audit-logged override
- `17.5ii2` — RO-mount gate: writable mount refuses; RO mount proceeds (binary gate, no override flag)
- `17.5ii3` — Phase 1 drift capture → authoritative `raw_imports`; Phase 2 stability converges; Phase 3 fence diff aborts cleanly
- `17.5ii6` — TOCTOU dirty-recheck aborts with `CollectionDirtyError`
- `17.5ii7` — dirty-preflight guard refuses restore/remap when dirty or sentinel non-empty
- `17.5ii9a` — UUID-migration preflight refuses with `UuidMigrationRequiredError` naming count + samples; running migrate-uuids then retrying succeeds (requires 5a.5a to be stubbed or deferred in test)
- `17.5ccc` — fresh-attach and first-use-after-detach always run `full_hash_reconcile`

### Deferred from Batch H

- `5.8e` (Phase 4 remap bijection / new-root verification) — complex identity-resolution logic, separate concern
- `5.8f` (online restore with live supervisor) — requires Groups 9 + 11 (CLI + serve)
- `5.8g` (offline restore full execution) — requires Group 9 CLI
- `5a.5` / `5a.5a` (UUID write-back / migrate-uuids) — requires Group 12 rename-before-commit primitives
- Groups 6, 9, 10, 11, 12 — watcher, collection commands, serve integration, brain_put

---

## Why this boundary is coherent and gateable

All seven core pipeline tasks answer the single question: **"what must be provably true before the destructive restore/remap step is allowed to execute?"** They form a strict sequential safety fence:

```
5.8a0 (trivial-content UUID gap)
  → 5.8a (writable mount refused)
    → 5.8a2 (dirty / sentinel check)
      → 5.8b (drift captured)
        → 5.8c (stable after N retries)
          → 5.8d (final stat fence OK)
            → 5.8d2 (TOCTOU dirty recheck)
```

Every function in this pipeline is **library-level** — no CLI execution, no Group 9/11 dependencies. Each can be unit-tested in isolation. The gate is crisp: the pipeline exists, is unit-tested, and by construction the destructive step cannot run without all preceding phases passing. Fresh-attach (5.9) trivially wires in now that `full_hash_reconcile` is real and completes the "RCRT can fully recover a collection" prerequisite for future Group 9 work.

---

## Highest-risk seams

### 1. `5.8b` — Phase 1 drift capture (PRIMARY RISK)

`full_hash_reconcile` is called with `mode=synchronous_drift_capture`, bypassing the `state='active'` guard that was established in Batch G. The authorization must be routed through the lease / `restore_command_id` contract. If the bypass is too permissive (accepts any caller), any code path can trigger a full-tree rehash on a live collection. If too restrictive (fails valid restore calls), the restore pipeline deadlocks.

**Mitigation:** Professor must gate the `mode` enum extension and lease/authorization signature before Fry starts the function body. The authorization contract must be the FIRST thing designed, not retrofitted.

### 2. `5.8a0` — UUID-migration preflight (SECONDARY RISK)

The "trivial content" threshold (body ≤ 64 bytes after frontmatter OR empty body) MUST match exactly the threshold used in the rename-guard (5.3) and in `NewTreeIdentity.body_size_bytes`. A discrepancy here creates a two-class system: pages that the rename guard considers "trivial" but the preflight misses (false negative → silent identity loss on restore), or the reverse (false positive → blocks valid restores). The body-size computation must call the same helper both places.

**Mitigation:** Nibbler must verify that 5.8a0's "trivial content" check is a direct call to the same predicate/helper used in 5.3's rename refusal logic. This is a one-line code audit, not a full adversarial pass, but it's a hard gate.

---

## Implementation author and reviewer/tester pairing

| Role | Who | Gate |
|------|-----|------|
| Implementation | Fry | May not start 5.8b body until Professor signs off on mode/lease contract extension |
| Authorization contract design | Professor | Gate 5.8b function signature before Fry starts |
| Adversarial review | Nibbler | Required on 5.8a0 (trivial-content predicate consistency) and 5.8b (bypass authorization) before merge |
| Coverage sign-off | Bender / Scruffy | 90%+ line coverage on pipeline functions |

---

## tasks.md wording adjustments before Batch H starts

1. **5.4h** — Add a note: "Batch H target: `--allow-rerender` CLI flag and its audit-logged WARN path. The InvariantViolationError guard itself was completed in Batch G for both full_hash_reconcile and apply_reingest paths."

2. **5.8** (container) — Add: "Batch H scope: 5.8a0, 5.8a, 5.8a2, 5.8b, 5.8c, 5.8d, 5.8d2 (pipeline library core only; no CLI execution). Phase 4 (5.8e) and online/offline CLI execution (5.8f/g) are later batches dependent on Groups 9 and 11."

3. **5.8a0** — Clarify: "Test 17.5ii9a requires a stub or deferred resolution for 5a.5a. The preflight gate itself can be implemented and tested independently; the post-migrate-uuids retry assertion may reference a stub command if 5a.5a is not in scope."

4. **5.8b** — Add: "Professor must sign off on the mode enum extension and lease/restore_command_id authorization signature before implementation starts. The authorization contract is the contract — not the hash logic."

5. **17.5aaa** — Add: "Batch H target. Covers both the guard (Batch G) and the `--allow-rerender` CLI recovery override path (Batch H)."


# Leela Recommendation — Vault Sync Next Batch After H

## Proposed batch

**Batch I — Restore/Remap Orchestration + Ownership Recovery**

## Task IDs

- Core restore/remap completion:
  - `5.8e`
  - `5.8f`
  - `5.8g`
- Collection command wiring:
  - `9.1`
  - `9.4`
  - `9.5`
  - `9.7`
  - `9.7a`
  - `9.7b`
  - `9.7c`
  - `9.7d`
  - `9.7e`
  - `9.7f`
- Serve ownership / handshake / write-gate:
  - `11.2`
  - `11.3`
  - `11.5`
  - `11.6`
  - `11.7`
  - `11.7a`
  - `11.8`
- Direct test lock for this batch:
  - `17.5ii`
  - `17.5ii4`
  - `17.5ii5`
  - `17.5jj`
  - `17.5kk`
  - `17.5kk2`
  - `17.5kk3`
  - `17.5ll`
  - `17.5ll2`
  - `17.5ll3`
  - `17.5ll4`
  - `17.5ll5`
  - `17.5pp`
  - `17.5qq`
  - `17.5qq2`
  - `17.5qq3`
  - `17.5qq4`
  - `17.5qq5`
  - `17.5qq6`
  - `17.5qq7`
  - `17.5qq8`
  - `17.9`
  - `17.10`
  - `17.11`
  - `17.13`

## Why this should come next

Batch H deliberately stopped at the Phase 0–3 safety core plus fresh-attach. That means the most dangerous state machine in the change now exists as library code, but users still cannot execute restore/remap end-to-end and reviewers cannot yet verify the ownership handoff, Tx-A/Tx-B recovery, or write-gate behavior in the real command path.

This batch is the tightest coherent follow-on because it keeps one high-risk workflow together:

1. **Finish the same restore/remap seam H opened.**
   - `5.8e` completes remap verification.
   - `5.8f` / `5.8g` connect the safety pipeline to real online/offline execution.
2. **Wire the operator surface that owns the workflow.**
   - `9.5` / `9.7*` expose sync/restore/finalize/reset entry points and recovery behavior.
3. **Wire the serve-side ownership model the H authorization repair depends on.**
   - `11.2` / `11.3` / `11.6` / `11.7` / `11.7a` / `11.8` make persisted ownership, handshake, and write-interlock real rather than theoretical.
4. **Lock recovery and idempotence before branching into new surfaces.**
   - `17.5kk*`, `17.5ll*`, `17.5pp`, `17.5qq*`, `17.10`, `17.11`, `17.13` are the review proof that restore/remap survives retries, crashes, and ownership changes.

## Gating advice

- **Professor pre-implementation gate: REQUIRED.**
  - He should approve the batch boundary and exact ownership/state-transition contract before coding starts, especially the `(session_id, reload_generation)` handshake, Tx-A/Tx-B finalize rules, and how `11.8` applies during remap/restore.
- **Nibbler adversarial review: MANDATORY.**
  - This batch is still a destructive-path batch: restore/remap, ownership transfer, pending-finalize recovery, and write-gate correctness are all data-loss surfaces.

## Why not the other plausible batches yet

- **`brain_put` / UUID write-back (`12.*`, `5a.5*`) should wait.**
  - It is a large standalone write-surface with its own IPC and dedup security story. Starting it now would leave restore/remap half-wired immediately after H created the safety seam it depends on.
- **Watcher / live-sync (`6.*`, `7.*`, `8.*`) should wait.**
  - Product-visible, yes, but it is a separate serve slice. The restore/remap state machine is already half-built and riskier to leave suspended.
- **Broad collection CLI / ignore / slug routing (`9.2*`, `9.3`, `9.8*`, `10.*`, `13.*`, `14.*`) should wait.**
  - Useful surface area, but not the shortest path to closing the currently open safety-critical workflow.


# Mom decision — 13.3 third revision

## Context

Batch `13.3` had one remaining production bug and two remaining proof gaps:

- `gbrain embed <collection>::<slug>` resolved the page up front, then later looked up the page id with a bare `pages.slug = ?` query.
- `gbrain query` and `gbrain unlink` lacked direct subprocess evidence that explicit `<collection>::<slug>` routing really works on the CLI surface.

Prior ambiguous-bare-slug refusals and canonical no-match reporting were already accepted and had to stay intact.

## Decision

Keep this revision strictly scoped to `13.3` by applying one routing rule consistently:

1. For explicit single-page embed, resolve once via collection-aware slug parsing.
2. Carry the resolved `(collection_id, slug)` pair through the later page lookup and page-id lookup.
3. Add subprocess proofs only for the still-missing explicit CLI surfaces (`query`, `unlink`) plus a focused embed regression test for the resolved-key binding bug.

## Why

The edge-case failure was not in initial parsing; it was in a second downstream lookup that quietly discarded collection identity. Fixing that specific bind point closes the real parity hole without widening into deferred collection-filter, IPC, startup, or broader routing work.

The new tests are deliberately narrow:

- `query` proves explicit exact-slug routing succeeds where bare exact-slug input must fail closed as ambiguous.
- `unlink` proves explicit routing removes the intended cross-collection edge and reports canonical addresses.
- `embed` proves duplicate-slug collections do not cross-bind during single-page embedding.


# Decision: 13.5 repair — collection filter must be threaded through progressive expansion

**Author:** Mom  
**Date:** 2026-04-25  
**Slice:** 13.5 (MCP read filter — `brain_search`, `brain_query`, `brain_list`)  
**Context:** Nibbler rejected Fry's 13.5 artifact; Mom assigned as revision author.

---

## Decision

### D-13.5-R1: `progressive_retrieve` must accept and enforce `collection_filter: Option<i64>`

**What changed:**
- `progressive_retrieve(initial, budget, depth, conn)` → `progressive_retrieve(initial, budget, depth, collection_filter, conn)`.
- `outbound_neighbours` likewise extended with `collection_filter: Option<i64>`.
- SQL condition added to `outbound_neighbours`: `AND (?3 IS NULL OR p2.collection_id = ?3)`.

**Why:**
The 13.5 contract requires that `brain_query` with an explicit or defaulted collection filter only returns pages from that collection — including pages reached via `depth="auto"` link expansion. Without this fix, `outbound_neighbours` fetched link targets without any collection constraint on the target page (`p2`), silently crossing into other collections during BFS expansion.

**Scope:**
- MCP path: `brain_query` passes `collection_filter.as_ref().map(|c| c.id)` → expansion stays inside the filter.
- CLI path (`commands/query.rs`): passes `None` → `?3 IS NULL` evaluates true, clause is a no-op, behaviour unchanged.
- All existing `progressive_retrieve` unit tests updated with `None`.

**Proof:**
New test `brain_query_auto_depth_does_not_expand_across_collections` in `src/mcp/server.rs`:
- Two collections: `default` (id=1) and `work` (id=2).
- Cross-collection link: `default::concepts/anchor` → `work::concepts/outside`.
- `brain_query` with `collection: Some("default")` and `depth: Some("auto")`.
- Asserts `work::concepts/outside` does NOT appear in results.

**Validation:** All three required passes green (see history.md).


# D-Mom-Quarantine: Quarantine Slice Revision (Export/Restore Blockers)

**Status:** Implemented  
**Date:** 2026-04-25  
**Author:** Mom (revision author)  
**Context:** Professor and Nibbler rejected Fry's initial quarantine slice on four production-behavior blockers. Mom assigned as revision author while Fry remains locked out of this cycle.

## Problem

Four specific rejection blockers in the quarantine implementation:

1. **Failed export still unlocks discard**: `export_quarantined_page()` called `record_quarantine_export()` *before* the filesystem write, so a failed `fs::write()` still recorded the export timestamp and unlocked non-`--force` discard.

2. **Restore accepts non-Markdown targets**: `restore_target_relative_path()` auto-appended `.md` if missing but did not *validate* that the final extension was `.md`, so `notes/restored.txt` → `notes/restored.txt.md` succeeded.

3. **Restore bypasses live-owner write seam**: `restore_quarantined_page()` wrote vault bytes directly even when `serve` owned the collection, bypassing the established `ServeOwnsCollectionError` gate.

4. **Restore can leave bytes on disk while SQLite still marks quarantined**: Restore wrote temp file → renamed → synced parent, *then* started the SQLite transaction. If any DB step failed, the file remained on disk while `pages.quarantined_at` stayed non-NULL.

## Decision

### D-Mom-Q1: Export only records tracking row after filesystem write succeeds

`export_quarantined_page()` now:
1. Loads page + builds payload
2. Writes `fs::write(output_path, serde_json::to_vec_pretty(&payload)?)?`
3. Only on success: calls `record_quarantine_export(...)` to persist the timestamp
4. Returns `QuarantineExportReceipt` with actual recorded timestamp

This ensures failed export never unlocks discard.

### D-Mom-Q2: Restore rejects non-`.md` targets with extension validation

`restore_target_relative_path()` now returns `Result<PathBuf, QuarantineError>`:
1. Auto-appends `.md` if missing
2. Validates `path.extension() == Some("md")`
3. Returns `Err` with descriptive message if validation fails

This prevents `.txt`, `.pdf`, or other non-Markdown file writes.

### D-Mom-Q3: Restore refuses when serve owns collection

`restore_quarantined_page()` now calls `vault_sync::ensure_collection_not_live_owned(conn, collection_id)?` immediately after the write-allowed check.

New `ensure_collection_not_live_owned()` helper added to `vault_sync.rs`:
- Checks `owner_session_id(conn, collection_id)`
- If owner exists and `session_is_live(conn, &owner_session)`, returns `ServeOwnsCollectionError`
- Otherwise proceeds

This closes the live-owner bypass gap.

### D-Mom-Q4: Restore commits DB changes first, then writes filesystem with rollback

`restore_quarantined_page()` reordered steps:
1. Start SQLite transaction
2. Clear `file_state`, update `pages.quarantined_at = NULL`, enqueue embedding, delete `quarantine_exports`
3. Open parent fd, create temp file
4. Write bytes, sync file
5. Rename temp → target, sync parent
6. Stat restored file, hash it
7. Upsert `file_state` (still inside same transaction)
8. **Commit transaction**

On any filesystem failure (steps 3-7), the transaction rolls back before committing. No residue left in either filesystem or DB.

## Rationale

**Export-first ordering** prevents the race where transient I/O failure (full disk, permission issue) records success before verifying it. The pattern now matches PUT's write-then-register model.

**Extension validation** is a narrow input gate that prevents the restore API from accepting invalid restore targets. The auto-`.md` convenience remains but is now followed by explicit rejection of non-Markdown final extensions.

**Live-owner check** preserves the serve-vs-CLI write fence. Restore is a vault write that modifies `file_state` and places bytes on disk; it must honor the same ownership gates as `put`.

**Transaction-first ordering** ensures atomicity: either both DB state and filesystem bytes succeed together, or neither persists. The earlier ordering (filesystem first, DB second) could leave orphaned bytes if the transaction failed. Rolling back the DB on filesystem failure is the correct recovery path here.

## Impact

- Export failures no longer silently unlock discard
- Restore rejects `.txt`, `.pdf`, and other non-Markdown paths with a clear error
- Restore refuses to run when `serve` owns the collection, surfacing `ServeOwnsCollectionError`
- Failed restore leaves no residue on disk or in DB (rollback on any failure)

Task `9.8` now closes fully with the default surface implemented and all four blockers resolved.

## Tests

Four new focused tests in `tests/quarantine_revision_fixes.rs`:

1. `blocker_1_failed_export_does_not_unlock_discard`: creates blocked parent dir, verifies export fails, verifies `quarantine_exports` row is absent, verifies discard still requires `--force`
2. `blocker_2_restore_rejects_non_markdown_targets`: attempts `notes/restored.txt`, verifies failure with `.md` validation error, verifies no `.txt` file written
3. `blocker_3_restore_refuses_live_owned_collection`: establishes live `serve_sessions` + `collection_owners` row, verifies restore fails with `ServeOwnsCollectionError`, verifies no file written
4. `blocker_4_restore_rollsback_db_when_filesystem_write_fails`: creates conflicting file at target path, verifies restore fails, verifies `quarantined_at` remains non-NULL, verifies `file_state` not created, verifies no temp files remain

All four tests pass. Existing lib-level quarantine tests remain green.

## Alternatives Considered

**Export-first:** Could have wrapped the write in a temp file + rename. Rejected as over-engineering — the simpler fix is to defer the tracking row until after success.

**Restore ordering:** Could have kept filesystem-first and added cleanup on DB failure. Rejected because cleanup paths are error-prone; transaction-first rollback is the canonical pattern for atomic multi-resource commits.

**Extension validation:** Could have rejected any input with a non-`.md` extension before auto-append. Rejected to preserve the convenience behavior (user types `notes/restored`, gets `notes/restored.md`). Validation after auto-append is sufficient.

## Related

- Task `9.8` (quarantine default surface)
- Task `7.5` (quarantine dedup cleanup prerequisite)
- Professor/Nibbler quarantine slice review (rejection cycle)
- `ServeOwnsCollectionError` first established in write-interlock seam (M1b-i)

## Review Cycle

- **Original author:** Fry (locked out during this revision)
- **Revision author:** Mom
- **Rejection reviewers:** Professor, Nibbler
- **Next reviewers:** TBD (different reviewers per phase 3 workflow)


# Mom — Vault-Sync Edge Case Audit

**Status:** Audit complete; no code modified  
**Date:** 2026-04-25  
**Author:** Mom (read-only audit)  
**Scope:** `vault_sync.rs`, `quarantine.rs`, `collection.rs`, `tests/watcher_core.rs`, `tests/quarantine_revision_fixes.rs`

---

## Findings

### Finding 1 — Deferred-restore tests assert the bail, not the validation logic

All four restore tests in `tests/quarantine_revision_fixes.rs` assert the same bail string:

```
"quarantine restore is deferred in this batch until crash-durable cleanup and no-replace install land"
```

Their names describe behaviors that are not exercised:

| Test name | Named behavior | Actually tested |
|---|---|---|
| `restore_surface_is_deferred_for_non_markdown_target` | `.md` extension validation | CLI bail fires before function call |
| `restore_surface_is_deferred_for_live_owned_collection` | live-owner gate | CLI bail fires before function call |
| `restore_surface_is_deferred_before_target_conflict_mutation` | conflict detection + no mutation | CLI bail fires before function call |
| `restore_surface_is_deferred_for_read_only_collection` | writable flag enforcement | CLI bail fires before function call |

**Impact:** The test names create false confidence that these behaviors are tested. When restore is re-enabled, the tests must be updated — but there is no indication of this in the test code itself. The bail string is also a substring match, so if the message changes the assertion still passes vacuously if the substring happens to reappear.

**Action:** When restore is re-enabled at the CLI layer, all four tests must be rewritten to assert actual behavior. Until then, a comment at the top of the test file should state: _"These tests assert the deferred-surface bail, not the validation logic their names describe."_

---

### Finding 2 — Discard happy-path after successful export is untested

`blocker_1_failed_export_does_not_unlock_discard` (the sole real production-behavior test in `quarantine_revision_fixes.rs`) proves the negative case: failed export → `quarantine_exports` row not written → discard remains blocked.

**The positive case is not tested anywhere:**
- Successful export → `quarantine_exports` row written with current epoch → `has_current_export = true` → `discard_quarantined_page(force=false)` succeeds even when `counts.any() = true`

This is the direct intended behavior of the blocker-1 fix. Only one side of the contract is tested.

**Highest-value missing test:** `discard_succeeds_after_current_epoch_export_with_db_only_state`  
**File:** `src/core/quarantine.rs` unit tests or `tests/quarantine_revision_fixes.rs`

```rust
#[test]
fn discard_succeeds_after_current_epoch_export_with_db_only_state() {
    // insert quarantined page with knowledge_gap (db_only_state)
    // call export_quarantined_page → should succeed
    // assert quarantine_exports row exists with current epoch
    // call discard_quarantined_page(force=false) → should succeed
    // assert page deleted from DB
}
```

---

### Finding 3 — `discard` with `force=true` is untested

The `force=true` branch in `discard_quarantined_page` bypasses the `ExportRequired` guard entirely:

```rust
if counts.any() && !force && !exported_before_discard {
    return Err(QuarantineError::ExportRequired { ... });
}
```

No test at any level (unit or integration) exercises `force=true` with `counts.any()=true`. The force path is used by the CLI `--force` flag but is untested.

**Missing test:** `discard_with_force_and_db_only_state_skips_export_guard`  
**File:** `src/core/quarantine.rs` unit tests

---

### Finding 4 — `discard_quarantined_page` writable-flag policy is undocumented and untested

`discard_quarantined_page` calls `ensure_collection_write_allowed`, which checks only `state` and `needs_full_sync`. It does **not** check the `writable` flag. This means discarding a quarantined page from a `writable=false` collection succeeds.

This is probably intentional: `discard` is a pure SQLite `DELETE FROM pages` with no vault filesystem bytes written. The `writable` flag guards vault-byte writes. Discard is not a vault write.

But it is undocumented and there is no test asserting this is the contract.

**Decision needed (P1):** Is `discard` on a read-only collection intentionally allowed? If yes, add a test and a code comment on the call site. If no, replace `ensure_collection_write_allowed` with `ensure_collection_vault_write_allowed`.

**Recommendation:** The intentional reading is correct. Add a test and a comment.

**Missing test:** `discard_allowed_on_read_only_active_collection`  
**File:** `src/core/quarantine.rs` unit tests  

```rust
// writable=false should not block discard because discard writes no vault bytes
fn insert_collection_readonly(conn: &Connection) -> i64 {
    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target)
         VALUES ('work', '/vault', 'active', 0, 0)",
        [],
    ).unwrap();
    conn.last_insert_rowid()
}

#[test]
fn discard_allowed_on_read_only_active_collection() {
    let conn = open_test_db();
    let cid = insert_collection_readonly(&conn);
    let page_id = insert_quarantined_page(&conn, cid, "notes/q", "2026-01-01T00:00:00Z");
    // no db_only_state, no force needed
    let receipt = discard_quarantined_page(&conn, "work::notes/q", false).unwrap();
    assert_eq!(receipt.slug, "notes/q");
    // page is gone
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM pages WHERE id=?1", [page_id], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
}
```

---

### Finding 5 — `record_quarantine_export` upsert silently overwrites audit trail

```sql
ON CONFLICT(page_id, quarantined_at) DO UPDATE SET
    exported_at = excluded.exported_at,
    output_path = excluded.output_path
```

Re-exporting a quarantined page to a different output path silently updates `exported_at` and `output_path`. The original export timestamp is lost. If re-export is a valid user operation (exporting to a different path), the behavior is correct. If the intent is an append-only audit trail, it is wrong.

The current test (`current_export_only_matches_current_quarantine_epoch`) tests epoch-matching only. No test exercises re-export.

**Decision needed (P2):** Is re-export a valid user operation? If yes, document the upsert semantics and add a test. If no, add an existence check before INSERT and return a conflict error.

**Recommendation:** Re-export to a different path is a legitimate use case (e.g., relocating the output file). Keep the upsert. Add a test documenting the behavior and an `eprintln!` noting the overwrite.

**Missing test:** `re_export_updates_exported_at_and_output_path`  
**File:** `src/core/quarantine.rs` unit tests

---

### Finding 6 — `sweep` with `GBRAIN_QUARANTINE_TTL_DAYS=0` is untested

`sweep_expired_quarantined_pages` uses:
```sql
CAST(strftime('%s', 'now') - strftime('%s', quarantined_at) AS INTEGER) >= (?1 * 86400)
```

With `ttl_days=0`, the filter becomes `>= 0` which matches every quarantined page. The existing test uses a fixed past date (`2026-01-01T00:00:00Z`) which is always expired, bypassing the boundary condition.

**Missing test:** `sweep_with_zero_ttl_discards_page_quarantined_just_now`  
**File:** `src/core/quarantine.rs` unit tests  

```rust
#[test]
fn sweep_with_zero_ttl_discards_page_quarantined_just_now() {
    std::env::set_var("GBRAIN_QUARANTINE_TTL_DAYS", "0");
    let conn = open_test_db();
    let cid = insert_collection(&conn);
    // insert page with quarantined_at = strftime now
    conn.execute(
        "INSERT INTO pages ... quarantined_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') ...",
        ...
    ).unwrap();
    let page_id = conn.last_insert_rowid();
    let summary = sweep_expired_quarantined_pages(&conn).unwrap();
    assert_eq!(summary.discarded, 1);
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM pages WHERE id=?1", [page_id], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
    std::env::remove_var("GBRAIN_QUARANTINE_TTL_DAYS");
}
```

---

### Finding 7 — Watcher integration coverage is minimal

`tests/watcher_core.rs` has exactly one test: `start_serve_runtime_defers_fresh_restore_without_mutating_page_rows`. This proves that `start_serve_runtime` does not mutate a collection in `restoring` state with a live foreign heartbeat. The watcher itself is essentially integration-untested.

**Missing coverage areas:**

| Gap | File target | Difficulty |
|---|---|---|
| Non-.md file events filtered by `relative_markdown_path` | `src/core/vault_sync.rs` unit tests | Low — unit test `relative_markdown_path(Path::new("file.json"))` returns `None` |
| Watcher replaced on `generation` bump (`sync_collection_watchers`) | `tests/watcher_core.rs` | Medium — two sequential serve sessions, verify watcher rebuilt |
| Reconcile halt written via `convert_reconcile_error` | `src/core/vault_sync.rs` unit tests | Low — can call `convert_reconcile_error` directly with a halt-class error |
| Channel overflow → `needs_full_sync` | `tests/watcher_core.rs` | High — requires flooding the channel before poll drains it |

**Minimum valuable addition:** Add a unit test for `relative_markdown_path` with non-.md inputs. This is a two-line test that closes a gap in the event-filtering contract.

---

### Finding 8 — `QUARANTINE_SWEEP_INTERVAL_SECS` is not configurable

`start_serve_runtime` uses a hardcoded `const QUARANTINE_SWEEP_INTERVAL_SECS: u64 = 24 * 60 * 60`. There is no env-var override. Writing an integration test that proves the serve loop fires the sweep requires either a 24-hour wait or a testability escape hatch.

**Action if serve-loop sweep integration test is desired:** Add `GBRAIN_QUARANTINE_SWEEP_INTERVAL_SECS` env var override (analogous to the existing `GBRAIN_QUARANTINE_TTL_DAYS` override).

---

## Prioritized test backlog

| Priority | Test | File | Complexity |
|---|---|---|---|
| P1 | `discard_succeeds_after_current_epoch_export_with_db_only_state` | `quarantine.rs` unit tests | Low |
| P1 | `discard_with_force_and_db_only_state_skips_export_guard` | `quarantine.rs` unit tests | Low |
| P1 | `discard_allowed_on_read_only_active_collection` (+ decision) | `quarantine.rs` unit tests | Low |
| P2 | `re_export_updates_exported_at_and_output_path` | `quarantine.rs` unit tests | Low |
| P2 | `sweep_with_zero_ttl_discards_page_quarantined_just_now` | `quarantine.rs` unit tests | Low |
| P3 | `relative_markdown_path_returns_none_for_non_md_inputs` | `vault_sync.rs` unit tests | Low |
| P3 | `convert_reconcile_error_writes_reconcile_halted_at` | `vault_sync.rs` unit tests | Low |
| P4 | `watcher_rebuilt_on_generation_bump` | `tests/watcher_core.rs` | Medium |
| P5 | Convert 4 deferred-restore tests to real behavior tests | `tests/quarantine_revision_fixes.rs` | High (needs restore re-enabled first) |
| P5 | `sweep_fires_in_serve_loop` (needs interval override) | `tests/watcher_core.rs` | High (needs FIX-3) |

---

## Production fix candidates

### FIX-1: Clarify discard writable-gate policy (comment + test)

Add a comment on the `ensure_collection_write_allowed` call in `discard_quarantined_page`:

```rust
// discard is a pure DB operation (DELETE FROM pages); it writes no vault bytes,
// so we enforce state/sync guards only — not the writable flag.
vault_sync::ensure_collection_write_allowed(conn, resolved.collection_id)?;
```

Add unit test `discard_allowed_on_read_only_active_collection` to lock this in.

**Risk:** Low. Documents existing behavior.

### FIX-2: Log re-export overwrite in `record_quarantine_export`

Add an `eprintln!` when the INSERT detects a conflict (re-export case):

```rust
// After execute: check rows_changed. If 0 rows inserted (conflict handled by DO UPDATE),
// log the overwrite.
eprintln!(
    "INFO: quarantine_re_exported page_id={} quarantined_at={} new_output_path={}",
    page_id, quarantined_at, output_path
);
```

This makes re-export visible in logs without changing semantics.

**Risk:** Low. Additive logging only.

### FIX-3 (if needed): Add `GBRAIN_QUARANTINE_SWEEP_INTERVAL_SECS` env override

Analogous to existing `GBRAIN_QUARANTINE_TTL_DAYS`. Required only if a serve-loop sweep integration test is planned.

**Risk:** Low. Additive env var.


## 2026-04-24: Vault-sync-engine Batch 13.3 review
**By:** Nibbler
**What:** REJECT Batch 13.3 in its current form.
**Why:** The main CLI slug-bearing paths are close: explicit `<collection>::<slug>` routing looks aligned with MCP, canonical page references now show up across get/link/graph/timeline/check/list/search/query surfaces, and the coupled `assertions.rs` UUID-first fallback is justified because duplicate-slug brains would otherwise let `extract_assertions()` bind import assertions to the wrong page_id. But two fail-closed seams remain open: `src/core/search.rs::exact_slug_result_canonical()` collapses `SlugResolution::Ambiguous` into `Ok(None)` and silently falls through to generic search semantics for `gbrain query`, and `src/commands/link.rs::unlink()` still emits `no matching link found between {from} and {to}` using raw user input after both pages were already resolved collection-aware.
**Required follow-up:** Make exact-slug query mode surface the same ambiguity failure instead of degrading into search/no-results behavior, canonicalize the resolved `unlink` no-match failure path, and keep the closure note tightly scoped to CLI slug-routing/output parity only. Do not claim `13.5` collection filters/defaults, `13.6`, or anything broader than the narrowly necessary duplicate-slug assertion binding fix.


# 2026-04-24 — Nibbler review of Batch 13.6 + 17.5ddd

**Verdict:** REJECT

## Blocking findings

1. **`integrity_blocked` escalates too early.**  
   `brain_collections` currently derives `manifest_incomplete_escalated` from a hardcoded `MANIFEST_INCOMPLETE_ESCALATION_SECS = 30` in `src\core\vault_sync.rs`, but the frozen contract says this boundary is controlled by `GBRAIN_MANIFEST_INCOMPLETE_ESCALATION_SECS` with a default of **1800 seconds / 30 minutes**. That means MCP clients would start treating a recoverable manifest-incomplete restore as terminally blocked about 29.5 minutes too early and recommend `restore-reset` prematurely.

2. **Deferred `.gbrainignore` absence semantics leaked into this slice.**  
   The batch was supposed to stay at `13.6 + 17.5ddd` only, with `17.5aa5` explicitly deferred, but the new MCP projection now surfaces the stateful absence discriminator `file_stably_absent_but_clear_not_confirmed`. That is not just generic schema plumbing; it exposes deferred collection-state semantics through `brain_collections`, and the new tests lock that widening in.

## Non-blocking notes

- `root_path` masking looks truthful: active collections surface the stored path, detached/restoring collections mask it to `null`.
- `restore_in_progress` is narrowly shaped rather than mirroring all `state='restoring'`, which is directionally correct for the frozen advisory contract.
- `integrity_blocked` precedence order itself matches the documented severity ordering; the main problem is the wrong escalation threshold feeding it.

## Required repair

- Drive manifest-incomplete escalation from the same configurable source the spec names, with the documented default.
- Remove or explicitly defer the absence-discriminator widening from this batch, or stop claiming this slice is limited to `13.6 + 17.5ddd`.


# Nibbler — N1 MCP slug-routing truth review

Date: 2026-04-24
Branch: `spec/vault-sync-engine`
Batch: **N1 — MCP slug-routing truth**
Task IDs reviewed: `13.1`, `13.2`, `13.4`

## Verdict

**REJECT**

## Why

The trust-boundary work inside MCP is mostly correct and fail-closed:

- bare ambiguous slugs surface a stable machine-readable payload (`code = "ambiguous_slug"`, `candidates = [...]`)
- explicit `<collection>::<slug>` goes through collection-aware parsing before page lookup
- MCP responses that reference pages now emit canonical `<collection>::<slug>` values across get/query/search/list/backlinks/graph/timeline/check surfaces

But the current diff is not honestly scoped to the approved MCP-only lane.

## Blocking seam

`src/commands/check.rs` now changes the CLI `check` flow itself:

- explicit collection slugs are resolved through `resolve_slug_for_op(...)`
- contradiction filtering switches to page-id targeting for the single-page path

That is real CLI behavior widening on a deferred surface (`13.3`), even if it was introduced to make `brain_check` correct. Shared-helper reuse is not a scope exemption.

## Smallest concrete repair

Keep the MCP `brain_check` fix, but stop widening the user-facing CLI `check` contract in this batch. If shared code must change, keep the deferred CLI behavior explicitly out of scope and non-landing for N1 rather than silently shipping partial parity.

## Validation notes

Targeted MCP proofs reviewed and passing locally:

- `brain_get_returns_structured_ambiguity_payload_for_colliding_bare_slug`
- `brain_tags_explicit_collection_slug_updates_only_resolved_page_when_slug_collides`
- `brain_check_filters_output_to_requested_slug`
- `brain_search_returns_matching_pages`
- `brain_query_auto_depth_expands_linked_results`


# Nibbler Gate — Vault Sync Batch H

Date: 2026-04-22
Change: `openspec/changes/vault-sync-engine`
Batch: H (`5.8a0`, `5.8a`, `5.8a2`, `5.8b`, `5.8c`, `5.8d`, `5.8d2`, `5.9`)
Verdict: **REJECT**

## Blocking seam

`5.8b` is not actually identity-scoped yet.

- `FullHashReconcileAuthorization` carries strings (`restore_command_id`, `lease_session_id`, `attach_command_id`), but `authorize_full_hash_reconcile()` only checks that the string is non-empty and that the enum variant matches the collection state.
- There is no comparison against persisted ownership state before the root is opened or files are walked.
- In the reviewed artifacts, `src/schema.sql` does not persist restore-command or lease-owner identity on `collections`, so the reconciler has nothing authoritative to compare against even if a caller supplies a forged non-empty token.

Result: any caller that can reach this seam can mint an arbitrary non-empty restore/remap identity string and satisfy the Batch H drift-capture bypass gate, defeating the “closed mode keyed to caller identity” requirement.

## Controlled seams in this slice

These seams look acceptably controlled:

- UUID-migration preflight and hash-rename both use the shared canonical body significance rules (`MIN_CANONICAL_BODY_BYTES`, empty-body refusal, trimmed compiled_truth+timeline sizing), so I do not see a drifted trivial-content copy.
- The public restore/remap safety pipeline orders UUID preflight → RO gate → dirty preflight → Phase 1 → Phase 2 → Phase 3 → fresh-connection dirty recheck, with no current destructive call site before the TOCTOU check.
- Fresh attach is kept separate from drift capture: `FreshAttach` is its own mode, requires `AttachCommand`, and clears `needs_full_sync` only after the dedicated full-hash attach pass succeeds.
- Tasks/docs stay honest that Batch H lands Phase 0–3 helpers plus fresh-attach wiring only; Phase 4 and end-to-end restore/remap execution remain deferred.

## Narrowest next authoring route

1. Persist the expected destructive owner identity in authoritative state (`restore_command_id` and the remap-owning lease/session identity, or exact equivalents).
2. Load that state in the reconciler authorization path and require exact equality with the supplied authorization token before opening the root.
3. Add regression tests that prove:
   - wrong-but-non-empty restore identity is rejected,
   - wrong-but-non-empty remap lease/session identity is rejected,
   - correct identity still passes,
   - fresh-attach authorization remains separate and does not reuse the drift-capture owner check.


# Nibbler Re-Gate — Vault Sync Batch H

Date: 2026-04-22
Change: `openspec/changes/vault-sync-engine`
Batch: H (`5.8a0`, `5.8a`, `5.8a2`, `5.8b`, `5.8c`, `5.8d`, `5.8d2`, `5.9`)
Verdict: **APPROVE**

## Controlled seam

The previously rejected seam is now acceptably controlled: forged non-empty restore/remap identities no longer authorize the destructive full-hash bypass.

- `authorize_full_hash_reconcile()` now routes restore/remap modes through `require_persisted_full_hash_owner_match()`.
- `require_owner_identity_match()` fails closed when persisted owner state is absent and only accepts exact equality against:
  - `collections.active_lease_session_id`
  - `collections.restore_command_id`
  - `collections.restore_lease_session_id`
- `full_hash_reconcile_authorized()` loads those persisted fields before opening the root or walking files, so the check is bound to stored collection ownership state rather than mode shape.
- `FreshAttach` remains separate: it still accepts only `AttachCommand` on a detached collection and does not reuse the restore/remap owner-match path.

## Test signal

The new unit coverage is meaningfully adversarial for this seam:

- wrong-but-non-empty `restore_command_id` is rejected
- wrong-but-non-empty `restore_lease_session_id` is rejected
- wrong-but-non-empty `active_lease_session_id` is rejected
- matching restore/restore-lease/active-lease identities are accepted
- fresh-attach still rejects lease auth and only allows attach-command auth on detached collections

That is sufficient for this narrow re-gate because the exploit depended on forged non-empty tokens being accepted without an equality check. That behavior is now closed.


# Nibbler — Vault Sync Batch I Adversarial Pre-Gate

## Verdict

**APPROVE** the proposed Batch I boundary, but only as one tightly-audited slice.

## Why it stays intact

This batch is the smallest honest unit that closes the restore/remap seam Batch H left open. Splitting ownership recovery, handshake release, Tx-B finalize, RCRT reattach, and the write-gate would create success-shaped commands that can move roots without a single trustworthy actor controlling when writes reopen.

## Adversarial non-negotiables

1. **`collection_owners` is the only live ownership truth.**
   - Capture expected owner from `collection_owners`, then prove that same `session_id` is live through `serve_sessions`.
   - Never resolve ownership from arbitrary live `serve_sessions`, `supervisor_handles`, or leftover restore-command columns.

2. **Ack acceptance must be exact and fail-closed.**
   - Accept only `(watcher_released_session_id == expected_session_id && watcher_released_generation == cmd_reload_generation && watcher_released_at IS NOT NULL)`.
   - Abort on stale generation, foreign session, owner change mid-poll, owner heartbeat expiry, or fresh-serve impersonation during `state='restoring'`.
   - Clear prior ack residue before every new handshake and bump generation on abort so delayed writes cannot satisfy a later command.

3. **Finalize authority must stay narrow.**
   - `run_tx_b` must be the only SQL path that flips `root_path`, clears pending restore state, arms `needs_full_sync`, and clears restore-command identity.
   - `finalize_pending_restore` must require an explicit caller identity and manifest revalidation; no existence-only or inline-UPDATE shortcut is acceptable.
   - Only `RestoreOriginator` may bypass the fresh-heartbeat defer gate. Runtime recovery belongs to `StartupRecovery`/`ExternalFinalize`; the supervisor is not a hidden finalize actor.

4. **Reattach authority must stay even narrower.**
   - RCRT is the only runtime actor allowed to reattach, run `full_hash_reconcile`, clear `needs_full_sync`, flip back to active attach-complete state, and spawn the next supervisor.
   - The originator may finalize, but it must not also reopen writes by attaching directly.

5. **Write reopening must be late, global, and OR-gated.**
   - Every mutator must refuse while `state='restoring'` **or** `needs_full_sync=1`, before any DB or FS mutation.
   - This must cover CLI, MCP, proxy-routed writes, and admin-like paths (`brain_check`, `brain_raw`, `brain_link`, slug-bound `brain_gap`, ignore edits, UUID write surfaces).
   - Any path that checks only `state` or only page-byte writers is a reopen hole.

6. **Double-attach and drift control must be single-flight.**
   - Online remap may do only the DB transition; RCRT owns the post-state reconcile.
   - Attach completion must be idempotent on re-entry, and `full_hash_reconcile` must run exactly once per remap/restore cycle.
   - RCRT must skip collections halted by reconcile-integrity state rather than bulldozing through blocked evidence.

## Mandatory test gate

Keep the proposed restore/recovery tests, but treat these as mandatory seams:

- `17.5ii4`, `17.5ii5`, `17.5jj`
- `17.5kk`, `17.5kk2`, `17.5kk3`
- `17.5ll`, `17.5ll2`, `17.5ll3`, `17.5ll4`, `17.5ll5`
- `17.5pp`, `17.5qq`, `17.5qq2`, `17.5qq3`, `17.5qq4`, `17.5qq6`, `17.5qq7`, `17.5qq8`
- `17.9`, `17.10`, `17.11`, `17.13`

And pull these into the credibility gate even if they were omitted from the initial batch list:

- **OR write-gate coverage** (`17.5qq12` or equivalent consolidated coverage)
- **RCRT skip-on-halt coverage** (`17.5oo2` or equivalent)

## Safe deferrals

Keep these out of Batch I:

- IPC hardening / socket-auth tests tied to proxying (`17.5ii10`–`17.5ii12`, task 11.9)
- Bulk UUID writeback live-owner behavior beyond the restore/remap seam (`17.5ii9`)

Those are real seams, but mixing them into Batch I would blur the restore/remap gate and make review less auditable.


# Nibbler — Vault Sync Batch I Re-Gate

Date: 2026-04-22
Change: `vault-sync-engine`
Batch: I

## Verdict

**APPROVE**

## Why this now clears

1. **Non-RCRT reopen/attach seam is acceptably closed for this slice.**
   - Offline restore now stops after Tx-B in `state='restoring'` with `needs_full_sync=1`.
   - Offline remap likewise leaves the collection blocked pending RCRT.
   - The attach-completion path is now centralized behind the RCRT-owned `complete_attach(...)` flow rather than being reopened directly by offline command paths.

2. **Ownership residue is no longer left behind outside `collection_owners` on unregister.**
   - Session teardown now clears the `collections.active_lease_session_id` and `collections.restore_lease_session_id` mirror columns when they point at the released session.
   - That removes the stale-owner residue that previously allowed persisted mirror state to outlive the authoritative owner row.

3. **Legacy compatibility writers now honor the OR write-gate.**
   - `ingest` and directory `import` both call the shared all-collections write interlock before mutation.
   - That closes the prior fallback path where restore/remap could be mid-flight while older ingest/import surfaces still wrote pages and raw-import state.

4. **The task ledger is now materially honest about proof scope.**
   - `9.5` is no longer claimed complete and explicitly says plain `gbrain collection sync <name>` remains deferred.
   - The offline restore note and `17.11` now state that CLI success does not prove writes reopened; end-to-end CLI→RCRT recovery remains deferred.
   - Only the now-supported `17.5kk2` and `17.5ll2` claims are checked complete in this slice.

## Boundary caveat

This approval is for the repaired Batch I boundary only. Plain `gbrain collection sync <name>` and the full offline CLI-to-RCRT integration proof remain explicitly deferred, so this is approval of the fail-closed seams now landed, not of those broader operator flows.


# Nibbler — Vault Sync Batch I Adversarial Review

Date: 2026-04-22
Change: `vault-sync-engine`
Batch: I

## Verdict

**REJECT**

## Blocking seam 1: non-RCRT reattach still exists

- `src/core/vault_sync.rs:955-960` calls `complete_attach(..., AttachReason::RestorePostFinalize)` directly from the offline restore path.
- `src/core/vault_sync.rs:1019-1024` calls `complete_attach(..., AttachReason::RemapPostReconcile)` directly from the offline remap path.

Why this blocks:

- Batch I was pre-approved only if reattach stayed RCRT-only.
- These direct calls clear `needs_full_sync` and flip the collection back to `active` without going through the RCRT recovery/reattach authority boundary.
- That makes the write-gate a command-local transition instead of the promised fail-closed barrier held until RCRT attach completion.

## Blocking seam 2: ownership truth still leaks into stale mirror state

- `src/core/vault_sync.rs:222-231` unregisters the session by deleting `collection_owners` and `serve_sessions`, but it does not clear `collections.active_lease_session_id` or `collections.restore_lease_session_id`.
- `src/core/reconciler.rs:481-510` authorizes full-hash restore/remap work from those persisted mirror columns.

Why this blocks:

- `collection_owners` is supposed to be the sole live ownership truth.
- Leaving the mirror columns behind after unregister means ownership residue can survive outside `collection_owners`.
- That is exactly the split-brain/stale-owner seam Batch I said it would close.

## Scope/truthfulness note

- The claimed supported tests include `17.5kk2` and `17.5ll2`, but `openspec/changes/vault-sync-engine/tasks.md:332` and `:335` still leave them unchecked.
- More importantly, the code already ships offline success-shaped restore/remap reattach behavior while `17.11` remains explicitly deferred from the landing claim.

## Narrowest correct repair route

1. Remove the direct offline `complete_attach(...)` calls.
2. Route all reopen/reattach through the RCRT path only (or a shared helper that is literally the RCRT authority path).
3. Clear stale lease mirror columns when sessions are released/unregistered, or stop using those mirror columns as standalone authorization truth.
4. Align `tasks.md` with the actual proof you are claiming before re-gating.


## Nibbler Review — Vault Sync Batch J

- Verdict: REJECT
- Date: 2026-04-23

### Concrete seam

`gbrain collection sync <name> --finalize-pending` currently renders success-shaped output for non-finalizing outcomes. In `src/commands/collection.rs`, the finalize branch always returns `render_success(...)` with `"status": "ok"` regardless of whether `finalize_pending_restore(...)` actually finalized the collection. But `src/core/vault_sync.rs` still returns blocked/non-final outcomes such as `Deferred`, `NoPendingWork`, `Aborted`, `ManifestIncomplete`, and `IntegrityFailed`.

### Why this blocks Batch J

The narrowed gate required truthful blocked-state diagnostics and no blocked path that looks successful. This seam lets operators or automation receive exit-0 / success JSON from `--finalize-pending` while the collection remains deferred or integrity-blocked, which violates the fail-closed requirement even though bare no-flag sync itself stays constrained to ordinary active-root reconcile.

### Correct repair route

Do not widen scope. Keep MCP widening, remap Phase 4, and destructive-path work deferred. Instead, route all non-finalizing `--finalize-pending` outcomes through truthful non-success CLI/JSON handling and add CLI-truth coverage proving deferred / manifest-incomplete / integrity-failed / aborted finalize states do not present as success.


# Decision: Batch K pre-implementation adversarial gate

**Author:** Nibbler  
**Date:** 2026-04-23  
**Session:** vault-sync-engine Batch K pre-gate

---

## Verdict

**REJECT** the proposed combined Batch K boundary.

## Narrower safer split

### Slice K1 — restore-integrity proof closure
- `1.1b`
- `1.1c`
- `17.5kk3`
- `17.5ll3`
- `17.5ll4`
- `17.5ll5`
- `17.5mm`

### Slice K2 — collection add + read-only operator surface
- `9.2`
- `9.2b`
- `9.3`
- `17.5qq10`
- `17.5qq11`
- `17.11`

`17.11` moves with K2 because its honesty depends on the real `collection add` path; do not use it to certify restore behavior before the add path itself is adversarially reviewed.

---

## Why the combined boundary is unsafe

`collection add` is not just scaffolding. It creates a new initial-owner acquisition path, a new add-time reconcile entrypoint, a capability-probe truth surface (`writable=0`), and a new refusal contract (`CollectionReadOnlyError`) that must be honored by every write path. That is broad operator-surface work.

The restore-integrity cluster is supposed to prove fail-closed recovery semantics around `pending_root_path`, manifest validation, and restore-command identity. Mixing that proof lane with a brand-new attach path lets the batch claim “offline restore is proven end-to-end” even if the scaffold path is still leaking lease residue, misclassifying writability, or leaving a collection in an owner state the restore tests never exercised honestly.

---

## Concrete adversarial seams that must be reviewed

1. **Restore-command identity theft**
   - Non-originator finalize must fail closed while heartbeat is fresh.
   - Exact persisted identity must be required; null, missing, or stale residue must never grant originator privileges.
   - Recovery callers must not inherit originator authority by reusing CLI/operator state.

2. **Manifest tamper / wrong-tree adoption**
   - Finalize must never succeed on path existence alone.
   - Any mismatch or incomplete manifest must block the collection, preserve pending state truth, and force `restore-reset` rather than soft-success.
   - Tests must prove tamper is caught before any success-shaped CLI output.

3. **Tx-B failure residue**
   - `pending_root_path` is durable authority after rename-before-finalize failure and must survive until explicit finalize or reset.
   - Generic recovery or reconcile workers must not clear, reinterpret, or overwrite this residue.
   - Operator surfaces must continue to point only at `sync --finalize-pending` or `restore-reset`, never plain sync.

4. **Initial add-time lease ownership**
   - The very first reconcile inside `collection add` must use the same short-lived `collection_owners` lease discipline as plain sync.
   - No row insertion before parse/probe failure; no lease residue after add abort; no duplicate owner truth between `collection_owners` and mirror columns.
   - Add-time reconcile must not create a bypass around serve-ownership refusal.

---

## Mandatory fail-closed behaviors

- Plain sync stays blocked for pending-finalize, integrity-failed, manifest-incomplete, and reconcile-halted states.
- Slug-bound `brain_gap` must take the write interlock; slug-less `brain_gap` must stay read-only and available during restore.
- Read-only probe failure may downgrade to `writable=0` only for plain attach without write-requiring flags.
- Once `writable=0` is persisted, every mutator must refuse before filesystem or DB mutation.
- Any restore finalize path with fresh originator heartbeat mismatch, manifest mismatch, or incomplete manifest beyond TTL must remain blocked and operator-visible.

---

## Mandatory proof tasks at implementation gate

- Successor identity mismatch test proving no finalize while heartbeat is fresh.
- Manifest mismatch test proving `IntegrityFailed` + `restore-reset` requirement.
- Missing-file retry-within-window and escalation-after-TTL tests.
- Tx-B failure residue test proving `pending_root_path` survives and plain/generic recovery does not consume it.
- Add-time lease test proving initial reconcile acquires and releases `collection_owners` correctly on success and abort.
- Read-only probe tests proving: (a) downgrade to `writable=0` with WARN for plain attach, (b) no row insert for write-required flags on RO roots, and (c) every mutator returns `CollectionReadOnlyError` fail-closed.
- `17.11` only after the above add-surface proofs land; otherwise it is a success-shaped restore claim built on untrusted scaffolding.

---

## New surfaces to review during implementation gate

- CLI truth for `collection add` / `collection list` / future `--recheck-writable` messaging.
- Partial-row residue when add fails after insert but before reconcile or after lease acquire but before release.
- Probe artifact residue (`.gbrain-probe-*`) and any accidental symlink-follow or wrong-root tempfile placement.
- Any attempt to fold `writable` checks into a subset of mutators instead of the shared write gate.


# Decision: Nibbler pre-implementation gate — vault-sync-engine Batch K2

**Author:** Nibbler  
**Date:** 2026-04-23  
**Requested by:** macro88

---

## Verdict

**APPROVE** the proposed K2 boundary **as one slice**, but only as an inseparable **implementation + proof closure** batch.

Do **not** treat K2 as “mostly tests.” The current tree still has live honesty holes:

- offline `begin_restore()` does **not** persist `restore_command_id` and bypasses the identity-aware finalize helper by calling `run_tx_b()` directly (`src\core\vault_sync.rs`)
- `restore_reset()` still clears restore/integrity state unconditionally once `--confirm` is given (`src\core\vault_sync.rs`)
- RCRT currently distinguishes restore-vs-remap follow-up by the presence/absence of `pending_root_path` and `restore_command_id`, so an offline path that clears/bypasses those fields can be misclassified into the remap-shaped lane (`src\core\vault_sync.rs`)

That means the six deferred K2 tasks are tightly coupled. Shipping any subset would create success-shaped offline restore claims on untrusted state.

---

## Why K2 is safe as one slice now

K1 removed the attach-surface coupling risk. The remaining K2 work is one state-machine closure:

1. **Originator identity persistence**
2. **Finalize/reset truthfulness**
3. **Tx-B residue durability**
4. **Manifest retry/escalation/tamper behavior**
5. **Real offline CLI → RCRT end-to-end proof**

These are not independent seams. The proofs are only honest if the production state machine is corrected at the same time, and the production fixes are only safe if the proofs pin every blocked path fail-closed.

---

## Adversarial non-negotiables

1. **Offline restore must stop bypassing identity checks**
   - Offline restore must mint and persist `restore_command_id` before any post-rename/finalize path.
   - The offline path must use the same identity-aware finalize helper as the online path; no direct `run_tx_b()` shortcut.
   - A foreign successor with a fresh heartbeat mismatch must stay blocked.

2. **Finalize and reset must stay honest**
   - `sync --finalize-pending` may report success only for real finalize/orphan-recovery outcomes.
   - `restore-reset` must not be a generic “make it green” eraser. It must require the bounded blocked states K2 promises, not clear fresh pending/finalizing state indiscriminately.
   - `collection info` / CLI messaging must continue to point operators only at the real next action.

3. **Tx-B residue stays authoritative**
   - Rename-before-Tx-B failure must preserve `pending_root_path`.
   - Plain sync, generic recovery, and RCRT must not consume or reinterpret that residue as a remap-like attachable state.
   - Pending-finalize rows remain blocked until explicit finalize or bounded reset.

4. **Manifest retry/escalation/tamper stays fail-closed**
   - Missing-files within the retry window must stay visibly pending, not succeed optimistically.
   - TTL expiry must escalate to terminal `integrity_failed_at`.
   - Manifest mismatch/tamper must force integrity failure and a reset-required recovery path; no soft-success, no silent retry loop.

5. **End-to-end offline restore proof must be real**
   - `17.11` must exercise the real CLI restore path plus the real RCRT attach completion path.
   - It must prove no success-shaped claim before the attach/reopen boundary is actually crossed.
   - No fixture shortcut that seeds DB rows directly past the identity or pending-finalize seam.

---

## Required review seams

Mandatory implementation-gate review focus:

1. **`src\core\vault_sync.rs::begin_restore()`**
   - confirm offline path persists `restore_command_id`
   - confirm offline path no longer jumps straight to `run_tx_b()`

2. **`src\core\vault_sync.rs::finalize_pending_restore()`**
   - exact persisted originator identity only
   - fresh-heartbeat mismatch stays deferred/blocked
   - manifest incomplete vs TTL escalation vs tamper are distinct and durable

3. **`src\core\vault_sync.rs::restore_reset()`**
   - no unconditional clear of pending/integrity/originator fields outside the bounded reset contract

4. **RCRT / recovery classification**
   - restoring rows created by offline restore must not fall into the remap-post-reconcile signature accidentally
   - pending-finalize residue must remain the only authority until finalize/reset resolves it

5. **CLI truth surface (`src\commands\collection.rs`)**
   - blocked finalize/reset paths remain non-success
   - operator guidance names the real blocked condition and next command

---

## Mandatory proofs

- foreign restore-command identity cannot finalize while heartbeat is fresh
- exact originator identity can finalize its own pending restore
- Tx-B failure preserves `pending_root_path`, and generic recovery does not clear it
- manifest incomplete retries within window and escalates after TTL
- manifest tamper yields terminal integrity failure plus reset-required recovery
- real offline restore integration proves CLI restore → blocked/write-gated state → RCRT attach completion truthfully, with no premature success claim

---

## Bottom line

K2 is now the right next slice, **but only as a single honest closure batch**. If anyone tries to peel off the proofs from the production fixes, or lands identity persistence without the residue/tamper/end-to-end proof matrix, the slice becomes success-shaped and should be re-rejected.


# Nibbler — Vault Sync Batch K2 Review

## Verdict

APPROVE

## Why

The pre-gate seams for K2 are acceptably controlled in the current slice:

- **Originator identity theft:** `finalize_pending_restore()` only treats a caller as the restore originator when the persisted `restore_command_id` exactly matches the caller command id; any external finalize caller still defers while the heartbeat is fresh.
- **Reset/finalize dishonesty:** `restore-reset` now blocks unless `integrity_failed_at` is set, and `collection sync --finalize-pending` only succeeds for `Attached` / `OrphanRecovered`; deferred, manifest-incomplete, integrity-failed, aborted, and no-pending outcomes fail closed.
- **Tx-B residue authority:** `run_tx_b()` is still the sole authority that consumes `pending_root_path` and transitions to `state='restoring'` + `needs_full_sync=1`; generic recovery does not erase residue.
- **Manifest retry/tamper:** missing files mark `pending_manifest_incomplete_at` and retry within the window, while TTL escalation or mismatched bytes set `integrity_failed_at`; operator reset is blocked during retryable gaps and allowed only for terminal failure.
- **17.11 honesty:** the completion proof is a real CLI path (`collection restore` → `collection sync --finalize-pending`) using `finalize_pending_restore_via_cli()` plus `complete_attach()`. It does not require serve startup, RCRT ownership, or online handshake topology.

## Required caveat

This approval covers **only** the K2 offline-restore integrity slice and its explicit CLI finalize path. It does **not** approve deferred startup-recovery/orphan-finalization topology (`17.5ll`, `17.13`), online restore handshake (`17.10`), or broader destructive restore/remap surfaces; the task ledger must keep those items explicitly deferred.


# Nibbler pre-gate — vault-sync Batch L

Date: 2026-04-23
Verdict: REJECT

## Why reject the proposed Batch L boundary

Batch L mixes two different recovery authorities:

1. **Restore orphan startup recovery** (`17.5ll`, `17.13`) — RCRT deciding whether a pending restore is truly orphaned and may be finalized at serve startup.
2. **Crash-mid-write sentinel recovery** (`11.4`) — startup/generic recovery consuming `brain_put` durability sentinels and reconciling dirty vault bytes.

Those are not the same seam. Bundling them creates a success-shaped recovery claim across both restore and ordinary write recovery without one narrow proof boundary.

Optional `2.4a2` is orthogonal platform gating and should not ride with startup-recovery truth.

## Concrete adversarial seams

### 1) Premature RCRT firing while the originator is only slow

- RCRT must not finalize while `pending_command_heartbeat_at` is still fresh.
- A fresh serve must not reinterpret “slow between rename and Tx-B” as “dead.”
- Proof must show startup recovery stays blocked for a live/slightly delayed originator and only proceeds once the stale/dead condition is real.

### 2) Stale `serve_sessions` residue

- Live-owner truth must continue to come from `collection_owners`, never ambient `serve_sessions`.
- Startup sweep/reclaim must not let stale rows, fresh-but-foreign rows, or owner changes satisfy the recovery lane.
- Recovery must fail closed if ownership changed or liveness is ambiguous.

### 3) Sentinel recovery reader correctness

- Sentinel scan must be collection-scoped, parse strictly, and never clear a dirty signal on malformed, unreadable, or partially processed sentinel state.
- Sentinel unlink must happen only after the intended reconciliation succeeds.
- A bad sentinel must leave the collection dirty/error-shaped, not silently “healed.”

### 4) Startup/orphan recovery widening into broader post-Tx-B topology

- The restore startup proof must not silently certify generic `needs_full_sync=1` attach, remap attach, or broader post-Tx-B auto-heal behavior.
- `17.13` must prove the exact restore-orphan signature only, not “startup recovery works” in general.
- Operator surfaces must remain explicit that this slice does **not** prove watcher-overflow recovery, remap startup completion, or generic active-reconcile startup attach.

## Required fail-closed behaviors

- Fresh originator heartbeat => RCRT returns blocked/deferred, not success.
- Missing/ambiguous owner truth => no finalize, no attach, no write reopen.
- Malformed/unreadable sentinel => dirty signal remains; startup must not claim convergence.
- Integrity-blocked / manifest-incomplete / reconcile-halted states => startup recovery does not reopen writes.
- Success reporting only after the real startup path proves both finalize and attach on the exact restore-orphan lane.

## Mandatory proofs before approval

1. **Slow-not-dead proof**: originator heartbeat fresh at serve startup => no RCRT finalize.
2. **Dead-orphan proof**: originator gone/stale after rename-before-Tx-B => next serve start finalizes exactly once as startup recovery.
3. **Foreign-owner/stale-row proof**: foreign or stale `serve_sessions` residue cannot satisfy recovery.
4. **Sentinel strictness proof** (if sentinel work is included): malformed/unreadable sentinel leaves collection dirty and not success-shaped.
5. **No-broadening proof**: restore startup recovery claim excludes remap/generic `needs_full_sync` attach lanes.

## Safer split

### Batch L1 — restore-only startup recovery

- `17.5ll`
- `17.13`
- only the minimal `11.1` registry work needed for RCRT/supervisor startup state

Why safe: one authority boundary, one recovery story, one honest proof target: “originator dies before finalize; next serve start recovers the restore.”

### Batch L2 — sentinel startup recovery

- `11.4`
- `17.12`
- the sentinel-directory portion of `11.1` if still needed

Why separate: this is a different dirty-signal surface with different failure modes, parser correctness requirements, and cleanup rules. It should land only with the crash-mid-write end-to-end proof.

### Keep out of both

- `2.4a2` optional Windows platform gating

Reason: unrelated truth surface; bundling it only dilutes review and invites accidental “recovery slice complete” language.


# Nibbler — Vault Sync Batch L1 Final Review

**Date:** 2026-04-23  
**Verdict:** APPROVE

## Why approve

The adversarial seams named at pre-gate are acceptably controlled for the narrowed L1 slice:

1. **Fresh-but-slow originator stays protected.**
   - `finalize_pending_restore(..., FinalizeCaller::StartupRecovery { .. })` still defers on a fresh `pending_command_heartbeat_at` unless the exact persisted `restore_command_id` matches.
   - The direct heartbeat-threshold tests pin the shared 15-second rule, and the startup test proves the collection stays blocked instead of being finalized early.

2. **Stale / foreign `serve_sessions` residue does not become ownership truth.**
   - Startup now does the order explicitly: stale-session sweep, register own serve session, claim collection ownership, then RCRT.
   - Recovery authority remains collection-scoped through `collection_owners`; ambient foreign `serve_sessions` rows are not sufficient to authorize recovery.

3. **Premature RCRT firing is constrained by real startup ordering.**
   - `start_serve_runtime()` performs registry init and ownership claim before the startup RCRT pass.
   - The stale-orphan integration proof shows exact-once startup finalize on the restore-owned pending-finalize lane before supervisor bookkeeping, with no supervisor-ack residue left behind.

4. **Success-shaped lies are avoided on the L1 path.**
   - Fresh-heartbeat startup leaves the collection in a blocked restoring state with pending fields intact.
   - The task ledger now splits `11.1a` from deferred `11.1b`, keeps `11.4` and `17.12` open, and therefore does not falsely claim sentinel recovery closure.

## Required caveat that must stay attached

This approval covers **only** restore-orphan startup recovery for restore-owned pending-finalize state (`11.1a`, `17.5ll`, `17.13`). It must **not** be cited as proof of sentinel recovery, generic `needs_full_sync` startup healing, remap startup attach, or any broader claim that “serve startup heals dirty collections.”


## 2026-04-24: Vault-sync-engine Batch 13.3 review
**By:** Professor
**What:** REJECT Batch 13.3 in its current form.
**Why:** The implementation mostly keeps CLI slug-bearing surfaces aligned with MCP collection-aware resolution and canonical `<collection>::<slug>` output, and the assertion UUID fallback stays acceptably narrow as duplicate-slug safety. But `gbrain unlink` still emits `no matching link found between {from} and {to}` using raw user inputs after both pages have already been resolved collection-aware, so CLI parity is not yet complete.
**Required follow-up:** Canonicalize that resolved failure path (and any equivalent already-resolved CLI page-reference output) before claiming `13.3` closed. When updating `tasks.md`, keep the closure note explicitly limited to CLI slug-routing/output parity only; do not imply `13.5` collection filter/default behavior or broaden the assertions fix beyond duplicate-slug safety.


# Professor — Batch 13.6 / 17.5ddd review

- **Verdict:** REJECT
- **Scope reviewed:** `brain_collections` MCP read surface only (`src/core/vault_sync.rs`, `src/mcp/server.rs`) against `openspec\changes\vault-sync-engine\design.md` and collection specs.

## Why rejected

1. **Schema truth predicate drift on `integrity_blocked`.**
   - Design/spec freeze `duplicate_uuid` and `unresolvable_trivial_content` behind `reconcile_halted_at IS NOT NULL AND reconcile_halt_reason = ...`.
   - Implementation derives those values from `reconcile_halt_reason` alone in `integrity_blocked_label(...)`, and `list_brain_collections()` does not even load `reconcile_halted_at`.
   - Result: the MCP tool can present a terminal blocking state from stale or partial reason metadata, which violates the "truthful masking/presentation" requirement for this slice.

## What is acceptable in the slice

- The surface stays read-only.
- The field set itself matches the frozen 13-field schema.
- Root masking (`root_path = null` when non-active) and tagged `ignore_parse_errors` shaping are aligned.
- Focused `brain_collections` tests passed locally.

## Required fix before approval

- Load `reconcile_halted_at` in the collection query and gate `duplicate_uuid` / `unresolvable_trivial_content` on the full frozen predicate, then keep the existing read-only boundary intact.


# Professor — N1 MCP slug-routing truth review

Date: 2026-04-24
Branch: `spec/vault-sync-engine`
Batch: **N1 — MCP slug-routing truth**
Task IDs reviewed: `13.1`, `13.2`, `13.4`

## Verdict

**REJECT**

## Why

The MCP truth slice itself is technically sound:

- slug-bearing MCP handlers resolve through collection-aware parsing
- MCP outputs that expose page identifiers now emit canonical `<collection>::<slug>` values
- ambiguity errors expose a stable payload with machine-readable `code` plus `candidates[]`

But the diff is not scope-disciplined enough to land as an MCP-only batch.

## Blocking seam

`src/commands/check.rs` now changes shared CLI behavior, not just MCP behavior:

- single-page `check` now resolves via `resolve_slug_for_op(...)`
- contradiction filtering for the single-page path now keys by page id rather than the original slug string

That is real CLI parity drift on a surface explicitly deferred in this batch (`13.3`). Reusing a shared helper does not make the widening disappear.

## Required repair

Keep the MCP `brain_check` fix, but isolate it from the user-facing CLI `check` contract for this batch. If the implementation needs shared lower-level helpers (for example page-id based assertion/check routines), that is fine; what cannot land under N1 is silently broadening CLI slug-routing semantics while the batch claims MCP-only truth.

## Validation

- `cargo test --quiet --lib mcp::server` passed locally (`89 passed`)

## Wording caveat if resubmitted

If this slice is narrowed and resubmitted, keep the wording exact:

- `13.1` = slug-bearing **MCP handlers** only
- `13.2` = MCP responses that explicitly emit page identifiers, not arbitrary markdown/body text
- `13.4` = stable MCP ambiguity payload shape only; no CLI parity implication


# Professor — Quarantine Restore Pre-Gate

**Date:** 2026-04-25  
**Verdict:** APPROVE THE NARROW SLICE TO START

## Current stop-point

The current tree is truthful: `openspec\changes\vault-sync-engine\tasks.md` reopens `9.8` and `17.5j`, `src\commands\collection.rs` hard-refuses `collection quarantine restore`, and both `tests\quarantine_revision_fixes.rs` and `tests\collection_cli_truth.rs` lock the deferred surface as "no vault-byte or DB mutation while restore is disabled."

The blocking history is also clear:

1. Fry's original quarantine slice was rejected on four production seams (failed export unlocking discard; non-Markdown restore targets; live-owner bypass; file-on-disk while DB stayed quarantined).
2. Mom's revision addressed those four, but Leela's third revision found two more restore blockers in the still-live body: read-only vault-byte gating and post-rename residue.
3. Bender then backed restore out entirely after the final two safety blockers remained: **post-unlink cleanup was not crash-durable** and **install still used pre-check + replace-prone rename semantics**.

## Minimum acceptable contract for the re-opened slice

Fry may implement **only** the narrow restore re-enable slice for `9.8` (restore arm only) + `17.5j`.

### Non-negotiable success contract

On success, restore must leave exactly one stable outcome:

- raw bytes are installed at the requested Markdown target inside the collection root
- the install is durable at the directory level (parent fsynced after install)
- `pages.quarantined_at` is cleared
- `file_state` is reactivated for that page at the restored relative path
- no restore temp residue remains

### Non-negotiable failure contract

On any failure, restore must leave exactly one stable outcome:

- the page remains quarantined
- no restored target path is left behind
- no restore temp residue is left behind
- no `file_state` row is reactivated

No "best effort" cleanup story is acceptable if it can still leave durable bytes behind while the DB says quarantined.

### Blocker 1 — post-unlink parent-fsync durability

If the restore path ever installs bytes and then rolls back by unlinking a target/temp entry, that cleanup is not complete until the **same parent directory is fsynced after the successful unlink**. This is mandatory on every post-install unlink path. Returning after `unlink` without parent fsync is still rejectable, because recovery can observe resurrected residue after power loss.

### Blocker 2 — no-replace install semantics

The final install step must enforce target absence **at install time**, not only at an earlier pre-check. A plain "target absent, then rename" sequence is not approvable. Fry may add a narrow helper to do the final same-directory install with no-replace semantics, but the contract is fixed: a concurrently-created target must win, and restore must fail closed without clobbering it.

## What Fry may implement now

- Re-enable `gbrain collection quarantine restore` on the existing CLI surface only
- Keep the current Unix/offline/write-gate/ownership/Markdown-target boundaries
- Add the smallest helper(s) needed for:
  - install-time no-replace behavior
  - durable post-unlink cleanup
  - deterministic failure injection/proof if needed for tests
- Land the real `17.5j` restore happy path

## What remains deferred

- `quarantine audit`
- any overwrite policy beyond strict no-replace refusal
- export-conflict policy widening
- watcher mutation handlers
- IPC / live write routing / online restore handshake
- UUID write-back
- broader remap / background restore / recovery-worker work

Do not smuggle any of that into this batch under the label "restore fix."

## Mandatory proof before I would approve landing

1. **Happy-path integration proof (`17.5j`)**  
   Restore a quarantined page, prove the exact restored page is no longer quarantined, and prove `file_state.relative_path` is active at the restored Markdown path.

2. **Existing refusal proofs must stay real**  
   Retain/update focused tests for:
   - non-Markdown target refusal
   - live-owned collection refusal
   - read-only collection refusal
   - existing-target refusal  
   All must prove no vault bytes or DB reactivation on failure.

3. **Direct no-replace race proof**  
   A dedicated test must prove that a target created after any earlier absence check is **not overwritten** by restore. A mere "target already existed before command start" test is insufficient.

4. **Direct post-install cleanup durability proof**  
   A dedicated test (integration with injection or helper-level proof) must force a failure after install begins and prove the rollback path unlinks residue and fsyncs the parent before returning. I will not accept a comment-only or reasoning-only argument here.

5. **Targeted regression run stays green**  
   At minimum: `quarantine_revision_fixes` and the quarantine restore CLI truth test.

## Start / no-start decision

**Yes, this slice may start now** — but only under the contract above. If Fry cannot prove install-time no-replace behavior and fsync-after-unlink rollback durability in code and tests, restore must remain deferred.


# Professor Gate — Vault Sync Batch H

Date: 2026-04-22
Change: `openspec/changes/vault-sync-engine`
Batch: H (`5.4h`, `5.8a0`, `5.8a`, `5.8a2`, `5.8b`, `5.8c`, `5.8d`, `5.8d2`, `5.9` + listed tests)
Verdict: **APPROVE**

Fry can start implementation **only inside this boundary**:

- Batch H is a **pre-destruction safety slice** plus fresh-attach wiring.
- It may land shared Phase 0–3 restore/remap safety helpers and `full_hash_reconcile` call-site wiring.
- It must **not** claim end-to-end remap completion, Phase 4 new-root verification, or the final restore/remap execution/orchestration tasks (`5.8e`, `5.8f`, `5.8g`), which remain deferred.

## 1) Contract constraints Fry must implement

### A. Restore/remap pipeline phases in this slice (0 through 3 only)

#### Phase 0 ordering
1. **`5.8a0` UUID-migration preflight runs first, before any filesystem gate.**
2. **`5.8a` RO-mount/quiescence gate runs second.**
3. **`5.8a2` dirty/sentinel preflight runs third.**
4. Only after all three pass may Phase 1 begin.

Allowed pre-Phase-1 mutation is limited to handshake/state-prep needed to take ownership. No Tx-A, no root swap, no `file_state` delete, no write-gate reopen.

#### Phase 1 — drift capture
- This is the **only** phase in the pre-destruction pipeline allowed to mutate page content state before the destructive step.
- It must reuse the existing `full_hash_reconcile` apply path so raw-import rotation, page updates, and invariant checks stay identical to already-approved reconcile behavior.
- **Restore:** non-zero drift is acceptable and becomes the updated authoritative DB state for later restore staging.
- **Remap:** the same pass may run, but **the caller** must treat any material drift as refusal, not success.

#### Phase 2 — stability
- Must compare two successive **stat-only** snapshots of the old root after Phase 1 completes.
- If snapshots differ, rerun Phase 1 before retrying stability.
- Retrying is bounded by `GBRAIN_RESTORE_STABILITY_MAX_ITERS`; exhaustion returns `CollectionUnstableError`.

#### Phase 3 — pre-destruction fence
- One final stat-only fence snapshot is compared against the final stable snapshot.
- Any diff aborts through the standard resume path and returns `CollectionUnstableError`.

#### TOCTOU dirty recheck (`5.8d2`)
- This is **after** Phase 3 passes and **immediately before** any destructive step.
- It must use a **fresh SQLite connection** for `is_collection_dirty` and a fresh sentinel-directory scan.
- Positive result aborts with `CollectionDirtyError`.

### B. `synchronous_drift_capture` / `full_hash_reconcile` bypass authorization surface

The current closed authorization contract was the right direction in Batch G; Batch H may extend it, but not weaken it.

Required constraints:

1. **No boolean bypass.** No `allow_non_active`, `skip_state_check`, or similar.
2. The mode remains a **closed enum** and gets an explicit drift-capture extension:
   - either a distinct `SynchronousDriftCapture` mode with operation kind (`restore` vs `remap`),
   - or two explicit modes (`RestoreDriftCapture`, `RemapDriftCapture`).
3. Authorization must carry **caller identity**, not just caller class:
   - restore path authorized by the current `restore_command_id`;
   - online/offline remap path authorized by the owning lease/session identity.
4. Validation must fail closed **before** opening the root or walking files.
5. `full_hash_reconcile` itself returns stats/errors only; it does **not** decide whether non-zero drift is acceptable. That decision stays in the restore/remap caller.

In short: the bypass is legal only for a caller that can prove “I am the command/session that owns this destructive pipeline right now.”

### C. RO-mount, dirty/sentinel, drift-capture, stability, fence, TOCTOU recheck ordering

The required order is:

`UUID-migration preflight` → `RO-mount/quiescence` → `dirty/sentinel preflight` → `Phase 1 drift capture` → `Phase 2 stability` → `Phase 3 fence` → `TOCTOU dirty recheck` → **then and only then** destructive execution (still deferred outside this batch).

Two non-negotiables:

- **No staging rename / Tx-A / remap DB root switch before TOCTOU recheck passes.**
- **No write-gate reopen** until the later attach-completion path finishes `full_hash_reconcile` and clears `needs_full_sync`.

### D. `5.9` fresh-attach usage of `full_hash_reconcile`

Fresh-attach must be wired as a **real full-hash attach path**, not a stat-diff shortcut.

Required constraints:

1. First attach after detach and first-use-after-detach both invoke `full_hash_reconcile`.
2. They use `FullHashReconcileMode::FreshAttach` (or equivalent closed-mode name), **not** the drift-capture mode.
3. Authorization is attach-specific only; it must not reuse the active-lease bypass intended for synchronous drift capture.
4. Writes remain blocked until attach completion clears `needs_full_sync`.
5. Batch H must not claim watcher/supervisor attach choreography complete unless the actual call site exists and the write-gate sequencing is honest.

### E. `5.4h` `InvariantViolationError` / `--allow-rerender` hook boundaries

This boundary must stay narrow.

1. Library/core reconcile code continues to surface **typed `InvariantViolationError`**.
2. `full_hash_reconcile` does **not** gain a generic “rerender instead” branch.
3. `--allow-rerender` is an **operator CLI restore override only**.
4. No silent fallback is permitted for audit, watcher recovery, remap, or fresh-attach.
5. If the override is used, it must be audit-visible (`WARN`) and leave the invariant breach explicit rather than pretending the DB healed itself.

That keeps the destructive recovery escape hatch above the reconciler seam and prevents passive background code from masking corruption.

## 2) Minimum `tasks.md` wording fixes

These are the minimum wording repairs needed before or during implementation:

1. **`5.8b` mode wording**
   - Replace the pseudo-signature `mode=synchronous_drift_capture` with wording that matches the actual closed enum/API Fry lands.
   - Also state explicitly that authorization is by **lease/session identity or `restore_command_id`**, not a raw state bypass.

2. **`5.8b` remap error counts**
   - The task currently invents `pages_updated/pages_added/pages_quarantined`.
   - Either align the text to the existing reconcile stats names, or define a dedicated remap-conflict summary type. Do not leave review guessing how counts map.

3. **`5.8a0` trivial-content predicate**
   - The task must say this uses the **same canonical trivial-content helper/predicate** as the `5.3` hash-rename guard.
   - No duplicate threshold logic, no second copy of the 64-byte/body-after-frontmatter rule.

4. **`5.9` fresh-attach wording**
   - Expand it from “invoke `full_hash_reconcile`” to “invoke `full_hash_reconcile` in fresh-attach mode before clearing `needs_full_sync` / reopening writes.”

5. **Batch-boundary honesty**
   - Add or preserve a note that Batch H lands **Phase 0–3 helpers and fresh-attach wiring only**; Phase 4/remap verification and full restore/remap execution remain deferred.

## 3) Reviewer rationale

I am approving because the batch boundary is coherent **if** it stays explicit:

- Batch G already established the right shape: closed-mode authorization, metadata-only unchanged-hash path, and fail-closed invariant errors.
- Batch H can build safely on that by adding the pre-destruction guardrail phases and fresh-attach wiring **without** pretending remap execution is complete.
- The two high-risk seams are known and containable:
  - the drift-capture bypass must be tied to explicit caller identity;
  - the UUID-migration trivial-content check must share the exact rename-guard predicate.

Nibbler’s adversarial review should still be required before merge on those two seams.


# Professor Re-Gate — Vault Sync Batch H

Date: 2026-04-22
Change: `openspec/changes/vault-sync-engine`

## Verdict

APPROVE

## Why

The prior blocker is fixed narrowly and in the right seam. Restore/remap full-hash authorization now binds the presented `restore_command_id`, `restore_lease_session_id`, or `active_lease_session_id` to persisted collection ownership state loaded from `collections`, and the check runs before root open or filesystem walk. Missing caller identity, missing persisted owner identity, wrong caller class, and owner mismatch all fail closed.

The repair stays inside the accepted Batch H boundary. `FreshAttach` remains attach-command scoped instead of borrowing the destructive bypass, the Phase 0–3 / fresh-attach task notes remain honest, and no new generic bypass or policy sprawl was introduced.

## Validation

- `cargo test full_hash_reconcile --lib`
- `cargo test full_hash_reconcile --lib --no-default-features --features bundled,online-model`

Both lanes passed, and the targeted tests now cover:

- restore-command match vs mismatch
- restore-lease match vs mismatch
- active-lease remap match vs mismatch
- wrong caller-class rejection
- fresh-attach separation


# Professor Review — Vault Sync Batch H

Date: 2026-04-22
Change: `openspec/changes/vault-sync-engine`
Reviewer: Professor
Verdict: **REJECT**

## Narrow blockers

1. **Drift-capture / full-hash authorization is not actually keyed to owner identity.**
   - `src/core/reconciler.rs` introduces closed enums for mode and authorization, but `authorize_full_hash_reconcile()` only checks:
     - mode/authorization variant pairing
     - collection state
     - non-empty identity string
   - It does **not** compare the supplied `restore_command_id` or `lease_session_id` against persisted ownership state.
   - Any non-empty restore token or lease token of the right variant currently authorizes the bypass.
   - That fails the pre-implementation gate requirement that the bypass be legal only for the command/session that currently owns the destructive pipeline.

2. **Coverage does not defend the missing identity-match contract.**
   - Added tests prove wrong-variant rejection and empty-identity rejection.
   - They do **not** prove “wrong token of the right class is refused,” because the implementation has no such check.
   - For this slice, that is not a test gap alone; it confirms the underlying contract is still open.

## What is good

- Batch H otherwise stays disciplined about the intended slice:
  - public restore/remap helper currently lands only Phase 0–3 + TOCTOU recheck, with destructive execution still deferred
  - UUID-migration preflight reuses the canonical trivial-content predicate shape
  - fresh-attach uses a dedicated `FreshAttach` mode and clears `needs_full_sync` only after the full-hash call succeeds
  - `tasks.md` is substantially more honest about deferred Phase 4 / orchestration work

## Next authoring route

- Because Professor issued the rejection, **Fry or Leela** should take the follow-up authoring pass.
- Required repair:
  1. persist/load the authoritative owner identity needed for this seam (`restore_command_id`, owning lease/session identity, or equivalent already-approved source of truth)
  2. make `authorize_full_hash_reconcile()` compare the supplied identity to that persisted owner identity and fail closed on mismatch
  3. add direct tests for mismatched-but-non-empty restore/lease identities
  4. re-check any Batch H wording that still reads like generic authorization instead of owner-bound authorization

## Review conclusion

Batch H is close, but this seam is not cosmetic: the current implementation proves caller class, not caller identity. That is below the quality bar for a pre-destruction safety bypass, so the slice should not land until the authorization contract is truly closed.


# Professor — Vault Sync Batch I Pre-Implementation Gate

## Verdict

**APPROVE** the proposed Batch I boundary, with non-negotiable constraints.

## Why this boundary is acceptable

Batch H deliberately stopped at the pre-destruction safety core. The next honest slice is the orchestration layer that makes that safety core real in command paths: Phase 4 remap verification, online/offline restore-remap execution, ownership lease, RCRT recovery, and the write interlock. Splitting `5.8e/5.8f/5.8g` away from `9.5/9.7*` and `11.2/11.3/11.5/11.6/11.7/11.7a/11.8` would create either dead helpers or a misleading "restore works" claim without the runtime ownership contract that makes it safe.

## Non-negotiable implementation constraints

1. **`collection_owners` is the sole ownership truth.**
   - Restore/remap coordination must resolve the owner from `collection_owners`, not from arbitrary live `serve_sessions`.
   - Offline paths must hold the lease with heartbeat for the full destructive window.

2. **Handshake acceptance is exact-match only.**
   - Online restore/remap may proceed only after ack matches the captured `(session_id, reload_generation)` pair.
   - Stale acks, foreign-session acks, or ownership changes mid-handshake must abort.

3. **Supervisor release and reattach responsibilities must stay separated.**
   - The live supervisor releases watcher/root state, writes the ack, and exits.
   - RCRT is the only runtime actor that reattaches, reconciles, clears `needs_full_sync`, and respawns supervision.

4. **`run_tx_b` is the only finalize path.**
   - Happy-path restore, startup recovery, and `sync --finalize-pending` must all route through the same canonical finalize helper.
   - No inline SQL variant may partially clear pending state.

5. **Writes stay blocked until attach completion, not command success.**
   - Mutations must refuse whenever `state='restoring'` **or** `needs_full_sync=1`.
   - This gate must cover CLI and MCP mutators, including admin-like paths that do not edit page bytes directly.

6. **Remap and restore must remain behaviorally distinct after the shared safety phases.**
   - Restore uses Tx-A / rename / manifest revalidation / Tx-B.
   - Remap does not adopt a new root by relative-path trust; it requires Phase 4 bijection and exactly one post-release full-hash reconcile before state flips active.

7. **Identity recovery stays read-only in this batch.**
   - This batch may recover ownership and page identity through UUID-first reconciliation and explicit reset/recovery commands.
   - It must not smuggle in new user-byte rewrite surfaces (`migrate-uuids`, bulk frontmatter repair, generalized write-back).

8. **Reset commands are recovery valves, not cleanup shortcuts.**
   - `restore-reset` / `reconcile-reset` may clear blocked state only after surfacing why the collection halted.
   - They must not silently adopt a pending target or erase evidence needed to explain integrity failure.

## Reviewability and truthfulness requirements

- Keep the batch scoped to restore/remap orchestration + ownership recovery only. Do **not** mix in unrelated collection CLI expansion, UUID migration, watcher product polish, or delete/purge work.
- Task and spec text must say plainly that command success can still mean "reattach pending under RCRT" until `needs_full_sync` clears.
- Review should be organized around three auditable seams:
  1. lease + handshake ownership contract,
  2. finalize/recovery state machine,
  3. write-gate + post-attach reopen.

## Test-gate ruling

The proposed test cluster is broadly the right set. Do **not** defer the core recovery proofs:
- `17.5ii4`, `17.5ii5`, `17.5jj`
- `17.5kk*`, `17.5ll*`
- `17.5pp`, `17.5qq*`
- `17.9`, `17.10`, `17.11`, `17.13`

If trimming is needed for auditability, trim only duplicate expression of the same seam, not the seam itself. In particular, "exactly once" and re-entry behavior (`17.5qq6`, `17.5qq8`) may be proven by tight deterministic tests instead of heavyweight end-to-end orchestration, but they remain mandatory behaviors.

## Mandatory adversarial review

**Nibbler review is required at implementation gate time.**

Target seams:

1. owner resolution from `collection_owners` versus accidental acceptance of non-owner `serve_sessions`;
2. stale/foreign ack acceptance in the `(session_id, reload_generation)` handshake;
3. originator-bypass versus non-originator finalize in `finalize_pending_restore`;
4. duplicate attach / double reconcile / generation bump drift in RCRT handoff;
5. post-Tx-B and post-remap write-gate holes;
6. reset-command misuse that clears blocked state without preserving operator truth.

## Batch boundary ruling

Batch I is the right next slice **only if** it lands as a full orchestration-and-recovery batch. If implementation pressure forces a split, the only safe split is to keep `5.8e/5.8f/5.8g`, `9.5`, `9.7*`, and `11.2/11.3/11.5/11.6/11.7/11.7a/11.8` together and defer any nonessential collection-surface polish around them.


# Professor — Vault Sync Batch I Re-gate

Date: 2026-04-22
Change: `vault-sync-engine`
Batch: I

## Verdict

**APPROVE**

## Why this repaired slice now clears

1. **The legacy write-gate hole is actually shut.**
   - `src/commands/ingest.rs` now calls `ensure_all_collections_write_allowed()` before any DB mutation.
   - `src/core/migrate.rs::import_dir()` does the same in write mode, which closes the `gbrain import` compatibility path too.
   - Both seams now have direct refusal tests proving no page rows land while restore/full-sync state is active.

2. **Reopen remains RCRT-only, not command-local.**
   - Offline `begin_restore(..., false)` and `remap_collection(..., false)` no longer call `complete_attach(...)`.
   - They stop after Tx-B / pending full-sync state, release their temporary lease/session, and leave `state='restoring'` + `needs_full_sync=1` in place until RCRT owns attach.
   - `complete_attach(...)` is now only exercised from `run_rcrt_pass(...)`, preserving the pre-gate “writes reopen only after RCRT attach completion” contract.

3. **Ownership truth is singular again.**
   - `unregister_session()` now clears `collections.active_lease_session_id` and `collections.restore_lease_session_id` alongside deleting `collection_owners` / `serve_sessions`.
   - That removes stale mirror residue from the authorization seam and keeps `collection_owners` as the sole live ownership truth rather than leaving split-brain leftovers behind.

4. **`tasks.md` is now honest about the shipped surface and deferred proof.**
   - `9.5` is no longer overstated: plain `gbrain collection sync <name>` remains unchecked and explicitly called out as still hard-erroring.
   - Offline restore is described truthfully as reaching Tx-B and then waiting for RCRT; `17.11` remains deferred rather than quietly implied complete.
   - The only claimed proof points promoted to complete here (`17.5kk2`, `17.5ll2`) are the ones directly supported by code and tests.

## Validation read

- `cargo test --quiet` ✅
- `GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model` ✅

## Required caveat

Batch I is ready to land **with the explicit caveat that end-to-end CLI→RCRT restore proof remains deferred**. Command success on offline restore/remap still does not mean writes have reopened; that remains a later integration claim, and the ledger now says so plainly.


# Professor — Vault Sync Batch I Review

Date: 2026-04-22
Change: `vault-sync-engine`
Batch: I

## Verdict

**REJECT**

## Narrow blockers

1. **Write-gate contract is not actually closed across legacy mutators.**
   - `src/commands/ingest.rs` writes `pages`/`raw_imports`/`ingest_log` without any `ensure_collection_write_allowed()` or `ensure_all_collections_write_allowed()` check.
   - `src/commands/import.rs` and `src/core/migrate.rs::import_dir()` likewise perform bulk page/raw-import writes without the Batch I `state='restoring' OR needs_full_sync=1` interlock.
   - That reopens exactly the class of fallback write surface Batch I was supposed to close: restore/remap can be in-flight while compatibility commands still mutate the brain.

2. **Task 9.5 is overstated versus the shipped operator surface.**
   - `openspec/changes/vault-sync-engine/tasks.md` marks `9.5` complete and says `gbrain collection sync <name>` runs `stat_diff + reconciler`.
   - The actual CLI in `src/commands/collection.rs` rejects plain sync with `collection sync currently requires --remap-root or --finalize-pending`.
   - For a safety-critical batch, a checked recovery/sync task cannot claim a live operator path that still hard-errors.

## Why the rest is close

- Ownership truth is correctly centered on `collection_owners`, not arbitrary `serve_sessions`.
- Ack acceptance is fail-closed on exact `(session_id, reload_generation)` match and rejects foreign/stale/replayed writes.
- `run_tx_b` is the canonical finalize SQL path, and RCRT remains the only runtime reattach actor.
- The touched CLI/MCP mutators (`put`, `check`, `link`, `tags`, `timeline`, `brain_raw`, slug-bound `brain_gap`) do carry the OR write-gate and have direct seam coverage.

## Correct repair route

- **Leela (or another repair-pass integrator), not Fry, should do the non-author repair.**
- Required repair is narrow:
  1. put the same Batch I write interlock in legacy compatibility writers (`ingest`, `import` / `import_dir`) before any DB or FS mutation;
  2. either implement the ordinary `collection sync <name>` path honestly enough to satisfy `9.5`, or mark `9.5` back to deferred/partial and remove the overclaim.

## Scope caveat if resubmitted

The batch can be re-submitted without pulling in deferred adversarial/e2e proofs, but only after the write-surface hole is shut and the task ledger/operator surface tell the same truth.


# Professor — Vault Sync Batch J Review

Date: 2026-04-23
Change: `vault-sync-engine`
Batch: J

## Verdict

**REJECT**

## Narrow blocker

1. **`sync --finalize-pending` still presents blocked outcomes as success.**
   - In `src/commands/collection.rs`, the `finalize_pending` branch always returns `render_success(...)` with `"status": "ok"` after calling `vault_sync::finalize_pending_restore(...)`.
   - But `src/core/vault_sync.rs` still returns non-finalizing outcomes (`Deferred`, `ManifestIncomplete`, `IntegrityFailed`, `Aborted`, `NoPendingWork`) for exactly the blocked restore states this narrowed review required to stay fail-closed and truthful.
   - That means the CLI can emit a success-shaped finalize result while the collection is still restoring or integrity-blocked. The no-flag plain-sync path itself is correctly constrained to active-root reconcile, but the adjacent `collection sync` recovery surface still lies about blocked state.

## Why the rest is close

- Bare `gbrain collection sync <name>` is correctly isolated to the active-root reconcile path and does not silently remap or finalize.
- Lease acquisition / heartbeat / cleanup are coherent and directly covered.
- Duplicate-UUID and trivial-content ambiguity now persist terminal reconcile halts and do not self-heal through plain sync.
- `tasks.md` is honest about the narrowed scope and keeps `17.5oo3` CLI-only.

## Correct repair route

- **Leela or another non-Fry repair integrator** should do the repair pass.
- Keep scope narrow: do not widen MCP, remap Phase 4, or handshake/finalize closure.
- Repair only the CLI truth seam so non-finalizing `--finalize-pending` outcomes return non-success-shaped exits / JSON and add direct CLI coverage for deferred, manifest-incomplete, integrity-failed, aborted, and no-pending-work outcomes.


# Decision: Professor pre-gate on vault-sync-engine Batch K

**Author:** Professor  
**Date:** 2026-04-23  
**Session:** vault-sync-engine Batch K pre-implementation gate  

---

## Decision

**REJECT** the proposed combined Batch K boundary.

Adopt the narrowest safer split instead:

### Batch K1 — collection-add scaffolding + read-only truth
- `1.1b`
- `1.1c`
- `9.2`
- `9.2b`
- `9.3`
- `17.5qq10`
- `17.5qq11`

### Batch K2 — offline restore integrity closure
- `17.5kk3`
- `17.5ll3`
- `17.5ll4`
- `17.5ll5`
- `17.5mm`
- `17.11`

---

## Why reject the combined boundary

The proposed batch still mixes a brand-new ordinary operator path with destructive-path closure, and the destructive half is not just proof-writing. Today `src\commands\collection.rs` has no `add` or `list` entrypoints at all, so `9.2`/`9.3` are real operator-surface additions, not scaffolding polish.

More importantly, the restore half still has implementation holes that make the "integrity matrix" misleading if treated as mostly-test work:
- Offline `begin_restore()` does not persist `restore_command_id`; the identity gate in `finalize_pending_restore()` therefore does not yet honestly protect the offline path.
- `restore_reset()` currently clears pending/integrity state unconditionally once `--confirm` is given, so the claimed blocked recovery flow is not yet bounded tightly enough for adversarial proof review.
- `collections.writable` exists in storage, but the current write interlock does not refuse writes with a typed `CollectionReadOnlyError`, so `9.2b` / `17.5qq11` are also real production changes.

That means the combined batch would ask reviewers to approve both a new attach surface and a still-moving destructive recovery surface together. Per standing Professor policy, that should be split.

---

## Non-negotiable constraints for Batch K1

1. `gbrain collection add` must be a fresh-attach command, not a compatibility alias for `import_dir()`.
2. The command must refuse before row creation if the collection name is invalid, the root cannot be opened with symlink-safe root validation, or `.gbrainignore` atomic parse fails.
3. Initial reconcile must run under the same short-lived owner lease discipline used by plain sync: register session, acquire `collection_owners`, heartbeat through reconcile, and release on every exit path.
4. Success must mean attach is actually complete: collection row persisted, initial reconcile finished, and only then `state='active'`, `needs_full_sync=0`, and `last_sync_at` updated.
5. Default add remains read-only with respect to vault bytes: no `gbrain_id` write-back, no watcher start, no serve/handshake widening.
6. Capability probe may downgrade to `writable=0` only for true permission/read-only signals (`EACCES`/`EROFS`-class results). Other probe failures must fail the command, not silently downgrade.
7. `CollectionReadOnlyError` must be enforced through the shared write gate used by mutating CLI and MCP paths, not only by `collection add`.
8. `collection list` must stay diagnostic-only; if the spec/task output columns differ, artifacts must be repaired before implementation claims the surface is done.

---

## Entry criteria for Batch K2

Batch K2 should not start until K1 lands and the restore slice is framed honestly as implementation + proof, not proof-only. The batch must make these real in code:

1. Offline restore must persist originator identity before any path that may later rely on finalize/recovery gating.
2. `finalize_pending_restore()` must allow the fresh-heartbeat bypass only for the exact persisted originator identity; successor processes may only proceed after the heartbeat/persistence rules declare the originator dead.
3. Tx-B failure recovery must preserve `pending_root_path` and not let generic recovery erase evidence.
4. Manifest-incomplete retry vs escalation must be deterministic and leave terminal state visible to operators.
5. Manifest tamper must force terminal integrity failure and a bounded `restore-reset` recovery path.
6. `17.11` must be a true end-to-end proof over real CLI scaffolding, not a fixture shortcut.

---

## Mandatory proof expectations

### Batch K1
- direct CLI/integration proof that `collection add` acquires and releases the short-lived lease around initial reconcile
- proof that add leaves `needs_full_sync=0` only on successful reconcile
- probe downgrade test (`writable=0`) and shared write-refusal test (`CollectionReadOnlyError`)
- `brain_gap` slug-bound vs slug-less restoring-state proof (`1.1c`)

### Batch K2
- adversarial proof that a different restore-command identity cannot bypass fresh-heartbeat defer
- Tx-B failure residue proof (`pending_root_path` preserved)
- manifest incomplete retry-within-window and TTL escalation proofs
- manifest tamper -> terminal integrity failure -> reset-required proof
- real CLI offline restore integration proving finalize/attach truthfully

---

## Nibbler focus

Mandatory Nibbler focus remains:
- identity theft prevention
- manifest tamper
- Tx-B failure residue

Additional required seam for K2:
- **restore-originator identity persistence and comparison**, especially the offline path, because the current tree does not yet bind offline restore to the `restore_command_id` gate and that is the easiest place for a "proof" batch to hide live behavior changes.


# Decision: Professor pre-gate on vault-sync-engine Batch K2

**Author:** Professor  
**Date:** 2026-04-23  
**Requested by:** macro88

---

## Verdict

**APPROVE** Batch K2 as the next safe boundary.

K1 has already landed the attach/read-only scaffolding that made earlier K2 proof claims dishonest. What remains is a single offline restore-integrity closure slice, but only if K2 is treated as **real code + proof**, not as a test-only batch.

---

## Why K2 is now the right boundary

The prior blocker was structural: offline restore did not persist `restore_command_id`, so `finalize_pending_restore()` could not honestly distinguish originator from successor on the offline path. That blocker is still present in code, but it is now isolated to the K2 slice rather than entangled with K1 attach/read-only work.

The current tree also keeps the remaining K2 risks tightly coupled:

1. **Originator identity is incomplete on offline restore.**
   - Online `begin_restore()` persists `restore_command_id`.
   - Offline `begin_restore()` does not.
   - `finalize_pending_restore()` already compares exact `restore_command_id`, so the missing offline persistence is the live hole K2 must close.

2. **Reset/finalize operator truth is still incomplete.**
   - `restore_reset()` clears restore state unconditionally once invoked.
   - Offline restore/operator surfaces still do not yet prove a full honest end-to-end completion path.

3. **Residue/integrity outcomes already live in the same state machine.**
   - `pending_root_path`
   - `pending_manifest_incomplete_at`
   - `integrity_failed_at`
   - `needs_full_sync`

That is coherent enough for one batch, provided K2 does **not** widen into unrelated work.

---

## Non-negotiable implementation and review constraints

### 1. Restore originator identity persistence and comparison

1. Offline `begin_restore()` must mint a real `restore_command_id` and persist it in the same authority update that writes `pending_root_path`, `pending_restore_manifest`, and `pending_command_heartbeat_at`.
2. The offline path must return/report the same persisted identity; do not keep using lease/session identity as a substitute.
3. `finalize_pending_restore()` may bypass fresh-heartbeat defer **only** for the exact persisted `restore_command_id`.
4. Missing/NULL/stale identity must never grant originator privileges. It must behave as **not originator**, not as an implicit bypass.
5. Startup recovery / external finalize callers may proceed only after the heartbeat/liveness rules say the originator is dead or stale.

### 2. Reset/finalize honesty

1. `restore_reset()` must become a bounded escape hatch, not a universal clear-state button.
2. Reset must require explicit operator intent **and** a restore-blocked state that actually warrants reset (`integrity_failed_at`, terminal manifest escalation, or equivalent explicit aborted-recovery case).
3. Operator surfaces must not success-shape blocked outcomes:
   - no exit-0 / `"status":"ok"` for `Deferred`
   - no exit-0 / `"status":"ok"` for `ManifestIncomplete`
   - no exit-0 / `"status":"ok"` for `IntegrityFailed`
   - no exit-0 / `"status":"ok"` for `NoPendingWork`
4. `collection restore` / `collection info` / `sync --finalize-pending` must describe the actual remaining state. If the collection is still blocked (`state='restoring'` or `needs_full_sync=1`), output must say so plainly.
5. K2 must not quietly widen plain no-flag `collection sync` into a generic recovery multiplexer. If CLI completion is needed, keep it on one explicit, named path and test that path directly.

### 3. Tx-B residue handling

1. Tx-B failure must leave `pending_root_path` durable.
2. Tx-B failure must not clear the manifest, originator identity, or blocked-state evidence needed for later finalize/review.
3. Generic recovery / reconcile workers must not erase or reinterpret Tx-B residue.
4. Plain sync must continue to refuse the collection while Tx-B residue remains.
5. Only explicit finalize logic or explicit operator reset may consume the residue.

### 4. Manifest retry / escalation / tamper handling

1. **Missing files** and **tamper/mismatch** must remain distinct states.
2. Missing-files within the retry window must:
   - set `pending_manifest_incomplete_at` once,
   - remain blocked,
   - retry idempotently,
   - clear cleanly on a later successful manifest match.
3. Missing-files beyond TTL must escalate to terminal `integrity_failed_at` and remain blocked until explicit `restore-reset`.
4. Manifest tamper/mismatch must escalate immediately to terminal integrity failure; no soft-success, no silent retry, no auto-clear.
5. Success after a prior incomplete state must clear the incomplete marker deterministically; success after tamper must not happen without explicit reset/restart of the flow.

### 5. End-to-end offline restore integration proof

1. `17.11` must use the real CLI/operator path, not fixture-only DB surgery.
2. The proof must demonstrate a complete offline restore lifecycle from CLI initiation through an explicit completion path to an honestly active collection.
3. The end state must prove:
   - correct root adoption,
   - `state='active'`,
   - `needs_full_sync=0`,
   - blocked-state fields cleared only when appropriate,
   - writes reopened only after attach completion is genuinely done.
4. If the chosen completion path is CLI-driven, the test must exercise that exact CLI command chain. If the chosen completion path relies on RCRT/serve ownership, then `17.11` is **not** satisfied and must stay deferred.

---

## What must become real in this batch vs. what may remain deferred

### Must become real in K2

- Offline `restore_command_id` persistence
- Exact originator-identity comparison on finalize
- Honest reset scoping
- Honest finalize/outcome surfacing
- Tx-B residue durability and non-erasure
- Deterministic manifest incomplete vs tamper state machine
- One real offline CLI completion path proven end-to-end (`17.11`)

### May remain deferred after K2

- Online restore handshake hardening (`17.5pp`, `17.5qq`, related serve ack work)
- Remap-specific destructive-path follow-ons
- Broader `CollectionReadOnlyError` widening beyond the already-approved K1 vault-byte scope
- `--write-gbrain-id`, writable recheck UX, watcher-mode widening, and other collection-admin expansion
- Any "originating command retries while still alive" quality-of-life loop, **if** the blocked-state/operator path remains fully honest without it

---

## Minimum proof / tasks required for honest landing

K2 does not land honestly without all of the following:

1. **`17.5kk3`** — direct proof that Tx-B failure leaves `pending_root_path` intact and generic recovery does not clear it.
2. **`17.5ll3`** — adversarial proof that a different restore-command identity cannot bypass fresh-heartbeat defer.
3. **`17.5ll4`** — proof that manifest-incomplete retries succeed within the allowed window and clear the incomplete marker correctly.
4. **`17.5ll5`** — proof that the same condition escalates to `integrity_failed_at` after TTL and remains reset-required.
5. **`17.5mm`** — proof that manifest tamper forces terminal integrity failure and bounded `restore-reset` recovery.
6. **`17.11`** — real end-to-end offline restore integration proof over the actual CLI recovery path.
7. Direct review proof that offline `begin_restore()` now persists `restore_command_id` and that the finalize comparison uses that exact persisted value.
8. Direct CLI truth proof that blocked outcomes are not rendered as success.

---

## Remaining scope risk that would still force a split

There is one real split trigger left:

**If `17.11` requires inventing a new recovery topology instead of exercising one explicit offline CLI path, split again.**

Concretely, if the implementation tries to combine:

- offline identity/state-machine closure,
- new CLI attach-completion topology,
- plain-sync semantic widening,
- or online/RCRT/serve ownership redesign

inside the same batch, that is no longer one coherent integrity closure slice. In that case, the narrow safer split is:

- **K2a:** offline identity persistence + finalize/reset/state-machine closure (`17.5kk3`, `17.5ll3`, `17.5ll4`, `17.5ll5`, `17.5mm`)
- **K2b:** the explicit CLI end-to-end completion path and `17.11`

Absent that expansion, K2 is the right next boundary and should proceed.


# Professor Review — vault-sync Batch K2

- Date: 2026-04-23
- Verdict: APPROVE
- Scope reviewed: 17.5kk3, 17.5ll3, 17.5ll4, 17.5ll5, 17.5mm, 17.11

## Why this lands

- Offline restore now persists and compares restore-originator identity instead of trusting caller class alone.
- Tx-B residue is durable and plain sync does not consume it.
- Manifest retry, escalation, and tamper branches are real, tested, and operator-truthful.
- `sync --finalize-pending` is a genuine CLI completion path that performs attach in-process after Tx-B, matching the new `17.11` note.
- `tasks.md` stays honest that online handshake, startup/orphan recovery, and broader destructive-path closure remain deferred.

## Required caveat

Keep the landing note explicit that K2 proves only the offline CLI restore-integrity closure. Do not let `17.11` be read as proving online restore handshake, RCRT startup recovery, or any broader second completion topology.


# Professor — Vault Sync Batch L Pre-gate

**Status:** APPROVED

**Scope:** OpenSpec `vault-sync-engine` Batch L as the startup-recovery closure slice: `11.1`, `11.4`, `17.5ll`, `17.13`. Treat `2.4a2` as deferred unless it stays file-local and does not widen the restore state machine.

**Decision:** Batch L is the right next safe boundary **only if it stays about startup-owned recovery after the restore originator is gone**. K2 already proved the offline CLI finalize path; the honest complementary proof is now "serve starts later, sees durable pending state, and RCRT finishes recovery before any new supervisor/watch path resumes." That is one coherent state-machine closure. Pulling in Windows gating or broader online-handshake behavior would widen the batch beyond that proof.

## Non-negotiable implementation / review constraints

1. **Startup order is fixed and reviewable.**
   - `gbrain serve` startup must execute in this order: **process-global registry init -> sentinel recovery sweep/dirty flagging -> startup-owned RCRT finalize/attach pass -> supervisor spawn**.
   - No supervisor/watcher handle may spawn for a collection until the startup RCRT pass has had first refusal on that collection.
   - A fresh serve must still obey the existing do-not-impersonate rule for restoring collections.

2. **Registry initialization must be real, not implied.**
   - Batch L must create the actual process-global registries required by startup recovery: supervisor-handle registry, dedup registry, and recovery-sentinel root.
   - Initialization failure is fatal to serve startup; do not fall back to partially initialized background behavior.

3. **Sentinel recovery reader is monotonic and non-destructive.**
   - Startup sweep reads `*.needs_full_sync` sentinels, maps them to collection ids, and sets `collections.needs_full_sync = 1` for affected collections.
   - Sentinel presence may mark a collection dirty; sentinel absence must **not** clear dirtiness.
   - Sentinels are unlinked only after a successful reconcile/attach commit for that collection. Unknown/malformed sentinel files warn and are skipped, not treated as success.

4. **Dead-vs-slow threshold must be explicit and small.**
   - The stale-command gate must stop keying off handshake timeout. Use **15 seconds** (three missed 5-second heartbeats) as the single threshold for "originator dead enough for `StartupRecovery`."
   - `StartupRecovery` must defer while the heartbeat is fresher than that threshold, and may finalize once it ages past it.
   - Do not add PID/start-time probes or a new identity surface in this batch; keep the decision on persisted command token + heartbeat age only.

5. **RCRT owns runtime finalize/attach at startup.**
   - Startup recovery must route through `finalize_pending_restore(..., FinalizeCaller::StartupRecovery { ... })` and then the existing attach-completion seam.
   - The batch is not honest if startup code hand-inlines a second finalize SQL path or clears pending state without going through the canonical helper.

6. **Truth boundary must stay narrow.**
   - Batch L may claim startup/orphan recovery for restore finalize + attach and startup sentinel ingestion only.
   - It must not claim broader online restore handshake closure, hot live-serve rebinding polish, MCP destructive-path widening, or Windows platform gating.

## Must become real in this batch

- Concrete serve-startup orchestration enforcing the order above.
- Real recovery-sentinel root initialization plus startup reader that raises `needs_full_sync`.
- Startup RCRT recovery of an orphaned restore into attach-complete active state.
- Direct proof that a crash between rename and Tx-B is recovered by the next serve startup, not only by CLI `--finalize-pending`.
- Direct proof that startup sentinel sweep blocks supervisor spawn until reconcile clears the dirty state and removes the sentinel.
- One shared stale-heartbeat helper/constant used by startup recovery decisions.

## Deferred from this batch

- `2.4a2` Windows platform gate, unless it lands as a strictly isolated, separately provable co-traveler.
- Any expansion of the online handshake protocol, IPC routing, or serve-side live rebind semantics beyond what is already in tree.
- Broader watcher/supervisor architecture cleanup unrelated to startup-first recovery.
- New originator identity dimensions beyond persisted `restore_command_id` plus heartbeat age.

## Minimum proof required for honest landing

1. **Startup orphan restore integration proof**
   - Seed a real pending-finalize restore (`pending_root_path` present, originator heartbeat stale, no live originator), start serve, prove RCRT finalizes and attaches before the collection is exposed as writable/active.

2. **Fresh-heartbeat defer proof**
   - Same pending-finalize shape with a fresh heartbeat must stay deferred at startup; serve must not steal finalize early.

3. **Sentinel startup recovery proof**
   - Seed a `brain_put`-style sentinel and stale DB/file divergence, start serve, prove startup marks the collection dirty, reconcile runs, sentinel is removed only after success, and no supervisor is considered attached first.

4. **Ordering proof**
   - Direct test or tightly-scoped instrumentation proving the startup sequence is registry init before RCRT and RCRT before supervisor spawn.

5. **No new success-shaped lies**
   - If startup recovery cannot finalize/attach, the collection must remain blocked in an observable state; do not silently clear pending columns or pretend serve is healthy.

## Scope risk that forces a split

Split immediately if implementation needs any of the following to pass:

- a new online-handshake protocol change,
- broader supervisor lifecycle rewrites beyond startup-first ordering,
- Windows platform-gate work mixed into the same review,
- or a second finalize/attach code path distinct from the current helper seams.

That narrower split would be: **L1 = startup order + sentinel sweep (`11.1`, `11.4`)** and **L2 = orphan restore finalize proof (`17.5ll`, `17.13`)**.


# Professor — Vault Sync Batch L1 Review

## Verdict
APPROVE

## Why
- The implementation stays inside the approved L1 boundary: process-global registry startup scaffolding plus restore-orphan startup recovery.
- Startup order is materially enforced in `start_serve_runtime()` / `run_startup_sequence()`: stale-session sweep, own-session lease reclamation, startup RCRT recovery, then supervisor-handle bookkeeping before runtime loop activity.
- One 15s threshold is reused for stale-session sweep, owner liveness, and fresh-heartbeat defer.
- Coverage is adequate for this slice: exact-once orphan finalize, fresh-heartbeat defer, stale owner takeover despite foreign session residue, and no stray supervisor-ack residue after startup recovery.
- `tasks.md` is honest about scope: `11.1a`, `17.5ll`, and `17.13` are in; `11.1b`, `11.4`, and `17.12` remain deferred.

## Required caveat
This approval does **not** cover recovery-sentinel directory work, crash-mid-write startup ingestion, or any IPC / online-handshake widening. Those remain deferred and must stay explicit in task and landing notes.


# Scruffy — 13.3 CLI proof map gate

**Date:** 2026-04-24  
**Topic:** vault-sync-engine `13.3` only  
**Scope:** CLI collection-aware slug acceptance, ambiguous bare-slug behavior, and canonical page-reference output parity with N1.  

## Decision

Treat `13.3` as a CLI-surface proof slice, not a shared-helper closure.

Direct tests must cover each CLI command family at its own dispatch seam:

### 1) Slug-bearing read commands

- `get`
- `graph`
- `timeline`
- `links`
- `backlinks`

**Required proofs**
- explicit `<collection>::<slug>` resolves the intended page when the bare slug collides across collections
- bare colliding slug fails closed
- ambiguity output names canonical candidates (`work::...`, `personal::...`), but do **not** overclaim MCP-style machine payload shape

### 2) Slug-bearing write/update commands

- `put`
- `link`
- `unlink`
- `tags`
- `timeline-add`
- slug-bound `check`

**Required proofs**
- explicit `<collection>::<slug>` acts on the selected collection only
- bare colliding slug fails closed before mutation
- `put` needs both create/update semantics respected under collection-aware routing because `WriteCreate` and `WriteUpdate` resolve differently

### 3) Output-parity commands that reference pages

- `get --json`
- `list`
- `search`
- `query`
- `graph`
- `links`
- `backlinks`
- `timeline --json`
- `check`
- success-path `link` / `unlink` / `timeline-add` / `put`

**Required proofs**
- any emitted page reference is canonical `<collection>::<slug>`
- parity claims should be limited to outputs that actually reference pages; `tags` list output is tag-only and does not need canonical page rendering

## Why

Current code still has command-local bare-slug seams:

- `graph` still calls `core::graph::neighborhood_graph(slug, ...)`, and core graph resolution still roots on `SELECT id FROM pages WHERE slug = ?1` and serializes bare `p.slug`
- `check` pre-resolves for gating, but then `resolve_targets()` and `assertions::check_assertions()` still flow canonical input back into bare `pages.slug` lookups
- `list` selects `slug` from `pages` with no collection join
- `search` uses non-canonical FTS output; `query` uses non-canonical hybrid search even though canonical helpers already exist
- `links` / `backlinks` query and print bare joined slugs
- `timeline` JSON and status strings echo the caller input rather than the resolved canonical page address
- `get --json` returns `Page.slug` bare; human `get` does not currently inject a canonical slug unless frontmatter already contains one

## Review posture

Do **not** let passing helper tests or MCP N1 proofs substitute for CLI proof here.

The likely first-pass failures for `13.3` are:

1. `graph` explicit canonical input still misses because the root lookup is bare-slug only  
2. `check` explicit canonical input still regresses inside assertion helpers  
3. `list` / `search` / `query` / `graph` / `links` / `backlinks` still emit bare slugs, so any CLI parity claim with N1 is premature until direct output assertions land


# Scruffy — 13.6 proof map gate

**Date:** 2026-04-24  
**Topic:** vault-sync-engine `13.6` + `17.5ddd`  
**Scope:** read-only `brain_collections` MCP tool; frozen response schema only.

## Decision

Gate `13.6` only on direct MCP schema-fidelity proof. Do not accept CLI `collection list/info` parity, helper-level status coverage, or broader collection-semantics tests as evidence.

## Required proof surface

Every returned collection object must prove the exact frozen 13-field schema from design.md, with no missing keys and no surrogate fields:

1. `name`
2. `root_path`
3. `state`
4. `writable`
5. `is_write_target`
6. `page_count`
7. `last_sync_at`
8. `embedding_queue_depth`
9. `ignore_parse_errors`
10. `needs_full_sync`
11. `recovery_in_progress`
12. `integrity_blocked`
13. `restore_in_progress`

### Non-negotiable assertions

- **Exact keyset:** parse the JSON object and assert the key set exactly matches the frozen 13 names.
- **Read-only behavior:** the call must succeed while collections are `restoring` / `needs_full_sync=1`; it must not write or clear any collection columns as a side effect.
- **Presentation masking:** `root_path` is the stored string only for `state='active'`; otherwise the API must return `null` even though storage keeps the last path.
- **Tagged-union fidelity:** `ignore_parse_errors` is either `null` or an array of objects with exactly `code`, `line`, `raw`, `message`.
  - `code='parse_error'` => `line` and `raw` populated
  - `code='file_stably_absent_but_clear_not_confirmed'` => `line=null`, `raw=null`
- **Recovery booleans:** queued recovery is `needs_full_sync=true` + `recovery_in_progress=false`; running recovery is `true/true`; `false/true` is unreachable and should be guarded by tests.
- **Integrity discriminator:** only `null | manifest_tampering | manifest_incomplete_escalated | duplicate_uuid | unresolvable_trivial_content` are valid, with precedence `manifest_tampering > manifest_incomplete_escalated > duplicate_uuid > unresolvable_trivial_content`.
- **Restore advisory:** `restore_in_progress=true` only for the narrow destructive restore window, not for every `state='restoring'` row.

## Likely under-proved seams

1. **Reusing CLI collection surfaces.** `collection info/list` already differ from the frozen MCP contract (`writable` stringification, extra fields, different blocker strings), so they cannot certify `brain_collections`.
2. **Borrowing `collection_status_summary()`.** Current helper logic surfaces values like `manifest_incomplete_pending` / `post_tx_b_attach_pending` that are not legal in the MCP discriminator schema.
3. **Skipping exact-object assertions.** Simple `field exists` checks will miss extra keys, wrong nullability, and accidental renames like `queue_depth` vs `embedding_queue_depth`.
4. **Conflating storage with presentation.** `collections.root_path` is `NOT NULL` in schema; only the MCP presentation masks it.
5. **Inventing extra progress flags.** The design explicitly forbids `recovery_scheduled`; queued-vs-running must come from the two booleans only.

## Review posture

If the slice lands without direct MCP tests that freeze the exact object shape and the discriminator/nullability cases above, `13.6` and `17.5ddd` should stay open.


# Scruffy post-batch gap audit

- Date: 2026-04-25
- Scope: watcher-core + quarantine-seam proof audit

## Decision

Treat the current watcher/quarantine proof lane as **improved but still intentionally narrow**.

## What landed in this audit

- Added direct quarantine epoch proofs in `src/core/quarantine.rs`:
  - stale export receipts do not surface on the current quarantine epoch
  - stale export receipts do not relax discard for DB-only-state pages
- Added a watcher lifecycle source invariant in `src/core/vault_sync.rs` that pins `sync_collection_watchers(...)` to:
  - active collections only
  - watcher removal when a collection leaves the active set
  - watcher replacement when `root_path` or `reload_generation` changes

## Remaining highest-value gaps

1. **Hard-delete guard coverage remains under-proved.** We still want one explicit invariant test that every page-delete site consults the DB-only-state guard before hard delete (`reconciler` missing-file path, quarantine discard, TTL sweep). This is the most important quarantine safety seam still not nailed down.
2. **Watcher overflow fallback remains unproved.** The bounded notify queue's `TrySendError::Full` branch should eventually get a direct proof that it flips `needs_full_sync=1` and does not silently drop safety on backpressure.

## Guardrail

Do **not** widen tests into quarantine restore. Restore remains deferred until Fry lands a no-replace install step plus crash-durable post-unlink cleanup.


# Scruffy quarantine proof note

- **Date:** 2026-04-25
- **Scope:** vault-sync-engine next narrow slice after watcher core
- **Decision:** treat the current proof lane as `7.5` dedup-cleanup only; do **not** claim quarantine lifecycle closure yet.

## Why

`src/core/vault_sync.rs` now exposes truthful internal seams for the two dedup failure paths that mattered in this batch:

- `writer_side_rename_failure_cleans_tempfile_dedup_and_sentinel_without_touching_target`
- `writer_side_post_rename_fsync_abort_retains_sentinel_removes_dedup_and_marks_full_sync`

Those prove the exact reviewer-facing safety claims for task `7.5`: rename failure removes the dedup stamp and local temp/sentinel residue without touching the live file, and post-rename failure removes the dedup stamp so reconcile can see the landed bytes while marking the collection dirty.

These tests are `#[cfg(unix)]`, so on the current Windows host they are reviewable/compile-visible but not executable in-session. I re-ran the adjacent collection-status proofs only; the dedup-cleanup call here is therefore scope-based and source-truthful rather than backed by a local Unix execution.

## What stayed unproved

Tasks `17.5g7`, `17.5h`, `17.5i`, and `17.5j` remain unproved because there is still no landed operator-facing `gbrain collection quarantine {list,restore,discard,export,audit}` seam in `src/commands/collection.rs`, no production export/discard/restore API to drive, and no TTL-sweep surface to assert against.

`gbrain collection info` also does **not** yet surface the deferred `9.9b` quarantine-awaiting count, so there was no truthful extra CLI proof to add for this slice beyond the already-landed blocked-state/status tests.

## Reviewer guidance

Approve the narrow dedup-cleanup proof lane if the batch stays confined to the two `vault_sync` tests above.

Do not let this batch imply quarantine lifecycle coverage, watcher overflow/supervision, live ignore reload, IPC, or broader watcher choreography. Those need new production hooks first.


# Scruffy — Vault Sync Batch H Test Lane

Date: 2026-04-22
Requested by: Matt

## Decision

Keep Batch H coverage centered on the current reconciler unit seams and defer orchestration claims until callable helpers exist.

## What I locked

- Added active unit coverage in `src/core/reconciler.rs` for:
  - empty-body refusal branches in `hash_refusal_reason()`;
  - fresh-attach authorization success on detached collections;
  - fail-closed authorization when fresh-attach or drift-capture modes receive the wrong caller identity.
- Added ignored blocker tests for:
  - the exact 64-byte canonical trivial-content boundary reuse required by task `5.8a0`;
  - strict Phase 0–3 ordering / abort-before-destruction checks for tasks `5.8a` through `5.8d2`;
  - fresh-attach `needs_full_sync` clear / write-gate reopen sequencing for task `5.9`.

## Exact blocker

Current worktree implementation is ahead of the error enum surface: `src/core/reconciler.rs` now constructs `InvalidFullHashAuthorization`, `UuidMigrationRequiredError`, `RemapDriftConflictError`, `CollectionUnstableError`, and `CollectionDirtyError`, but `ReconcileError` in the same file does not define those variants yet. Because of that mismatch, `cargo test` currently fails at compile time before the new Batch H seam tests can run.

## Why this matters

This keeps test names honest: we can prove the implemented closed-mode / caller-identity contracts today without pretending Phase 4, destructive execution, or attach-completion orchestration already exists. The ignored seam tests preserve the intended invariants and make the remaining implementation debt explicit instead of silently dropping coverage pressure.


# Decision: Scruffy K2 proof lane outcome

**Author:** Scruffy  
**Date:** 2026-04-23  
**Requested by:** macro88

---

## Outcome

I landed and validated the honest K2 proof slice for:

- `17.5kk3`
- `17.5ll3`
- `17.5ll4`
- `17.5ll5`
- `17.5mm`

The repo now has direct unit/CLI coverage for originator-identity matching, pending-root residue preservation, manifest retry vs TTL escalation, tamper-driven integrity failure, retryable-vs-terminal operator guidance, and bounded `restore-reset`.

## Boundary kept honest

`17.11` is still **not** honestly supported by the current implementation. Offline restore still reaches Tx-B in the CLI and depends on the later serve/RCRT attach path to reopen writes; I did not widen scope into a new pure-CLI completion topology just to claim the proof.

## Notes

- Offline restore now mints/persists a real `restore_command_id` and uses `finalize_pending_restore()` instead of jumping straight to `run_tx_b()`.
- `restore-reset` now fail-closes unless the collection is in terminal restore-integrity failure.
- Retryable `pending_manifest_incomplete_at` states now point operators to `gbrain collection sync <name> --finalize-pending`, while tamper / escalated TTL failure still point to `restore-reset`.


# Scruffy vault-sync coverage audit

- **Date:** 2026-04-24
- **Decision:** Prioritize the next coverage slice on (1) Unix watcher failure-path internals in `src/core/vault_sync.rs` and (2) quarantine CLI truth/path-matrix tests in `tests/quarantine_revision_fixes.rs` plus `tests/collection_cli_truth.rs`.
- **Why:** Existing coverage already proves the narrow happy seams, but the highest-risk unguarded branches are still watcher queue overflow / watcher reload replacement and quarantine epoch-sensitive list/export/discard behavior. Those are the branches most likely to regress silently under refactor while still leaving current suites green.
- **Exact targets:**
  - `src/core/vault_sync.rs` — watcher channel full → `needs_full_sync`, watcher replacement on `reload_generation` / root-path change, disconnected watcher receiver error path.
  - `src/core/quarantine.rs` — export payload completeness, force-discard / clean-discard behavior, current-epoch export upsert behavior.
  - `tests/quarantine_revision_fixes.rs` — deferred restore refusal before slug resolution / mutation for missing or ambiguous slugs.
  - `tests/collection_cli_truth.rs` — quarantine list happy-path JSON contract with `exported_at` + per-category `db_only_state`, CLI export/discard receipts for the remaining force/no-db-state branches.
- **Small seam note:** the watcher overflow branch is awkward to drive from integration tests because the `notify` callback and bounded sender are buried inside `start_collection_watcher()`. A tiny extraction of the callback dispatch into a helper (or a test-only injectable sender) would make that branch directly testable without widening production behavior.


# Scruffy watcher-core proof decision

- Date: 2026-04-25
- Scope: `vault-sync-engine` post-`17.5aa5` watcher-core proof lane

## Decision

Keep the reviewer-facing proof set narrowed to the currently real public seam: `vault_sync::start_serve_runtime()` must defer a fresh restore heartbeat without mutating page/file-state rows.

## Why

- `openspec\changes\vault-sync-engine\tasks.md` still shows watcher pipeline tasks `6.1`-`6.11` and dedup tasks `7.1`-`7.6` open.
- `Cargo.toml` does not yet include `notify`, and the current production code does not expose a watcher/debounce/path+hash TTL dedup surface to test.
- Writing debounce or dedup assertions now would bless design intent, not shipped behavior.

## Reviewer / implementation trap

When Fry lands the watcher slice, expose a deterministic seam for:

1. debounce coalescing window,
2. path+hash-within-TTL suppression,
3. TTL expiry acceptance,
4. path-only mismatch acceptance.

Without that seam, integration tests can only prove the current non-mutation/defer behavior.


# Zapp — Promo / Docs Pass Decision Log

**Date:** 2026-04-25  
**Agent:** Zapp  
**Branch context:** vault-sync-engine active; v0.9.4 is current release tag

---

## Decisions made

### D1 — Tool count: use 16 for released surface, 17 for branch-context prose

**Surface:** `website/src/content/docs/index.mdx` code comment; `getting-started.mdx` prose; `phase3-capabilities.md` call/related sections.

**Decision:** Any surface that a user reaches via the standard install path (curl installer → v0.9.4 binary) must show **16 tools** — the released count. Prose that explicitly discusses the vault-sync-engine branch may say 17 and should include the branch qualifier. "All 17 tools" without a branch qualifier is a lie for released users.

**Prior art:** The README already makes this split: "All 17 tools are available when you run `gbrain serve` from the vault-sync-engine branch (16 from the current `v0.9.4` release)." Docs site now matches.

---

### D2 — Homepage feature grid: add vault-sync card

**Surface:** `website/src/content/docs/index.mdx` CardGrid.

**Decision:** Added a 4th card "Live Vault Sync" with an *(vault-sync-engine branch)* qualifier. The Obsidian-sync angle is the most compelling growth hook for the target audience of developers with large markdown vaults. Showing it on the homepage with a clear branch label builds aspiration without lying about the release state.

---

### D3 — README features: promote vault-sync-engine features

**Surface:** `README.md` `## Features` bullet list.

**Decision:** Moved "Live file watcher" and "Collection management" from the bottom of the list to positions 5–7 (just after MCP server). The live watcher is the headline growth narrative for the vault-sync-engine work. Burying it at line 15 of a 15-item list undercuts its impact. Kept *(vault-sync-engine branch)* labels intact.

---

### D4 — Roadmap: remove stale "tag pending" language for Phase 1

**Surface:** `website/src/content/docs/contributing/roadmap.md`.

**Decision:** The Phase 1 entry said "v0.1.0 — tag pending. All ship gates passed; pushing the v0.1.0 tag triggers the release workflow." The project is at v0.9.4 — this is meaninglessly stale and mildly embarrassing. Simplified to `**Release:** \`v0.1.0\``.

---

### D5 — Roadmap version targets: add vault-sync-engine TBD row

**Surface:** `website/src/content/docs/contributing/roadmap.md` version targets table.

**Decision:** Added a "TBD" row for vault-sync-engine so the version targets table matches the roadmap section above it. Silence = deferred; explicit TBD = in progress. Restore and IPC are called out as deferred in the table description.

---

## Deferred launch work (not done in this pass)

- A dedicated vault-sync-engine guide page (collections, watcher setup, quarantine CLI)
- `why-gigabrain.mdx` could get a full "Obsidian + agent workflow" section once vault-sync ships a release tag
- Blog / changelog post for vault-sync-engine to drive OSS discoverability
- npm public publication remains blocked (NPM_TOKEN gate)



### 2026-04-25: macOS preflight cache-key sanitization — Issue #79/#80 workflow unblocker
**By:** Mom
# Mom — issue #79 / #80 macOS preflight readout

Date: 2026-04-25

## Findings

- The four failed PR #83 macOS preflight jobs (`72986784880`, `72986784883`, `72986784888`, `72986784898`) all die at the same place: **`actions/cache@v4` rejects the cache key before `cargo check` starts**.
- Root cause is not `fs_safety.rs` in those jobs. The cache key in `.github/workflows/ci.yml:78` embeds `matrix.features`, and values like `bundled,online-model` / `bundled,embedded-model` contain commas. `actions/cache` hard-fails with `Key Validation Error ... cannot contain commas.`
- This is still live on current PR head `db851e5`: the rerun macOS preflight jobs (`72986952241`, `72986952246`, `72986952248`, `72986952250`) failed the same way.

## Issue #80 status

- `src/core/fs_safety.rs:199` **does** contain the widening cast now: `mode_bits: stat.st_mode as u32`.
- But issue #80 is still **operationally open on this branch** because the new macOS proof job never reaches compilation. There is no fresh evidence yet that macOS `cargo check` actually passes on PR #83.
- I do **not** see evidence of a second macOS compiler seam in the failed logs; the branch is blocked earlier by workflow validation.

## Minimum fix

- Exact seam: `.github/workflows/ci.yml:78`
- Minimum repair: stop using raw comma-joined feature strings in the cache key. Sanitize that field (for example, replace `,` with `-`) or add an explicit matrix-safe cache-key token such as `bundled-online-model`.
- No product-code change is indicated by these failures; this is a workflow-only unblocker.

---

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

### 2026-04-28: Leela Batch 1 watcher-reliability scope repair — narrowed to CLI-only

**By:** Leela  
**What:** Repaired Batch 1 scope post-professor-rejection through direct OpenSpec artifact edits + lockout enforcement.  
**Why:** Professor's three blockers are all resolvable by narrowing MCP watcher-health widening to CLI-only, moving overflow-recovery mode from authorization to operation enum, and simplifying WatcherMode semantics.

**Scope changes:**
- **6.7a:** Overflow recovery mode moved to `FullHashReconcileMode`; authorization stays `ActiveLease`; worker loads `active_lease_session_id` and skips on mismatch
- **6.9:** `WatcherMode` narrowed to `Native | Poll | Crashed` (no `Inactive`); `null` for non-active/Windows
- **6.11:** Narrowed to CLI-only `quaid collection info` watcher-health fields; `memory_collections` MCP tool unchanged; 13.6 frozen 13-field schema preserved

**Deferred:**
- `memory_collections` MPC watcher-health widening — requires explicit 13.6 reopen (design + test + gate)
- Any broader `memory_collections` schema changes — same 13.6 freeze

**Implementation routing:**
- **Fry is locked out** of the next revision of Batch 1 artifact
- **Implementer:** Mom (recommended) — track record on repair work; not involved in rejected scope
- **Task sequence:** 6.7a (overflow recovery) → 6.8 (.quaidignore live reload) → 6.9/6.10/6.11 (watcher chain, CLI health surface)

**v0.10.0 gate requirements:**
- All 13 Batch 1 tasks marked `[x]` with truthful closure notes
- 6.7a closure names `FullHashReconcileAuthorization::ActiveLease` explicitly
- 6.11 closure confirms `memory_collections` NOT widened; 13.6 exact-key test passes clean
- `cargo test` passes zero failures
- Coverage ≥ 90% on all new paths
- Nibbler adversarial sign-off on 6.9 (poll fallback) and 6.10 (backoff timing)
- Cargo.toml version bumped to `0.10.0`
- CHANGELOG.md updated with v0.10.0 feature list

**Result:** Batch 1 scope now honestly closable under narrowed v0.10.0. Awaiting Mom to begin 6.7a implementation.



---
author: amy
date: 2026-04-25
type: docs-audit
subject: hard-rename GigaBrain → Quaid across all prose docs
status: audit-complete — no files edited yet
---

# Quaid Hard-Rename Docs Audit

Read-only audit. No files were changed. This document maps every location in user-facing
prose docs, skill files, and agent context files that must change when the product is renamed
from GigaBrain/gbrain/brain to Quaid/quaid/memory.

Scope of rename (per task spec):
- Product name: **GigaBrain** → **Quaid**
- CLI binary: **`gbrain`** → **`quaid`**
- Core concept: **"brain"** → **"memory"** (context-specific — see § Nuance below)
- MCP tools: **`brain_*`** → **`memory_*`**
- Primary env var: **`GBRAIN_DB`** → **`QUAID_DB`**
- All env vars: **`GBRAIN_*`** → **`QUAID_*`**
- Default install dir: **`~/.gbrain/`** → **`~/.quaid/`**
- Default DB filename: **`brain.db`** → **`memory.db`** (or `quaid.db` — see § Open questions)
- Ignore file: **`.gbrainignore`** → **`.quaidignore`**
- GitHub repo slug: **`gigabrain`** → **`quaid`** (in all URLs)
- npm package: **`gbrain`** → **`quaid`**

---

## 1. README.md — Full audit

### 1.1 Title and lede (line 1–3)
- `# GigaBrain` → `# Quaid`
- Lede tagline: "personal knowledge brain" → "personal knowledge memory"
- "GigaBrain adapts the same core concept" → remove or rewrite; Garry Tan attribution
  paragraph references "GBrain work" and "personal knowledge brain" — needs rewrite since
  the project identity changes. The inspiration credit can be retained in prose but
  "GigaBrain" as the product name must go.

### 1.2 Status line (line 5)
- `v0.9.8` status badge paragraph: "GigaBrain" x1 implicit (in roadmap reference, fine),
  plus `gbrain serve` command → `quaid serve`.

### 1.3 Roadmap table (lines 15–22)
- All `gbrain` command references in the "What ships" column → `quaid`.
- No product-name occurrences in the table headers but "GigaBrain" appears in section prose.

### 1.4 "Why" section (lines 29–36)
- "Every existing knowledge tool … GigaBrain is designed …" → "Quaid is designed …"

### 1.5 "How it works" section (lines 40–48)
- "GigaBrain stores them in a single SQLite database" → "Quaid stores them …"
- "brain.db file" → "memory.db file" throughout this section.

### 1.6 Features bullet list (lines 52–66)
- `brain.db` → `memory.db` (or `quaid.db`)
- `GBRAIN_MODEL` → `QUAID_MODEL`
- `gbrain serve` → `quaid serve`
- `gbrain collection` → `quaid collection`
- `gbrain collection quarantine` → `quaid collection quarantine`
- `~/.gbrain/` → `~/.quaid/`

### 1.7 Quick start / Install options (lines 82–141)
- Script URLs: `macro88/gigabrain` → `macro88/quaid` (or whatever the new repo slug is)
- `GBRAIN_CHANNEL=online` → `QUAID_CHANNEL=online`
- `GBRAIN_NO_PROFILE=1` → `QUAID_NO_PROFILE=1`
- Asset names: `gbrain-${PLATFORM}-airgapped` → `quaid-${PLATFORM}-airgapped`
- Binary name in install commands: `gbrain` → `quaid`
- `npm install -g gbrain` → `npm install -g quaid`
- PATH line: `~/.local/bin/gbrain` → `~/.local/bin/quaid`
- `GBRAIN_DB` → `QUAID_DB`

### 1.8 Embedding model selection section (lines 157–177)
- `GBRAIN_MODEL=large gbrain query` → `QUAID_MODEL=large quaid query`
- `gbrain --model m3 query` → `quaid --model m3 query`
- All `GBRAIN_MODEL` occurrences → `QUAID_MODEL`
- "GigaBrain continues with embedded small" → "Quaid continues with embedded small"
- DB init sentence: "GigaBrain errors before" → "Quaid errors before"

### 1.9 Environment variables table (lines 182–192)
All nine env vars must be renamed:
- `GBRAIN_DB` → `QUAID_DB`
- `GBRAIN_MODEL` → `QUAID_MODEL`
- `GBRAIN_CHANNEL` → `QUAID_CHANNEL`
- `GBRAIN_WATCH_DEBOUNCE_MS` → `QUAID_WATCH_DEBOUNCE_MS`
- `GBRAIN_QUARANTINE_TTL_DAYS` → `QUAID_QUARANTINE_TTL_DAYS`
- `GBRAIN_RAW_IMPORTS_KEEP` → `QUAID_RAW_IMPORTS_KEEP`
- `GBRAIN_RAW_IMPORTS_TTL_DAYS` → `QUAID_RAW_IMPORTS_TTL_DAYS`
- `GBRAIN_RAW_IMPORTS_KEEP_ALL` → `QUAID_RAW_IMPORTS_KEEP_ALL`
- `GBRAIN_FULL_HASH_AUDIT_DAYS` → `QUAID_FULL_HASH_AUDIT_DAYS`

### 1.10 Usage section (lines 199–289)
Every `gbrain` command → `quaid`. Also:
- `~/brain.db` → `~/memory.db`
- `brain_stats`, `brain_gap`, `brain_search` (in `gbrain call`/`gbrain pipe` examples)
  → `memory_stats`, `memory_gap`, `memory_search`

### 1.11 MCP integration section (lines 293–316)
- MCP config key `"gbrain"` → `"quaid"` (the server alias is user-defined, but example
  should use the new name)
- `"command": "gbrain"` → `"command": "quaid"`
- `"GBRAIN_DB"` → `"QUAID_DB"`
- `brain.db` → `memory.db`
- All 17 `brain_*` tool names → `memory_*`

### 1.12 Skills section (lines 319–341)
- `~/.gbrain/skills/` → `~/.quaid/skills/`
- No product name occurrences but `gbrain skills list` / `gbrain skills doctor` → `quaid ...`

### 1.13 PARA / page types section (lines 344–370)
- `gbrain import` → `quaid import` (several occurrences)

### 1.14 Contributing section (lines 372–391)
- "GigaBrain is open for contributions" → "Quaid is open for contributions"
- `gbrain serve`, `brain_collections` etc. → updated names
- `docs/spec.md`, `openspec/changes/` references remain structurally accurate; no rename.

### 1.15 Build from source section (lines 394–414)
- `cargo build` comments reference no product name but the binary output path
  `target/release/gbrain` → `target/release/quaid`
- Repo clone URL: `github.com/macro88/gigabrain` → `github.com/macro88/quaid`

### 1.16 Acknowledgements (lines 433–435)
- "GigaBrain takes the same architecture" → "Quaid takes the same architecture"
- "Same brain, different stack" → **nuanced rewrite needed** — "brain" here is Garry
  Tan's concept, not the product word. Suggest: "Same memory architecture, different stack,
  different deployment story."

---

## 2. docs/getting-started.md — Full audit

### 2.1 Title and lede (lines 1–7)
- `# Getting Started with GigaBrain` → `# Getting Started with Quaid`
- Lede: "personal knowledge brain" → "personal knowledge memory"
- `brain.db` → `memory.db`

### 2.2 "What it does" section (lines 5–14)
- "GigaBrain stores your knowledge" → "Quaid stores your knowledge"
- `brain.db` file → `memory.db` file

### 2.3 Status section (lines 18–21)
- "The current release is `v0.9.8`" → adjust if needed; `gbrain serve` → `quaid serve`

### 2.4 Install options table (lines 27–32)
- `cargo build` note: binary name changes (output is `target/release/quaid`)
- `npm install -g gbrain` → `npm install -g quaid`
- `GBRAIN_CHANNEL=online` → `QUAID_CHANNEL=online`
- `GBRAIN_MODEL` → `QUAID_MODEL`

### 2.5 Build from source (lines 39–56)
- `git clone https://github.com/macro88/gigabrain` → updated repo URL
- `cd gigabrain` → `cd quaid`
- Binary path comment: `target/release/gbrain` → `target/release/quaid`
- `cargo build --release --no-default-features --features bundled,online-model` stays the
  same (Cargo feature flags are implementation-level, not user-facing names — but
  post-rename the feature names may themselves change; flag for Fry)

### 2.6 "Your first brain" section header + body (lines 61–79)
- Section header: "Your first brain" → "Your first memory" (or "Your first Quaid database")
  — see § Nuance for the "brain" → "memory" call
- Post-install note: `GBRAIN_DB` → `QUAID_DB`
- `gbrain init ~/brain.db` → `quaid init ~/memory.db`
- Schema note: `v5 schema` language is fine; all `gbrain` command refs → `quaid`

### 2.7 All command examples throughout (lines 83–460+)
Every occurrence of `gbrain` → `quaid`. All `brain_*` MCP tool names → `memory_*`.
All `GBRAIN_*` → `QUAID_*`. All `brain.db` → `memory.db`. All `~/.gbrain/` → `~/.quaid/`.

### 2.8 MCP config JSON example (lines 155–163)
Same as README: key `"gbrain"` → `"quaid"`, `"command": "gbrain"` → `"command": "quaid"`,
`"GBRAIN_DB"` → `"QUAID_DB"`, `brain.db` → `memory.db`.

### 2.9 "Connect an AI agent via MCP" section (lines 150–186)
All 17 `brain_*` tool names in the bulleted list → `memory_*`.

### 2.10 Skills section (lines 192–210)
- `~/.gbrain/skills/` → `~/.quaid/skills/`
- "GigaBrain" implicit in "the binary embeds default skills" — no hard rename needed but
  could say "Quaid embeds default skills …"

### 2.11 Environment variable table (lines 249–252)
- `GBRAIN_DB` → `QUAID_DB`
- Default value `./brain.db` → `./memory.db`

### 2.12 Phase 2/3/vault-sync command sections (lines 258–460)
Every `gbrain` command → `quaid`. Brain health section header "Brain health" → "Memory health"
(or keep as "Health checks" for neutrality — see § Nuance).

---

## 3. docs/roadmap.md — Full audit

### 3.1 Title and intro (line 1–3)
- `# GigaBrain Roadmap` → `# Quaid Roadmap`
- "GigaBrain is built in phases" → "Quaid is built in phases"

### 3.2 Sprint 0 deliverables (lines 20–21)
- `CLAUDE.md` and `AGENTS.md` refs remain fine structurally.

### 3.3 Phase 1 narrative (lines 36–69)
- "proves GigaBrain's value proposition" → "proves Quaid's value proposition"
- All `gbrain` command refs → `quaid`
- `brain_get`, `brain_put`, etc. → `memory_get`, `memory_put`, etc.

### 3.4 Phase 2 narrative (lines 73–108)
- "separate GigaBrain from a glorified FTS5 wrapper" → "separate Quaid from …"
- All `gbrain` command refs → `quaid`
- All `brain_*` MCP tool refs → `memory_*`

### 3.5 Phase 3 narrative (lines 111–131)
- All `brain_*` tool names in the delivered list → `memory_*`

### 3.6 Version targets table (lines 154–159)
- No product names; CLI commands only; update `gbrain` → `quaid` where present.

### 3.7 vault-sync-engine section (lines 163–199)
- `gbrain collection`, `gbrain serve`, `brain_collections`, `brain_put`,
  `.gbrainignore`, `GBRAIN_QUARANTINE_TTL_DAYS`, `GBRAIN_WATCH_DEBOUNCE_MS` all need rename.
- `brain_search`, `brain_query`, `brain_list` → `memory_search`, `memory_query`, `memory_list`

---

## 4. docs/contributing.md — Full audit

### 4.1 Title and intro (lines 1–3)
- `# Contributing to GigaBrain` → `# Contributing to Quaid`
- "What GigaBrain is" → "What Quaid is"
- "GigaBrain is a local-first personal knowledge brain" → "Quaid is a local-first personal
  knowledge memory"

### 4.2 Repository layout (lines 16–75)
- Top-level directory label: `gigabrain/` → `quaid/`
- Comment inside layout: no product names except in prose above the block.

### 4.3 Build and test section (lines 81–100)
- Binary path comment: `target/release/gbrain` → `target/release/quaid`

### 4.4 Release process section (lines 106–123)
- `gbrain serve` → `quaid serve`
- Asset names: `gbrain-<platform>-airgapped` / `gbrain-<platform>-online`
  → `quaid-<platform>-airgapped` / `quaid-<platform>-online`
- npm package name note: `gbrain` package → `quaid` package

### 4.5 Contributing section prose (lines 143–168)
- "GigaBrain uses OpenSpec" → "Quaid uses OpenSpec"
- `docs/spec.md` reference fine; "every meaningful code, docs, or architecture change" fine.

---

## 5. docs/spec.md — Full audit (representative — file is large)

`spec.md` is 561 KB and was not fully read, but from the first 80 lines the following
rename needs are clear:

### 5.1 Front matter and title
- `title: GigaBrain - Personal Knowledge Brain` → `title: Quaid - Personal Knowledge Memory`
- `status: spec-complete-v4` — unchanged (version marker, not a name)
- Tag `knowledge-base` fine; add `quaid` tag.

### 5.2 Title heading
- `# GigaBrain - Personal Knowledge Brain` → `# Quaid - Personal Knowledge Memory`

### 5.3 "Repo (planned)" line
- `GitHub.com/[owner]/gbrain` → `GitHub.com/[owner]/quaid`

### 5.4 Product/concept prose throughout
- All "GigaBrain" occurrences → "Quaid"
- All `gbrain` CLI commands → `quaid`
- All `brain_*` MCP tool names → `memory_*`
- All `GBRAIN_*` → `QUAID_*`
- All `brain.db` → `memory.db`
- `brain_config` table name → `quaid_config` or `memory_config` (implementation-level;
  flag for Fry — may affect schema DDL as well)
- `~/.gbrain/` → `~/.quaid/`
- Embedded Cargo.toml `[features]` block — feature names (`embedded-model`, `online-model`,
  `bundled`) are technical identifiers, not product names; leave unless Fry renames them.

### 5.5 Spec history / version notes
- "v1 differentiator over Garry's spec" — rewrite to refer to Quaid rather than GigaBrain.
- Research technique attribution paragraphs — keep intact; just swap product name.

---

## 6. docs/gigabrain-vs-qmd-friction-analysis.md — Full audit

### 6.1 Title
- `# GigaBrain vs QMD: Friction Analysis` → `# Quaid vs QMD: Friction Analysis`

### 6.2 Throughout
- "GigaBrain" (product) → "Quaid"
- All `gbrain` commands → `quaid`
- All `GBRAIN_*` → `QUAID_*`
- `brain.db` → `memory.db`
- "The Core Problem: … GigaBrain feels like effort" → "Quaid feels like effort"
- Comparison table: "GigaBrain" column header → "Quaid"
- Recommendation section: `gbrain sync`, `gbrain daemon install`, `gbrain status`,
  `gbrain import` → `quaid` equivalents
- Note: "OpenClaw skill for GigaBrain" → "OpenClaw skill for Quaid"; `gbrain_search`
  function in skill example → `quaid_search` or `memory_search` (use `memory_search`
  to align with MCP rename)

---

## 7. docs/openclaw-harness.md — Full audit

### 7.1 Title and lede (line 1–3)
- `# Using GigaBrain v0.9.6 as an OpenClaw Harness` → `# Using Quaid v0.9.6 as an OpenClaw Harness`
- "GigaBrain works well as the memory and knowledge layer" — interesting: "memory" appears
  here as a concept word, not a brand word. After rename, "Quaid works well as the memory
  layer for agents" — this phrasing becomes more natural, not less.

### 7.2 Prerequisites / init example
- `gbrain init ~/brain.db` → `quaid init ~/memory.db`
- `gbrain v0.9.6` → `quaid v0.9.6`

### 7.3 Collection attach examples
- All `gbrain collection ...` → `quaid collection ...`
- `.gbrainignore` → `.quaidignore`

### 7.4 OpenClaw config JSON (lines 59–71)
- Server key `"gbrain"` → `"quaid"`
- `"command": "gbrain"` → `"command": "quaid"`
- `"GBRAIN_DB"` → `"QUAID_DB"`
- `brain.db` → `memory.db`

### 7.5 Live sync workflow and prose (lines 84–98)
- All `gbrain` refs → `quaid`
- `brain.db` → `memory.db`
- "the GigaBrain MCP tools" → "the Quaid MCP tools"

### 7.6 MCP usage patterns section (lines 99–end)
- `brain_query`, `brain_search`, `brain_put`, `brain_get`, `brain_collections`
  → `memory_query`, `memory_search`, `memory_put`, `memory_get`, `memory_collections`
- "GigaBrain MCP tools" → "Quaid MCP tools"

---

## 8. AGENTS.md — Full audit

### 8.1 Title and description
- `# GigaBrain — Agent Instructions` → `# Quaid — Agent Instructions`
- "Personal knowledge brain" → "Personal knowledge memory"
- `brain.db` → `memory.db`

### 8.2 "What this is" section
- "GigaBrain stores your knowledge" → "Quaid stores your knowledge"
- `brain.db` → `memory.db`

### 8.3 Skill references
- `skills/query/SKILL.md` — "how to search and synthesize across the brain"
  → "how to search and synthesize across memory" (or "across your memory")
  
### 8.4 Key commands
Every `gbrain` command → `quaid`. `~/brain.db` → `~/memory.db`.

### 8.5 Constraints section
- `brain_put` → `memory_put`
- `brain_gap` → `memory_gap`
- `brain_gap_approve` → `memory_gap_approve`

### 8.6 Database schema section
- `brain_config` table → `quaid_config` or `memory_config`
- `page_embeddings_vec_384` (technical; leave for Fry)
- `knowledge_gaps` table name — leave; it's a concept not a brand name

### 8.7 MCP tools section
- All `brain_*` → `memory_*`

### 8.8 Optimistic concurrency section
- `brain_put` → `memory_put`

---

## 9. CLAUDE.md — Full audit

### 9.1 Title
- `# GigaBrain` → `# Quaid`
- "Personal knowledge brain" → "Personal knowledge memory"

### 9.2 Architecture diagram
- `brain.db` → `memory.db`

### 9.3 Key files table
- No product names in filenames; table is fine structurally.
- `brain_config` table name in schema section → flag for Fry (DB-level rename)
- `knowledge_gaps` — conceptual name, keep.

### 9.4 Build section
All references are cargo commands; no product names in the commands. Binary output path
`target/release/gbrain` → `target/release/quaid` only in the comment, not the cargo command.

### 9.5 Embedding model section
- "GigaBrain defaults to …" → "Quaid defaults to …"
- `GBRAIN_MODEL` → `QUAID_MODEL`
- `brain_config` table → flag for Fry

### 9.6 Skills section
- `~/.gbrain/skills/` → `~/.quaid/skills/`

### 9.7 MCP tools section
- All `brain_*` → `memory_*`

### 9.8 Optimistic concurrency
- `brain_put` → `memory_put`

---

## 10. skills/*/SKILL.md — Full audit

All eight skill files have the same pattern of changes needed:

### 10.1 Frontmatter `name:` field
- `gbrain-ingest` → `quaid-ingest`
- `gbrain-query` → `quaid-query`
- `gbrain-maintain` → `quaid-maintain`
- `gbrain-briefing` → `quaid-briefing`
- `gbrain-research` → `quaid-research`
- `gbrain-enrich` → `quaid-enrich`
- `gbrain-alerts` → `quaid-alerts`
- `gbrain-upgrade` → `quaid-upgrade`

### 10.2 Frontmatter `description:` fields
- "Ingest meeting notes … into GigaBrain" → "into Quaid"
- "Answer questions from the brain" → "Answer questions from memory"
- "Maintain brain integrity" → "Maintain memory integrity"
- "Generate a structured 'what shifted' report from the brain" → "from memory"
- "Resolve knowledge gaps … logged in the brain" → "logged in memory"
- "Enrich brain pages" → "Enrich memory pages"
- "monitors brain state" → "monitors memory state"
- "safely replacing the `gbrain` binary" → "safely replacing the `quaid` binary"

### 10.3 All CLI command examples in every skill file
Every `gbrain <command>` → `quaid <command>`.

### 10.4 All MCP tool names
Every `brain_*` → `memory_*` (brain_put, brain_get, brain_gap, brain_raw, brain_query, etc.)

### 10.5 skills/upgrade/SKILL.md — GitHub API URL
- `https://api.github.com/repos/macro88/gigabrain/releases/latest`
  → `https://api.github.com/repos/macro88/quaid/releases/latest`
- Asset filename table: `gbrain-x86_64-…` → `quaid-x86_64-…`
- "Back up existing binary": `$(which gbrain)` → `$(which quaid)`
- Version output example: `gbrain 0.2.0 (commit abc1234)` → `quaid 0.2.0 (commit abc1234)`
- SKILL.md prose: "keeping a `.bak` copy of the previous binary"
  — `gbrain.new` → `quaid.new`

### 10.6 skills/query/SKILL.md — GBRAIN_MODEL
- `GBRAIN_MODEL` → `QUAID_MODEL`
- "Vector semantic … BGE-small-en-v1.5 … via `GBRAIN_MODEL` / `--model`"
  → "via `QUAID_MODEL` / `--model`"

### 10.7 skills/alerts/SKILL.md
- `gbrain check --all` → `quaid check --all`
- `brain_put` / `brain_link` → `memory_put` / `memory_link`

### 10.8 skills/enrich/SKILL.md
- "Enrich brain pages" → "Enrich memory pages"
- `brain_raw` → `memory_raw`
- `gbrain import` → `quaid import`

### 10.9 skills/research/SKILL.md
- "gaps logged in the brain" → "gaps logged in memory"
- `brain_gap_approve` → `memory_gap_approve`
- All `gbrain` command refs → `quaid`

---

## 11. phase2_progress.md — Full audit

This is an internal handoff doc, not a user-facing doc, but it contains terminology:
- `github.com/macro88/gigabrain/pull/22` → update URL if repo is renamed
- MCP tool names listed (`brain_link`, `brain_link_close`, etc.) → `memory_*`
- Note: this file is historical record; depending on policy it may be left as-is
  or updated for consistency. Recommend a note at the top acknowledging the rename
  rather than rewriting the historical record.

---

## 12. docs/contributing.md — Additional items not yet listed above

### 12.1 GitHub labels script (lines 181–197)
- Labels `"squad:fry"`, `"squad:bender"` etc. are team labels, not product names — leave.
- Phase labels `"phase-1"` etc. — leave.

---

## "brain" → "memory" nuance guide

### Always rename "brain" → "memory"
These are clear product-branded uses of "brain" that must become "memory":
- `brain.db` filename → `memory.db`
- "personal knowledge brain" (product tagline) → "personal knowledge memory"
- `brain_config` (SQLite table) → `memory_config` ← flag for Fry; affects schema DDL
- `knowledge_gaps` table — keep as-is; "knowledge" is not a brand word
- All `brain_*` MCP tool names → `memory_*`
- "the brain" when used as "the Quaid database/index" → "memory" or "Quaid"
- `~/.gbrain/` path → `~/.quaid/`
- "GigaBrain" product name → "Quaid"
- `gbrain` binary name → `quaid`

### "brain" phrases that need contextual judgment
These phrases use "brain" as a general English concept, not the product name:
- "personal knowledge brain" (Garry Tan concept) — becomes "personal knowledge memory"
  since that is what Quaid calls the concept now.
- "compiled-truth / brain architecture" — keep "compiled-truth / timeline architecture"
  (no brand word here).
- "knowledge brain" (description of what Quaid is) → "knowledge memory" or just "knowledge store"
  — "knowledge memory" reads better and matches Quaid's chosen concept word.
- "wiki-brain" (generic concept in Why section) — becomes "wiki" or "markdown brain" → leave
  "wiki-brain" as it describes the problem space, not the product.
- "Same brain, different stack" (Acknowledgements) → "Same memory architecture, different stack"
- "Brain health" (section header) → "Memory health" — straight rename.

### "brain" phrases that should NOT be renamed
- "Karpathy's compiled knowledge model" — no brand word; keep.
- "above the line / below the line" architecture description — no brand word; keep.
- References to Garry Tan's "GBrain" as a historical attribution — acknowledge as "Garry
  Tan's GBrain" (his name for his project); do not rename that.

---

## Open questions for the team

1. **Default DB filename:** The task spec says `memory.db` or `quaid.db`. Amy recommends
   **`memory.db`** because: (a) it matches the concept rename "memory"; (b) it is consistent
   with the `QUAID_DB` env var description; (c) `quaid.db` creates a name collision when
   users have multiple Quaid instances. Fry to confirm before any file is edited.

2. **GitHub repo slug:** The task says rename from `macro88/gigabrain`. Amy assumes the
   new slug is `macro88/quaid`. All URL references should be updated together. Confirm
   before editing — URLs that exist in scripts or CI will break if the rename is partial.

3. **`brain_config` SQLite table:** Renaming this is a schema migration. Fry must confirm
   the DDL change and migration strategy before Amy updates docs that reference it.

4. **Cargo feature flag names (`embedded-model`, `online-model`, `bundled`):** These are
   Rust source-level identifiers. Amy will update any docs that expose them to users
   only if Fry renames them in Cargo.toml. They are not a docs-only concern.

5. **`phase2_progress.md`:** Internal handoff doc with historical MCP tool names. Recommend
   adding a rename notice at the top rather than rewriting tool names throughout.

6. **`docs/gigabrain-vs-qmd-friction-analysis.md`:** This doc uses GigaBrain as the
   product name throughout and also references `gbrain_search` as a hypothetical skill
   function. Confirm whether this is a living user-facing reference or an archived analysis.
   If archived, a rename notice at the top may be sufficient.

---

## File-by-file summary of change volume

| File | Occurrences (approx.) | Rename type |
|------|-----------------------|-------------|
| `README.md` | ~80 | Product, CLI, MCP tools, env vars, paths, URLs |
| `docs/getting-started.md` | ~70 | Product, CLI, MCP tools, env vars, paths |
| `docs/roadmap.md` | ~40 | Product, CLI, MCP tools, env vars |
| `docs/contributing.md` | ~30 | Product, CLI, asset names, npm package |
| `docs/spec.md` | ~200+ (large file) | Product, CLI, MCP tools, env vars, schema |
| `docs/openclaw-harness.md` | ~35 | Product, CLI, MCP tools, env vars, paths |
| `docs/gigabrain-vs-qmd-friction-analysis.md` | ~25 | Product, CLI, comparison table |
| `AGENTS.md` | ~25 | Product, CLI, MCP tools, schema, paths |
| `CLAUDE.md` | ~20 | Product, CLI, MCP tools, env vars, paths |
| `skills/ingest/SKILL.md` | ~15 | CLI commands |
| `skills/query/SKILL.md` | ~15 | CLI commands, env var |
| `skills/maintain/SKILL.md` | ~5 | CLI commands |
| `skills/briefing/SKILL.md` | ~8 | CLI commands |
| `skills/research/SKILL.md` | ~10 | CLI commands, MCP tools |
| `skills/enrich/SKILL.md` | ~12 | CLI commands, MCP tools |
| `skills/alerts/SKILL.md` | ~10 | CLI commands, MCP tools |
| `skills/upgrade/SKILL.md` | ~15 | CLI commands, asset names, API URL |
| `phase2_progress.md` | ~10 | MCP tools, URL |

---

## Decisions logged

**Decision 1:** Use `memory.db` as the recommended default DB filename (not `quaid.db`).
Rationale: aligns with the "memory" concept word and avoids a product-name collision in
multi-instance setups. Pending Fry confirmation.

**Decision 2:** The `brain_config` SQLite table is a schema-level concern. Docs will
reference the new name only after Fry confirms the DDL rename and migration strategy.

**Decision 3:** Garry Tan's "GBrain" attribution is preserved as a historical reference —
it refers to his project, not ours. All references to "GigaBrain" (our product) are renamed;
the Garry Tan credit stands.

**Decision 4:** `phase2_progress.md` will receive a rename notice at the top rather than
a full retroactive rewrite of historical tool names.

**Decision 5:** The `.gbrainignore` file rename to `.quaidignore` must be coordinated with
Fry as it affects file-system conventions used by the reconciler and watcher.

# Decision: Release note body — breaking-rename warning fix

**Author:** Amy  
**Date:** 2026-04-25  
**File changed:** `.github/workflows/release.yml`  
**Triggered by:** Nibbler rejection of final release approval

---

## Decisions recorded

### 1. Breaking-rename callout must open the release body — not follow install instructions

The previous body put `## Install` first, relegating all context to a single prose paragraph
at the bottom. Nibbler's rejection was correct: a user scanning the release page sees the
install commands before they see that this is a hard-breaking change. The new body opens with
`## ⚠️ BREAKING RENAME — Read before upgrading` and a four-row table of required user actions,
making it impossible to miss on any device or email preview.

### 2. Migration guide pointer is mandatory in release notes for this rename

`MIGRATION.md` exists and is thorough. Not linking to it in the release notes forced users to
find it by browsing the repo. The new body includes a prominent bolded pointer immediately
after the breaking-change table: `**→ Full step-by-step migration instructions: [MIGRATION.md](...)**`.

### 3. "npm installs the `online` channel" removed; replaced with explicit deferred note

The old prose stated "npm installs the `online` channel" as a present-tense fact. npm is
**not** a live install path for `quaid` (see `RELEASE_CHECKLIST.md` deferred-channels gate
and `MIGRATION.md` npm section). This is a factual error in release messaging that would
mislead users into running `npm install -g quaid` and getting nothing. Replaced with a
blockquote: *"npm — planned follow-on, not yet available. `quaid` is not in the public npm
registry. Use the shell installer or a GitHub Releases binary above."*

### 4. "This patch release" framing removed

The phrase "This patch release" understated the severity of the change. A hard rename that
breaks every user-facing surface, invalidates all existing databases, and requires manual
MCP client reconfiguration is not a patch in any user-visible sense. The changelog prose was
rewritten to open with the rename context and present the Issue #81 fix as a secondary item.

### 5. No overlap with Zapp's RELEASE_CHECKLIST.md scope

`.github/RELEASE_CHECKLIST.md` was left untouched. Zapp owns that file's sign-off gates.
All changes were confined to the `body:` block of the `Create release` step in
`.github/workflows/release.yml`.

---

## Files changed

- `.github/workflows/release.yml` — `body:` block of the `Create release` step rewritten

# Amy — Residual Rename Cleanup Decisions

**Date:** 2026-04-25  
**Task:** quaid-hard-rename residual audit — docs, openspec, benchmarks  
**Requested by:** macro88

---

## Decisions

### 1. `docs/gigabrain-vs-qmd-friction-analysis.md` renamed to `docs/quaid-vs-qmd-friction-analysis.md`

**Decision:** Renamed the file. The content already used "Quaid" throughout and contained no old-brand product strings. Only the filename was stale. No cross-file references to the old filename were found (the one reference in `openspec/changes/quaid-hard-rename/tasks.md` is in the tasks list under G.7, which was already marked `[x]`).

---

### 2. "personal knowledge brain" replaced with "personal AI memory" across all Amy surfaces

**Decision:** Replaced the product tagline "personal knowledge brain" with "personal AI memory layer" (prose) or "personal AI memory" (short form) in:
- `README.md` (tagline)
- `CLAUDE.md` (lede line)
- `docs/spec.md` (frontmatter title, H1, lede quote, embedded CLAUDE.md block)
- `docs/getting-started.md` (lede)
- `docs/contributing.md` (intro paragraph)

**Rationale:** "knowledge brain" is the old product-concept term. "AI memory" is the Quaid-aligned term consistent with the `memory_*` MCP tool names and `memory.db` default filename. Generic domain uses of "personal knowledge base" (e.g., in retrieval quality discussion in spec.md) were left untouched — those refer to the domain category, not the product.

---

### 3. `docs/openclaw-harness.md` — comprehensive `gbrain`/`GigaBrain` cleanup

**Decision:** Full pass on the file. All changes applied:
- `GigaBrain` → `Quaid`
- `gbrain` CLI commands → `quaid`
- `brain.db` → `memory.db`
- `GBRAIN_DB` → `QUAID_DB`
- `.gbrainignore` → `.quaidignore`
- `brain_collections`, `brain_query`, `brain_search`, `brain_put`, `brain_get` → `memory_*` equivalents
- "SQLite brain" → "SQLite memory store"

**Rationale:** This file was the most visibly stale in the docs/ directory — it used GigaBrain branding throughout. Now reads as if Quaid was always the name.

---

### 4. `openspec/` prose updated (J.2 confirmed)

**Decision:** Full pass across all `openspec/changes/` subdirectories (including `archive/`) excluding `quaid-hard-rename/` (which documents the rename itself). Zero remaining occurrences of `gbrain`, `GigaBrain`, `brain.db`, `brain_config`, or `GBRAIN_` confirmed by post-pass scan.

J.2 in `openspec/changes/quaid-hard-rename/tasks.md` was already marked `[x]` and is now verified complete.

---

### 5. `AGENTS.md` — two minor old-concept fixes

**Decision:** Updated two comment strings:
- "search and synthesize across the brain" → "search and synthesize across memory"
- "create new brain" (code comment) → "create new memory store"

These were not product-name strings but used "brain" as a product concept, inconsistent with the Quaid memory model.

---

### 6. `benchmarks/README.md` — synthetic query fixture updated

**Decision:** Changed query 3 from "knowledge brain sqlite embeddings" to "quaid memory sqlite embeddings" to remove the old product-concept phrasing from the documented baseline fixture. The `projects/quaid` result references were already updated by prior work.

---

## Items NOT changed (intentional)

- `docs/spec.md` lines 2655, 2661, 2714, 2924: "personal knowledge base" in retrieval quality discussion — domain category term, not product name. Left as-is.
- `docs/spec.md` line 28: "Inspired by Garry Tan's GBrain work" — historical attribution citation. Left as-is.
- `.squad/` — explicitly out of scope per task instructions.
- `website/` — owned by Hermes; not in Amy's surfaces.

# Decision: `quaid call` doc truth fix — `memory_collections` exclusion

**Author:** Bender  
**Date:** 2026-04-25  
**Context:** quaid-hard-rename — Nibbler reviewer blocker on `docs/getting-started.md`

## Finding

`docs/getting-started.md` §"Raw MCP tool invocation" claimed `quaid call` could invoke **any** MCP tool. This was false. `src/commands/call.rs` `dispatch_tool()` contains a 16-arm `match` covering:

`memory_get`, `memory_put`, `memory_query`, `memory_search`, `memory_list`, `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph`, `memory_check`, `memory_timeline`, `memory_tags`, `memory_gap`, `memory_gaps`, `memory_stats`, `memory_raw`

The 17th tool, `memory_collections`, is implemented in `src/mcp/server.rs` (method `memory_collections`, line ~1327) but has no arm in `dispatch_tool()`. Passing `memory_collections` to `quaid call` falls through to `_ => Err("unknown tool: ...")`.

## Decision

Fix the doc wording only (no code change). Changed:

> "Call any MCP tool directly from the CLI without starting the server:"

to:

> "Call MCP tools directly from the CLI without starting the server. The dispatcher covers the 16 stateless tools; `memory_collections` requires `quaid serve` and is not available via `quaid call`."

## Rationale

`memory_collections` is a vault-sync serve-side tool (reads live watcher/collection state from a running serve context). Wiring it into the headless `quaid call` dispatcher is a separate implementation decision that belongs to the vault-sync lane (Fry/Professor), not a docs cycle. The doc must not claim broader coverage than the shipped code provides.

## Follow-on

If vault-sync owners decide to add `memory_collections` to the `call` dispatcher, the doc sentence should be updated to remove the exclusion note.

# Bender — Migration Blocker Fix

**Date:** 2026-04-27  
**Author:** Bender  
**Scope:** Fix two Professor-rejected artifacts (`upgrade.mdx`, `schema.sql`) and create the missing `MIGRATION.md`.

---

## What Was Broken

### 1. `src/schema.sql` — header version mismatch

The file header read `-- memory.db schema — Quaid v5` while `SCHEMA_VERSION` in
`src/core/db.rs` is `6`. Anyone reading the SQL file saw a version number that
contradicted the runtime constant. Professor's rejection was correct.

**Fix:** Header updated to `-- memory.db schema — Quaid v6`.

---

### 2. `website/src/content/docs/how-to/upgrade.mdx` — two false claims

**Claim A:** Line 50 told users the current schema version is `5`. It is `6`.  
**Fix:** Updated to `currently \`6\``.

**Claim B:** The hard-rename `<Aside>` directed users to "`MIGRATION.md` at the
repository root", which did not exist (tasks.md Phase K item K.1 was unchecked).  
**Fix:** The reference now links to the file on GitHub
(`https://github.com/quaid-app/quaid/blob/main/MIGRATION.md`), and `MIGRATION.md`
has been created at the repository root (see below).

---

### 3. `MIGRATION.md` — did not exist (K.1 was unchecked)

Created `MIGRATION.md` at the repository root covering all surfaces listed in K.1:

- Binary rename (`gbrain` → `quaid`)
- Env var table (`GBRAIN_*` → `QUAID_*`, all 8 user-facing vars)
- MCP tool rename table (all 17 tools, `brain_*` → `memory_*`)
- DB migration path (export with old binary → `quaid init` → `quaid import`)
- Default DB path change (`~/.gbrain/brain.db` → `~/.quaid/memory.db`)
- npm package rename
- MCP client config update instructions

---

## Decision

The upgrade doc and schema header are now internally consistent and point only to
files that exist. `MIGRATION.md` satisfies task K.1 from the quaid-hard-rename
checklist. Tasks K.1 can be marked done; L.3 (PR) remains open.

No other artifacts were modified. Fry, Hermes, Amy, and Zapp remain locked out of
revising these artifacts for this cycle per the reviewer rejection context.

---

## Files changed

| File | Change |
|------|--------|
| `src/schema.sql` | Header: `v5` → `v6` |
| `website/src/content/docs/how-to/upgrade.mdx` | Schema version `5` → `6`; `MIGRATION.md` bare path → GitHub URL link |
| `MIGRATION.md` | Created (was missing; K.1 deliverable) |

# Bender Validation Audit — Quaid Hard Rename

**Date:** 2026-04-25  
**Auditor:** Bender  
**Scope:** Read-only validation audit for `openspec/changes/quaid-hard-rename`. No code changes.  
**Verdict:** OPEN — Rename is implementable, but several gaps in the proposal will make it incomplete even if the code compiles.

---

## Summary Judgment

The proposal (`proposal.md`, `tasks.md`) covers the obvious surfaces correctly. However, four issues are **silent-failure risks** — the codebase will still compile while the rename is structurally incomplete. The test suite has hardcoded string assertions that must be updated atomically with the source changes or CI will gate incorrectly. A fifth issue (`gbrain_id`) is a scope question that requires an explicit decision before implementation starts.

---

## Critical Gaps (implementation WILL be incomplete unless addressed)

### GAP-1: `gbrain_id` frontmatter field — not covered by the proposal

`src/core/page_uuid.rs` reads and validates a `gbrain_id:` key from page YAML frontmatter. `src/core/markdown.rs` emits it on export. The `tests/roundtrip_raw.rs` byte-exact fixture literally contains `gbrain_id: 01969f11-9448-7d79-8d3f-c68f54761234\n` as the first frontmatter line. This field is user-visible data embedded in every exported page file on disk.

The proposal's non-goals say "Renaming internal non-surface tables" — but `gbrain_id` is neither internal nor a table. It is a key in every page's YAML frontmatter. If it stays as `gbrain_id` after the rename, every page a user exports from `quaid` will still contain `gbrain_id:`.

**Decision required:** Is `gbrain_id` → something else (e.g., `quaid_id`, `memory_id`) in scope? If yes, the roundtrip_raw fixture must change and it is a data-migration breaking change for anyone who has pages in their brain already. If no, document why the user-visible frontmatter key retains the old product name.

**Files affected:** `src/core/page_uuid.rs`, `src/core/markdown.rs`, `src/core/types.rs` (test), `src/mcp/server.rs` (test fixture), `tests/roundtrip_raw.rs` (canonical fixture), `tests/roundtrip_semantic.rs`.

---

### GAP-2: Crate library name — `use gbrain::` across all integration tests

`src/lib.rs` exposes the crate as `gbrain`. Once `Cargo.toml` renames `name = "gbrain"` to `name = "quaid"`, every file that uses `use gbrain::` will fail to compile:

- `tests/roundtrip_raw.rs` — `use gbrain::core::db;`
- `tests/roundtrip_semantic.rs` — `use gbrain::core::db;`
- `tests/graph.rs`, `tests/assertions.rs`, `tests/collection_cli_truth.rs`, `tests/corpus_reality.rs`, `tests/concurrency_stress.rs`, `tests/embedding_migration.rs`, `tests/search_hardening.rs`, `tests/beir_eval.rs`, `tests/quarantine_revision_fixes.rs`, `tests/model_selection.rs`, `tests/watcher_core.rs`

The proposal's Phase I audit command `rg "gbrain|brain_config|GBRAIN_" tests/ src/ --type rust` will catch these, but the Phase C task description does not explicitly call out `use gbrain::` imports as a consequence of the crate rename. Fry must know to update all `use gbrain::` → `use quaid::` in test files at the same time as `Cargo.toml`.

**This is a build-gate failure, not just a test failure.** The crate rename is atomic with the import updates.

---

### GAP-3: Build-time env vars in `build.rs` and `src/core/inference.rs` — partially listed but not fully mapped

`build.rs` sets three `cargo:rustc-env` variables at compile time:
- `GBRAIN_EMBEDDED_CONFIG_PATH`
- `GBRAIN_EMBEDDED_TOKENIZER_PATH`
- `GBRAIN_EMBEDDED_MODEL_PATH`

`src/core/inference.rs` reads them via `env!(...)`. `build.rs` also checks `GBRAIN_EMBEDDED_MODEL_DIR` and `GBRAIN_MODEL_DIR` as candidate model directory overrides, and hard-codes the default model cache lookup at `~/.gbrain/models/bge-small-en-v1.5`.

Additionally, `src/core/inference.rs` uses:
- `GBRAIN_FORCE_HASH_SHIM` — CI test env var (used in `.github/workflows/ci.yml` as `GBRAIN_FORCE_HASH_SHIM: "1"`)
- `GBRAIN_HF_BASE_URL` — runtime HuggingFace base URL override
- `GBRAIN_MODEL_CACHE_DIR` — runtime model cache directory override

The proposal's Phase E env-var list (`proposal.md` table) covers the user-facing install vars but omits these six internal/build-time vars. All must be renamed for consistency.

**CI will break**: `ci.yml` line 114 sets `GBRAIN_FORCE_HASH_SHIM: "1"` for the online-channel test run. If `inference.rs` is renamed to `QUAID_FORCE_HASH_SHIM` but the workflow still sets `GBRAIN_FORCE_HASH_SHIM`, the shim guard will silently stop working and the online-model test will attempt live downloads.

---

### GAP-4: `tests/install_profile.sh` has 14+ hardcoded `GBRAIN_DB` string assertions

`install_profile.sh` tests T5, T6, T15, T16, T19, and others call `grep -Fq "export GBRAIN_DB="` against profile file contents. After `install.sh` is updated to write `QUAID_DB`, these tests will fail.

Additionally `install_profile.sh` itself is sourced by `main()` tests with `GBRAIN_TEST_MODE=1`, `GBRAIN_CHANNEL=`, `GBRAIN_NO_PROFILE=`, etc. — all of which must be updated.

Similarly, `tests/install_release_seam.sh` uses `GBRAIN_TEST_MODE`, `GBRAIN_RELEASE_API_URL`, `GBRAIN_RELEASE_BASE_URL`, `GBRAIN_INSTALL_DIR`, `GBRAIN_CHANNEL`, `GBRAIN_VERSION` as hard-coded env var names in the test setup. All 6 must change to `QUAID_*`.

The proposal Phase I says "audit all test files" but does not call out that the shell test env var names are implementation-coupled assertions — not just CLI invocations.

---

### GAP-5: `tests/release_asset_parity.sh` — hardcoded `gbrain-` prefix checks

`release_asset_parity.sh` T1 calls `simulate_asset "$platform" "$channel"` which formats `gbrain-%s-%s`. T2 checks `artifact: ${name}` in release.yml. T4 counts `artifact: gbrain-` lines. T5 asserts exactly 17 manifest entries. T7 checks for `'gbrain-${PLATFORM}-${CHANNEL}'` in `install.sh`. T8 checks for `gbrain-<platform>-<channel>` in spec docs.

If the rename changes all artifacts to `quaid-*`, ALL of these assertions flip. The test itself is a contract checker — it must be updated in lock-step with the manifest, workflow, and installer.

---

## High-Risk Regressions to Verify

### R-1: Schema breaking change — `SCHEMA_VERSION` bump is mandatory, not optional

`src/core/db.rs` line 15: `const SCHEMA_VERSION: i64 = 5;`. The `open_with_model()` call checks stored schema version against `SCHEMA_VERSION`. If `brain_config` is renamed to `quaid_config` in `schema.sql` but `SCHEMA_VERSION` is not bumped, the `table_exists(conn, "brain_config")` fallback path in `read_brain_config` will silently treat the renamed table as a legacy DB. This must be bumped to 6 atomically with the DDL rename.

### R-2: `format_model_mismatch` error message — hardcoded tool names

`src/core/db.rs` lines 523, 536 format error messages containing:
- `"rm {} && gbrain init"` 
- `"GBRAIN_MODEL={} gbrain <command>"`

After the rename these must say `quaid init` and `QUAID_MODEL={} quaid <command>`. Tests in `db.rs` lines 1034 and 1048 assert these exact substrings — they will fail (correctly) if the production message is updated but the assertion strings are not updated to match.

### R-3: MCP tool names are derived from Rust method names by `rmcp` proc macro

The `#[tool(tool_box)]` and `#[tool(description = "...")]` proc macros in `rmcp` derive the protocol-visible tool name from the Rust method name. There are NO separate `name = "..."` attributes on the `brain_*` methods. Renaming the Rust method `brain_get` → `memory_get` IS the protocol rename. Implementation must rename both in the same commit. Partial renaming (e.g., Rust method renamed, struct name `GigaBrainServer` not renamed) will not break compilation but is inconsistent. The `GigaBrainServer` struct name and the `"GigaBrain personal knowledge brain"` server instructions string in `get_info()` must both be updated.

### R-4: `build.rs` user-agent string

`build.rs` line 88: `user_agent("gigabrain-build/0.9.2")` — this is the HTTP user-agent sent to HuggingFace during model download. It hardcodes the old product name and an old version. Must be updated to `quaid-build/<version>` (or use `env!("CARGO_PKG_VERSION")`).

### R-5: `.gitignore` must be updated

`.gitignore` currently ignores `/brain.db`, `/brain.db-journal`, `/brain.db-shm`, `/brain.db-wal` and `packages/gbrain-npm/bin/gbrain.bin` / `gbrain.download`. After rename these should ignore `memory.db` (or `quaid.db`) variants and `quaid-npm/bin/quaid.bin`.

If `.gitignore` is not updated, the new default DB filename will not be ignored and may accidentally get committed to git.

### R-6: `packages/gbrain-npm/` directory and npm package

The npm package directory is `packages/gbrain-npm/`. `package.json` has:
- `"name": "gbrain"` → must become `"name": "quaid"` (or `@quaid-app/quaid`)
- `"description": "GigaBrain..."` → updated
- `"bin": { "gbrain": "bin/gbrain" }` → `"bin": { "quaid": "bin/quaid" }`
- `"files": ["bin/gbrain", ...]` → `"files": ["bin/quaid", ...]`
- `"repository": "macro88/gigabrain"` → `"quaid-app/quaid"`

`postinstall.js` uses `GBRAIN_REPO`, `GBRAIN_RELEASE_BASE_URL`, `GBRAIN_RELEASE_TAG_URL` env vars, constructs asset names like `gbrain-darwin-arm64-online`, logs `[gbrain]` prefix, writes to `bin/gbrain.bin`. The `publish-npm.yml` workflow points at `packages/gbrain-npm` — if the directory is renamed, the workflow path must update.

### R-7: `src/core/quarantine.rs`, `src/core/reconciler.rs`, `src/core/raw_imports.rs` — contain `GBRAIN_*` env refs

These files contain `GBRAIN_*` env var references (from the grep count output). Phase E must not stop at `scripts/install.sh` — `rg "GBRAIN_"` across the full `src/` tree must be run and all hits addressed.

---

## Verification Checklist

Implementation must prove all of the following before the PR is considered complete:

### Build Gate
- [ ] `cargo build --release` succeeds and produces a binary named `quaid` (not `gbrain`)
- [ ] `cargo build --release --no-default-features --features bundled,online-model` succeeds
- [ ] `cargo check --all-targets` produces zero errors

### Test Gate
- [ ] `cargo test` passes with zero failures (default / airgapped channel)
- [ ] `cargo test --no-default-features --features bundled,online-model` passes (with `QUAID_FORCE_HASH_SHIM=1`)
- [ ] `cargo test --test roundtrip_raw --test roundtrip_semantic` passes (uses `use quaid::`, fixture has correct frontmatter key for `gbrain_id` or confirmed no-change)
- [ ] `sh tests/install_profile.sh` passes (all `GBRAIN_DB` string checks updated to `QUAID_DB`)
- [ ] `sh tests/release_asset_parity.sh` passes (all `gbrain-` prefix checks updated to `quaid-`)
- [ ] `sh tests/install_release_seam.sh` passes (all `GBRAIN_*` env var setup updated to `QUAID_*`)

### Schema Gate
- [ ] `SCHEMA_VERSION` has been bumped (5 → 6)
- [ ] `brain_config` table DDL in `src/schema.sql` renamed to `quaid_config`
- [ ] All `brain_config` SQL string literals in `src/core/db.rs` updated to `quaid_config`
- [ ] Functions `read_brain_config`, `write_brain_config` renamed to `read_quaid_config`, `write_quaid_config`
- [ ] Error messages in `db.rs` say `quaid init` not `gbrain init`

### MCP Gate
- [ ] `GigaBrainServer` struct renamed (suggested: `QuaidServer` or `MemoryServer`)
- [ ] All 17 `brain_*` Rust methods renamed to `memory_*`
- [ ] Server instructions string updated from `"GigaBrain personal knowledge brain"` to `"Quaid personal memory"`
- [ ] `BrainGetInput`, `BrainPutInput`, etc. struct names updated (consistency — these are exposed in tool schemas)

### Env Var Gate (exhaustive)
- [ ] `GBRAIN_DB` → `QUAID_DB`
- [ ] `GBRAIN_MODEL` → `QUAID_MODEL`
- [ ] `GBRAIN_CHANNEL` → `QUAID_CHANNEL`
- [ ] `GBRAIN_INSTALL_DIR` → `QUAID_INSTALL_DIR`
- [ ] `GBRAIN_VERSION` → `QUAID_VERSION`
- [ ] `GBRAIN_NO_PROFILE` → `QUAID_NO_PROFILE`
- [ ] `GBRAIN_RELEASE_API_URL` → `QUAID_RELEASE_API_URL`
- [ ] `GBRAIN_RELEASE_BASE_URL` → `QUAID_RELEASE_BASE_URL`
- [ ] `GBRAIN_FORCE_HASH_SHIM` → `QUAID_FORCE_HASH_SHIM` (internal/test; also update `ci.yml`)
- [ ] `GBRAIN_HF_BASE_URL` → `QUAID_HF_BASE_URL` (internal)
- [ ] `GBRAIN_MODEL_CACHE_DIR` → `QUAID_MODEL_CACHE_DIR` (internal)
- [ ] `GBRAIN_EMBEDDED_MODEL_DIR` → `QUAID_EMBEDDED_MODEL_DIR` (build)
- [ ] `GBRAIN_MODEL_DIR` → `QUAID_MODEL_DIR` (build)
- [ ] `GBRAIN_EMBEDDED_CONFIG_PATH` / `_TOKENIZER_PATH` / `_MODEL_PATH` (build-time rustc-env) → `QUAID_*`

### CI / Release Gate
- [ ] `.github/workflows/release.yml` artifact matrix uses `quaid-*` prefix
- [ ] `.github/workflows/ci.yml` updates `GBRAIN_FORCE_HASH_SHIM` env var
- [ ] `.github/release-assets.txt` all 16 binary entries updated to `quaid-*`
- [ ] `.github/RELEASE_CHECKLIST.md` updated
- [ ] Release body install instructions use `quaid-app/quaid`, `QUAID_CHANNEL`, `quaid-<platform>-<channel>` naming

### Install Script Gate
- [ ] `scripts/install.sh` `REPO` updated to `quaid-app/quaid`
- [ ] All `GBRAIN_*` vars renamed to `QUAID_*` in install.sh
- [ ] Asset naming changed from `gbrain-${PLATFORM}-${CHANNEL}` to `quaid-${PLATFORM}-${CHANNEL}`
- [ ] Profile injection lines updated to write `QUAID_DB` not `GBRAIN_DB`
- [ ] Install target path `${INSTALL_DIR}/gbrain` → `${INSTALL_DIR}/quaid`
- [ ] Printed messages say `quaid` not `gbrain`

### Final Audit Grep (must be zero hits outside `.squad/` history files)
```
rg -i "gbrain|gigabrain|brain\.db|brain_config|GBRAIN_" \
  --type-add "text:*.{rs,md,toml,sh,yml,yaml,json,mjs,ts,js}" -t text -l \
  --glob "!.squad/agents/*/history.md" \
  --glob "!.squad/decisions*.md" \
  --glob "!openspec/changes/quaid-hard-rename/**"
```
Expected: zero files. Any file that appears in this output represents an incomplete rename.

---

## Scope Question for macro88

**`gbrain_id` frontmatter key** — The proposal does not address this. It appears in every exported page's YAML. Before implementation begins, macro88 must decide:

1. **Rename it** (e.g., `quaid_id` or `memory_id`) — breaking change for anyone with existing pages. `roundtrip_raw.rs` fixture must change.
2. **Leave it as `gbrain_id`** — product is named Quaid but every page carries a `gbrain_id:` key. Document this explicitly as "internal identifier, not renamed for migration safety."

This decision gates the Phase B schema work. If it's renamed, the `page_uuid` validation code, the `markdown.rs` render path, and several test fixtures all change. If it's left alone, Phase I can skip `gbrain_id` patterns explicitly.

---

## Risky Regressions Not in the Proposal Tasks

| Risk | Location | Why Dangerous |
|------|----------|---------------|
| `GBRAIN_FORCE_HASH_SHIM` not renamed in CI | `ci.yml` line 114 + `inference.rs` | Online-model CI test silently runs live downloads |
| `build.rs` `user_agent` string | `build.rs` line 88 | Old product name in HF download headers (cosmetic but embarrassing post-release) |
| `.gitignore` not updated | `.gitignore` | `memory.db` or `quaid.db` gets committed accidentally |
| `packages/gbrain-npm/` dir not renamed | `publish-npm.yml` | npm publish workflow path breaks if dir is renamed without updating workflow |
| `BrainGetInput` etc. struct names | `src/mcp/server.rs` | These names appear in MCP tool JSON schemas — users who introspect tool schemas see old branding |
| `default_db_path` in `inference.rs` | `~/.gbrain` cache | Online model downloads cache to `~/.gbrain` not `~/.quaid` |
| `src/commands/version.rs` | `println!("gbrain {}")` | `quaid version` prints `gbrain 0.x.x` |

# Decision: docs/spec.md truth repair for default DB path and release asset naming

**Author:** Bender  
**Date:** 2026-04-25  
**Change:** quaid-hard-rename  
**Triggered by:** Nibbler reviewer rejection — spec.md misstated default DB path and release asset naming convention in user-breaking ways.

---

## What was wrong

Six locations in `docs/spec.md` contained stale contract claims after the hard rename:

| Location | Stale | Correct |
|---|---|---|
| CLI help block (`--db` default) | `./memory.db` | `~/.quaid/memory.db` |
| DB path resolution list (item 3) | `./memory.db in current directory` | `~/.quaid/memory.db` |
| Usage examples (`init` / `import`) | `~/my-memory.db`, `~/memory.db` | `~/.quaid/memory.db` |
| Upgrade skill pre-check step 2 | `${QUAID_DB:-./memory.db}` | `${QUAID_DB:-~/.quaid/memory.db}` |
| Upgrade skill download block | `quaid-${PLATFORM}` (no channel suffix) | `quaid-${PLATFORM}-${CHANNEL}` |
| Rollback step DB_PATH resolve | `${QUAID_DB:-./memory.db}` | `${QUAID_DB:-~/.quaid/memory.db}` |
| Quick install example | `quaid-${PLATFORM}` (no channel suffix) | `quaid-${PLATFORM}-${CHANNEL}` |

## Validation performed

- `src/core/db.rs` `default_db_path()` confirmed: `~/.quaid/memory.db` with `memory.db` fallback (no-HOME case only).
- `.github/release-assets.txt` confirmed: all assets follow `quaid-<platform>-<channel>` naming (e.g., `quaid-darwin-arm64-airgapped`). No unsuffixed assets exist.
- `.github/workflows/release.yml` confirmed: every matrix entry sets `artifact: quaid-<platform>-<channel>` explicitly.
- Full forbidden-literal scan of `docs/spec.md`: zero `gigabrain`, `gbrain`, `brain_` occurrences.

## Decision

Apply surgical fixes to `docs/spec.md` only. README and docs/getting-started.md correctly point to `docs/spec.md` as the authority and already use the correct `~/.quaid/memory.db` default; no changes needed there after the spec is repaired.

The upgrade skill and quick-install download blocks now use an explicit `CHANNEL` variable (defaulting to `airgapped`) instead of a bare `quaid-${PLATFORM}` string, matching the canonical asset manifest.

### 2026-04-26T21:17:23+08:00: User directive
**By:** macro88 (via Copilot)
**What:** Hard-reset rename the project from GigaBrain/gbrain/brain to Quaid/quaid/memory everywhere; no backward compatibility, aliases, shims, or migration layers. Use Quaid for the product, quaid for the CLI, memory for the concept, QUAID_DB for the env var, ~/.quaid for config storage, memory.db as the default database file, and memory_* for MCP tool names.
**Why:** User request — captured for team memory

# Fry — issue #79 / #80 release lane decision

- Date: 2026-04-25
- Scope: release/v0.9.7, issues #79 and #80

## Decision

Use `.github/release-assets.txt` as the single authoritative public release-asset manifest for v0.9.7.

## Why

Issue #79 was not an installer lookup bug in isolation; it was a release-contract drift / partial-release problem amplified by the macOS build break from #80. A checked-in manifest lets the release workflow fail closed on missing assets, keeps installer seam tests honest, and gives docs/reviewer surfaces one exact contract to reference instead of repeating handwritten lists.

## Consequences

- Release verification now reads the manifest instead of maintaining an inline expected array in `release.yml`.
- Release seam/parity tests validate installer/workflow/doc truth against that same manifest.
- Checklist/spec/docs should point to the manifest when naming the public artifact family.

# Fry — Issue #81 release-ready decision

- **Date:** 2026-04-25
- **Decision:** Ship PR #84 as patch release `v0.9.8`, and rewrite the GitHub release body to describe the empty-root watcher hotfix instead of reusing the prior `v0.9.7` macOS release-contract notes.
- **Why:** The code fix in PR #84 is already the next patch candidate, so leaving product/version surfaces at `0.9.7` would mislabel the release and stale release-note prose would describe the wrong user-visible repair.
- **Scope touched:** `Cargo.toml`, `Cargo.lock`, `packages/gbrain-npm/package.json`, `src/core/inference.rs`, `README.md`, `docs/getting-started.md`, `website/src/content/docs/guides/install.mdx`, `.github/workflows/release.yml`.

# Fry — Issue #81 watcher empty-root decision

- Decision: treat any `collections` row with `state='active'` and blank `root_path` as invalid legacy state during serve watcher bootstrap, and normalize it to `state='detached'` before watcher registration.
- Why: the crash surface happens before any filesystem work worth preserving; demoting the row is safer than attempting to watch an empty path or silently keeping an impossible active root around.
- Implementation seam: `src/core/vault_sync.rs::detach_active_collections_with_empty_root_path()` runs from watcher sync, logs `WARN: serve_detached_empty_root ...`, and leaves watcher selection gated on `trim(root_path) != ''`.

# 2026-04-26: Hard rename runtime cutover

**By:** Fry

**What:** Implement the hard rename with no runtime legacy bridge: product `Quaid`, CLI `quaid`, user-facing concept `memory`, config dir `~/.quaid`, default database `memory.db`, env vars `QUAID_*`, MCP tools `memory_*`, and frontmatter UUID field `memory_id`.

**Why:** The rename scope explicitly rejected aliases and shims. Existing pre-rename databases now fail closed unless re-exported and re-imported into a fresh Quaid database, which keeps the runtime surface honest and avoids hidden compatibility layers in schema/model metadata handling.

**Consequences:**
- Missing `quaid_config` on an existing database is treated as a schema-mismatch error, not a small-model fallback.
- `quaid init` creates the parent directory for the default `~/.quaid\memory.db` path so the new default works on first use.
- Package, installer, release workflow, and test surfaces all align on `quaid-*` assets and `QUAID_*` variables.

# Fry — residual rename cleanup

- Decision: treat `openspec/changes/quaid-hard-rename/` as the only intentional non-website location where legacy `GigaBrain`/`gbrain`/`brain_*` names may remain, because that change directory is the migration source of truth and must preserve the old-to-new mapping.
- Action taken: cleaned remaining implementation-owned residuals in code, tests, scripts, benchmarks, and non-rename OpenSpec change/spec files so focused rename scans now isolate to the migration spec itself.

# hermes-quaid-rename-audit

**By:** Hermes  
**Date:** 2026-04-25  
**Status:** Audit complete — no edits made  
**Task:** Read-only audit of all docs-site and website-facing surfaces requiring rename from GigaBrain/gbrain/brain → Quaid/quaid/memory

---

## Scope of rename (docs-site surfaces only)

The rename touches every public-facing doc surface. Raw counts across `website/src/content/docs/` (35 files), `README.md`, and `docs/*.md`:

| Term | Count |
|------|-------|
| `GigaBrain` (product name) | ~106 |
| `gbrain` (CLI binary) | ~682 |
| `brain.db` (default DB filename) | ~129 |
| `brain_*` (MCP tool prefix) | ~435 |
| `GBRAIN_*` (env vars) | ~69 |
| `macro88/gigabrain` (GitHub repo URL) | ~29 |
| `~/brain` (default DB path pattern) | ~86 |

---

## File-level inventory

### website/astro.config.mjs
- `title: "GigaBrain"` → `title: "Quaid"`
- `description:` tagline → update product name
- `href: "https://github.com/macro88/gigabrain"` → new repo URL
- `const repo = ... ?? "gigabrain"` → `?? "quaid"` (GitHub Pages base path fallback)
- Sidebar entry `"why-gigabrain"` → `"why-quaid"` (requires file rename below)

### website/package.json
- `"name": "gigabrain-docs"` → `"name": "quaid-docs"`

### website/SITE.md
- Title "GigaBrain Documentation Site" → "Quaid Documentation Site"
- References to `why-gigabrain` guide slug

### website/src/content/docs/index.mdx (homepage)
- Hero title, tagline, "What makes GigaBrain different" heading
- Install URL `macro88/gigabrain` → updated repo URL
- CLI snippet: `gbrain init ~/brain.db` → `quaid init ~/memory.db`
- CLI snippet: `GBRAIN_DB=~/brain.db gbrain serve` → `QUAID_DB=~/memory.db quaid serve`
- Tool count card: `brain_*` tool description → `memory_*`
- "Live Vault Sync" card: GigaBrain watches → Quaid watches
- All `gbrain` commands in "Get running in seconds" block

### website/src/content/docs/why-gigabrain.mdx ← FILE RENAME REQUIRED
- Rename to `why-quaid.mdx`
- All `GigaBrain` product name references → `Quaid`
- Navigation sidebar entry (in astro.config.mjs) must be updated simultaneously

### website/src/content/docs/start-here/welcome.mdx
- `displayTitle: Welcome to GigaBrain` → `Welcome to Quaid`
- Product description paragraph: GigaBrain → Quaid
- `gbrain` CLI references in conventions block
- `brain.db` → `memory.db`
- Links: "Install GigaBrain" → "Install Quaid"

### website/src/content/docs/tutorials/install.mdx
- Title "Install GigaBrain" → "Install Quaid"
- Install URL `macro88/gigabrain` → updated repo URL
- `gbrain` binary refs throughout → `quaid`
- `GBRAIN_VERSION=v0.9.8` → `QUAID_VERSION=v0.9.8`
- `~/.local/bin/gbrain` → `~/.local/bin/quaid`
- `~/.gbrain/skills/`, `~/.gbrain/models/` → `~/.quaid/skills/`, `~/.quaid/models/`
- `~/brain.db` default path → `~/memory.db`

### website/src/content/docs/tutorials/first-brain.mdx
- "GigaBrain useful" prose → Quaid
- `gbrain init ~/brain.db` → `quaid init ~/memory.db`
- All `gbrain` CLI commands → `quaid`
- `GBRAIN_DB` → `QUAID_DB`
- `~/.gbrain/skills/` → `~/.quaid/skills/`
- `brain.db` WAL sidecar references

### website/src/content/docs/tutorials/connect-claude-code.mdx
- "GigaBrain via the MCP server" → Quaid
- `gbrain serve` → `quaid serve`
- MCP config key `"gbrain":` → `"quaid":`
- `GBRAIN_DB` → `QUAID_DB`
- `brain.db` → `memory.db`
- All `brain_*` tool names → `memory_*` (brain_get, brain_put, brain_query, brain_search, brain_list, brain_link, brain_link_close, brain_backlinks, brain_graph, brain_check, brain_timeline, brain_tags, brain_gap, brain_gaps, brain_stats, brain_collections, brain_raw)
- `GBRAIN_MODEL` → `QUAID_MODEL`

### website/src/content/docs/reference/cli.mdx
- `gbrain` binary name throughout → `quaid`
- `GBRAIN_DB` → `QUAID_DB`
- `GBRAIN_MODEL` → `QUAID_MODEL`
- `./brain.db` default path → `./memory.db`
- `brain_config` table name → `memory_config` (or `quaid_config` — **open decision**)
- Every command example: `gbrain init`, `gbrain get`, `gbrain put`, etc. → `quaid *`

### website/src/content/docs/reference/mcp.mdx
- `gbrain serve` → `quaid serve`
- All 17 `brain_*` tool names in table and section headers → `memory_*`
- All JSON-RPC examples: `"name":"brain_get"` → `"name":"memory_get"` etc.
- "Seventeen `brain_*` tools" → "Seventeen `memory_*` tools"

### website/src/content/docs/reference/configuration.mdx
- All `GBRAIN_*` env var names → `QUAID_*` (10 variables)
- `./brain.db` default → `./memory.db`
- `~/.gbrain/` filesystem layout → `~/.quaid/`
- `brain_config` table name → `memory_config`
- `gbrain config`, `gbrain compact`, `gbrain skills doctor` → `quaid *`
- "GigaBrain reads configuration" → "Quaid reads configuration"

### website/src/content/docs/reference/schema.mdx
- `brain.db` file name → `memory.db`
- GitHub link `macro88/gigabrain` → updated repo URL
- `brain_config` table name → `memory_config`
- All `gbrain` CLI command refs → `quaid`
- `brain_check`, `brain_link` tool refs → `memory_check`, `memory_link`

### website/src/content/docs/reference/errors.mdx
- (Not fully read — likely contains `gbrain`/`brain_*` error context)

### website/src/content/docs/reference/page-types.mdx
- (Not fully read — likely contains `gbrain` CLI refs)

### website/src/content/docs/explanation/architecture.mdx
- Product name "GigaBrain" → "Quaid"
- `brain.db` in architecture diagram → `memory.db`
- `gbrain` serve/CLI refs → `quaid`

### website/src/content/docs/explanation/hybrid-search.mdx
### website/src/content/docs/explanation/page-model.mdx
### website/src/content/docs/explanation/skills-system.mdx
### website/src/content/docs/explanation/embedding-models.mdx
### website/src/content/docs/explanation/privacy.mdx
- All: product name GigaBrain → Quaid, `gbrain` CLI → `quaid`, `GBRAIN_*` → `QUAID_*`

### website/src/content/docs/agents/quickstart.mdx
- "connecting to a GigaBrain" → "connecting to a Quaid"
- `gbrain serve` → `quaid serve`
- `brain.db` → `memory.db`
- All `brain_*` tool names → `memory_*`

### website/src/content/docs/agents/tool-catalog.mdx
- All 17 `brain_*` section headers → `memory_*`
- All "→ [reference](/reference/mcp/#brain_*)" anchor links → `#memory_*`

### website/src/content/docs/agents/skill-workflows.mdx
### website/src/content/docs/agents/sensitivity-contract.mdx
- (Not fully read — will contain `brain_gap`, `brain_put` etc. → `memory_*`)

### website/src/content/docs/how-to/collections.mdx
- `gbrain collection` → `quaid collection`
- `.gbrainignore` → `.quaidignore` (**open decision: file name change**)
- `brain_put`, `brain_link` → `memory_put`, `memory_link`

### website/src/content/docs/how-to/import-obsidian.mdx
### website/src/content/docs/how-to/write-pages.mdx
### website/src/content/docs/how-to/build-graph.mdx
### website/src/content/docs/how-to/contradictions-and-gaps.mdx
### website/src/content/docs/how-to/airgapped-vs-online.mdx
### website/src/content/docs/how-to/switch-embedding-model.mdx
### website/src/content/docs/how-to/skills.mdx
### website/src/content/docs/how-to/upgrade.mdx
### website/src/content/docs/how-to/troubleshooting.mdx
- All: `gbrain` → `quaid`, `GBRAIN_*` → `QUAID_*`, `brain.db` → `memory.db`, `brain_*` tools → `memory_*`, `~/.gbrain/` → `~/.quaid/`

### website/src/content/docs/integrations/hermes.mdx
- "GigaBrain can function…" → "Quaid can function…"
- Install URL `macro88/gigabrain` → updated
- MCP config YAML: `gigabrain:` key → `quaid:`
- `command: "gbrain"` → `command: "quaid"`
- `GBRAIN_DB` → `QUAID_DB`
- `brain.db` → `memory.db`
- `brain_put`, `brain_query` → `memory_put`, `memory_query`
- "GigaBrain exposes 17 tools" → "Quaid exposes 17 tools"
- SQLite comment `brain.db` → `memory.db`

### website/src/content/docs/contributing/roadmap.md
- Product name GigaBrain → Quaid throughout
- All `gbrain` CLI refs → `quaid`
- All `brain_*` MCP tool names → `memory_*`
- `.gbrainignore` → `.quaidignore`
- Schema table `brain_config` → `memory_config`

### website/src/content/docs/contributing/specification.md
- Title "GigaBrain — Personal Knowledge Brain" → "Quaid — Personal Knowledge Brain"  
- Repo URL `GitHub.com/macro88/gigabrain` → updated
- All `gbrain`/`GigaBrain` → `quaid`/`Quaid`
- `brain_*` MCP tool refs → `memory_*`

### website/src/content/docs/contributing/contributing.md
- "Contributing to GigaBrain" → "Contributing to Quaid"
- Repo layout block: `gigabrain/` root dir label → `quaid/`
- All `gbrain` CLI commands → `quaid`

### docs/contributing.md (non-website source)
- Same as website version above

### docs/roadmap.md (non-website source)
- Same as website contributing/roadmap.md above

### docs/spec.md (non-website source)
- Extensive: product name, repo URL, all CLI + MCP surface references

### docs/getting-started.md (non-website source, referenced but not read in full)
- All `gbrain`/`GigaBrain`/`GBRAIN_*` references

### README.md
- Product name GigaBrain → Quaid in headline and tagline
- Inspiration attribution (Garry Tan's GBrain — attribution stays but product name changes)
- All `gbrain` CLI commands throughout
- All `GBRAIN_*` env vars
- Install URLs `macro88/gigabrain` → updated
- `brain.db` default file → `memory.db`
- `~/.gbrain/` → `~/.quaid/`

---

## Open decisions requiring team input before implementation

| # | Decision |
|---|----------|
| **D-Q1** | Default DB filename: `memory.db` or `quaid.db`? Task spec says either; pick one canonical name for all docs. |
| **D-Q2** | `brain_config` SQLite table rename: `memory_config` or `quaid_config`? This affects both schema docs and runtime. |
| **D-Q3** | Vault ignore file: `.gbrainignore` → `.quaidignore` or `.quaid-ignore`? |
| **D-Q4** | GitHub repo URL: will the repo be renamed to `macro88/quaid`? Affects all install URLs, raw script URLs, and the `astro.config.mjs` base-path fallback. |
| **D-Q5** | npm package: `gbrain` → `quaid`? Affects install.mdx npm row and README install table. |

---

## Implementation note for the team

The rename is purely mechanical — search-and-replace with the above mapping — but it must be done atomically across all surfaces in a single commit. The astro.config.mjs sidebar entry and the `why-gigabrain.mdx` file rename must be co-ordinated (rename file + update sidebar key simultaneously) or the docs build breaks.

Recommend: Fry or a general-purpose agent executes the rename after D-Q1 through D-Q5 are resolved.

# Decision: Website rename GigaBrain → Quaid (G.8)

**Date:** 2026-04-26  
**Author:** Hermes (Docs Site Engineer)  
**Status:** Accepted  
**Scope:** `website/` — all 56 source files

## Summary

Completed task G.8 of the `quaid-hard-rename` OpenSpec change. All website docs, config,
and content files have been updated from the old `GigaBrain / gbrain / brain_*` naming to
`Quaid / quaid / memory_*`.

## Decisions made

### 1. `.gbrainignore` → `.quaidignore`

**Decision:** Rename to `.quaidignore` in all docs references.

**Rationale:** The file is user-visible (users create it in their vault directories).
Since this is a hard product rename and the proposal covers all user-visible surfaces,
keeping `.gbrainignore` would be an inconsistency. Updated in `collections.mdx`,
`roadmap.md`, and `specification.md`.

### 2. `brain_gap_approve` → `memory_gap_approve`

**Decision:** Rename to `memory_gap_approve` even though it is not in the 17-tool
official rename list in `proposal.md`.

**Rationale:** The tool is a planned future tool that follows the `memory_*` naming
convention. Leaving it as `brain_gap_approve` in docs while all other tools are
`memory_*` would be confusing. Updated in `sensitivity-contract.mdx` and
`specification.md`.

### 3. Spec-draft tool names in `specification.md`

**Decision:** Rename all spec-draft `brain_*` tool names to `memory_*` in `specification.md`.

**Rationale:** `specification.md` is public-facing docs. Even tool names not yet
implemented should follow the new convention. Renamed: `brain_ingest`, `brain_links`,
`brain_unlink`, `brain_timeline_add`, `brain_tag`, `brain_briefing`,
`brain_ingest_meeting` → `memory_*` equivalents.

## Files changed

- `website/package.json` — `"name": "quaid-docs"`
- `website/SITE.md` — product name
- `website/astro.config.mjs` — title, sidebar slug, social href, repo/owner defaults
- `website/src/content/docs/why-quaid.mdx` (renamed from `why-gigabrain.mdx`)
- 43 content MDX/MD files — all `gbrain`/`brain_*`/`GBRAIN_*`/`brain.db`/`~/.gbrain/` references

## Verification

- `astro build` passed: 37 pages built with zero errors.
- Post-build grep scan: zero remaining old-brand occurrences in `website/src/`.

# Hermes — Residual Rename Cleanup Decisions

**Date:** 2026-04-25  
**Agent:** Hermes  
**Task:** Residual old-brand audit — close all remaining GigaBrain/gbrain/brain traces from website surfaces

---

## Decision 1: Rename tutorial route `/tutorials/first-brain/` → `/tutorials/first-memory/`

**Decision:** Rename `tutorials/first-brain.mdx` to `tutorials/first-memory.mdx` and update the route throughout the site.

**Rationale:** The tutorial slug `first-brain` is a visible URL in the browser, in the sidebar nav, and in all internal links. Leaving it as `first-brain` after the product rename to Quaid/memory would be the most prominent remaining old-brand trace for any user who looks at the URL bar. A clean rename removes it from every user-facing surface simultaneously.

**Impact:** All internal links updated (`/tutorials/first-brain/` → `/tutorials/first-memory/`). Sidebar nav entry updated. Old `.mdx` file deleted.

---

## Decision 2: Replace product-term `brain` with `memory` throughout website docs

**Decision:** Systematically replace "brain" (as a product/instance noun referring to the local knowledge store) with "memory" across all website `.mdx`/`.md` content.

**Rationale:** "brain" is the old product-instance term from the GigaBrain era (e.g. `brain.db`, `brain_*` MCP tools, "initialize a brain", "your first brain"). The rename spec maps this to "memory" (e.g. `memory.db`, `memory_*` tools). Inconsistent terminology — Quaid binary, but still "a brain" in prose — fragments the product identity and confuses users.

**Scope of replacements:**
- Prose: "a brain" / "your brain" / "the brain" / "this brain" → "a memory" / "your memory" / "the memory" / "this memory"
- Titles: "Build your first brain" → "Build your first memory"
- Code/examples: `memory.db`, `memory_*` MCP tool names, `~/.quaid/`, `QUAID_DB`, `QUAID_MODEL`
- GitHub URL: `macro88/gigabrain` → `quaid-app/quaid`

**Preserved:** Idiomatic English metaphor "second brain" in tagline left as-is where it describes a concept rather than the product instance (one occurrence reviewed in context).

---

## Decision 3: Retain `gb-guide-grid` CSS class names in HTML/Astro templates

**Decision:** Do NOT rename CSS classes such as `gb-guide-grid`, `gb-guide-stack`, etc.

**Rationale:** These are internal CSS identifiers not visible to end users (no rendered text, no public URL). Renaming them would require a coordinated CSS + template change with no user-visible benefit. Deferring to a dedicated CSS pass if/when the design system is revisited.

---

## Files changed in this cleanup pass

| File | Action |
|------|--------|
| `website/src/content/docs/index.mdx` | Brand + CLI + URL updates |
| `website/src/content/docs/tutorials/first-brain.mdx` | Deleted (replaced by `first-memory.mdx`) |
| `website/src/content/docs/tutorials/first-memory.mdx` | Created (renamed + fully updated) |
| `website/src/content/docs/tutorials/install.mdx` | Brand + path + CLI updates |
| `website/src/content/docs/tutorials/connect-claude-code.mdx` | Brand + tool-name + CLI updates |
| `website/src/content/docs/start-here/welcome.mdx` | Brand updates |
| `website/src/content/docs/why-quaid.mdx` | Residual brand traces |
| `website/src/content/docs/how-to/**` | CLI + env var + path updates (all how-to pages) |
| `website/src/content/docs/reference/**` | CLI + tool name + env var updates |
| `website/src/content/docs/explanation/**` | Brand + product term updates |
| `website/src/content/docs/agents/**` | Tool name updates (`brain_*` → `memory_*`) |
| `website/src/content/docs/integrations/**` | Product name updates |
| `website/src/content/docs/contributing/**` | CLI + product name updates |
| `website/astro.config.mjs` | Sidebar nav: `tutorials/first-brain` → `tutorials/first-memory` |
| `website/SITE.md` | Product name update |
| `website/src/components/**` | Brand/product references in components |

**Verification result:** Zero matches for `gbrain|GigaBrain|macro88/gigabrain|GBRAIN_|first-brain|brain\.db` across all `website/` content after cleanup.

# Decision: Concurrent-open busy-timeout fix (benchmark lane)

**Date:** 2026-04-25  
**Author:** Kif  
**Affects:** `src/core/db.rs` → `open_connection`  
**CI run:** 24898484969 on head 18ac3d7

---

## What failed

`concurrent_readers_see_consistent_data` in `tests/concurrency_stress.rs` panicked on
every run.  Four reader threads each called `open_conn` → `db::open()` simultaneously
against an already-initialized on-disk database.  The panic was at line 31
(`db::open(path).unwrap_or_else(...)`) and line 320 (`handle.join().expect("reader thread
panicked")`).

## Root cause

`open_connection` in `src/core/db.rs` calls `Connection::open(path)` and then immediately
runs `execute_batch(schema.sql)`.  The schema batch begins with `PRAGMA journal_mode = WAL`
followed by multiple `CREATE TABLE IF NOT EXISTS` statements — DDL that requires a write
lock.  No busy timeout was set before this batch, so any thread that couldn't acquire the
write lock immediately received `SQLITE_BUSY` and the `?` propagation caused the panic.

The coordinator's fix in `tests/concurrency_stress.rs` added `conn.busy_timeout(1s)` to
the `open_conn` helper, but that call runs *after* `db::open()` returns — too late to
protect schema initialization.

## Why this is a runtime bug, not just a test fluke

Any two `gbrain` processes opening the same `brain.db` simultaneously (e.g. a background
`gbrain serve` and a foreground `gbrain query`) will hit the same failure.  The busy timeout
must be applied before DDL, not after.

## Fix

`src/core/db.rs`, `open_connection`:

```rust
let conn = Connection::open(path)?;
// Set busy timeout *before* schema DDL so concurrent opens don't race on the
// write lock required by the initial PRAGMA + CREATE TABLE IF NOT EXISTS batch.
conn.busy_timeout(Duration::from_secs(5))?;
conn.execute_batch(include_str!("../schema.sql"))?;
```

Five seconds is a conservative production ceiling.  The test helper's 1-second
post-open override is harmless and was left untouched.

## Verification

```
running 4 tests
test wal_compact_during_open_reader_both_succeed ... ok
test concurrent_readers_see_consistent_data ... ok
test parallel_occ_exactly_one_write_wins ... ok
test duplicate_ingest_from_two_threads_produces_one_row ... ok

test result: ok. 4 passed; 0 failed
```

`corpus_reality` (8/8 non-ignored) and `embedding_migration` (3/3) also pass.  
The two pre-existing lib failures (`open_rejects_nonexistent_parent_dir`,
`init_rejects_nonexistent_parent_directory`) reproduce identically on the unmodified head
and are out of scope for this lane.

## Benchmark lane status

All offline benchmark / stress gates are green.  No regressions introduced.

# Batch 1 — Watcher Reliability: Scope, Sequencing, and Release Gate

**By:** Leela  
**Date:** 2026-04-28  
**Release target:** v0.10.0  
**Status:** ⚠️ SUPERSEDED — Professor rejected this artifact on 2026-04-28. See `leela-batch1-repair.md` for the authoritative repaired scope.

---

## 1. Authoritative Task List

Batch 1 contains exactly **13 open tasks**. All are currently `- [ ]` in `tasks.md`:

| Task ID | Description | Paired Test(s) |
|---------|-------------|----------------|
| 6.7a | Overflow recovery worker (500ms poll, `needs_full_sync` clear) | 17.5w, 17.5x, 17.5aaa2 |
| 6.8 | `.quaidignore` live reload via `WatchEvent::IgnoreFileChanged` | 17.5y, 17.5z, 17.5aa |
| 6.9 | Native-first watcher, poll fallback on init error with WARN | 17.5aaa3 |
| 6.10 | Per-collection supervisor with crash/restart + exponential backoff | 17.5aaa4 |
| 6.11 | Watcher health in `memory_collections` + `quaid collection info` | — |
| 17.5w | `needs_full_sync=1` clears within 1s via recovery worker | (test body for 6.7a) |
| 17.5x | Recovery worker skips `state='restoring'` collections | (test body for 6.7a) |
| 17.5y | Valid `.quaidignore` edit refreshes mirror + reconciles | (test body for 6.8) |
| 17.5z | Single-line parse failure preserves last-known-good mirror | (test body for 6.8) |
| 17.5aa | Absent `.quaidignore` with prior mirror → WARN, mirror unchanged | (test body for 6.8) |
| 17.5aaa2 | Watcher overflow sets `needs_full_sync=1`, recovery within 1s | (test body for 6.7a) |
| 17.5aaa3 | Watcher auto-detects native, downgrades to poll on init error | (test body for 6.9) |
| 17.5aaa4 | Watcher supervisor restarts on panic with exponential backoff | (test body for 6.10) |

**Already closed and NOT in scope:** 17.5aa2, 17.5aa3, 17.5aa4, 17.5aa4b, 17.5aa4c, 17.5aa5, 17.5aaa, 17.5aaa1.

---

## 2. Honesty Constraints — Non-Negotiable Pre-Conditions

From `now.md` (2026-04-28): **"No next vault-sync slice is active yet; require a fresh scoped gate before implementation resumes."**

This is a hard blocker. Fry cannot start a single line of Batch 1 body until Professor issues a pre-gate on the new interfaces. Specifically, Professor must sign off on:

1. `ReconcileMode::OverflowRecovery` variant in `reconciler.rs` — the authorization contract for calling `full_hash_reconcile_authorized` from the serve loop's recovery timer.
2. `WatcherMode` enum (`Native` | `Poll` | `Crashed` | `Inactive`) added to `CollectionWatcherState` — this struct change ripples into 6.10 (backoff fields) and 6.11 (health reporting).
3. The `last_overflow_recovery` timer placement in `start_serve_runtime()` relative to existing timers (`last_heartbeat`, `last_quarantine_sweep`, `last_dedup_sweep`).

Without Professor's gate, Batch 1 is not open.

---

## 3. Hidden Dependencies

### 6.9 → 6.10 → 6.11 (strict chain)
- 6.9 must land first: it introduces `WatcherMode` and the `mode` field on `CollectionWatcherState`.  
- 6.10 adds `last_watcher_error` and `backoff_until: Option<Instant>` to the same struct — cannot land without 6.9's struct changes.  
- 6.11 reads `WatcherMode` from the process-global registry — cannot surface correct values without both 6.9 and 6.10.

### 6.11 schema extension note
Task 13.6's closure note says "frozen 13-field read-only collection object." Task 6.11 explicitly adds three new fields: `watcher_mode`, `watcher_last_event_at`, `watcher_channel_depth`. This is a documented extension, not a schema violation. However, Fry must add a closure note to 13.6 (addendum, not rewrite) acknowledging the three new fields so the audit trail is clean. Nibbler will flag this if not pre-addressed.

### `memory_collections` on Windows
The three watcher health fields must return `null` (not an error) when the collection is on Windows. The platform gate for vault-sync CLI surfaces is `#[cfg(unix)]`; the MCP tool `memory_collections` is cross-platform. The health fields belong to a cross-platform response shape, so they should null out on Windows rather than be cfg-excluded.

---

## 4. Fry's Execution Sequence

**Phase 0 (gate):**
- Professor issues pre-gate on `ReconcileMode::OverflowRecovery`, `WatcherMode`, and timer placement.  
- No Fry code written until this clears.

**Phase 1 (parallel, after Professor gate):**
- Fry: 6.7a + tests 17.5w, 17.5x, 17.5aaa2 in `src/core/vault_sync.rs`
- Fry: 6.8 + tests 17.5y, 17.5z, 17.5aa in `src/core/vault_sync.rs`

**Phase 2:**
- Fry: 6.9 (`WatcherMode` enum + poll fallback + test 17.5aaa3)

**Phase 3 (parallel, after 6.9):**
- Fry: 6.10 + test 17.5aaa4 (backoff state machine)
- Fry: 6.11 (health surface in `vault_sync.rs` + `commands/collection.rs`)

**Phase 4 (review):**
- Nibbler adversarial review on 6.9 (mock failed native init path) and 6.10 (simulated disconnect + backoff timing).
- Nibbler also confirms 6.11 null-out on Windows is correct.

---

## 5. Test Lane Requirements Before "Batch 1 Done"

All tests are inline `#[cfg(test)]` modules per project convention:

| Requirement | Owner |
|-------------|-------|
| `cargo test` passes clean with all 13 tasks covered | Scruffy |
| Coverage ≥ 90% (all new paths hit) | Scruffy |
| 17.5w: `needs_full_sync` clears within 1s in serve loop | Fry → Scruffy verify |
| 17.5x: `state='restoring'` blocks recovery (NOT cleared) | Fry → Scruffy verify |
| 17.5aaa2: channel-overflow integration (sets flag, recovery runs) | Fry → Scruffy verify |
| 17.5aaa3: mock failed native init, assert poll fallback + WARN | Nibbler adversarial |
| 17.5aaa4: simulate watcher disconnect, assert backoff timing | Nibbler adversarial |

Scruffy runs the full test suite after Phase 3 completes. Nibbler adversarial review is required specifically for 6.9 and 6.10 before merge.

---

## 6. v0.10.0 Release Gate

The release is greenlit when ALL of the following are true:

1. **All 13 Batch 1 tasks marked `[x]`** in `tasks.md` with truthful closure notes.
2. **`cargo test` passes** on the `spec/vault-sync-engine` branch with zero failures.
3. **Coverage ≥ 90%** confirmed by Scruffy.
4. **Nibbler adversarial sign-off** on 6.9 + 6.10 review lanes.
5. **13.6 closure addendum** in `tasks.md` documenting the three new watcher health fields.
6. **`Cargo.toml` version bumped to `0.10.0`**.
7. **`CHANGELOG.md` / release notes** include: overflow recovery, `.quaidignore` live reload, native/poll watcher fallback, per-collection supervisor with backoff, and watcher health MCP + CLI surface.

---

## 7. What Is NOT In Batch 1

These remain deferred and must not be reopened by Fry during this batch:

- Online restore handshake, IPC socket work (`17.5pp` / `17.5qq*`)
- Quarantine restore (backed out; `9.8`, `17.5j` remain open)
- Watcher overflow/supervision beyond the crashed-state surface in 6.10
- Embedding worker (Batch 2)
- UUID write-back (Batch 3)
- Any broader `12.1`, `12.4`, `12.6*` claims

# Decision: Migration docs fix — CLI truthfulness + zero-trace enforcement

**Date:** 2026-04-25  
**Author:** Leela  
**Triggered by:** Professor rejection of migration guidance (cycle: quaid-hard-rename)

---

## Context

The Professor rejected `MIGRATION.md` and `website/src/content/docs/how-to/upgrade.mdx`
on two grounds:

1. **CLI lie** — the export step called `gbrain export --out backup/` (and in upgrade.mdx,
   `<old-binary> export > backup/`). The real CLI signature is a positional path argument:
   `quaid export <path>`. There is no `--out` flag and no stdout export flow.

2. **Zero-trace violation** — literal legacy names (`gbrain`, `GigaBrain`, `GBRAIN_`,
   `brain_*`, `~/.gbrain/...`) were present in `MIGRATION.md`, which is a public
   non-hidden surface.

Amy, Hermes, Zapp, Fry, and Bender are locked out for this cycle.

---

## Decisions taken

### 1. Export command corrected

`MIGRATION.md` line 113: `gbrain export --out backup/` → `<old-binary> export backup/`  
`upgrade.mdx` line 32: `<old-binary> export > backup/` → `<old-binary> export backup/`

Rationale: `src/commands/export.rs` and `src/main.rs` confirm the `Export` variant takes
a positional `path: String` field. No `--out` flag exists. The stdout-redirect form (`>`)
was also wrong — `export_dir` writes files to a directory path, not to stdout.

### 2. Legacy literal names masked across MIGRATION.md

All occurrences of the literal legacy binary name, product name, env-var prefix, MCP tool
prefix, and default DB path have been replaced with opaque placeholders consistent with
the style already established in `upgrade.mdx`:

| Pattern | Replacement |
|---------|-------------|
| literal old binary | `<old-binary>` |
| literal old npm package | `<old-package-name>` |
| literal old env prefix `GBRAIN_` | `<LEGACY>_` |
| literal old MCP prefix `brain_` | `<old-prefix>_` |
| literal old default DB path | `*(legacy default path)*` |
| title/prose product name | generic "Legacy → Quaid" / "legacy binary" |

### 3. Coupled docs verified clean

`README.md`, `docs/getting-started.md`, `docs/contributing.md`, `docs/roadmap.md`,
and `docs/openclaw-harness.md` contain no legacy literal names. No changes needed.

---

## Ownership lock-out

Amy, Hermes, Zapp, Fry, and Bender are excluded from this fix cycle per the rejection
context. Leela self-executed. Re-review by Professor before merge.

---
author: leela
date: 2026-04-25
status: proposed
change: quaid-hard-rename
---

# Decision: quaid-hard-rename — Lead routing memo

## Context

macro88 has directed a complete hard rename of the GigaBrain/gbrain/brain surface to Quaid/quaid/memory. This is the largest single cross-cutting change the team has taken on: 230+ files, a breaking schema change, 17 MCP tool renames, full CI/artifact renaming, and every piece of user-facing documentation. The OpenSpec change is at `openspec/changes/quaid-hard-rename/`.

## Decisions made

### 1. Default DB file: `memory.db` (not `quaid.db`)

The user specified "memory.db or quaid.db". I am resolving this as **`memory.db`** at path `~/.quaid/memory.db`. Rationale: the conceptual layer is explicitly named "memory" (MCP tools are `memory_*`, concept is `memory`), and naming the file after the concept mirrors the original `brain.db` convention. The product name (`quaid`) appears in the directory, not the filename.

If macro88 has a strong preference for `quaid.db`, override this before Phase B lands.

### 2. Scope of `.squad/` history files

Agent history files (`.squad/agents/*/history.md`, `.squad/log/`) are **excluded** from the rename. They are a historical record and retroactively rewriting them would corrupt the audit trail. The final Phase J audit scan must explicitly exclude `.squad/` from its zero-match assertion.

### 3. Breaking schema change: no automatic migration

Consistent with the "no legacy support, no aliases, no shims" directive: the `brain_config` → `quaid_config` rename is handled as a clean schema version bump with no fallback reading of the old table name. Users must export and re-init. This is documented in the migration guide (Phase K).

### 4. vault-sync-engine branch must coordinate first

The `vault-sync-engine` branch is in-flight (`spec/vault-sync-engine`) and touches many of the same files (400+ files changed). This rename **must not** be started until one of:
- vault-sync-engine is merged to `main`, OR
- Fry, Professor, and Nibbler confirm they will rebase vault-sync onto the rename branch after it lands

This is the single hardest sequencing constraint on the entire change. Violating it means a very large, risky merge conflict resolution.

### 5. MCP tool rename is user-breaking

Every MCP client config referencing `brain_*` tool names will silently stop working after this change. The migration guide (Phase K) and release notes must call this out explicitly and provide the full old→new mapping before the first `quaid` tag is published.

## Parallel work that can start now

The following tracks are independent of each other and can be assigned in parallel once vault-sync coordination is confirmed:

| Track | Owner lane | Key dependency |
|-------|-----------|----------------|
| **B — Schema** | Professor | Must land first; gates D (MCP rename depends on quaid_config) |
| **C — Cargo/binary** | Fry | Independent of B |
| **E — Env vars** | Fry | Independent of B |
| **F — CI/Release** | Zapp/Kif | Independent of B; depends on C for artifact names |
| **G — Docs** | Scribe/Amy | Independent of B, C |
| **H — Skills** | Amy/Scribe | Independent of all above |

## What must wait

| Track | Waits for |
|-------|-----------|
| **D — MCP tool rename** | B (schema) — can technically start in parallel but reviewers should gate on B being confirmed |
| **I — Tests** | B + C + D + E (needs final names stable) |
| **J — Final audit** | All phases |
| **K — Migration guide** | J complete |
| **L — PR** | K + I |

## Key invariants reviewers must enforce

1. **No `brain_*` tool name** remains in any `#[tool(name = "...")]` annotation.
2. **No `GBRAIN_*`** env var in any source, script, workflow, or doc (excluding `.squad/` history).
3. **`brain_config` completely absent** from `src/schema.sql` and `src/core/db.rs`.
4. **`SCHEMA_VERSION` is bumped** — verify the constant value is greater than the last released version.
5. **Schema DDL + SCHEMA_VERSION + test fixtures in one atomic commit** — no partial schema commits.
6. **Default path is `~/.quaid/memory.db`** — not `~/.gbrain/` or `brain.db`.
7. **Binary is `quaid`** — `cargo build --release` must not produce `gbrain`.
8. **Zero legacy shims** — no aliases, no compatibility forwarders anywhere.
9. **Migration guide exists before the first `quaid` tag** — Phase K cannot be deferred to post-release.
10. **vault-sync coordination confirmed in writing** before any implementation PR is raised.

# Decision: README.md + docs/getting-started.md truth repair

**Date:** 2026-04-25
**Author:** Leela
**Trigger:** Nibbler + Professor reviewer rejections; zero-trace audit

---

## Context

Both `README.md` and `docs/getting-started.md` contained claims that conflicted with
the current shipped CLI surface documented in `.squad/identity/now.md` and audited
against `src/schema.sql` and `src/core/db.rs`.

---

## Decisions made

### 1. GBrain literals removed from README.md

Two `GBrain` occurrences in README.md (the inline "GBrain work" link text on line 11
and the Acknowledgements paragraph) were replaced with neutral descriptive text
referencing Garry Tan's compiled knowledge model. The hyperlink target (Garry Tan's
gist) is preserved; only the forbidden literal was excised.

**Rationale:** Zero-trace audit flagged these; they are legacy product-name adjacents
that must not appear post-rename.

### 2. npm install row changed to "Not yet published"

Both files had `🚧 Staged — online channel by default once published` for the npm row.
This was changed to `❌ Not yet published — use binary release or build from source`.

**Rationale:** Nibbler's rejection cited npm wording implying imminent publication.
The npm package is not published and no publish date is known. "Staged" overstates
readiness. The new wording is truthful and directs users to working install paths.

### 3. Quarantine restore claims removed

**README.md:** Removed "narrowly restore on Unix" from the quarantine lifecycle
feature bullet; removed `discard|restore` from the roadmap table (now `discard` only);
removed `quaid collection restore` and `--finalize-pending` commands from the
usage block; removed "narrow Unix restore" from the Contributing blurb.

**getting-started.md:** Replaced the block claiming
`quaid collection quarantine restore` is "implemented and available on Unix"
with a single deferred-surface note: "Quarantine restore is deferred in the current
release. The safe landed surface is `list`, `export`, and `discard` only."
Also removed `serve`/restore from the vault-sync section header callout.

**Rationale:** `.squad/identity/now.md` explicitly states restore has been backed out
of the live CLI surface. Professor's rejection cited schema/restore surface conflicts
with now.md. Both reviewers flagged overstated restore claims.

### 4. Schema version corrected in getting-started.md

Line 78 said "full v5 schema". `src/schema.sql` header and `SCHEMA_VERSION` constant
in `src/core/db.rs` are both 6. Corrected to "full v6 schema".

---

## Files touched

- `README.md`
- `docs/getting-started.md`

## Files consulted for validation

- `.squad/identity/now.md`
- `src/schema.sql`
- `src/core/db.rs`
- `openspec/changes/quaid-hard-rename/proposal.md`

# Decision: Release Contract Alignment (quaid-hard-rename)

**Date:** 2026-05-XX  
**Author:** Leela  
**Triggered by:** Nibbler/Professor rejection of prior release surfaces authored by Amy and Zapp

---

## Context

The release surface (`release.yml`, `RELEASE_CHECKLIST.md`, `MIGRATION.md`) contained three
independent contract violations that caused reviewer rejection:

1. **Broken online installer command** — `QUAID_CHANNEL=online` was placed before `curl`, not
   piped to `sh`. The env var never reached the shell interpreter. Silent failure on every
   online install attempt.

2. **Shell installer listed as unsupported** — `RELEASE_CHECKLIST.md` under "Deferred
   distribution channels" explicitly listed the `curl | sh` one-command installer as "not
   available." This contradicts the release workflow, install.sh, and README, which all
   document and ship the shell installer as a primary path.

3. **npm showing as an installable step** — `MIGRATION.md`'s npm section included
   `npm install -g quaid` inside the code block (as a step to execute), followed immediately
   by a note saying it isn't available yet. The checklist's migration gate reinforced the
   confusion by requiring `packages/quaid-npm/` to be present. npm is a follow-on; no npm
   package ships in the hard-rename release.

---

## Decisions Made

1. The online installer override command is the `| QUAID_CHANNEL=online sh` pipe-pattern,
   consistent with the README and install.sh implementation. This is the canonical form.

2. The shell installer (`curl ... | sh`) IS a supported install path for the hard-rename
   release. The checklist now explicitly lists it under "Supported install paths."

3. npm is deferred. `MIGRATION.md` instructs users to uninstall the pre-rename package but
   does NOT present `npm install -g quaid` as an executable step. The checklist's migration
   gate item is reworded to reflect follow-on status; the `packages/quaid-npm/` gate is
   removed from the hard-rename release requirement.

---

## Affected Files

- `.github/workflows/release.yml` — online installer command corrected
- `.github/RELEASE_CHECKLIST.md` — supported/deferred install path sections reconciled; npm gate reworded
- `MIGRATION.md` — npm section rewritten to not present an unavailable command as a step

---

## Reviewer note

Amy and Zapp are locked out of this cycle. The fixes are structural/content only —
no logic or workflow behavior changed. Nibbler should re-review the three files against
the ship contract before the next tag is cut.

# Mom — Batch 1 edge memo

## What I fixed

- Closed **6.8** honestly. `src/core/vault_sync.rs` now treats the root `.quaidignore` as a live control file instead of dropping it at the markdown-only watcher filter.
- The watcher emits a dedicated `IgnoreFileChanged` event, debounces it with other watch traffic, reloads the mirror through `src/core/ignore_patterns.rs::reload_patterns(...)`, and only runs reconcile after a successful atomic parse.
- Added focused proofs for:
  - `.quaidignore` change → mirror refresh + reconcile
  - invalid glob → last-known-good mirror preserved, parse errors surfaced, reconcile skipped
  - deleted `.quaidignore` with prior mirror → mirror preserved, stable-absence error surfaced, reconcile skipped

## False-closure risks still open

Do **not** check off these Batch 1 items yet:

1. **6.7a overflow recovery**
   - Current code sets `needs_full_sync=1` on channel overflow, but the serve loop does **not** run the promised 500ms overflow-recovery poller.
   - Real consequence: an overflow can leave a collection dirty until manual sync / restart / some other attach path, which is weaker than the spec.

2. **6.9 native→poll fallback**
   - `start_collection_watcher(...)` still hard-fails on `notify::recommended_watcher(...)` init/config/watch errors.
   - There is no `PollWatcher` downgrade path yet, so native watcher init failure is still a startup/runtime kill seam.

3. **6.10 watcher crash/restart backoff**
   - `poll_collection_watcher(...)` returns `InvariantViolation` on disconnected channels.
   - The serve loop currently ignores that error and keeps the dead watcher entry in the map; `sync_collection_watchers(...)` then sees matching root/generation and does **not** replace it. That is a permanent silent-sync-loss risk, not a restarted supervisor with backoff.

4. **6.11 watcher health reporting**
   - `quaid collection info` still lacks watcher mode / last-event time / channel depth.
   - `memory_collections` must remain frozen per the Batch 1 repair note, so any temptation to "close" 6.11 by widening MCP would be dishonest.

## Decision

- **Decision:** Close only `6.8` from the Batch 1 failure lane. Keep `6.7a`, `6.9`, `6.10`, and `6.11` open until their actual runtime guarantees exist.

## 2026-04-26: Public default memory path must track runtime truth

**By:** Mom

**What:** Public docs that describe the default database location must state `~/.quaid/memory.db`, because `src/core/db.rs` resolves the default path under the user's home directory and only falls back to bare `memory.db` when home-directory resolution fails.

**Why:** Stale examples like `./memory.db`, `~/memory.db`, or `$HOME/memory.db` create a false contract and break upgrade/install guidance. Docs may still show custom `--db` or `QUAID_DB` overrides, but any claim about the default path has to match the runtime path builder exactly.

# Decision: docs/spec.md must describe the shipped MCP surface, not the superseded design surface

**Author:** Mom  
**Date:** 2026-04-26  
**Change:** quaid-hard-rename  
**Triggered by:** Professor + Nibbler rejection of `docs/spec.md` for documenting non-landed MCP tools and approval flows.

---

## What was wrong

`docs/spec.md` still described a larger MCP contract than Quaid actually ships:

- removed/non-landed batch-ingest, outbound-link, unlink, split-timeline, split-tag, and gap-approval tool names
- missing shipped tool `memory_collections`
- wrong parameter shapes for shipped tools (`memory_query`, `memory_search`, `memory_list`, `memory_link`, `memory_tags`, `memory_gap`, `memory_gaps`, `memory_raw`)
- claims that MCP resources/prompts and gap-approval flows were live when `src/mcp/server.rs` exposes tools only
- no note that `quaid call` currently omits `memory_collections` even though `quaid serve` exposes it

## Decision

Repair `docs/spec.md` to match the code that ships today:

1. Treat `src/mcp/server.rs` as the authoritative MCP tool inventory and parameter surface.
2. Treat `src/commands/call.rs` as the authoritative `quaid call` surface, even when it is narrower than the MCP server.
3. Keep schema truth for `knowledge_gaps` columns, but explicitly mark approval/audit fields as schema-resident, not publicly exposed via MCP today.
4. Remove or rewrite prose that assumes a public approval/escalation or resolve workflow for gaps.

## Validation target

- Tool inventory in `docs/spec.md` must match the 17 `memory_*` handlers in `src/mcp/server.rs`.
- `docs/spec.md` must not claim public support for non-landed ingest/link/timeline/tag/gap-approval MCP surfaces.
- `docs/spec.md` must note that `memory_collections` is MCP-shipped but not wired through `quaid call`.

# Nibbler Absolute Final Approval

- Requested by: macro88
- Scope: hard rename final adversarial review
- Verdict: APPROVE

## Why

The two previously blocking seams are now closed without widening risk elsewhere.

1. `quaid call` now truthfully exposes the full shipped MCP surface, including `memory_collections`, and the dispatcher is backed by an explicit regression test in `src\commands\call.rs`.
2. `.github\workflows\publish-npm.yml` no longer auto-publishes on release tags; it is manual-only and now matches the docs/release copy that treat npm as staged but not live.

I did not find a remaining mismatch in the reviewed artifacts that would make the hard rename unsafe to ship.

## 2026-04-26: Quaid hard rename final adversarial review

**By:** Nibbler  
**Verdict:** REJECT

**Why:** The rename codepath itself looks coherent, but the ship-facing migration story is still not safe. The new release is a hard break with no shims; users need exact old→new mappings and a prominent stop-and-migrate warning on primary entry surfaces.

**Blocking artifacts:**
1. `MIGRATION.md`
2. `website/src/content/docs/how-to/upgrade.mdx`
3. `README.md` (and mirror onboarding surface `docs/getting-started.md`)

**Blocking findings:**
- `MIGRATION.md` and `website/src/content/docs/how-to/upgrade.mdx` still use placeholders like `<old-binary>`, `<old-prefix>`, `<LEGACY>_DB`, and `<old-package-name>` instead of the real legacy surface (`gbrain`, `brain_*`, `GBRAIN_*`, `brain.db`, `packages/gbrain-npm`). That makes the migration guide non-executable at exactly the point where this release intentionally removes compatibility.
- `README.md` / `docs/getting-started.md` do not carry a prominent hard-break / manual-migration warning or link to `MIGRATION.md`, so an upgrader can treat the rename like a routine patch and silently lose MCP tools / env wiring / DB continuity.

**Required revision:**
- A docs/release agent must replace placeholders with the concrete legacy names everywhere in migration/upgrade docs.
- A docs/release agent must add a top-level breaking-change warning on README/getting-started that links directly to `MIGRATION.md` and states the manual steps are mandatory before upgrade.

# Nibbler final release review — REJECT

Date: 2026-04-26
Requested by: macro88
Scope: final ship-readiness check for `quaid-hard-rename`

## Verdict

REJECT

## Why

The hard-rename messaging itself is much safer now, but the public docs surface is still not truthful enough to ship:

1. **`README.md` and `docs/getting-started.md` still advertise npm as a current install option.**
   - Both files list ``npm install -g quaid`` in the install-options table as `🚧 Staged — online channel by default once published`.
   - The release checklist now says npm is **not yet live** and must be labeled as planned-only / unavailable. Presenting it in the install matrix is still success-shaped guidance for a path that does not exist today.

2. **`README.md` and `docs/getting-started.md` still claim quarantine restore is shipped/live.**
   - `README.md` says the `v0.9.6` vault-sync slice includes quarantine ``discard|restore`` and later says users can “inspect, export, discard, or narrowly restore on Unix”.
   - `docs/getting-started.md` still says `quaid collection quarantine restore` is implemented and available on Unix.
   - That contradicts the current squad truth (`.squad/identity/now.md`), which says restore was backed back out of the live CLI surface and remains deferred. Shipping a hard rename on top of success-shaped feature drift is not safe.

## Required revision

Assign a docs/release lane to revise **`README.md`** and **`docs/getting-started.md`** so they:

- label npm as **planned follow-on / not yet live**, not as a present install option, and
- remove all restore-live wording unless/until the restore surface is actually back in the product.

Re-review after those two docs match the real release contract and current shipped scope.

# Nibbler Final Tree Approval

**Date:** 2026-04-26  
**Requested by:** macro88  
**Verdict:** **REJECT**

## Blocking ship-safety mismatch

The tree still carries a release-path contradiction on npm distribution.

- `.github/workflows/publish-npm.yml` is live on tag pushes and will run `npm publish --access public` from `packages/quaid-npm` whenever `NPM_TOKEN` is present.
- But the ship-facing surfaces say the opposite for this release:
  - `README.md` says `npm install -g quaid` is “❌ Not yet published”.
  - `docs/getting-started.md` says npm is “❌ Not yet published”.
  - `MIGRATION.md` says `quaid` is “not yet in the public registry”.
  - `.github/RELEASE_CHECKLIST.md` says npm is a planned follow-on and “not supported in this release”.

## Why this blocks approval

If the tag is cut with `NPM_TOKEN` configured, this tree will publish `quaid` to npm as part of the same release while the docs, migration guide, release checklist, and release-note contract all tell users npm is not live. That is a real ship-facing truth break on install/migration behavior, so the hard rename is not safe to ship yet.

# Nibbler MCP spec sign-off — 2026-04-26

**Verdict:** REJECT

## What I reviewed

- `.squad/agents/nibbler/history.md`
- `.squad/decisions.md`
- `.squad/identity/now.md`
- `openspec/changes/quaid-hard-rename/proposal.md`
- `openspec/changes/quaid-hard-rename/tasks.md`
- `docs/spec.md`
- `src/mcp/server.rs`
- `src/commands/call.rs`
- `README.md`
- `docs/getting-started.md`
- current working-tree diff

## Decision

Mom fixed the blocker I previously called out: `docs/spec.md` now matches the shipped `src/mcp/server.rs` MCP surface, including the 17-tool table, `memory_put` OCC semantics, `memory_raw` overwrite/object rules, and the real `memory_collections` exposure. Targeted validation also passed: `cargo test --quiet mcp::server`.

I am still **not** signing off the hard rename as safe to ship because one user-facing seam remains false in a way that breaks the renamed collections tool path:

- `docs/getting-started.md` still says **“Call any MCP tool directly from the CLI without starting the server”**
- `src/commands/call.rs` still hard-dispatches only **16** tools and does **not** route `memory_collections`
- `docs/spec.md` now truthfully documents that gap

That means a user following getting-started for the renamed collections surface can reasonably try:

```bash
quaid call memory_collections '{}'
```

and hit a dispatcher limitation instead of the documented MCP tool. That is still a real contract seam, not wording trivia.

## Rejected artifacts

1. `docs/getting-started.md`
   - Revise the “Raw MCP tool invocation” section so it no longer claims `quaid call` can invoke **any** MCP tool unless parity is actually implemented.

2. `src/commands/call.rs`
   - Either wire `memory_collections` through the fixed dispatcher, or explicitly keep it out-of-scope and ensure all user-facing docs say `quaid call` exposes only 16 tools.

## Ship bar

Safe to approve once **either**:

- `memory_collections` is added to `quaid call`, **or**
- `docs/getting-started.md` is narrowed to the real 16-tool dispatcher contract.

Until then, the MCP server spec itself is corrected, but the hard-rename user story is still not fully honest end-to-end.

## Nibbler re-review — quaid hard rename

- **Requested by:** macro88
- **Verdict:** REJECT
- **Date:** 2026-04-26

### Cleared from prior rejection

- `.gitignore` now fail-closes the legacy local DB files and the new npm-downloaded binary artifacts.
- npm cutover now points the publish workflow at `packages/quaid-npm/`, that package is tracked in git, `packages/gbrain-npm/` is gone, and `npm pack --dry-run` succeeds.

### Blocking seam

The public migration surface is still not trustworthy enough to ship a hard rename:

1. `website/src/content/docs/how-to/upgrade.mdx` tells users to see `MIGRATION.md` at the repo root, but no such file exists.
2. The same upgrade guide's post-upgrade validation commands point at `~/memory.db` instead of the new documented path `~/.quaid/memory.db`, which makes the manual migration/verification path drift right where this release must be exact.

### Required revision

Assign docs/release ownership (Amy, Scribe, or Zapp lane) to either:

- add the referenced `MIGRATION.md` with the hard-breaking rename steps, or
- remove that reference and make the existing upgrade/release docs fully self-contained,

and fix the upgrade guide commands so they validate the migrated DB at `~/.quaid/memory.db`.

### Validation seen in re-review

- `cargo test` passed.
- `tests/install_release_seam.sh` passed.
- `tests/release_asset_parity.sh` passed.
- `npm pack --dry-run ./packages/quaid-npm` passed.

---
author: nibbler
date: 2026-04-26
status: proposed
change: quaid-hard-rename
---

# Decision: quaid-hard-rename adversarial review

## Verdict

**REJECT**

## Blocking findings

### 1. Hidden legacy leakage still exists in `.gitignore`

Rejected artifact: `.gitignore`

The hard-rename audit explicitly drove non-hidden files to zero legacy hits, but the hidden leakage path is still open. `.gitignore` still ignores `/brain.db*` and `packages/gbrain-npm/bin/gbrain.*`, while the new default database is `~/.quaid/memory.db` and the npm wrapper now downloads `packages/quaid-npm/bin/quaid.bin` / `quaid.download`. That means the new default memory file and the npm-downloaded native binary can leak into `git status` and be accidentally committed, while the old ignored paths are dead names.

Different agent must revise `.gitignore` to track the Quaid-era defaults (`memory.db*`, `packages/quaid-npm/bin/quaid.*`) and remove the dead legacy package-binary entries.

### 2. Release/install guidance is still success-shaped for a breaking rename

Rejected artifacts: `.github/workflows/release.yml`, `README.md`, `website/src/content/docs/how-to/upgrade.mdx`

The OpenSpec says this rename is a breaking schema change with no auto-migration, and explicitly requires the export → `quaid init` → import path plus manual MCP client config updates before the first tag ships. `tasks.md` still shows K.1 open, `proposal.md` calls out this exact mitigation, but the release workflow body still reads like an ordinary patch release ("fixes Issue #81"), and the checked docs do not publish the required rename migration table or the MCP/tool/config cutover steps. That will strand existing users on upgrade with disappearing tools and an unreadable database.

Different agent must revise the public release/install surfaces so the first `quaid` release notes and human docs explicitly cover: binary rename, env-var rename, full `brain_*` → `memory_*` tool mapping, MCP config updates, and the manual export/re-init/import migration path.

### 3. The npm rename cutover is incomplete in the current tree

Rejected artifacts: `packages/quaid-npm/`, `.github/workflows/publish-npm.yml`

The tracked `packages/gbrain-npm/*` files are deleted, but the replacement `packages/quaid-npm/` directory is still untracked in the current working tree while `publish-npm.yml` now `cd`s into `packages/quaid-npm`. Rust build/test green does not protect this lane; if this tree is pushed as-is or partially staged, the npm publish path breaks outright.

Different agent must land the package rename as a complete tracked change set and re-verify the npm publish/dry-run lane against the tracked `packages/quaid-npm/` contents.

# 2026-04-26 — Nibbler README / getting-started re-review

**Requested by:** macro88  
**Verdict:** REJECT

## Scope reviewed

- `README.md`
- `docs/getting-started.md`
- directly coupled truth those artifacts point readers to

## Why rejected

Leela fixed the previously blocked README/getting-started seams themselves: npm is no longer presented as available, quarantine restore is no longer overclaimed, and the narrowed vault-sync surface is described truthfully in those two docs.

The remaining blocker is coupled truth. `README.md` says `docs/spec.md` is the authoritative design document, and `docs/getting-started.md` points readers there for command/tool details. That authoritative spec still contradicts the hard-rename surface in user-breaking ways:

- `docs/spec.md` still documents `QUAID_DB` / `--db` defaulting to `./memory.db` instead of `~/.quaid/memory.db`
- upgrade / rollback snippets still resolve `${QUAID_DB:-./memory.db}`
- release download examples still use unsuffixed `quaid-${PLATFORM}` assets even though the release contract is `quaid-<platform>-<channel>`

## Required revision

A different docs agent must repair `docs/spec.md` (and any linked upgrade/install snippets it owns) to match the renamed release contract and default DB path, or else remove the README/getting-started language that delegates authority to that stale spec surface.

## Review note

This is a truthfulness rejection, not a wording nit. As long as the linked authoritative spec tells users the wrong DB default and wrong release asset contract, the hard rename is not safe to approve.

# Nibbler release-surface review — REJECT

Date: 2026-04-26
Requested by: macro88
Scope: release/migration ship surfaces for `quaid-hard-rename`

## Verdict

REJECT

## Why

The hard-break visibility is now prominent enough, but three ship-facing seams still fail truthfulness:

1. **`.github/workflows/release.yml` — broken online installer example in release notes**
   - The release body shows:
     ```bash
     QUAID_CHANNEL=online \
       curl -fsSL ... | sh
     ```
   - That assigns `QUAID_CHANNEL` to `curl`, not to `sh`, so users following the published release notes will still run the installer in the default channel instead of `online`.
   - Required fix: rewrite the command so `sh` receives `QUAID_CHANNEL=online` (matching the working README/docs pattern).

2. **`MIGRATION.md` — unsupported npm path still presented as an install command**
   - The npm section still tells users to run:
     ```bash
     npm install -g quaid
     ```
   - The same section then states the package is not yet in the public registry. That is success-shaped guidance for a path that is explicitly unavailable.
   - Required fix: remove the install command or label the whole path as planned-only / unavailable until the package is actually published.

3. **`.github/RELEASE_CHECKLIST.md` — installer truth drift**
   - The checklist still says `curl | sh` is **not available**.
   - But `release.yml`, `README.md`, `docs/getting-started.md`, and the shell-test lane (`tests/install_release_seam.sh`, `tests/release_asset_parity.sh`) all treat `scripts/install.sh` as a real supported release surface.
   - Required fix: make the checklist truthful about the supported installer path, or remove the installer from every other public ship surface. Current state is internally contradictory.

## Re-review bar

Approve after:

- release notes use a working `online` installer command,
- migration docs stop telling users to run an unavailable npm install,
- and the release checklist matches the actual supported install surface.

# Nibbler — final rename ship review

- Requested by: macro88
- Change: `openspec/changes/quaid-hard-rename`
- Verdict: **REJECT**

## Rejected artifact

- `docs/spec.md`

## Why this is still a ship blocker

`README.md` and `docs/getting-started.md` now correctly describe a shipped **17-tool** `memory_*` MCP surface and explicitly send readers to `docs/spec.md` for the signatures. But `docs/spec.md` still documents a materially different contract:

- it advertises non-shipped MCP tools such as `memory_ingest`, `memory_links`, `memory_unlink`, `memory_timeline_add`, `memory_tag`, and `memory_gap_approve`
- it describes concurrency rules and research flows built around those non-shipped tools
- it therefore contradicts the live server surface in `src/mcp/server.rs`, which exports only the 17 shipped tools

This is still user-breaking for the hard rename release because the primary spec remains the delegated source of truth for integration work. An agent or operator following `docs/spec.md` will implement against tools that do not exist.

## Required revision

A docs owner must revise `docs/spec.md` so its MCP/CLI contract matches the shipped Quaid surface exactly:

1. keep the renamed `quaid` / `memory_*` / `~/.quaid/memory.db` / `quaid-*` release-asset contract
2. replace the obsolete MCP matrix and surrounding prose with the real 17-tool surface from `src/mcp/server.rs`
3. remove or clearly defer any flows that depend on non-shipped tools

Until that happens, the hard rename is **not** safe to ship.

# Nibbler final hard-rename review

- **Verdict:** REJECT
- **Change:** `openspec/changes/quaid-hard-rename`
- **Reviewer:** Nibbler
- **Requested by:** macro88

## Blocking artifact

### `.github/workflows/release.yml`

The generated GitHub Release body is still success-shaped for a routine patch release instead of a hard-breaking rename:

1. It opens with install snippets and a watcher hotfix summary, not an explicit breaking-rename warning.
2. It does not point users at `MIGRATION.md` even though the rename makes old binaries, env vars, MCP tool names, and databases incompatible.
3. It still says `npm installs the online channel`, which conflicts with the current migration/docs story and can mislead operators into a nonexistent or not-yet-live upgrade path.

## Required revision

A different agent must rewrite the release-note body in `.github/workflows/release.yml` so the published release opens with a clear **BREAKING RENAME** callout, links to `MIGRATION.md`, states there is no in-place upgrade path, and removes or truthfully narrows the npm claim.

# Professor — Absolute Final Approval

**Date:** 2026-04-26
**Requested by:** macro88
**Change:** `openspec/changes/quaid-hard-rename`
**Verdict:** **APPROVE**

I re-checked the final revision against the last two concrete blockers and the ship-truth surfaces.

- `src/commands/call.rs` now routes `memory_collections` and includes a direct dispatcher test.
- `.github/workflows/publish-npm.yml` no longer auto-publishes on release tags; it is manual-only and now matches the docs/checklist/release notes that say npm is staged, not live.
- `README.md`, `docs/getting-started.md`, `docs/spec.md`, `MIGRATION.md`, `.github/RELEASE_CHECKLIST.md`, and `.github/workflows/release.yml` now tell one coherent rename/migration story: `quaid` binary, `memory_*` MCP tools, `QUAID_*` env vars, manual migration for existing databases, and no promise of live npm distribution in this release.
- `src/mcp/server.rs` and the CLI/docs surfaces are aligned on the shipped MCP set, including `memory_collections`.

I did not find a remaining design-integrity, maintainability, or release-truth blocker in the reviewed artifacts. This rename is finally approvable.

# Professor final review — quaid hard rename

Requested by: macro88
Date: 2026-04-26
Verdict: REJECT

## Why

The rename is not yet approvable.

1. Migration/cutover guidance is still non-operational in user-facing docs. `MIGRATION.md` and `website/src/content/docs/how-to/upgrade.mdx` still use placeholders like `<old-binary>`, `<LEGACY>_DB`, `<old-prefix>_get`, and `<old-package-name>` instead of the real pre-rename values, so users cannot perform the required manual cutover from the published legacy surfaces.
2. Schema-version truth is still wrong in modified website docs. `website/src/content/docs/reference/configuration.mdx` and `website/src/content/docs/explanation/embedding-models.mdx` still say schema version `5` even though the code and schema are at `6`.

## Required revision

A different agent must revise:
- `MIGRATION.md`
- `website/src/content/docs/how-to/upgrade.mdx`
- `website/src/content/docs/reference/configuration.mdx`
- `website/src/content/docs/explanation/embedding-models.mdx`

The revision must make the migration steps concrete and mechanically actionable, and it must bring every schema-version statement back into alignment with the actual shipped schema.

## 2026-04-26 — Quaid hard-rename final release approval

**By:** Professor  
**Verdict:** REJECT

The rename is not approvable yet because public operator surfaces are still internally inconsistent on two material truths. `docs/getting-started.md` still says `quaid init` creates the “full v5 schema” even though `src\schema.sql` and `src\core\db.rs` are on schema version 6, and both `README.md` and `docs/getting-started.md` still advertise restore surfaces (`quaid collection restore`, quarantine restore, `discard|restore`) that conflict with the current team gate in `.squad/identity/now.md`, which says restore has been backed out of the live CLI surface again.

**Artifacts that must be revised before approval:**

1. `docs/getting-started.md` — fix schema-version truth (`v5` → current v6 truth) and remove/retell restore claims to match the currently approved live surface.
2. `README.md` — remove/retell restore claims (`collection restore`, quarantine restore, `discard|restore`, “narrow Unix restore”) so the release-facing docs match the active gate.

Release workflow, checklist, migration guide, configuration reference, schema DDL, and `db.rs` no longer show the earlier rename blockers I was asked to watch; the remaining block is public-surface truthfulness.

# Professor Final Tree Approval

**Date:** 2026-04-27  
**Requested by:** macro88  
**Verdict:** **REJECT**

## Blocking MCP surface mismatch

The rename is not approvable as-is because one shipped surface still contradicts the documented “all 17 MCP tools are now `memory_*`” contract.

- `src\mcp\server.rs` exposes `memory_collections` as part of the live MCP surface.
- `README.md`, `docs\spec.md`, and `MIGRATION.md` all describe a 17-tool `memory_*` surface that includes `memory_collections`.
- But `src\commands\call.rs` never dispatches `memory_collections`; the match only covers the first 16 renamed tools, so `quaid call memory_collections ...` still falls through to `unknown tool`.

## Why this blocks approval

This is not a cosmetic omission. `quaid call` is the repo’s local MCP harness, and the hard-rename contract says the new `memory_*` surface is complete and truthful everywhere. Shipping with one renamed tool missing from the harness means the tree still does not fully deserve the “hard rename complete” claim.

# Professor MCP Spec Signoff

Requested by: macro88  
Date: 2026-04-26

Decision: APPROVE

The previous blocker is cleared. `docs/spec.md` now describes the shipped MCP surface truthfully: 17 `memory_*` tools on `quaid serve`, explicit Unix gating for MCP hosting, and the real `quaid call` limitation that still excludes `memory_collections`.

Review basis:
- `docs/spec.md` MCP table and behavior notes match `src/mcp/server.rs`
- `README.md` and `docs/getting-started.md` now hand off to the spec without overstating the shipped contract
- `src/commands/call.rs` still routes 16 tools, and the spec now says so explicitly
- Validation rerun: `cargo test --quiet mcp::server` passed

No remaining blocker found in the requested artifacts for the hard rename signoff.

## 2026-04-26 — Professor rereview: quaid-hard-rename remains blocked

**Verdict:** REJECT

### What cleared
- `src\core\db.rs` now truthfully runs at `SCHEMA_VERSION = 6` and reads/writes `quaid_config`.
- `website\src\content\docs\how-to\upgrade.mdx` now at least points readers to a dedicated migration guide.

### Blocking issues still open
1. **Migration guidance is still not trustworthy.**
   - `website\src\content\docs\how-to\upgrade.mdx` tells users to run `<old-binary> export > backup/`.
   - `MIGRATION.md` tells users to run `gbrain export --out backup/`.
   - The real CLI surface is `export <PATH>` (positional path; no stdout export and no `--out` flag). Publishing commands that do not exist is a ship blocker.

2. **Published schema version labeling is still inconsistent.**
   - `src\schema.sql` is labeled “Quaid v6” but still seeds `config.version = '5'`.
   - Published docs still describe the live schema as v5 (`docs\getting-started.md`, `docs\contributing.md`, `docs\roadmap.md`, `docs\openclaw-harness.md`).
   - Until the repo presents one truthful schema version, the earlier schema-labeling rejection is not cleared.

### Required revision lane
- A docs-focused agent must fix the migration commands in `website\src\content\docs\how-to\upgrade.mdx` and `MIGRATION.md`.
- The same pass must make `src\schema.sql` and all published docs speak with one truthful voice about schema v6 and the real `export` syntax.

# Professor review — quaid hard rename

Decision: REJECT
Requested by: macro88
Change: quaid-hard-rename

Rejected artifacts:
- website/src/content/docs/how-to/upgrade.mdx
- src/schema.sql

Why:
1. `website/src/content/docs/how-to/upgrade.mdx` points users to a root `MIGRATION.md` that does not exist. For a hard-breaking rename with no aliases or auto-migration, sending operators to a missing migration reference is a release-blocking documentation gap.
2. The same upgrade guide states that `quaid_config.schema_version` is currently `5`, but the shipped code bumped `SCHEMA_VERSION` to `6`. That makes the migration instructions factually wrong at the exact seam users need to trust.
3. `src/schema.sql` still advertises `Quaid v5` in its header while the runtime schema version is `6`. That is secondary to the broken upgrade guide, but it reinforces the version-truth mismatch around a breaking schema change.

Required revision by a different agent:
- Fix the upgrade/migration documentation so every referenced artifact exists and the documented schema version matches the shipped runtime.
- Align schema-file version labeling with `src/core/db.rs` before this rename lands.

## 2026-04-26 — Professor approval: README/getting-started hard-rename docs

Approved the rename updates in `README.md` and `docs/getting-started.md`.

Why this clears:
- README/getting-started now match the coupled source truths for the hard rename: `SCHEMA_VERSION` is `6` in `src/core/db.rs`, the default DB path is `~/.quaid/memory.db`, and the persisted model metadata table is `quaid_config`.
- The previously overstated public surface has been corrected: npm install is explicitly not published yet, quarantine restore is no longer claimed as part of the current release surface, and the quarantine docs now name only `list`, `export`, and `discard` as landed.
- The reviewed docs no longer carry the residual legacy product/binary literals that previously blocked approval.

Review boundary:
- `README.md`
- `docs/getting-started.md`
- `src/core/db.rs`
- `src/schema.sql`
- current working tree diff for the touched docs

Result: APPROVE the rename on this docs lane.

# 2026-04-26 — Professor rename/spec final approval

**Verdict:** REJECT

**Why:** `docs/spec.md` still does not match the shipped MCP contract, so `README.md` and `docs/getting-started.md` cannot safely point to it as authoritative yet.

## Blocking artifact
- `docs/spec.md`

## Required revision
Update the MCP tools section to the real landed surface from `src/mcp/server.rs`:
`memory_get`, `memory_put`, `memory_query`, `memory_search`, `memory_list`, `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph`, `memory_check`, `memory_timeline`, `memory_tags`, `memory_gap`, `memory_gaps`, `memory_stats`, `memory_collections`, `memory_raw`.

Remove or rewrite stale entries that are not shipped as MCP tools (`memory_ingest`, `memory_links`, `memory_unlink`, `memory_timeline_add`, `memory_tag`, `memory_gap_approve`). If spec remains the authority target, its tool signatures must match `src/mcp/server.rs` exactly.

## Professor review — quaid-hard-rename

- Date: 2026-04-26
- Requested by: macro88
- Verdict: REJECT

### Blocking issue

The rename is not yet zero-trace complete. `.github/RELEASE_CHECKLIST.md` still contains the legacy package path `packages/gbrain-npm/` in the npm release gate, which reintroduces the retired product name into a team-facing artifact and leaves Phase J.1 materially open.

### Required revision

A different agent should revise `.github/RELEASE_CHECKLIST.md` so the npm release gate verifies the new package surface without naming the retired package directly, then rerun the zero-trace audit.

# Scruffy — Batch 1 watcher reliability coverage memo

Batch 1 is **not** ready to claim as shipped from a coverage perspective. In the current worktree, the `.quaidignore` watcher lane now has real code and direct tests, but the production seams for `6.7a`, `6.9`, `6.10`, and `6.11` / proofs `17.5w`, `17.5x`, `17.5aaa2`, `17.5aaa3`, `17.5aaa4` are still missing. The honest move is to keep the remaining Batch 1 tasks open and aim coverage directly at those branches.

## What I safely landed now

- `src/commands/collection.rs`
  - `describe_collection_status_reports_plain_restoring_without_finalize_hint`
  - `describe_collection_status_points_active_reconcile_needed_to_plain_sync`
- `src/core/vault_sync.rs`
  - `list_memory_collections_only_marks_restore_in_progress_after_release_ack`

These are low-conflict branch guards around the already-landed status surface:
- restoring vs pending-attach vs active-needs-reconcile CLI gating
- `memory_collections.restore_in_progress` requiring a real restore ack, not just stray timestamps or command IDs

## Coverage state by requested focus

### 1. Overflow recovery timing / active-vs-restoring gating

Current state:
- Overflow **marks** `needs_full_sync=1` today (`start_collection_watcher` TrySendError::Full branch).
- There is **no** recovery worker yet in `start_serve_runtime()`.
- `run_watcher_reconcile()` already fail-closes to `state='active'`, which is the right gating model for the future recovery worker.

Tests still needed once Fry lands the worker:
- `17.5w`: seed `needs_full_sync=1`, `state='active'`, start serve, assert the flag clears within ~1s and `last_sync_at` advances.
- `17.5x`: same seed with `state='restoring'`, assert the flag stays set, no reconcile runs, and write gating remains closed.
- Add one explicit branch test for repeated poll ticks before 500ms doing nothing, then the 500ms+ branch firing exactly once.

### 2. `.quaidignore` live reload success / parse-error / delete-with-prior-mirror

Current state:
- Parser/mirror semantics are already well-covered in `src/core/ignore_patterns.rs`:
  - valid reload updates mirror
  - invalid reload preserves prior mirror and records parse errors
  - deleted file with prior mirror preserves mirror and records stable-absence error
- The watcher delivery seam is now covered in `src/core/vault_sync.rs`:
  - ignore-file events emit `WatchEvent::IgnoreFileChanged`
  - valid reload updates the mirror and still triggers reconcile
  - invalid reload preserves the mirror, records errors, and skips reconcile
  - deleted file with a prior mirror preserves the mirror and skips reconcile

Residual test value:
- keep one branch test for “no prior mirror + file absent” staying quiet/default-only if Fry threads that through the watcher path rather than relying only on helper coverage
- once tasks are updated, `17.5y`, `17.5z`, and `17.5aa` look credibly covered by the direct watcher tests now in `src/core/vault_sync.rs`

### 3. Native watcher init fallback to poll

Current state:
- `start_collection_watcher()` hard-requires `notify::recommended_watcher(...)`; no fallback mode exists.

Tests still needed once mode plumbing lands:
- inject native watcher init failure and assert watcher state records `poll`
- assert warn log mentions `falling_back_to_poll`
- assert the poll-backed watcher still feeds the same channel and processes a real edit
- add one health-surface assertion that `memory_collections.watcher_mode == "poll"` and CLI info reports the same

### 4. Watcher crash / restart backoff behavior

Current state:
- Channel disconnect currently returns `InvariantViolation`; there is a unit test for that raw error.
- No crash state, no restart tracking, no exponential backoff, no restart proof.

Tests still needed once crash supervision lands:
- simulate disconnect, assert watcher enters `crashed` mode and records failure timestamp
- assert `sync_collection_watchers()` refuses to restart before `backoff_until`
- assert first restart waits ~1s, second consecutive failure backs off longer, cap respected at 60s
- assert a successful restart resets the consecutive-failure backoff chain

### 5. Watcher health surfacing / fragile channel-depth + timestamp + inactive branches

Current state:
- `memory_collections` and `quaid collection info` do **not** yet expose watcher mode, last-event timestamp, or channel depth.
- I added branch guards for adjacent status truth only:
  - plain restoring vs pending attach vs active reconcile needed
  - restore-in-progress requiring real ack state

Tests still needed once health fields land:
- inactive/detached collection reports `watcher_mode="inactive"` (or null on non-Unix), `last_event_at=null`, `channel_depth=0`
- active collection with no events yet reports live mode + null timestamp
- queued events update `channel_depth` without consuming them just to report health
- processed event updates `watcher_last_event_at` monotonically
- crashed watcher reports `watcher_mode="crashed"` while retaining last known timestamp/depth semantics

## Practical approval bar from the test lane

I would not sign off on “Batch 1 shipped / v0.10.0 ready” until overflow recovery, native→poll fallback, crash backoff/restart, and watcher-health surfacing all exist with the tests above. Right now the honest statement is: watcher-core debounce is covered, `.quaidignore` live reload is materially covered, and the remaining Batch 1 reliability surface is still missing its implementation-coupled proof set.

# Decision: Remove legacy package-path leak from RELEASE_CHECKLIST.md

**Author:** Zapp
**Date:** 2026-04-25
**Artifact:** `.github/RELEASE_CHECKLIST.md` line 97 (pre-fix)

## Context

Professor rejected the quaid-hard-rename review because `.github/RELEASE_CHECKLIST.md:97`
explicitly named `packages/gbrain-npm/` — a retired path that violates the zero-trace
rule for `gigabrain`, `gbrain`, and `brain_` across all non-hidden surfaces.
The original item read:

> `packages/quaid-npm/` is committed and tracked; `packages/gbrain-npm/` is fully removed
> from the tree.

## Decision

Rewrote the checklist item to remove the named legacy path while preserving the intent:

> `packages/quaid-npm/` is committed and tracked; no retired npm package directory exists
> in the tree. The publish workflow will fail if `packages/quaid-npm/` is absent.

The new wording is truthful (it still gates on the presence of the current package dir
and absence of any legacy dir) without naming the legacy path explicitly.

## Rationale

- The zero-trace rule exists to prevent any surface from teaching users or bots the old
  package name. A release checklist is a non-hidden surface reviewed during every release.
- The fix is purely phrasing — no behavioural or workflow change is implied.
- Previous author of this line is locked out; Zapp holds the release-copy sign-off lane
  and is authorized to revise this entry.

## Follow-on

None required. The checklist is now clear of all legacy identifiers. Phase F.4 task in
`openspec/changes/quaid-hard-rename/tasks.md` remains checked; this is a correction to
work already landed, not a new implementation.

# Decision: Hard-break migration docs rewrite

**Date:** 2026-04-28
**Author:** Zapp
**Triggered by:** macro88 directive + Professor/Nibbler rejection of placeholder-based migration story

---

## Context

`MIGRATION.md` and `website/src/content/docs/how-to/upgrade.mdx` were previously rejected
by Professor and Nibbler because the placeholder approach (`<old-binary>`, `<LEGACY>_`,
`<old-prefix>_`) was not actionable. Leela's prior fix (masking literal names with
placeholders) was itself a partial answer — it avoided the zero-trace violation but still
presented a step-by-step in-place migration story that no longer has a basis: the governing
directive is that no supported in-place upgrade path exists, and no backward compatibility
is warranted.

Leela and Bender are locked out of revising these artifacts for this cycle.

---

## Decisions taken

### 1. `MIGRATION.md` — full rewrite, hard-break stance

Dropped all "Before → After" table rows that required naming the pre-rename binary.
Replaced with:
- Prominent hard-break callout at the top (no in-place upgrade path exists).
- A single "What Quaid looks like now" table (new surface only; no before column).
- Fresh install and shell profile sections (quaid-only).
- MCP client config section (clean rebuild; no reference to old config shape).
- Data recovery section that accurately states: this repo does not carry a migration tool;
  users must locate their own pre-rename binary and consult that release's docs, then
  re-init with quaid. No placeholder binary commands.
- npm section: actionable (`npm list -g --depth 0` to identify; uninstall; install quaid).

### 2. `website/src/content/docs/how-to/upgrade.mdx` — Aside rewrite

Dropped the "Before → After" surface table (it required legacy placeholder names) and
the step-by-step that referenced `<old-binary>` and `<old-package-name>`. Replaced the
entire Aside with a clean hard-break statement:
- States clearly: cannot upgrade in place, must start fresh.
- Four numbered actions (install, rebuild MCP config, update shell profile, recover data).
- Data recovery: points users to their own historical binary and GitHub Release history
  for that project; this repo does not carry a migration tool.
- Links to MIGRATION.md for full reference.

### 3. `README.md` — hard-break warning added

Added a prominent `> ⚠️` callout immediately after the status line, before the body of
the README. Visible at the top of the page on GitHub. Links to MIGRATION.md.

### 4. `docs/getting-started.md` — hard-break warning added

Added the same callout immediately after the tagline at the top, before the What it does
section.

---

## Rationale

The previous approach (placeholders) was rejected because it still implied an actionable
in-place migration the repo can't actually document without either (a) naming the old binary
or (b) being useless. The correct stance — per the governing directive — is that there is
no in-place upgrade path and that legacy data recovery is the user's responsibility using
historical tooling outside this repo. Stating this plainly is more honest and more useful
than a broken-placeholder step-by-step.

---

## Zero-trace check

Scanned all four touched files for `gigabrain`, `gbrain`, and `brain_` — zero matches.

# Decision: quaid-hard-rename cutover fix — Zapp

**Date:** 2026-04-25
**Author:** Zapp
**Triggered by:** Nibbler reviewer rejection of three blocked rename artifacts

---

## Context

Nibbler rejected the rename artifacts with three specific findings:

1. `.gitignore` still referenced `packages/gbrain-npm/bin/gbrain.bin` and
   `packages/gbrain-npm/bin/gbrain.download` (legacy paths). The `memory.db` default was
   also missing from the ignore list.
2. `.github/RELEASE_CHECKLIST.md` and `website/src/content/docs/how-to/upgrade.mdx`
   described upgrade steps as if this were a routine patch, with no explicit callout that
   the rename is hard-breaking and requires manual user-side migration.
3. `packages/quaid-npm/` existed on disk but was untracked (`??` in git status) while
   `.github/workflows/publish-npm.yml` already declared
   `working-directory: packages/quaid-npm` — a missing-path failure waiting to occur.

Fry, Amy, and Hermes are locked out of revising these artifacts. Zapp owns this revision.

---

## Decisions made

### D.1 — `.gitignore` legacy path removal
**Decision:** Remove the two `gbrain-npm` ignore entries; add the two corresponding
`quaid-npm` entries (`quaid.bin` and `quaid.download`). Also add `memory.db` and its
WAL/journal/SHM siblings alongside the existing `brain.db` entries (both defaults get
ignored; comment clarifies both are present).

**Rationale:** The `gbrain-npm` paths point to files that no longer exist on disk or in the
tree. Leaving them in `.gitignore` is inert noise that signals incomplete rename work to any
reviewer doing a grep for legacy names. The `memory.db` omission was a genuine gap: the new
default DB path would have been committable by accident.

### D.2 — RELEASE_CHECKLIST.md rename migration gate
**Decision:** Insert a dedicated "Hard-breaking rename migration gate" section into the
checklist before the "Release notes" section. The new section carries seven explicit
line-item checkboxes covering binary, MCP tool, env var, DB migration, npm package, and
package tree consistency. Zapp is the named sign-off owner on this gate. The existing
sign-off table gains a "Hard-breaking rename migration gate / Zapp" row.

**Rationale:** A checklist that described asset names and checksums but had no line item
for the most disruptive user-visible change this project has ever shipped would have let a
release go out without explicit confirmation that migration docs exist. The gate is Zapp's
because migration messaging is DevRel/growth surface work.

### D.3 — upgrade.mdx hard-breaking rename callout
**Decision:** Prepend a `<Aside type="danger">` component to the upgrade guide with a
before/after table of every renamed surface, five numbered manual steps the user must take,
and a pointer to `MIGRATION.md`. The existing routine upgrade steps follow unchanged below.

**Rationale:** A docs page titled "Upgrade your binary" that silently omits a breaking
rename is a support incident factory. The callout is typed `danger` (not `caution`) because
clients will silently stop working if MCP configs are not updated — that is a data-access
outage, not a warning-level concern.

### D.4 — install.sh rename notice block
**Decision:** Insert a multi-line rename notice block into `scripts/install.sh` immediately
after the "Installed quaid to <path>" success message. The block lists the three required
manual steps (MCP config, shell profile, DB migration) and links to the GitHub repo for the
full migration guide.

**Rationale:** `install.sh` is the primary upgrade path for existing binary users. A user
who runs `install.sh` to upgrade from a pre-rename binary will complete the install
successfully — and then have a broken MCP setup with no indication of why. The notice makes
the post-install action required, not discoverable only by reading release notes.

### D.5 — packages/quaid-npm/ git tracking
**Decision:** `git add packages/quaid-npm/` to track the directory, and
`git rm --cached packages/gbrain-npm/*` to deindex the already-deleted gbrain-npm files
(they showed as `D` in git status). Git detects the four files as renames from
`gbrain-npm/` to `quaid-npm/`, which is correct history.

**Rationale:** The publish workflow's `working-directory: packages/quaid-npm` step would
have failed on any clean CI checkout because the directory was untracked. This was the
most directly breaking of the three Nibbler findings.

---

## Files changed

| File | Change |
|------|--------|
| `.gitignore` | Remove gbrain-npm entries; add quaid-npm entries; add memory.db entries |
| `.github/RELEASE_CHECKLIST.md` | Add rename migration gate section + sign-off row |
| `website/src/content/docs/how-to/upgrade.mdx` | Add `<Aside type="danger">` rename callout |
| `scripts/install.sh` | Add rename notice block after successful install |
| `packages/quaid-npm/` (git index) | Staged for tracking |
| `packages/gbrain-npm/` (git index) | Removed from index (files already deleted) |

---

## What was NOT changed

- `README.md` — not part of the blocked artifacts; existing rename prose is sufficient
  for this fix pass.
- `docs/` content — migration guide authorship (Phase K of tasks.md) is deferred; Amy/Scribe
  own K.1 and are not locked out of that task.
- `publish-npm.yml` — already correct; no changes needed.
- `packages/quaid-npm/` file content — all four files (package.json, README.md,
  bin/quaid, scripts/postinstall.js) were already correct; only the git tracking was missing.


