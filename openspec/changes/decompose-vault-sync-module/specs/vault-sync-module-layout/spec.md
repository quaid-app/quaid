## ADDED Requirements

### Requirement: vault_sync is a directory module with a fixed submodule layout

The `crate::core::vault_sync` module SHALL be implemented as a directory
module at `src/core/vault_sync/` containing the following submodules,
each owning a single concern:

| File | Owns |
|---|---|
| `mod.rs` | re-exports + `ensure_unix_platform` helpers |
| `error.rs` | `VaultSyncError` parent enum |
| `session.rs` | session register / unregister / heartbeat / sweep |
| `ownership.rs` | live collection owner + lease acquire / release |
| `write_lock.rs` | `with_write_slug_lock` + write-dedup helpers |
| `ipc/mod.rs` | `ServeRuntime`, `IpcSocketLocation` |
| `ipc/socket.rs` | socket auth + permission checks (`cfg(unix)`) |
| `ipc/handler.rs` | `handle_ipc_client` / `accept_ipc_clients` |
| `watcher.rs` | `CollectionWatcherState`, `WatchEvent`, `WatchBatchBuffer` |
| `restore.rs` | `begin_restore` / `finalize_pending_restore` / `RestoreManifest` |
| `recovery.rs` | `RecoveryInProgressGuard` + post-rename sentinels |
| `precondition.rs` | `FsPreconditionInspection` + `check_fs_precondition` |

A single-file `src/core/vault_sync.rs` SHALL NOT exist. New code that
extends `vault_sync` SHALL be placed in the existing submodule whose
concern it matches; if no submodule owns the concern, a new
submodule SHALL be added rather than overloading an existing one.

#### Scenario: vault_sync.rs single-file form is removed

- **WHEN** the change lands
- **THEN** `src/core/vault_sync.rs` does not exist as a file
- **AND** `src/core/vault_sync/` exists as a directory
- **AND** the directory contains at minimum `mod.rs`, `error.rs`,
  `session.rs`, `ownership.rs`, `write_lock.rs`, `ipc/mod.rs`,
  `ipc/socket.rs`, `ipc/handler.rs`, `watcher.rs`, `restore.rs`,
  `recovery.rs`, `precondition.rs`

#### Scenario: New vault_sync code lives in a concern-matched submodule

- **WHEN** a future change extends vault_sync with a new function or
  type
- **THEN** the new item is placed in the submodule whose concern
  matches its purpose (e.g., a new IPC handler goes in `ipc/`, a new
  watcher field goes in `watcher.rs`)
- **AND** if no submodule matches, a new submodule is created with a
  named concern rather than overloading `mod.rs` or any existing file

### Requirement: Files under vault_sync/ obey an 800-LOC per-file budget

No file under `src/core/vault_sync/` SHALL exceed 800 lines (counted by
`wc -l`, including blank lines and comments). This budget applies to
production code files; any inline `#[cfg(test)] mod tests` block under
this directory MUST also fit within the same budget — long inline test
blocks SHALL be moved to `tests/vault_sync_*.rs` instead of growing the
file.

#### Scenario: Initial split holds the budget

- **WHEN** the change lands
- **THEN** `wc -l src/core/vault_sync/**/*.rs` reports no file with
  more than 800 lines

#### Scenario: A future change cannot grow a file past the budget without re-splitting

- **WHEN** a future edit would push any file under `src/core/vault_sync/`
  past 800 lines
- **THEN** the change instead extracts a new submodule (or moves test
  code to `tests/`) so the budget is preserved

### Requirement: Public surface of crate::core::vault_sync is preserved across the split

The public surface of `crate::core::vault_sync` SHALL be byte-for-byte preserved by this change. Every public item (`pub fn`, `pub struct`, `pub enum`, `pub type`, `pub const`) that was reachable as `crate::core::vault_sync::Foo` before the split SHALL remain reachable at the same path after the split, via re-exports in `mod.rs` if its definition has moved into a submodule. External crates and other modules MUST NOT need to update any `use crate::core::vault_sync::...` import as a result of this change.

#### Scenario: Existing call sites compile unchanged

- **WHEN** the change lands
- **AND** the 11 external call sites that today match
  `grep -rE "use crate::core::vault_sync"` are not edited
- **THEN** `cargo build` succeeds with zero unresolved-import errors
  attributable to the split

#### Scenario: Re-exports cover every previously top-level public item

- **WHEN** an external caller writes `use crate::core::vault_sync::Foo;`
  for any `Foo` that was a top-level `pub` item before the split
- **THEN** the import resolves successfully
- **AND** the resolved `Foo` is the same item (same `TypeId` for types,
  same definition for fns) as the one defined inside its new
  submodule

### Requirement: VaultSyncError uses parent + child enum composition

`VaultSyncError` SHALL be a parent enum that composes per-subsystem
child enums via `#[from]`-bearing `#[error(transparent)]` variants,
plus shared variants for cross-cutting concerns (`Sqlite`, `Io`,
`InvariantViolation`, etc.). A child enum SHALL exist for each
subsystem with non-trivial error variety: at minimum `IpcError`,
`RestoreError`, `ConflictError`, and `WatcherError`. Each child enum
SHALL be defined in the submodule that produces its variants
(`IpcError` in `ipc/`, `RestoreError` in `restore.rs`, etc.) — not
collected into a single `error.rs`. The parent `VaultSyncError`
itself lives in `error.rs`.

#### Scenario: Parent enum delegates to child enums

- **WHEN** a reader inspects the definition of `VaultSyncError`
- **THEN** at least four variants are `#[error(transparent)]
  #[from]`-style wrappers around child enums (`IpcError`,
  `RestoreError`, `ConflictError`, `WatcherError`)
- **AND** no IPC-specific, restore-specific, conflict-specific, or
  watcher-specific *leaf* variant lives directly on
  `VaultSyncError` — leaf variants for those subsystems live on the
  child enum

#### Scenario: Child enums live next to the code that produces them

- **WHEN** a reader looks for the definition of a child enum
- **THEN** it is found in the submodule that produces its variants
  (e.g., `IpcError` in `src/core/vault_sync/ipc/`, `RestoreError` in
  `src/core/vault_sync/restore.rs`)
- **AND** `error.rs` contains the parent `VaultSyncError` plus only
  shared/cross-cutting variants

#### Scenario: Existing match-arm callers still compile (possibly with one nesting level)

- **WHEN** a caller previously matched
  `if let Err(VaultSyncError::HashMismatch { … })`
- **THEN** the same caller — after the split — matches
  `if let Err(VaultSyncError::Conflict(ConflictError::HashMismatch { … }))`
  with semantically equivalent behaviour
- **AND** no formerly-matchable error case is silently dropped or
  re-routed to a different variant

### Requirement: Error variants use structured types for debug metadata, not pre-formatted Strings

Variants of `VaultSyncError` and its child enums SHALL NOT carry
`String` fields whose only purpose is to hold a `Display`-formatted
list of structured data. When a variant needs to surface a list of
paths, ids, or samples, the field SHALL be the structured type
(`Vec<PathBuf>`, `Vec<i64>`, `Vec<(PathBuf, …)>`, etc.); the variant's
`Display` implementation SHALL perform the join into human-readable
form. Specifically, `NewRootVerificationFailed`'s
`missing_samples` / `mismatched_samples` / `extra_samples` fields
SHALL be `Vec<PathBuf>`, not `String`.

#### Scenario: NewRootVerificationFailed exposes structured paths

- **WHEN** a caller pattern-matches on
  `VaultSyncError::Restore(RestoreError::NewRootVerificationFailed { missing_samples, … })`
- **THEN** `missing_samples` is `&Vec<PathBuf>` (or equivalent
  structured type), iterable without string parsing
- **AND** the same data, formatted via the variant's `Display`,
  produces a human-readable comma-joined message equivalent to what
  callers saw before the change

#### Scenario: map_vault_sync_error consumes structured data

- **WHEN**
  [src/mcp/server.rs](../../../src/mcp/server.rs)::`map_vault_sync_error`
  surfaces a `NewRootVerificationFailed` error to an MCP client
- **THEN** it iterates the structured `Vec<PathBuf>` fields directly
  without re-parsing a comma-separated string

### Requirement: Every file under vault_sync/ carries a module-level //! doc

Every production `.rs` file under `src/core/vault_sync/` SHALL begin with a one-paragraph `//!` module-level doc-comment that names the concern the file owns and any non-obvious invariant a reader needs before editing it. This requirement intentionally precedes a follow-up crate-wide `#![warn(missing_docs)]` lint: the lint becomes tractable to enable because every file in this module already complies.

#### Scenario: Each new file has a module doc

- **WHEN** the change lands
- **THEN** every file matching
  `src/core/vault_sync/**/*.rs` begins with at least one
  `//! …` line before any `use` or item declaration
- **AND** the doc paragraph is at least one full sentence describing
  the concern the file owns

### Requirement: Long functions are decomposed into named phases

The two longest production functions previously in `vault_sync.rs` SHALL be decomposed into named, single-concern helpers as follows:

- `start_serve_runtime` SHALL be split into at least three named
  phases: `bind_socket`, `register_session`, and `spawn_watcher`.
- `begin_restore` SHALL be split into at least three named phases:
  `validate_target`, `stage_pending`, and `register_manifest`.

The public callable shape of `start_serve_runtime` and `begin_restore`
SHALL be preserved (same name, same signature, same return type) —
the decomposition is purely internal.

#### Scenario: start_serve_runtime delegates to named phases

- **WHEN** a reader inspects the body of `start_serve_runtime`
- **THEN** the body is a short orchestrator that calls
  `bind_socket`, `register_session`, and `spawn_watcher` (or
  equivalently named functions covering the same concerns) in order
- **AND** none of those phase functions exceeds 100 lines

#### Scenario: begin_restore delegates to named phases

- **WHEN** a reader inspects the body of `begin_restore`
- **THEN** the body is a short orchestrator that calls
  `validate_target`, `stage_pending`, and `register_manifest` (or
  equivalently named functions covering the same concerns) in order
- **AND** none of those phase functions exceeds 100 lines

### Requirement: Behavioural regression net is the existing tests/vault_sync_*.rs suite

The `tests/vault_sync_*.rs` integration test suite SHALL pass unchanged before and after every commit in this change. The suite was added by the preceding `extract-inline-tests-to-integration` change. No test in that suite SHALL be modified, disabled, or deleted as part of this change — the suite is the regression gate that proves the split is behaviour-preserving.

#### Scenario: Tests pass at every commit

- **WHEN** any individual commit in this change is checked out
- **AND** `cargo test --test 'vault_sync_*'` is run
- **THEN** every test passes
- **AND** the count of passing tests is at least equal to the count
  on the parent commit (no test was silently removed)

#### Scenario: No test file is edited by this change

- **WHEN** the diff for this change is inspected
- **THEN** no file under `tests/vault_sync_*.rs` appears in the diff
  (excluding pure rename/move that preserves contents byte-for-byte,
  which is also disallowed by this change)
