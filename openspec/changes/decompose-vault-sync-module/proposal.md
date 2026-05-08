## Why

[src/core/vault_sync.rs](../../../src/core/vault_sync.rs) is the single
largest file in the crate. Once
[extract-inline-tests-to-integration](../extract-inline-tests-to-integration/proposal.md)
moves the 6,596 LOC inline test mass to `tests/`, the production module
is still ~5,909 LOC of unrelated concerns — IPC sockets, owner leases,
write locks, watcher state, restore manifests, recovery sentinels,
filesystem preconditions, and a 30+-variant kitchen-sink error enum —
all in one file. The
[code review](../../../docs/CODE_REVIEW.md) §1.3 calls this out as the
biggest single quality win available; §10 sets a 800-LOC per-file
budget that this module violates by 7×. The natural seams already
exist (each `cfg(unix)` block and each runtime-registry helper); this
change lands the split before downstream proposals — file-size lints
(§10), warn-on-missing-docs (§6.1), and any future feature work in this
module — try to operate on a 5,909-line target.

## What Changes

- Decompose [src/core/vault_sync.rs](../../../src/core/vault_sync.rs)
  into a directory module `src/core/vault_sync/` with the layout
  prescribed by §1.3:

  ```text
  src/core/vault_sync/
  ├── mod.rs        — re-exports + ensure_unix_platform helpers
  ├── error.rs      — VaultSyncError parent enum
  ├── session.rs    — register/unregister/heartbeat/sweep_stale_sessions
  ├── ownership.rs  — live_collection_owner / acquire_owner_lease /
  │                   release_owner_lease
  ├── write_lock.rs — with_write_slug_lock + write_dedup helpers
  ├── ipc/
  │   ├── mod.rs    — ServeRuntime, IpcSocketLocation
  │   ├── socket.rs — socket auth + permission checks (cfg(unix))
  │   └── handler.rs — handle_ipc_client / accept_ipc_clients
  ├── watcher.rs    — CollectionWatcherState, WatchEvent, WatchBatchBuffer
  ├── restore.rs    — begin_restore / finalize_pending_restore /
  │                   RestoreManifest
  ├── recovery.rs   — RecoveryInProgressGuard + post-rename sentinels
  └── precondition.rs — FsPreconditionInspection + check_fs_precondition
  ```

- Split `VaultSyncError` per §2.2: replace the single 30+-variant
  `enum VaultSyncError` with a parent enum that `#[from]`-composes
  child enums (`IpcError`, `RestoreError`, `ConflictError`,
  `WatcherError`) plus shared variants (`Sqlite`, `Io`,
  `InvariantViolation`). Each child enum lives next to the module that
  produces it. Pattern matching in callers becomes
  `if let Err(VaultSyncError::Conflict(ConflictError::HashMismatch …))`
  rather than fan-out across one flat enum.

- Replace `String`-typed debug metadata in error variants with
  structured types per §2.3. Specifically
  `NewRootVerificationFailed { missing_samples: String,
  mismatched_samples: String, extra_samples: String }` becomes
  `Vec<PathBuf>` for each, with `Display::fmt` doing the join. This
  unblocks
  [src/mcp/server.rs](../../../src/mcp/server.rs)::`map_vault_sync_error`
  from re-parsing pre-formatted strings to structurally surface the
  data.

- Extract the two longest in-module production functions per §5.3
  into named phases:
  - `start_serve_runtime` (227 lines) → `bind_socket` +
    `register_session` + `spawn_watcher`.
  - `begin_restore` (181 lines) → `validate_target` + `stage_pending` +
    `register_manifest`.

- Each new file gains a one-paragraph `//!` module doc summarising
  what lives there and what invariants it owns. (A follow-up proposal
  switches `missing_docs` from `allow` to `warn` across the public
  surface; this change is what makes that follow-up tractable.)

Explicitly **out of scope**: no behavioural change. No `pub` items are
added, removed, or renamed. The 11 external call sites
(`grep -rE "use crate::core::vault_sync"`) compile unchanged via
re-exports in `mod.rs` — every existing `crate::core::vault_sync::Foo`
import path is preserved. The error split is structural only — every
discriminant a caller can match on today remains matchable (possibly
through one nested level, e.g. `Conflict(ConflictError::HashMismatch …)`
instead of `HashMismatch …`); no error case is silently dropped or
re-routed. `tests/vault_sync_*.rs` (added by
`extract-inline-tests-to-integration`) is the regression net: it must
pass before and after every commit in this change.

## Capabilities

### New Capabilities
- `vault-sync-module-layout`: Codifies the structural invariants this
  refactor introduces and that downstream proposals will lean on — the
  module's directory layout, the 800-LOC per-file budget for files
  under `src/core/vault_sync/`, the public-surface-preservation
  invariant (re-exports stable across external import sites), the
  error-type composition pattern (parent `VaultSyncError` enum with
  `#[from]`-composed child enums per submodule), the
  no-`String`-formatted-debug-metadata rule for error variants, and
  the module-level `//!` doc requirement on every file.

### Modified Capabilities
<!-- None — no requirement-level behavior changes to vault-sync
itself. The vault-sync spec describes external behaviour (file_state
tracking, cold-start reconciliation, watcher semantics, restore
correctness); this change only reorganises the implementation that
satisfies those requirements, with no observable difference. The two
structural changes (error split, String→Vec<PathBuf> metadata) are
internal to the error type and do not alter any of the user-visible
error messages or recovery paths. -->

## Impact

- **Affected source files (split target)**:
  - [src/core/vault_sync.rs](../../../src/core/vault_sync.rs) — deleted;
    replaced by [src/core/vault_sync/](../../../src/core/vault_sync/)
    directory module with 11 sub-files. Each new file targets the
    300–800 LOC range; no file may exceed 800 LOC after the split
    (§10 budget).
  - [src/core/mod.rs](../../../src/core/mod.rs) — `pub mod vault_sync;`
    line is unchanged; `cargo` resolves directory module
    automatically.
- **Affected source files (consumer side)**: 11 external call sites
  (per `grep -rE "use crate::core::vault_sync"`) compile unchanged
  because `mod.rs` re-exports every previously top-level item. No
  consumer edit is required by this change. One consumer,
  [src/mcp/server.rs](../../../src/mcp/server.rs)::`map_vault_sync_error`,
  is updated *opportunistically* in the same change to consume the
  new structured `Vec<PathBuf>` fields instead of re-parsing the
  formatted `String` — this is the one place where the metadata
  reshape has any observable surface.
- **APIs**: the public surface of `crate::core::vault_sync` is
  preserved verbatim. `VaultSyncError` keeps the same name and is
  still the type returned from public functions; what changes is that
  some of its variants now wrap nested child-enum types.
- **Dependencies**: none added.
- **Tests**:
  [tests/vault_sync_*.rs](../../../tests/) (created by
  `extract-inline-tests-to-integration`) is the regression gate. Every
  one of the ~12 commits in this change runs `cargo test
  --test 'vault_sync_*'` clean before being committed.
- **Downstream proposals unblocked**:
  - File-size budget enforcement (§10): the warn-or-deny lint can be
    turned on without grandfathering this file.
  - Crate-level `#![warn(missing_docs)]` (§6.1): every new file already
    carries the required module-level `//!` doc.
- **Sequencing**: depends on
  [extract-inline-tests-to-integration](../extract-inline-tests-to-integration/proposal.md)
  landing first. Without that, the diff for this change collides with
  6.5k lines of test code being moved at the same time, which is
  unreviewable and breaks `git bisect`. This change does not start
  until the test extraction merges to `main`.
- **Reviewability**: shipped as one commit per submodule extraction
  plus two commits for the error split and one each for the two
  function extractions — roughly 12 commits, none touching more than
  one concern, every commit `cargo build && cargo test` clean.
