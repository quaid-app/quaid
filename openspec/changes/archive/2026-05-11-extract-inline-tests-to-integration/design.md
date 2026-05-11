## Context

The Quaid crate has six production files where the bottom-of-file
`#[cfg(test)] mod tests { ... }` block is between 1,062 and 6,596 lines
long. In aggregate that is ~20,000 lines of test code embedded in `src/`
(roughly half the crate's `src/` LOC). The
[code review](../../../docs/CODE_REVIEW.md) §1.5 and §4.2 identifies this
as the single largest organizational debt in the codebase.

The follow-up proposals
([temp_IMPL_PLAN.md](../../../docs/temp_IMPL_PLAN.md) #4 and #5) plan to
split [src/core/vault_sync.rs](../../../src/core/vault_sync.rs) and
[src/mcp/server.rs](../../../src/mcp/server.rs) into multiple modules.
Those splits cannot be reviewed honestly while the diff is dominated by
test code that just-so-happens to live in the same file. They cannot be
bisected usefully either, because every commit in the split-up sequence
also moves test fixtures around. Pulling tests out first, in a single
commit per source file with `cargo test` numbers held constant, makes
the subsequent module splits a pure production-code diff.

The crate already has a healthy `tests/` directory (~40 files using
`tests/common/` for shared helpers), so the destination pattern is
established and well-understood by contributors.

## Goals / Non-Goals

**Goals:**

- After this change, no production source file in `src/` has a single
  `mod tests { ... }` block over ~500 lines. The big block at the bottom
  of each of the six files is gone or much smaller.
- `cargo test` passes at every commit boundary. The test count at
  HEAD is `>=` the test count at the parent commit, never less.
- Each source-file migration is a single, independently revertable
  commit. Bisect through the series stays meaningful.
- New `tests/*.rs` files are split by *feature*, not 1:1 with the source
  file they came from. Each is ≤1,500 LOC per the
  [§10 budget](../../../docs/CODE_REVIEW.md).
- Inline tests that survive the migration (white-box tests that need
  private items) are annotated with their reason, so the next reviewer
  knows the inline test is intentional, not residue.
- The rule — public-API tests under `tests/`, white-box tests inline —
  becomes a written spec the team can point at in code review.

**Non-Goals:**

- No production code is moved, renamed, split, or refactored. Visibility
  modifiers (`pub`, `pub(crate)`) are not changed. If a test cannot be
  moved without making something public, it stays inline.
- No test logic is rewritten. Test bodies move verbatim; only `use`
  paths are adjusted.
- Module splits of `vault_sync.rs` and `mcp/server.rs` are explicitly
  tracked as separate proposals (#4, #5) and are out of scope here.
- New tests are not added. Coverage gaps surfaced during the migration
  are noted but addressed separately.
- Multi-assertion test cleanup ([§4.2](../../../docs/CODE_REVIEW.md))
  is out of scope. Tests move as-is.

## Decisions

### 1. Classification rule: "uses only `pub` items" → moveable

A test moves to `tests/` iff every identifier it references from this
crate is reachable through the crate's public API (i.e. compiles when
the test is built as `cargo test --test <name>` from `tests/*.rs`).
Otherwise it stays inline.

The mechanical procedure per source file:

1. Cut the entire `mod tests { ... }` block to a scratch file.
2. Drop it into a fresh `tests/<area>_scratch.rs` with the appropriate
   `use quaid::...;` lines guessed.
3. `cargo test --test <area>_scratch` and let the compiler enumerate
   every reference that is not `pub`.
4. Tests that compile move out (split into per-feature `tests/*.rs`
   files at the end). Tests that fail to compile due to private-item
   access go *back* into the inline `mod tests` and gain a
   `// reason: white-box; needs <private-item>` comment.
5. Delete the scratch file.

**Alternative considered:** add `pub(crate)` to the items the failing
tests need, then move them all out. **Rejected:** widening internal
visibility just to relocate tests is a behavior change disguised as
a refactor. It also expands the seam that proposals #4 and #5 will have
to navigate. The cost of a few inline tests per file is much smaller
than the cost of an artificially fattened `pub(crate)` surface.

### 2. Split by feature, not by source file

Naming pattern: `tests/<area>_<concern>.rs`, where `<area>` matches an
existing `tests/` prefix when one applies (`vault_sync_*`,
`reconciler_*`, `mcp_server_*`, `cli_collection_*`, `cli_put_*`, `db_*`)
and `<concern>` is a feature/scenario, not a code symbol.

Example for `vault_sync.rs`'s 6,596-line block: 6–10 files such as
`vault_sync_ipc.rs`, `vault_sync_restore.rs`, `vault_sync_watcher.rs`,
`vault_sync_session.rs`, `vault_sync_serialize.rs`,
`vault_sync_handshake.rs`. Each ≤ 1,500 LOC.

**Alternative considered:** one file per source file (`tests/vault_sync.rs`).
**Rejected:** the whole point of §10's 1,500-LOC budget is "split by
feature, not by file under test." A single 6,000-line `tests/vault_sync.rs`
just relocates the monolith.

The exact carve-up of features is decided during the migration commit
for each source file — this design does not pre-name every file because
the tests' own `mod` substructure will usually point to a natural split.

### 3. One commit per source file

Series:

1. Setup — add `tests/common/` helpers if any are pre-known. (Optional;
   skip if no shared helper is needed yet.)
2. `db.rs` migration — smallest test block (1,062 LOC), validates the
   approach.
3. `put.rs` migration.
4. `collection.rs` migration.
5. `mcp/server.rs` migration.
6. `reconciler.rs` migration.
7. `vault_sync.rs` migration — largest, last, after the procedure is
   well-rehearsed.
8. `tests/collection_cli_truth.rs` split — separate commit, same
   per-feature rule applied to a `tests/` file.

Each commit:

- Carries the verbatim `cargo test 2>&1 | tail -50` output from
  `HEAD~1` and `HEAD` in the commit body, showing the test counts
  match-or-grow.
- Touches only the one source file being migrated and the new
  `tests/*.rs` files it produces (plus `tests/common/` if shared
  helpers were extracted in this commit).

**Alternative considered:** one giant commit. **Rejected:** unbisectable.
**Alternative considered:** one commit per *new* `tests/*.rs` file.
**Rejected:** the source file's `mod tests` block has to disappear
atomically with the new files appearing — splitting that across many
commits leaves intermediate states where tests are duplicated and `cargo
test` runs them twice. One commit per source file is the right atomic
unit.

### 4. Verification: test count must match or grow at every commit

Concrete check, run before and after each migration commit:

```sh
cargo test 2>&1 \
  | grep -E '^test result: (ok|FAILED)' \
  | awk '{ p += $4; f += $6 } END { printf "passed=%d failed=%d\n", p, f }'
```

`failed` must be `0`. `passed` at HEAD must be `>= passed` at HEAD~1.
Drift downward signals a silently-dropped test and blocks the commit.

`passed` is allowed to *grow* across a migration commit. This happens
naturally when an inline test was previously aggregated into one
top-level `#[test] fn` and is now expressed as multiple integration
tests, or when a `#[cfg(test)] mod` previously gated by some feature
flag becomes always-built. Either is fine.

**Alternative considered:** strict equality on test count. **Rejected:**
forces contortions when a perfectly good split happens to fan out to
more `#[test] fn` items.

### 5. Helpers go in `tests/common/`

Rust's integration-test layout treats every `tests/*.rs` file as its own
crate. Any helper used by 2+ migrated test files lives in
`tests/common/` and is pulled in via `mod common;` in each consumer.
This matches the existing pattern
([tests/common/mod.rs](../../../tests/common/mod.rs),
[tests/common/subprocess.rs](../../../tests/common/subprocess.rs)) — no
new convention introduced.

Helpers stay in the source file's inline `mod tests` if they are only
used by white-box tests that did not move.

### 6. `use` path adjustments are the only allowed test-body edits

Inside the moved test bodies, the only edits are:

- `use super::*;` → explicit `use quaid::<area>::<item>;`
- `use crate::...;` → `use quaid::...;`
- Removing `super::` qualifiers in expressions, replacing with
  `quaid::<area>::`

Any other change (renaming a variable, splitting an assertion,
re-ordering, "while we're here" cleanups) is rejected at review and
must go to a follow-up commit. This keeps the migration mechanical and
keeps the diff legible.

### 7. White-box residue is annotated, not hidden

Every `#[test] fn` that stays inline because it touches a private item
gets a one-line comment immediately above it:

```rust
// reason: white-box; needs `<private_item>`
#[test]
fn foo_internal_invariant() { ... }
```

When the inline `mod tests` block has more than ~5 such tests, a single
module-level comment at the top of `mod tests` is acceptable instead of
per-fn comments, listing the private items being exercised.

This is the signal to a future reader (or to proposal #4 / #5) that the
inline test is *load-bearing*, not residual.

## Risks / Trade-offs

- **[Risk] Compile-time dependency surfaces only after the move.** A
  test may compile inline because of a glob `use super::*;` that pulled
  in something `pub(crate)`. **Mitigation:** the scratch-file procedure
  in Decision 1 surfaces these at the cut step, not at PR-review time.
  Each failed compile sends the test back inline with an annotation.

- **[Risk] `cargo test` link time grows.** Each new `tests/*.rs` file
  is its own integration test crate with its own linker invocation.
  **Mitigation:** acceptable trade-off; the test count was already too
  large for a single integration crate. Developers gain
  `cargo test --test vault_sync_ipc` for fast iteration on a single
  feature.

- **[Risk] Tests that depended on `#[cfg(test)]`-only items in the
  source file (test-only helper functions inline in production source)
  break when moved.** **Mitigation:** these are white-box dependencies
  by definition — the test stays inline.

- **[Risk] Hidden test-ordering coupling.** Two tests that lived in the
  same `mod tests` and shared module-level static state (via `OnceLock`
  or similar) may behave differently when split across two
  `tests/*.rs` files (separate processes per integration crate).
  **Mitigation:** the migration moves tests verbatim into one
  `tests/*.rs` file per *feature* — co-located tests stay co-located.
  Cross-feature shared state was already an integration-test concern
  and would have been just as broken across the existing 40
  `tests/*.rs` files.

- **[Risk] Reviewer fatigue.** Six commits, each touching ~2,000 LOC of
  test code, plus a seventh for `collection_cli_truth.rs`. **Mitigation:**
  per-commit `cargo test` numbers in the commit body are the primary
  review signal; reviewers do not have to re-read every moved test
  because the rule "no body edits except `use` paths" makes the diff
  mechanically verifiable with a tool that strips `use` lines and
  diffs the remainder.

- **[Trade-off] `pub` API stability assumed.** The migration uses what
  is `pub` *today*. If proposal #4 or #5 later changes a public path
  while splitting a module, the migrated test files will need their
  `use` lines updated. This is expected and exactly the kind of churn
  those proposals already plan for. It is strictly better than the
  alternative — leaving the tests inline, where the same module split
  would also have to relocate them.

- **[Trade-off] No test cleanup while we're here.** §4.2's
  multi-assertion cleanup is appealing to do now, but bundling it with
  a relocation makes the diff unreviewable. Defer to a follow-up.
