## ADDED Requirements

### Requirement: Public-API tests SHALL live under `tests/`

A test that exercises only items reachable through the crate's public
API (i.e. items declared `pub` and re-exported from `lib.rs`) MUST be
placed in a file under `tests/`. It MUST NOT be placed inside a
`#[cfg(test)] mod tests { ... }` block in any `src/` source file.

The check is operational: if the test compiles when extracted to a fresh
`tests/<name>.rs` file using only `use quaid::...;` paths (no `super::`
or `crate::` references), it is a public-API test and SHALL live in
`tests/`.

#### Scenario: Test references only `pub` items

- **WHEN** a `#[test] fn` body uses identifiers that are all reachable
  through the crate's public API
- **THEN** the test SHALL be defined in a `tests/*.rs` file and SHALL
  NOT be defined in any `src/` file

#### Scenario: Test compiles cleanly as integration test

- **WHEN** the test body, with `super::*`/`crate::*` imports rewritten
  to `quaid::*`, builds under `cargo test --test <name>` from
  `tests/<name>.rs`
- **THEN** the test SHALL be located at `tests/<name>.rs` (or another
  per-feature file under `tests/`), not inline in `src/`

### Requirement: White-box tests SHALL stay inline and SHALL be annotated

A test that requires access to a non-`pub` item — including
`pub(crate)`, `pub(super)`, module-private items, or `#[cfg(test)]`-only
helpers defined in the source file — MUST remain in a
`#[cfg(test)] mod tests { ... }` block inside the source file that
exposes the item. Each such test SHALL carry a comment immediately
above it (or above the surrounding `mod tests` block, if more than
roughly five tests share the same reason) of the form:

```
// reason: white-box; needs <private-item-or-list>
```

Visibility modifiers MUST NOT be widened (e.g. private → `pub(crate)`,
or `pub(crate)` → `pub`) for the sole purpose of making a test movable.
If a test can only move out by widening visibility, it stays inline.

#### Scenario: Test calls a private function

- **WHEN** a `#[test] fn` invokes a function that is not `pub`
- **THEN** the test SHALL stay inline in the source file and SHALL be
  annotated with `// reason: white-box; needs <function-name>`

#### Scenario: Tempting visibility widening rejected

- **WHEN** an author proposes changing a `pub(crate)` item to `pub`
  solely so a test can move to `tests/`
- **THEN** the change SHALL be rejected and the test SHALL stay inline

### Requirement: Migrated test files SHALL be split by feature, not by source file

When the public-API tests previously inline in a single `src/` file are
moved to `tests/`, they MUST be split across multiple `tests/*.rs`
files, one per feature or scenario area, rather than collected into a
single `tests/<source-file>.rs` mirror file.

Each new `tests/*.rs` file MUST be ≤ 1,500 LOC. The 1,500 LOC budget is
the hard ceiling for `tests/*.rs` files set by the
[code review](../../../docs/CODE_REVIEW.md) §10.

File naming SHALL follow the pattern `tests/<area>_<concern>.rs`, where
`<area>` is the conceptual subsystem under test (e.g. `vault_sync`,
`mcp_server`, `cli_collection`) and `<concern>` is the feature or
scenario being exercised (e.g. `ipc`, `restore`, `watcher`,
`session`).

#### Scenario: Large `mod tests` block migration

- **WHEN** an inline `mod tests { ... }` block of ≥ 1,500 LOC is moved
  to `tests/`
- **THEN** the resulting `tests/*.rs` files SHALL be ≥ 2 in number, each
  ≤ 1,500 LOC, named by feature

#### Scenario: Single-file mirror rejected

- **WHEN** the migration of a 6,000-LOC `mod tests` block proposes a
  single `tests/<source>.rs` of similar size
- **THEN** the migration SHALL be rejected as violating the
  per-feature-split rule

### Requirement: Shared test helpers SHALL live under `tests/common/`

When two or more `tests/*.rs` files share helper code (fixtures, setup
utilities, custom assertion functions), the shared code SHALL live in
`tests/common/` and SHALL be imported into each consumer via
`mod common;` at the top of the consumer file. Inline copies of shared
helpers across multiple `tests/*.rs` files are not permitted.

A helper used only by inline white-box tests MAY remain inline in the
source file's `mod tests` block.

#### Scenario: Helper used by multiple integration test files

- **WHEN** a helper function is needed by ≥ 2 `tests/*.rs` files
- **THEN** the helper SHALL be defined in `tests/common/` and SHALL be
  consumed via `mod common;`

#### Scenario: Helper used only inline

- **WHEN** a helper function is used only by inline white-box tests
- **THEN** the helper MAY stay in the source file's `mod tests` block

### Requirement: Test-relocation commits SHALL preserve `cargo test` results

A commit that moves tests from `src/` to `tests/`, or that splits an
existing `tests/*.rs` file, MUST satisfy:

1. `cargo test` exits with code 0 (no failed tests) at the commit.
2. The total count of passing tests at the commit is greater than or
   equal to the count at the immediate parent commit.
3. The commit body or PR description records the before/after pass
   counts as evidence.

The commit MUST NOT modify production source files (anything in `src/`)
beyond the deletion of the test code being moved out and any
adjustments to `#[cfg(test)]` annotations on remaining inline items.
Specifically: function bodies, type definitions, visibility modifiers,
and module structure of production code SHALL be unchanged.

#### Scenario: Migration commit with passing tests

- **WHEN** a commit moves tests out of `src/foo.rs` to `tests/foo_*.rs`
- **THEN** `cargo test` SHALL pass at the commit, AND the total passing
  test count SHALL be `>=` the parent commit's count

#### Scenario: Migration commit drops a test silently

- **WHEN** a commit's `cargo test` passing count is *less* than its
  parent's
- **THEN** the commit SHALL be rejected; the missing test must be found
  and re-included before the commit is accepted

#### Scenario: Migration commit edits production logic

- **WHEN** a commit purports to move tests but also modifies the body
  of a `pub fn` or changes a visibility modifier in `src/`
- **THEN** the commit SHALL be rejected; the production change must be
  factored into a separate commit

### Requirement: Each migrated source file SHALL be its own commit

The migration of inline tests out of any one source file SHALL be a
single, atomic, self-contained commit. The series of commits SHALL be
ordered such that `git bisect` between any two points yields a working
build at every step.

The migration of `tests/collection_cli_truth.rs` (split by command into
multiple `tests/cli_collection_*.rs` files) SHALL be its own commit
under the same rule.

#### Scenario: Multiple source files in one commit

- **WHEN** a commit moves inline tests out of two or more source files
  simultaneously
- **THEN** the commit SHALL be rejected; the migration SHALL be split
  into one commit per source file

#### Scenario: Bisect remains useful

- **WHEN** any two commits in the migration series are checked out
  independently
- **THEN** `cargo build` and `cargo test` SHALL succeed at each commit
