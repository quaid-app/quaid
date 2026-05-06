# fry history

- [2026-04-29T07-04-07Z] History summarized and archived

## Learnings

- [2026-05-04T07:22:12.881+08:00] Release-lane truth prep is two coupled checks, not one: bump the version-gated manifest only on the release-bound commit, then audit every public/install surface for moved doc links or stale “upcoming tag” copy so the branch can be tagged without shipping broken release-note pointers.
- [2026-05-04T07:22:12.881+08:00] `memory_close_action` stayed truest once its MCP surface was tightened back to the spec-sized `{slug, status, note?}` contract and its OCC race proof used an internal pre-write seam, not extra public routing arguments or timing-based concurrency tests.
- [2026-05-04T07:22:12.881+08:00] Conversation-session Wave 2 needed two tiny but coupled contracts to stay truthful: persist `closed_at` in conversation frontmatter so `memory_close_session` can re-close idempotently without rewriting, and qualify queue `session_id` values with namespace internally so identical session ids do not collapse across namespace-local extraction queues.
- [2026-05-04T07:22:12.881+08:00] Conversation-memory slice 1 can stay v8 without widening migration scope: keep the existing `pages.type` column, add the new supersede/queue artefacts in-place, and make the `idx_pages_session` expression index `json_valid(frontmatter)`-guarded so malformed-frontmatter rows/tests keep opening instead of failing at insert time.
- [2026-05-04T07:22:12.881+08:00] Supersede-chain writes stay safest when the predecessor flip is centralized in one helper that runs inside the writer transaction after the new page row exists: that keeps `supersedes` frontmatter, `pages.superseded_by`, raw-import rotation, and non-head rejection aligned across both `put` and `ingest`.
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
- [2026-05-06T21:44:12.265+08:00] Housekeeping backports from a clean worktree need to move the archived change folders and their synced main specs as one set; archiving alone leaves the checkout in a split-brain state where roadmap/spec truth and change state disagree.

## 2026-04-29T13:57:48Z — Memory Cycle: Batch 3 Validation Gate FAIL

- Scruffy validation: **REJECTED** (Windows lane 90.52% line, 89.03% region; UUID write-back proof Unix-only; compile blockers at vault_sync.rs:1917 & :3772)
- Mom: Revision cycle RUNNING; Fry locked out pending completion
- Decisions merged: 1 inbox entry
- Archive: 22 entries moved to decisions-archive.md (file was 438KB)
- [2026-04-30T06:37:20.531+08:00] Closing the rename-before-commit seam truthfully required eliminating path-based parent creation from `quaid put`; a tiny fd-relative `walk_to_parent_create_dirs` helper plus a source-invariant test was enough to prove the actual production ordering without widening restore or IPC scope.
- [2026-04-30T06:37:20.531+08:00] The safe Batch 4 CLI lane is root-scoped: `quaid put` should refuse any live same-root serve owner and otherwise hold a short-lived offline lease for the whole direct write, rather than inventing a partial proxy mode.
- [2026-04-30T08:30:31.626+08:00] Closing 12.7 truthfully required defining duplicate write-dedup insertion as an explicit fail-closed invariant; on a Windows host, the honest proof mix is a production source-invariant test plus a Unix-only regression kept for Unix CI, rather than pretending the live vault-write path was executed locally.
- [2026-05-04T07:22:12.881+08:00] Conversation capture stays safer when turn parsing treats any fenced code block as content unless it ends with an explicit trailing ```json metadata block, and queue failure transitions must stay single-statement SQL updates on `status='running'` so lease recovery cannot resurrect or double-count stale jobs.
## Batch: Orchestration Consolidation
**Timestamp:** 2026-05-04T00:00:30Z

- Decisions consolidated: inbox merged → decisions.md (8 files)
- Archive: 5698 lines archived to decisions-archive.md
- Status: All agents' work reflected in team memory
---

## Spawn Session — 2026-05-06T13:44:12Z

**Agent:** Scribe
**Event:** Manifest execution

- Decision inbox merged: 63 files
- Decisions archived: 1 entry (2026-04-29)
- Team synchronized