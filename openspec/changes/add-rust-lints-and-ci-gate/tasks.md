## 1. Establish the lint configuration

- [ ] 1.1 Add `[lints.rust]` table to `Cargo.toml` with `unsafe_code = "forbid"`, `missing_docs = "warn"`, `unreachable_pub = "warn"`.
- [ ] 1.2 Add `[lints.clippy]` table to `Cargo.toml` with `all` and `pedantic` at warn (priority `-1`), plus explicit warns for `unwrap_used`, `expect_used`, `panic`, `print_stdout`, `redundant_clone`, `needless_collect`, `large_enum_variant`.
- [ ] 1.3 Run `cargo clippy --all-targets --all-features --locked 2>&1 | tee /tmp/clippy-baseline.txt` and capture the full warning list. This is the working set for tasks 2–4.
- [ ] 1.4 Decide and document the `missing_docs` posture: either (a) `#[expect(missing_docs, reason = "addressed in add-crate-and-public-api-docs")]` per item — high churn — or (b) downgrade `missing_docs` to `allow` at crate level temporarily, gated to remove when proposal #7 lands. Pick one; record in `design.md` if not (a).

## 2. Audit and migrate existing `#[allow(...)]` annotations

- [ ] 2.1 Generate the audit list: `rg -e '#\[allow\(' --line-number src/ > /tmp/allow-audit.txt`. Expected: ~46 occurrences across 16 files.
- [ ] 2.2 For each file in the audit, classify each annotation as: (a) dead — underlying lint no longer fires, delete it; (b) ongoing — legitimate, migrate to `#[expect(...)]` with `reason = "..."`; (c) deferred — needs engineering work, migrate to `#[expect(...)]` with `reason = "addressed in <follow-up-name>"`. Cross-reference against `/tmp/clippy-baseline.txt` to identify dead suppressions.
- [ ] 2.3 Migrate `src/commands/search.rs` (1 annotation), `src/commands/link.rs` (1), `src/commands/query.rs` (1). Run `cargo clippy --all-targets --all-features --locked -- -D warnings` after each file; commit each file separately.
- [ ] 2.4 Migrate `src/core/graph.rs` (1), `src/core/gaps.rs` (1), `src/core/db.rs` (1), `src/core/assertions.rs` (1), `src/core/palace.rs` (1), `src/core/progressive.rs` (1), `src/core/conversation/file_edit.rs` (1). One commit per file.
- [ ] 2.5 Migrate `src/core/search.rs` (4 annotations) and `src/core/inference.rs` (4 annotations). One commit per file.
- [ ] 2.6 Migrate `src/core/fts.rs` (6 annotations). Several are `#[allow(dead_code)]` on un-namespaced FTS variants — those become `#[expect(dead_code, reason = "addressed in collapse-search-fts-api-surface")]` rather than getting deleted, since proposal #6 deletes the variants entirely.
- [ ] 2.7 Migrate `src/commands/put.rs` (3 annotations) and `src/core/vault_sync.rs` (10 annotations). One commit per file. The `vault_sync.rs` migration is the largest single batch — review carefully.

## 3. Suppress deferred-engineering warnings inline

- [ ] 3.1 Add `#[expect(clippy::unwrap_used, reason = "addressed in fix-production-unwraps-and-panics")]` directly above `src/core/vault_sync.rs:3380`'s `pending_root_path.clone().unwrap()`.
- [ ] 3.2 Add `#[expect(clippy::unwrap_used, reason = "addressed in fix-production-unwraps-and-panics")]` above each of the four `serde_json::to_string_pretty(...).unwrap()` sites in `src/mcp/server.rs` (lines 1793, 1880, 1892, 1990 per `docs/CODE_REVIEW.md` §2.1).
- [ ] 3.3 Add `#[expect(clippy::unwrap_used, reason = "addressed in fix-production-unwraps-and-panics")]` above each of the six `lock().unwrap()` sites in `src/core/inference.rs`.
- [ ] 3.4 Add `#[expect(clippy::large_enum_variant, reason = "addressed in split-vault-sync-module")]` above the `VaultSyncError` enum declaration in `src/core/vault_sync.rs`.
- [ ] 3.5 Run `cargo clippy --all-targets --all-features --locked -- -D warnings`. If any warnings remain, either fix trivially or add another `#[expect(...)]` with a follow-up reason. Document the final warning count (should be 0) in the PR description.

## 4. Verify both feature channels are green

- [ ] 4.1 Run `cargo clippy --all-targets --all-features --locked -- -D warnings`. Expect: pass.
- [ ] 4.2 Run `cargo clippy --all-targets --no-default-features --features bundled,online-model --locked -- -D warnings`. Expect: pass. If new warnings surface in `online-model`-gated code, add per-site `#[expect(...)]` annotations.
- [ ] 4.3 Run `cargo fmt --all -- --check`. Expect: pass.
- [ ] 4.4 Run `cargo test --all-features --locked` to verify no test regressions from the migration.

## 5. Tighten the CI gate

- [ ] 5.1 In `.github/workflows/ci.yml::check`, change the default-channel clippy step from `cargo clippy --all-targets -- -D warnings` to `cargo clippy --all-targets --all-features --locked -- -D warnings`.
- [ ] 5.2 In the same job, add `--locked` to the online-channel clippy step so it reads `cargo clippy --all-targets --no-default-features --features bundled,online-model --locked -- -D warnings`.
- [ ] 5.3 Verify the `cargo fmt --all -- --check` step is unchanged and present.
- [ ] 5.4 Open a draft PR and confirm both clippy steps and the fmt step run green on the CI runner. If green locally but red in CI, investigate the cache vs `--locked` interaction before merging.

## 6. Cross-cutting verification

- [ ] 6.1 Confirm zero `#[allow(...)]` remains in `src/` outside `#[cfg(test)]` modules: `rg -e '#\[allow\(' src/ | rg -v '#\[cfg\(test\)\]'`.
- [ ] 6.2 Confirm every `#[expect(...)]` carries a `reason = "..."` argument: `rg -e '#\[expect\(' src/ | rg -v 'reason'` should return zero lines.
- [ ] 6.3 Grep for the follow-up change names referenced in deferred suppressions and confirm they match `docs/temp_IMPL_PLAN.md`: `rg -e 'reason = "addressed in' src/`.
- [ ] 6.4 Update `docs/temp_IMPL_PLAN.md` to mark proposal #1 row as ✅ shipped, with the change name and a one-line summary of what landed.
- [ ] 6.5 Commit message references this OpenSpec change name and the originating `docs/CODE_REVIEW.md` sections (§3.1, §3.2, §3.3).
