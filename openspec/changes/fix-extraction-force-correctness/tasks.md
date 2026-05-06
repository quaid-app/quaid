## 1. Bug 1 — `extract --force` enqueues every day-file

- [ ] 1.1 Add a failing test in `tests/cli_extract.rs` that creates a session with day-files dated `2026-05-03`, `2026-05-04`, and `2026-05-05`, runs `extract <id> --force`, drains the queue, and asserts every day-file's `last_extracted_turn > 0` after drain.
- [ ] 1.2 Verify the existing UPSERT collapse rule in `src/core/conversation/queue.rs` keys on `(session_id, conversation_path)` and not just `session_id`. If it collapses N forced jobs into one, extend the collapse key (or document why same-session different-paths must coexist as multiple pending rows).
- [ ] 1.3 In `src/commands/extract.rs::run`, replace the single `queue::enqueue(... &target.latest_relative_path ...)` call inside `if args.force` with a loop over `target.day_files` (already chronological after `discover_sessions` sort) issuing one `queue::enqueue` per day-file. Keep the non-force path unchanged.
- [ ] 1.4 Update or remove the existing CLI test that codifies "reset cursors + enqueue latest only" so the suite no longer documents the bug as desired behavior.
- [ ] 1.5 Run the test from 1.1 and confirm it now passes.

## 2. Bug 2 — `reset_cursors` acquires the session lock

- [ ] 2.1 Add a failing test that holds the `SessionFileLock` for `<session>` from a helper thread, invokes `reset_cursors` against the same session, and asserts the call either blocks until the helper releases the lock or exits with a clear error — but does not write while the lock is held.
- [ ] 2.2 In `src/core/conversation/turn_writer.rs`, expose a `pub(crate) fn with_session_file_lock<F>(root, namespace, session_id, F)` helper that acquires the in-process `session_lock` and the on-disk `SessionFileLock`, parses the day-file, runs the closure with `&mut ConversationFile`, renders, and writes. Match the lock-acquisition order used by `append_turn`.
- [ ] 2.3 Rewrite `reset_cursors` in `src/commands/extract.rs` to call the new helper for each day-file instead of `format::parse` + `fs::write`.
- [ ] 2.4 Decide and implement the bounded-wait vs fail-fast policy in 2.1 (default: bounded wait via the existing `SessionFileLock::acquire`; on lock-acquire failure, return a `bail!` naming the contended day-file). Document the choice as a one-line code comment if non-obvious.
- [ ] 2.5 Run the test from 2.1 and confirm it now passes.

## 3. Bug 3 — Queue transactions roll back on commit failure

- [ ] 3.1 Add a failing unit test in `src/core/conversation/queue.rs` (or a sibling test file) that exercises the transaction wrapper such that `COMMIT` fails. Practical injection: open a connection, invoke the wrapper with a closure that succeeds; in a separate thread (or by closing the underlying connection mid-flight) force the commit to error, then assert that a follow-up wrapper invocation on a fresh connection succeeds and that no transaction is observed open.
- [ ] 3.2 Replace `with_immediate_transaction` with the `rusqlite::Transaction` + `TransactionBehavior::Immediate` form: `let tx = conn.transaction_with_behavior(...)?; let value = action(&tx)?; tx.commit()?; Ok(value)`. Adjust callers from `&Connection` to `&mut Connection` where required.
- [ ] 3.3 Update all call sites in `src/core/conversation/queue.rs` (enqueue, dequeue, mark_done, mark_failed, lease recovery, etc.) to compile against the new signature and to pass `&Transaction` into helper queries that previously took `&Connection`.
- [ ] 3.4 Confirm `rusqlite::Connection::transaction_with_behavior` is available in the current `rusqlite` version pinned in `Cargo.toml`. If not, add the necessary feature flag or fall back to `Transaction::new_unchecked` with explicit behavior.
- [ ] 3.5 Run the test from 3.1 and confirm it now passes.

## 4. Cross-cutting verification

- [ ] 4.1 Run `cargo fmt --all` and `cargo clippy --all-targets --all-features` and address any warnings the change introduces.
- [ ] 4.2 Run `cargo test` and confirm all tests pass, including pre-existing extraction and conversation tests.
- [ ] 4.3 Manually verify the multi-day `--force` flow against a local vault with 2+ day-files: confirm each file's `last_extracted_turn` advances past 0 after `quaid extract <id> --force` and the worker drains.
- [ ] 4.4 Update `docs/CODE_REVIEW.md` to note that the three high/medium correctness findings are addressed in this change (or add a short note to the change directory's `proposal.md` if `docs/CODE_REVIEW.md` is treated as immutable history).
- [ ] 4.5 Commit with a message referencing the originating Codex finding IDs and the OpenSpec change name.
