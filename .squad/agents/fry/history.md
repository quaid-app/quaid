# fry history

- [2026-04-29T07-04-07Z] History summarized and archived

## Learnings

- [2026-04-30T12:07:19.084+08:00] Batch 5 seam map: `start_serve_runtime` owns the right lifecycle hook for `serve_sessions.ipc_path`, but `commands\serve.rs` / `src\mcp\server.rs` still expose only stdio transport and a `QuaidServer` with no session context, so truthful IPC will need a shared server/session seam plus a `whoami` cross-check surface before `quaid put` can proxy safely.
- [2026-04-30T12:07:19.084+08:00] Batch 5 seam map: `src\commands\put.rs` still has no live-owner routing at all, and `src\commands\collection.rs` still defers `--write-quaid-id` / lacks `migrate-uuids`; Batch 5 must treat bulk-refusal claims as coupled truth debt, not assume those CLI surfaces already exist.
- [2026-04-29T20:33:01.970+08:00] Batch 3 recon: `src\commands\collection.rs` still exposes deferred `write_memory_id` on `CollectionAddArgs`; `CollectionAction` has no `migrate-uuids` variant yet, so Batch 3 must add new CLI args/dispatch and retire the defer test.
- [2026-04-29T20:33:01.970+08:00] UUID/frontmatter naming is still `memory_id` across `src\core\page_uuid.rs`, `src\core\markdown.rs`, `src\core\reconciler.rs`, `src\core\vault_sync.rs`, `src\commands\put.rs`, and `tests\roundtrip_raw.rs`, while vault-sync OpenSpec Batch 3 language says `quaid_id`; this is the main contract seam to settle before write-back lands.
- [2026-04-29T20:33:01.970+08:00] Rename-before-commit production logic currently lives in `src\commands\put.rs::persist_with_vault_write`, while `src\core\vault_sync.rs` only has test-only writer crash-core helpers; Batch 3 should reuse/extract that path rather than duplicate raw_import/file_state rotation logic.
- [2026-04-29T20:33:01.970+08:00] Live-owner data already exists in `serve_sessions(pid, host, heartbeat_at)` plus `collection_owners`, but `VaultSyncError::ServeOwnsCollectionError` only carries `owner_session_id`; Batch 3 bulk-write guards will need an owner-detail lookup seam before CLI can report pid/host truthfully.
- [2026-04-29T20:33:01.970+08:00] Batch 3 landed by routing UUID write-back through `src\commands\put.rs::put_from_string`, so `collection add --write-quaid-id` and `collection migrate-uuids` reuse the production sentinel/tempfile/rename/file_state/raw_imports path instead of duplicating a weaker writer.
- [2026-04-29T20:33:01.970+08:00] `src\core\page_uuid.rs` now accepts legacy `memory_id` on read, but `src\core\markdown.rs::render_page` canonicalizes every persisted/exported write to `quaid_id`; migration commands intentionally rewrite files that still lack `quaid_id`. 
- [2026-04-30T06:37:20.531+08:00] Batch 4 audit: the rename-before-commit core is mostly landed in `src\commands\put.rs`, but the real `12.1` gap is still step 2 — `src\core\fs_safety.rs::walk_to_parent` has no `create_dirs` mode, and the writer still falls back to path-based `fs::create_dir_all(...)` before reopening the parent fd.
- [2026-04-30T06:37:20.531+08:00] Batch 4 audit: `implementation_plan.md` assumes Batch 3 `migrate-uuids` / `--write-quaid-id` already exist for `12.6b`, but `src\commands\collection.rs` still rejects `write_memory_id` as deferred and has no `MigrateUuids` action, so Batch 3 task state is still incomplete/stale.
- [2026-04-30T06:37:20Z] Batch 4 decision merged to team ledger. Awaiting Leela worktree setup before implementation begins.

## 2026-04-29T13:57:48Z — Memory Cycle: Batch 3 Validation Gate FAIL

- Scruffy validation: **REJECTED** (Windows lane 90.52% line, 89.03% region; UUID write-back proof Unix-only; compile blockers at vault_sync.rs:1917 & :3772)
- Mom: Revision cycle RUNNING; Fry locked out pending completion
- Decisions merged: 1 inbox entry
- Archive: 22 entries moved to decisions-archive.md (file was 438KB)
- [2026-04-30T06:37:20.531+08:00] Closing the rename-before-commit seam truthfully required eliminating path-based parent creation from `quaid put`; a tiny fd-relative `walk_to_parent_create_dirs` helper plus a source-invariant test was enough to prove the actual production ordering without widening restore or IPC scope.
- [2026-04-30T06:37:20.531+08:00] The safe Batch 4 CLI lane is root-scoped: `quaid put` should refuse any live same-root serve owner and otherwise hold a short-lived offline lease for the whole direct write, rather than inventing a partial proxy mode.
- [2026-04-30T08:30:31.626+08:00] Closing 12.7 truthfully required defining duplicate write-dedup insertion as an explicit fail-closed invariant; on a Windows host, the honest proof mix is a production source-invariant test plus a Unix-only regression kept for Unix CI, rather than pretending the live vault-write path was executed locally.
