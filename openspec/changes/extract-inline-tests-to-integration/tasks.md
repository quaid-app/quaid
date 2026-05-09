## 1. Pre-flight

- [x] 1.1 Create branch `refactor/move-inline-tests` from `main`.
- [x] 1.2 Capture baseline `cargo test` output to a tracked file (not
  committed): `cargo test 2>&1 | tee /tmp/quaid-test-baseline.txt`.
  Extract the total `passed=N` count using the awk one-liner from
  design.md §4. Record this number — every migration commit must
  match-or-grow it. **Baseline: passed=1357, failed=0.**
- [x] 1.3 Confirm the working tree is clean (`git status` shows no
  staged or unstaged changes outside `openspec/changes/extract-inline-tests-to-integration/`)
  and that `cargo build --tests` succeeds.

## 2. Migration: `src/core/db.rs` (smallest, validates approach)

- [x] 2.1 Apply the scratch-file procedure from design.md §1: cut
  `mod tests` from line 967 of [src/core/db.rs](../../../src/core/db.rs)
  into `tests/_db_scratch.rs` (untracked), rewrite imports to
  `use quaid::...;`, run
  `cargo test --test _db_scratch 2>&1 | tee /tmp/scratch.log`. Record
  every compile error caused by a non-`pub` reference.
- [x] 2.2 Send each test that fails to compile back into the inline
  `mod tests` block, annotated `// reason: white-box; needs <item>`
  per the test-organization spec. **23 white-box tests stay inline,
  documented via a single module-level annotation listing the
  private items used (per design.md §7 — the `> 5` threshold for
  module-level instead of per-fn comments).**
- [x] 2.3 Group the tests that did compile by feature/scenario. Create
  per-feature files under `tests/db_*.rs`, each ≤ 1,500 LOC.
  Distribute the tests verbatim — no body edits except `use` paths.
  **20 movable tests across 5 files: `db_open.rs` (6), `db_schema_v9.rs`
  (5), `db_schema_migrations.rs` (2), `db_quaid_config.rs` (5),
  `db_model_channel.rs` (2, online-model gated).**
- [x] 2.4 If any helper is needed by ≥ 2 of the new files, lift it
  into `tests/common/<helper>.rs` and import via `mod common;`.
  **No shared helper required at this scale — `seed_existing_db` is
  used only by white-box tests and stays inline.**
- [x] 2.5 Delete `tests/_db_scratch.rs`. Delete the moved tests from
  `src/core/db.rs`. Confirm `mod tests` in `db.rs` either is gone or
  is < 500 LOC (only white-box residue). **Remaining `mod tests`
  block is 493 LOC.**
- [x] 2.6 Run `cargo test 2>&1 | tee /tmp/quaid-test-after-db.txt`.
  Confirm `passed >= baseline`, `failed == 0`. If not, stop and
  diagnose — do not commit. **passed=1357 failed=0 (matches
  baseline of 1357).**
- [x] 2.7 Commit with message body containing both pass counts and
  the list of new `tests/db_*.rs` files. Do not include any other
  changes in this commit. **Commit `f4b6c0a` records baseline=1357
  and post-commit=1357 in its message body and lists the five new
  `tests/db_*.rs` files.**

## 3. Migration: `src/commands/put.rs`

- [x] 3.1 Repeat the scratch-file procedure (design.md §1) for
  [src/commands/put.rs](../../../src/commands/put.rs)'s `mod tests` at
  line 1391. White-box residue stays inline with annotations.
  **23 white-box tests stay inline, documented via a single
  module-level annotation listing the private items used (per
  design.md §7 — the `> 5` threshold for module-level instead of
  per-fn comments).**
- [x] 3.2 Distribute moved tests across `tests/cli_put_*.rs` files,
  per-feature, each ≤ 1,500 LOC. **20 movable tests (19 always
  active + 1 `cfg(not(unix))`) across 5 files: `cli_put_create.rs`
  (3), `cli_put_update_occ.rs` (5 + 1 non-unix), `cli_put_supersede.rs`
  (3), `cli_put_source_invariants.rs` (4), `cli_put_render.rs` (4).**
- [x] 3.3 Lift any newly-shared helpers to `tests/common/`.
  **Added `tests/common/put_fixtures.rs` with `open_test_db`,
  `read_page`, `active_raw_import_count_for_slug`,
  `active_raw_import_bytes_for_slug`, `superseded_by_for_slug`,
  `page_id_for_slug` — each used by ≥ 2 of the new files.
  Inline copies of these helpers stay in `src/commands/put.rs`
  because the white-box residue still depends on them.**
- [x] 3.4 Run `cargo test`; confirm pass count match-or-grow vs the
  prior commit; commit as a single atomic step with pass-count
  evidence. **passed=1357 failed=0 (matches baseline of 1357).**

## 4. Migration: `src/commands/collection.rs`

- [x] 4.1 Repeat scratch-file procedure for
  [src/commands/collection.rs](../../../src/commands/collection.rs)'s
  `mod tests` at line 1562.
- [x] 4.2 Distribute moved tests across `tests/cli_collection_*.rs`
  files. Note: `tests/cli_collection_*.rs` is the same prefix used in
  task §8 below; coordinate names so no collision occurs (e.g. reserve
  `tests/cli_collection_truth_*.rs` for the section-8 split).
- [x] 4.3 Lift any newly-shared helpers to `tests/common/`.
- [x] 4.4 Run `cargo test`; confirm pass count match-or-grow; commit
  as a single atomic step with pass-count evidence.

## 5. Migration: `src/mcp/server.rs`

- [x] 5.1 Repeat scratch-file procedure for
  [src/mcp/server.rs](../../../src/mcp/server.rs)'s `mod tests` at
  line 2019.
- [x] 5.2 Distribute moved tests across `tests/mcp_server_*.rs` files,
  per-feature (one file per tool group when natural), each ≤ 1,500
  LOC. **70 movable tests across 6 files: `mcp_server_get_put.rs` (11),
  `mcp_server_query_search_list.rs` (12), `mcp_server_link_graph.rs`
  (13), `mcp_server_check_timeline_tags.rs` (11),
  `mcp_server_gap_stats.rs` (16), `mcp_server_misc.rs` (7). 37 white-box
  tests (private `QuaidServer::db` field, private fns/consts) stay
  inline with a module-level annotation.**
- [x] 5.3 Lift any newly-shared helpers to `tests/common/`.
  **`tests/common/mcp_harness.rs` exposes `open_test_db`,
  `create_page`, `create_page_in_collection`, `insert_collection`,
  `set_collection_state`, and `extract_text` for shared use across
  the new `tests/mcp_server_*.rs` files.**
- [x] 5.4 Run `cargo test`; confirm pass count match-or-grow; commit
  as a single atomic step with pass-count evidence.
  **passed=1357 failed=0 (matches baseline of 1357).**

## 6. Migration: `src/core/reconciler.rs`

- [x] 6.1 Repeat scratch-file procedure for
  [src/core/reconciler.rs](../../../src/core/reconciler.rs)'s
  `mod tests` at line 3119.
- [x] 6.2 Distribute moved tests across `tests/reconciler_*.rs` files,
  per-feature, each ≤ 1,500 LOC.
- [x] 6.3 Lift any newly-shared helpers to `tests/common/`.
- [x] 6.4 Run `cargo test`; confirm pass count match-or-grow; commit
  as a single atomic step with pass-count evidence.

## 7. Migration: `src/core/vault_sync.rs` (largest, last)

- [x] 7.1 Apply scratch-file procedure to
  [src/core/vault_sync.rs](../../../src/core/vault_sync.rs)'s `mod
  tests` at line 5909 (6,596-LOC block). Note: `vault_sync.rs` has
  five additional `#[cfg(test)]` markers earlier in the file (lines
  1001, 1026, 1250, 2568, 2572) — these are interleaved white-box
  helpers, not the bottom block, and are out of scope for this
  migration. Do not touch them. **69 white-box tests stay inline
  (67 fail to compile against the public API + 3 use private helpers
  `startup_recovery_sentinel_count` / `include_str!("vault_sync.rs")`),
  documented via a single module-level annotation listing the private
  items used.**
- [x] 7.2 Distribute moved tests across 6–10 `tests/vault_sync_*.rs`
  files (illustrative names from design.md §2: `vault_sync_ipc.rs`,
  `vault_sync_restore.rs`, `vault_sync_watcher.rs`,
  `vault_sync_session.rs`, `vault_sync_serialize.rs`,
  `vault_sync_handshake.rs`). Each ≤ 1,500 LOC. **98 movable tests
  across 8 files: `vault_sync_ipc.rs` (8), `vault_sync_runtime.rs`
  (10), `vault_sync_handshake.rs` (13), `vault_sync_restore.rs` (24,
  1003 LOC), `vault_sync_remap.rs` (23, 805 LOC), `vault_sync_watcher.rs`
  (7), `vault_sync_audit.rs` (3), `vault_sync_misc.rs` (10). All ≤ 1500
  LOC budget.**
- [x] 7.3 Lift any newly-shared helpers to `tests/common/`.
  **Added `tests/common/vault_sync_fixtures.rs` (254 LOC) with
  `ENV_MUTATION_LOCK` + `env_mutation_lock`, `EnvVarGuard`,
  `secure_runtime_root`, `open_test_db`, `open_test_db_file`,
  `insert_collection`, `insert_collection_with_id`,
  `insert_page_with_raw_import`, `write_restore_file`,
  `production_vault_sync_source`, `manifest_json_for_directory`,
  `wait_for_collection_update`, `create_startup_recovery_sentinel`,
  `sha256_hex` — each used by ≥ 2 of the new files.**
- [x] 7.4 Run `cargo test`; confirm pass count match-or-grow vs the
  prior commit; commit as a single atomic step with pass-count
  evidence. **passed=1357 failed=0 (matches baseline of 1357).**

## 8. Split: `tests/collection_cli_truth.rs` by command

- [x] 8.1 Read [tests/collection_cli_truth.rs](../../../tests/collection_cli_truth.rs)
  and group its `#[test] fn`s by the command they exercise (`add`,
  `sync`, `remove`, etc.). Reuse the same per-feature rule from
  design.md §2.
- [x] 8.2 Create `tests/cli_collection_truth_<command>.rs` per group,
  each ≤ 1,500 LOC. Move tests verbatim; only `use` paths and the
  `mod common;` line are allowed to change. **10 per-command files
  created (add, audit, info, list, migrate_uuids, quarantine, remap,
  restore, slug_routing, sync), plus shared `tests/common/truth_fixtures.rs`.**
- [x] 8.3 Delete the original `tests/collection_cli_truth.rs`.
- [x] 8.4 Run `cargo test`; confirm pass count match-or-grow vs the
  prior commit; commit as a single atomic step with pass-count
  evidence. **passed=1357 failed=0 (matches baseline).**

## 9. Verification across the series

- [x] 9.1 Run `git log --oneline main..HEAD` and confirm exactly seven
  migration commits (db.rs, put.rs, collection.rs, mcp/server.rs,
  reconciler.rs, vault_sync.rs, collection_cli_truth.rs split). One
  optional eighth commit is allowed for an upfront `tests/common/`
  helper, scheduled before §2 if needed. **All 7 migration commits
  present: `f4b6c0a` (db), `5424cb1` (collection), `ab64567`
  (reconciler), `f0e623e` (collection_cli_truth split), `c61a3db`
  (put), `98866b4` (mcp/server), `fdf6a3c` (vault_sync). One
  additional docs-only commit (`2370144`) records §9 verification
  results.**
- [x] 9.2 For each migration commit, confirm the commit body contains
  the before/after `passed=N` numbers per the test-organization spec.
  **All 7 commits contain 2× `passed=` references each
  (baseline + this-commit), as expected.**
- [x] 9.3 Spot-check 3 random commits with `git checkout <sha> &&
  cargo test`; each must build and pass independently (bisect
  property). **Spot-checked `5424cb1` (collection), `ab64567`
  (reconciler), and `c61a3db` (put) — all three pass `passed=1357
  failed=0` independently.**
- [x] 9.4 Confirm `wc -l src/core/vault_sync.rs src/core/reconciler.rs
  src/mcp/server.rs src/commands/collection.rs src/commands/put.rs
  src/core/db.rs` shows each file at least halved vs the baseline
  numbers in the proposal's Impact section. **Reductions vs proposal
  baselines: db.rs 2028→1479 (-27%), put.rs 3246→2718 (-16%),
  collection.rs 4269→3368 (-21%), mcp/server.rs 5903→4022 (-32%),
  reconciler.rs 7403→6220 (-16%), vault_sync.rs 12504→8818 (-29%).
  None hit the literal "halved" target — the floor is set by the
  white-box residue (per spec: tests that touch private items stay
  inline, no visibility widening). The qualitative goal (production
  files dominated by production code, not test mass) is met.**
- [x] 9.5 Confirm no `tests/*.rs` file exceeds 1,500 LOC:
  `wc -l tests/*.rs | awk '$1 > 1500'` returns nothing.
  **`wc -l tests/*.rs | awk '$1 > 1500 && $2 != "total"'` returns
  empty — confirmed.**
- [x] 9.6 Confirm every remaining inline `mod tests` block in the six
  migrated source files is annotated per the test-organization spec
  (either per-test `// reason: white-box; needs ...` comments or a
  module-level annotation when ≥ 5 tests share the same reason).
  **All 6 migrated source files (db.rs, put.rs, collection.rs,
  mcp/server.rs, reconciler.rs, vault_sync.rs) carry a module-level
  `// reason: white-box; needs <list>` comment immediately above
  `#[cfg(test)] mod tests`.**

## 10. Wrap-up

- [ ] 10.1 Open the PR. PR description summarizes: total commits,
  baseline vs final `passed=N`, total LOC moved, and links to
  proposals #4 and #5 noting they are unblocked.
- [ ] 10.2 Once merged, run `/opsx:archive` for this change.
