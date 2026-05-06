## ADDED Requirements

### Requirement: `memory_put` write sequence — rename-before-commit (vault is authoritative)

When an agent (or CLI `quaid put`) writes content, the DB state SHALL NOT advertise new bytes until the rename is durable on disk. The SQLite commit happens AFTER the atomic rename, not before. This ordering is required so that a crash or process-kill during the write never leaves the DB claiming content that the vault does not hold — which would otherwise make a later `quaid collection restore` materialize bytes that never existed on disk (the "ghost content" risk). Because rename is the commit-to-vault step, and the DB tx is the commit-to-index step, ordering rename before the tx gives us the single clean invariant: **DB state never leads disk state**.

The exact sequence:

1. Parse + validate slug; resolve collection; enforce `expected_version` discipline (CAS check against current `pages.version` BEFORE any filesystem work — updates without matching version return `ConflictError`). If `collections.writable = 0`, refuse with `CollectionReadOnlyError`.
2. `walk_to_parent(root_fd, relative_path, create_dirs=true)` → trusted `parent_fd`.
3. `check_fs_precondition(parent_fd, target_name)` per the "Filesystem precondition" requirement; any mismatch resolves via hash-verify or returns `ConflictError`.
4. Compute new `sha256` of content.
5. **Create recovery sentinel** — generate `write_id` as a UUIDv7; call `openat(recovery_dir_fd, "<write_id>.needs_full_sync", O_CREAT | O_EXCL | O_NOFOLLOW, 0600)` followed by `fsync(recovery_dir_fd)`. The sentinel is the durable "please reconcile" marker that survives SQLite-unavailable failure modes (see the sentinel backstop requirement). Sentinel creation failure aborts the write HERE with `RecoverySentinelError` — no tempfile, no dedup entry, no vault mutation.
6. Create tempfile via `openat(parent_fd, tempfile_name, O_CREAT | O_EXCL | O_WRONLY | O_NOFOLLOW | O_CLOEXEC)`; write content; `fsync(tempfile_fd)`; call `fstat(tempfile_fd)` and capture `tempfile_inode` BEFORE close — this is the inode `renameat` will preserve at the target name; close.
7. Defense-in-depth: `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)` — if an entry exists at the target name AND is a symlink, unlink the tempfile and return `SymlinkEscapeError`.
8. Insert `(target_path, new_sha256, now)` into the self-write dedup set.
9. Atomically rename via `renameat(parent_fd, tempfile_name, parent_fd, target_name)`. Both names scoped to the trusted `parent_fd`; no path lookup at rename.
10. `fsync(parent_fd)` to ensure the rename is durable on the underlying filesystem before the DB is updated to reflect it. After this line returns, the vault holds the new bytes durably.
11. `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)` → read the COMPLETE post-rename stat tuple `(mtime_ns, ctime_ns, size_bytes, inode)`. All fields including `ctime_ns` are known now. **Inode-hijack guard (; CODEX-F2 sentinel-retention clarification):** verify `stat.inode == tempfile_inode` captured at step 6. POSIX `renameat` preserves the source inode at the destination, so any mismatch means a foreign rename landed at `target_name` between step 9 and step 11 — another actor's bytes now occupy the path while our hash/bytes sit in-memory. Abort with `ConcurrentRenameError` under the **universal post-rename abort contract**: any abort on or after step 10 (rename is durable on disk) SHALL follow the same rule — (a) REMOVE the dedup entry, (b) **LEAVE the sentinel in place** so recovery re-ingests whatever bytes actually occupy `target_name`, (c) best-effort via a FRESH SQLite connection set `collections.needs_full_sync = 1` as an optimization for fast live recovery (not required for correctness because the sentinel is primary and survives even if this write fails), (d) do NOT commit Tx-B. The sentinel is unlinked ONLY at step 13 after Tx-B commits successfully. This closes the narrow external-rename-over race that could otherwise persistently desync `pages.sha256` (ours) from `file_state.inode`/on-disk bytes (theirs) — without the sentinel-retention rule, a concurrent rename combined with a failing fresh-connection `needs_full_sync` write would leave the DB at the old version while the vault holds foreign bytes and startup recovery would have nothing guaranteed to notice.
12. **SINGLE SQLite transaction**: upsert `pages` (version++); trigger FTS; upsert `file_state` with the FULL stat tuple — no NULL `ctime_ns`, no provisional state; rotate `raw_imports` (UPDATE prior active row `is_active = 0`; INSERT new row `is_active = 1` with the bytes just renamed in); enqueue `embedding_jobs`. COMMIT.
13. Best-effort unlink recovery sentinel — `unlinkat(recovery_dir_fd, "<write_id>.needs_full_sync", 0) + fsync(recovery_dir_fd)`. Unlink failure is logged at WARN but DOES NOT fail the write; the worst outcome is a spurious reconcile on next startup, which is over-correction, not corruption.

Failure recovery (exhaustive — one authoritative matrix):

- **Steps 1–4 fail** (validation, walk, precondition, hash compute): no sentinel, no filesystem or DB mutation. Return error.
- **Step 5 fails** (sentinel create): no tempfile, no vault mutation, no DB mutation; return `RecoverySentinelError`. The recovery directory should exist from `quaid collection add`; its absence indicates data-directory corruption and warrants loud error.
- **Step 6 fails** (tempfile create/write/fsync): sentinel has been created and SHALL be unlinked in the cleanup path (`unlinkat(recovery_dir_fd, "<write_id>.needs_full_sync", 0)`) since the vault never mutated — retaining the sentinel would cause a spurious but harmless reconcile. Best-effort `unlinkat(parent_fd, tempfile_name, 0)` for the tempfile. No DB state. Return error.
- **Step 7 rejects** (symlink at target): same as step 6 — unlink tempfile AND unlink sentinel (no vault mutation); no DB state. Return `SymlinkEscapeError`.
- **Step 8 fails** (mutex poisoning, OOM during dedup insert): unlink tempfile AND unlink sentinel (no vault mutation yet); no DB state. Return error.
- **Step 9 fails** (rename I/O error): dedup entry was inserted at step 8; REMOVE it. Unlink tempfile. **Unlink the sentinel** — POSIX `renameat` is atomic and on failure leaves the target untouched, so there is no drift to recover from. No DB state. Return error.
- **Step 10 fails** (`fsync(parent_fd)` I/O error — rename landed on the page cache but may NOT be durable across a power loss): **DO NOT proceed to step 12; DO NOT commit the SQLite tx**. Committing when durability is unconfirmed would reintroduce the /"DB advertises bytes the vault does not hold" class of failure. The handler SHALL: (a) REMOVE the dedup entry (so the watcher/reconciler, if serve is running, can re-process whatever disk actually holds post-recovery); (b) **LEAVE the sentinel in place** as the durable recovery marker; (c) best-effort via a FRESH SQLite connection set `collections.needs_full_sync = 1` (optimization for fast live recovery; not required for correctness because the sentinel is primary); (d) log `memory_put_fsync_parent_failed collection=<N> slug=<S> errno=<E>` at ERROR; (e) return `DurabilityError`. On the next `quaid serve` start OR the next recovery cycle (1s cadence), startup sweep or the live recovery task observes the sentinel (and, if the fresh-connection write succeeded, also `needs_full_sync = 1`), runs `full_hash_reconcile`, ingests whatever is actually on disk, and unlinks the sentinel only after reconcile commits successfully.
- **Step 11 fails** (fstatat I/O error after successful rename + fsync): the bytes are on disk and durable, but we cannot capture the stat tuple needed for `file_state`. Same hard-failure discipline as step 10: remove dedup, **leave sentinel**, best-effort set `needs_full_sync = 1` via fresh connection, return `PostRenameStatError`. Recovery re-ingests from disk and populates `file_state` via the normal walk.
- **Step 11 inode-hijack guard fires** (`fstatat.inode != tempfile_inode`): our tempfile's rename landed at step 9, but a foreign actor renamed a DIFFERENT file over `target_name` between step 9 and step 11. The DB has NOT committed yet. The handler SHALL: (a) REMOVE the dedup entry (our `(path, our_sha256)` entry would otherwise suppress a watcher event for the foreign bytes); (b) **LEAVE the sentinel in place** — disk state is ambiguous from our perspective and recovery MUST run; (c) best-effort via a FRESH SQLite connection set `collections.needs_full_sync = 1` so the reconciler re-ingests whatever actually holds the path; (d) log `memory_put_inode_hijack collection=<N> slug=<S> tempfile_inode=<I_T> target_inode=<I_A>` at ERROR; (e) return `ConcurrentRenameError` to the caller. The reconciler (live task or startup sweep) then hashes the current `target_name` bytes and re-ingests — the DB never advertises our hash for their bytes, which closes the GEMINI-F1 metadata-hijack class.
- **Step 12 fails** (SQLite constraint violation, disk full on WAL, `SQLITE_IOERR`, etc.): disk has new bytes; DB is unchanged. The handler SHALL: (a) REMOVE the dedup entry; (b) **LEAVE the sentinel in place**; (c) best-effort via a FRESH SQLite connection set `collections.needs_full_sync = 1`. **If the fresh-connection write ALSO fails** (the same underlying fault that failed the main tx may still be active), that is acceptable — the sentinel remains on disk and startup sweep will catch it. Return the underlying SQLite error. The live recovery task (on next successful SQLite open) OR the startup sweep runs `full_hash_reconcile` and unlinks the sentinel after reconcile commits.
- **Step 13 fails** (sentinel unlink after successful commit): the write SUCCEEDED. Log at WARN; do NOT propagate as an error to the caller. The orphan sentinel will cause exactly one spurious full-hash reconcile on the next startup sweep, which detects no drift and simply unlinks the sentinel. This is a deliberate bias toward durability over efficiency.
- **Crash/kill between any two steps**: the sentinel is on disk from step 5 onward. Startup sweep observes it and triggers `full_hash_reconcile` regardless of which step was interrupted. Reconcile re-ingests from whatever actually landed on disk, matching the vault-authoritative invariant.

Crucially: there is NO window in which the DB advertises bytes that never reached the vault. The DB tx is the LAST step; a crash before the tx commits means the DB has old state and the vault has either the old file (crash before step 8) or the new file (crash after step 8). In both cases the reconciler self-heals from disk. A later `quaid collection restore` reads `raw_imports.raw_bytes` which was inserted in the SAME tx as the `pages`/`file_state` update — so restore bytes match DB bytes match (post-recovery) disk bytes.

The earlier "two-commit with NULL ctime" pattern is no longer needed: ctime is known before the commit (step 11 stats the post-rename target), so there's no rename-pending window to self-heal. The `file_state.ctime_ns` column remains nullable in schema for forward compatibility but is NEVER written as NULL by `memory_put` under this design.

#### Scenario: Happy path — single DB tx after rename, ctime captured before commit

- **WHEN** an agent calls `memory_put("work::notes/meeting", content, expected_version=5)` on a writable collection at current version 5
- **THEN** the system validates the CAS check (version 5 matches), walks to a trusted `parent_fd`, runs the filesystem precondition, creates+fsyncs the tempfile, checks the target is not a symlink, inserts the dedup entry, atomically renames tempfile → target, fsyncs the parent dir, stats the renamed target for `(mtime_ns, ctime_ns, size_bytes, inode)`, and commits a SINGLE SQLite tx that upserts `pages` (version 5 → 6), upserts `file_state` with the full stat tuple (no NULL ctime), rotates `raw_imports` (prior `is_active=0`, new `is_active=1` with the just-written bytes), and enqueues an embedding job
- **AND** returns success
- **AND** a subsequent `memory_get` sees `content`; a subsequent `memory_search` finds it via FTS; a subsequent stat-diff reconciliation skips the file (all four stat fields match); a subsequent `memory_put` filesystem precondition passes without spurious ConflictError

#### Scenario: Crash between rename and DB commit — reconciler self-heals from disk

- **WHEN** `memory_put("work::notes/x", new_content, expected_version=5)` reaches step 9 (rename succeeds) and the process is killed before step 12 (DB commit runs)
- **THEN** on-disk target has `new_content`; DB still has the pre-call `pages` row at version 5 with the old sha256 in `file_state`; dedup set is lost on restart
- **AND** on `quaid serve` restart, cold-start reconciliation stat-diffs the target → detects mismatch (at minimum mtime or inode differs) → re-hashes → hash differs from `file_state.sha256` → re-ingests the file via the normal ingest path, creating a new `pages` version and a fresh `raw_imports` active row from the actual on-disk bytes
- **AND** the caller sees "no response" or a broken-pipe error on their original `memory_put`; they can re-fetch and either re-issue the write (which will now succeed against the reconciler-installed version) or accept the disk state as the current state
- **AND** a subsequent `quaid collection restore` materializes the `new_content` bytes (from the reconciler-installed `raw_imports` row) — those bytes exist on disk and in `raw_imports`; there is no ghost content

#### Scenario: DB commit failure after successful rename — `needs_full_sync` recovery

- **WHEN** the rename at step 9 succeeds but the SQLite commit at step 12 fails (SQLite constraint violation, disk full on WAL, etc.) and the process keeps running
- **THEN** the handler removes the dedup entry, sets `collections.needs_full_sync = 1` on the affected collection in a fresh brief tx, and returns an error to the caller
- **AND** the recovery task observes the flag within 1 second and runs `full_hash_reconcile`, which re-ingests the file from disk per the reconciler's normal path
- **AND** after recovery, `pages`, `file_state`, and `raw_imports` reflect the on-disk bytes; the flag is cleared; subsequent `memory_put` and `memory_get` see consistent state

#### Scenario: Foreign rename lands at target between steps 9 and 11 — ConcurrentRenameError

- **WHEN** `memory_put("work::notes/x", content_A, expected_version=5)` completes step 9 (our tempfile atomic-renamed to `target_name` with our inode `I_A`), step 10 (`fsync(parent_fd)` succeeds)
- **AND** immediately before step 11, a foreign writer (external editor, cloud-sync daemon, another user-land tool) stages its own tempfile under `parent_fd` and `renameat`s it over `target_name`, so the path now holds the foreign file with inode `I_B != I_A`
- **THEN** step 11 captures `fstatat(parent_fd, target_name).inode = I_B`, the inode-hijack guard fires because `I_B != tempfile_inode` (which was `I_A`), and the handler aborts per the failure matrix: remove the dedup entry `(target_path, sha256_A)`, leave the sentinel in place, best-effort set `collections.needs_full_sync = 1` via a fresh connection, log `memory_put_inode_hijack` at ERROR, return `ConcurrentRenameError` to the caller
- **AND** NO SQLite tx is committed; `pages.version` stays at 5; `file_state` is unchanged; `raw_imports` is not rotated
- **AND** the recovery task observes `needs_full_sync = 1` within 1s (or the startup sweep observes the sentinel after a restart), runs `full_hash_reconcile` against the CURRENT on-disk bytes at `target_name` — the foreign bytes — and ingests them as a normal drift-corrected version; the DB never advertises our `sha256_A` for the foreign bytes
- **AND** the caller sees `ConcurrentRenameError`; they can re-fetch (which returns the post-recovery version the foreign write produced) and either re-issue against that version or accept the foreign bytes as the current state

#### Scenario: ConcurrentRenameError + failing fresh-connection `needs_full_sync` write — sentinel alone drives recovery

- **WHEN** step 11 inode-hijack guard fires (foreign inode `I_B != tempfile_inode I_A`) AND the handler's best-effort fresh-connection `UPDATE collections SET needs_full_sync = 1` ALSO fails (e.g., SQLite busy-timeout exceeded, WAL disk full, or the same underlying fault that would have failed Tx-B)
- **THEN** the handler STILL returns `ConcurrentRenameError` to the caller AND STILL leaves the recovery sentinel in place per the universal post-rename abort contract — the sentinel is the PRIMARY durable recovery marker; the `needs_full_sync = 1` write is an optimization, not a correctness requirement
- **AND** the dedup entry is removed so no in-process path suppresses a watcher event for the foreign bytes
- **AND** log `memory_put_inode_hijack collection=<N> slug=<S> tempfile_inode=<I_A> target_inode=<I_B>` at ERROR AND `memory_put_needs_full_sync_fresh_conn_failed collection=<N> slug=<S> errno=<E>` at ERROR (the latter captures that the live-recovery optimization failed so audit can correlate with eventual startup-recovery latency)
- **AND** the next `quaid serve` startup sweep observes the on-disk sentinel `<write_id>.needs_full_sync`, invokes `full_hash_reconcile` for the owning collection, hashes the current bytes at `target_name` (the foreign bytes), ingests them via the normal reconciler path (new `pages.version`, new `raw_imports` active row with the foreign bytes, `file_state` populated from the live stat tuple), and ONLY THEN unlinks the sentinel after the reconcile tx commits
- **AND** an integration test SHALL exercise this combined-failure path: inject foreign rename between steps 9 and 11 AND stub the fresh-connection SQLite write to return `SQLITE_BUSY`; assert the sentinel file exists on disk at handler return; restart serve; assert the foreign bytes are ingested and the sentinel is unlinked. This pins the invariant that sentinel retention — not the best-effort `needs_full_sync` flag — is the authoritative recovery signal

#### Scenario: Parent-directory fsync failure at step 10 — DB commit is REFUSED

- **WHEN** `memory_put` reaches step 10 (`fsync(parent_fd)` after a successful `renameat` at step 9) and fsync returns an I/O error — rename is in the page cache but NOT yet durable across a power loss
- **THEN** the handler SHALL NOT proceed to step 11 or step 12; NO SQLite tx is committed; NO `pages` version bump, NO `file_state` insert/update, NO `raw_imports` rotation lands
- **AND** the handler removes the dedup entry from the in-memory set so a later reconciler pass is not suppressed
- **AND** via a FRESH SQLite connection, the handler sets `collections.needs_full_sync = 1` so recovery runs regardless of subsequent process state
- **AND** the handler logs `memory_put_fsync_parent_failed collection=<N> slug=<S> errno=<E>` at ERROR
- **AND** the handler returns a distinct `DurabilityError` to the caller (NOT a generic I/O error) so agents can treat it explicitly
- **AND** post-recovery behavior: on the next `quaid serve` startup OR recovery-task cycle, `full_hash_reconcile` runs; if the fs flush did eventually land the rename, the reconciler ingests the new bytes as a normal drift-corrected version; if a power loss rolled the rename back, the reconciler observes the OLD file and the DB stays at pre-call state — in BOTH cases the DB never advertises bytes the vault does not hold, closing the durability regression

#### Scenario: Rename failure — tempfile cleaned up, dedup removed, target untouched

- **WHEN** `memory_put` reaches step 9 but `renameat` returns an I/O error (e.g., disk full, target mount read-only)
- **THEN** the handler removes the dedup entry (inserted at step 8), unlinks the recovery sentinel (created at step 5), `unlinkat(parent_fd, tempfile_name, 0)` removes the tempfile, returns an error
- **AND** the target path is UNCHANGED (POSIX renameat is atomic); no DB state was committed; the caller can retry after resolving the underlying I/O condition

#### Scenario: Subsequent `memory_get` sees the new content

- **WHEN** `memory_put("memory::x", new_content)` succeeds, and then `memory_get("memory::x")` is called
- **THEN** the returned page contains `new_content` (Tier 1 is fully consistent before `memory_put` returns)

#### Scenario: Subsequent FTS search finds the new content

- **WHEN** `memory_put("memory::x", content_containing_word_XYZZY)` succeeds and `memory_search("XYZZY")` is called immediately after
- **THEN** the page is included in the result set via the FTS lane

### Requirement: Filesystem precondition check — hash-on-stat-mismatch (all four stat fields always participate)

Before committing the single SQLite transaction, `memory_put` SHALL verify that the target file's on-disk state matches the system's last recorded view of it. If the target file has been modified, deleted, or created externally since the last time the system indexed it, `memory_put` SHALL abort with `ConflictError` before the tempfile is written. The precondition is NOT "stat-fields must match exactly" — that would leave ctime-preserving external writes undetected. It is "stat-fields match (all four including ctime), OR re-hash confirms the stored sha256 is still current." Under the rename-before-commit sequence `memory_put` never writes NULL ctime; a legacy/transient NULL-ctime row is handled by the hash-verify slow path rather than a fast-path carveout.

**Exact precondition algorithm:**

1. Stat the target file via `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)`. Absence/existence classifies the call: if stat returns `ENOENT` and a `file_state` row exists → `ExternalDelete` → `ConflictError`. If stat succeeds and no `file_state` row exists → `ExternalCreate` (the file was created outside our index) → `ConflictError`. If stat returns `ENOENT` and no `file_state` row exists → `FreshCreate` (happy-path create) → proceed.
2. If both stat and `file_state` row exist, compare fields:
 - Fast path: all four fields `(mtime_ns, ctime_ns, size_bytes, inode)` match. Precondition passes without a hash read. Under the rename-before-commit write sequence, `memory_put` always writes the complete stat tuple in its single SQLite tx, so all four fields are authoritative. A `file_state.ctime_ns IS NULL` row (legacy or transient reconciler provisional) does NOT take the fast path — it forces the slow path (hash-verify) so the NULL never masks an external edit.
 - Slow path: any of the four fields mismatch. The system SHALL re-hash the target file (streaming sha256) and compare to `file_state.sha256`. If the hash matches, the stat mismatch was a false alarm — self-heal by `UPDATE file_state SET (mtime_ns, ctime_ns, size_bytes, inode) = (stat_values)` in a brief tx, then proceed with the write. If the hash differs, the file's content has changed since the last index — return `ConflictError` with the current and expected sha256 in the error payload.

**NULL-ctime handling (no longer a fast-path carveout):** under the rename-before-commit write sequence defined in this spec, `memory_put` captures the full post-rename `(mtime_ns, ctime_ns, size_bytes, inode)` tuple BEFORE committing and writes all four fields in one tx. `memory_put` therefore NEVER produces `file_state.ctime_ns IS NULL` rows. A NULL-ctime row can still exist transiently from legacy code paths or reconciler provisional markers; the precondition treats any such row as "hash-verify required" rather than fast-path skip. The hash verify is the final authority, so a NULL-ctime row cannot produce a silent overwrite — at worst it forces an extra hash read, which is the correct cost/safety tradeoff.

This closes the "external rewrite preserves mtime+size but changes ctime" class of race. Tools that use `touch -m` + write, editors that preserve mtime on save, or sync agents that replay prior mtime all change ctime (on POSIX, ctime cannot be spoofed back in user-space). The ctime mismatch puts us on the hash-verify slow path; the hash differs; `ConflictError` is raised.

#### Scenario: All stat fields match — fast path, no hash

- **WHEN** `memory_put("work::notes/x", new_content, expected_version=5)` is called and `file_state` for `(work, notes/x)` has `(mtime_ns=T, ctime_ns=C, size_bytes=S, inode=I)`
- **AND** stat on the target returns the same `(T, C, S, I)`
- **THEN** the precondition passes without reading file content; `memory_put` proceeds with tempfile write and DB tx

#### Scenario: mtime mismatch — hash verify catches external edit

- **WHEN** `memory_put("work::notes/x", new_content, expected_version=5)` is called and `file_state.sha256 = H`
- **AND** stat on the target returns a different `mtime_ns` than the stored value (user saved the file externally)
- **THEN** the system reads and hashes the target file; the computed hash differs from `H`
- **AND** `memory_put` returns `ConflictError` with both hashes in the error payload; no tempfile is written; no DB changes occur
- **AND** the agent is expected to wait for the watcher to re-index the file and re-issue `memory_put` with a fresh `expected_version`

#### Scenario: ctime-only mismatch with unchanged bytes — self-heal, proceed

- **WHEN** `memory_put("work::notes/x", new_content, expected_version=5)` is called, `file_state.ctime_ns` is NOT NULL and differs from stat, but `(mtime_ns, size_bytes, inode)` all match
- **AND** the system hashes the file and the hash equals `file_state.sha256` (content unchanged — ctime diverged because of a metadata-only touch or a prior self-heal we missed)
- **THEN** the system UPDATEs `file_state.ctime_ns` to the current stat value in a brief tx and proceeds with the write. No `ConflictError` is raised — the bytes are what we indexed; ctime alone is noise in this case.

#### Scenario: ctime mismatch with drifted bytes — ConflictError

- **WHEN** `memory_put("work::notes/x", new_content, expected_version=5)` is called, `(mtime_ns, size_bytes, inode)` all match the stored `file_state`, but `ctime_ns` differs
- **AND** an external tool rewrote the file in-place preserving `(mtime, size, inode)` while necessarily changing ctime (the case Codex flagged)
- **THEN** the system hashes the file; the hash does NOT match `file_state.sha256`
- **AND** `memory_put` returns `ConflictError` (the exact case the "ctime in precondition" change is designed to catch). The write is refused; no silent overwrite.

#### Scenario: Transient NULL-ctime row (legacy / reconciler provisional) — forces hash-verify

- **WHEN** the precondition encounters a `file_state` row with `ctime_ns IS NULL` (NOT produced by the current `memory_put` path — possibly from a pre-change v4 migration or a reconciler provisional marker), and a `memory_put` targeting the same page runs
- **THEN** the precondition does NOT take the fast path regardless of whether `(mtime_ns, size_bytes, inode)` match; it forces the hash-verify slow path
- **AND** if the hash matches `file_state.sha256`, the self-heal `UPDATE file_state SET (mtime_ns, ctime_ns, size_bytes, inode) = (stat_values)` runs, filling in the missing ctime and proceeding with the write
- **AND** if the hash differs, `ConflictError` is returned — the NULL-ctime state cannot mask an external edit

#### Scenario: External delete detected

- **WHEN** `memory_put("work::notes/x", new_content)` is called and `file_state` for `(work, notes/x)` exists but the target file no longer exists on disk
- **THEN** `memory_put` returns `ConflictError`; no write occurs

#### Scenario: External create detected

- **WHEN** `memory_put("work::notes/x", new_content)` is called as a create (no page exists for this slug) and no `file_state` row exists for the target path
- **AND** the target file already exists on disk (created externally, not yet indexed)
- **THEN** `memory_put` returns `ConflictError`; no overwrite occurs; the agent is expected to wait for reconciliation and retry

#### Scenario: Create happy path (rename-before-commit — explicit ordering)

- **WHEN** `memory_put("work::notes/x", new_content)` is called as a create and neither the target file nor the `file_state` row exists
- **THEN** the sequence proceeds in the authoritative 13-step order from the `memory_put write sequence` requirement — never commit-before-rename:
 1. slug resolution + path-traversal validation + `expected_version` discipline (create path: no prior page, so `expected_version` MAY be omitted); writable-collection check
 2. `walk_to_parent` produces trusted `parent_fd`
 3. filesystem precondition check — both target file and `file_state` row absent → proceeds
 4. compute sha256 of the new content
 5. recovery sentinel created via `openat(recovery_dir_fd, "<write_id>.needs_full_sync", O_CREAT | O_EXCL | O_NOFOLLOW)` + `fsync(recovery_dir_fd)`
 6. tempfile created via `O_CREAT | O_EXCL | O_NOFOLLOW`, content written, `fsync(tempfile_fd)`, closed
 7. defense-in-depth `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)` — target absent, passes
 8. self-write dedup entry `(absolute_path, new_sha256, now)` inserted into the in-memory set
 9. atomic `renameat(parent_fd, tempfile_name, parent_fd, target_name)` — the tempfile becomes the target on disk
 10. `fsync(parent_fd)` — rename durability
 11. post-rename `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)` captures the complete `(mtime_ns, ctime_ns, size_bytes, inode)` tuple
 12. SINGLE SQLite tx upserts `pages` (version 1 for create), FTS trigger fires, `file_state` inserted with the full stat tuple, `raw_imports` rotated to insert the new active row with the bytes just renamed, `embedding_jobs` enqueued, COMMIT
 13. best-effort unlink recovery sentinel
- **AND** a crash at ANY step between 5 and 12 leaves the vault truthful — the recovery sentinel (created at step 5) is on disk, startup sweep observes it and runs `full_hash_reconcile`, which re-ingests from whatever actually landed on disk. DB state NEVER leads disk state. The phrase "renamed into place on DB success" is NOT the correct ordering — the rename is step 9, the DB commit is step 12, and the commit runs AFTER the rename has landed, not before.

### Requirement: Self-write dedup to suppress watcher echo (in-process only)

`memory_put` SHALL record `(absolute_path, new_sha256, now)` in an in-memory dedup set at step 8 of the rename-before-commit sequence — AFTER the tempfile is created + fsynced + the symlink-at-target defense-in-depth check, and IMMEDIATELY BEFORE the atomic `renameat` at step 9. The watcher SHALL consult this set on every inbound event; events whose `(path, sha256)` match an entry younger than 5 seconds SHALL be dropped. A background sweeper SHALL remove entries older than 5 seconds at 10-second intervals. On any pre-commit failure (rename fail, fsync-parent fail, post-rename stat fail, DB commit fail), the dedup entry SHALL be removed so the watcher or reconciler can re-process the change normally — see tasks 12.3 (pre-rename cleanup) and 12.4 (post-rename recovery) for the exhaustive handler matrix.

**The in-memory dedup set is in-process only.** It lives in the serve process that owns the collection (watcher + MCP handlers share the same set). The set CANNOT cross process boundaries — a separate CLI process cannot populate the serve process's in-memory HashMap. This is a deliberate design constraint that shapes where writes are allowed to originate (see the "CLI write routing when serve owns the collection" requirement below for the routing contract this constraint implies).

#### Scenario: Self-write dedup entry inserted at step 8 suppresses the watcher's rename event

- **WHEN** `memory_put("memory::note", content)` executes the rename-before-commit sequence, inserting `(absolute_path, new_sha256, now)` into the in-memory dedup set at step 8 — AFTER the tempfile has been created and fsynced and AFTER the defense-in-depth symlink-at-target re-check — and then `renameat` lands at step 9
- **THEN** the watcher's inbound event for that path carries a sha256 matching the dedup entry that is younger than 5 seconds
- **AND** the watcher drops the event; no redundant `full_hash_reconcile` or ingest fires for the self-written file
- **AND** the single SQLite tx at step 12 commits the authoritative `pages`/`file_state`/`raw_imports`/`embedding_jobs` rows without any double-work from the watcher path

#### Scenario: Pre-commit failure removes the dedup entry so external recovery is not suppressed

- **WHEN** any pre-commit step fails between step 8 (dedup insert) and step 12 (SQLite commit) — e.g., rename failure at step 9, `fsync(parent_fd)` failure at step 10, post-rename `fstatat` failure at step 11, or SQLite commit failure at step 12
- **THEN** the handler SHALL remove the dedup entry for `(absolute_path, new_sha256)` before returning the error to the caller
- **AND** subsequent watcher events for that path are NOT suppressed (no matching dedup entry), so either the reconciler OR the startup recovery sweep can observe the on-disk truth and reconcile without the dedup set masking a real change

#### Scenario: Background sweeper removes dedup entries older than 5 seconds

- **WHEN** a dedup entry has been in the set for more than 5 seconds (normal operation — the watcher event typically lands within milliseconds; a 5-second TTL is a defense against lost events)
- **THEN** the background sweeper running at 10-second intervals removes the entry
- **AND** any subsequent external edit at the same path is processed normally without spurious suppression

### Requirement: CLI write routing when serve owns the collection (fix; scope expansion)

Direct `quaid put` AND EVERY OTHER CLI path that lands bytes in a vault file from a separate process SHALL NOT bypass the owning serve's dedup set while serve is live. The CLI entry point SHALL, before taking any such write path, check `collection_owners` for the target collection.

**Enumerated in-scope CLI paths.** This contract binds every CLI command that performs vault self-writes via the rename-before-commit sequence, INCLUDING but not limited to: `quaid put`, `quaid collection add --write-quaid-id` (opt-in UUID write-back during initial walk), `quaid collection migrate-uuids` (one-shot frontmatter UUID backfill across an already-attached collection), and any future admin command that rewrites file bytes in place. The "rewrites vault bytes" property — not the command name — is the routing criterion. Admin commands that only rotate `raw_imports` / `pages` without touching disk bytes (e.g., `quarantine list`, `audit --raw-imports-gc`) are NOT in scope for this requirement.

- **No live owner (offline mode).** No serve process currently owns the collection (`collection_owners` has no row, OR the referenced `serve_sessions` row has aged past the 15s liveness threshold and is swept). The CLI proceeds with the direct fd-relative rename-before-commit sequence. Its in-process dedup set is empty (no watcher is running) so there is nothing to miss; the next `quaid serve` startup reconciles any drift from on-disk state.
- **Live owner (online mode).** `collection_owners` has a row AND the referenced session is live. The CLI SHALL NOT write directly — doing so would race the owning serve's watcher, which cannot see an in-memory dedup entry from a foreign process. Two resolution paths are supported, and the CLI SHALL pick based on flags/environment:
 - **Proxy mode (recommended default for `quaid put`):** The CLI connects to the owning serve's MCP interface (Unix socket path recorded in `serve_sessions.ipc_path`) and forwards the write as a `memory_put` MCP request. The write lands in the serve process, populates the in-process dedup set, follows the normal 13-step sequence, and the watcher suppresses the resulting event. From the user's perspective the CLI behaves identically to the offline case.
 - **Refuse mode (fallback when IPC is unavailable, AND the DEFAULT for bulk-rewrite admin commands).** If the CLI cannot establish an IPC connection to the live owner (socket absent, permission denied, etc.), it SHALL refuse with `ServeOwnsCollectionError` naming the owner's pid and host, instructing the user to either (a) stop the owning serve and retry offline, or (b) use the MCP tool directly. **`migrate-uuids` and `--write-quaid-id` attach-walk write-back are `Refuse`-mode by default** (NOT Proxy): these commands can rewrite thousands of files in a single invocation and interleaving each file through MCP while serve's watcher is live introduces per-file proxy RTT, amplifies the foreign-process self-write event fan-out on the watcher, and gives no stronger guarantee than "run it with serve down and let startup reconcile observe the final state once." The operator contract for these commands SHALL be: the CLI refuses with `ServeOwnsCollectionError` and instructs the operator to stop serve (or use `quaid collection detach --online` to hand off), run the bulk rewrite offline, then restart serve. Future releases MAY add `--online` proxy support for `migrate-uuids` once a batched proxy protocol exists, but the contract is Refuse-by-default. `design.md` MUST reflect this decision explicitly — the earlier implicit "per-slug mutex + restore-state interlock is sufficient" stance is superseded because the per-slug mutex is process-local and does not cover foreign-process self-writes against a live watcher.

The CLI SHALL NEVER write directly while a live owner exists — the lack of shared dedup makes that path unsafe by construction. This is uniform with the MCP tool: both paths go through the same in-process dedup-aware handler when serve is running.

**Schema addition (task 1.1).** `serve_sessions` gains an `ipc_path TEXT NULL` column recording the Unix socket (or named pipe, on future Windows support) path for MCP forwarding; `quaid serve` writes this on startup and the CLI reads it to resolve the proxy endpoint.

**Security model for `ipc_path`.** The IPC socket is a privileged local write channel — any process that can connect to it can submit `memory_put` requests to the owning serve. The socket contract SHALL:

- **Placement in a user-private runtime directory.** Serve creates the socket under `$XDG_RUNTIME_DIR/quaid/<session_id>.sock` on Linux (falling back to `$HOME/.cache/quaid/run/` if XDG is unset) and `$HOME/Library/Application Support/quaid/run/<session_id>.sock` on macOS. The parent directory is created with mode `0700` if it does not exist; if it already exists with broader permissions, serve REFUSES to start and emits `IpcDirectoryInsecureError`. System-wide `/tmp` or `/var/run` locations are forbidden because other users may enumerate or hijack them.
- **File permissions.** The socket file itself is created with umask-adjusted `0600` (user read/write only). A permissions audit after `bind()` verifies the final mode; any deviation aborts startup.
- **Peer identity verification on accept.** When a client connects, serve calls `getsockopt(SO_PEERCRED)` (Linux) or `LOCAL_PEERCRED` (macOS) to read the peer's UID and PID. Serve SHALL refuse any connection whose UID differs from its own process UID. PID is logged for observability but is not a security boundary (easy to forge on a shared UID).
- **Session-bound path.** The socket path embeds the `session_id` UUID, which is unguessable (UUIDv7 over the same memory's lifetime). A stale socket from a previous session SHALL be `unlink`ed at startup before `bind()`; a `bind()` failure because the path is held by another live process (same UID, different session) aborts startup with `IpcSocketCollisionError`.
- **CLI-side verification — kernel-backed peer credentials are authoritative.** Before forwarding, the CLI SHALL (a) `stat` the socket and verify mode `0600` and owning UID matches the current process UID; (b) after `connect()`, call `getsockopt(SO_PEERCRED)` on Linux or `getpeereid()` on macOS to read the server's kernel-reported peer PID and UID; (c) verify that the peer UID equals the current process UID AND that the peer PID matches `serve_sessions.pid` for the row whose `session_id` is embedded in the socket path. Kernel-backed peer credentials are the authoritative check — a same-UID attacker process can mimic the socket path and protocol but cannot forge its kernel-reported PID. (d) Only after (a)–(c) pass may the CLI issue a protocol-level `whoami` call; `whoami`'s returned `session_id` is treated as a CROSS-CHECK against the path-embedded session_id, not as the auth primitive. If any of (a)–(c) fail, or if `whoami` returns a mismatch, the CLI refuses with `IpcPeerAuthFailedError` and does NOT forward the write. These checks close the race where a same-UID process races the legitimate serve to bind the socket path and return a spoofed session_id in the protocol.
- **Server-side peer verification (defense in depth).** Serve SHALL call `getsockopt(SO_PEERCRED)` / `getpeereid()` on every accepted connection and refuse any connection whose peer UID differs from its own. PID is logged at INFO for observability. This matches the CLI's client-side check and ensures both endpoints reject cross-UID access even if the socket permissions were somehow relaxed.

This security model makes the proxy channel strictly user-local, session-bound, and permission-gated — matching the existing trust posture of `memory.db` (user-owned SQLite file) and the vault root (user-owned directory).

#### Scenario: Watcher does not double-index self-written files

- **WHEN** `memory_put("memory::note", content)` writes the tempfile, inserts the dedup entry (step 8 of the rename-before-commit sequence), atomically renames the tempfile over the target (step 9), and then commits the single SQLite tx (step 12)
- **THEN** the watcher event emitted by the rename is matched against the dedup set and dropped; no redundant re-ingestion occurs
- **AND** a pre-commit failure (rename fail, fsync-parent fail, post-rename stat fail, DB commit fail) removes the dedup entry per tasks 12.3/12.4 so the watcher or reconciler can process any real change

#### Scenario: External edit within TTL is not suppressed

- **WHEN** `memory_put("memory::note", content_A)` completes, and within 4 seconds a different process writes `content_B` to the same path
- **THEN** the external-edit event has a different sha256 than the dedup entry, so the watcher processes it normally and re-indexes with `content_B`

#### Scenario: Dedup set loss on crash is safe

- **WHEN** `quaid serve` is killed and restarted
- **THEN** the dedup set is empty; cold-start reconciliation uses `file_state` stat-diff to catch up on any missed events; no work is lost

### Requirement: Collection resolution for agent writes (ambiguity-safe)

`memory_put` SHALL apply the ambiguity-aware slug resolution rules defined in the collections capability (`Requirement: External addressing by <collection>::<slug> string with ambiguity protection`). Specifically for writes: an explicit `<collection>::<slug>` form always resolves to that collection; a bare slug (no `::`) resolves only when unambiguous per the write-operation rules. When resolution is ambiguous, `memory_put` SHALL return `AmbiguityError` without creating a tempfile, committing any SQLite state, or touching disk.

#### Scenario: Full slug resolves to named collection

- **WHEN** an agent calls `memory_put("work::notes/x", content)` and collection `work` exists
- **THEN** the file is written under `<work_root>/notes/x.md` and the page is created or updated in the `work` collection

#### Scenario: Bare slug in single-collection memory resolves to that collection

- **WHEN** only the `default` collection exists and an agent calls `memory_put("notes/x", content)`
- **THEN** the file is written under `<default_root>/notes/x.md` and the page is created or updated in `default`

#### Scenario: Bare slug create in multi-collection memory — write-target is only candidate

- **WHEN** collections `work` (write-target) and `personal` exist, no page `notes/new` exists in either, and an agent calls `memory_put("notes/new", content)` without `expected_version`
- **THEN** the system resolves to `(work, notes/new)` and creates the page in the write-target

#### Scenario: Bare slug update in multi-collection memory — unique existing page in write-target

- **WHEN** collections `work` (write-target) and `personal` exist, a page `notes/meeting` exists only in `work`, and an agent calls `memory_put("notes/meeting", content, expected_version=5)`
- **THEN** the system resolves to `(work, notes/meeting)` and updates that page

#### Scenario: Bare slug `WriteUpdate` resolves to the unique existing owner regardless of write-target

- **WHEN** collections `work` (write-target) and `personal` exist, a page `notes/meeting` exists only in `personal` (NOT in `work`), and an agent calls `memory_put("notes/meeting", content, expected_version=5)` (no explicit `::` prefix; this is a `WriteUpdate` because `expected_version` is present)
- **THEN** per the `WriteUpdate` routing contract in [collections/spec.md:76](../collections/spec.md#L76), the unique existing owner is the resolution target regardless of write-target status: the system resolves to `(personal, notes/meeting)` and proceeds with the normal `memory_put` CAS flow (`expected_version` check, `CollectionRestoringError` interlock per task 11.8, then update or `ConflictError`). There is no shadow-create into `work` and no `AmbiguityError` — both would be wrong for the single-owner case: create-into-write-target would silently fork identity, and `AmbiguityError` is reserved for multi-owner cases
- **AND** if the caller wants the create-into-`work` behavior instead, they use the fully-qualified `work::notes/meeting` form (which routes to the `WriteCreate` / `WriteUpdate` path against `work` explicitly); the bare-slug form is deliberately biased toward preserving existing identity

#### Scenario: Bare slug `WriteCreate` with a unique existing owner outside the write-target refuses shadow-create

- **WHEN** collections `work` (write-target) and `personal` exist, a page `notes/meeting` exists only in `personal`, and an agent calls `memory_put("notes/meeting", content)` WITHOUT `expected_version` (this is a `WriteCreate`)
- **THEN** per the `WriteCreate` routing contract in [collections/spec.md:75](../collections/spec.md#L75), a unique existing owner in a non-write-target collection returns `AmbiguityError`: the system refuses to silently shadow-create `work::notes/meeting` alongside the existing `personal::notes/meeting`, and refuses to silently update the `personal` page with no `expected_version` guard. The error lists both fully-qualified forms so the caller picks one
- **AND** no filesystem or database mutation occurs
- **AND** this is the only `WriteCreate`-specific refusal; the `WriteUpdate` (with `expected_version`) scenario above proceeds to the unique owner

#### Scenario: Bare slug refused when multiple collections own the slug

- **WHEN** collections `work` and `personal` both contain a page `notes/meeting` and an agent calls `memory_put("notes/meeting", content)`
- **THEN** the system returns `AmbiguityError` listing `work::notes/meeting` and `personal::notes/meeting`; no write occurs

#### Scenario: Unknown collection prefix is an error

- **WHEN** an agent calls `memory_put("nonexistent::foo", content)` and no collection named `nonexistent` exists
- **THEN** the call errors without touching the filesystem or database; the error message lists available collections

### Requirement: Path-traversal and symlink-escape rejection via fd-relative path resolution

The system SHALL enforce that every filesystem operation for a collection (`memory_put`, restore materialization, reconciler walk, watcher event processing) stays within the collection's `root_path`. The defense SHALL be implemented as a parent-directory-fd walk — NOT as a canonicalize-then-compare-prefix check, which is unsafe on the create path because `canonicalize()` fails on non-existent paths. The specific algorithm SHALL be:

**Setup (once per collection session — NOT for the life of the serve process).** Open the collection's `root_path` with `openat(AT_FDCWD, root_path, O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)` to obtain a trusted `root_fd`. If this open fails (because `root_path` is itself a symlink), refuse to attach the collection — log an ERROR and mark the collection `detached`. The `root_fd` is held for the current COLLECTION SESSION, defined as the interval during which the collection is in `state = 'active'` under the current `collections.root_path` value. A transition out of `active` (to `detached` or `restoring`) or a change to `collections.root_path` SHALL end the session: `quaid serve` closes `root_fd`, stops the watcher for that collection, and releases the collection's dedup/debounce resources. When the collection re-enters `active` — possibly under a new `root_path` after restore or `sync --remap-root` — serve SHALL open a fresh `root_fd` against the new path before resuming watcher/write/read operations. This rebinding SHALL be observed by serve via the coordination contract defined in the "Live-serve coordination for restore/remap" requirement (vault-sync spec) and SHALL NOT rely on process restart.

**Parse-time checks (applied to every slug/relative path).** Reject slugs that (a) contain `..` components, (b) begin with `/` (absolute), (c) contain empty components (e.g., `a//b`), or (d) contain NUL bytes. These rejections happen before any filesystem syscall.

**Walk (applied to every filesystem operation: memory_put, restore materialization, reconciler-walk resolution, watcher event handling).** Starting from `root_fd`, walk each non-terminal path component using `openat(current_fd, component, O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`. If a component does not exist, the operation SHALL decide per its semantics: for writes, create the directory with `mkdirat(current_fd, component, 0o755)` and immediately `openat` it (again with `O_NOFOLLOW | O_DIRECTORY`); for reads/walks/watcher events, treat the missing component as "target absent." If `openat` returns `ELOOP` (the component is a symlink), reject the operation — a symlinked ancestor is never traversed. At the terminal component, the caller holds a `parent_fd` that is guaranteed to be inside the collection root (because every step was `O_NOFOLLOW`).

**Terminal write (memory_put / restore).** Using the verified `parent_fd`:

1. Create the tempfile: `openat(parent_fd, tempfile_name, O_CREAT | O_EXCL | O_WRONLY | O_NOFOLLOW | O_CLOEXEC, 0o644)`. `O_EXCL` prevents collision with an attacker-planted file at the tempfile name; `O_NOFOLLOW` ensures the tempfile name is created fresh rather than resolving through an existing symlink.
2. Write content, `fsync(tempfile_fd)`, close.
3. Defense-in-depth check at the target: `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)`. If the target exists and is a symlink, REJECT the write — this is an attacker planting a symlink at the managed file name to redirect our rename. Remove the tempfile via `unlinkat(parent_fd, tempfile_name, 0)` and return a structured error. (`renameat` does not follow the destination symlink — it replaces the name atomically — but rejecting here surfaces the attack rather than silently overwriting the symlink.)
4. Atomic rename scoped to the trusted parent: `renameat(parent_fd, tempfile_name, parent_fd, target_name)`. Because both the source and destination are named relative to the SAME trusted `parent_fd`, no path-component lookup occurs at rename time — the rename cannot redirect outside the trusted directory.

**Terminal read / stat / walk.** Use `fstatat(parent_fd, name, AT_SYMLINK_NOFOLLOW)` (no-follow stat). If the entry is a symlink (regular-file test fails because it's a symlink), SKIP it with a WARN log. Walks NEVER descend into symlinked directories. Watcher events on symlinked paths are dropped.

**Platform scope.** The vault-sync and agent-writes capabilities are supported on macOS (aarch64 and x86_64) and Linux (x86_64 and aarch64 musl) ONLY — matching the existing `cross build` matrix. `openat`, `fstatat`, `mkdirat`, `renameat`, and `unlinkat` are implemented via the Rust `rustix` crate. Windows is explicitly OUT OF SCOPE for this change: its path-resolution primitives (`CreateFileW` with reparse-point handling, `NtCreateFile` with `OBJ_DONT_REPARSE`, handle-based verification) differ enough that the fd-relative algorithm specified here cannot be ported as-is without introducing equivalent trust-boundary weaknesses. A future OpenSpec change MAY add secure Windows-native support using handle-based path resolution; until then, `quaid serve`, `quaid put`, `quaid collection add`, and `quaid collection restore` SHALL refuse to run on Windows with a clear error ("vault-sync not supported on this platform; see <follow-up-issue>"). The Windows `quaid` binary MAY still build for offline commands that do not touch collection roots (`quaid --version`, `quaid --help`, `quaid init` against `memory.db`-only operations); any command that would exercise vault-sync / agent-writes refuses at invocation.

Rejection SHALL occur before any filesystem mutation outside the walk itself and before any database operation that would depend on the operation's success.

#### Scenario: `..` component rejected at parse time

- **WHEN** an agent calls `memory_put("work::../../etc/shadow", content)` (or a bare slug containing `..` components)
- **THEN** parse-time validation rejects the slug; the filesystem walk never begins; DB and disk are unchanged

#### Scenario: Absolute path rejected at parse time

- **WHEN** an agent calls `memory_put("/etc/passwd", content)`
- **THEN** the parse-time validator rejects the leading `/`; the call errors; filesystem and database are untouched

#### Scenario: Symlinked intermediate directory rejected during walk

- **WHEN** an agent calls `memory_put("work::notes/sub/sensitive.md", content)` and `<work_root>/notes` is a symlink pointing outside the collection (e.g., to `/tmp/attacker-controlled/notes`)
- **THEN** `openat(root_fd, "notes", O_DIRECTORY | O_NOFOLLOW)` returns `ELOOP`
- **AND** `memory_put` returns a structured `SymlinkEscapeError` naming the offending component; no tempfile is created; no DB changes occur
- **AND** a WARN log records the attempt with the component path (no symlink target resolution, to avoid side effects)

#### Scenario: Symlink planted at target file name refused before rename

- **WHEN** `memory_put` targets `<work_root>/notes/x.md`, the walk successfully obtains a trusted `parent_fd` for `<work_root>/notes/`, the tempfile is created, and between component-walk and rename an attacker places a symlink at `<work_root>/notes/x.md` pointing to `/etc/passwd`
- **THEN** the defense-in-depth `fstatat(parent_fd, "x.md", AT_SYMLINK_NOFOLLOW)` detects that the existing entry is a symlink
- **AND** the tempfile is removed via `unlinkat`; `memory_put` returns a structured error; `/etc/passwd` is never touched
- **AND** the self-write dedup entry (if inserted per the write sequence) is removed; no DB changes are committed

#### Scenario: Tempfile name collision refused by O_EXCL

- **WHEN** `memory_put` walks to `parent_fd` and an attacker has pre-placed a file at the tempfile name inside the parent directory
- **THEN** `openat(parent_fd, tempfile_name, O_CREAT | O_EXCL |...)` fails with `EEXIST`; `memory_put` returns an error; no bytes are written to any file
- **AND** the caller MAY retry with a fresh tempfile name (tempfile naming uses a random suffix so collisions are negligible in normal operation)

#### Scenario: Collection root itself is a symlink — refused at attach

- **WHEN** a user runs `quaid collection add work /Users/u/work-vault` and `/Users/u/work-vault` is itself a symlink
- **THEN** `openat(AT_FDCWD, "/Users/u/work-vault", O_DIRECTORY | O_NOFOLLOW)` returns `ELOOP`; the `add` command errors with a message instructing the user to resolve the symlink manually (point at the real directory) or remove the symlink; no `collections` row is inserted
- **AND** at serve-start, if a previously-added collection's root has become a symlink (e.g., user rearranged directories), the collection is marked `detached` and an ERROR log is emitted; other collections continue to serve

#### Scenario: Reconciler walk skips symlinked files with WARN

- **WHEN** a vault contains a symlink at `<root>/notes/linked.md` that points to a file outside the collection
- **THEN** the reconciler's walk uses `fstatat(parent_fd, "linked.md", AT_SYMLINK_NOFOLLOW)` and identifies the entry as a symlink
- **AND** does NOT treat it as a managed markdown file (no ingest, no `file_state` row)
- **AND** logs `skip_symlink collection=<name> path=<relative_path>` at WARN
- **AND** does NOT descend into symlinked directories during walks (`openat(... O_DIRECTORY | O_NOFOLLOW)` on the directory component returns `ELOOP` and the walk skips)

#### Scenario: Watcher ignores symlink-targeted events

- **WHEN** the watcher receives a file event for a path whose resolution via fd-walk encounters a symlink component
- **THEN** the event is dropped (not processed as a managed-file change); WARN log records the skip
- **AND** if the symlink was newly created (an attacker trying to redirect `memory_put` via a planted symlink), neither the watcher nor `memory_put` will traverse it

#### Scenario: Directory creation during walk uses no-follow and fd-relative

- **WHEN** `memory_put("work::deep/new/dir/x.md", content)` is called and `deep/new/dir/` does not yet exist
- **THEN** the walk creates missing directories with `mkdirat(current_fd, component, 0o755)` and immediately opens each with `openat(current_fd, component, O_DIRECTORY | O_NOFOLLOW)`
- **AND** if an attacker wins a race and plants a symlink at a component name after `mkdirat` but before `openat`, the subsequent `openat` with `O_NOFOLLOW` returns `ELOOP` and the operation rejects — the attacker cannot hijack the walk even under TOCTOU

### Requirement: Tempfile + sentinel cleanup on pre-rename failure

Because the SQLite commit is the LAST step of `memory_put` (step 12) and the rename is step 9, a failure in steps 1–9 (validation, walk, precondition, hash compute, sentinel create, tempfile write, symlink-at-target check, dedup insert, renameat returning an error) SHALL leave the filesystem and DB in a pre-call state:

- If the tempfile was created, it SHALL be `unlinkat(parent_fd, tempfile_name, 0)`-removed (best-effort).
- If the dedup entry was inserted (step 8), it SHALL be removed so a later external write of the same bytes is not suppressed.
- If the recovery sentinel was created (step 5) AND the vault was NEVER mutated (steps 1–9 covers every pre-rename failure), the sentinel SHALL be unlinked — it is no longer needed because no disk drift occurred. Retaining the sentinel here would trigger a harmless-but-wasteful full reconcile on the next startup.
- No DB mutation has occurred (the tx runs at step 12).
- The target file is unchanged — POSIX `renameat` at step 9 is atomic: either the rename lands or it doesn't.

Failures at steps 10–12 (post-rename — fsync parent, post-rename stat, SQLite commit) are POST-rename: the vault has the new bytes, the DB may or may not reflect them. Those cases are handled by the "Post-rename failure recovery" requirement below, which uses the filesystem sentinel (primary) plus `collections.needs_full_sync` (best-effort optimization) to re-ingest from disk — the rename is not reversible (we did not retain the prior bytes).

#### Scenario: Tempfile write failure leaves target intact

- **WHEN** `memory_put("work::notes/x", content)` reaches step 6 and the tempfile write fails (disk full, I/O error)
- **THEN** the handler unlinks whatever partial tempfile was created, unlinks the recovery sentinel from step 5; no dedup entry was inserted; no DB mutation; the target file is unchanged
- **AND** the caller receives an error identifying the write-side failure

#### Scenario: Symlink-at-target rejection — tempfile + sentinel cleaned up, no DB state

- **WHEN** step 7 detects a symlink at `target_name` via `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)`
- **THEN** the tempfile is `unlinkat`-removed and the recovery sentinel is unlinked (the vault never mutated); no dedup entry was inserted (step 8 has not run); no DB mutation; `memory_put` returns `SymlinkEscapeError`
- **AND** the attacker's symlink is untouched (we do NOT unlink the attacker's symlink — that would be an out-of-scope mutation); the user resolves it manually

#### Scenario: Rename failure — tempfile + sentinel cleaned up, dedup removed, target untouched, no DB state

- **WHEN** step 9 `renameat` returns an I/O error (disk full on target mount, read-only FS, etc.)
- **THEN** the handler removes the dedup entry inserted at step 8, unlinks the tempfile, unlinks the recovery sentinel (the rename did not land so the vault never mutated), returns an error
- **AND** the target path is UNCHANGED (POSIX `renameat` is atomic); no DB mutation has occurred (commit is at step 12)
- **AND** the caller MAY retry after resolving the underlying I/O condition

### Requirement: Post-rename failure recovery — filesystem-sentinel backstop

The system SHALL provide a durable filesystem-sentinel recovery backstop for every `memory_put` write that reaches step 5 of the rename-before-commit sequence. The sentinel MUST survive even when SQLite itself is unwritable, and it MUST be the authoritative recovery signal when the best-effort `collections.needs_full_sync = 1` write path is unavailable. This closes the durability gap where a post-rename pre-commit failure AND a concurrent SQLite fault would otherwise leave the vault ahead of the DB with no durable recovery marker.

**Window context.** The rename happens at step 9 and the SQLite commit at step 12, so there is a narrow window — steps 10, 11, 12 — in which the rename has succeeded but a later step fails before the DB reflects it. Under this design the vault holds the new bytes, the DB still reflects the prior state, and no compensating tx is possible (the rename is not reversible without re-reading the prior bytes, which we don't retain). The sentinel-backed backstop below is the normative recovery contract for that window.

**Durability gap.** The post-rename failure modes include exactly the conditions under which SQLite itself may be unable to accept new writes: `ENOSPC` on the WAL (step 12), hardware fault on the database device, or a `disk full` that triggered the original fsync failure in step 10. Relying exclusively on a follow-up `UPDATE collections SET needs_full_sync = 1` to trigger recovery is therefore not safe; that write can fail under the same fault that failed the main tx, leaving the vault ahead of the DB with no durable "please reconcile me" marker. The recovery contract MUST work even when every SQLite write path is currently refused.

**Filesystem recovery sentinel.** Each collection owns a dedicated recovery directory at `<memory_data_dir>/recovery/<collection_id>/` (created at `quaid collection add` time, on the same filesystem as `memory.db`). The directory is part of the collection's durable state and participates in `memory.db` backup semantics (copy the memory data dir as a unit). The file format is stable and minimal: a single empty (0-byte) file whose name is `<write_id>.needs_full_sync`. Writing a 0-byte file via `openat(O_CREAT | O_EXCL) + fsync(parent_dir)` is about as likely to survive a partial-disk-failure as the vault rename itself; if it cannot be written, every path in the system is already broken and an uncaught error to the caller is the honest outcome. **The sentinel lifecycle is integrated into the canonical write sequence at step 5 (create) and step 13 (unlink); there is no separate "overlay" step map.** See the exhaustive failure matrix in the `memory_put write sequence` requirement for the authoritative per-step rules.

**Post-rename failure handling (steps 10, 11, 12).** On ANY failure at these steps, the handler SHALL:

1. Remove the self-write dedup entry (so the file change will NOT be suppressed when the recovery task reads it).
2. **Leave the recovery sentinel in place** — this is the durable marker that survives even when SQLite is unusable.
3. Best-effort: in a fresh SQLite connection, set `collections.needs_full_sync = 1`. This is an OPTIMIZATION that lets the live recovery task pick up the work within 1s. It is explicitly NOT required for correctness; if the fresh-connection tx ALSO fails, the sentinel remains and startup recovery catches the drift on the next `quaid serve` boot.
4. Return an error to the caller.

**Startup recovery sweep.** Before the watcher/supervisor initializes, `quaid serve` SHALL scan every collection's recovery directory and for each sentinel file found: (a) `UPDATE collections SET needs_full_sync = 1 WHERE id = ?` (if this fails, defer to a later retry — the sentinel is still present), (b) enqueue the collection for immediate `full_hash_reconcile` (same code path as the live recovery task), (c) unlink the sentinel only AFTER `full_hash_reconcile` completes successfully and commits (otherwise leave it so the next boot retries). This closes the durability gap: no post-rename dirty state can silently persist across restarts, regardless of whether the original SQLite write succeeded.

**Live recovery fast path.** When the in-process recovery task observes `collections.needs_full_sync = 1` (set either via the best-effort fresh-connection write above OR by startup scan OR by watcher overflow), it runs `full_hash_reconcile`, re-ingests the on-disk file into `pages`, `file_state`, and `raw_imports`, and on success unlinks any matching recovery sentinels. Recovery is therefore driven by two signals (DB flag + filesystem sentinel) and requires only that ONE of them is durable, which holds even when the other is failing.

#### Scenario: DB commit failure — dedup cleared, needs_full_sync set, recovery task ingests from disk

- **WHEN** `memory_put` reaches step 12 with the tempfile already renamed and the parent dir fsynced, and the SQLite commit fails (e.g., SQLITE_BUSY after retries, disk full on WAL)
- **THEN** the handler removes the dedup entry, leaves the recovery sentinel in place, best-effort sets `collections.needs_full_sync = 1` via a fresh connection in a brief tx (may itself fail under the same fault, in which case the sentinel is the sole recovery signal), and returns an error to the caller
- **AND** the recovery task (live, if SQLite is healthy) OR the next startup sweep observes the sentinel (and possibly the flag), runs `full_hash_reconcile`, observes the stat mismatch against the stale `file_state`, hashes the target file, observes the sha256 drift, re-ingests — inserting a new `pages` version and a fresh active `raw_imports` row with the on-disk bytes — and unlinks the sentinel after reconcile commits
- **AND** after recovery, `memory_get` and `quaid collection restore` both see the same bytes: what's on disk = what's in `raw_imports` = what the DB advertises

#### Scenario: Process killed between rename and DB commit — cold-start reconciliation self-heals

- **WHEN** `memory_put` reaches step 9 (rename succeeds) and the process is killed before step 12 (DB commit)
- **THEN** no dedup entry survives (in-memory only); the vault has new bytes; the DB has the pre-call state; **the recovery sentinel (from step 5) survives on disk**
- **AND** on next `quaid serve` startup, the startup sweep observes the sentinel and triggers `full_hash_reconcile` immediately (before watcher/supervisor init); independently, cold-start reconciliation's stat-diff path would also detect the drift — the sentinel is belt-and-suspenders that guarantees reconciliation even if no future event ever touches the file
- **AND** reconciler re-ingests from the actual on-disk bytes, inserting a new `pages` version and a fresh active `raw_imports` row; the sentinel is unlinked after reconcile commits
- **AND** a subsequent `quaid collection restore` sees `raw_imports` bytes that match what's actually on disk; no ghost-content scenario is possible because the DB never committed bytes that weren't already on disk

### Requirement: Optimistic concurrency — `expected_version` required for updates

`memory_put` SHALL enforce compare-and-swap semantics on every update. If a page already exists for the resolved `(collection_id, slug)`, `memory_put` SHALL require an `expected_version` parameter matching the current `pages.version`; a missing `expected_version` on an update SHALL return `ConflictError` WITHOUT tempfile write, DB mutation, or filesystem mutation. Only the create path (no prior page at the slug) MAY omit `expected_version`.

The version check SHALL precede the filesystem precondition check. Both checks together guard against concurrent MCP writes (caught by `expected_version`) and concurrent external file edits (caught by the filesystem precondition). The per-slug async mutex (see "Per-slug write serialization") serializes writes within a single process but is NOT a substitute for `expected_version`: the mutex is useless across `quaid serve` + `quaid put` CLI + any future writer, so the compare-and-swap guarantee MUST live in the DB-version check and NOT in in-process locking.

This contract is uniform across every write entry point — MCP `memory_put`, CLI `quaid put`, and any future writer. No interface MAY offer an "update without expected_version" escape hatch. If a caller genuinely needs a force-overwrite (rare; typically recovery tooling), the caller SHALL `memory_get` first to read the current version and then supply it — making the override an explicit, observable step rather than a silent policy.

#### Scenario: Update with matching `expected_version` allows write

- **WHEN** an agent reads a page at version 5 and calls `memory_put("x", new_content, expected_version=5)`
- **THEN** the version check passes; the filesystem precondition runs next; if both pass, the write proceeds and the page's version bumps to 6

#### Scenario: Update without `expected_version` rejected with `ConflictError`

- **WHEN** a page exists at version 5 for `(work, notes/meeting)` and an agent calls `memory_put("work::notes/meeting", new_content)` WITHOUT supplying `expected_version`
- **THEN** the system returns `ConflictError` with a message including the current version (5) and instructing the caller to re-fetch and retry
- **AND** NO tempfile is written; NO DB mutation occurs; NO filesystem mutation occurs
- **AND** the page's version is unchanged

#### Scenario: Update with stale `expected_version` rejected

- **WHEN** an agent reads a page at version 5, another writer (MCP or CLI) bumps it to version 6, and the original agent calls `memory_put("x", new_content, expected_version=5)`
- **THEN** the system returns `ConflictError` on the version check; the filesystem precondition is not run; no tempfile is written; no DB update occurs

#### Scenario: Create without `expected_version` is allowed when no prior page exists

- **WHEN** no page exists for `(work, notes/new)` and an agent calls `memory_put("work::notes/new", content)` without `expected_version`
- **THEN** the create path proceeds; page is created at version 1; no conflict occurs

#### Scenario: Create with `expected_version` on a non-existent page rejected

- **WHEN** no page exists for `(work, notes/new)` and an agent calls `memory_put("work::notes/new", content, expected_version=1)` (supplying a version for a page that doesn't exist)
- **THEN** the system returns `ConflictError` with a message indicating no page exists at that slug (the caller's `expected_version` reveals an inconsistent view)

#### Scenario: Version match but filesystem mismatch rejects write

- **WHEN** an agent reads a page at version 5 and calls `memory_put("x", new_content, expected_version=5)`
- **AND** version 5 is current in the DB
- **AND** the target file was modified externally (e.g., saved from Obsidian) after the agent's last read
- **THEN** the version check passes but the filesystem precondition fails; the system returns `ConflictError`

#### Scenario: CLI `quaid put` enforces the same contract

- **WHEN** a user runs `quaid put "work::notes/meeting" --content-file new.md` (or equivalent) against an existing page without passing `--expected-version`
- **THEN** the CLI surfaces the same `ConflictError` as the MCP tool; no interface offers a "blind update" escape hatch

#### Scenario: `quaid collection migrate-uuids` refuses while a live owner exists

- **WHEN** `quaid serve` is running and owns collection `work` (`collection_owners` has a row, `serve_sessions.heartbeat_at > now() - 15s`)
- **AND** a user runs `quaid collection migrate-uuids work` in a separate process
- **THEN** the CLI resolves `collection_owners`, observes the live owner, and refuses immediately with `ServeOwnsCollectionError` naming the owning pid and host — BEFORE opening `root_fd`, scanning files, or writing any tempfile; NO file bytes are rewritten, NO `raw_imports` is rotated, NO `pages.uuid` column is touched
- **AND** the error message directs the user to stop serve (or use `quaid collection detach --online` to hand off) and re-run offline, OR wait for a future release that implements `migrate-uuids --online` batched proxy
- **AND** rationale: the bulk-rewrite contract is Refuse-by-default (not Proxy) because interleaving thousands of per-file proxy RTTs against a live watcher trades no correctness for measurable latency and event-queue pressure; the same outcome is cleaner via `serve stop → migrate-uuids → serve start` where startup reconciliation observes the final state in one pass

#### Scenario: `quaid collection add --write-quaid-id` refuses while a live owner exists on the same collection path

- **WHEN** the user invokes `quaid collection add shared /path --write-quaid-id`, and a different existing collection `work` with `root_path = /path` is already owned by a live serve (or the same path is about to collide at attach)
- **THEN** the CLI performs the live-owner check against any collection whose `root_path` (or canonicalized parent) overlaps with `/path`; if a live owner is detected, the CLI refuses with `ServeOwnsCollectionError` BEFORE the initial walk begins and BEFORE any UUID write-back runs
- **AND** NO `collections` row is inserted for `shared`, NO file is rewritten, NO dedup entries are created in any process
- **AND** the operator is directed to stop serve before re-attaching with `--write-quaid-id`, OR to attach without the flag and run `migrate-uuids` offline later

### Requirement: Per-slug write serialization

Concurrent `memory_put` calls targeting the same `(collection_id, slug)` SHALL be serialized by a per-page async mutex to prevent interleaved writes. Writes to different slugs SHALL proceed concurrently without contention beyond the bounded SQLite write lock.

#### Scenario: Concurrent same-slug writes serialize

- **WHEN** two `memory_put("x",...)` calls arrive concurrently
- **THEN** one completes fully (tempfile write + DB tx + rename) before the other begins; neither sees partial state from the other

#### Scenario: Concurrent different-slug writes do not block

- **WHEN** `memory_put("x",...)` and `memory_put("y",...)` arrive concurrently
- **THEN** both proceed without waiting on the other beyond SQLite's normal write-transaction lock
