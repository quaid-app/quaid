
**Role:** P3 Release gate review (task 5.2 coverage inspectability)

**What happened:**
- Scruffy's initial review (task 5.2) verified coverage outputs are free and GitHub-visible (lcov.info artifact + job summary), but identified two blocking issues: coverage surface not documented in public docs, README/docs-site status messaging still drifts.
- Marked task 5.2 blocked with specific doc revision requirements. Amy added coverage guidance to README/docs pages pointing to GitHub Actions surface and stating coverage is informational. Hermes synced docs-site roadmap/status with README.
- Re-reviewed after fixes. Both doc accuracy issues resolved. Task 5.2 **APPROVED**.

**Outcome:** P3 Release gate 5.2 (inspectability) **COMPLETE & APPROVED**. Coverage surface documented and GitHub-visible, status messaging aligned across all surfaces, sign-off complete.

**Decision notes:** `.squad/decisions.md` (merged from inbox) — documents Scruffy's task 5.2 review, blocking issues, and re-review approval.

## 2026-04-15 Cross-team Update

- **Professor completed graph parent-aware tree rendering** (commit `44ad720`). Multi-hop depth-2 edges now render beneath actual parent instead of flattening under root. Depth-2 integration test strengthened with exact text shape assertions. All validation gates pass.
- **Fry advancing slices:** Progressive retrieval (tasks 5.1–5.6) and assertions/check (tasks 3.1–4.5) both implemented. All 193 tests pass (up from 185). Decisions merged into canonical ledger. Awaiting Nibbler's final graph re-review and completion.

## 2026-04-16T14:59:20Z Simplified-install v0.9.0 Release — Scruffy Completion

- **Task:** Validated installer and package paths, normalized line endings, updated task documentation, added validation skill
- **Changes:**
  1. Installer path validation — confirmed `simplified-install/` paths and script locations
  2. Package paths normalization — verified `scripts/install.sh` and Windows/Unix consistency
  3. Line endings normalization — updated `scripts/install.sh` to consistent CRLF/LF handling
  4. Task documentation — updated `simplified-install/tasks.md` with validation guidance
  5. Validation skill — created/appended skill documentation for install validation
- **Status:** ✅ COMPLETE. Installer paths validated and documented. scripts/install.sh ready for v0.9.0 release.
- **Orchestration log:** `.squad/orchestration-log/2026-04-16T14-59-20Z-scruffy.md`

## 2026-04-22: Vault-Sync Batch B Coverage Seams Locked

**What:** Completed targeted coverage work on vault-sync Batch B seams before full reconciler lands. Locked parse_slug routing matrix, .gbrainignore error-shape contracts, and file_state drift/upsert behavior.

**Decisions:**

### Early Seam Coverage Prevents Silent Refactor Failures
Helper-level tests as integration scaffold. Tests serve double duty: immediate validation of parse/ignore/stat helpers AND early warning system for integration hazards.

### Coverage Delivered
- parse_slug() routing matrix: all branching cases covered
- .gbrainignore error-shape contracts: all error codes and line-level reporting fidelity proved
- file_state stat-diff behavior: ctime/inode-only and mtime/size changes both trigger re-hash

**Validation:** 10 new direct unit tests for coverage seams. All tests pass. Error paths tested and will fail loudly if later changes break contracts.

**Why:** These are touched-surface seams with branchy behavior that future reconciler/watcher work will reuse directly. Guarding them now keeps Batch B credible even before the larger integration paths exist.

**Status:** ✅ COMPLETE. Ready for full reconciler implementation.

## 2026-04-22 Vault Sync Batch C — Re-gate (Approved)

**Session:** Scruffy coverage validation after Leela's targeted repair pass.

**What happened:**
- Scruffy re-reviewed the repaired Batch C to validate that foundation seams are locked with direct tests, safety-critical stubs explicitly error, and task claims are truthful.
- Focused on three seams: ile_state::stat_file_fd() (wrapper layer), 
econciler stubs (error contracts), 	asks.md (truthfulness).

**Key findings:**
1. **Direct seam coverage locked:** stat_file_fd() directly tested for nofollow preservation and full Unix stat field population. ull_hash_reconcile() and has_db_only_state() directly tested for explicit Err return. stat_diff() foundation behavior (DB rows as "missing") pinned by direct assertion.
2. **Safety-critical stubs explicitly error:** No more silent success defaults. 
econcile(), ull_hash_reconcile(), has_db_only_state() all required to return Err("not yet implemented") rather than Ok(empty).
3. **Task surface truthful:** Unchecked items remain pending; checked items annotated as foundation/scaffold. Deferred walk/hash/apply behavior not claimed complete.

**Validation rerun:**
- cargo test --quiet ✅
- GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model ✅

**Outcome:** APPROVE. Coverage sufficient on touched surface. Safety-critical stub behavior asserted directly. Ready to land.


### 2026-04-22 17:02:27 - Vault-Sync Batch E Coverage Lane

**Session:** Lock honest Batch E test coverage on real seams

**Coverage strategy:**

Do not add tests that would accidentally bless incomplete implementation or imply finished behavior. Focus on gbrain_id round-trip fidelity and ingest safety.

**Tests added (and locked):**

1. **gbrain_id frontmatter round-trip:**
   - parse_frontmatter() preserves gbrain_id
   - render_page() re-emits when present
   - import/export round-trip fidelity
   - serde serialization preserves the field

2. **Ingest non-rewrite behavior:**
   - Default ingest does not modify source markdown
   - Generated UUIDs stored in DB only, not in file
   - Git worktree stays clean after import

3. **Explicit delete-vs-quarantine outcomes:**
   - Quarantine classification on ambiguous/trivial cases
   - Delete predicate respects source_kind boundaries

**Tests explicitly NOT added:**

- Rename inference (native events, UUID matching, hash pairing) — these are deferred to Batch F apply pipeline
- Frontmatter write-back (brain_put UUID preservation) — deferred to later batch
- Watcher-produced rename events — Group 6 deferred entirely

**Why this matters:**

- gbrain_id is already a data-fidelity guard even before pages.uuid becomes fully non-optional
- Honest coverage prevents accidental false confidence in incomplete rename logic
- Round-trip tests survive rename implementation without false-positive regressions

**Validation:**

- cargo test --quiet: all 439 tests pass
- cargo clippy --quiet -- -D warnings: clean
- Default model validation: green
- Online-model validation: green

**Next coverage focus:**
- Batch F: direct tests for rename inference outcomes (UUID → page_id preservation, hash ambiguity → quarantine)
- Later: watcher-native event seam once Group 6 lands
- Batch 1 watcher-reliability coverage map (2026-04-27): the `.quaidignore` lane is now split cleanly: parser-level semantics live in `src/core/ignore_patterns.rs`, while watcher delivery into reload/reconcile is covered directly in `src/core/vault_sync.rs`. The remaining Batch 1 proof debt is overflow recovery timing/gating, native→poll fallback, crash backoff/restart, and watcher-health surfacing. I landed low-conflict guard tests in `src/commands/collection.rs` for the restoring-vs-pending-attach-vs-active-reconcile CLI status split and in `src/core/vault_sync.rs` for `memory_collections.restore_in_progress` only flipping true after a real restore ack (`state='restoring'` + `restore_command_id` + `watcher_released_at`).

## 2026-04-24 Vault-Sync 13.5 Slice Review

Reviewed only the 13.5 MCP read-filter slice on `spec/vault-sync-engine`. `brain_search`, `brain_query`, and `brain_list` now accept optional `collection`, explicit names fail clearly when missing, defaulting follows sole-active-else-write-target, and the CLI/write paths remain unchanged apart from passing the new internal `None` filter parameter. Ran `cargo test --quiet mcp::server`; all scoped MCP server tests passed. Verdict: APPROVE.

## 2026-04-24 Vault-Sync 13.5 Slice Re-review

- Verdict: **APPROVE**
- The only real hole Nibbler found is now sealed: `brain_query` threads the effective MCP collection filter into `progressive_retrieve()`, `outbound_neighbours()` enforces that filter in SQL during `depth="auto"` expansion, and the new direct MCP regression test proves a filtered query cannot leak across collections through linked-page expansion. The rest of the 13.5 surface stays narrow and honest: `brain_search`, `brain_query`, and `brain_list` still share the same read-filter/default resolver, while CLI paths continue to pass `None` and no write-path or wider-scope claims were added.
- **Proof seam narrowness guards data-loss surfaces (2026-04-25):** Watcher/quarantine coverage audit landed two epoch-regression proofs and one lifecycle source-invariant proof without widening into restore. The hard-delete guard coverage (every delete path consults DB-only-state) is the highest-value remaining gap. The watcher overflow fallback path (TrySendError::Full branch) is secondary. Restore tests remain deferred until Fry lands no-replace install + post-unlink fsync durability proofs.
- Reconciler/fs-safety lane check at head `7804234` (2026-04-25): current head already contains coordinator commit `03d932e`, so the clustered Unix-lane expectation repairs are present in source with no further Scruffy test edits needed. `src/core/reconciler.rs` now matches the reconciler truth seam (`walked/new == 2` for the real file plus the non-symlink file under the real directory, modified-page compiled truth is asserted with `trim_end()`, the dirty-recheck test still pins fresh-connection TOCTOU ordering, and the 500-file chunk test truthfully expects zero committed pages because invalid `gbrain_id` is rejected during pre-apply tree scanning before any chunk transaction opens); `src/core/fs_safety.rs` also keeps the symlink-root error kind tolerant across Unix kernels (`InvalidInput | NotADirectory`).


- Batch 1 coverage audit complete (2026-04-27T23:51:40Z): Guard tests landed with low conflict. Honest >90% coverage cannot be claimed due to pending implementation-coupled proof for tasks 6.7a, 6.9, 6.10, 6.11. Coverage audit recorded in .squad/orchestration-log/2026-04-27T23-51-40Z-scruffy.md.
- Batch 1 coverage lift (2026-04-28): the honest Windows coverage command is `cargo llvm-cov --lib --tests --summary-only`, not the bare default invocation. After repairing CLI binary discovery for coverage runs (`tests/common/mod.rs`) and stale lib tests in `src/commands/call.rs` / `src/commands/timeline.rs`, the repo measures **84.51%** line coverage (`20,190 / 23,891`), still **1,312 covered lines short** of a 90% gate. The remaining gap is concentrated in `src/commands/collection.rs` (46.07%), `src/core/reconciler.rs` (59.71%), and `src/core/vault_sync.rs` (77.90%), so Batch 1 is not truthfully shippable from this lane without a dedicated backfill sprint across those three files.
- Coverage-safe CLI tests (2026-04-28): subprocess tests that invoke `quaid` should resolve the binary through `tests/common/mod.rs::quaid_bin()` rather than raw `env!("CARGO_BIN_EXE_quaid")`. `cargo llvm-cov` on this Windows lane leaves some integration suites with a missing direct env path even though sibling `target\llvm-cov-target\debug\deps\quaid.exe` exists; the fallback helper keeps `tests/collection_cli_truth.rs`, `tests/quarantine_revision_fixes.rs`, and `tests/search_hardening.rs` runnable under both plain `cargo test` and coverage runs.

## Learnings

- 2026-04-29T20:33:01.970+08:00 — Batch 3 UUID write-back coverage reconnaissance: the honest test split is `src/core/vault_sync.rs` for atomic rewrite + `file_state`/`raw_imports` rotation + live-owner seams, `src/commands/collection.rs` for `WriteAdmin` routing / dry-run / restoring-write-gate behavior, and `tests/collection_cli_truth.rs` for subprocess exit-code plus JSON/text operator guidance. `tests/command_surface_coverage.rs` is only the cheap dispatch backstop if the new `collection migrate-uuids` arm still misses under coverage. On Windows, iterate with targeted `cargo test` first, then measure with `cargo llvm-cov --lib --tests --summary-only --no-clean -j 1` and refresh exact misses via `cargo llvm-cov report --json --output-path target\llvm-cov-report.json`.
- 2026-04-29T20:33:01.970+08:00 — Batch 3 validation gate: the Windows coverage lane can clear a 90% **line** bar while still failing the honest task gate. In this branch, `memory_put`/frontmatter UUID preservation is exercised on Windows, but the UUID write-back/dry-run/read-only claims live behind `#[cfg(unix)]` tests, so `src\commands\collection.rs::migrate_uuids` and `src\core\vault_sync.rs::write_quaid_id_to_file` stay uncovered on the Windows lane and must not be counted as proven there.
## 2026-04-29T13:29:11Z — Batch 3 review close

- **Professor:** Rejected Batch 3 on incomplete task closure (`12.6b`/`17.5ii9`). Error text lacks "stop serve first" guidance. Tests incomplete.
- **Nibbler:** Rejected Batch 3 on safety: live-owner guard keyed to `collection_id` (not unique), bulk rewrite lacks offline lease, test coverage insufficient.
- **Mom:** Reassigned to fix both blocking findings. Fry locked out.
- **Scruffy:** Paused validation; coverage lane held pending implementation revisions.


## 2026-04-29T13:57:48Z — Memory Cycle: Batch 3 Validation Gate FAIL

- Scruffy validation: **REJECTED** (Windows lane 90.52% line, 89.03% region; UUID write-back proof Unix-only; compile blockers at vault_sync.rs:1917 & :3772)
- Mom: Revision cycle RUNNING; Fry locked out pending completion
- Decisions merged: 1 inbox entry
- Archive: 22 entries moved to decisions-archive.md (file was 438KB)

## 2026-04-29 Session: Batch 3 Multi-Agent Review

**Session ID:** 2026-04-29T13:29:11.071Z
**Agents:** Professor, Nibbler, Scruffy
**Outcome:** Professor APPROVE, Nibbler APPROVE, Scruffy FAIL/STARTED

### Session Summary

Batch 3 revision received multi-agent re-review for same-root alias race and offline-lease closure. Professor and Nibbler approved the fix. Scruffy's revalidation failed on Windows coverage gap; coverage fix work started. Task claims narrowed to match actual proof seams.

### Cross-Agent Dependencies

- Professor → Nibbler: Review results aligned (same-root alias closed + root-scoped lease works)
- Nibbler → Scruffy: Revalidation launched on Windows lane; coverage still insufficient
- Scruffy: Blocking on honest Windows coverage for 17.5ww/ww2/ww3 or narrowed claims

