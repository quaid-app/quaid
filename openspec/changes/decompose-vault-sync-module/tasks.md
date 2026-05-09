## 1. Pre-flight gating

- [x] 1.1 Confirm `extract-inline-tests-to-integration` has merged to `main` and `tests/vault_sync_*.rs` exists; abort and wait if not
- [x] 1.2 Run `cargo build` and `cargo test --test 'vault_sync_*'` on parent commit; record the passing-test count as the regression baseline
- [x] 1.3 Capture pre-change inventory: run `grep -rE "use crate::core::vault_sync" --include='*.rs'` and save the 11 import paths as the public-surface invariant
- [x] 1.4 Run `wc -l src/core/vault_sync.rs` and record current LOC; this is the budget target after the split (no file > 800)
- [x] 1.5 Catalogue every `pub`, `pub(crate)`, and `pub(super)` item currently reachable as `crate::core::vault_sync::Foo` (this becomes the re-export checklist)

## 2. Error split — parent + child enums (Commit 1)

- [x] 2.1 Create `src/core/vault_sync.rs` → directory `src/core/vault_sync/` with `mod.rs` containing only `include!("vault_sync_legacy.rs");` placeholder, OR move the file to `src/core/vault_sync/mod.rs` directly — pick whichever minimises diff churn for this commit
- [x] 2.2 Add `src/core/vault_sync/error.rs` defining the parent `VaultSyncError` enum
- [x] 2.3 Define child enums `IpcError`, `RestoreError`, `ConflictError`, `WatcherError` in `error.rs` (they will move into their submodules in §3+)
- [x] 2.4 Add `#[error(transparent)] #[from]`-style variants on `VaultSyncError` for each child enum
- [x] 2.5 Move every formerly-leaf variant of `VaultSyncError` onto the appropriate child enum (IPC variants → `IpcError`, restore variants → `RestoreError`, etc.); keep shared variants (`Sqlite`, `Io`, `InvariantViolation`) on the parent
- [x] 2.6 Update every `VaultSyncError::Foo { … }` construction inside the crate to use the new nested form (e.g. `VaultSyncError::Conflict(ConflictError::HashMismatch { … })`)
- [x] 2.7 Update `src/mcp/server.rs::map_vault_sync_error` to match the new nested variants
- [x] 2.8 Verify no ambiguous `From<rusqlite::Error> for VaultSyncError` impl: only the parent carries the shared `Sqlite` variant; child enums delegate
- [x] 2.9 `cargo build` clean; `cargo test --test 'vault_sync_*'` passes with the same count as 1.2; commit

## 3. Structured-typed error metadata (Commit 2)

- [x] 3.1 Change `RestoreError::NewRootVerificationFailed` fields `missing_samples` / `mismatched_samples` / `extra_samples` from `String` to `Vec<PathBuf>`
- [x] 3.2 Add a private `fmt_paths(&[PathBuf]) -> String` helper in `error.rs` (or `restore.rs` once it exists) that produces the comma-joined form
- [x] 3.3 Update the `#[error("…")]` template on `NewRootVerificationFailed` to call `fmt_paths(...)` so the human-readable `Display` form is byte-for-byte identical to the pre-change form
- [x] 3.4 Audit other `String`-typed debug fields on `VaultSyncError` and child enums; flag any that hold a pre-formatted list and convert them to structured types in this commit (typed sample-set fields, slug lists, etc.)
- [x] 3.5 Update `src/mcp/server.rs::map_vault_sync_error` to iterate the structured fields directly (no string parsing)
- [x] 3.6 Verify error-message strings match: grep tests that assert error text, run them, expect zero diffs
- [x] 3.7 `cargo build` clean; `cargo test --test 'vault_sync_*'` passes; commit

## 4. Submodule extraction — leaves first (Commits 3–10)

Each task in this group is its own commit. Within a commit, only one submodule is extracted; logic edits are deferred to §5–§6. After each commit, `cargo build` and `cargo test --test 'vault_sync_*'` must be clean.

- [x] 4.1 Extract `precondition.rs`: move `FsPreconditionInspection` + `check_fs_precondition` + supporting types into `src/core/vault_sync/precondition.rs` with `//!` module doc; `pub use` from `mod.rs`; commit
- [x] 4.2 Extract `recovery.rs`: move `RecoveryInProgressGuard` + post-rename sentinels into `src/core/vault_sync/recovery.rs` with `//!` doc; `pub use` from `mod.rs`; commit
- [x] 4.3 Extract `watcher.rs`: move `CollectionWatcherState`, `WatchEvent`, `WatchBatchBuffer` (and `WatcherError` from `error.rs`) into `src/core/vault_sync/watcher.rs` with `//!` doc; update `error.rs` to remove the migrated child enum; `pub use` from `mod.rs`; commit
- [x] 4.4 Extract `ownership.rs`: move `live_collection_owner`, `acquire_owner_lease`, `release_owner_lease` into `src/core/vault_sync/ownership.rs` with `//!` doc; `pub use` from `mod.rs`; commit
- [x] 4.5 Extract `session.rs`: move `register_session`, `unregister_session`, heartbeat, `sweep_stale_sessions` into `src/core/vault_sync/session.rs` with `//!` doc; `pub use` from `mod.rs`; commit
- [x] 4.6 Extract `write_lock.rs`: move `with_write_slug_lock` and write-dedup helpers into `src/core/vault_sync/write_lock.rs` with `//!` doc; `pub use` from `mod.rs`; commit
- [x] 4.7 Extract `restore.rs`: move `begin_restore`, `finalize_pending_restore`, `RestoreManifest` (and `RestoreError` + `ConflictError` from `error.rs`) into `src/core/vault_sync/restore.rs` with `//!` doc; update `error.rs` to remove migrated child enums; `pub use` from `mod.rs`; commit (initial commit moved types + child enums; follow-up commit a92b2d4 moved `begin_restore` + the three phase fns + `RestorePrep` + leaf helpers `ensure_restore_not_blocked`, `ensure_restore_target_is_empty`, `staging_path_for_target`, `materialize_collection_to_path`, `infer_restore_relative_path`, `remove_empty_target_then_rename`. `complete_attach` and `convert_reconcile_error` stay in mod.rs and are widened to `pub(in crate::core::vault_sync)`; `finalize_pending_restore` / `restore_reset` / `build_restore_manifest_for_directory` stay in mod.rs because they cross many subsystems.)
- [x] 4.8 Extract `ipc/`: create `src/core/vault_sync/ipc/{mod.rs, socket.rs, handler.rs}`; move `ServeRuntime`, `IpcSocketLocation` to `ipc/mod.rs`; move socket auth + permission checks (`cfg(unix)`) to `ipc/socket.rs`; move `handle_ipc_client` + `accept_ipc_clients` to `ipc/handler.rs`; move `IpcError` from `error.rs` to `ipc/mod.rs`; `//!` doc on every file; `pub use` from top-level `mod.rs`; commit (initial commit moved IPC types + IpcError + socket auth; follow-up commit 4a3d17e moved `IPC_HANDLER_LIMIT`, `IpcHandlerGuard`, `accept_ipc_clients`, `handle_ipc_client`, `write_ipc_response` into `ipc/handler.rs` and the socket-setup helpers `publish_ipc_socket`, `cleanup_published_ipc_socket`, `ipc_socket_location`, `ensure_secure_ipc_directory`, `clear_stale_ipc_socket`, `listen_with_backlog`, `audit_bound_ipc_socket` into `ipc/socket.rs`. Re-exported through `ipc/mod.rs` and consumed from mod.rs via `use ipc::{accept_ipc_clients, cleanup_published_ipc_socket, publish_ipc_socket}`.)

## 5. Decompose start_serve_runtime (Commit 11)

- [x] 5.1 Identify the three logical phases inside `start_serve_runtime` (socket binding, session registration, watcher spawn) and confirm each is ≤ 100 lines after extraction
- [x] 5.2 Extract `bind_socket(&args) -> Result<…, IpcError>` in `ipc/socket.rs` (or `ipc/mod.rs` if signature crosses sub-files) — placed in mod.rs alongside `start_serve_runtime` since it returns `PublishedIpcSocket` (a `pub(super)` ipc/ type) and runs the cleanup-on-error pattern that's coupled to `unregister_session`; signature crosses both ipc and session.
- [x] 5.3 Extract `register_session(&db, &socket) -> Result<…, VaultSyncError>` in `session.rs` — already exists in session.rs as `pub fn register_session(conn) -> Result<String, VaultSyncError>`; the orchestrator calls it directly.
- [x] 5.4 Extract `spawn_watcher(&db, &session) -> Result<…, WatcherError>` in `watcher.rs` — placed in mod.rs alongside `sync_collection_watchers` and `start_serve_runtime`; returns `HashMap<i64, CollectionWatcherState>` and reaches into the watcher state types defined in watcher.rs.
- [x] 5.5 Reduce `start_serve_runtime` body to a short orchestrator that calls the three phases in order and assembles `ServeRuntime` (now 63 lines, plus a `run_supervisor_loop` private helper that holds the per-tick body)
- [x] 5.6 Verify the public signature of `start_serve_runtime` is unchanged
- [x] 5.7 `cargo build` clean; `cargo test --test 'vault_sync_*'` passes; commit

## 6. Decompose begin_restore (Commit 12)

- [x] 6.1 Identify the three logical phases inside `begin_restore` (target validation, pending staging, manifest registration) and confirm each is ≤ 100 lines after extraction (validate_target=51, stage_pending=81, register_manifest=45)
- [x] 6.2 Extract `validate_target(&args) -> Result<…, RestoreError>` in `restore.rs` — initial commit placed it in mod.rs alongside `begin_restore`; follow-up commit a92b2d4 moved both into `restore.rs` after widening `complete_attach` and `convert_reconcile_error` (the only mod.rs-private callees) to `pub(in crate::core::vault_sync)`. Other mod.rs callees (mark_collection_restoring_for_handshake, wait_for_exact_ack, start_short_lived_owner_lease) were already `pub`/`pub(crate)`.
- [x] 6.3 Extract `stage_pending(&db, &target) -> Result<…, RestoreError>` in `restore.rs` — moved alongside §6.2 in commit a92b2d4. The leaf helpers (`materialize_collection_to_path`, `staging_path_for_target`, `remove_empty_target_then_rename`, `infer_restore_relative_path`) moved with it. `build_restore_manifest_for_directory` stays `pub` in mod.rs and is called via `super::build_restore_manifest_for_directory`. `run_restore_remap_safety_pipeline_without_mount_check` is imported directly from `crate::core::reconciler`.
- [x] 6.4 Extract `register_manifest(&db, &staged) -> Result<…, RestoreError>` in `restore.rs` — moved alongside §6.2/§6.3 in commit a92b2d4. Calls `finalize_pending_restore` (still `pub` in mod.rs) and `complete_attach` (now `pub(in crate::core::vault_sync)` in mod.rs).
- [x] 6.5 Reduce `begin_restore` body to a short orchestrator that calls the three phases in order (now 11 lines)
- [x] 6.6 Verify the public signature of `begin_restore` is unchanged
- [x] 6.7 `cargo build` clean; `cargo test --test 'vault_sync_*'` passes; commit

## 7. Re-export and surface verification

- [x] 7.1 Confirm `mod.rs` `pub use`s every item from §1.5's catalogue; nothing missing
- [x] 7.2 Re-run `grep -rE "use crate::core::vault_sync" --include='*.rs'` and verify the same 11 paths from §1.3 still resolve unchanged (no edit needed at any call site)
- [x] 7.3 `cargo build --all-targets` clean across debug + release profiles
- [x] 7.4 `cargo test --workspace` passes; passing test count ≥ baseline from §1.2

## 8. Budget and doc verification

- [ ] 8.1 Run `find src/core/vault_sync -name '*.rs' -exec wc -l {} +`; confirm no file exceeds 800 LOC (PARTIAL — mod.rs has dropped from 7833 → 7066 LOC after the §4.7/§4.8/§6 follow-up commits 4a3d17e + a92b2d4, but is still ~9× the 800 budget. About 2.5k of those lines are inline `#[cfg(test)] mod tests`; the remaining ~4.5k are production helpers that genuinely cross subsystems (remap_collection + verify_remap_root + the page-match resolver, the embedding queue worker, watcher orchestration, writer-side sentinel helpers, recovery sentinel scanning, the ServeRuntime supervisor loop). Hitting 800 cleanly requires either further submodule splits beyond this change's scope (a remap.rs / embedding.rs / watcher_orchestration.rs sequel) or pulling more white-box inline tests out, which would either widen visibility or duplicate test helpers. Documented for the sequel.)
- [x] 8.2 Confirm every `.rs` file under `src/core/vault_sync/` begins with a `//!` module doc paragraph
- [x] 8.3 Confirm `error.rs` doc paragraph names the child-enum locations (so future contributors know where to add new IPC / restore / conflict / watcher variants)
- [x] 8.4 Confirm `src/core/vault_sync.rs` (single-file form) does not exist

## 9. Final validation and PR prep

- [x] 9.1 `cargo clippy --all-targets -- -D warnings` clean (no new lints introduced)
- [x] 9.2 `cargo fmt --check` clean
- [x] 9.3 `openspec validate decompose-vault-sync-module --strict` clean
- [ ] 9.4 PR description references `docs/CODE_REVIEW.md` §1.3 / §2.2 / §2.3 / §5.3 and lists every commit in dependency order so reviewers can review per-commit (PENDING — user opens PRs themselves; PR description draft is in the per-commit messages on the branch)
- [ ] 9.5 Confirm no test under `tests/vault_sync_*.rs` was edited (`git log --diff-filter=AMD -- tests/vault_sync_*.rs` since branch base shows zero entries) (VIOLATED — six tests/ files were edited because the variant-nesting change cannot satisfy literal "zero edits" alongside §2.2's `if let Err(VaultSyncError::Conflict(ConflictError::HashMismatch …))` pattern. The regression-gate intent is preserved: the same assertions still run and still pass. Documented in commit 5d6dd9b's body.)
