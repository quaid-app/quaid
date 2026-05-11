## 1. Establish the lint configuration

- [x] 1.1 Add `[lints.rust]` table to `Cargo.toml` with `unsafe_code = "deny"` (changed from `forbid` per design.md update — codebase has ~18 legitimate FFI/mmap/syscall `unsafe` blocks), `missing_docs = "allow"` (deferred to proposal #7), `unreachable_pub = "warn"`.
- [x] 1.2 Add `[lints.clippy]` table to `Cargo.toml` with `all` at warn (priority `-1`) plus explicit warns for `unwrap_used`, `expect_used`, `panic`, `print_stdout`, `redundant_clone`, `needless_collect`, `large_enum_variant`. (`clippy::pedantic` was dropped per design.md Decision 1 — 5,672 baseline warnings made it unmanageable for this proposal; deferred to a follow-up after production cleanup.)
- [x] 1.3 Run `cargo clippy --all-targets --locked 2>&1 | tee /tmp/clippy-baseline.txt` and capture the full warning list. Baseline after dropping pedantic + applying test/CLI exemptions: 89 warnings (from 5,672). This is the working set for tasks 2–4.
- [x] 1.4 Decide and document the `missing_docs` posture: chose **option (b)** — `missing_docs = "allow"` at crate level via `Cargo.toml [lints.rust]`. Recorded in `design.md` "Decision 1" (alternatives section). Removal is gated on proposal #7 (`add-crate-and-public-api-docs`).
- [x] 1.5 Annotate every existing `unsafe` block in `src/` with `#[expect(unsafe_code, reason = "...")]` describing why the FFI/mmap/syscall is unavoidable. Audit list (~18 sites): `core/db.rs` (sqlite-vec init), `core/inference.rs` ×2 (candle mmap), `core/conversation/slm.rs` (candle mmap), `core/conversation/turn_writer.rs` ×4 (POSIX flock / Win32 LockFile), `core/vault_sync.rs` ×5 (libc euid/listen/getpeereid/ucred), `core/raw_imports.rs` ×3 (libc), `core/reconciler.rs` ×2 (libc), `commands/put.rs` ×2 (libc).
- [x] 1.6 Add test-side exemptions: `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stdout, reason = "..."))]` at `src/lib.rs`, plus equivalent inner `#![allow(...)]` at the top of each integration test in `tests/*.rs` (35 files), `tests/common/*.rs` (2 files), and `benches/*.rs` (1 file). Test code legitimately uses `unwrap`/`expect`/`panic`/`println!` in fixtures; per-site `#[expect]` would generate noise across thousands of test sites (design.md §Decision 2 test exemption).
- [x] 1.7 Add `#![expect(clippy::print_stdout, reason = "CLI command prints user-facing output to stdout by design")]` at the top of each `src/commands/*.rs` (25 files), and `#![allow(clippy::print_stdout, reason = "...")]` at `src/main.rs`. The `quaid` CLI legitimately prints user-facing output; library `print_stdout` would otherwise warn on each call.

## 2. Audit and migrate existing `#[allow(...)]` annotations

- [x] 2.1 Generate the audit list: `rg -e '#\[allow\(' --line-number src/ > /tmp/allow-audit.txt`. Found 46 occurrences across 16 files (matches proposal estimate).
- [x] 2.2 For each file in the audit, classify each annotation. Result: 33 dead (deleted; underlying lint did not fire — these were redundant suppressions), 13 ongoing/deferred (migrated to `#[expect(...)]` with `reason = "..."`).
- [x] 2.3 Migrate `src/commands/search.rs`, `src/commands/link.rs`, `src/commands/query.rs`. (`link.rs` `dead_code` was unfulfilled — deleted.)
- [x] 2.4 Migrate `src/core/graph.rs`, `src/core/gaps.rs`, `src/core/db.rs`, `src/core/assertions.rs`, `src/core/palace.rs`, `src/core/progressive.rs`, `src/core/conversation/file_edit.rs`. Several `dead_code` allows were unfulfilled (public lib items reachable through callers) — deleted.
- [x] 2.5 Migrate `src/core/search.rs` (4) and `src/core/inference.rs` (7). All 4 dead_code allows in search.rs and 5 dead_code allows in inference.rs were unfulfilled — deleted; only `clippy::too_many_arguments` and `unreachable_code` annotations migrated.
- [x] 2.6 Migrate `src/core/fts.rs` (6). All 4 `#[allow(dead_code)]` on un-namespaced/canonical FTS variants were unfulfilled — deleted (lint never fired because they're public lib API). The 2 `clippy::too_many_arguments` annotations migrated to `#[expect(..., reason = "addressed in collapse-search-fn-variants")]`.
- [x] 2.7 Migrate `src/commands/put.rs` (3) and `src/core/vault_sync.rs` (13 — count corrected from 10 in original audit). 11 dead_code in vault_sync were unfulfilled — deleted; the remaining `dead_code` on field-Drop semantics and the `clippy::question_mark` annotation in put.rs were migrated.

## 3. Suppress deferred-engineering warnings inline

- [x] 3.1 Suppression for `vault_sync.rs:3385` `pending_root_path.clone().unwrap()` covered by file-level `#![expect(clippy::unwrap_used, clippy::expect_used, reason = "addressed in remove-production-panic-paths")]` in `src/core/vault_sync.rs`. (Per-site annotations would have been ~15 distinct edits across the file; file-level scope keeps the suppression greppable and surfaces as unfulfilled when ALL sites are fixed.)
- [x] 3.2 Suppression for the 4 `serde_json::to_string_pretty(...).unwrap()` sites in `src/mcp/server.rs` covered by file-level `#![expect(clippy::unwrap_used, reason = "addressed in remove-production-panic-paths")]`.
- [x] 3.3 Inference.rs no longer contains any `lock().unwrap()` mutex sites — those were refactored away in a prior commit. The 6 `expect_used` sites that did surface (lines 1042, 1052, 1073, 1087, 256, 268) are covered by file-level `#![expect(clippy::expect_used, reason = "addressed in remove-production-panic-paths")]` in `src/core/inference.rs`.
- [x] 3.4 `clippy::large_enum_variant` does NOT currently fire on `VaultSyncError` (verified by clippy run after lint-set change) — annotation skipped to avoid an unfulfilled `#[expect]`. If proposal #4 introduces variants that trip the threshold, this annotation can be added then.
- [x] 3.5 Final clippy: `cargo clippy --all-targets --locked -- -D warnings` passes with 0 warnings. The deferred-engineering suppressions are listed in `src/main.rs`, `src/mcp/server.rs`, `src/core/{vault_sync,inference,db,collections,ignore_patterns,links,assertions}.rs`, `src/core/conversation/{correction,format,slm,supersede}.rs`, `src/commands/{extract,link}.rs`, and `build.rs`. Greppable via `rg 'addressed in remove-production-panic-paths' src/`.

## 4. Verify both feature channels are green

- [x] 4.1 Run `cargo clippy --all-targets --locked -- -D warnings`. **Pass.**
- [x] 4.2 Run `cargo clippy --all-targets --no-default-features --features bundled,online-model --locked -- -D warnings`. **Pass** — no online-model-gated code surfaced new warnings.
- [x] 4.3 Run `cargo fmt --all -- --check`. **Pass** after `cargo fmt --all` cleaned blank-line spacing on script-injected attributes.
- [x] 4.4 Run `cargo test --locked --lib`: 1114 of 1131 tests pass. The 17 failing tests are **pre-existing failures** verified by reproducing on a clean baseline (stash + retest before this change). They involve slug-lock timing in `commands::put::tests::*`, `commands::collection::tests::*`, and `core::vault_sync::tests::start_serve_runtime_*`, and are unrelated to this lint scaffolding change. (`cargo test --all-features` cannot be run because `embedded-model` and `online-model` are mutually exclusive — same channel-mutex `compile_error!` covered in task 5.)

## 5. Tighten the CI gate

- [x] 5.1 In `.github/workflows/ci.yml::check`, changed the default-channel clippy step from `cargo clippy --all-targets -- -D warnings` to `cargo clippy --all-targets --locked -- -D warnings`. (`--all-features` is incompatible with the channel-mutex `compile_error!` in `src/core/inference.rs`.)
- [x] 5.2 Added `--locked` to the online-channel clippy step: `cargo clippy --all-targets --no-default-features --features bundled,online-model --locked -- -D warnings`.
- [x] 5.3 Verified the `cargo fmt --all -- --check` step is unchanged and present at `.github/workflows/ci.yml:66`.
- [x] 5.4 (User-driven) Open a draft PR and confirm both clippy steps and the fmt step run green on the CI runner. If green locally but red in CI, investigate the cache vs `--locked` interaction before merging.

## 6. Cross-cutting verification

- [x] 6.1 Confirmed zero `#[allow(...)]` remains in `src/` (verified by `rg -e '#\[allow\(' src/` returning empty). The migration converted all 46 sites; the test exemptions at the crate root in `lib.rs`, `main.rs`, and per-test-file in `tests/` use `#![allow(...)]` (allowed by spec for test code).
- [x] 6.2 Confirmed every `#[expect(...)]` in `src/` carries a `reason = "..."` argument (verified by Python regex scan over multi-line attributes). Two pre-existing `#[expect(clippy::too_many_arguments)]` annotations in `src/core/quarantine.rs:1124` and `src/commands/put.rs:1183` (not in original audit) were also given reasons.
- [x] 6.3 Grepped for follow-up change names. Three distinct follow-ups referenced in source: `remove-production-panic-paths` (proposal #2), `decompose-vault-sync-module` (proposal #4), `collapse-search-fn-variants` (proposal #6) — all match `docs/temp_IMPL_PLAN.md`. The proposal/design/spec/tasks artifacts also use these canonical names (renamed from earlier draft names that didn't match).
- [x] 6.4 Updated `docs/temp_IMPL_PLAN.md` proposal #1 row to ✅ shipped with one-line summary.
- [x] 6.5 (User-driven) Commit message references this OpenSpec change name (`add-rust-lints-and-ci-gate`) and the originating `docs/CODE_REVIEW.md` sections (§3.1, §3.2, §3.3).
