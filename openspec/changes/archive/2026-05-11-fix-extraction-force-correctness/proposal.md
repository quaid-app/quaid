## Why

An adversarial review of `fix/issue-162-wireup-worker` (`docs/CODE_REVIEW.md` second pass) surfaced two correctness bugs and one resilience gap in the conversation-extraction path. `quaid extract <session> --force` silently leaves historical day-files unprocessed, the cursor-reset path can clobber concurrent turn appends, and the queue's manual transaction wrapper can wedge the shared SQLite connection on a `COMMIT` failure. The first two are user-visible — incomplete re-extraction and source-file data loss respectively — and block shipping until repaired.

## What Changes

- Make `extract --force <session>` enqueue **every** day-file in the session (chronological order), not only the latest, so a forced reset actually rebuilds the full session.
- Make `reset_cursors` acquire the same per-session in-process mutex and on-disk `SessionFileLock` that `turn_writer::append_turn` uses before rewriting any day-file. No conversation-markdown writer in the codebase may bypass this discipline.
- Replace `with_immediate_transaction` in `core/conversation/queue.rs` with a wrapper that guarantees rollback on any commit failure (preferring `rusqlite`'s RAII `Transaction` with `TransactionBehavior::Immediate`), so a failed `COMMIT` cannot leave the shared `Connection` stuck mid-transaction.
- Update CLI / worker tests that currently codify "reset cursors + enqueue latest only" to instead assert full-session re-extraction and lock-respecting rewrites. Add a fault-injection regression test covering commit failure.

No CLI surface, MCP surface, on-disk format, or migration changes — these are internal correctness fixes against existing requirements.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `extraction-queue`: tightens the manual / `--force` enqueue contract so a session reset enqueues all day-files, and pins down transaction-atomicity for queue operations under commit failure.
- `conversation-turn-capture`: extends the same-session serialization requirement to cover **all** writers of conversation day-files, not only `memory_add_turn`. Cursor-reset and any future admin path must take the same lock.

## Impact

- Code: `src/commands/extract.rs` (force path + cursor reset), `src/core/conversation/queue.rs` (`with_immediate_transaction`), `src/core/conversation/turn_writer.rs` (expose lock-acquiring helper if needed for reuse).
- Tests: `tests/cli_extract.rs` (update `--force` assertions), `tests/extraction_worker.rs` (multi-day-file re-extraction), new regression for queue commit-failure recovery, new regression for `--force` racing against `memory_add_turn`.
- Behavior: `extract --force` against multi-day sessions now re-processes the full session (visible to operators); concurrent `extract --force` + `memory_add_turn` is now safe instead of racy.
- No schema, MCP-tool, or vault-format changes. No new dependencies.
- Risk: low. All three fixes are local; the lock change reuses existing primitives. The `--force` semantics change is the only user-observable behavior shift, and it brings the implementation in line with its documented intent.
