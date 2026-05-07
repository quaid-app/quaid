## Why

`docs/CODE_REVIEW.md` §3 flagged that `Cargo.toml` has no `[lints.rust]` or `[lints.clippy]` table, no crate-level deny attributes, no `clippy.toml`, and no `rustfmt.toml`. The repo does have a CI clippy/fmt gate (`.github/workflows/ci.yml::check`), but it enforces only `cargo clippy --all-targets -- -D warnings` against whatever clippy considers warning-by-default — which is much weaker than the Apollo Rust Best Practices Handbook recommendation. The result: the four `serde_json::*.unwrap()` cases in `src/mcp/server.rs`, the six mutex `lock().unwrap()` cases in `src/core/inference.rs`, the `large_enum_variant` bloat on `VaultSyncError`, and the nine dead `#[allow(dead_code)]` annotations in `core/fts.rs` etc. all pass CI today. None of them should.

Landing a comprehensive lint configuration is the cheapest, highest-leverage step in the code-review follow-up plan (`docs/temp_IMPL_PLAN.md` #1). It surfaces the targets for proposal #2 (`fix-production-unwraps-and-panics`) automatically, and turns several later proposals from "audit by hand" into "fix what clippy already flagged."

## What Changes

- Add `[lints.rust]` to `Cargo.toml` per Apollo §3.1: `unsafe_code = "forbid"`, `missing_docs = "warn"`, `unreachable_pub = "warn"`.
- Add `[lints.clippy]` to `Cargo.toml`: `clippy::all = "warn"` and `clippy::pedantic = "warn"` (priority `-1` so individual rules can override), plus explicit `warn` for `unwrap_used`, `expect_used`, `panic`, `print_stdout`, `redundant_clone`, `needless_collect`, `large_enum_variant`.
- Migrate every existing `#[allow(...)]` annotation in `src/` (audit shows 46 occurrences across 16 files) to `#[expect(...)]` (Rust ≥ 1.81). Each gets a one-line `// reason: ...` comment per Apollo §3.2 so future readers see why the suppression exists, and so dead suppressions surface as warnings when the underlying lint stops firing.
- Tighten the existing CI clippy gate at `.github/workflows/ci.yml::check`: add `--all-features --locked` and keep the existing `-D warnings`. Verify both feature channels (default airgapped, online-model) still pass.
- For warnings that need real engineering work (notably the production unwraps deferred to proposal #2), gate them locally with a scoped `#[expect(clippy::<lint>, reason = "addressed in fix-production-unwraps-and-panics")]` so this proposal can land green and the next one removes the suppression. Do not silently keep `#[allow]` for these.

This proposal does not fix any production unwrap or panic itself. Its scope is the lint infrastructure plus the mechanical migrations and trivially-fixable warnings (e.g. `redundant_clone` on `String` slugs in MCP, dead code in `core/fts.rs`'s un-namespaced variants). Anything requiring design decisions is gated and deferred to a named follow-up.

## Capabilities

### New Capabilities
- `rust-lints-and-ci`: the lint configuration (Cargo.toml `[lints.*]` tables, CI workflow gate, suppression conventions) that defines what counts as "must be fixed before merge" for Rust code in this repo.

### Modified Capabilities
None.

## Impact

- **Code:** `Cargo.toml` (new `[lints.rust]` and `[lints.clippy]` tables), every file under `src/` containing `#[allow(...)]` (16 files, 46 occurrences). The migration is mechanical: `s/#\[allow\(/#\[expect\(/` plus a `// reason:` line above each.
- **CI:** `.github/workflows/ci.yml` `check` job — extend the existing `cargo clippy --all-targets -- -D warnings` to `cargo clippy --all-targets --all-features --locked -- -D warnings`. Both feature channels (airgapped, online-model) remain enforced.
- **Behavior:** No runtime change. Clippy/fmt warnings that previously didn't gate CI now do. Warnings that need engineering work are tagged `#[expect(...)]` with a reason linking to `fix-production-unwraps-and-panics`.
- **Risk:** Low. The change is configuration plus mechanical migration. The blast radius is "merges may be blocked by pre-existing warnings" — which is the intent. Any false positive can be suppressed inline with a `#[expect(...)]` and a reason.
- **Follow-on dependency:** Proposal #2 (`fix-production-unwraps-and-panics`) depends on this landing first so the lints surface the fix targets. Proposals #3–#7 benefit but do not strictly depend on this.
