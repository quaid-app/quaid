# coverage-helper-seams-windows

Use this when a Windows coverage lane is blocked by Unix-gated CLI success paths but the target file still has real helper logic worth proving.

## Pattern

1. Prefer same-file unit tests over subprocess CLI tests for Windows-only coverage pushes.
2. Target helpers that are still Windows-reachable:
   - fail-closed dispatch branches
   - status/summary formatting
   - cached-mirror / parse-error handling
   - root validation and path rejection
   - offline helper paths that avoid Unix watcher/reconcile backends
3. Do **not** fake Unix success with broad seams unless the contract explicitly requires it.
4. Stop when remaining misses are mostly:
   - Unix-only attach/sync/restore happy paths
   - injected I/O cleanup branches with poor line yield
   - reconciler/vault-sync bodies that need Linux/macOS integration proof

## Quaid-specific note

For `src/commands/collection.rs`, Windows can still buy meaningful lines through direct helper tests, but the final stretch to a repo-wide 90% gate is controlled by `core/reconciler.rs` and `core/vault_sync.rs`, not by more CLI wrapper squeezing.

For Batch 6 audit/retention work, prefer moving idle-page retention into a cross-platform helper (`core/raw_imports.rs`) and cover the Unix-only `collection audit` happy path with `#[cfg(unix)]` tests while Windows only proves the platform guard. That keeps coverage truthful when `vault_sync` owns the real timer hooks but the reconcile walk itself is Unix-only.
