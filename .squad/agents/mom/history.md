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

---

### 2026-04-25 Vault Sync CI Fix — 6 Failing Tests (spec/vault-sync-engine lane)

**Context:** 6 tests were failing in CI run 24892249558 at HEAD `7804234`. Off-limits files:
`src/commands/put.rs`, `src/core/reconciler.rs`, `src/core/fs_safety.rs`,
`tests/concurrency_stress.rs`, `src/mcp/server.rs`. Four distinct root causes required
surgical fixes in `src/core/vault_sync.rs` and `src/core/raw_imports.rs`.

**What happened:**

1. **Global registry state leak** (`run_rcrt_pass_*` family, `start_serve_runtime_*`):
   `run_rcrt_pass_clears_needs_full_sync_after_tx_b` registers `supervisor_handles[2] = "serve-1"`.
   Subsequent tests calling `run_rcrt_pass(&conn, "serve-1")` short-circuit at
   `has_supervisor_handle(collection_id, session_id)` with `"work:supervised"` — never
   executing the body. Fix: added `init_process_registries().unwrap()` to 4 test bodies
   (`run_rcrt_pass_preserves_pending_root_path_when_manifest_is_incomplete`,
   `run_rcrt_pass_skips_reconcile_halted_collections`,
   `start_serve_runtime_recovers_owned_sentinel_dirty_collection_and_unlinks_all_sentinels`,
   and the poison-source test itself).

2. **Frontmatter mismatch in uuid_migration_preflight** (`start_serve_runtime_recovers_*`,
   `writer_side_foreign_rename_*`, `restore_safety_pipeline_aborts_on_fresh_connection_*`):
   `insert_page_with_raw_import` hardcoded `frontmatter = '{}'`. Raw bytes contained YAML
   with `gbrain_id`. `uuid_migration_preflight` found non-matching uuid+frontmatter+trivial-body
   (`< MIN_CANONICAL_BODY_BYTES = 64`) and returned `UuidMigrationRequiredError`, blocking
   `complete_attach`. Fix: `insert_page_with_raw_import` now parses frontmatter from
   `raw_bytes` via `markdown::parse_frontmatter` and stores correct JSON in `pages.frontmatter`.

3. **`rotate_active_raw_import` doesn't sync frontmatter** (`restore_safety_pipeline_aborts_*`):
   The off-limits reconciler test calls `seed_page_with_identity` (empty frontmatter) then
   `rotate_active_raw_import` with YAML containing `gbrain_id`. But `rotate_active_raw_import`
   did not update `pages.frontmatter`. Fix: after inserting the raw_import row, parse the
   UTF-8 frontmatter and UPDATE pages.frontmatter to match.

4. **OCC conflict error format missing "Conflict:" prefix** (`brain_put_returns_occ_conflict_*`):
   Old format: `"ConflictError: ... reason=StaleExpectedVersion ... current_version=N"`.
   `server.rs` handler gates on `message.contains("Conflict:")` — old format has no colon.
   Another agent (put.rs change) expects `"current version: 2"` (space + colon).
   Fix: new format: `"Conflict: ConflictError StaleExpectedVersion ... current version: N"` —
   satisfies all four checks: "Conflict:", "Conflict", "ConflictError", "StaleExpectedVersion",
   "current version: 2".

**Outcome:** 591 tests pass. 2 pre-existing Windows-only failures confirmed unrelated
(`init_rejects_nonexistent_parent_directory`, `open_rejects_nonexistent_parent_dir` —
SQLite on Windows resolves `/nonexistent/dir/brain.db` as a valid drive-rooted path).
Pushed as commit `56e44ce` on `spec/vault-sync-engine`.

**Decision record:** `.squad/decisions/inbox/mom-vault-sync-lane.md`

**Lessons:**
- `init_process_registries()` clears ALL registries — safe to call as first line of any test
  that exercises code paths using global handles. Make it a standard first-line in affected tests.
- `uuid_migration_preflight` blocks on trivial-body pages where `frontmatter.gbrain_id ≠ page.uuid`.
  Any test helper that inserts pages with uuid AND raw bytes containing `gbrain_id` must
  ensure `pages.frontmatter` is populated from the raw bytes, not left as `'{}'`.
- Error string contracts are bidirectional: both the format producer (vault_sync.rs) and all
  format consumers (server.rs handler, put.rs tests, mcp/server.rs tests) must agree. When
  multiple consumers exist with different substring expectations, enumerate all expectations
  before choosing the format.
- On Windows, unix-style paths like `/nonexistent/dir/brain.db` are NOT invalid — SQLite
  opens them relative to the drive root. Tests asserting on path-nonexistence must be
  platform-conditioned (`#[cfg(unix)]`) or use a truly nonexistent platform-native path.

---

### 2026-04-25 Restore Artifact Reconciliation (mixed-author cleanup)

**Context:** Fry's restore artifact was rejected and Fry locked out. My prior commit
`e29d1d0` fixed the 5 quarantine restore blockers. Uncommitted changes remained in 4 files
containing a mix of required glue and a dropped Fry artifact piece.

**What happened:**
- Audited all 4 uncommitted files against the narrow Unix restore contract.
- **Kept:** `linkat_parent_fd` (required by committed `quarantine.rs`), `pub(crate)` lease
  visibility (required by `quarantine.rs`), restore routing in `collection.rs`, rewritten
  CLI truth tests.
- **Dropped:** `walk_to_parent_create_dirs` (both variants + test + doc line) — explicitly
  rejected in `e29d1d0` as Blocker 4. Fry's artifact included it; the safe contract refuses
  absent parents rather than silently recreating them without a durable fsync chain.
- Committed clean artifact in `6a3d54c`. 591 tests pass. 2 Windows-only pre-existing failures.
- Decision record: `.squad/decisions/inbox/mom-restore-artifact-reconcile.md`

**Lesson:**
- When auditing a mixed-author worktree, the question is not "does this compile?" but
  "which piece was explicitly rejected and why?" A removed function that sneaks back in
  as a dependency of another change is a silent contract violation. Check the commit
  message of the prior revision for the exact reason each piece was excluded.
- Compile-required ≠ contract-required. `walk_to_parent_create_dirs` was not called by
  any live path — it was dead code that would have silently re-opened the Fry design.
  The test for it would have falsely green-lit the pattern as accepted behavior.

---

## 2026-04-25 macOS Preflight Diagnostics — Issue #79/#80 Root Cause

**Session:** mom-issue79-80-macos (2026-04-25T12:37:39Z)  

**Status:** Diagnostic phase complete. Root cause identified; minimum workflow-only fix specified.

**Finding:** PR #83 four macOS preflight jobs (72986784880, 72986784883, 72986784888, 72986784898) all fail at the same point: .github/workflows/ci.yml:78 cache-key construction. The cache key embeds raw matrix.features with comma-joined values (e.g., undled,online-model, undled,embedded-model). ctions/cache@v4 hard-fails with Key Validation Error ... cannot contain commas before cargo check starts.

**Issue #80 Status:** src/core/fs_safety.rs:199 now contains the widening cast (mode_bits: stat.st_mode as u32), but issue #80 remains operationally open on this branch because the macOS proof job never reaches compilation. No fresh evidence that macOS cargo check passes with the fix.

**Minimum Fix:** Workflow-only change. Sanitize cache-key field:
- Option 1: Replace commas with dashes in cache key
- Option 2: Use explicit matrix-safe token (e.g., undled-online-model)

**Decision Recorded:** .squad/decisions.md entry "2026-04-25: macOS preflight cache-key sanitization — Issue #79/#80 workflow unblocker" (D-M1 through D-M3 decisions for future workflow safety).

**Skill Added:** .squad/skills/github-actions-cache-key-sanitization/SKILL.md for reference in future workflow maintenance.

**Artifact:** Orchestration log at .squad/orchestration-log/20260425T123739Z-mom-issue79-80-macos.md. Session log at .squad/log/20260425T123739Z-issue79-80-macos.md.

**Lesson:** Workflow parameter constraints (cache-key format, matrix safety) are not obvious from CI failure output alone. Log inspection needed to find the ctions/cache validation error earlier in the job lifecycle.


- Batch 1 edge-case implementation (6.8 + cleanup) complete (2026-04-27T23:51:40Z): .quaidignore watcher surface hardened with atomic pattern reload, markdown-only filter bypass, and last-known-good mirror preservation. Tasks 6.7a, 6.9, 6.10, 6.11 explicitly kept open. Orchestration log at .squad/orchestration-log/2026-04-27T23-51-40Z-mom.md.
- Batch 1 core-coverage lane (2026-04-28): after `cargo llvm-cov --lib --tests --summary-only --no-clean -j 1`, refresh `target\\llvm-cov-report.json` with `cargo llvm-cov report --json --output-path target\\llvm-cov-report.json` before claiming exact moved lines. In this repo that exposed the honest ceiling of the Mom lane: Windows stub coverage in `fs_safety.rs` is cheap and worth taking, but even pushing `search.rs` + `quarantine.rs` into the high 90s only moved global line coverage to 89.79%, so Batch 1/release closure still needs coverage from outside Mom's files.
- Batch 1 heavy coverage continuation (2026-04-28): the honest Windows gains in `src/commands/collection.rs` came from edge-first helper seams, not pretending Unix-only attach/sync/restore bodies are reachable. Direct tests for duplicate attach refusal, deferred `--write-quaid-id`, ignore-source read failures, quarantine list/export/discard wrappers, and canonical exact-slug search branches moved the refreshed global gate to 88.76% and `collection.rs` to 86.43% on the JSON line map, but the lane still cannot truthfully claim 90% or ship `v0.10.0`.
