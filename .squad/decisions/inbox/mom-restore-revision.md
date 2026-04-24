# Mom — Quarantine Restore Second Revision (5-Blocker Fix)

## Context

Fry authored the quarantine restore slice. Professor/Nibbler rejected it. Bender authored
a first partial fix. That fix was itself rejected on 5 consolidated blockers. Mom assigned
as second revision author.

## Decisions

### D-R1: Pre-install tempfile cleanup on write/sync failure

**Blocker:** `write_all`/`sync_all` called with bare `?` on the tempfile. Any failure leaves
a `.quarantine-restore-*.tmp` orphan in the vault directory.

**Decision:** Wrap `write_all(...)?; sync_all(...)?` in an `if let Err` block. On failure,
call `cleanup_tempfile` before returning the write error. If cleanup also fails, return the
cleanup error (matches the pattern established in the existing install-failure path).

**Consequence:** No tempfile residue on any write or sync failure. Secondary cleanup failure
takes precedence in error reporting — honest about the worst-case state.

### D-R2: Parse-failure rollback after install

**Blocker:** After `linkat` installs the target, `parse_restored_page` is called with bare `?`.
If parse fails (UUID conflict, JSON serialization error), the target stays on disk while
the page remains quarantined in DB. Inconsistent state, no rollback.

**Decision:** Wrap `parse_restored_page(...)` in an explicit `match`. On `Err`, call
`rollback_target_entry(...)` (the existing rollback helper) before returning the parse error.

**Consequence:** Any post-install work failure puts the vault back to pre-install state.
Install is atomic from the caller's perspective.

### D-R3: Refuse absent parent directories (don't silently create them)

**Blocker:** `walk_to_parent_create_dirs` creates the parent directory chain on restore, but
never fsyncs the newly created directories. Crash after directory creation but before
vault bytes are written leaves an unreachable empty tree.

**Options considered:**
- A: add fsync chain after `walk_to_parent_create_dirs`
- B: switch to `walk_to_parent` (no-create) and refuse absent parents

**Decision:** Option B — switch to `walk_to_parent`. Restore should never need to create
parent directories; those directories existed when the page was originally vaulted.
If they're absent, something has gone wrong at a higher level and restore should surface
that rather than silently reconstructing the tree without durable persistence.
`walk_to_parent_create_dirs` is left in `fs_safety.rs` for future callers that genuinely
need to create directory chains.

**Consequence:** Restore fails with a clear `IoError` if the parent directory is absent.
No silent directory creation, no fsync gap. The narrowest truthful fix for this blocker.

### D-R4: tasks.md 9.8 non-contradiction

**Blocker:** Task 9.8 body said "restore remains deferred in this batch" while the closure
note attributed to Fry said restore was re-enabled. The two statements were directly
contradictory.

**Decision:** Rewrite task 9.8 body to accurately list the current surface
(`{list,discard,export,restore}`) and name the current restore gates (absent-parent refusal,
no-replace install, crash-durable rollback). Replace the Fry-attributed note with a
Mom-attributed repair note.

**Consequence:** tasks.md now accurately describes the contract. No reader needs to
reconcile contradictory statements.

### D-R5: Narrow contract preserved

**Scope:** No watcher integration, no audit-trail widening, no overwrite policy, no
live-owner gate modifications were added. All four code changes are surgical to the
exact failing path.

## Tests added

- `restore_cleans_up_tempfile_when_write_fails` — GBRAIN_TEST_QUARANTINE_RESTORE_FAIL_AFTER_TEMPFILE_CREATE=1 hook
- `restore_rolls_back_target_when_parse_fails_after_install` — GBRAIN_TEST_QUARANTINE_RESTORE_FAIL_IN_PARSE=1 hook
- `restore_refuses_absent_parent_directory` — directly passes a missing-parent target path

## Validation

591 lib tests pass. 2 pre-existing Windows-only failures
(`init_rejects_nonexistent_parent_directory`, `open_rejects_nonexistent_parent_dir`)
confirmed present on baseline before any changes. Zero new failures introduced.
