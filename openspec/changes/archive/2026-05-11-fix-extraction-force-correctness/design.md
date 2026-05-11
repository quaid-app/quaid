## Context

`fix/issue-162-wireup-worker` wired the extraction worker into the live request path. A subsequent adversarial review (`docs/CODE_REVIEW.md` second pass, `b95cg1cgh`) found three latent defects in the surrounding code:

1. `src/commands/extract.rs:67-80` — `--force` resets every day-file's cursor but only enqueues `target.latest_relative_path`. `process_job` in `src/core/conversation/extractor.rs:257-293` operates on a single day-file per job, so non-latest day-files of multi-day sessions are reset to cursor `0` and never re-extracted. The watcher does not retroactively enqueue files whose mtime did not change, so they remain stuck.
2. `src/commands/extract.rs:151-159` — `reset_cursors` rewrites each day-file with `fs::write` after parsing it, with no synchronization. `turn_writer::append_turn` (`src/core/conversation/turn_writer.rs:75-81, 155-160`) takes both an in-process per-session mutex and an on-disk `SessionFileLock` before mutating. A concurrent `memory_add_turn` between `reset_cursors`'s parse and write loses the appended block on disk.
3. `src/core/conversation/queue.rs:307-321` — `with_immediate_transaction` only runs `ROLLBACK` on closure failure. On success, a `COMMIT TRANSACTION` failure returns the error directly without rollback. SQLite auto-rolls back many commit failures, but `SQLITE_BUSY` does not, leaving the connection inside a transaction. Because the queue uses a shared `Connection`, subsequent `BEGIN IMMEDIATE` calls would error with "cannot start a transaction within a transaction".

All three are in code touched by, or directly adjacent to, the wired-up worker, so the same release that activates the worker should also fix them.

## Goals / Non-Goals

**Goals:**

- `extract --force` faithfully re-extracts every day-file of the targeted session, in chronological order, through the existing job pipeline.
- All conversation-day-file writers — `memory_add_turn`, `memory_close_session`, the CLI cursor-reset, and any future admin path — share a single locking discipline (per-session in-process mutex + on-disk advisory file lock).
- The queue's transaction wrapper guarantees rollback on any transaction failure, including failed `COMMIT`, so the shared SQLite connection cannot be wedged.
- Each fix carries a regression test that fails on `main` and passes after the change.

**Non-Goals:**

- No CLI flag, MCP tool surface, on-disk format, frontmatter key, or schema migration changes.
- No restructuring of `src/core/conversation/extractor.rs::process_job` to accept multiple day-files in a single job — we use the existing single-file contract and enqueue N jobs instead.
- No change to lease-expiry behavior, retry counts, or any other queue semantics outside the transaction wrapper.
- No broader refactor of the items called out in `docs/CODE_REVIEW.md` §1–§7 (file-size monoliths, error-type splitting, lints table). Those remain follow-up work, deliberately separated from this correctness fix per Codex's prioritization critique.

## Decisions

### 1. `--force` enqueues one job per day-file, not a session-level job

**Decision:** In `src/commands/extract.rs::run`, when `args.force` is set, replace the single `queue::enqueue(&target.latest_relative_path, ...)` call with a loop over `target.day_files` (already sorted ascending by date in `discover_sessions`) that issues one `queue::enqueue(&day_file.relative_path, ExtractionTriggerKind::Manual, scheduled_for)` per day-file.

**Alternatives considered:**

- *Session-level job processed by a new worker contract.* Would require teaching `process_job` to walk multiple day-files, propagate errors mid-walk, and update each cursor independently — significant surface area for an admin code path. Rejected: the existing per-day-file contract is sufficient.
- *Detect "needs re-extraction" in the worker by re-scanning when cursor is `0`.* Would couple the worker to filesystem scans and break the invariant that the queue alone schedules work. Rejected.

**Why one-per-day-file enqueue:**

- Reuses the existing `pending → running → done` lifecycle and lease-expiry recovery.
- Per-day-file `enqueued_at`/`scheduled_for` ordering preserves chronological re-extraction even under interleaving with debounced jobs.
- Enqueue is already idempotent within a session via the UPSERT/collapse rule (`extraction-queue` spec, "Enqueue UPSERTs collapse pending jobs per session"), so back-to-back forced resets do not double-up pending rows.
- Interaction with the collapse rule: the rule collapses pending rows on `(session_id)` only, but each day-file maps to a distinct `conversation_path` within that session. Verify the existing UPSERT key includes `conversation_path` (it does in current schema; if not, a fix would extend the collapse to be `(session_id, conversation_path)` — confirm during implementation).

### 2. `reset_cursors` reuses `turn_writer`'s session lock primitives

**Decision:** Extract a small public helper from `turn_writer` of the form `with_session_file_lock(root, namespace, session_id, |&mut ConversationFile| -> Result<()>)` that acquires the in-process `session_lock` and on-disk `SessionFileLock`, parses the file, runs the closure, renders, and writes — all under the same locks `append_turn` holds. Rewrite `reset_cursors` in terms of this helper. Existing `append_turn` may be refactored onto the helper later, but only if mechanical; this change scope is locking, not deduplication.

**Alternatives considered:**

- *Refuse `--force` for sessions with any open day-file.* Rejected as the primary mechanism: it papers over the bug by restricting the surface, but the underlying race (any concurrent writer) still applies. Kept as a *secondary* requirement (`Cursor reset is forbidden for sessions with an in-flight writer`) so the implementation MAY surface a clear error if it cannot acquire the lock within a bounded wait.
- *Take only the file-system lock, skip the in-process mutex.* Rejected: same-process concurrent writers (e.g. `quaid serve` running in another thread) would still race. Both primitives are needed.

### 3. Add explicit rollback after a failed commit, keep `&Connection`

**Decision (revised during implementation):** Patch `with_immediate_transaction` so the success branch attempts a `ROLLBACK TRANSACTION` if `COMMIT TRANSACTION` returns an error, then surfaces the commit error to the caller. Keep the `&Connection` signature, so no caller needs to thread a mutable borrow.

```rust
match action(conn) {
    Ok(value) => match conn.execute_batch("COMMIT TRANSACTION") {
        Ok(()) => Ok(value),
        Err(commit_error) => {
            // SQLITE_BUSY on COMMIT does not auto-rollback.
            let _ = conn.execute_batch("ROLLBACK TRANSACTION");
            Err(ExtractionQueueError::from(commit_error))
        }
    },
    Err(error) => {
        let _ = conn.execute_batch("ROLLBACK TRANSACTION");
        Err(error)
    }
}
```

**Why this rather than `rusqlite::Transaction`:**

This change was originally specced as "use `rusqlite::Connection::transaction_with_behavior(TransactionBehavior::Immediate)` and let RAII handle rollback." That API requires `&mut Connection`. Auditing the call graph during implementation showed `enqueue` is reached through `&Connection` everywhere — `src/mcp/server.rs` holds `db` via a `Mutex<Connection>` and dereferences immutably; `src/core/conversation/idle_close.rs` and `src/commands/extract.rs` likewise pass `&Connection`. Switching to `&mut Connection` would cascade through those modules' public APIs and the MCP tool surface, which is out of scope for a correctness fix.

The Codex finding itself called out both options as acceptable: "Replace the manual SQL transaction wrapper with `rusqlite`'s transaction API using `TransactionBehavior::Immediate`, **or** explicitly attempt `ROLLBACK` when `COMMIT` fails before returning the error." The explicit-rollback path produces the same observable behavior with a much smaller blast radius.

**Alternatives considered:**

- *Switch to `rusqlite::Transaction` (original plan).* Cleaner with RAII, but cascades `&mut Connection` through every queue caller. Rejected because the change scope is correctness-only; refactoring queue callers' borrows is follow-up work.

**Test:** Deterministic commit-failure injection via SQLite's `commit_hook` (rusqlite `hooks` feature, enabled in `Cargo.toml`). The test registers a hook that aborts the next commit, runs `enqueue`, asserts it returns `Err`, clears the hook, then runs another `enqueue` and asserts it succeeds — proving the connection was not left wedged inside an open transaction. Without the rollback fix, the second enqueue's `BEGIN IMMEDIATE` errors with "cannot start a transaction within a transaction".

### 4. Test strategy

- **Bug 1 regression:** New test in `tests/cli_extract.rs` that creates a multi-day session, runs `extract <id> --force`, then drains the queue and asserts every day-file's `last_extracted_turn > 0`.
- **Bug 2 regression:** New test that holds `SessionFileLock` from a helper thread, invokes `reset_cursors` on the same session, and asserts the cursor-reset path waits (or fails fast, depending on chosen semantics) rather than writing while the lock is held.
- **Bug 3 regression:** New test in `tests/extraction_queue_*.rs` that wraps the connection in a fault-injecting handle which forces `COMMIT` to fail, runs an enqueue, and then verifies that a follow-up enqueue on the same connection succeeds. SQLite has no built-in commit-failure injection; the practical approach is to drop a transaction without committing (closure returns error) and then verify recovery, plus a pure unit test on the wrapper that asserts no transaction is active after a forced commit failure (exercise via a closing connection or by issuing a conflicting `BEGIN` and observing it succeeds).

## Risks / Trade-offs

- **Risk:** N-job enqueue for `--force` increases queue churn for very long sessions (hundreds of day-files). **Mitigation:** Multi-day quaid sessions are typically a small handful of files; the queue handles N=20 trivially. If this becomes a concern, batch by date range later.
- **Risk:** Extracting a session-locking helper from `turn_writer` exposes more API surface. **Mitigation:** Keep the helper `pub(crate)` and constrained to "open-parse-mutate-write under lock"; do not expose lock handles directly.
- **Risk:** Switching to `rusqlite::Transaction` changes the borrow signature from `&Connection` to `&mut Connection` for callers. **Mitigation:** All queue write callers already path through the queue module; the change is local. Verify no caller holds a long-lived `&Connection` shared with another transaction concurrently.
- **Risk:** `rusqlite::Transaction`'s implicit Drop-rollback can mask bugs by silently swallowing rollback failures. **Mitigation:** Same risk as the existing `let _ = ...ROLLBACK...` line; behavior is no worse than today, and the `commit()` path now correctly surfaces the error to the caller.
- **Trade-off:** We deliberately do not address `docs/CODE_REVIEW.md` §1–§7 in this change. The review's structural recommendations (file splits, lints table, error-type decomposition) are valid follow-up work; bundling them here would defeat the prioritization critique that motivated this change in the first place.

## Migration Plan

No data migration. No flag rollout. The change ships as one logical commit (or a small stack), each fix landing with its regression test. Rollback is `git revert` of the change; the cursor-reset CLI continues to function but exhibits the original bugs.
