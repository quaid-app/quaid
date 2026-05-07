## ADDED Requirements

### Requirement: `Cargo.toml` declares the canonical Rust and Clippy lint set

The repository SHALL declare its lint configuration in `Cargo.toml` so that every contributor's local `cargo clippy` invocation matches what CI enforces. The configuration SHALL include:

- a `[lints.rust]` table with at minimum `unsafe_code = "forbid"`, `missing_docs = "warn"`, and `unreachable_pub = "warn"`.
- a `[lints.clippy]` table with `clippy::all = { level = "warn", priority = -1 }`, `clippy::pedantic = { level = "warn", priority = -1 }`, and explicit `warn`-level entries for `unwrap_used`, `expect_used`, `panic`, `print_stdout`, `redundant_clone`, `needless_collect`, and `large_enum_variant`.

Adding a new lint to the table SHALL be a code-review event, not a silent change — the table is the authoritative list of "what we care about."

#### Scenario: Fresh `cargo clippy` against `Cargo.toml` honors the table

- **WHEN** a contributor runs `cargo clippy --all-targets --all-features --locked` against the current `main`
- **THEN** the lints declared in `[lints.rust]` and `[lints.clippy]` are applied, including `unwrap_used = "warn"` and `large_enum_variant = "warn"`

#### Scenario: `unsafe_code` is forbidden, not warned

- **WHEN** a contributor introduces an `unsafe` block in `src/` and runs `cargo build`
- **THEN** the build fails with `unsafe code is forbidden by Cargo.toml [lints.rust]`

#### Scenario: Pedantic warns are downgrade-able per file

- **WHEN** a `pedantic` lint fires on a specific function and the team accepts it as a known false positive
- **THEN** that function MAY be annotated with `#[expect(clippy::<lint>, reason = "...")]` rather than removing the `pedantic` warn from `Cargo.toml`

### Requirement: All in-source lint suppressions use `#[expect(...)]` with a reason

Every lint suppression in `src/` SHALL use `#[expect(...)]` rather than `#[allow(...)]`, and SHALL be paired with a `reason = "..."` argument or an immediately-preceding `// reason: ...` line. This ensures that when the underlying lint stops firing — because the code was refactored or the lint was relaxed — the suppression itself becomes a warning, preventing dead annotations from accumulating.

The repository's pre-existing `#[allow(...)]` annotations (audit at change time: 46 occurrences across 16 files) SHALL be migrated to `#[expect(...)]` as part of the change that lands this requirement.

`#[allow(...)]` MAY remain in test code (`#[cfg(test)]` modules and `tests/`) where the suppressed lint legitimately fires intermittently across compilations and `#[expect(...)]` would itself produce noise.

#### Scenario: Production code uses `#[expect]`

- **WHEN** a contributor adds a new lint suppression to a function in `src/core/`
- **THEN** the suppression is `#[expect(clippy::<lint>, reason = "...")]`, not `#[allow(...)]`, and the change is rejected in code review otherwise

#### Scenario: Stale suppression surfaces as a warning

- **WHEN** a refactor removes the code that used to trigger a clippy lint while the `#[expect(clippy::<lint>)]` annotation remains
- **THEN** the next `cargo clippy` run emits an "unfulfilled `#[expect]`" warning naming the dead suppression, and CI fails because warnings are denied

#### Scenario: Test-side `#[allow]` is permitted

- **WHEN** a `#[cfg(test)]` module in `src/` contains `#[allow(clippy::unwrap_used)]` for a test fixture
- **THEN** the suppression is permitted without a reason annotation

### Requirement: CI enforces clippy and rustfmt with denied warnings

The repository's CI workflow SHALL fail any pull request whose checked-out tree produces a clippy warning, an rustfmt diff, or a `Cargo.lock` mismatch under the project's declared lint set. The enforcement SHALL run on at least one ubuntu-latest job for every PR, against every supported feature channel.

The minimum gate SHALL include:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings` for the default (airgapped) feature channel
- `cargo clippy --all-targets --no-default-features --features bundled,online-model --locked -- -D warnings` for the online-model channel

The `--locked` flag SHALL be present so a stale `Cargo.lock` cannot mask a dependency change.

#### Scenario: Pull request with a new clippy warning fails CI

- **WHEN** a contributor opens a PR that introduces a new `clippy::redundant_clone` warning in `src/`
- **THEN** the `Check` job in `.github/workflows/ci.yml` fails with `error: warnings emitted` and the PR is blocked from merge

#### Scenario: Both feature channels are gated

- **WHEN** a contributor adds code that warns only when the `online-model` feature is enabled
- **THEN** the second clippy invocation in the CI `Check` job (the one that compiles with `--no-default-features --features bundled,online-model`) fails

#### Scenario: Stale `Cargo.lock` fails CI

- **WHEN** a contributor edits `Cargo.toml` to bump a dependency version but forgets to update `Cargo.lock`
- **THEN** `cargo clippy --locked` fails because the lockfile is out of date, before any lint runs

### Requirement: Engineering-work warnings are tracked, not silently allowed

When this change lands, some clippy warnings — notably the production `unwrap` panics at `src/core/vault_sync.rs:3380`, `src/mcp/server.rs:1793/1880/1892/1990`, and the six mutex `lock().unwrap()` cases in `src/core/inference.rs` — require engineering work to fix and are deferred to a named follow-up proposal. Each such warning SHALL be suppressed inline with `#[expect(clippy::<lint>, reason = "addressed in <follow-up-change-name>")]` so a `grep` for the follow-up name surfaces every site, and so the follow-up's first commit can simply remove the suppressions.

A blanket `#[allow(...)]` at file or crate level SHALL NOT be used to defer this work.

#### Scenario: Deferred unwrap is annotated, not allowed

- **WHEN** the lint set lands and `src/core/vault_sync.rs:3380`'s production unwrap is not yet fixed
- **THEN** the unwrap line carries `#[expect(clippy::unwrap_used, reason = "addressed in fix-production-unwraps-and-panics")]` directly above it, and `cargo clippy -- -D warnings` passes

#### Scenario: Follow-up proposal removes deferred annotations

- **WHEN** the `fix-production-unwraps-and-panics` change replaces the unwrap with proper error handling
- **THEN** the `#[expect(clippy::unwrap_used, reason = "addressed in fix-production-unwraps-and-panics")]` annotation is also removed, and `cargo clippy` continues to pass because the `#[expect]` is no longer dead

#### Scenario: Crate-wide `#[allow]` is rejected

- **WHEN** a contributor proposes adding `#![allow(clippy::unwrap_used)]` to `src/lib.rs` to silence multiple sites at once
- **THEN** code review rejects the change in favor of per-site `#[expect(...)]` with reasons
