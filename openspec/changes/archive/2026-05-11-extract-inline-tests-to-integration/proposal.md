## Why

Six production source files in `src/` have grown past 1,000 LOC largely
because of a single bottom-of-file `#[cfg(test)] mod tests { ... }`. The worst
offender, [src/core/vault_sync.rs](../../../src/core/vault_sync.rs), is 12,504
lines total: production code runs to line 5,908, then a 6,596-line test
block starts at line 5,909. Two follow-up refactors that the
[code review](../../../docs/CODE_REVIEW.md) calls out as the largest
quality wins — proposal #4 (`split-vault-sync-module`) and proposal #5
(`split-mcp-server-by-tool-group`) — cannot start cleanly until the test
mass is moved out, because moving 6k lines of tests *and* splitting the
production module in the same change makes the diff impossible to review
and a bisect useless. This change is the pure-mechanical unblocking step.

## What Changes

- Move public-API tests (those touching only `pub` items) out of inline
  `mod tests` blocks in six monolith `src/` files into per-feature
  integration test files under `tests/`. One `tests/` file per concern,
  not one per source file.
- Keep white-box tests (those reaching into private items) inline in
  `src/`. Each remaining inline test is annotated
  `// reason: white-box; needs <private-item>` so a future reader knows
  why it didn't move.
- Split [tests/collection_cli_truth.rs](../../../tests/collection_cli_truth.rs)
  (108 KB) by command into separate `tests/cli_collection_<verb>.rs` files,
  using the same per-feature split rule as above.
- Lift any helpers shared across the new test files into `tests/common/`,
  the existing pattern already used by `tests/common/mod.rs` and
  `tests/common/subprocess.rs`.
- Ship the migration as one commit per source file (six source-file
  commits + one `collection_cli_truth.rs` commit) so `git bisect` stays
  useful and each step is independently reversible. `cargo test` is run
  before and after every commit; the test count must match or grow,
  never shrink.
- Codify the resulting rule as a spec so this is the last time we have
  to debate "where do tests live."

Explicitly **out of scope**: no production code is moved, renamed, or
split in this change. No `pub` visibility is added or removed. No test
logic is rewritten — the `#[test] fn` bodies move verbatim, with only the
`use` lines adjusted from `super::` / `crate::` paths to the public
crate path. Module-level production splits are tracked as proposals #4
and #5.

## Capabilities

### New Capabilities
- `test-organization`: Codifies where Rust tests live in this crate
  (public-API tests under `tests/`, white-box tests inline in `src/`),
  the annotation requirement for inline tests, the per-file LOC budget
  for `tests/*.rs`, and the per-feature split rule.

### Modified Capabilities
<!-- None — no requirement-level behavior changes; this is a pure
mechanical refactor of test placement. -->

## Impact

- **Affected source files** (test blocks shrink or disappear; production
  logic untouched). LOC numbers are total file size today / size of the
  bottom `mod tests` block:
  - [src/core/vault_sync.rs](../../../src/core/vault_sync.rs) — 12,504 / 6,596 (`mod tests` at line 5909)
  - [src/core/reconciler.rs](../../../src/core/reconciler.rs) — 7,403 / 4,285 (`mod tests` at line 3119)
  - [src/mcp/server.rs](../../../src/mcp/server.rs) — 5,903 / 3,888 (`mod tests` at line 2016)
  - [src/commands/collection.rs](../../../src/commands/collection.rs) — 4,269 / 2,708 (`mod tests` at line 1562)
  - [src/commands/put.rs](../../../src/commands/put.rs) — 3,246 / 1,864 (`mod tests` at line 1383)
  - [src/core/db.rs](../../../src/core/db.rs) — 2,028 / 1,062 (`mod tests` at line 967)
- **Affected test files**: many new files added under
  [tests/](../../../tests/) (estimated 20–30 new files, each ≤1,500 LOC
  per the §10 budget). One existing file split:
  [tests/collection_cli_truth.rs](../../../tests/collection_cli_truth.rs).
- **Build artifacts**: each new `tests/*.rs` file is a separate
  integration test binary, so `cargo test` link time grows. Acceptable
  trade-off; offset by the option to `cargo test --test <name>` a single
  feature.
- **APIs**: none. No `pub` items are added; tests only consume what is
  already public.
- **Dependencies**: none added. `tests/common/` may grow; no new crates.
- **Downstream consumers (proposal #4, #5)**: unblocked. After this
  change, [src/core/vault_sync.rs](../../../src/core/vault_sync.rs) is
  ~700 lines of production code instead of 6,596, making the
  module-split diff reviewable.
- **CI / review workflow**: file-size budgets in `docs/CODE_REVIEW.md`
  §10 become enforceable for the first time without hitting test code.
