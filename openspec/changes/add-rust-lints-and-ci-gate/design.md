## Context

`Cargo.toml` currently has no lint configuration. `.github/workflows/ci.yml::check` already runs `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings` against both feature channels (default airgapped, online-model), so the *enforcement* infrastructure exists. What's missing is the *configuration* of which lints to enforce. As a result, CI passes today even though the codebase has 46 `#[allow(...)]` annotations across 16 files in `src/`, four production `serde_json::*.unwrap()` panics, six mutex `lock().unwrap()` calls in `core/inference.rs`, and several un-namespaced FTS variants kept alive only by `#[allow(dead_code)]`. These are the targets of `docs/CODE_REVIEW.md` §2.1, §3.1, §3.2.

This change is item #1 in `docs/temp_IMPL_PLAN.md`. Its job is to declare the lint policy and migrate the existing suppressions. It deliberately does **not** fix the production unwraps — that is proposal #2 (`fix-production-unwraps-and-panics`). Sites that need engineering work get a per-site `#[expect(clippy::<lint>, reason = "addressed in fix-production-unwraps-and-panics")]` so they're greppable and surface the moment the next proposal removes them.

Rust toolchain pin: the repo has no `rust-toolchain.toml`; CI uses `dtolnay/rust-toolchain@stable`. `#[expect(...)]` requires Rust ≥ 1.81. Stable is currently 1.92+, so no toolchain pin is required.

## Goals / Non-Goals

**Goals:**

- `Cargo.toml` declares the canonical lint set so `cargo clippy` locally matches CI.
- Every existing `#[allow(...)]` in `src/` is either deleted (warnings that no longer fire) or migrated to `#[expect(clippy::<lint>, reason = "...")]`.
- CI enforces the new lint set with `--locked` and `--all-features` against both feature channels.
- Warnings requiring engineering work are tagged with a `reason` that names the follow-up change, so a grep finds them all.
- `cargo clippy --all-targets --all-features --locked -- -D warnings` passes on the resulting `main` branch.

**Non-Goals:**

- Fixing the four production `serde_json` unwraps in `mcp/server.rs`, the `vault_sync.rs:3380` panic, or the six `inference.rs` mutex unwraps. These move to proposal #2.
- Restructuring code to satisfy `large_enum_variant` on `VaultSyncError`. The split into per-subsystem child enums is part of proposal #4 (`split-vault-sync-module`); for this change, the `large_enum_variant` warning is suppressed with a reason pointing at #4.
- Removing the `#[allow(dead_code)]` annotations on FTS un-namespaced variants by deleting the variants themselves. That's proposal #6 (`collapse-search-fts-api-surface`); for this change, the `dead_code` warnings are suppressed with a reason pointing at #6.
- Adding a `rust-toolchain.toml` pin. The current `dtolnay/rust-toolchain@stable` is fine; pinning is a separate decision.
- Adding `clippy.toml` or `rustfmt.toml`. Defaults are sufficient until a specific tuning need arises.

## Decisions

### 1. Lint set: Apollo defaults plus four explicit safety lints

**Decision:** Adopt the lint set verbatim from `docs/CODE_REVIEW.md` §3.1, which mirrors Apollo Rust Best Practices Handbook chapter 3. Specifically:

```toml
[lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"
unreachable_pub = "warn"

[lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
unwrap_used = "warn"
expect_used = "warn"
panic = "warn"
print_stdout = "warn"
redundant_clone = "warn"
needless_collect = "warn"
large_enum_variant = "warn"
```

The `priority = -1` on `all` and `pedantic` lets individual lints raise themselves above the group level — required because we want some `pedantic` rules to remain warnings even when the group is downgraded.

**Alternatives considered:**

- *`clippy::nursery = "warn"`.* Rejected: nursery lints have a higher false-positive rate. We can opt in per-rule later if useful.
- *`unsafe_code = "deny"` instead of `"forbid"`.* `forbid` cannot be locally overridden by `#[allow]` or `#[expect]`; `deny` can. Quaid genuinely has zero `unsafe` blocks today, and locking that in is the right default. If a future feature legitimately needs `unsafe`, downgrading to `deny` for that one block is a defensible code-review event.
- *`missing_docs = "deny"` instead of `"warn"`.* Rejected for now. The repo will not be fully documented until proposal #7 lands. Warning is enough — CI denies warnings, so the practical effect is the same once #7 fills in the gaps.

### 2. `#[allow]` → `#[expect]` migration: file-by-file, with reasons

**Decision:** Audit every `#[allow(...)]` in `src/` (currently 46 occurrences across 16 files per `rg -e '#\[allow\(' src/`). For each:

1. If the suppressed lint no longer fires (the underlying code was already fixed), delete the annotation.
2. If the suppression is legitimate and ongoing, replace `#[allow(...)]` with `#[expect(...)]` and add `reason = "..."` (or a `// reason:` line above) explaining why.
3. If the suppression is deferring engineering work, replace with `#[expect(...)]` and a reason naming the follow-up change.

Test-side `#[allow]` (in `#[cfg(test)] mod tests { ... }` and `tests/`) is exempted. Test code can have intermittent warnings (e.g. `clippy::float_cmp` in numerical tests, `clippy::cast_possible_truncation` in fixture builders) and `#[expect]` would itself produce noise.

**Alternatives considered:**

- *Bulk `s/allow/expect/`.* Rejected: misses the chance to delete dead suppressions and to add reasons. The whole point of `#[expect]` is that it's tied to a specific reason.
- *Crate-level `#![allow(clippy::pedantic)]` to defer pedantic warns.* Rejected — the team agreed in `docs/CODE_REVIEW.md` §3 that pedantic adds real value. Each pedantic warning gets fixed or has a per-site `#[expect]` with a reason.

### 3. Deferred-engineering suppressions name the follow-up

**Decision:** When this change cannot fix a warning because the fix requires real engineering work, suppress it inline with:

```rust
#[expect(clippy::<lint>, reason = "addressed in <follow-up-change-name>")]
```

The follow-up names are stable (defined in `docs/temp_IMPL_PLAN.md`):

- `fix-production-unwraps-and-panics` — for the 11 production unwraps in `vault_sync.rs:3380`, `mcp/server.rs:1793/1880/1892/1990`, and `inference.rs` (×6 mutex locks).
- `split-vault-sync-module` — for `clippy::large_enum_variant` on `VaultSyncError`.
- `collapse-search-fts-api-surface` — for `dead_code` allows on un-namespaced FTS variants and the two `clippy::too_many_arguments` in `core/fts.rs`.

When the follow-up lands, its first commit removes the suppressions, and clippy starts surfacing the underlying issues that the follow-up then fixes.

**Alternatives considered:**

- *Track in a TODO.md or GitHub issues.* Rejected — the in-source `#[expect]` annotation is greppable, mechanically verifiable, and impossible to drift. The reason-naming convention encodes the issue tracker in the source.

### 4. CI gate: tighten what's already there, don't restructure

**Decision:** Edit `.github/workflows/ci.yml::check` in place to add `--all-features --locked` to the default-channel clippy invocation, and `--locked` to the online-channel invocation. Keep the existing job structure. Both channels stay enforced.

```yaml
- name: Cargo clippy (default / airgapped channel)
  run: cargo clippy --all-targets --all-features --locked -- -D warnings

- name: Cargo clippy (online channel)
  run: cargo clippy --all-targets --no-default-features --features bundled,online-model --locked -- -D warnings
```

`--all-features` triggers a build of every feature combination; for Quaid this means the airgapped invocation also exercises `online-model` code paths gated by `cfg(feature = "online-model")`. Combined with the dedicated online-channel invocation, both feature sets get linted. `--locked` ensures `Cargo.lock` reflects `Cargo.toml`.

**Alternatives considered:**

- *Add a separate `lints` job.* Rejected — the existing `Check` job is the right place. Splitting adds complexity without adding signal.
- *Drop the online-channel clippy invocation now that `--all-features` covers it.* Tempting but rejected: the two invocations exercise *different* feature combinations (`default = bundled,embedded-model` vs `bundled,online-model`), and a regression in one isn't necessarily caught by the other. Keep both.

## Risks / Trade-offs

- **Risk:** Pedantic lints surface false positives that cost engineering time to triage. **Mitigation:** Each false positive becomes a per-site `#[expect(...)]` with a reason. If the false-positive rate is unmanageable, drop `pedantic` from the table — that decision becomes evidence-driven, not pre-emptive.
- **Risk:** `--all-features` doubles compile time on the `Check` job. **Mitigation:** Cache hits dominate after warm-up; the dedicated `release-macos-preflight` job already does multi-channel builds, so the extra cost on `Check` is bounded. Measure on the first three PRs and revisit if it pushes wall-clock past 10 minutes.
- **Risk:** Some `#[allow]` annotations might suppress warnings that are no longer fired, but identifying them mechanically requires running the full clippy set first. **Mitigation:** Run `cargo clippy --all-targets --all-features` after the `Cargo.toml` table lands (without migrating any `#[allow]`s yet). Diff the warning list against current `#[allow]` sites — anything not in the diff is a candidate for deletion. Then migrate the rest.
- **Trade-off:** This change blocks no merges by default but makes future merges stricter. That's the intent — the cost of "I have to fix one more warning before merge" is much less than the cost of `unwrap_used` panics in production.
- **Risk:** A nightly-only clippy lint that landed in stable post-1.81 fires unexpectedly on contributors using older toolchains. **Mitigation:** CI uses `dtolnay/rust-toolchain@stable` which always tracks stable. If contributor pain materializes, add a `rust-toolchain.toml` pin in a follow-up.

## Migration Plan

Single PR, multi-commit. Suggested commit sequence:

1. **Add the lint tables to `Cargo.toml`** with `#[allow]` migrations *not yet done*. Run `cargo clippy --all-targets --all-features --locked` locally; capture the full warning list.
2. **Migrate `#[allow]` annotations** file-by-file. For each file: review every annotation, decide delete-vs-migrate-vs-defer, commit. 16 files, so ~16 commits. Each commit's diff is small and reviewable.
3. **Suppress deferred warnings** with `#[expect(...)]` + reason naming the follow-up. Verify `cargo clippy ... -- -D warnings` passes after each commit.
4. **Tighten CI** in `.github/workflows/ci.yml::check`. Verify both channels green in CI.
5. **Run `cargo doc --no-deps`** locally to confirm `missing_docs` warnings are accounted for (most will trigger; that's expected — proposal #7 fixes them, so they get `#[expect(missing_docs)]` with `reason = "addressed in add-crate-and-public-api-docs"` for now, OR the warning is downgraded to `allow` at crate level for the duration. The latter is simpler. Decide during implementation.)

Rollback: `git revert` of the PR. The follow-up proposals can still proceed without this scaffolding, just with less mechanical leverage.
