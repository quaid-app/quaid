## 1. Pre-flight gating

- [ ] 1.1 Confirm `extract-inline-tests-to-integration` has merged to `main` and `tests/vault_sync_*.rs` exists; abort and wait if not
- [ ] 1.2 Run `cargo build` and `cargo test --test 'vault_sync_*'` on parent commit; record the passing-test count as the regression baseline
- [ ] 1.3 Capture pre-change inventory: run `grep -rE "use crate::core::vault_sync" --include='*.rs'` and save the 11 import paths as the public-surface invariant
- [ ] 1.4 Run `wc -l src/core/vault_sync.rs` and record current LOC; this is the budget target after the split (no file > 800)
- [ ] 1.5 Catalogue every `pub`, `pub(crate)`, and `pub(super)` item currently reachable as `crate::core::vault_sync::Foo` (this becomes the re-export checklist)

## 2. Error split — parent + child enums (Commit 1)

- [ ] 2.1 Create `src/core/vault_sync.rs` → directory `src/core/vault_sync/` with `mod.rs` containing only `include!("vault_sync_legacy.rs");` placeholder, OR move the file to `src/core/vault_sync/mod.rs` directly — pick whichever minimises diff churn for this commit
- [ ] 2.2 Add `src/core/vault_sync/error.rs` defining the parent `VaultSyncError` enum
- [ ] 2.3 Define child enums `IpcError`, `RestoreError`, `ConflictError`, `WatcherError` in `error.rs` (they will move into their submodules in §3+)
- [ ] 2.4 Add `#[error(transparent)] #[from]`-style variants on `VaultSyncError` for each child enum
- [ ] 2.5 Move every formerly-leaf variant of `VaultSyncError` onto the appropriate child enum (IPC variants → `IpcError`, restore variants → `RestoreError`, etc.); keep shared variants (`Sqlite`, `Io`, `InvariantViolation`) on the parent
- [ ] 2.6 Update every `VaultSyncError::Foo { … }` construction inside the crate to use the new nested form (e.g. `VaultSyncError::Conflict(ConflictError::HashMismatch { … })`)
- [ ] 2.7 Update `src/mcp/server.rs::map_vault_sync_error` to match the new nested variants
- [ ] 2.8 Verify no ambiguous `From<rusqlite::Error> for VaultSyncError` impl: only the parent carries the shared `Sqlite` variant; child enums delegate
- [ ] 2.9 `cargo build` clean; `cargo test --test 'vault_sync_*'` passes with the same count as 1.2; commit

## 3. Structured-typed error metadata (Commit 2)

- [ ] 3.1 Change `RestoreError::NewRootVerificationFailed` fields `missing_samples` / `mismatched_samples` / `extra_samples` from `String` to `Vec<PathBuf>`
- [ ] 3.2 Add a private `fmt_paths(&[PathBuf]) -> String` helper in `error.rs` (or `restore.rs` once it exists) that produces the comma-joined form
- [ ] 3.3 Update the `#[error("…")]` template on `NewRootVerificationFailed` to call `fmt_paths(...)` so the human-readable `Display` form is byte-for-byte identical to the pre-change form
- [ ] 3.4 Audit other `String`-typed debug fields on `VaultSyncError` and child enums; flag any that hold a pre-formatted list and convert them to structured types in this commit (typed sample-set fields, slug lists, etc.)
- [ ] 3.5 Update `src/mcp/server.rs::map_vault_sync_error` to iterate the structured fields directly (no string parsing)
- [ ] 3.6 Verify error-message strings match: grep tests that assert error text, run them, expect zero diffs
- [ ] 3.7 `cargo build` clean; `cargo test --test 'vault_sync_*'` passes; commit

## 4. Submodule extraction — leaves first (Commits 3–10)

Each task in this group is its own commit. Within a commit, only one submodule is extracted; logic edits are deferred to §5–§6. After each commit, `cargo build` and `cargo test --test 'vault_sync_*'` must be clean.

- [ ] 4.1 Extract `precondition.rs`: move `FsPreconditionInspection` + `check_fs_precondition` + supporting types into `src/core/vault_sync/precondition.rs` with `//!` module doc; `pub use` from `mod.rs`; commit
- [ ] 4.2 Extract `recovery.rs`: move `RecoveryInProgressGuard` + post-rename sentinels into `src/core/vault_sync/recovery.rs` with `//!` doc; `pub use` from `mod.rs`; commit
- [ ] 4.3 Extract `watcher.rs`: move `CollectionWatcherState`, `WatchEvent`, `WatchBatchBuffer` (and `WatcherError` from `error.rs`) into `src/core/vault_sync/watcher.rs` with `//!` doc; update `error.rs` to remove the migrated child enum; `pub use` from `mod.rs`; commit
- [ ] 4.4 Extract `ownership.rs`: move `live_collection_owner`, `acquire_owner_lease`, `release_owner_lease` into `src/core/vault_sync/ownership.rs` with `//!` doc; `pub use` from `mod.rs`; commit
- [ ] 4.5 Extract `session.rs`: move `register_session`, `unregister_session`, heartbeat, `sweep_stale_sessions` into `src/core/vault_sync/session.rs` with `//!` doc; `pub use` from `mod.rs`; commit
- [ ] 4.6 Extract `write_lock.rs`: move `with_write_slug_lock` and write-dedup helpers into `src/core/vault_sync/write_lock.rs` with `//!` doc; `pub use` from `mod.rs`; commit
- [ ] 4.7 Extract `restore.rs`: move `begin_restore`, `finalize_pending_restore`, `RestoreManifest` (and `RestoreError` + `ConflictError` from `error.rs`) into `src/core/vault_sync/restore.rs` with `//!` doc; update `error.rs` to remove migrated child enums; `pub use` from `mod.rs`; commit
- [ ] 4.8 Extract `ipc/`: create `src/core/vault_sync/ipc/{mod.rs, socket.rs, handler.rs}`; move `ServeRuntime`, `IpcSocketLocation` to `ipc/mod.rs`; move socket auth + permission checks (`cfg(unix)`) to `ipc/socket.rs`; move `handle_ipc_client` + `accept_ipc_clients` to `ipc/handler.rs`; move `IpcError` from `error.rs` to `ipc/mod.rs`; `//!` doc on every file; `pub use` from top-level `mod.rs`; commit

## 5. Decompose start_serve_runtime (Commit 11)

- [ ] 5.1 Identify the three logical phases inside `start_serve_runtime` (socket binding, session registration, watcher spawn) and confirm each is ≤ 100 lines after extraction
- [ ] 5.2 Extract `bind_socket(&args) -> Result<…, IpcError>` in `ipc/socket.rs` (or `ipc/mod.rs` if signature crosses sub-files)
- [ ] 5.3 Extract `register_session(&db, &socket) -> Result<…, VaultSyncError>` in `session.rs`
- [ ] 5.4 Extract `spawn_watcher(&db, &session) -> Result<…, WatcherError>` in `watcher.rs`
- [ ] 5.5 Reduce `start_serve_runtime` body to a short orchestrator that calls the three phases in order and assembles `ServeRuntime`
- [ ] 5.6 Verify the public signature of `start_serve_runtime` is unchanged
- [ ] 5.7 `cargo build` clean; `cargo test --test 'vault_sync_*'` passes; commit

## 6. Decompose begin_restore (Commit 12)

- [ ] 6.1 Identify the three logical phases inside `begin_restore` (target validation, pending staging, manifest registration) and confirm each is ≤ 100 lines after extraction
- [ ] 6.2 Extract `validate_target(&args) -> Result<…, RestoreError>` in `restore.rs`
- [ ] 6.3 Extract `stage_pending(&db, &target) -> Result<…, RestoreError>` in `restore.rs`
- [ ] 6.4 Extract `register_manifest(&db, &staged) -> Result<…, RestoreError>` in `restore.rs`
- [ ] 6.5 Reduce `begin_restore` body to a short orchestrator that calls the three phases in order
- [ ] 6.6 Verify the public signature of `begin_restore` is unchanged
- [ ] 6.7 `cargo build` clean; `cargo test --test 'vault_sync_*'` passes; commit

## 7. Re-export and surface verification

- [ ] 7.1 Confirm `mod.rs` `pub use`s every item from §1.5's catalogue; nothing missing
- [ ] 7.2 Re-run `grep -rE "use crate::core::vault_sync" --include='*.rs'` and verify the same 11 paths from §1.3 still resolve unchanged (no edit needed at any call site)
- [ ] 7.3 `cargo build --all-targets` clean across debug + release profiles
- [ ] 7.4 `cargo test --workspace` passes; passing test count ≥ baseline from §1.2

## 8. Budget and doc verification

- [ ] 8.1 Run `find src/core/vault_sync -name '*.rs' -exec wc -l {} +`; confirm no file exceeds 800 LOC
- [ ] 8.2 Confirm every `.rs` file under `src/core/vault_sync/` begins with a `//!` module doc paragraph
- [ ] 8.3 Confirm `error.rs` doc paragraph names the child-enum locations (so future contributors know where to add new IPC / restore / conflict / watcher variants)
- [ ] 8.4 Confirm `src/core/vault_sync.rs` (single-file form) does not exist

## 9. Final validation and PR prep

- [ ] 9.1 `cargo clippy --all-targets -- -D warnings` clean (no new lints introduced)
- [ ] 9.2 `cargo fmt --check` clean
- [ ] 9.3 `openspec validate decompose-vault-sync-module --strict` clean
- [ ] 9.4 PR description references `docs/CODE_REVIEW.md` §1.3 / §2.2 / §2.3 / §5.3 and lists every commit in dependency order so reviewers can review per-commit
- [ ] 9.5 Confirm no test under `tests/vault_sync_*.rs` was edited (`git log --diff-filter=AMD -- tests/vault_sync_*.rs` since branch base shows zero entries)
