## Context

[src/core/vault_sync.rs](../../../src/core/vault_sync.rs) today is one
file with eleven loosely-related concerns: `VaultSyncError` (a
30+-variant enum spanning IPC permissions, restore manifests,
reconcile fences, conflict detection, and registry poisoning),
session register / unregister / heartbeat / sweep, owner-lease
acquire / release, write-slug locking and write dedup, IPC socket
binding, IPC permission checks, IPC client handlers, watcher state
and event buffering, restore staging and finalisation, recovery
sentinels, and filesystem precondition checks. Each concern already
has a natural seam in the file (each `cfg(unix)` block, each
runtime-registry helper); they're stuck together by file boundaries,
not by coupling.

The
[code review](../../../docs/CODE_REVIEW.md) §1.3 ranks splitting this
module as the single biggest quality win, with the §10 file-size
budget setting 800 LOC per production file. After the preceding
[extract-inline-tests-to-integration](../extract-inline-tests-to-integration/proposal.md)
change moves 6,596 LOC of inline tests to `tests/`, the remaining
~5,909 LOC of production code still violates the budget by 7×.

External coupling is small: 11 call sites match
`grep -rE "use crate::core::vault_sync"`, mostly importing
`VaultSyncError` and `ResolvedSlug`. This means the split can be
done as a re-export job — public API is preserved verbatim.

Sequencing matters. This change runs strictly after
`extract-inline-tests-to-integration`. Trying to ship both at once
makes the diff unreviewable (~12k lines moving simultaneously) and
breaks `git bisect` because you can't tell whether a regression came
from the test move or the code split.

## Goals / Non-Goals

**Goals:**

- Replace the single 5,909-LOC `src/core/vault_sync.rs` with the
  directory module `src/core/vault_sync/` per §1.3, holding every
  file ≤ 800 LOC.
- Preserve the public surface byte-for-byte: every existing
  `crate::core::vault_sync::Foo` import path resolves unchanged.
- Replace the kitchen-sink `VaultSyncError` enum with a parent
  enum + `#[from]`-composed child enums per §2.2.
- Replace `String`-formatted debug fields in error variants with
  structured types (`Vec<PathBuf>` etc.) per §2.3, and update the
  one consumer (`map_vault_sync_error` in `src/mcp/server.rs`) to
  iterate the structured fields directly.
- Decompose `start_serve_runtime` (227 lines) and `begin_restore`
  (181 lines) into named phases per §5.3, with each phase ≤ 100
  lines.
- Give every new file a one-paragraph `//!` module doc so a
  follow-up `#![warn(missing_docs)]` rollout is tractable.
- Make the change reviewable: ≤ 12 commits, one concern per
  commit, every commit `cargo build` and
  `cargo test --test 'vault_sync_*'` clean.

**Non-Goals:**

- No behavioural change. No new feature, no fix to a known bug, no
  perf change. The only test surface that should react to this
  change is "does it still compile and pass the existing test
  suite?"
- No public API change. No new `pub` items, no removed `pub` items,
  no renames. Re-exports preserve everything.
- No edits to `tests/vault_sync_*.rs`. That suite (created by the
  preceding change) is the regression gate; touching it would
  invalidate the gate.
- No edits to other large files: `src/mcp/server.rs`,
  `src/core/reconciler.rs`, `src/commands/{put,collection}.rs`,
  `src/core/db.rs`. Those have their own follow-up proposals.
- No lint flips this change (no enabling of `missing_docs`, no
  enabling of file-size lints). Those are downstream proposals
  that this change *unblocks*.
- Not redoing per-§1.6 the search/FTS API surface, even though
  some of the `String`-typed metadata cleanup is similar in
  flavour. Out of scope.

## Decisions

### Decision: Directory module, not flat-with-`mod` declarations

`src/core/vault_sync/` as a directory containing `mod.rs` and
sibling files. The alternative — keep `vault_sync.rs` flat with
`mod session;`, `mod ownership;` etc. inside it — would require
either re-introducing the same file or splitting along the same
lines but in a less idiomatic Rust layout. Directory modules are
the canonical Rust convention for multi-file modules and are what
§1.3 explicitly recommends. No real downside: `cargo` resolves
either form; tooling (rustdoc, rust-analyzer, IDE jump-to-def)
prefers directory modules.

### Decision: One commit per submodule extraction

Each of the 11 submodules lands in its own commit. `cargo build`
and `cargo test --test 'vault_sync_*'` are clean at every commit.
The error split lands in one or two commits before any concern is
moved out (because every submodule depends on the new error
types). The two function-extraction phases each get their own
commit. Total: ~12 commits.

Alternatives considered:

- **One big commit**: rejected. ~6k lines moving in one diff is
  unreviewable; bisect becomes useless.
- **One commit per concern in a single rebase-merge**: tempting,
  but loses the per-commit `cargo test` invariant that the spec
  requires.

### Decision: Move-then-modify, not modify-then-move

Each submodule extraction is a pure code move (no logic edits in
the same commit). If a function needs to be reorganised — e.g.,
`start_serve_runtime` decomposition — that happens in a *separate*
follow-up commit, after the move. This makes review trivial: the
move commit shows file-rename + import-path edits and nothing
else; the modify commit shows the actual logic change.

### Decision: Error split lands first, before any submodule moves

Concrete sequence:

1. **Commit 1**: introduce `error.rs` with the new parent
   `VaultSyncError` and the four child enums (`IpcError`,
   `RestoreError`, `ConflictError`, `WatcherError`). Until
   submodules are extracted, the child enums live in `error.rs`
   alongside the parent, so this commit's diff is local. Old
   variants on `VaultSyncError` are converted to
   `#[from]`-composed wrappers around the corresponding child
   enums. Callers updated to match the nested form. `cargo build`
   and `cargo test` clean.
2. **Commit 2**: `String` → `Vec<PathBuf>` for
   `NewRootVerificationFailed` (now a `RestoreError` variant) plus
   any other variants flagged by §2.3. `Display` impls do the
   join. Update `src/mcp/server.rs::map_vault_sync_error` to
   consume the structured fields. `cargo build` and `cargo test`
   clean.
3. **Commits 3–10**: submodule extractions, one at a time, in
   dependency order (leaves first, so each step is independent).
   Suggested order: `precondition.rs` → `recovery.rs` →
   `watcher.rs` → `ownership.rs` → `session.rs` → `write_lock.rs`
   → `restore.rs` → `ipc/`. When a submodule moves, its child
   enum (if any) moves with it from `error.rs` into the
   submodule, leaving only the parent + shared variants in
   `error.rs`.
4. **Commit 11**: decompose `start_serve_runtime` into
   `bind_socket` + `register_session` + `spawn_watcher`.
5. **Commit 12**: decompose `begin_restore` into
   `validate_target` + `stage_pending` + `register_manifest`.

This ordering means the parent enum's external shape stabilises
in commit 1; every later commit can move code without touching
the type system surface again.

Alternative considered: extract submodules first, *then* split
the error. Rejected because submodule extractions need the new
child-enum types to exist already (a moved IPC handler that
returns `IpcError` needs `IpcError` to exist) — otherwise each
submodule commit forces a temporary `VaultSyncError::Ipc(…)`
adapter that gets ripped out two commits later, doubling churn.

### Decision: Re-exports go in `mod.rs`, not in a top-level prelude

`mod.rs` `pub use`s every previously top-level item:

```rust
pub use error::{
    VaultSyncError, IpcError, RestoreError, ConflictError, WatcherError,
};
pub use session::{register_session, unregister_session, sweep_stale_sessions, …};
pub use ownership::{live_collection_owner, acquire_owner_lease, release_owner_lease, …};
// …
```

Alternatives:

- **Inline `pub mod session;`**: would force callers to use
  `crate::core::vault_sync::session::register_session` instead of
  the existing `crate::core::vault_sync::register_session`. That
  *is* an API change. Rejected.
- **A separate `prelude` module**: callers would need
  `use crate::core::vault_sync::prelude::*;`, also an API change.
  Rejected.

`pub use` in `mod.rs` is the only option that keeps existing
imports working. Cost: `mod.rs` ends up with a long re-export
list (probably ~80–100 lines of `pub use`); that's well within
the 800-LOC budget.

### Decision: Child enums live next to producing code, not all in `error.rs`

§2.2 explicitly recommends "each child enum lives next to the code
that produces it." After the submodule extractions, `IpcError`
lives in `ipc/`, `RestoreError` in `restore.rs`, etc. `error.rs`
holds only the parent `VaultSyncError` plus shared variants
(`Sqlite`, `Io`, `InvariantViolation`).

Trade-off: if a future contributor wants to add a new IPC error
variant, they have to know it lives in `ipc/`, not `error.rs`.
Mitigation: `error.rs` carries a `//!` doc paragraph that points
to the child-enum locations. The convention is also captured by
the spec's "Child enums live next to the code that produces them"
scenario, so `openspec validate` is a long-term forcing function.

### Decision: `Display` for structured-typed variants does the join

For `NewRootVerificationFailed { missing_samples: Vec<PathBuf>, … }`,
the struct holds the data; `impl Display` (or `#[error("…")]` via
`thiserror`) formats it. The format string is preserved verbatim
from the previous `String`-typed variant so error messages look
identical to anything that grep-greps logs or matches them in
tests.

```rust
#[error("new root verification failed: missing={}, mismatched={}, extra={}",
    fmt_paths(missing_samples), fmt_paths(mismatched_samples), fmt_paths(extra_samples))]
NewRootVerificationFailed {
    missing_samples: Vec<PathBuf>,
    mismatched_samples: Vec<PathBuf>,
    extra_samples: Vec<PathBuf>,
},

fn fmt_paths(p: &[PathBuf]) -> String {
    p.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
}
```

The `fmt_paths` helper lives in `error.rs` (or its `restore.rs`
equivalent for `RestoreError`).

### Decision: Function decomposition keeps the public name and signature

`start_serve_runtime(args…) -> Result<…>` keeps its public name and
signature. Internally:

```rust
pub(crate) fn start_serve_runtime(args…) -> Result<…> {
    let socket = bind_socket(&args)?;
    let session = register_session(&db, &socket)?;
    let watcher = spawn_watcher(&db, &session)?;
    Ok(ServeRuntime { socket, session, watcher })
}
```

Each helper is a `pub(super)` (or private) fn whose body is what
used to be a logical block of `start_serve_runtime`. Names match
the spec's required phases. No external caller needs to know the
helpers exist; they're not re-exported.

`begin_restore` follows the same pattern with
`validate_target` + `stage_pending` + `register_manifest`.

### Decision: Module docs are written in the move commit, not deferred

When a concern is moved into its own submodule (e.g., commit 5 moves
session code), the `//!` module doc for the new file is written in
the *same commit*. This keeps each new file from existing for any
period without a doc and makes `proposal #7`'s
`#![warn(missing_docs)]` rollout straightforward — every file
already complies the moment it's introduced.

## Risks / Trade-offs

- **Risk: A submodule extraction silently changes visibility.**
  The most common failure mode for this kind of refactor is that
  an item that was `pub(crate)` in the flat file becomes `pub` (or
  vice versa) when moved into a submodule, because the contributor
  forgets to think about visibility when re-exporting.
  **Mitigation**: explicit checklist for each move commit — diff
  the set of `pub`/`pub(crate)`/`pub(super)` items reachable from
  the crate root before and after, ensure exact equality. The
  "Public surface preserved" spec scenario plus the regression
  test `cargo test --test 'vault_sync_*'` plus `cargo build`
  catch the most common cases. Risk remains for items that are
  not called by any test or build path; these are inspected by
  hand during review.

- **Risk: Compilation churn during the multi-commit sequence.**
  Each commit must `cargo build` cleanly; this means temporary
  `pub use` re-exports during a multi-step move can pile up.
  **Mitigation**: commit ordering is leaves-first, so every move
  is independent and no commit needs a temporary shim. If a shim
  *is* needed for a step, it gets ripped out in the same commit
  rather than the next.

- **Risk: `#[from]` recursion in error variants.**
  `VaultSyncError::Ipc(IpcError)` plus `IpcError::Sqlite(rusqlite::Error)`
  plus `VaultSyncError::Sqlite(rusqlite::Error)` would create
  ambiguous `From<rusqlite::Error> for VaultSyncError` impls.
  **Mitigation**: only the parent `VaultSyncError` carries the
  shared `Sqlite`/`Io` variants. Child enums delegate through
  the parent for shared concerns, or carry their own
  subsystem-specific wrapping (e.g., `IpcError::SocketBind` carries
  the `io::Error` directly). `cargo build` will catch any
  ambiguous `From` impl immediately.

- **Risk: Error message string compatibility.**
  Some downstream tooling (logs, alerts, tests) may grep error
  messages by string content. Restructuring `Display` could
  inadvertently change the message.
  **Mitigation**: keep `Display` format strings byte-for-byte
  identical to the pre-change form. The `#[error("…")]` template
  on each variant is the contract. Any change to wording is
  flagged by the existing `tests/vault_sync_*.rs` cases that
  match error messages, plus a manual diff of `cargo expand` on
  a representative variant before/after.

- **Risk: 11 external call sites grow during the work.**
  If another in-flight branch adds a new `use
  crate::core::vault_sync::Foo;` while this change is being
  prepared, the re-export list in `mod.rs` may be incomplete.
  **Mitigation**: re-run `grep -rE "use crate::core::vault_sync"`
  immediately before the final commit; add any missing
  re-exports. The regression gate (`cargo build`) catches this
  at zero cost.

- **Trade-off: `mod.rs` becomes a long re-export list.**
  Probably ~80–100 lines of `pub use`. Some readers prefer
  `pub mod foo;` over `pub use foo::*;` for discoverability.
  **Acknowledged**: this is the cost of preserving the existing
  external API path. Mitigation: group re-exports by submodule
  with section comments, and have `mod.rs`'s `//!` doc paragraph
  list the submodules explicitly.

- **Trade-off: error matches may now require one nesting level.**
  `if let Err(VaultSyncError::HashMismatch { … })` becomes
  `if let Err(VaultSyncError::Conflict(ConflictError::HashMismatch { … }))`.
  Slightly more verbose at the call site.
  **Acknowledged**: per §2.2 this is the intended outcome — it
  makes pattern matching focused and lets each subsystem evolve
  independently. The 11 external call sites are mostly imports,
  not pattern matches; only `src/mcp/server.rs::map_vault_sync_error`
  has a deep match block, and updating it is part of this
  change.

## Migration Plan

Migration is the commit sequence above. There is no
runtime-state migration; this is a compile-time refactor.
Rollback strategy: any single commit can be reverted; the
preceding commit was `cargo test` clean. Reverting the entire
change reverts to the pre-split form with no intermediate state.

## Open Questions

- **Should the `IpcSocketLocation` and `ServeRuntime` types live
  in `ipc/mod.rs` or in `mod.rs`?** §1.3 puts them in `ipc/mod.rs`;
  reading the current code, `ServeRuntime` is also referenced
  outside the IPC path (it's the runtime handle returned to
  callers). Provisional decision: keep them in `ipc/mod.rs` and
  re-export from the top-level `mod.rs`. Confirm during
  implementation.

- **Where does `ResolvedSlug` live?** It's one of the most-imported
  items per the 11 call sites. It's not obviously IPC, restore,
  watcher, or any other submodule's concern — it's a shared type.
  Provisional decision: keep it in `mod.rs` (or in a new
  `slug.rs`) and re-export. Confirm during implementation.

- **Whether to land the
  `src/mcp/server.rs::map_vault_sync_error` consumer-side update
  in the same change.** §2.3's motivation is precisely that this
  consumer is structurally crippled by the `String`-typed metadata.
  Splitting the consumer update into a separate change leaves the
  `vault-sync-module-layout` capability with an unsatisfied
  scenario ("`map_vault_sync_error` consumes structured data") for
  one merge cycle. Provisional decision: include the consumer
  update in this change (1–2 lines edited in `src/mcp/server.rs`).
  Confirm during implementation that no other consumer of these
  variants needs the same treatment.
