- **Scope:** 17.5s2–17.5s5 — write-interlock proof batch (tests/evidence only, no production changes).
- **Finding:** No missing behavior. All five MCP mutators (`brain_put`, `brain_link`, `brain_check`, `brain_raw`, `brain_gap` slug-bound) already call `ensure_collection_write_allowed` before any mutation. The gate is consistently wired under task 11.8.
- **New tests (6):** `brain_put_refuses_when_collection_is_restoring`, `brain_link_refuses_when_collection_is_restoring`, `brain_link_refuses_when_collection_needs_full_sync`, `brain_check_refuses_when_collection_is_restoring`, `brain_check_refuses_when_collection_needs_full_sync`, `brain_raw_refuses_when_collection_is_restoring`. All 6 pass.
- **Pre-existing tests that already covered remaining matrix cells (5):** `brain_put`/`brain_raw` + `needs_full_sync`, `brain_gap` slug ×2, `brain_gap` slug-less.
- **17.5s2 mutator matrix:** 5 mutators × 2 conditions = 10 cells, all proved. `brain_gap` slug-less correctly excluded (Read carve-out).
- **Tasks closed:** 17.5s2 ✅, 17.5s3 ✅, 17.5s4 ✅, 17.5s5 ✅
- **Lesson:** When auditing write-gate coverage, build the explicit (mutator × condition) matrix first. Ad-hoc sampling by condition or by op alone will miss cells.

## 2026-04-24 M1b-ii/M1b-i Session Completion

- **M1b-i proof lane COMPLETE:** Write-gate restoring-state proof closure (tests-only). Found no missing behavior. All mutators already call `ensure_collection_write_allowed` before mutation.
- **M1b-ii implementation lane COMPLETE (Fry):** Unix precondition/CAS hardening. Real `check_fs_precondition()` helper with self-heal; separate no-side-effect pre-sentinel variant for write path.
- **Inbox decisions merged:** Bender M1b-i write-gate closure + Fry M1b-ii precondition split decision. Both entries now in canonical `decisions.md`.
- **Orchestration logs written:** `2026-04-24T12-53-00Z-bender-m1b-i-proof-lane.md` + `2026-04-24T12-54-00Z-fry-m1b-ii-implementation-lane.md`.
- **Session log written:** `2026-04-24T12-55-00Z-m1b-session.md`.
- **Status:** Awaiting final Professor + Nibbler gate approval for both M1b-i and M1b-ii.

## 2026-04-25 Docs Validation — vault-sync-engine refresh pass

- **Scope:** Validated all Amy (prose docs), Hermes (website), and Zapp (promo/website) doc changes from the 2026-04-25 vault-sync-engine post-batch refresh.
- **Site build:** 15 pages, zero errors. ✅
- **Finding (FIXED):** `docs/roadmap.md` Phase 1 and Phase 2 release lines still said "tag pending" for v0.1.0 and v0.2.0. Both tags are live. Fixed in commit `9f56a16`. Amy added the vault-sync section but missed cleaning up the stale Phase 1/2 release lines. Zapp's D4 decision only targeted the website surface.
- **All other surfaces approved:** tool counts (16 released / 17 branch), schema version (v5), channel defaults (airgapped), vault-sync branch qualifiers, deferred items tables, install.mdx version pins (v0.9.4), homepage accuracy (no fake HTTP output).
- **Decision written:** `.squad/decisions/inbox/bender-docs-validation.md`

## Learnings

- **When `docs/roadmap.md` and `website/contributing/roadmap.md` are updated in separate passes, both must be checked for the same stale language.** Zapp's D4 only fixed the website version. Amy updated docs/roadmap.md for vault-sync content but missed the pre-existing stale Phase 1/2 release lines. Rule: any docs-refresh checklist must diff both roadmap files together.

## 2025-07-22 Vault-Sync-Engine Batch 1 Coverage Audit

- **Scope:** Audited test coverage for vault-sync-engine Batch 1 implementation against v0.10.0 ship decision.
- **Baseline:** Linux CI canonical = **82.53%** (`cargo llvm-cov` full run on ubuntu-latest). Windows lib-only = 80.20%.
- **Platform gap:** ~2.33% explained by (1) unix-gated code not compiling on Windows, (2) integration tests excluded from `--lib` mode. Windows `--lib` is not canonical; CI Linux full-run is ground truth.
- **90% verdict:** **NOT ACHIEVABLE** in this lane. Reaching 90% requires ~1,768 more covered lines. Primary blockers: `core/vault_sync.rs` watcher pipeline, `core/reconciler.rs` ingest loop, `core/fs_safety.rs`, `commands/collection.rs` vault lifecycle — all substantially `#[cfg(unix)]`-gated or requiring mounted filesystem.
- **Quick wins landed:** 25 new unit tests in `commands/call.rs` (9 dispatch routes), `commands/timeline.rs` (7 `run()` + `add()` paths), `commands/gaps.rs` (4 `run()` paths), `commands/version.rs` (1 smoke). All pass. Estimated Linux CI delta: ~0.5–1%.
- **Recommendation:** Ship v0.10.0 at 82.53% (post-PR: ~83–83.5%). Assign remaining gap to Scruffy in a dedicated test sprint.
- **Decision written:** `.squad/decisions/inbox/bender-batch1-coverage-audit.md`

## Learnings

- **`cargo llvm-cov --lib` vs `cargo llvm-cov` (no flag):** `--lib` = unit tests only; no flag = all tests including integration. The CI canonical run uses no flag, which is 2–3% higher than `--lib` on Windows. Always compare same mode when quoting coverage numbers.
- **Unix-gated code is a coverage ceiling, not a test gap.** `#[cfg(unix)]` blocks are unreachable on Windows and won't appear in any coverage report regardless of test quality. When evaluating a coverage target, subtract estimated unix-gated missed lines before setting expectations.
- **`get_page_by_key` requires non-NULL `uuid` in the pages row.** Tests that call command functions going through `get_page_by_key` must insert pages with an explicit UUID (`uuid::Uuid::now_v7().to_string()`). The schema allows NULL by design for legacy inserts, but the query function rejects it. `add()` (timeline) does NOT call `get_page_by_key` and works with NULL uuid; `run()` (timeline) DOES and fails without it.
- **For single-collection DBs, `OpKind::Read` via `parse_slug` resolves to that collection regardless of whether the page exists.** No need to worry about collection state for read-only paths; detached collections resolve fine for reads.
- **"tag pending" is the most reliably stale string in docs.** Cross-check against `git tag -l` output on every docs validation pass that touches roadmap files.

## Learnings

- **Exact-slug shortcuts must fail closed before generic search fallback.**If a hybrid-query path recognizes a bare slug or `[[slug]]`, ambiguity is a routing failure, not a "no results" case. Returning `None` from the exact-slug fast path silently lies about the seam and hides duplicate-slug defects.
- **For CLI parity claims, prove every slug-bearing entry point directly.** One `get` ambiguity test is not evidence for `graph`, `timeline`, `check`, `link`, `links`, `backlinks`, `unlink`, or exact-slug `query`. Build the command matrix first, then add one direct refusal assertion per command family so the task text stays truthful.
- **For frozen MCP diagnostic schemas, test the full predicate, not just the label column.** A terminal discriminator like `integrity_blocked` must prove its timestamp/age gate, precedence, and negative cases (reason present without terminal state, queued recovery, pre-window restore) or reviewers will correctly reject it as overclaimed.
- **When a restore seam still fails crash-durability and no-replace safety, back the surface out instead of inventing a bigger repair.** A deferred command with explicit task reopen is a better batch than pretending a risky restore is "close enough."

## 2026-04-25 Quarantine Restore Re-Enable Validation

- **Verdict:** APPROVED (conditional on Linux CI green for 5 `#[cfg(unix)]` tests).
- **Scope:** Narrow quarantine-restore re-enable slice — `linkat` no-replace install, crash-durable rollback, env-gated test hooks, and full gate chain.
- **Gate chain confirmed fail-closed:**
  1. Double-gated on Windows: `ensure_unix_collection_command` (CLI) + `#[cfg(not(unix))]` (library). Both independently return `UnsupportedPlatformError`.
  2. `ensure_collection_vault_write_allowed`: checks `state=Restoring`, `needs_full_sync`, and `writable=false` before any FS mutation.
  3. `start_short_lived_owner_lease` → `acquire_owner_lease`: refuses live foreign serve-owner with `ServeOwnsCollectionError`.
  4. Pre-check: `stat_at_nofollow` fd-relative, no-follow. Fires before tempfile creation.
  5. Install: `linkat_parent_fd` — hard-link, not rename. Cannot silently overwrite a competing target.
  6. Rollback: every unlink followed by parent `fsync`. Trace test verifies exact event sequence: `unlink:temp → fsync-after-unlink:temp → unlink:target → fsync-after-unlink:target`.
  7. DB tx only commits after FS install succeeds; on DB failure, `rollback_target_entry` fires.
  8. All three test hooks (pause, fail-after-install, trace-file) are env-gated no-ops in production.
- **Tests run:** 1 platform-applicable test in `quarantine_revision_fixes.rs` ✅; 5 `#[cfg(unix)]` tests skipped (Windows); 25/25 `collection_cli_truth.rs` pass (including Windows fail-closed check ✅); 591/591 full suite pass (2 pre-existing unrelated failures).
- **Minor observation (non-blocking):** `ensure_collection_vault_write_allowed` loads the collection twice — once directly, once through `check_writable`. No logic error, just a redundant DB read.
- **Linux CI required:** The 5 Unix-specific tests in `quarantine_revision_fixes.rs` (non-Markdown target, live-owned, read-only, post-precheck race, rollback trace) must be confirmed green on Linux before full closure.
- **Decision written:** `.squad/decisions/inbox/bender-restore-validation.md`

## Learnings

- **Double-gating (CLI dispatch + library `#[cfg(not(unix))]`) is stronger than either gate alone.** When validating platform exclusions, always check that both layers are in place. A platform regression in one leaves the other standing.
- **Trace-file hooks prove rollback ordering without mocking the filesystem.** The `unlink:X → fsync-after-unlink:X` pattern is a reusable proof seam for any cleanup sequence that must guarantee fsync before returning. See `.squad/skills/quarantine-noreplace-rollback/SKILL.md`.
- **When validating on Windows, enumerate which `#[cfg(unix)]` tests are being skipped and flag them explicitly.** "1 passed" looks weak but is correct if the other tests are platform-gated. Always note the skip count and where CI must close the gap.

## 2026-04-25 v0.9.7 Release Validation — Issues #79/#80

- **Scope:** Validate `release/v0.9.7` branch fixing macOS build failure (#80) and installer 404 (#79). Run seam tests, confirm CI, merge, tag, verify 17-asset release.
- **Root cause confirmed:** `stat.st_mode` is `u16` on macOS/Darwin, `u32` on Linux. `FileStatNoFollow.mode_bits: u32` caused type-mismatch compile errors on all 4 macOS CI jobs in v0.9.6. No macOS binaries uploaded → install.sh returned HTTP 404 for all darwin targets.
- **Fix:** `stat.st_mode as u32` at `src/core/fs_safety.rs:199` (lossless widening cast). Already committed on `release/v0.9.7` before this session.
- **D-R79-2 implementation:** Centralized release asset manifest to `.github/release-assets.txt` (17 lines, canonical single source of truth). `release.yml`, `RELEASE_CHECKLIST.md`, `release_asset_parity.sh`, and `install_release_seam.sh` all validate against it.
- **Seam tests:** `release_asset_parity.sh` 22/22 PASS (static analysis, any platform). `install_release_seam.sh` is CI-only (requires real Unix exec semantics for uname stubs).
- **CI blocker discovered and fixed:** All 4 `release-macos-preflight` jobs failed at "Cache cargo registry" with error `Key Validation Error: ... cannot contain commas`. The cache key used `matrix.features` (`bundled,embedded-model`); `actions/cache@v4` rejects commas. Fixed by adding `channel` field (airgapped/online) to each matrix entry and using `matrix.channel` in the key.
- **CI green confirmed:** Run `24922724381` at `2b9221c` — all 8 jobs passed including all 4 macOS preflight "Cargo check release target" steps. `stat.st_mode as u32` fix proven on aarch64+x86_64 × airgapped+online.
- **PR #83 merged** to main (admin merge required — branch protection policy).
- **Release verified:** Tag `v0.9.7` at `72b5ed0` (macro88). Release workflow `24922783295` succeeded. All 17 assets present on GitHub Release including `gbrain-darwin-x86_64-airgapped` and `gbrain-darwin-arm64-airgapped` (the previously missing assets).
- **Issues #79 and #80 closed** (both already closed when verified).

## Learnings

- **`actions/cache@v4` rejects commas in cache keys.** When a matrix variable (like `features`) contains comma-separated values (`bundled,embedded-model`), it must NOT be used directly in the `key:` field. Extract a separate comma-free matrix field (e.g., `channel: airgapped`) for the cache key. The comma restriction is not documented prominently — it manifests as an instant job failure at the cache setup step, causing all downstream steps to be skipped.
- **Cache key failures cause downstream steps to be SKIPPED, not FAILED.** When diagnosing CI failures where "Cargo check release target" shows as skipped, look at the step that ran before it — often a cache setup failure. A skipped build step does NOT mean the build passed; it means the step never ran.
- **A CI infrastructure failure (cache key error) can mask a code fix being unproven.** All 4 macOS preflight jobs "failed" but the actual cargo check never ran. Declaring the code fix valid on that basis would have been wrong. Always confirm the build step itself ran and succeeded, not just that the job infrastructure passed.
- **When a job fails in CI infrastructure (cache, checkout, env setup), the fix is in the workflow YAML, not in the source code.** Before concluding a compile error persists, audit which step the job actually failed at and whether the compile step ran at all.

## 2025-07-22 reconciler.rs + fs_safety.rs Coverage Sprint

- **Scope:** Drive non-unix-gated coverage gaps in `src/core/reconciler.rs` and `src/core/fs_safety.rs`.
- **Baseline:** Windows lib-only ~80.85% (Linux CI canonical: ~82.53%).
- **New tests added:** +42 in `reconciler.rs`, +10 unix-gated + 3 non-unix in `fs_safety.rs`. Total: 678 lib tests pass (up from 636).
- **Coverage of:** `ReconcileError::Display` (4 variants), `FullHashReconcileMode::as_str` (7 variants), `RestoreRemapOperation::as_str` (2 variants), `DriftCaptureSummary` (has_material_changes false + from_stats + add_assign), `CollectionDirtyStatus::is_dirty` (sentinel-only), `run_phase2_stability_check` (max-iters exhaustion for Restore and Remap), `infer_type_from_path` (all 9 folder variants + numeric prefix + unknown), `strip_numeric_prefix` (with/without prefix), `is_markdown_file` (true/false/case), `raw_import_invariant_result` (multi-active-row enforce + allow-override), `uuid_migration_preflight` (3 OK paths), `load_frontmatter_map` (invalid JSON), `default_restore_stability_max_iters` (env var + zero fallback), `sentinel_count` (non-sentinel + missing dir), `authorize_full_hash_reconcile` (RemapRoot, OverflowRecovery, Restore modes), `reconcile_with_native_events` (non-Active early return).
- **fs_safety additions:** `FileStatNoFollow::is_regular_file/is_directory/is_symlink` via real stat (unix), direct mode_bits construction (both platforms), `walk_to_parent` single-component path, `linkat_parent_fd` success case, Windows fallback stubs all return `io::ErrorKind::Unsupported`.
- **90% gap:** Not bridged. Unix-gated code is a coverage ceiling, not a test gap. Estimated Linux CI delta: +1–2%, pushing from 82.53% toward ~84%.
- **Decision written:** `.squad/decisions/inbox/bender-reconciler-coverage.md`

## Learnings

- **On Windows, `#[cfg(unix)]` tests compile out.** All new coverage tests in this sprint must be platform-agnostic. Tests calling unix syscalls (`stat_at_nofollow`, `open_root_fd`) must remain inside the existing `#[cfg(all(test, unix))]` block and simply don't run locally.
- **The Windows stub for `walk_to_parent<Fd>` takes a fully generic, unconstrained Fd parameter** — pass `0u32` in non-unix tests to avoid any real file I/O. Attempting `std::fs::File::open(".")` will panic if the current directory can't be opened as a `File` on Windows.
- **`FileStatNoFollow` struct and methods are NOT unix-gated.** They can be tested on any platform by directly constructing `FileStatNoFollow { mode_bits: 0o100644, .. }`. This gives cheap, deterministic coverage of the three mode-bit predicates without any filesystem dependency.
- **`run_phase2_stability_check` is fully testable with fake closures.** The function takes `check_fn: impl FnMut() -> bool` — pass `|| false` to exercise the max-iters exhaustion branch without any real vault.
- **For env-var-controlled constants, test with `std::env::set_var` + restore pattern.** `default_restore_stability_max_iters()` reads `QUAID_RESTORE_STABILITY_MAX_ITERS`; setting it to "0" forces the zero-fallback branch. Always restore the var (or use `remove_var`) after the test to avoid polluting parallel tests.


