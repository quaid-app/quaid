# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

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
     to `walk_to_parent` (no-create). Absent parents are now refused with a clear error
     rather than silently recreated without a durable fsync chain.
  5. **Narrow contract preserved:** No watcher, no audit, no overwrite policy widening.

- Added 3 focused tests: tempfile cleanup, parse-failure rollback, absent-parent refusal.
- 591 lib tests pass. 2 pre-existing Windows-only failures confirmed unrelated.
- Decision record: `.squad/decisions/inbox/mom-restore-revision.md`
- Skill file updated: `.squad/skills/quarantine-noreplace-rollback/SKILL.md`

**Lessons:**
- Pre-install tempfile residue is a silent crash hazard: wrap every write/sync call that
  follows tempfile creation with cleanup-on-failure, not just the install step.
- Post-install work failure must roll back. Any `?` after `linkat` is a potential half-
  installed vault state. Make rollback the explicit default, not a case-by-case addition.
- When a directory-creation variant exists, prefer the no-create variant for restore paths.
  Missing parents are a signal, not a task. Surface them; don't quietly patch them over.
- Contradictory documentation is a blocker in its own right. A task body that says
  "deferred" and a note that says "included" cannot both be true. Pick one and say it.

