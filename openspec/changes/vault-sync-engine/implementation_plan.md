# Vault-Sync-Engine: Implementation Plan

This document is the agent instruction guide for completing the `vault-sync-engine` OpenSpec change in `quaid-app/quaid`. It is structured as sequential batches targeting releases 0.10 through 0.16. Read this fully before starting any batch.

---

## Repository orientation

- **Repo root:** `d:\repos\quaid` (Windows host; all vault-sync code is `#[cfg(unix)]` gated)
- **Core source:** `src/core/` — one file per subsystem
- **Command handlers:** `src/commands/` — one file per CLI subcommand
- **MCP tool handlers:** `src/mcp/server.rs`
- **Schema:** `src/schema.sql` — embedded in the binary via `include_str!("schema.sql")` in `src/core/db.rs`; current schema version is **v6** (`SCHEMA_VERSION = 6` in `db.rs`)
- **Spec artifacts:** `openspec/changes/vault-sync-engine/` — `proposal.md`, `design.md`, `specs/`, `tasks.md`
- **Task tracking:** Mark completed tasks in `tasks.md` by changing `- [ ]` to `- [x]` immediately after the implementation is verified
- **Error types:** `VaultSyncError` enum lives in `src/core/vault_sync.rs`; new error variants go there
- **Tests:** Written inline in the same `.rs` file as the code under test, in a `#[cfg(test)]` module at the bottom. Follow the pattern of existing tests in `vault_sync.rs` and `collection_cli_truth.rs`

## Key invariants — never violate these

1. **DB state never leads disk state.** The SQLite commit for any vault write is the LAST step, after `renameat` + `fsync(parent_fd)`. See `design.md` § rename-before-commit.
2. **Exactly one active `raw_imports` row per page at all times.** Every content-changing write rotates `raw_imports` in the same SQLite tx. Zero active rows → `InvariantViolationError` before mutation.
3. **Vault-byte write entry points (`quaid put`, `memory_put`) refuse when `collections.writable=0`, `state='restoring'`, or `needs_full_sync=1`.** DB-only mutators (`memory_link`, `memory_check`, `memory_raw`, etc.) have their own separate interlock — do not merge the two gates.
4. **Watcher never drops events silently.** Channel overflow → set `needs_full_sync=1`, WARN log, continue. Recovery runs within ~1s.
5. **All vault-sync CLI surfaces are `#[cfg(unix)]`-gated.** Windows path returns `UnsupportedPlatformError`. DB-only commands (restore-reset, reconcile-reset) are exempt from the platform gate.
6. **Schema version bumps require updating `SCHEMA_VERSION` in `src/core/db.rs` AND the comment header in `src/schema.sql`.** The open() path refuses any DB whose stored version is less than `SCHEMA_VERSION`.

## Current completion state

As of 2026-04-28: **203 done / 110 open / 313 total (65%).**

The core sync loop is working end-to-end: serve → watcher → debounce → stat-diff reconcile → FTS/DB update → MCP query. The foundation tasks (schema v5/v6, collections model, ignore patterns, file state, reconciler, self-write dedup, collection CLI, UUID lifecycle, MCP slug routing, restore Tx-A/Tx-B, write interlocks) are all done. What remains is watcher hardening, embedding worker, UUID write-back, full rename-before-commit (steps 2–13 beyond the M1a sentinel core), IPC socket, restore/remap end-to-end, and cleanup.

---

## Batch 1 — Watcher reliability (target release: 0.10)

**Tasks:** 6.7a, 6.8, 6.9, 6.10, 6.11 + 17.5w, 17.5x, 17.5y, 17.5z, 17.5aa, 17.5aaa2, 17.5aaa3, 17.5aaa4

**Why first:** Highest user-visible reliability gap. The debounce→reconcile path is wired (tasks 6.3/6.4 done) and tests show it works after a single edit. But overflow goes unrecovered, `.quaidignore` changes require a serve restart, watcher panics kill sync permanently, and there is no native→poll fallback.

**Note on 6.5/6.6/6.7:** These tasks describe per-file targeted handlers (create/delete/rename individually), but the current `run_watcher_reconcile` already calls the full `reconcile_with_native_events` path which handles all three cases correctly via stat-diff. Do NOT add redundant per-file dispatch on top of the existing reconcile call. Tasks 6.5/6.6/6.7 can be marked `[x]` with a note that functional coverage is provided by `run_watcher_reconcile`.

### Task 6.7a — Overflow recovery worker

**File:** `src/core/vault_sync.rs`

The serve loop in `start_serve_runtime()` (around line 2795) already has `last_heartbeat`, `last_quarantine_sweep`, and `last_dedup_sweep` timers. Add a `last_overflow_recovery` timer alongside them, polling every **500ms** within the serve thread loop.

Poll logic:
```sql
SELECT id FROM collections
WHERE state = 'active' AND needs_full_sync = 1
```
For each matching collection:
1. Load `collections.active_lease_session_id` for the matching collection.
2. Call `full_hash_reconcile` with `mode = FullHashReconcileMode::OverflowRecovery` (add this variant to the mode/label enum in `reconciler.rs`) AND `authorization = FullHashReconcileAuthorization::ActiveLease { lease_session_id }` (the live serve session's persisted lease). This is NOT a new authorization variant — `ActiveLease` already exists and is the correct proof that the serve session owns the collection.
3. If `active_lease_session_id` is null or does not match the current serve session, skip with WARN `overflow_recovery_skipped_lease_mismatch`. This is the existing liveness guard — do not bypass it.
4. On success, clear `needs_full_sync = 0`. Log `overflow_recovery_complete collection=<name>` at INFO.

> **Repair note (Leela, Batch 1 repair):** The earlier draft said "add this mode variant to the authorization enum." Professor rejected that as an authorization bypass. The correct shape is: `OverflowRecovery` goes into `FullHashReconcileMode` (operation label), and authorization is `FullHashReconcileAuthorization::ActiveLease { lease_session_id }` — the same token already used by the serve-loop watcher path. The recovery worker is only honest when the serve session owns the collection lease.

**Test 17.5w:** Seed a collection with `needs_full_sync=1`, start the serve runtime, assert `needs_full_sync` becomes 0 within 1s.  
**Test 17.5x:** Same seed but with `state='restoring'` — assert the flag is NOT cleared and reconcile is NOT called.

### Task 6.8 — `.quaidignore` live reload

**File:** `src/core/vault_sync.rs`

Currently `classify_watch_event` pushes `WatchEvent::DirtyPath(relative_path)` for all markdown-file events. `.quaidignore` is not markdown, so it's currently dropped by the `relative_markdown_path` filter.

Add `WatchEvent::IgnoreFileChanged` to the enum. In `classify_watch_event`, before the markdown filter, check if any event path ends in `.quaidignore`. If so, push `WatchEvent::IgnoreFileChanged` (suppress the dirty-path fallthrough for that path). In `poll_collection_watcher`, when draining the buffer, if `IgnoreFileChanged` is set call `reload_patterns(conn, collection_id)` (already exists in `src/core/ignore_patterns.rs`) and then run a stat-diff reconcile to pick up the new ignore set.

**Test 17.5y:** Write a valid `.quaidignore` line to the watch root, emit `WatchEvent::IgnoreFileChanged` into the watcher channel, assert `collections.ignore_patterns` mirror is updated and a reconcile runs.  
**Test 17.5z:** Write a `.quaidignore` with one invalid glob line, emit the event, assert the mirror is UNCHANGED and `ignore_parse_errors` is populated.  
**Test 17.5aa:** Delete `.quaidignore` with a prior mirror, emit the event, assert WARN is logged and mirror is UNCHANGED.

### Task 6.9 — Native-first watcher with poll fallback

**File:** `src/core/vault_sync.rs`

In `start_collection_watcher`, wrap the `notify::recommended_watcher(...)` call in a match. On `Ok`, store `WatcherMode::Native`. On `Err(e)`, log `WARN: watcher_native_init_failed collection_id=<N> error=<e> falling_back_to_poll` and construct `notify::PollWatcher` instead, storing `WatcherMode::Poll`. Both paths push to the same `mpsc::channel`.

Add a `mode: WatcherMode` field to `CollectionWatcherState` and surface it via task 6.11 (CLI only). `WatcherMode` has three variants: `Native`, `Poll`, `Crashed`. There is no `Inactive` variant — collections without an active watcher entry simply return null in CLI output.

**Test 17.5aaa3:** Mock a failed native watcher init path; assert the watcher falls back to poll mode with WARN logged.

### Task 6.10 — Per-collection supervisor with crash/restart + exponential backoff

**File:** `src/core/vault_sync.rs`

Currently `sync_collection_watchers` creates watchers but if the `notify` backend crashes the watcher is never restarted. Add a `last_watcher_error` and `backoff_until: Option<Instant>` field to `CollectionWatcherState`. In `poll_collection_watcher`, catch `Err(VaultSyncError::InvariantViolation { .. })` from a disconnected channel (the existing error path). On panic/disconnect, record the failure time, set `backoff_until = Instant::now() + backoff_duration` (start at 1s, double on each consecutive failure, cap at 60s), and set the watcher state to `WatcherState::Crashed`. In `sync_collection_watchers`, skip restarting a crashed watcher until `Instant::now() >= backoff_until`.

**Test 17.5aaa4:** Simulate a watcher disconnect; assert the serve loop logs the crash, waits for backoff, and restarts the watcher.

### Task 6.11 — Watcher health in `quaid collection info` (CLI only — v0.10.0)

**Files:** `src/core/vault_sync.rs`, `src/commands/collection.rs`

> **Scope repair (Leela, Batch 1 repair — Professor rejection enforced):** The original draft proposed widening `memory_collections` with three new MCP fields. Professor rejected this: the 13.6 contract is frozen at 13 fields with an exact-key test in `src/mcp/server.rs`. Widening MCP is a breaking schema change that requires explicitly reopening 13.6 with updated design, tasks, and tests — that scope is deferred to a later batch. `memory_collections` is **NOT** widened in v0.10.0.

Add three watcher-health fields to `quaid collection info` CLI output **only**:
- `watcher_mode`: `"native"` | `"poll"` | `"crashed"` (null if collection is not active or platform is Windows — `"inactive"` is **not** a surfaced value; non-active collections surface null)
- `watcher_last_event_at`: ISO-8601 timestamp of the last event processed (null if none yet or not active)
- `watcher_channel_depth`: current number of buffered events (null if not active or Windows)

`WatcherMode` enum covers **only real runtime states**: `Native`, `Poll`, `Crashed`. The `null` rule covers both "collection not active" and "Windows platform" — there is no `Inactive` variant. The CLI output field is absent (null) for non-active collections.

Read these from `CollectionWatcherState` entries in the process-global `supervisor_handles` registry. If no registry entry exists for the collection (e.g., `quaid collection info` called outside a running serve), output null for all three fields.

`memory_collections` MCP tool is untouched in this batch — no new fields, no schema change.

---

## Batch 2 — Embedding worker (target release: 0.11)

**Tasks:** 8.1, 8.2, 8.3, 8.4, 8.5, 8.6 + 17.5ee, 17.5ff

**Why second:** Self-contained, no dependencies on batches 3–6. Without a drain worker, every `memory_put` and every reconciler re-ingest enqueues jobs that are never executed — semantic search degrades over time. Schema change is bounded (adding columns to one table).

### Task 8.1 — Extend `embedding_jobs` table schema

**File:** `src/schema.sql`, `src/core/db.rs`

The current `embedding_jobs` table has: `id`, `page_id`, `priority`, `enqueued_at`, `started_at`. Add:
- `chunk_index INTEGER NOT NULL DEFAULT 0`
- `job_state TEXT NOT NULL DEFAULT 'pending' CHECK(job_state IN ('pending','running','failed'))`
- `attempt_count INTEGER NOT NULL DEFAULT 0`
- `last_error TEXT DEFAULT NULL`

Bump `SCHEMA_VERSION` in `db.rs` from `6` to `7`. Update the comment header in `schema.sql` to "Quaid v7". The existing `open()` refusal path handles the version guard automatically.

**Important:** `raw_imports.rs` already inserts into `embedding_jobs` with only `(page_id)`. That INSERT will continue to work because all new columns have defaults. No callsite changes needed.

### Tasks 8.2, 8.3, 8.4, 8.5 — Background embedding worker

**File:** `src/core/vault_sync.rs` (add within the serve loop) or new `src/core/embedding_worker.rs`

Implement `drain_embedding_queue(conn: &Connection) -> Result<(), VaultSyncError>`:

1. Claim a batch: `UPDATE embedding_jobs SET job_state='running', started_at=now(), attempt_count=attempt_count+1 WHERE job_state IN ('pending','failed') AND attempt_count < 5 ORDER BY priority DESC, enqueued_at ASC LIMIT <concurrency>`. Use `min(available_cpus, 4)` as concurrency limit; read from `QUAID_EMBEDDING_CONCURRENCY` env var if set.
2. For each claimed job, call the existing `embed()` / `search_vec` path from `src/core/inference.rs` to regenerate chunk embeddings for the page.
3. On success: `DELETE FROM embedding_jobs WHERE id=?`.
4. On failure: `UPDATE embedding_jobs SET job_state='failed', last_error=? WHERE id=?`. After 5 attempts the job stays in `failed` state permanently.

Wire into the serve loop with a `last_embedding_drain` timer, polling every **2s**.

**Task 8.5** (startup resume): In `run_startup_sequence` (called by `start_serve_runtime`), reset all `job_state='running'` rows back to `'pending'` (they were orphaned by the previous serve crash).

**Task 8.6**: Surface `queue_depth` (count of `pending`+`running`) and `failing_jobs` (count of `failed`) in `memory_collections` and `quaid collection info`.

**Test 17.5ee:** Write a page via `memory_put`, assert `embedding_jobs` row exists, run `drain_embedding_queue`, assert job is deleted and embedding is queryable via `search_vec`.  
**Test 17.5ff:** Seed a `running` job row (simulating a crash), call the startup resume logic, assert the row resets to `pending`.

---

## Batch 3 — UUID write-back (target release: 0.12)

**Tasks:** 5a.5, 5a.5a, 9.2a + 5a.7 + 17.5ww, 17.5ww2, 17.5ww3, 17.5ii9

**Why third:** The UUID migration preflight guard (`5.8a0`, already done) blocks restore and remap for vaults where trivial-content pages lack `quaid_id` frontmatter. This batch provides the escape hatch (`migrate-uuids`). Also closes the "bulk rewrites must refuse when serve is live" requirement without needing the IPC socket.

### Task 5a.5 — Opt-in UUID write-back implementation

**Files:** `src/core/vault_sync.rs`, `src/core/page_uuid.rs`

Implement `write_quaid_id_to_file(conn: &Connection, collection: &Collection, page_id: i64) -> Result<WriteBackOutcome, VaultSyncError>`:

Uses the full rename-before-commit discipline (same as M1a seam, `12.1a`): sentinel → tempfile (with `quaid_id` injected into frontmatter via `render_page`) → `O_NOFOLLOW` defense check → dedup insert → `renameat` → `fsync(parent_fd)` → stat → single tx upsert `file_state` + rotate `raw_imports`. Read-only files (`EACCES`/`EROFS`) log WARN and return `WriteBackOutcome::SkippedReadOnly`. `pages.uuid` is always set regardless.

Gate this behind `WriteAdmin` op kind — subject to `CollectionRestoringError` interlock and `needs_full_sync` write-gate.

### Task 5a.5a — CLI: `quaid collection migrate-uuids` and `collection add --write-quaid-id`

**File:** `src/commands/collection.rs`

Add `MigrateUuids { name: String, #[arg(long)] dry_run: bool }` to `CollectionAction`. Walks all pages in the collection where `quaid_id` is absent from the file frontmatter, calls `write_quaid_id_to_file` for each. `--dry-run` reports count without mutating. Emits JSON summary: `{ migrated, skipped_readonly, already_had_uuid }`.

Add `--write-quaid-id` flag to `CollectionAddArgs`. When set, run `migrate-uuids` logic after the initial fresh-attach reconcile.

### Task 9.2a — `--write-quaid-id` on `collection add`

Wire the flag through the `collection add` handler. After `fresh_attach_reconcile_and_activate()` returns successfully, if `--write-quaid-id` is set, invoke the batch write-back loop.

### Task 17.5ii9 — Bulk writes refuse when serve is live

**File:** `src/commands/collection.rs`

In the `MigrateUuids` and `collection add --write-quaid-id` handlers, before any write-back begins, check `collection_owners` + `serve_sessions`: if any live owner (heartbeat within 15s) exists for the collection, refuse with `ServeOwnsCollectionError` naming pid/host and instructing the operator to stop serve first.

**Tests (5a.7):** Default ingest read-only; `quaid_id` adoption; opt-in rewrite rotates `file_state`/`raw_imports` atomically; `migrate-uuids --dry-run` mutates nothing; UUID write-back on EACCES skips with WARN.

---

## Batch 4 — Full rename-before-commit (target release: 0.13)

**Tasks:** 12.1 (steps 2–13, completing beyond the M1a seam), 12.6, 12.6a, 12.6b, 12.7

**Why fourth:** The current M1a write path does sentinel + tempfile + rename + parent fsync + single tx, but steps 2 (`walk_to_parent`) and 3 (`check_fs_precondition`) are only on the Unix `quaid put` path — not wired through the full 13-step spec. Task 12.2 (`check_fs_precondition`) and 12.3 (mandatory `expected_version`) are marked done but only for the Unix `quaid put`/`memory_put` path. This batch completes the remaining seams and adds CLI write routing.

### Task 12.1 — Complete the 13-step sequence

**File:** `src/core/vault_sync.rs`

Read `design.md` § "memory_put rename-before-commit write sequence" for the exact 13-step spec. The M1a seam (task 12.1a) already lands steps 5 (sentinel), 6 (tempfile), 9 (rename), 10 (fsync parent, HARD STOP), 11 (post-rename stat), 12 (single SQLite tx), 13 (unlink sentinel). Steps 2 (`walk_to_parent`), 3 (`check_fs_precondition`), 7 (symlink defense-in-depth), and 8 (dedup insert timing) must be verified to be in place on ALL vault-byte write paths (both `quaid put` CLI and `memory_put` MCP), not just the Unix path.

Audit `put_from_string` in `vault_sync.rs` against each of the 13 steps. Add any missing steps. Every failure mode in the design must be handled exactly as specified — no shortcuts.

### Task 12.6 — Mandatory `expected_version` contract everywhere

**Files:** `src/mcp/server.rs`, `src/commands/put.rs`

Audit every write entry point. For `WriteUpdate` (page already exists), `expected_version` MUST be required; the write MUST be refused with a clear error if omitted. For `WriteCreate` (page does not exist), `expected_version` MAY be omitted. Confirm this is enforced identically across MCP and CLI surfaces.

### Tasks 12.6a, 12.6b — CLI write routing

**Files:** `src/commands/put.rs`, `src/commands/collection.rs`

**12.6a** (`quaid put` single-file routing): For now (IPC not yet implemented), `quaid put` detects a live owner via `collection_owners` + `serve_sessions`. If a live owner exists, refuse with `ServeOwnsCollectionError` instructing the user to issue writes via MCP while serve is running, or stop serve and write directly. (Full proxy-over-IPC is deferred to Batch 5.) If no live owner, acquire the offline `collection_owners` lease with heartbeat and write directly.

**12.6b** (bulk rewrite routing): `quaid collection migrate-uuids` and `quaid collection add --write-quaid-id` (from Batch 3) already implement this: refuse with `ServeOwnsCollectionError` when any live owner exists. Verify the guard is in place and the error names pid/host.

### Task 12.7 — Tests

Cover every failure mode documented in `design.md` § rename-before-commit: tempfile fsync error, parent fsync error, commit error, foreign rename in the window, concurrent dedup entries, external write mid-precondition. Follow the pattern of existing 17.5k–17.5v tests in `vault_sync.rs`.

---

## Batch 5 — IPC socket (target release: 0.14)

**Tasks:** 11.9, 12.6c, 12.6d, 12.6e, 12.6f, 12.6g + 17.5ii10, 17.5ii11, 17.5ii12

**Why its own batch:** Security-critical. Kernel-PID verification, bind-time audit, and cross-UID refusal are a distinct security surface from the rename-before-commit sequence. A subtle umask bug here (like the scenario tested by 17.5ii11) would be much harder to find if buried in a large diff. Keep this batch small, focused, and peer-reviewed independently.

### Task 11.9 — UNIX socket server

**File:** `src/core/vault_sync.rs`

Read the exact socket placement spec: `$XDG_RUNTIME_DIR/quaid/` on Linux (fallback `$HOME/.cache/quaid/run/` if unset), `$HOME/Library/Application Support/quaid/run/` on macOS. Create the parent directory at mode `0700`. Socket path: `<dir>/<session_id>.sock`.

Implement in `start_serve_runtime`:
1. Create parent dir at `0700`. If it exists with broader permissions OR non-matching UID → `IpcDirectoryInsecureError`, refuse startup.
2. Unlink any stale socket at the target path before `bind()`.
3. `bind()`, set listen backlog. After bind, `stat()` the socket — verify mode `0600` and owning UID matches self. Any deviation → `IpcSocketPermissionError`, abort.
4. Write `ipc_path` to `serve_sessions` row AFTER bind+audit succeeds.
5. On shutdown, `unlink` the socket and NULL the column.
6. Server-side `accept()` loop: call `getsockopt(SO_PEERCRED)` (Linux) or `getpeereid()` (macOS) on every accepted connection. Refuse any peer whose UID ≠ serve's own UID. Log peer PID at INFO.

### Tasks 12.6c–f — Client-side IPC routing

Update `quaid put` (task 12.6a from Batch 4 used a refuse-when-live stub) to use full proxy mode:
- Stat the socket: verify mode `0600` and owning UID.
- `connect()`, then read kernel-backed peer PID+UID via `SO_PEERCRED`/`getpeereid()`.
- Verify peer UID == current UID AND peer PID == `serve_sessions.pid`.
- After passing auth, issue a protocol-level `whoami` request; cross-check the returned `session_id` against the path-embedded session_id.
- Only after all checks pass, forward the write. Any failure → `IpcPeerAuthFailedError`, NO write forwarded.

### Task 12.6g + Tests 17.5ii10–12

Write the five negative tests documented in tasks.md under 12.6g and 17.5ii10–12. These must pass before Batch 5 ships.

---

## Batch 6 — Restore/remap end-to-end (target release: 0.15)

**Tasks:** 5.8e, 5.8f, 5.8g + 17.5ii4, 17.5ii5, 17.5pp, 17.5qq through 17.5qq9 + 4.6, 5.4f

**Dependencies:** Requires Batch 3 (UUID write-back) to be complete and stable. The UUID migration preflight guard (`5.8a0`) blocks Phase 4 for vaults with trivial-content pages without `quaid_id` — Batch 3's `migrate-uuids` command resolves that.

**Context:** Phases 0–3 (preflight, RO-mount gate, dirty-preflight guard, Phase 1 drift capture, Phase 2 stability, Phase 3 fence) are already implemented (tasks 5.8a0–5.8d2 done). This batch lands Phase 4 (remap bijection verification), the full online restore handshake (with live-serve coordination), and the offline remap path.

### Tasks 5.8e — Phase 4 remap verification

Read `design.md` § "Two-phase defense" for the bijection spec. Implement `verify_new_root_bijection(conn, collection_id, new_root_path)`:
- Use `resolve_page_identity(...)` (UUID-first, then content-hash uniqueness with `size>64` and non-empty body guards — the same helper used by the reconciler in task 5.3; see task 17.17a for the unification requirement).
- Three failure modes: missing pages → `NewRootVerificationFailedError`; sha256 mismatch → same error; extra unmatched files → same error. Include counts and sampled diffs in the error.
- Full-tree stability fence (`newroot_snap_pre` vs `newroot_snap_fence`) — any file-set / per-file-tuple / `.quaidignore`-sha256 drift between verification and DB-update → `NewRootUnstableError`.
- Quarantined pages are excluded from both sides of the bijection check.

**Test 17.5ii4:** Missing, mismatch, and extra each produce correct errors.  
**Test 17.5ii5:** Non-zero drift in remap Phase 1 refuses with `RemapDriftConflictError`; second pass after operator edit succeeds.

### Tasks 5.8f, 5.8g — Online and offline end-to-end paths

Wire Phase 4 into `quaid collection sync --remap-root` (offline) and into the online handshake path. Reference the spec in `design.md` § "Live-serve coordination for restore/remap".

**Tests 17.5pp, 17.5qq–qq9:** Cover ack triple matching, stale ack rejection, serve-died-during-handshake timeout, do-not-impersonate at startup, remap online DB-only tx, remap offline CLI reconcile, and the other cases listed in tasks.md.

### Tasks 4.6, 5.4f — Background sweeps

Add `QUAID_FULL_HASH_AUDIT_DAYS` polling (default 7 days) to the serve loop. Add `QUAID_RAW_IMPORTS_TTL_DAYS` daily background sweep for TTL-expired inactive `raw_imports` rows. Both are straightforward additions to the existing timer pattern in `start_serve_runtime`.

---

## Batch 7 — Cleanup + docs (target release: 0.16)

**Tasks:** 9.6, 10.1–10.3, 14.1–14.2, 15.1–15.4, 16.1–16.8, 17.1, 17.4, 17.5 (integration group), 17.6–17.10, 17.14–17.15, 17.17a/c, 18.1–18.2

**Note on §15:** `src/commands/import.rs` MUST NOT be deleted until the documentation tasks (§16) are complete in the same batch. The §15.4 constraint is self-enforcing — do not merge import.rs removal before the README and getting-started.md changes are ready.

### Task 9.6 — `quaid collection remove`

**File:** `src/commands/collection.rs`

Add `Remove { name: String, #[arg(long)] purge: bool }` to `CollectionAction`. Without `--purge`: set `state='detached'`, leave page rows. With `--purge`: require explicit confirmation (prompt or `--confirm` flag), then `DELETE FROM pages WHERE collection_id=?` in a transaction (cascades to FTS, embeddings, file_state, raw_imports via FK ON DELETE CASCADE). Refuse if `state='restoring'`.

### Tasks 10.1–10.3 — `quaid init` cleanup

Verify `quaid init` correctly writes `schema_version=<current>` and removes any import-related bootstrap logic. Document that vault paths are attached via `quaid collection add` post-init.

### Tasks 14.1–14.2 — `quaid stats` update

Augment `quaid stats` output with per-collection rows (name, page_count, queue_depth, last_sync_at, state, writable) and aggregate totals (pages across all collections, quarantined count, embedding jobs pending/failed).

### Tasks 15.1–15.4 — Remove legacy ingest

1. Delete `src/commands/import.rs`
2. Delete `import_dir()` from `src/core/migrate.rs`; split remaining logic between `reconciler.rs` and `vault_sync.rs` as needed
3. Drop `ingest_log` table from schema (bump schema version)
4. **Only merge after §16 docs are complete**

### Tasks 16.1–16.8 — Documentation

Update `README.md`, `docs/getting-started.md`, `docs/spec.md`, `AGENTS.md`, all `skills/*/SKILL.md` files, `CLAUDE.md`, and `docs/roadmap.md` as specified. Document all `QUAID_*` env vars. Document the five DB-only-state categories and quarantine resolution flow.

### Remaining tests (17.x)

Fill in the integration test gaps: schema table/index audit (17.1), `.quaidignore` atomic parse (17.4), full collection lifecycle (17.5), performance budget (17.6), watcher 2s latency (17.7), embedding eventual consistency (17.8), restore round-trip bytes (17.9), online restore with live serve (17.10), git checkout overflow (17.14), multi-collection slug collisions (17.15), resolver unification unit proof (17.17a), `raw_imports_active_singular` invariant (17.17c).

### Tasks 18.1–18.2 — Follow-up stubs

Create `openspec/changes/daemon-install/proposal.md` and `openspec/changes/openclaw-skill/proposal.md` stubs as specified.

---

## Cross-cutting instructions for all batches

1. **Read `design.md` before implementing any vault-sync primitive.** The decisions section documents rejected alternatives — do not reintroduce them.
2. **Read the relevant spec file** (`specs/agent-writes/`, `specs/collections/`, `specs/vault-sync/`) for the precise acceptance criteria before writing tests.
3. **Do not widen platform gates.** Vault-byte operations remain `#[cfg(unix)]`. DB-only operations are always cross-platform.
4. **Error variant naming:** Follow existing patterns (`CollectionRestoringError`, `UnsupportedPlatformError`, etc.). Add new variants to `VaultSyncError` in `vault_sync.rs`.
5. **Env var naming:** All env vars follow `QUAID_*` convention. Document each new one added.
6. **Mark tasks immediately** in `tasks.md` as each task is completed — use the closure note format already established (brief description of what was implemented and any scope limitations).