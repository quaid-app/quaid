# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Batch 1 collection final push (2026-04-28): on Windows, `src/commands/collection.rs` still moves with direct same-file helper tests, especially fail-closed dispatch, status-summary branches, ignore-mirror parsing, root validation, and offline restore/remap proofs that avoid Unix watcher/reconcile paths. This pass lifted `commands\\collection.rs` line coverage to 88.95% and global `cargo llvm-cov --lib --tests --summary-only --no-clean -j 1` to 89.40%, but the remaining gap to 90% is dominated by Unix-only success paths in collection/reconciler/vault-sync rather than more honest Windows-reachable branches.

- **Batch 1 Coverage Arc (2026-04-28/29) — Three-Pass Windows Recovery:**
  - **Pass 1 (Collection):** Targeted `src/commands/collection.rs` with direct helper tests for status formatting, ignore mutation, reset commands, root validation. Moved 54.01%→77.53% locally; global 84.51%→85.58% (+1.07 pts).
  - **Pass 2 (Core):** Targeted `fs_safety.rs`, `search.rs`, `quarantine.rs` with Windows-stub coverage and pure-logic branches. Measured global 85.58%→89.79% (+4.21 pts). Lane file wins: fs_safety 70.00%→100.00%, search.rs 85.20%→97.15%, quarantine.rs 90.07%→97.50%.
  - **Pass 3 (Heavy):** Targeted cross-platform helper seams and wrapper arms without claiming Unix-only bodies. Result: 88.76% (+0.97 pts from Pass 2 endpoint).
  - **Final authorized measurement (Zapp):** 90.77% from `target\llvm-cov-final.json` (Windows authoritative).
  - **Status:** v0.10.0 coverage gate CLEARED. Decision records (mom-collection-coverage.md, mom-core-coverage.md, mom-heavy-coverage.md, mom-collection-final.md) merged to decisions.md.
- Batch 1 collection coverage push (2026-04-28): on Windows, the real collection CLI ceiling is the Unix gate. The biggest honest gains came from testing private helper paths in `src/commands/collection.rs` directly (status formatting, ignore-file mutation, reset commands, root validation) while proving Windows-only helper failures stay closed and preserve DB state; that moved `commands\collection.rs` coverage from 54.01% to 77.53% and global `cargo llvm-cov --lib --tests --summary-only` from 84.51% (reported baseline) to 85.58%, but it cannot close the remaining Unix-only attach/sync/restore bodies.
- Batch 1 watcher reliability continuation (2026-04-28): in `src/core/vault_sync.rs`, watcher health is safest when it is derived from live `CollectionWatcherState` and published into the in-process supervisor registry as snapshots (`mode`, `last_event_at`, `channel_depth`). `quaid collection info` may surface those fields, but non-active collections and out-of-process CLI calls must stay `null`; do not invent an `Inactive` watcher mode or widen `memory_collections` in this release.
- Batch 1 watcher reliability continuation (2026-04-28): the parked-lane `CollectionAddArgs.watcher_mode` flag in `src/commands/collection.rs` was not an accepted contract. Remove dormant CLI knobs rather than building on them; watcher mode is runtime-owned state, not an attach-time user input in v0.10.0.
- Batch 1 coverage wave (2026-04-28): orchestration logs written for Scruffy (watcher lane), Bender (audit lane), Mom (batch 1 continue lane). Session log created. Four inbox decisions merged to canonical `.squad/decisions.md` (Batch 1 Coverage Audit, Batch 1 Release Decision + prior 2026-04-25 decisions). Inbox files deleted. Cross-agent histories (Scruffy, Bender, Mom) updated. Ready for git commit.
- Batch 1 edge fix (2026-04-27): the root `.quaidignore` file is a control surface, not content. In `src/core/vault_sync.rs`, watcher classification must bypass the markdown-only path filter, emit a dedicated reload event, and let `src/core/ignore_patterns.rs::reload_patterns()` decide whether reconcile is safe. On parse failure or stable absence with a prior mirror, keep last-known-good patterns and skip reconcile rather than walking with stale ignore state.
- Issue #79 / #80 macOS preflight audit (2026-04-25): PR #83's four macOS preflight jobs were not failing in `src/core/fs_safety.rs` anymore; they all died earlier in `.github/workflows/ci.yml` because `actions/cache` rejects cache keys containing commas from raw `matrix.features` values like `bundled,online-model`. The `stat.st_mode as u32` cast is present, but issue #80 stays operationally open until macOS preflight actually reaches `cargo check`.
- Vault-sync CI burndown lane closeout (2026-04-25): Mom's edge-case fix lane landed in commits 56e44ce and 18ac3d7. Four targeted decisions (D-V1 through D-V4) fixed 6 failing CI tests constrained to `src/core/vault_sync.rs` and `src/core/raw_imports.rs`. Orchestration log at `.squad/orchestration-log/2026-04-25T15-48-57Z-mom.md` documents all decisions and test results (591 pass, 2 pre-existing Windows failures unrelated).
- Quarantine restore artifact reconciliation (2026-04-25): Mom audited leftover restore glue from rejected Fry artifact, kept required pieces (fs_safety linkat, vault_sync leases, collection routing), dropped permanently-excluded `walk_to_parent_create_dirs`. Decisions D-MR1 and D-MR2 recorded in `.squad/decisions.md`. Commit 6a3d54c is wholly Mom-authored.
- Quarantine restore second revision — 5-blocker fix (2026-04-25): Mom fixed all 5 consolidated blockers (tempfile cleanup, post-install rollback, absent-parent refusal, task wording, contract narrowness). Decisions D-R1 through D-R5 merged into `.squad/decisions.md`. All tests pass (591 total, 0 new failures).
- Vault-sync CI fix decisions (2026-04-25): D-V1 (process registry isolation), D-V2 (frontmatter parsing), D-V3 (frontmatter sync), D-V4 (error format consistency) merged to `.squad/decisions.md`.

- Edge-case work is an explicit part of this squad, not an afterthought.
- The requested target model is Gemini 3.1 Pro when available on the active surface.
- Proposal-first work makes it easier to identify which assumptions deserve stress.

## 2026-04-15 Graph Temporal Gate Fix Resolution

- **Mom's edge-case note** on future-dated links was identified as part of initial graph slice review (directionality contract blockers).
- **Temporal gate gap:** The original graph query only checked `valid_until >= today` but did not gate `valid_from <= today`, which allowed future-dated links to appear in the "active" graph.
- **Resolution:** Leela's graph slice revision (tasks 1.1–2.5) incorporated the fix into decision D2. Active temporal filter now enforces:
  ```sql
  (l.valid_from IS NULL OR l.valid_from <= date('now'))
  AND (l.valid_until IS NULL OR l.valid_until >= date('now'))
  ```
- **Status:** INCORPORATED. Graph slice approved for landing on `phase2/p2-intelligence-layer` 2026-04-15T23:15:50Z.
- **Lessons:** Edge-case work is most effective when it surfaces during contract-review blockers, not during post-landing firefighting. Mom's temporal concern directly influenced the final graph design.

## 2026-04-17 Phase 3 MCP Rejection Fixes (brain_raw + brain_gap + pipe)

- **Context:** Fry's Phase 3 MCP implementation was rejected by Nibbler on four specific grounds. Mom assigned as revision author while Fry is locked out of this cycle.
- **Fixes shipped:**
  - `brain_raw` now rejects non-object payloads (array/scalar) with `-32602`.
  - `brain_raw` now has an `overwrite: Option<bool>` field; silent `INSERT OR REPLACE` is blocked — returns `-32003` conflict if `overwrite` is not explicitly `true`.
  - `brain_gap` now caps `context` at 500 characters (`MAX_GAP_CONTEXT_LEN`) to prevent privacy leakage through the context sidecar.
  - `gbrain pipe` now blocks JSONL lines exceeding 5 MB (`MAX_LINE_BYTES`), emitting an error per oversized line and continuing — no process crash.
- **Tests added:** 7 new targeted edge-case tests covering all four rejection points plus boundary conditions.
- **All 282 tests pass. Clippy clean.**
- **Task 8.2 left pending** — Nibbler re-review required before it can close.
- **Decision record:** `.squad/decisions/inbox/mom-phase3-mcp-fixes.md`
- **Lesson:** The `INSERT OR REPLACE` pattern is a latent data-loss hazard. Any store-to-keyed-table operation should require an explicit opt-in for destructive replacement. The context-as-privacy-vector risk is subtle but real — bounded fields are the right default for any input that touches the privacy model.

---

## 2026-04-16 Phase 3 Task 8.2 — MCP Edge-Case Fixes (mom-phase3-mcp-fixes)

**Session:** mom-phase3-mcp-fixes (2309s, claude-sonnet-4.6)  
**Timestamp:** 2026-04-16T07:20:47Z

**What happened:**
- Task 8.2 REVISION COMPLETE: Addressed all four Nibbler Phase 3 MCP review blockers.
  - Decision D-M1: `brain_raw` data field restricted to JSON objects only. Non-objects rejected with `-32602`.
  - Decision D-M2: `brain_raw` now requires explicit `overwrite=true` to replace existing `(page_id, source)` rows. Silent replacement blocked; returns `-32003` conflict error with guidance.
  - Decision D-M3: `brain_gap` context capped at 500 characters. Longer values rejected with `-32602`. Prevents privacy leakage through context sidecar.
  - Decision D-M4: `gbrain pipe` blocks oversized JSONL lines at 5 MB (`MAX_LINE_BYTES`). Emits error per line, continues processing — no process crash.
- 4 decisions merged to `decisions.md`.
- 7 targeted tests added. All 282 tests pass. Clippy clean.
- Orchestration log written.
- **Status:** Task 8.2 left for re-review by different reviewer per phase 3 workflow (Nibbler).

**Next:** Await Nibbler re-review of all fixes before closing task 8.2.

## Learnings

### 2026-04-19 Import Diagnostics Fix (issues #34, #35, #39)

**What happened:**
- Fixed two beta-tester reported issues in the import diagnostics lane.
- **#34 / #39 (duplicate):** `embed::run` batch loop changed from fail-fast (`?`) to per-slug error handling. Each failed page emits `warning: embedding skipped '<slug>': <reason>` and the batch continues. Infrastructure-level failures still propagate.
- **#35:** `ImportStats.skipped` replaced with `skipped_already_ingested` + `skipped_non_markdown` + `total_skipped()`. Non-markdown files are now counted by `collect_files()` (renamed from `collect_md_files`). Import output message shows the breakdown.
- 5 new tests added; 288 total pass. Clippy clean.
- PR #45 opened referencing both issues. #39 closed as duplicate.
- Decision record: `.squad/decisions/inbox/mom-import-diagnostics.md`

**Key files:**
- `src/core/migrate.rs` — ImportStats struct, collect_files(), import_dir()
- `src/commands/embed.rs` — per-slug batch error handling
- `src/commands/import.rs` — output message with skip-reason breakdown
- `tests/corpus_reality.rs` — integration test using ImportStats fields

**Architecture note:**
- `chunk_page()` in current code cannot produce empty-content chunks (all code paths guard against it), so the "input text is empty" error the user saw was likely a transient or historical condition. The fix is still correct and valuable as a defensive guardrail.
- Naming convention for ImportStats fields: each skip reason gets its own named field — never fold multiple reasons into a catch-all counter.

### 2026-04-24 Vault Sync 13.3 Third Revision

**What happened:**
- Closed the remaining `embed` explicit-routing hole without widening beyond `13.3`: single-page embed now resolves `<collection>::<slug>` first and binds the embedding write by `(collection_id, slug)` instead of falling back to a bare `pages.slug = ?` lookup.
- Added direct subprocess proofs for the two missing CLI surfaces Scruffy flagged: `query work::notes/meeting` now succeeds even when bare `notes/meeting` is ambiguous, and `unlink work::notes/a memory::notes/b --relationship relates` removes only the explicitly addressed edge.
- Added a focused embed regression test covering duplicate slugs across collections so explicit embed cannot silently drift back to bare-slug page-id binding.
- Validation: targeted embed/query/unlink tests passed, then full `cargo test --locked` passed.

**Lesson:**
- For CLI slug parity, resolving the page and then doing any later raw `WHERE slug = ?` lookup is not a harmless shortcut — it reopens the duplicate-slug bug through a second, quieter path. The safe pattern is resolve once, then carry `(collection_id, slug)` all the way through every downstream lookup and proof.

### 2026-04-25 Vault Sync 13.5 Repair — `brain_query` cross-collection expansion fix

**Context:** Fry authored slice 13.5 (MCP-only read filter). Nibbler rejected; Mom assigned as revision author.

**What happened:**
- `brain_query` correctly scoped the initial `hybrid_search_canonical(...)` call to the effective collection filter, but when `depth="auto"`, `progressive_retrieve(...)` was called without that filter, allowing `outbound_neighbours()` to follow cross-collection links and return pages from outside the requested/defaulted collection.
- Fix: added `collection_filter: Option<i64>` parameter to `progressive_retrieve` and `outbound_neighbours`. The SQL now includes `AND (?3 IS NULL OR p2.collection_id = ?3)` so target pages are constrained to the active collection when a filter is in effect. When `?3 IS NULL` (CLI path, which always passes `None`), the clause is a no-op, preserving existing CLI behaviour.
- `brain_query` in `server.rs` now passes `collection_filter.as_ref().map(|c| c.id)`.
- `commands/query.rs` passes `None` (no collection filter concept in CLI path).
- All existing `progressive_retrieve` unit tests updated with `None`.
- New test `brain_query_auto_depth_does_not_expand_across_collections` added to `server.rs` — creates a cross-collection link and asserts the `work::` page never appears in `default`-scoped `depth="auto"` results.
- All three validation passes green: `cargo test --quiet mcp::server` (101 tests), `cargo test --quiet` (full suite), `GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model`.

**Decision record:** `.squad/decisions/inbox/mom-13-5-repair.md`

**Lesson:**
- When a filter is established at the query entry point, it must be threaded through every subsequent expansion step. A filter that only covers the seed set but not the BFS frontier is a half-fence. The `?3 IS NULL OR p2.collection_id = ?3` pattern is the right idiom: one SQL clause handles both the filtered (MCP) and unfiltered (CLI) call sites without branching the prepared statement or duplicating the query.

### 2026-04-25 Vault-Sync Edge Case Audit — Read-Only Coverage Survey

**Context:** User commissioned a read-only audit of vault-sync test coverage gaps. No code was modified. Deliverable: written audit report, decision record, and skill file.

**Scope surveyed:**
- `src/core/vault_sync.rs` (watcher runtime, ownership, write guards, session management)
- `src/core/quarantine.rs` (list, export, discard, sweep, export audit trail)
- `src/commands/collection.rs` (CLI dispatch, restore deferral, confirm gates)
- `tests/quarantine_revision_fixes.rs` (5 process-level integration tests)
- `tests/watcher_core.rs` (1 Unix-only integration test)

**Key findings:**

1. **Deferred-restore tests test the bail, not the behavior.** All four restore tests in `quarantine_revision_fixes.rs` assert the same `"quarantine restore is deferred in this batch"` substring. They verify that restore is correctly disabled — but none exercise the validation logic their names describe (non-.md extension check, live-owner gate, conflict detection, read-only guard). When restore is re-enabled, all four tests must be rewritten. The test setups are correct but the assertions are parking-lot placeholders.

2. **Discard happy-path after successful export is untested.** `blocker_1_failed_export_does_not_unlock_discard` proves the negative (failed export → still blocked). The positive (successful export + db_only_state → non-force discard succeeds) has no test anywhere. This is the highest-value missing unit test.

3. **`discard` with `force=true` is also untested.** The force path bypasses the export guard unconditionally. No test exists at any level.

4. **`discard_quarantined_page` calls `ensure_collection_write_allowed` not `ensure_collection_vault_write_allowed`.** This is a policy decision, not necessarily a bug — discard is a pure SQLite DELETE with no vault bytes touched. But the distinction is undocumented and there is no test asserting this is the intended contract. Read-only collections (`writable=false`) can have quarantined pages discarded.

5. **`record_quarantine_export` upserts silently.** Re-exporting a page overwrites `exported_at` and `output_path` via `ON CONFLICT DO UPDATE`. No test covers re-export behavior. The current test only verifies epoch-matching logic.

6. **`sweep_expired_quarantined_pages` with `GBRAIN_QUARANTINE_TTL_DAYS=0` is untested.** The TTL-zero edge case (sweep everything) is never exercised. Current tests use a fixed past date which bypasses the boundary.

7. **Watcher integration coverage is minimal.** One Unix-only test exists (`start_serve_runtime_defers_fresh_restore_without_mutating_page_rows`). Missing: channel overflow → `needs_full_sync` escalation; reconcile-halt via watcher path; non-.md file event filtering; watcher replacement on generation bump.

8. **`QUARANTINE_SWEEP_INTERVAL_SECS` is hardcoded (86400 secs).** No env-var override exists, making it impossible to write an integration test for the serve-loop sweep without a 24-hour clock dependency.

**Architecture note — `ensure_collection_write_allowed` vs `ensure_collection_vault_write_allowed`:**
- `ensure_collection_write_allowed` → gates on `state` + `needs_full_sync` only
- `ensure_collection_vault_write_allowed` → additionally checks `writable` flag
- `discard_quarantined_page` uses the first (no writable check) — intentional because discard writes no vault bytes
- `export_quarantined_page` uses `resolve_slug_for_op(OpKind::Read)` — no write gate at all, which is correct (export is read-only)
- This distinction should be tested and documented, not left implicit

**`ensure_collection_not_live_owned` does not exist as a named function.** History.md (Mom's prior entry) says it was added to `vault_sync.rs`, but Bender's truth repair backed out the entire restore implementation. The function is not present in the codebase. Ownership enforcement for non-serve code paths goes through `acquire_owner_lease` + `session_is_live` inline check.

**Decision record:** `.squad/decisions/inbox/mom-vault-sync-edge-audit.md`
**Skill file:** `.squad/skills/gate-vs-bail-test-discipline/SKILL.md`

---

### 2026-04-25 Quarantine Lifecycle Revision (9.8 Full Closure)

**Context:** Fry authored quarantine slice; Professor and Nibbler rejected on four specific blockers. Mom assigned as revision author while Fry locked out of this cycle.

**What happened:**
- Fixed all four rejection blockers in narrow, targeted edits:
  1. **Export ordering:** `export_quarantined_page()` now writes filesystem first, only records `quarantine_exports` row on success. Failed export no longer unlocks discard.
  2. **Restore `.md` validation:** `restore_target_relative_path()` now returns `Result`, validates final extension is `.md` after auto-append. Prevents `.txt`, `.pdf` writes.
  3. **Restore live-owner gate:** Added `ensure_collection_not_live_owned()` to `vault_sync.rs`, wired into `restore_quarantined_page()`. Restore now refuses when serve owns collection.
  4. **Restore atomicity:** Reordered restore to start SQLite tx first, update DB state, then write filesystem bytes, commit only after all steps succeed. Rollback on any failure prevents residue.
- Added 4 focused tests in `tests/quarantine_revision_fixes.rs` proving each blocker fix.
- Existing lib-level quarantine tests remain green.
- Task `9.8` now closes fully (default quarantine surface complete).

**Decision record:** `.squad/decisions/inbox/mom-quarantine-revision.md`

**Key files:**
- `src/core/quarantine.rs` — export ordering fix, restore validation + atomicity reordering
- `src/core/vault_sync.rs` — `ensure_collection_not_live_owned()` helper
- `tests/quarantine_revision_fixes.rs` — 4 targeted blocker tests

**Lesson:**
- **Export-first is the wrong order for any operation that records success state.** Write the effect first, record the tracking row second. This matches PUT's write-then-register pattern and prevents failed writes from unlocking downstream relaxations.
- **Validation after auto-convenience is safer than before.** Auto-appending `.md` is user-friendly; validating the final result afterward prevents inadvertent bypasses without removing the convenience.
- **Transaction-first rollback is the canonical atomicity pattern for multi-resource commits.** Filesystem-first + cleanup-on-failure is error-prone and misses corner cases. Committing the DB only after all filesystem steps succeed (inside the same transaction) is the correct ordering.
- **When a gate exists (like `ServeOwnsCollectionError`), every code path that performs the gated action must enforce it.** Restore is a vault write; it must honor the same ownership fences as PUT. Adding `ensure_collection_not_live_owned()` as a reusable helper makes this explicit.

---

### 2026-04-25 Quarantine Restore Second Revision (5-Blocker Fix)

**Context:** Bender's first revision of the quarantine restore slice was itself rejected
on 5 consolidated blockers. Mom assigned as second revision author.

**What happened:**
- Fixed all 5 blockers in narrow, targeted edits:
  1. **Pre-install tempfile residue:** `write_all`/`sync_all` failures now clean up the
     tempfile before returning. If cleanup also fails, the cleanup error takes precedence.
  2. **tasks.md contradiction:** Task 9.8 body said "restore remains deferred" while the
     closure note said it was re-enabled. Rewrote body + note to be non-contradictory and
     accurate, attributing the note to Mom with a description of the current contract.
  3. **Parse-failure orphan:** `parse_restored_page` failure after `linkat` now rolls back
     the installed target via `rollback_target_entry` before returning.
  4. **Absent-parent fsync gap:** Switched restore from `walk_to_parent_create_dirs`
