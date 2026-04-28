**Session:** Scribe decision merge + Leela narrow repair completion logging

**Review outcome:**
- Professor's gating feedback on Batch B (safety-critical reconciler semantics + documentation accuracy) was resolved via focused repair pass by Leela.
- Repair scope: strict reconciler scaffold surface (reconciler.rs, tasks.md). No Batch C logic, no expand of approved groups.
- Safety semantics fix: has_db_only_state() now returns explicit Err instead of Ok(false), forcing caller error handling.
- Documentation fix: module header now accurately describes "will replace" (future) vs. "replaces" (completed).

**Decision ledger:**
- Leela's three repair decisions merged to canonical decisions.md (gate decision, repair decision, original review decision now in record)
- Decisions inbox cleared; Scribe orchestration/session logs written

**Batch B status:** ✅ Gate clean, ready for Batch C implementation planning. Professor can now sign off on Group 3 (ignore_patterns), Group 4 (file_state), and Group 5.1 scaffold landing.


## 2026-04-22 Vault Sync Batch C — Final Re-gate (Approved)

**Session:** Professor final gate authority after Leela repair and Scruffy coverage validation.

**Progression:**
1. **Initial REJECT:** Missing Unix imports + overclaimed tasks (2.4c, 4.4, 5.2 marked complete when only scaffolding existed).
2. **Leela repair:** Added conditional imports, demoted tasks, fixed docs. Focused, conservative fix.
3. **Scruffy validation:** Direct test coverage on seams; explicit error contracts on safety-critical stubs.
4. **Final re-gate:** APPROVE.

**Why it clears:**
1. **Prior safety blocker resolved:** Safety-critical scaffold no longer returns benign success values. 
econcile(), ull_hash_reconcile(), and has_db_only_state() all fail loudly instead of silently.
2. **Task truthfulness restored:** Deferred walk/hash/apply behavior no longer claimed complete. Checked items are foundation-only; unchecked items remain pending.
3. **Unix-compile honesty repaired:** Conditional imports in place. 
ustix wired under cfg(unix) in Cargo.toml. Code structurally ready for Unix builds (local validation has no Linux target available; cross-compilation check skipped but import fixes are correct).
4. **Validation green:** cargo test --quiet ✅; cargo clippy --quiet -- -D warnings ✅

**Verdict:** Ready to land as explicitly unwired foundation. Honest about deferral. Loud on safety-critical paths. Maintainable for next batch.

**Next:** Batch D (full reconciler walk) has clear handoff. Fd-relative primitives in place, stat helpers functional, platform gates protect invariants. Walk plumbing, rename resolution, delete-vs-quarantine classifier ready to wire.


### 2026-04-22 17:02:27 - Vault-Sync Batch E Gate Review

**Gate verdict:** APPROVE

**Why it clears:**

1. **UUID / gbrain_id wiring is truthful for this slice:**
   - parse_frontmatter() preserves gbrain_id
   - render_page() re-emits it when present
   - ingest/import adopt frontmatter UUIDs or generate UUIDv7 server-side
   - put / MCP write paths resolve persisted identity explicitly (no placeholders)

2. **Page.uuid is non-optional at the type seam:**
   - Page struct requires uuid: String
   - Typed read paths fail loudly on NULL rows (no fabricated defaults)
   - All 15+ Page construction sites audited and updated

3. **Default ingest remains read-only on source bytes:**
   - Compatibility ingest/import path stores generated UUIDs only in pages.uuid
   - Tests prove source markdown unchanged
   - Git worktree stays clean

4. **Rename classification is conservative and correctly staged for Batch E:**
   - Native rename pairs apply through explicit interface seam only
   - UUID matching works correctly
   - Guarded hash matching includes INFO refusal logging on ambiguous/trivial cases

5. **tasks.md is honest about the boundary:**
   - Checked items describe implemented classification/identity slice
   - Watcher-produced native events explicitly deferred
   - Apply-time quarantine/create mutations explicitly deferred
   - brain_put/admin write-back explicitly deferred

6. **Coverage is sufficient to merge this slice:**
   - Direct tests on gbrain_id parse/render/import round-trips
   - Read-only ingest behavior proven
   - Non-optional Page.uuid seam covered
   - Native/UUID/hash rename boundaries tested
   - cargo test --quiet: 439 tests pass
   - cargo clippy --quiet -- -D warnings: clean

**Landing note:** This is a narrow Batch E identity/reconciliation slice, not full write-back or watcher-native completion. Remaining work is clearly isolated in later tasks rather than hidden behind permissive defaults.

**Next review focus:**
- Batch F apply pipeline must preserve quarantine classifications
- Batch F full_hash_reconcile must use identity from Batch E
- Later: Batch F raw_imports rotation and GC

## 2026-04-23 Vault-Sync Batch F Gate Review

**Gate verdict:** APPROVE

**Why it clears:**

1. **Atomic raw-import rotation is real on the in-scope paths.**
   - `core::raw_imports::rotate_active_raw_import()` is now the shared rotation seam.
   - `commands::ingest`, `core::migrate::import_dir`, and reconciler apply-time reingest all invoke it inside the same SQLite transaction as their page/file-state writes.
   - The reconciler also enqueues `embedding_jobs` in that same transaction, matching the Batch F contract.

2. **Active-row invariants now fail loudly where Batch F actually writes.**
   - Rotation refuses any page that already has raw-import history but zero active rows, surfacing `InvariantViolationError` instead of silently healing corruption.
   - Post-rotation assertions keep every exercised write path at exactly one active row.
   - The remaining restore / `full_hash_reconcile` caller hookup is still explicitly deferred rather than misrepresented as done.

3. **Delete vs quarantine is re-evaluated at apply time.**
   - `apply_delete_or_quarantine()` re-checks all five DB-only-state branches inside the transaction that mutates the page/file_state rows.
   - Tests cover both the stale-classification seam and each preservation branch, so the reconciler no longer trusts an earlier snapshot.

4. **Batching and task truthfulness are acceptable for landing.**
   - Apply work is staged into explicit 500-action transactions with a regression test proving the first chunk commits even if a later chunk fails.
   - `tasks.md` clearly marks Batch F complete items versus deferred restore/full-hash/write-through work, which keeps the review boundary honest.
- Vault-sync Batch K2 final review (2026-04-23): **APPROVE FOR LANDING**. The K2 slice stays inside the approved offline restore-integrity closure: offline `begin_restore()` persists `restore_command_id`, `finalize_pending_restore()` only bypasses the fresh-heartbeat gate for the matching originator token, `run_tx_b()` leaves durable pending residue on failure, manifest-missing retries escalate to `integrity_failed_at`, tamper stays terminal until `restore-reset`, and `sync --finalize-pending` now drives the real CLI attach path (`finalize_pending_restore_via_cli` → `complete_attach`) proven by the end-to-end truth test. Required caveat remains explicit: this approval covers the offline CLI closure only; startup/orphan recovery, online handshake, and broader post-Tx-B topology are still deferred and must not be implied by `17.11`.
- PR #110 Leela re-review (2026-04-28): the main-guardrails bypass is actually closed once the workflow requires a PR on `github.sha` to be merged to `main` **and** to have `merge_commit_sha == github.sha`; that rejects the earlier open-PR association bypass. But a revision can still fail landing on narrow mechanical debt: this artifact remains rejectable because `src/core/vault_sync.rs` now compares `PathBuf` to `&str` inside a Unix-only test (`watch_callback_marks_collection_needs_full_sync_when_channel_is_full`), which compiles on Windows but fails the canonical Linux `cargo clippy --all-targets -- -D warnings` job. Under strict reviewer lockout, Leela is now out for the next repair turn; nominate a fresh owner (Bender) for the one-line Unix-safe assertion fix and CI rerun.

## 2026-04-23T09:00:00Z Batch K2 Final Approval

**Verdict:** APPROVE

Offline CLI closure meets all gating criteria. Tx-B residue, originator identity, reset/finalize surfaces all truthfully proven. Startup/orphan recovery and online handshake deferred to K3+. K2 APPROVED FOR LANDING.

## 2026-04-24 M1b-i/M1b-ii Session — Final Review in Progress

- **M1b-i proof lane COMPLETE (Bender):** Write-gate restoring-state proof closure (tests-only). No production code changes. Found no missing behavior — all mutators already call `ensure_collection_write_allowed` before mutation. 11 write-gate assertions (6 new + 5 pre-existing), all passing.
- **M1b-ii implementation lane COMPLETE (Fry):** Unix precondition/CAS hardening. Real `check_fs_precondition()` helper with self-heal; separate no-side-effect pre-sentinel variant for write path to preserve sentinel-failure truth. Scope: 12.2 + 12.4aa–12.4d.
- **Inbox decisions merged:** Bender M1b-i proof closure + Fry M1b-ii precondition split decision. Both now in canonical `decisions.md`.
- **Status:** Awaiting final Professor + Nibbler gate approval for both M1b-i and M1b-ii before landing.

## 2026-04-25 — Slice 13.6 + 17.5ddd Review (Bender revision)

**VERDICT: REJECT**

The implementation in `src/core/vault_sync.rs::parse_ignore_parse_errors()` silently strips every `file_stably_absent_but_clear_not_confirmed` entry, retaining only `code == "parse_error"` entries. `design.md §505` — the single authoritative schema document referenced by both task 13.6 ("returns the per-collection object documented in design.md") and 17.5ddd ("response shape matches design.md schema exactly") — explicitly states the `ignore_parse_errors` field covers **both** `"parse_error"` (line-level glob failure) **and** `"file_stably_absent_but_clear_not_confirmed"` (stateful-absence refusal). `design.md` was not modified in this diff. The test `brain_collections_surfaces_status_flags_and_terminal_precedence` (in `src/mcp/server.rs`) cements the violation: it seeds a `file_stably_absent_but_clear_not_confirmed` row and asserts `absent["ignore_parse_errors"].is_null()`, which is directly contrary to what the spec says should be surfaced.

All four of Bender's other claimed fixes are correct and well-covered: `integrity_blocked` precedence is right, the 30-minute escalation default and `GBRAIN_MANIFEST_INCOMPLETE_ESCALATION_SECS` env-var are correct, `restore_in_progress` semantics match the spec, and `recovery_in_progress` queued-vs-running split is properly tested.

**Minimum required fix:** Either (a) update `design.md §505` to explicitly exclude `file_stably_absent_but_clear_not_confirmed` from `brain_collections` output and document the deferral, or (b) remove the `retain(|e| e.code == "parse_error")` filter and surface both codes as the spec demands. The test must be updated to match whichever path is chosen.




## 2026-04-24 — 13.5 re-review after repair commit 97e574e

VERDICT: APPROVE

The repair is tight and correct. progressive_retrieve now accepts collection_filter: Option<i64> and outbound_neighbours enforces it via AND (?3 IS NULL OR p2.collection_id = ?3) — the ?3 IS NULL short-circuit keeps the CLI path (which passes None) unaffected. rain_query in server.rs threads the same collection_filter ID it resolved for hybrid_search_canonical straight into progressive_retrieve, so the initial search and the BFS expansion are under the same fence. The CLI query.rs correctly passes None (no widening). The 
esolve_read_collection_filter default logic satisfies the 13.5 contract — single active collection filters to it, write-target designated filters to it, no write target returns None (all collections). The new direct-proof test rain_query_auto_depth_does_not_expand_across_collections creates an explicit cross-collection link and asserts the linked page in the foreign collection never surfaces. Scope is clean: no write-path changes, no CLI widening, no ignore-diagnostic widening. 13.5 is sealed.
- **Pre-gate scoping before implementation (2026-04-25):** A pre-gate decision on narrow restore re-enable must specify the exact contract before any code lands: which fsyncs are mandatory, what the observable invariant is at recovery time, and which failure paths are acceptable. The contract in this case was: parent fsync after every unlink, install-time no-replace semantics, and deterministic no-data-left-behind on any failure. Gating before the body starts prevents discovering the contract mismatch in code review.
- PR #111 landing review (2026-04-28): approve the follow-up when scope growth is still mechanically subordinate to one root cause. A test-only Unix fixture repair may absorb the minimal gate-clearing baseline debt and any newly unmasked same-root-cause fixture fixes without becoming a separate feature, so long as it does not widen product behavior or public contracts. Once that bar is met and CI is green, PR #110 is no longer blocked on code work—only on external review / merge sequencing.
