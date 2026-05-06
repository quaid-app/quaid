## Context

Quaid's landed ingest model is collection-based. Vault-backed sync flows through `quaid collection add` / `quaid serve`, while `quaid ingest` remains the single-file entry point. Exact-byte duplicate short-circuiting now relies on `raw_imports.raw_bytes`, and active-source paths come from `raw_imports.file_path`.

The viral positioning — "Quaid as THE memory for Claude Code / OpenClaw / Hermes" — requires a frictionless daily experience. QMD's advantage is architectural: a persistent SQLite index, hourly cron-driven refresh with hash-skip short-circuit, no daemon required for queries. We can beat it by adding live file watching to what `quaid serve` is already doing for MCP.

Zero current users means no migration tax. We can reshape the schema and treat the vault as the single source of truth without worrying about preserving old external setups. That end-state is now landed: `quaid import` and `ingest_log` are removed, and the reconciler / watcher path is the only directory-ingest flow.

## Goals / Non-Goals

**Goals:**

- Live sync: edits to vault markdown appear in `memory.db` within ~2s for FTS, within seconds for semantic search.
- Zero-manual-import workflow for daily use.
- Multiple collections per memory (composite `(collection_id, slug)` key).
- Clean mental model: vault is authoritative for page content; DB is authoritative for programmatic relationships and operational state. Both must be backed up together for complete recovery.
- Portability: moving `memory.db` alongside the vault preserves all state; detached collections are a first-class state that re-attaches cleanly via `sync --remap-root`.
- Crash-safe: persistent embedding queue, reconciliation on startup, no corrupted state after unclean shutdown, no data-loss path through `memory_put` failures.
- Safe concurrency: `expected_version` catches MCP-vs-MCP races; filesystem precondition catches MCP-vs-external races.
- Clean extension points for cross-machine sync (Merkle subtree hashes, content-addressed exchange).

**Non-Goals:**

- Cross-machine sync itself — deferred to a follow-up change with its own spec.
- Daemon install (launchd/systemd) — deferred to the `daemon-install` follow-up change.
- Agent-bootstrap skill — deferred to the `openclaw-skill` follow-up change.
- Non-markdown ingest (images, PDFs) — out of scope; reconsider on real demand.
- Reconstruction of DB-authoritative state from vault-only backups — documented as expected behavior; users back up `memory.db` for full fidelity.
- Real-time vector index hot-reload during an in-flight query — eventual consistency is fine.

## Decisions

### Vault is authoritative for page content; DB is authoritative for programmatic state

**Decision:** Authority is split cleanly along what markdown can represent.

- **Vault-authoritative state** — page content (`compiled_truth`, `timeline`, `frontmatter`, `wing`, `room`) plus everything derivable from it on re-ingest: `page_fts` (trigger), `page_embeddings` (regenerated), wiki-link rows in `links` (extracted via `extract_links()`), heuristic rows in `assertions` (produced by `check_assertions()`), `timeline_entries`, frontmatter-derived `tags`. Files on disk are truth here; deletion deletes the page; rename (detected via native events or hash-match with uniqueness guards) updates the index in place.
- **DB-authoritative state** — things that CANNOT be reconstructed from page markdown: programmatic typed/temporal links created via the `memory_link` MCP tool, programmatic assertions with supersession chains created via `memory_check`, `raw_data` (external API sidecar content), `contradictions`, `knowledge_gaps`, `config`, `quaid_config`, `embedding_models`, `raw_imports` (original bytes), `import_manifest`.

**Alternatives considered:**

- *DB-authoritative for everything, vault is a mirror* — preserves structured metadata that markdown can't cleanly round-trip, but fights the "edit in Obsidian and it just works" pitch for the 95% case (page content).
- *Bidirectional with explicit reconciliation* — maximum flexibility, maximum conflict-resolution complexity, maximum bugs for a single-user tool.
- *Serialize all state into markdown sidecars or extended frontmatter* — adds complexity and noise to human-edited files to preserve fidelity of a minority of state.

**Rationale:** One honest mental model — your vault is where page content lives, your `memory.db` is where relationships and operational state live. Back up `memory.db` alongside the vault for complete recovery. This is the same honesty git has about `.gitignore`: the tool is clear about what it does and doesn't preserve.

### `memory_put` rename-before-commit write sequence (vault is authoritative; DB never leads disk)

**Decision:** MCP and CLI writes go through a sequence designed so that DB state NEVER advertises bytes the vault does not hold. The SQLite commit is the LAST step, not the first; the atomic rename happens BEFORE the commit. This inversion from earlier rounds is the direct fix for the "ghost content" risk: if the DB commits before the rename, a crash in the commit-to-rename gap leaves the DB claiming content that never hit disk — and a later `quaid collection restore` would materialize bytes from `raw_imports.raw_bytes` that never existed in the vault. Reordering makes that scenario impossible.

Sequence:

1. Precondition checks — parse slug, resolve collection, reject path-traversal, CAS on `expected_version` BEFORE any filesystem work. If `collections.writable = 0`, refuse with `CollectionReadOnlyError`.
2. `walk_to_parent(root_fd, relative_path, create_dirs=true)` → trusted `parent_fd`.
3. Filesystem precondition (`check_fs_precondition`) — fast path / hash-verify slow path per the "Filesystem precondition" decision.
4. Compute new sha256.
5. **Create recovery sentinel** — `openat(recovery_dir_fd, "<write_id>.needs_full_sync", O_CREAT | O_EXCL | O_NOFOLLOW) + fsync(recovery_dir_fd)`. This is the durable "please reconcile" marker that survives when SQLite is unusable (see the "filesystem-sentinel backstop" requirement in `agent-writes` spec). Sentinel-creation failure aborts the write here (no vault mutation yet).
6. Create+fsync tempfile via `openat(parent_fd, tempfile_name, O_CREAT | O_EXCL | O_NOFOLLOW)`.
7. Defense-in-depth `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)` — refuse if existing entry is a symlink.
8. Insert self-write dedup entry `(target_path, new_sha256, now)`.
9. Atomic `renameat(parent_fd, tempfile_name, parent_fd, target_name)`.
10. `fsync(parent_fd)` — rename is durable on disk.
11. `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)` → full post-rename stat `(mtime_ns, ctime_ns, size_bytes, inode)`.
12. **SINGLE SQLite transaction** — upsert `pages` (version++), FTS triggers, upsert `file_state` with the full stat tuple (no NULL ctime — all four fields known), rotate `raw_imports` (prior `is_active=0`, new `is_active=1` with the just-written bytes), enqueue `embedding_jobs`. COMMIT.
13. Best-effort unlink the recovery sentinel (`unlinkat(recovery_dir_fd, "<write_id>.needs_full_sync", 0) + fsync(recovery_dir_fd)`). An unlink failure here is logged but NEVER fails the write — the worst outcome is a spurious reconcile on next startup.

Failure recovery:

- **Pre-sentinel failure (steps 1–4)**: no vault mutation, no sentinel, no DB state. Target untouched.
- **Sentinel failure (step 5)**: abort the write; no tempfile, no rename, no DB state. Caller sees error; vault and DB remain consistent.
- **Pre-rename failure (steps 6–8)**: clean up tempfile via `unlinkat`; remove dedup entry if inserted; unlink the sentinel (no reconciliation needed since the vault never changed). No DB state written.
- **Rename failure (step 9)**: tempfile cleaned up; dedup removed; sentinel unlinked; no DB state; target unchanged (POSIX `renameat` is atomic).
- **fsync_parent failure (step 10)**: HARD STOP. The rename may not be durable, so the DB MUST NOT commit bytes that could be rolled back after power loss. Remove the self-write dedup entry, **leave the sentinel in place** (it is the durable recovery marker), best-effort set `collections.needs_full_sync = 1` via a fresh SQLite connection (this optimization may itself fail under ENOSPC; the sentinel is the primary signal), log at ERROR, return error. No SQLite commit occurs. Startup recovery or the live recovery task picks up the work.
- **Crash between rename and commit** (steps 9–12): vault holds new bytes; DB has pre-call state; dedup is in-memory and lost on restart; **the sentinel survives on disk**. On `quaid serve` restart, the startup sweep scans the recovery directory, sets `needs_full_sync = 1`, and schedules `full_hash_reconcile` for the collection — sentinel is unlinked only after reconciliation commits. Belt-and-suspenders: cold-start reconciliation would ALSO detect the stat/hash drift even without the sentinel, but the sentinel guarantees the flag is set even if no future `memory_put` or watcher event ever touches the file.
- **DB commit failure (step 12)**: vault holds new bytes; DB has pre-call state. Handler removes the dedup entry, leaves the sentinel in place, best-effort sets `collections.needs_full_sync = 1` via a fresh connection (no hard failure if that write also rejects — the sentinel still drives recovery), returns error. Live recovery task (on next successful SQLite open) or startup sweep runs `full_hash_reconcile`, which re-ingests from disk and then unlinks the sentinel.

The earlier "two-commit with NULL ctime" pattern is no longer needed: step 11 stats the post-rename target and step 12 commits the full tuple in a single tx. `file_state.ctime_ns` is never written as NULL by `memory_put` under this design. The column remains nullable in the schema for forward compatibility.

**Alternatives considered:**

- *Commit-before-rename with compensating tx on rename failure (rounds 1–11)*: crash between commit and rename leaves DB claiming bytes the vault doesn't have, and restore would materialize them from `raw_imports`. Rejected in favor of rename-before-commit.
- *Pending-write journal (a `pending_writes` row inserted before rename, removed after commit)* — another valid fix for the same problem. Adds a table and requires startup recovery to scan it. Reordering to rename-before-commit is simpler and achieves the same invariant with no new schema.
- *Two-phase commit between DB and filesystem* — over-engineered for a single-user tool; we have the natural ordering primitives (fsync, atomic rename) to achieve the invariant without a 2PC protocol.
- *Rename first, then DB tx, delete renamed file on DB failure* — can't delete safely: the rename has already overwritten the prior bytes, so deletion would lose both old and new content. Keeping the rename and letting the reconciler self-heal from the new disk bytes is the correct posture: vault is authoritative, reconciler catches up.
- *Single-commit write with tempfile ctime (rounds 2–5)* — rejected because rename updates ctime; stored value was immediately stale.

**Rationale:** The rename-before-commit ordering creates exactly one invariant: DB state is always a subset of what the vault holds (it may lag briefly but never leads). A crash at any point in the sequence is recoverable from disk state alone: tempfile orphans are cleaned on restart; renamed-but-uncommitted bytes are re-ingested by the reconciler via normal stat-diff. The ghost-content failure mode that identified cannot exist under this design because `raw_imports` is only committed when the bytes are already on disk. The caller may see an error after rename-but-before-commit — but the vault state is truthful, and subsequent reads/searches/restores reflect reality rather than a DB-only fiction.

### Watcher overflow: `needs_full_sync` flag + immediate recovery task (no event dropping)

**Decision:** The watcher pipeline NEVER drops events silently. When the bounded mpsc channel for a collection fills, the system sets `collections.needs_full_sync = 1` in a brief tx, logs at WARN, and continues accepting subsequent events. A dedicated recovery task polls this flag every 500ms; on detection it runs `full_hash_reconcile` for the collection (independent of the periodic audit schedule) and clears the flag on completion. Recovery starts within one second of the overflow, so the window of index staleness is bounded to the reconciliation duration (seconds for a typical vault).

The flag is surfaced via `memory_collections` and `quaid collection info` so MCP callers can observe eventually-consistent state. Panics during recovery restart with backoff without clearing the flag, so recovery is retried until it succeeds.

**Alternatives considered:**

- *Drop-oldest on overflow, rely on periodic audit* — the original design. Codex called out that a burst of deletes/renames exceeding channel capacity can leave `memory.db` serving pages that no longer exist on disk for up to the audit interval (days). Live-sync contract violated. Rejected.
- *Unbounded channel* — risks unbounded memory growth under pathological bursts.
- *Producer backpressure (block the notify thread)* — notify delivers events from the kernel/backend; blocking the producer just pushes overflow to the kernel's event buffer (same silent loss, different layer). Not portable.
- *Drop duplicate/coalescable events at high watermark* — complicates event-semantic reasoning (which events are safe to merge?) and still loses information about create/delete/rename distinctions in pathological cases.

**Rationale:** The flag-and-recover model preserves the live-sync contract: no event is silently lost at our layer; a dirty-flag + fast recovery means "eventually consistent within seconds" rather than "potentially stale for days." The 4096-slot channel is generous enough that overflow in practice means a genuine bulk operation (`git checkout`, vault-wide find-and-replace), which warrants a full-collection reconciliation anyway.

### Link provenance (`source_kind`) + explicit quarantine predicate

**Decision:** Add `links.source_kind TEXT NOT NULL DEFAULT 'programmatic' CHECK(source_kind IN ('wiki_link', 'programmatic'))` to v5 schema. Ingest's wiki-link extraction (`extract_links()` in `src/core/links.rs`) sets `source_kind = 'wiki_link'` (re-derivable from markdown). The `memory_link` MCP tool sets `source_kind = 'programmatic'` (DB-only authoritative). Existing `assertions.asserted_by` (already in v4 schema with values `'agent'/'manual'/'import'/'enrichment'`) provides the same classification for assertions: `'import'` = re-derivable heuristic from `check_assertions()`; other values = programmatic.

The quarantine predicate `has_db_only_state(page_id) -> bool` is defined explicitly:

```sql
EXISTS (links WHERE (from_page_id=? OR to_page_id=?) AND source_kind='programmatic')
OR EXISTS (assertions WHERE page_id=? AND asserted_by != 'import')
OR EXISTS (raw_data WHERE page_id=?)
OR EXISTS (contradictions WHERE page_id=? OR other_page_id=?)
OR EXISTS (knowledge_gaps WHERE page_id=?)
```

The `contradictions` branch was added in response to a Codex finding: `contradictions.page_id` and `contradictions.other_page_id` both `ON DELETE CASCADE` against `pages`, so without this guard a page delete would silently erase contradiction history that `memory_check` built up — state that cannot be reconstructed from markdown. The `knowledge_gaps` branch: `knowledge_gaps.page_id` is `ON DELETE CASCADE` against `pages`, so a page delete without this guard would silently erase the audit history of what agents could not answer — the same loss class. Every page-scoped table that is NOT reconstructible from markdown MUST participate in this predicate; future additions follow the schema-audit rule.

Every code path that inserts into `links` or `assertions` SHALL populate the provenance column correctly — this is a correctness invariant, not a nice-to-have. The `source_kind` default `'programmatic'` is a fail-safe: if a future code path inserts a link without specifying `source_kind`, it is treated as DB-only state and preserved on delete rather than silently lost.

**Alternatives considered:**

- *Split `links` into two tables (`wiki_links`, `programmatic_links`)* — cleaner in one sense but doubles the join surface for every query that asks "what links this page?" Rejected.
- *Assume all `links` are programmatic (safest)* — overly conservative; every ingested wiki-link preserved forever on delete would bloat quarantine backlogs.
- *Assume all `links` are derivable (simplest)* — destroys programmatic relationships on any disk-level delete. Exactly the failure the quarantine model prevents. Rejected.
- *Infer provenance from content matching* — heuristics that diff link rows against `extract_links()` output. Complex, error-prone, and the information (who/what created this row) is already known at insert time. Rejected.

**Rationale:** Provenance needs to be known at insert time (call-site knows whether it's extracting from markdown or executing a `memory_link`), so storing it explicitly is cheaper and more correct than post-hoc inference. The CHECK constraint + explicit callsite audits (task 5.4a) prevent the "forgot to set source_kind" class of bugs.

### Filesystem precondition: ctime-aware stat + hash-on-mismatch

**Decision:** Before writing the tempfile, `memory_put` stats the target file and compares against `file_state` with a fast-path / slow-path structure:

- **Fast path** — if stat succeeds, `file_state` row exists, and all four fields `(mtime_ns, ctime_ns, size_bytes, inode)` match → proceed without reading content. Under the rename-before-commit write sequence (task 12.1), `memory_put` always writes the complete stat tuple in its SINGLE SQLite tx, so a `file_state.ctime_ns IS NULL` value is NEVER produced by `memory_put` itself. Any transient NULL-ctime row (e.g., from legacy data or a reconciler provisional marker) is treated as "hash-verify required" rather than fast-path skip.
- **Slow path** — any field mismatch (including ctime when `file_state.ctime_ns` is NOT NULL) triggers a streaming sha256 of the target. Hash equals `file_state.sha256` → self-heal (UPDATE stat fields in a brief tx, proceed). Hash differs → `ConflictError` with both hashes in the payload.
- **ExternalDelete** — stat returns `ENOENT` and `file_state` exists → `ConflictError`.
- **ExternalCreate** — stat succeeds and no `file_state` row (caller expected create path) → `ConflictError`.
- **FreshCreate** — stat returns `ENOENT` and no `file_state` row → proceed.

Note: the earlier NULL-ctime carveout (which covered the rename-pending window of a two-commit memory_put sequence) is NO LONGER APPLICABLE. Under the rename-before-commit design in task 12.1, `memory_put` captures the full stat tuple AFTER the rename and writes all four fields in one tx — there is no rename-pending window. A `file_state.ctime_ns IS NULL` row can only arise from legacy data or a reconciler provisional marker; the precondition treats it as "hash-verify required" rather than fast-path skip, so correctness is preserved.

This check runs after `expected_version` validation. Together they cover two distinct races: `expected_version` catches concurrent MCP writes (via the DB version check, which is cross-process safe); the filesystem precondition catches concurrent external edits (which bypass our mutex and never bump `pages.version`).

**Alternatives considered:**

- *Stat-only precondition with ctime ignored (the earlier stance)* — Codex flagged this as unsafe: an external tool that preserves mtime+size (editors that save-with-preserve-mtime, rsync `--times`, sync agents replaying history) necessarily changes ctime. A stat-only check misses these and silently overwrites the external edit. Rejected.
- *Hash the file on every write unconditionally* — definitive but pays a full-file-read cost on every `memory_put` even when nothing has changed. The fast-path / slow-path split pays the hash cost only when stat actually diverges, which is rare in practice.
- *Rely on `expected_version` alone* — doesn't catch external edits at all (they never bump `pages.version`).
- *Trust ctime alone* — ctime can be updated by metadata-only operations (permission changes, hardlink additions) that don't touch content. Using ctime as a veto would raise spurious ConflictErrors. The hash verify resolves this: ctime mismatch with unchanged bytes self-heals and proceeds.

**Rationale:** The fast path preserves the "cheap stat-only" pathway for the overwhelming majority of writes where nothing has changed. The slow path adds a hash cost exactly when something looks different, which is the only case where reading the content matters. `ctime` is the signal that makes the fast path safe against mtime-preserving external writes; the hash is the signal that keeps the slow path from false-positive on benign ctime drift. Together they make the precondition both cheap and correct.

### Durable page identity via `quaid_id` frontmatter UUID (opt-in write-back)

**Decision:** Every page has a persistent `pages.uuid` column. The UUID MAY ALSO appear as a `quaid_id: <uuid>` field in the page's markdown frontmatter, but persistence to the file is OPT-IN and NOT the default. On first ingest of a file without `quaid_id`, the system generates a UUIDv7 and stores it in `pages.uuid` ONLY; default ingest, default reconciliation, and watcher-observed external edits are read-only with respect to user bytes. The UUID is written back to frontmatter only through explicit user-initiated paths: (a) `quaid collection add --write-quaid-id`, (b) `quaid collection migrate-uuids`, or (c) `memory_put`, which always emits `quaid_id` because the user is already writing the page. `render_page()` always includes `quaid_id` in its output whenever `pages.uuid` is set.

Rename detection priority becomes: (1) UUID match across `missing` and `new` sets when the frontmatter `quaid_id` is present on disk → honored regardless of content; (2) native rename events (FSEvents paired, inotify cookies) → honored; (3) content-hash uniqueness fallback (the primary signal for files that never had a frontmatter UUID written); (4) quarantine + fresh create. Case (4) remains bounded: it requires BOTH no UUID on disk AND non-unique content.

**Alternatives considered:**

- *Automatic write-back on first ingest (rounds 14–21)* — wrote `quaid_id` into every file during the initial walk. Round-22 Codex flagged this as a bulk-mutation hazard: attaching a real vault would dirty thousands of files, create sync-tool / git churn, and fail unpredictably on permission-limited files. Rejected; attach must be read-only by default.
- *Content-hash only* — the original design. Round-6 Codex found this still forks identity for short notes, template-derived notes, and duplicate-content files on backends without paired rename events. Small-but-unique files that cross the hash-uniqueness bar still work, but the broader class is lossy. Rejected as insufficient on its own.
- *Inode-based identity* — POSIX inodes are unique within a single filesystem but don't survive cross-machine moves, restores, or any tool that creates a new inode (git checkout, cp, rsync). Fails precisely the cases the UUID is meant to cover.
- *DB-only UUID (never persisted to frontmatter)* — survives in the DB but provides no rename signal once the file hits a different machine via rsync/git. Opt-in write-back gives users the cross-machine path when they want it without forcing the mutation on everyone.

**Rationale:** Writing a small `quaid_id` field to frontmatter is a cost with real tradeoffs: it dirties git trees, triggers sync-tool uploads, and can fail on read-only files. Those costs are acceptable when the user explicitly asks for them (`--write-quaid-id`, `migrate-uuids`, `memory_put`) but not as a silent default side-effect of attaching a vault. Collision handling (/82 fail-stop contract — accidental or adversarial duplicates halt reconcile): when two or more files carry the same `quaid_id`, both Phase 4 (`/new/path` verification) AND post-attach `full_hash_reconcile` halt with `DuplicateUuidError`. No auto-rewrite, no `pages.uuid` rebind, no new UUID is minted, no self-write is enqueued. The operator resolves manually by stripping `quaid_id` frontmatter from every file EXCEPT the one intended to retain the original identity, then re-running sync. This replaces the pre, which was flagged as an unsafe identity-mutation route under ambiguous ownership.

**Remap / fresh-attach identity preservation.** The `quaid_id` UUID is the authoritative identity key during `sync --remap-root`, `collection restore`, first-serve-after-detach, and cross-machine `memory.db` moves. The full-hash reconciler builds an in-memory HashMap of `quaid_id → (on_disk_relative_path, sha256)` from the new tree, then looks up every `pages.uuid` against that map. UUID match preserves `pages.id` and updates `file_state.relative_path` to the observed path — directory reorganization during a move (user splits a folder, renames a subtree, moves files deeper) does NOT trigger delete-and-create churn. This closes the failure where a path-based remap would quarantine or hard-delete dozens of pages any time a user reorganized their vault during a machine move, destroying programmatic links/assertions/raw_data/contradictions that cannot be reconstructed from markdown.

### Rename detection: native-event preference with uniqueness guards

**Decision:** Native rename events (FSEvents paired `Rename`, inotify `IN_MOVED_FROM`/`IN_MOVED_TO` cookies) are honored directly regardless of content. Content-hash inference is used only when native events are unavailable (reconciler walks, or backends without paired rename events) AND all of the following hold (/82 canonical-resolver guards, shared by Phase 4 and `full_hash_reconcile`):

1. Exactly one `missing` entry in the batch has sha256 = H.
2. Exactly one `new` entry in the batch has sha256 = H.
3. File size > 64 bytes (configurable via `QUAID_RENAME_MIN_BYTES`).
4. **Body after frontmatter is non-empty** (/82 addition — matches Phase 4's `UnresolvableTrivialContentError` branch; files whose non-frontmatter content is empty or whitespace-only are too trivial to anchor identity via content-hash alone; operator is directed to `migrate-uuids` for UUID-based anchoring).

When any condition fails, the system treats the missing side as a hard-delete and the new side as an independent create (new `pages.id`, no identity reassignment).

**Alternatives considered:**

- *Hash-only pairing without uniqueness guards* — the original design. Identity-corrupts when a vault has identical-content files (copied templates, empty placeholders, duplicated stubs). A delete+create of two different files with the same content would silently reassign the old page's backlinks to the new file. Rejected.
- *Never auto-pair; require explicit user intent* — every rename becomes a history gap. Hostile to normal file-management patterns.

**Rationale:** Native rename events are ground truth when available; the uniqueness rules keep inference safe for the remaining cases. The 64-byte threshold is a pragmatic guard against empty-file false pairs. Each ambiguous decision is logged so that unexpected behavior is debuggable.

### Composite `(collection_id, slug)` key; `<collection>::<slug>` external addressing

**Decision:** Pages get an integer PK and a `collection_id` FK. Slugs are unique within a collection and remain path-shaped (`people/alice`, `notes/meeting`, etc.). The external address for explicit collection routing uses the literal two-colon separator `::` — e.g., `work::people/alice` — which never collides with path segments. Bare slugs (no `::`) resolve via the ambiguity-safe rules. Collection names SHALL NOT contain `::` (rejected at `collection add` time); this keeps the parsing rule trivially "split on first `::` occurrence."

**Alternatives considered:**

- *Use `/` as the collection-routing separator* — the original design. Caused a silent address collision as soon as a collection name matched an existing top-level slug segment (e.g., a `people` collection would shadow every existing `people/<slug>` page, or worse, silently re-route agents' references). Called out by the Codex review as user-visible lookup/write breakage on adding a colliding collection. Rejected.
- *Globally unique prefix-encoded slug* — simple schema, but renaming a collection becomes a global string-rewrite.
- *Composite with explicit collection parameter on every MCP tool* — pure in DB terms, but breaks every tool signature.
- *Single-collection with error on collision* — punishes real-world use cases (two vaults with overlapping paths).
- *Reserve collection names against existing top-level slug segments* — requires a per-add audit across all pages AND still breaks when a user later ingests content whose top-level segment matches a collection name. Tool complexity for a property `::` gives us for free.

**Rationale:** The composite schema gives correctness (rename = one-row update, no string parsing to derive collection). The `::` external separator gives ergonomics (single-argument addressing; stable in URLs/logs/CLI) AND non-collision with slug namespaces (since `::` is not a valid path component). Zero users means we can pick the right separator once and be done.

### Deletion policy: hard-delete only when unambiguous AND page has no DB-only state; otherwise quarantine

**Decision:** A disk-remove has three possible outcomes depending on what else is happening and what the page carries:

1. **Unambiguous + purely vault-derivable state → hard-delete.** The file is gone, no other file in the vault has the same sha256, AND the page has NONE of the five DB-only categories (no programmatic `links` with `source_kind='programmatic'`, no non-import `assertions`, no `raw_data`, no `contradictions` referencing the page as `page_id` or `other_page_id`, no `knowledge_gaps` referencing the page). Deleting is safe because everything associated with the page is derivable from markdown that no longer exists. Cascaded deletes through wiki-link `links` and heuristic `assertions` are losses of re-derivable state, not authoritative state.
2. **Unambiguous + DB-only state present → quarantine.** The file is gone and no other file matches, BUT the page carries ANY of the FIVE DB-only categories: programmatic links (`source_kind = 'programmatic'`), non-import assertions (`asserted_by != 'import'`), `raw_data`, `contradictions` (referenced as either `page_id` or `other_page_id`), or `knowledge_gaps` (referenced via `knowledge_gaps.page_id` — audit history added to the predicate because `memory_gap` slug-bound output is DB-authoritative and cannot be reconstructed from markdown). None of these can be reconstructed from markdown. Set `pages.quarantined_at = now`, delete the `file_state` row (the file is gone), but keep the `pages` row and all DB-only state. The page is hidden from default queries; user resolves via `quaid collection quarantine {list,restore,discard}`. The five-branch predicate MUST be enumerated identically everywhere it is referenced inline (enforced by spec-consistency audit task 17.17 (f) — partial enumerations fail the build).
3. **Ambiguous rename refused → quarantine (regardless of DB-only state).** A file was removed AND a new file appeared with the same sha256, but uniqueness rules refused the pair (non-unique hash, trivial content, etc.). The very ambiguity means we cannot prove the user intended a delete rather than a move. Quarantine the old page and create a fresh page for the new path. User manually resolves via `quaid collection quarantine restore` if the move was intentional.

Quarantined pages have a TTL (default 30 days, via `QUAID_QUARANTINE_TTL_DAYS`). An auto-sweep on startup and daily timer hard-deletes quarantined pages older than the TTL **only when the DB-only-state predicate returns FALSE for the page** (all FIVE categories empty). Pages that carry ANY DB-only state category are NEVER auto-deleted — they remain quarantined indefinitely until a user explicitly resolves them via `quaid collection quarantine {restore,discard,export}`. `discard` on a page with DB-only state requires `--force` OR a prior `export` (which dumps ALL five DB-only state categories to a JSON file before allowing the discard). `quaid collection info` surfaces the count of "quarantined pages awaiting user action" so backlogs are visible. The sweep logs each discard so users who misconfigure quarantine can still audit what was removed; each SKIPPED DB-only-state page is logged at DEBUG for auditability.

**Alternatives considered:**

- *Always hard-delete on file removal* — the original design. Loses DB-only state (programmatic relationships painfully built up via `memory_link`/`memory_check`) on any rename that the inference rules can't confidently pair. A duplicate-content template rename destroys its link graph silently.
- *Always quarantine every delete* — safer but accumulates phantom pages. Users see stale state in `quaid collection quarantine list` for every note they ever deleted. Conservative but noisy.
- *Quarantine only on ambiguous refusal; hard-delete all unambiguous deletes* — simpler but still loses DB-only state whenever a page with `memory_link`/`memory_check` history is cleanly deleted from disk. The cost of preserving it via quarantine is small.

**Rationale:** The two quarantine triggers map to the two ways we can be wrong about a delete: (a) we might be mistaking a rename for a delete (ambiguity), (b) we might be deleting state the user built manually and can't reconstruct (DB-only state). Hard-delete remains the default for the common case (plain note removal with no hand-built relationships) so the quarantine list doesn't balloon. The 30-day TTL is tuned for "user changes their mind within a month"; the sweep ensures quarantine doesn't bloat indefinitely.

### Bare-slug addressing: require global uniqueness in multi-collection brains

**Decision:** The `<collection>::<slug>` form always resolves unambiguously to the named collection. Bare slugs (without a `::`) resolve according to:

- **Single-collection memory** (one collection total): bare slugs always resolve to it. Convenience for the default setup.
- **Multi-collection memory**:
 - **Read ops** — non-mutating only (`memory_get`, `memory_backlinks`, `memory_timeline`, `memory_graph`, `memory_tags`, `memory_list`, `memory_link_close` in its lookup-only mode): resolve to the unique collection that owns a page with this slug. Zero → `NotFoundError`. Multiple → `AmbiguityError`.
 - **WriteCreate** (`memory_put` without `expected_version`): if no collection owns the slug, resolve to the write-target. If exactly one collection owns it AND that collection is the write-target, resolve to it. If exactly one owns it but a DIFFERENT collection, return `AmbiguityError` (refuse to silently shadow-create in the write-target or silently cross-collection-update).
 - **WriteUpdate** — every DB-mutating tool that references an EXISTING page (`memory_put` with `expected_version`, `memory_check`, `memory_raw`, `memory_link` for BOTH source and target, `memory_link_close` mutate mode, **`memory_gap` WITH a slug** — see the carve-out below for the slug-less form): require exactly one owner collection or `AmbiguityError`. Zero owners → `NotFoundError` (cannot mutate state for a page that does not exist). The resolved collection's `state` MUST be checked against `restoring` via task 11.8 BEFORE any DB or filesystem mutation. The /16 reclassification placed `memory_check`, `memory_raw`, `memory_link`, `memory_link_close` mutate mode, and slug-BOUND `memory_gap` under WriteUpdate because they all mutate DB-only state referencing a resolved page.
 - **Slug-less `memory_gap` (Read carve-out)**: `memory_gap` invoked WITHOUT a slug logs a memory-wide gap (`knowledge_gaps` row with `page_id = NULL`) that resolves no collection and references no specific page. It is classified as `Read` for routing and interlock purposes: no bare-slug resolution runs (there is no slug to resolve), no `CollectionRestoringError` interlock applies (no collection is resolved, so no `state` check is possible or meaningful), and the call SUCCEEDS during `state = 'restoring'` on any/all collections. This carve-out is intentional — memory-wide audit path is exactly the recovery-visibility channel agents need most during restore windows. Misclassifying slug-less `memory_gap` as WriteUpdate would block that audit path when it matters most; misclassifying slug-bound `memory_gap` as Read would allow page-scoped gap rows to be written against a page in a non-write-target collection without the interlock. Both directions are correctness bugs; task 1.1c, task 2.3, task 13.1, task 11.8, task 17.5www-mutators, and the spec-consistency audit (task 17.17) all encode this split. A future new mutating tool that targets a specific page SHALL follow the slug-BOUND WriteUpdate contract; a future new tool that logs memory-wide observations MAY follow the slug-less Read carve-out but MUST declare explicitly which variant applies.
 - **WriteAdmin** — collection-level mutators that do not target a single page slug (`quaid collection ignore add`, `quaid collection ignore remove`, `quaid collection ignore clear --confirm`, `quaid collection migrate-uuids`, `quaid collection add --write-quaid-id` when applied to an existing collection, any future `--set-write-target`, and any future collection-level admin mutator): resolve by collection name only (no bare-slug form applies); enforce the same `restoring`-state interlock per task 11.8 so admin changes — including frontmatter rewrites, pattern clears, and UUID writes — cannot race a restore/remap and end up applied against the wrong root. The spec-consistency audit (task 17.17) enforces that every authoritative WriteAdmin enumeration lists this full set.

`AmbiguityError` includes the full list of candidate `<collection>::<slug>` strings so the caller can pick.

**Alternatives considered:**

- *Bare slugs always resolve to write-target* — the original design. Silently misroutes reads and updates to the wrong page when the same slug exists in multiple collections. Called out by the Codex review as a real behavior break. Rejected.
- *Ban bare slugs entirely once multiple collections exist* — safest but hostile to single-collection users who then can't just add a second collection without breaking every hardcoded agent call at once. The "unique-match" rule preserves the common case of bare slugs for globally-unique names.
- *Prefer write-target silently, but warn* — mixes "silent misroute" with "log noise"; still wrong semantically.

**Rationale:** Ambiguity should be an error, not a guess. A caller using a bare slug is either working in a single-collection setup (in which case the rule is trivially satisfied) or has a globally-unique slug (also fine) or needs to be told to disambiguate (which this rule does explicitly). The additional work to count owners on every resolution is one cheap indexed query.

### Atomic staged restore: absent-target-only, prepare → verify → Tx-A (pending_root_path) → atomic rename → Tx-B (recoverable finalize)

**Decision:** `quaid collection restore` operates only when the target path is absent or exists as an empty directory. The command writes every page to a sibling staging directory (`<target>.quaid-restoring-<uuid>/`), verifies file count and per-file sha256 against expectations computed from the DB, removes the target if it's an empty directory (`rmdir` succeeds only on empty dirs), and then runs a **two-phase** DB commit around the atomic `rename()`:

- **Tx-A (pre-rename intent).** Sets `collections.pending_root_path = <target>`. This is the durable "rename is imminent" signal that enables recovery if the post-rename finalize fails.
- **Rename.** Atomically `rename()` the staging directory onto the target.
- **Tx-B (post-rename finalize — idempotent, `run_tx_b` canonical SQL).** The single authoritative finalize SQL path (per task 17.17(l)) is `UPDATE collections SET root_path = <target>, pending_root_path = NULL, pending_restore_manifest = NULL, integrity_failed_at = NULL, pending_manifest_incomplete_at = NULL, pending_command_heartbeat_at = NULL, restore_command_id = NULL, restore_command_pid = NULL, restore_command_host = NULL, restore_command_start_time_unix_ns = NULL, state = 'active', needs_full_sync = 1, reload_generation = reload_generation + 1, watcher_released_session_id = NULL, watcher_released_generation = NULL, watcher_released_at = NULL WHERE id = ?` plus `DELETE FROM file_state WHERE collection_id = ?` in one tx. The full restore-command identity tuple (`restore_command_id` + `restore_command_pid` / `restore_command_host` + `restore_command_start_time_unix_ns`) is cleared atomically with finalize so that no stale same-host PID-liveness match can survive into post-finalize state — every recovery path (Tx-B, orphan-recovery cleanup, and `restore-reset`) writes this same tuple back to NULL to preserve the single-source-of-truth invariant. Setting `needs_full_sync = 1` arms the write-gate — the write-interlock refuses all mutating tools against this collection until RCRT's attach-completion clears the flag. This closes the post-Tx-B pre-attach hole where `memory_put` against the restored tree would misclassify every page as `ExternalCreate` under the canonical precondition (stat succeeds + no `file_state` row → ExternalCreate → `ConflictError`), since Tx-B's `DELETE FROM file_state` left no matching rows. RCRT (task 9.7d) re-populates `file_state` via `full_hash_reconcile` against the new root during its post-Tx-B attach handoff under the per-collection single-flight mutex, THEN in the attach-completion tx clears `needs_full_sync = 0` to open the write-gate, THEN spawns the supervisor. Tx-B writes the same values on every execution; running it N times after pending state still produces one correct finalize (RCRT's guarded attach-completion UPDATE `... WHERE needs_full_sync = 1` is a no-op on re-entry so generation only bumps once per finalize-then-attach cycle).

**Round-15/59/60 motivation.** The single-phase "rename, then update root_path in one tx" design had a concrete failure mode flagged by Codex if the final DB tx fails after the rename, the vault is on disk at the target path but `root_path` still points at the old location, AND retry is blocked because the target is no longer absent. The operator gets a fully-restored tree that the tool refuses to adopt. The two-phase design with `pending_root_path` eliminates this by making "rename landed, finalize did not" a recoverable durable state. On Tx-B failure, the command does NOT attempt to reverse the rename (the vault is already on disk; processes may have opened files under it; a rename-back is racy and unnecessary). The command ALSO does NOT set `needs_full_sync = 1` — the generic recovery worker (task 6.7a) reconciles against `collections.root_path`, which at Tx-B failure time still points at the OLD vault while `pending_root_path` holds the new target; setting the flag would reconcile the wrong tree and clear it without adopting the restored vault. The worker is now explicitly state-gated to `state = 'active'` only, so even a stray flag would be skipped. Recovery is exclusively via `finalize_pending_restore(collection_id, FinalizeCaller::...)`: the originating command's own retry loop while alive (`RestoreOriginator`), the RCRT continuous sweep after the command dies (`StartupRecovery`), or the operator-driven `quaid collection sync <name> --finalize-pending` (`ExternalFinalize`). If the crash happened BEFORE rename (pending_root_path set but target directory does not exist), recovery cleans up the staging dir, reverts `state` + NULLs all pending columns, and the user can restart restore normally.

No `--force` flag is provided. POSIX `rename()` cannot atomically replace a non-empty directory with a directory, and it cannot replace a file with a directory at all. Any `--force` semantic would force the implementation into a destructive pre-delete (or multi-step swap) that is not atomic under failure — the exact invariant this decision exists to preserve. Users who need to restore onto an occupied path must move or remove the existing content themselves, making the destructive step an explicit out-of-band user action rather than a hidden side effect of the restore command.

`collections.state` values (`active | detached | restoring`) make the in-flight state explicit. Watchers and reconcilers treat `restoring` like `detached` — they skip the collection. The write-interlock (task 11.8) refuses all mutating tools during `restoring`. A second concurrent `quaid collection restore` for the same collection while `pending_root_path` is set returns `RestorePendingFinalizeError` immediately rather than opening a fresh staging pass that would conflict with the pending finalize.

On any PRE-Tx-A failure (write error, verification mismatch), the staging directory is removed and the collection returns to its prior state — `pending_root_path` was never set because Tx-A never ran, so recovery has nothing to do. On Tx-B failure, the state is recoverable as described above.

**Alternatives considered:**

- *Write directly to target path, update DB as we go* — the original design. A mid-flight failure leaves the collection pointing at a partial vault; the next reconciliation interprets missing remainder as real deletes and cascades DB-only state away. Rejected.
- *Single-phase "rename then finalize in one tx" (rounds 7–14)* — Codex flagged the post-rename DB-failure hole: vault on disk, root_path stale, retry blocked by non-empty target. Rejected in favor of the two-phase `pending_root_path` intent + idempotent Tx-B finalize + auto-recovery path.
- *Reverse the rename on Tx-B failure* — rename-back is not safe: other processes may have opened files at the target between rename and Tx-B attempt; the rename-back can race with concurrent reads and leave arbitrary state. Worse, it means Tx-B must always succeed OR always be able to reverse, which is strictly more failure surface than "finalize is idempotent, finish later." Rejected.
- *WAL-style pre-write journal that records "intend to rename source to target"* — over-engineered; the one-bit `pending_root_path` column in the same DB carries the same semantics at a fraction of the complexity.
- *Provide a `--force` that destructively swaps occupied targets* — called out by the Codex review: `rename()` cannot atomically replace non-empty directories or files, so any `--force` implementation would be a multi-step non-atomic destructive operation that can lose the user's existing vault if interrupted. Rejected in favor of refusing non-empty targets outright.
- *Use a filesystem snapshot (APFS / btrfs)* — platform-specific; unavailable on many user systems.
- *Parent-directory swap primitive (e.g., `renameat2(RENAME_EXCHANGE)` on Linux)* — not available on macOS and not portable across the supported targets; would require platform-specific paths for a rare operation.

**Rationale:** The atomic rename of a directory onto an absent (or just-emptied) target on POSIX is a single inode operation; before it, the target path has no prior content to lose; after it, the target path is the complete restored vault. The two-phase `pending_root_path` intent turns the narrow "rename succeeded, finalize failed" crash window from a permanent inconsistency into a recoverable state that auto-heals via `finalize_pending_restore` driven by the originating command (while alive) or RCRT (after the command dies) or the `sync --finalize-pending` CLI (explicit operator trigger). The Tx-B idempotency property means recovery is trivial to reason about: re-running it either finalizes (if pending) or is a no-op (if already active). Post-finalize, RCRT's single-flight attach handoff opens a fresh `root_fd`, runs `full_hash_reconcile`, starts a watcher, and spawns a new per-collection supervisor — all under the per-collection mutex, so exactly one supervisor attaches to each owned active collection. This decomposition replaces "Tx-B must never fail" with "Tx-B is safe to finalize whenever we next open the DB," which is a far stronger correctness property.

### Symlink policy: fd-relative path walk (no canonicalize-target; no rely-on-renameat-symlink-semantics)

**Decision:** Every filesystem operation in a collection enforces the collection-root boundary using a parent-directory-fd walk. The earlier "canonicalize target + O_NOFOLLOW at rename" approach was rejected in round 8 because (a) `canonicalize()` fails on non-existent paths (so it cannot protect the create path at all) and (b) `rename()` / `renameat()` do not follow symlinks at the destination — `O_NOFOLLOW` "on the rename" is not a real guard, it's an application-layer check that must be spelled out explicitly.

The actual algorithm:

1. **Parse-time rejection:** reject `..` components, absolute paths, empty segments, and NUL bytes.
2. **Trusted `root_fd`:** at serve-start (and at `collection add`), open the collection's `root_path` with `openat(AT_FDCWD, root, O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`. If this fails with `ELOOP`, the root itself is a symlink — refuse to attach and mark the collection detached.
3. **fd-walk of path components:** for every filesystem operation that targets a path under the collection, walk component-by-component from `root_fd` using `openat(current_fd, component, O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`. `ELOOP` means a symlinked ancestor — reject. Missing intermediate directories on the write path are created with `mkdirat(current_fd, component, 0o755)` and re-opened with `O_NOFOLLOW` (which catches a TOCTOU attacker planting a symlink at the newly-created name).
4. **Terminal write:** `openat(parent_fd, tempfile_name, O_CREAT | O_EXCL | O_WRONLY | O_NOFOLLOW | O_CLOEXEC)` for the tempfile. `O_EXCL` refuses to overwrite an attacker-pre-placed file. `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)` before rename: if an entry exists at the target name AND is a symlink, reject and unlink the tempfile. `renameat(parent_fd, tempfile_name, parent_fd, target_name)` — both names scoped to the trusted parent fd; no path lookup happens at rename.
5. **Terminal read / walk / watcher:** `fstatat(parent_fd, name, AT_SYMLINK_NOFOLLOW)` and skip any entry that is a symlink, with a WARN log. Walks never descend into symlinked directories.

All of this uses `rustix` (or `nix`) — POSIX-portable across macOS and Linux. Windows is explicitly OUT OF SCOPE for vault-sync and agent-writes: the fd-relative algorithm does not port directly to `CreateFileW` + reparse-point semantics without re-introducing the trust-boundary weaknesses (junction/symlink TOCTOU, non-handle-scoped renames) that the POSIX algorithm eliminates. The `quaid` binary MAY still build on Windows for offline operations that don't touch a collection root, but `quaid serve`, `quaid put`, `quaid collection add`, and `quaid collection restore` SHALL refuse to run on Windows with a clear unsupported-platform error. A future OpenSpec change MAY add secure Windows-native support via handle-based path resolution (`NtCreateFile` with `OBJ_DONT_REPARSE`, `GetFileInformationByHandleEx`) — that work is separable and not part of this change.

**Alternatives considered:**

- *Relative-path rejection only (round 1)* — a symlink inside the vault redirects writes anywhere. Rejected.
- *Canonicalize target + prefix-match + `O_NOFOLLOW` at rename (round 6)* — Codex round 8 showed this is unimplementable on the create path (`canonicalize()` fails on non-existent targets) and misleading at rename (`renameat` doesn't open the destination). Rejected.
- *One-shot canonicalize at `collection add`* — symlinks created later still escape. Rejected.
- *Disallow symlinks anywhere in the vault* — overly restrictive; real Obsidian vaults legitimately use symlinks for attachment folders and reference directories. Skip-with-WARN is less hostile.
- *Follow-and-trust (treat symlinked files as managed)* — breaks the "vault root contains all managed files" invariant; silently indexes content the user didn't intend to manage. Rejected.
- *Bind-mount the collection root into a chroot-like sandbox* — correct but requires privileges and disk-layout cooperation we don't have as a user-space tool. Out of scope.
- *Cross-platform canonicalize-plus-lock fallback on Windows (the earlier stance)* — Codex flagged that this reintroduces the exact path-escape mechanism the spec just declared unsafe, just scoped to Windows. A trust-boundary that only holds on POSIX is not a trust boundary. Rejected in favor of explicitly marking Windows unsupported for vault-sync/agent-writes until a handle-based implementation lands.

**Rationale:** The fd-relative walk is the standard POSIX pattern for safe path resolution inside a trust boundary — it is exactly what `systemd`, container runtimes, and security-sensitive daemons use. Each `openat` step with `O_NOFOLLOW` gives a trusted fd for the next step; once we have `parent_fd`, `renameat(parent_fd, src, parent_fd, dst)` is genuinely scoped (not a path-resolution operation at all). The target-existence `fstatat(..., AT_SYMLINK_NOFOLLOW)` check before rename is defense-in-depth: `renameat` alone would replace a symlink at the target name with our file (not follow it), which is safe-but-silent; surfacing the symlink-at-target case as an explicit refusal makes attacks visible. The cost is a handful of extra syscalls per write — unnoticeable in practice — and the correctness/safety gain is substantial.

### Byte-exact restore via `raw_imports` (strict invariant — restore has exactly one source of bytes)

**Decision:** Under v5, EVERY content-changing write rotates `raw_imports` — ingest, re-ingest, `memory_put` create, `memory_put` update, UUID write-back — without exception. The invariant is absolute: every page has exactly one active `raw_imports` row at all times, and restore reads those bytes directly. There is no happy-path branch that materializes a page via `render_page()`; the restore code has a single source of bytes, which makes byte-exact fidelity a property of the invariant itself rather than of the restore algorithm.

If restore ever encounters a page with zero active `raw_imports` rows, that is definitionally a corruption or invariant violation — not a supported state. Restore aborts with an `InvariantViolationError` and leaves the collection detached. An explicit `--allow-rerender` flag (undocumented outside error messages, audit-logged WARN per page) exists for operator-driven last-resort recovery, making the render-as-fallback path an explicit and observable user action rather than a silent degrade.

**Alternatives considered:**

- *Always re-render via `render_page()`* — loses original formatting/comments/whitespace. Rejected in round 6.
- *Dual contract (ingest rotates; `memory_put` does not)* — rejected in round 8 as self-contradictory.
- *Keep a silent `render_page()` fallback when raw_imports is missing*: one implementer treats missing rows as corruption, another treats them as a valid agent-authored-page branch; restore fidelity diverges invisibly. Rejected — the invariant is only real if the spec refuses to describe a happy-path branch that ignores it.
- *Schema-level `CHECK` constraint requiring an active `raw_imports` row per `pages` row* — correct in principle but would fail page-insert transactions until raw_imports is inserted in the same tx; adds ordering brittleness across every write site. We instead enforce the invariant at every write site (task 5.4d, 5.4e, 12.4c) and validate it continuously via the post-ingest unit-test assertion `COUNT(is_active=1) = 1` plus the on-demand `quaid collection audit` check.
- *Drop `raw_imports` and rely on `render_page()` roundtripping* — would require `render_page()` to be byte-exact for every syntax variant, which it is not (`tests/roundtrip_semantic.rs` vs `tests/roundtrip_raw.rs`). Out of scope.

**Rationale:** The value of an invariant comes from its enforcement. Allowing a "defensive fallback" re-creates the ambiguity the invariant exists to prevent. The strict stance — abort and require an explicit override for the corruption-recovery path — is uncomfortable the first time it fires but correct: it guarantees that byte-exact restore is either achieved or loudly refused, never silently compromised. Users who face a genuinely corrupt database and cannot recover otherwise retain a path (`--allow-rerender`), and that path is audit-logged so its use is always visible.

### Active `raw_imports` rotation on every content-changing write

**Decision:** The active `raw_imports` row for a page SHALL always reflect the page's current on-disk bytes. Every content-changing write — initial ingest, reconciler/watcher re-ingest after external edit, `memory_put`, and the UUID write-back self-write — rotates the active row IN THE SAME SQLite transaction as the `pages` / `file_state` update:

```sql
UPDATE raw_imports SET is_active = 0 WHERE page_id = ? AND is_active = 1;
INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path, created_at) VALUES (?, ?, 1, ?, ?, ?);
```

Under the rename-before-commit write sequence, rotation and the rest of the DB update are a single atomic tx that runs AFTER the rename lands. A pre-rename failure never rotates `raw_imports` (no DB tx ran). A post-rename failure (steps 9–11) leaves `pages`/`file_state`/`raw_imports` all unchanged relative to pre-call state; recovery via `needs_full_sync` re-ingests from disk, installing a fresh active row matching on-disk bytes. There is no "revert rotation" compensating transaction — the rotation simply didn't happen. The existing partial index `idx_raw_imports_active ON raw_imports(page_id, is_active) WHERE is_active = 1` gives O(1) lookup for restore.

**Bounded retention for inactive rows.** Prior inactive rows are retained subject to an explicit per-page cap (`QUAID_RAW_IMPORTS_KEEP`, default 10) AND an age threshold (`QUAID_RAW_IMPORTS_TTL_DAYS`, default 90). Round-14 Codex flagged unbounded retention as a disk-pressure risk for live-watched vaults: an Obsidian user's autosave cycle can produce hundreds of writes per page in a working session, each of which would otherwise persist forever. With retention, steady-state storage per page is `O(active bytes) + O(KEEP × average page size)` instead of `O(write count × average page size)`. GC runs inline with every rotation (same tx) AND on a periodic daily sweep (needed because TTL can expire rows on idle pages with no rotation trigger). The active row is NEVER subject to GC. `QUAID_RAW_IMPORTS_KEEP_ALL=1` disables retention for users who genuinely want full edit history (forensic/research workflows). `quarantine export` captures whatever inactive rows survive retention at export time, with the export header recording the effective policy so consumers can tell elision from absence.

**Alternatives considered:**

- *Populate `raw_imports` only at first ingest (original design)* — Codex flagged this as a recovery-time data-loss path: a file edited after first ingest restores from the old snapshot, silently rolling back the user's edits. For pages that lacked `quaid_id` at first ingest, the restored file also loses the UUID, breaking later rename detection. Rejected as unsafe for the live-sync model.
- *Disable `raw_imports` as the restore source; treat it as historical export only; always restore via `render_page()`* — removes the byte-exact recovery path that introduced for exactly this reason. Rejected.
- *Rotate raw_imports AFTER the pages/file_state commit (separate tx)* — creates a crash window where `pages` is current but `raw_imports` is stale; restore could emit pre-edit bytes. Rejected in favor of same-tx rotation.
- *Keep every historical raw_imports row active and let restore pick the latest `created_at`* — loses the partial-index fast path and complicates the "one active row per page" invariant the index already enforces. Rejected.
- *Retain all inactive rows forever (rounds 1–13)*: autosave cycles would bloat `memory.db` in proportion to edit count rather than corpus size, eventually creating disk-pressure or ENOSPC in the subsystem meant to improve usability. Rejected in favor of `KEEP` + `TTL_DAYS` bounded retention with a `KEEP_ALL=1` opt-out.
- *Compress or delta-encode inactive rows instead of GC* — introduces a new encoding path (and decoding for export), and doesn't actually bound growth; a 1000× compression ratio still grows with edit count. Rejected as solving the wrong axis. Compression is still a viable future optimization on top of bounded retention but is out of scope here.

**Rationale:** The existing schema already has `is_active` and a partial index optimizing for exactly this access pattern — rotation is the access pattern the schema was designed for. Same-transaction rotation means a crash between writes cannot produce a state where `pages.version` has advanced but restore would emit the prior bytes. Retention bounds (`KEEP` + `TTL_DAYS`) convert inactive rows from an unbounded append-only log into a bounded recent-history buffer that covers "oops, I overwrote this an hour ago" without promising a permanent forensic record. Users who need the latter set `KEEP_ALL=1` and accept the storage tradeoff explicitly. Inline GC in the same tx as rotation keeps the invariant simple; the periodic sweep backfills TTL-expired rows on idle pages.

### Live `.quaidignore` reload — atomic parse, last-known-good on failure

**Decision:** `.quaidignore` is a watched control file, not content. When the per-collection watcher observes a write, delete, or rename affecting `<collection_root>/.quaidignore`, the system performs an atomic whole-file parse — every non-empty, non-comment line is validated via `globset::Glob::new` BEFORE any effect is applied. Exactly two outcomes:

- **Fully-valid parse** — `collections.ignore_patterns` is refreshed as the cached mirror of the file's validated user patterns (defaults are merged in code at reconciler-query time, not stored in the column); `collections.ignore_parse_errors` is cleared; an immediate reconciliation runs. Newly-matching files are hard-deleted or quarantined per the DB-only predicate; newly-un-matched files are ingested as `new`. A PRESENT file with zero non-comment non-whitespace lines parses cleanly to "zero user patterns" and takes this path (mirror set to empty user-pattern set; defaults still apply at query time). An ABSENT file does NOT unconditionally take this path — absence is three-way per the "`.quaidignore` is authoritative" decision below (fresh-attach/no-prior-mirror takes the defaults-only path; prior-mirror-present with the default opt-out UNSET is fail-closed; prior-mirror-present with `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1` OR explicit `ignore clear --confirm` clears the mirror).
- **Any line fails** — `collections.ignore_patterns` is UNCHANGED (last-known-good mirror preserved — the file is NOT re-read on the failed parse, so the mirror continues to reflect the last successful file contents); `collections.ignore_parse_errors` records the failing lines; NO reconciliation runs; previously-applied exclusions remain active until the file is fixed.

Parse errors surface via WARN log, `memory_collections` MCP `ignore_parse_errors` field, and `quaid collection info` — operator sees exactly which line is malformed.

CLI `quaid collection ignore add|remove` validates the resulting pattern set BEFORE persisting any state. A malformed pattern is refused at the CLI layer (no DB write, no `.quaidignore` file write, non-zero exit code).

**Alternatives considered:**

- *Per-line fail-closed — apply the valid subset, drop invalid lines*: an editor's intermediate save (truncated write, mid-edit garbage, a line accidentally deleted) could drop a protective `private/**` pattern, trigger reconciliation, and ingest previously-excluded files into FTS/embeddings before the user noticed the typo. The per-line approach trades "take effect as much as possible" for privacy safety — wrong tradeoff for a control file whose purpose IS privacy. Rejected.
- *Abort the reload and preserve prior patterns with only a log* — rejected this for the "first-time edit with a typo" case. Re-examined: with atomic-all-or-nothing the user sees a clear error AND has a clear recovery path (fix the file, save again). The worry was overstated — a first-time edit with a typo is rare, operator-visible, and self-correcting; the editor-glitch case from is common, invisible without the guard, and privacy-destroying. Accepted as the current design.
- *Silently strip invalid lines* — loses debuggability. Rejected.
- *Reload-at-`sync`-only* — rejected (live editing is the contract).
- *Serve refusal / 500s while parse is broken* — catastrophic over-reaction. Rejected.

**Rationale:** The mechanical invariant is: "no non-deliberate edit can deactivate a protective pattern." Removing an exclusion requires saving a fully-valid file — a deliberate, observable act — not accidentally via a typo on an adjacent line. Parse errors surface in three operator-visible channels so the fix is obvious. The tradeoff — "a first-time privacy edit with a typo doesn't take effect until the typo is fixed" — is acceptable because the error is loud and the delay is bounded to how long the user takes to fix the typo (seconds). The tradeoff we avoid — "a protective pattern silently disappears mid-edit" — is not acceptable for a confidentiality boundary.

### Two-tier indexing: FTS synchronous, embedding deferred via queue

**Decision:** FTS + metadata updates commit in the same SQLite transaction as `file_state` upsert. Embeddings are enqueued in an `embedding_jobs` table and drained by a background worker with bounded concurrency (`min(cpus, 4)`).

**Alternatives considered:**

- *Everything synchronous* — a 50-file bulk save stalls agent queries for the embedding duration.
- *Everything async* — breaks the `put → get` pattern agents rely on.
- *In-memory queue* — loses pending work on crash; doesn't survive `quaid serve` restart.

**Rationale:** FTS and embedding have orders-of-magnitude-different cost profiles. Separating them matches the cost. Persistent queue survives crashes — startup reconciliation resumes where the worker left off. Hybrid `memory_search` always finds newly-saved pages via the FTS lane; the semantic lane catches up within seconds.

### Stat-diff with ctime/inode invalidation; full-hash on remap; periodic audit

**Decision:** `file_state` stores `(collection_id, relative_path, mtime_ns, ctime_ns, size_bytes, inode, sha256, last_seen_at, last_full_hash_at)`. Steady-state reconciliation uses the four-field stat tuple `(mtime_ns, ctime_ns, size_bytes, inode)` as its short-circuit. A file is skipped without hashing only if ALL four fields match; any mismatch triggers a re-hash and (if sha256 differs) re-ingest.

Beyond steady-state, three stronger paths are required:

1. **Full-hash on remap/fresh-attach/first-use-after-detach.** After `quaid collection sync --remap-root`, `quaid collection restore`, or the first serve startup after a collection was detached, stat fields from the prior filesystem are meaningless. The system ignores stat-diff and hashes every file. `last_full_hash_at` is refreshed on completion.
2. **Periodic full-hash audit.** A background task (default interval 7 days via `QUAID_FULL_HASH_AUDIT_DAYS`) rehashes files whose `last_full_hash_at` is older than the interval. Audit work is spread across cycles (a daily task processes ~1/N of the vault) so a 10k-file vault doesn't all hash at once. `quaid collection audit <name>` is the on-demand trigger.
3. **Merkle subtree hashes (future extension).** The `fs_tree(collection_id, dir_path, subtree_hash, last_computed_at)` schema is designed for cross-machine sync but not created in this change.

**Alternatives considered:**

- *mtime+size only (the original design)* — Codex called out that `git checkout`, rsync with `--times`, backup restores, and in-place truncate-and-rewrite can all preserve `(mtime, size)` while changing bytes, leaving the index silently stale. Rejected as unsafe for real workflows.
- *Always hash every file on every walk* — 10k-file walk becomes tens of seconds on every startup. Bad steady-state experience.
- *Hash only on mtime mismatch (skip ctime/inode)* — ctime is kernel-enforced on POSIX (user-space can't backdate it), and inode changes on create-and-rename patterns like `git checkout` or restore-to-temp-then-rename. Cheap to include; closes the most common drift paths.
- *Skip periodic audit; rely only on stat-diff and explicit triggers* — works for disciplined workflows but doesn't catch the long-tail drift cases. The audit is the belt-and-suspenders backstop; 7-day cadence is low enough impact to justify.
- *Full Merkle from day one* — marginal benefit over stat-diff + audit on local SSD; adds schema and code for a future feature.

**Rationale:** Stat-diff is fast; ctime+inode closes nearly all real-world drift paths for free (four stat fields cost the same as two); full-hash on remap/attach makes cross-machine moves correct by construction; periodic audit catches the residual adversarial cases without user intervention. The combination gives correctness AND sub-second steady-state performance. We accept that an audit that finds drift means some prior state was briefly stale — fine for an eventually-consistent personal knowledge tool.

### `.quaidignore` is authoritative; `collections.ignore_patterns` is a cached mirror

**Decision:** `.quaidignore` on disk is the sole source of truth for user-authored ignore patterns. `collections.ignore_patterns` is a cached mirror populated from the file on every successful atomic parse; it is NOT independently authoritative. Built-in defaults (`.obsidian/**`, `.git/**`, `node_modules/**`, `_templates/**`, `.trash/**`) are always applied in addition to user patterns (the cache column stores user patterns only; defaults are merged in code). The sync is one-way (file → DB cache), transactional (atomic parse writes the full mirror or leaves it at last-known-good), and mtime-free (no last-writer-wins comparison). The watcher treats `.quaidignore` as a live control file and triggers immediate atomic parse + mirror update + reconciliation on change (see "Live `.quaidignore` reload with atomic parse" decision). CLI `quaid collection ignore add|remove|clear --confirm` is **dry-run first, file-write second, mirror-refresh last**: the CLI computes the proposed `.quaidignore` contents in memory from the current file plus the requested transformation, runs the same atomic-parse validator against the proposed contents, refuses with NO disk mutation and NO DB mutation on any parse error, and only writes `.quaidignore` to disk on a fully-valid dry-run. The DB mirror is refreshed EXCLUSIVELY by `reload_patterns()` — either via the watcher's self-observed event (serve running) or the next `quaid serve` startup (serve not running). The CLI NEVER writes `collections.ignore_patterns` directly (enforced by code-audit test 17.5qq9). **Absent-file semantics are three-way:** (a) absent with no prior mirror (`ignore_patterns IS NULL`) → fresh-attach path, mirror stays NULL, reconciler applies defaults only; (b) absent with a prior mirror AND `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE` unset → fail-closed, mirror UNCHANGED, `ignore_parse_errors = file_stably_absent_but_clear_not_confirmed`, WARN log, NO reconciliation; (c) absent with `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1` OR the user ran `quaid collection ignore clear <name> --confirm` → mirror cleared, reconciliation runs. Round-17 rejects the earlier last-writer-wins-by-mtime arbiter (timestamps are unreliable), and rejects the original "absent always clears" default because a transient delete/editor-crash/sync-glitch is indistinguishable from user intent and the confidentiality boundary (`private/**`-style patterns) is too sensitive to treat ambiguous absence as clear.

**Rationale for file-authoritative rather than DB-authoritative.** `.quaidignore` moves with the vault; a user who moves or restores the vault expects their ignore patterns to come with it. Making the file authoritative means portability is automatic. Moving `memory.db` alongside the vault still works because the next serve start re-reads `.quaidignore` and refreshes the cache. DB-only patterns would require export/re-import and would silently drift when a user edits the file directly (the very case `.quaidignore` exists for).

**Rationale:** `.quaidignore` keeps config portable with the vault (version-control friendly). DB column means moving `memory.db` alone preserves config. Built-in defaults mean users who never touch ignore patterns still get a clean index. Live-reload preserves the privacy invariant users expect from a `.gitignore`-style control file.

### Live-serve coordination for restore/remap — session ownership + polled rebind

**Decision:** `quaid collection restore` and `quaid collection sync --remap-root` mutate `collections.root_path`, which invalidates any `root_fd` and watcher a running `quaid serve` holds for the collection. Rather than rely on process restart (fragile — users forget) or signal-handling across processes, we coordinate through SQLite for the restore/remap handshake. This is ORTHOGONAL to the decision to adopt a per-session UNIX socket (`serve_sessions.ipc_path`) for CLI write proxying (`quaid put` / `migrate-uuids` / `--write-quaid-id` → owning serve, per `agent-writes/spec.md`): the UNIX-socket channel was ORIGINALLY listed below as a rejected alternative for THIS handshake, and it remains rejected for the handshake specifically (polled SQLite is strictly sufficient and portable), but the SAME channel is adopted for the separate live-owner write-routing contract. The two sub-decisions live in different requirements and do not conflict: handshake coordination uses polled SQLite; write proxying uses `ipc_path`. Any implementer reading this section MUST also implement `serve_sessions.ipc_path` per `agent-writes/spec.md` — omitting it reopens the direct-write race under a live watcher.

- Serve maintains a row in a `serve_sessions` table with a heartbeat refreshed every 5 seconds; stale rows (>15s) are ignored and swept at the next serve startup.
- Restore/remap checks for a live session; without `--online`, refuses with `ServeOwnsCollectionError` naming the owning pid/host.
- With `--online`, the handshake is **lease-based and bound to `(session_id, reload_generation)`** — a bare timestamp is not sufficient. Schema carries three ack fields on `collections`: `watcher_released_session_id TEXT NULL`, `watcher_released_generation INTEGER NULL`, `watcher_released_at TEXT NULL`. Protocol (lease-based via `collection_owners`, /55/56/57/61/63 RCRT-driven reattach):
 - Command captures `expected_session_id` from `collection_owners` — NOT from an arbitrary `serve_sessions` row. `collection_owners(collection_id PRIMARY KEY, session_id, acquired_at)` is the single-owner lease per collection (task 11.6), so the row uniquely identifies the serve session currently responsible for this collection. The command verifies `serve_sessions.heartbeat_at > now() - 15s` for that same `session_id` to confirm the owner is live. Using `collection_owners` (not `serve_sessions`) is mandatory: a live non-owner session could otherwise write an ack that the command would accept, while the REAL owner continues holding its watcher/`root_fd` on the old tree — reopening the split-memory the lease model closes by construction. Then in one tx sets `state = 'restoring'`, computes `cmd_reload_generation = reload_generation + 1`, writes it back, NULLs the three ack fields (wiping any leftover from a prior handshake), and sets `pending_command_heartbeat_at = now()`.
 - Serve's per-collection supervisor polls `state` + `reload_generation` every 250ms; on observing `state = 'restoring'` with a strictly greater generation, it releases the watcher and `root_fd`, writes the ack triple in one tx (`watcher_released_session_id = <own session_id>`, `watcher_released_generation = <observed generation>`, `watcher_released_at = now()`), AND EXITS (removes its entry from the process-global `supervisor_handles` registry per task 11.7's contract). Serve NEVER writes an ack on behalf of a different session (do-not-impersonate rule).
 - Command polls for the exact match `(watcher_released_session_id = expected_session_id) AND (watcher_released_generation = cmd_reload_generation) AND (watcher_released_at IS NOT NULL)`. Stale acks from prior generations and foreign-session acks never match, so they are rejected by construction. Concurrently, command re-reads BOTH `serve_sessions` for `expected_session_id` AND `collection_owners` for the collection on every poll: if the `serve_sessions` row disappears or its `heartbeat_at` ages past 15s, command aborts with `ServeDiedDuringHandshakeError`; if `collection_owners.session_id` has changed to a DIFFERENT session (the original owner crashed and a successor claimed the lease mid-handshake), command aborts with `ServeOwnershipChangedError`. Both short-circuit the 30s timeout.
 - On timeout or early abort, command reverts `state` to its prior value, NULLs `pending_command_heartbeat_at`, clears the ack triple, bumps `reload_generation` to `cmd_reload_generation + 1` as the ordering marker, cleans up any staging directory, and returns non-zero. RCRT's next sweep observes the owned active collection with no live `supervisor_handles` entry and performs the single-flight attach handoff (opens new `root_fd`, runs `full_hash_reconcile`, starts watcher, spawns a new supervisor) — expected latency: up to `QUAID_DEFERRED_RETRY_SECS` (default 30s). No serve restart required.
 - On success, command proceeds with stage/verify/atomic-rename and calls `finalize_pending_restore(..., FinalizeCaller::RestoreOriginator { command_id })` which runs Tx-B via `run_tx_b`, flipping `root_path` to the new target, setting `state = 'active'`, bumping `reload_generation` again, clearing the ack triple and ALL pending/integrity/command columns. RCRT's next sweep observes the owned active collection with no live supervisor handle and performs the single-flight attach handoff against the NEW `root_path` (opens `root_fd`, runs `full_hash_reconcile` — required because stat fields from the prior root are meaningless — starts watcher, spawns a new supervisor). The command exits 0 as soon as the helper returns `Finalized`; it does NOT wait for RCRT attach. No serve restart required.
 - **Serve startup do-not-impersonate rule:** a fresh serve that observes `state = 'restoring'` at startup SHALL NOT write the ack triple. It treats the collection as detached (no supervisor spawned) until RCRT's sweep drives the collection to `active` OR the originating command completes or aborts the handshake. This guarantees that only the session that was live when the command captured `expected_session_id` from `collection_owners` can produce an accepted ack; a successor session cannot impersonate its predecessor.
- The `root_fd` lifetime is scoped to the collection session (the interval while `state = 'active'` under the current `root_path`), NOT the life of the serve process. This correction was required to make the earlier "life of the serve process" language true under restore/remap.

**Alternatives considered:**

- *Require serve restart after any restore/remap (the earlier implicit model)* — Codex showed this left a split-memory window: DB says root moved, serve still watches the old tree until the user remembers to restart. Rejected as user-hostile and fragile.
- *Inter-process signals (SIGUSR1 on the serve pid)* — works on POSIX but not Windows, and requires the command to know the serve pid reliably (we'd end up needing the heartbeat table anyway). The polled-state mechanism is portable and uses infrastructure we already have (SQLite).
- *SQLite update hooks / `sqlite3_update_hook` pushed to serve* — SQLite update hooks fire only on the connection that performed the write, not on other processes. Useless here.
- *Named-pipe / UNIX-socket IPC between CLI and serve for the handshake* — adds a second IPC channel beyond the MCP stdio, when polled-SQLite already suffices for the generation/ack handshake at negligible cost. Rejected for handshake coordination. **Superseded for write proxying:** a separate, narrowly-scoped UNIX-socket at `serve_sessions.ipc_path` IS adopted for CLI → serve write forwarding (see `agent-writes/spec.md` "CLI write routing when serve owns the collection"). The two use-cases are orthogonal: handshake coordination is polled-SQLite; write proxying goes over the socket. This bullet remains to explain the handshake decision; it does NOT reject the write-proxy socket.
- *File-lock on the collection root directory* — works cross-platform but conflates access-rights with coordination and doesn't convey the "release watcher now" semantics we need.
- *Always refuse online restore, require manual stop+restore+start* — simpler spec but user-hostile, and we already have all the pieces (state column, heartbeat row, polled state) for a clean in-process rebinding. The `--online` opt-in with a hard default of "refuse" preserves the "user must intentionally choose" property.
- *Ack by timestamp only (`watcher_released_at >= tx_start_time`)* — the original design. Round-14 showed this is unsafe: a stale delayed write from an earlier timed-out handshake, or from a serve instance racing shutdown/restart, can satisfy a later command even when the current owner has not released. Rejected in favor of the three-field lease above.
- *Ack by a single opaque request UUID* — would work but still has to be correlated to the live session to protect against the "new serve impersonates predecessor" race. Binding directly on the pair `(session_id, reload_generation)` requires no new column beyond those we already have semantic reasons for, and the generation bump doubles as the signal serve is already polling on.

**Rationale:** The `serve_sessions` heartbeat + `collections.reload_generation` polling pattern uses infrastructure we already rely on (SQLite) to solve a coordination problem that is fundamentally about "does anyone hold an fd I'm about to invalidate." The 250ms poll is fast enough that users perceive `restore --online` as instantaneous yet cheap enough to run continuously without CPU impact. The hard default (refuse without `--online`) makes the cross-process implication visible instead of trying to "just make it work magically." Stale-row handling (15s liveness) covers the crashed-serve case without manual cleanup. Binding the ack to `(session_id, reload_generation)` — rather than a bare timestamp — makes stale and foreign-session acks impossible to confuse for a live release, closing the handshake race.

**Decision:** `quaid serve` spawns one `notify` watcher per collection on a tokio task. Events flow into a single `tokio::mpsc` channel tagged with `CollectionId`. A per-collection debounce buffer coalesces events; a batch processor runs stat-diff and commits to SQLite; a single embedding worker drains `embedding_jobs` across all collections.

**Alternatives considered:**

- *Separate sync daemon process, IPC to serve* — two processes to supervise, two launchd plists, clearer separation of concerns but operational overhead for benefits we don't need at this stage.
- *Per-collection worker processes* — over-isolated; SQLite WAL already handles shared-connection concurrency safely.

**Rationale:** Single process, single supervision story. Panic in one collection's pipeline is caught and restarted with backoff; other collections unaffected.

### Short-TTL self-write dedup set suppresses watcher echoes

**Decision:** The dedup insert is placed at **step 8** of the rename-before-commit write sequence — AFTER the tempfile is written+fsync'd AND the defense-in-depth symlink check passes, and IMMEDIATELY BEFORE `renameat` (step 9). This ordering aligns with the authoritative 13-step sequence in `specs/agent-writes/spec.md` and the `memory_put write sequence` decision earlier in this file; the SQLite commit is step 12 and does NOT precede the dedup insert; the recovery sentinel (step 5) precedes both. `memory_put` inserts `(target_path, new_sha256, now)` into an in-memory `Arc<Mutex<HashMap>>`; when the watcher receives an event for that path, it consults the set: if path + hash match an entry younger than 5s, drop the event. Background sweeper removes expired entries every 10s.

On pre-rename failure (any error at steps 1–8 inclusive): the dedup entry was NOT yet inserted (or was inserted at step 8 and fails between 8 and 9), so there is nothing — or at most one entry — to clean from the set. The tempfile is unlinked, the sentinel is unlinked, and the caller receives an error; no DB mutation occurred.

On rename failure (step 9): the dedup entry was inserted in step 8 but the rename did not happen; the handler SHALL remove the dedup entry, unlink the tempfile, and unlink the sentinel (no disk drift). No DB mutation occurred.

On post-rename failure (steps 10–12): the dedup entry is live and the bytes are on disk. The handler SHALL remove the dedup entry so the reconciler's post-rename recovery (driven by the filesystem sentinel, and optimistically by `collections.needs_full_sync = 1` set via a fresh SQLite connection) is NOT suppressed — the recovery needs to observe the disk state and re-ingest. See the rename-before-commit write sequence in `specs/agent-writes/spec.md` and tasks 12.4 / 12.4b / 12.4d for the failure-handler specifics.

**Alternatives considered:**

- *Database-backed dedup* — unnecessary; in-memory is safe because startup reconciliation handles any missed events after a crash.
- *Path-only match* — fails when a user edits the same path externally within the TTL window.
- *Dedup insert AFTER the SQLite commit (rounds 1–11 ordering)* — was valid under the earlier commit-before-rename design where the final rename happened after commit; became incompatible with rename-before-commit because under the new ordering there IS no "after commit but before rename" window (rename precedes commit). Historical note; superseded by step-7 ordering.

**Rationale:** Simple, fast, crash-safe (losing the set on crash means the next startup walk catches up via stat-diff). Hash match prevents a rapid-fire external edit being masked by our own stale entry. Inserting the dedup entry at step 7 — immediately before `renameat` — minimizes the window between dedup-visibility and the watcher's `Rename` event firing; the 5-second TTL absorbs any filesystem-propagation lag. Inserting BEFORE the rename rather than after guarantees that however fast the watcher delivers the rename event (FSEvents/inotify can fire within microseconds), the dedup set is already primed to suppress it — inserting after would create a window where the echo lands before suppression is in place.

## Risks / Trade-offs

- **[Hard-delete semantics]** an accidental `rm` deletes pages. *Mitigation:* existing vault-level backups (Time Machine, Obsidian Sync, git) are the user's responsibility, as with any "disk is truth" tool. Documentation warns clearly.
- **[Watcher drops on network filesystems]** `notify` native backends can miss events on NFS, SMB, Dropbox. *Mitigation:* per-collection `watcher_mode` flag with `poll` fallback; startup reconciliation catches up regardless.
- **[Embedding worker backlog under write stampede]** bulk paste of 1000 files queues 1000 embedding jobs. *Mitigation:* semantic search is eventually consistent (FTS lane always fresh); operator-visible queue depth in `quaid collection info`.
- **[DB-authoritative state preserved via quarantine; never auto-deleted]** pages with ANY of the five DB-only categories (programmatic `links`, non-import `assertions`, `raw_data`, `contradictions`, `knowledge_gaps`) are quarantined rather than hard-deleted; ambiguous rename refusals also quarantine the old page. The TTL sweep auto-discards only quarantined pages whose DB-only-state predicate returns FALSE; pages carrying any DB-only state persist indefinitely until the user explicitly acts. *Mitigation:* `quaid collection quarantine list` shows what's pending; `discard` on a DB-only-state page requires `--force` or a prior `export` (which dumps all five categories); `quaid collection info` surfaces the awaiting-user-action count so the backlog is visible.
- **[Quarantine backlog growth for DB-only-state pages]** a user who accumulates programmatic links, assertions, raw_data, or contradictions on many pages and then bulk-deletes files will create a persistent quarantine backlog that auto-sweep does not clear. *Mitigation:* this is intentional — the alternative is silent irreversible data loss. `quaid collection quarantine export --all-older-than <duration>` + `discard --after-exported` lets power users drain the backlog in controlled batches while preserving a JSON record of what they removed.
- **[Post-rename DB-commit failure leaves disk ahead of DB]** rare but possible if SQLite commit fails after a successful `renameat`. *Mitigation:* the handler removes the dedup entry, sets `collections.needs_full_sync = 1` via a fresh connection, and returns error; the recovery task runs `full_hash_reconcile` within 1s and re-ingests from the actual on-disk bytes. Logged at ERROR for operator visibility. There is NO "ghost content" failure because `raw_imports` only commits when bytes are already on disk (see the rename-before-commit write sequence).
- **[Stat-based external-edit detection misses sub-millisecond overwrites]** an external write that preserves both mtime and size exactly could slip past the precondition check. *Mitigation:* implausible outside adversarial scenarios on a personal tool; strengthen to per-write hashing only if real cases surface.
- **[Rename inference refuses to pair duplicate-content files]** users who copy a file and then delete the original expect rename behavior but get quarantine+create. *Mitigation:* the old page is preserved in quarantine (not destroyed); logged at INFO with the specific condition that failed so users can understand the decision; `quaid collection quarantine restore` recovers the move; native rename events (which we prefer) handle the common case of renames inside Finder/Obsidian/mv regardless of content.
- **[Bare-slug ambiguity errors on agents upgrading from single- to multi-collection setups]** hardcoded bare slugs in agent code start returning `AmbiguityError` once a second collection is added and a slug collides. *Mitigation:* the error names the candidate `<collection>::<slug>` strings; agents get a clear signal rather than silent misrouting; `quaid collection list` documents the active collections so developers can update their code.
- **[Restore interrupted mid-rename]** if the process is killed after `rename()` but before Tx-B (the finalize tx that flips `collections.root_path` to the new target), the target path holds the restored vault and `collections.pending_root_path` carries the recoverable intent signal. *Mitigation:* the state is `restoring` with `pending_root_path = <target>` and `pending_restore_manifest` holding the per-file sha256 list, file count, and post-rename `rename_inode_dev` tuple captured at Tx-A. Recovery is ALWAYS invoked as `finalize_pending_restore(collection_id, caller: FinalizeCaller)` — NEVER with implicit caller identity and NEVER as an existence-only "target exists → finalize" branch. Valid callers: `FinalizeCaller::RestoreOriginator { command_id }` (the original restore command — bypasses the fresh-heartbeat defer gate by matching `collections.restore_command_id`); `FinalizeCaller::StartupRecovery { session_id }` (task 9.7d's auto-recovery on `quaid serve` start AND the continuous Restoring-Collection Retry Task sweep — this is the sole runtime backstop actor under the /55/56/57 single-actor contract); `FinalizeCaller::ExternalFinalize { session_id }` (`quaid collection sync <name> --finalize-pending`); `FinalizeCaller::SupervisorRetry { session_id }` remains defined in the helper API for test harnesses that exercise the lease-holder authority path with explicit caller identity, but has NO runtime invocation — the per-collection supervisor (task 11.7) exits after writing the release ack and does not retry finalize. Only `RestoreOriginator` bypasses the heartbeat gate; every other caller observes it. Before Tx-B runs, the helper re-validates the target against the persisted manifest — file count MUST match, every per-file sha256 MUST match, and the target-root `fstat` tuple MUST match the recorded `rename_inode_dev`. Possible outcomes: `Finalized` (all gates + manifest pass → `run_tx_b` commits, single authoritative finalize SQL per task 17.17(l)); `Deferred` (heartbeat fresh and caller is not the originator — retry later); `IntegrityFailed` (manifest mismatch — blocks the collection in `state='restoring'` with `integrity_failed_at` set, requires operator `restore-reset --confirm`); `Aborted` (target missing — clean up staging, revert `state`); `OrphanRecovered` (no pending target, originator provably dead). There is NO branch that finalizes on target existence alone — that would let a tampered or wrong tree slip past recovery. Manual deletion of the target and re-running `quaid collection restore` is NOT the recovery path — deleting would discard the restored vault that the rename already landed. Users see `pending_finalize=true pending_root_path=<P>` in the post-restore summary when this path is active; the resolution is to start serve (which triggers `StartupRecovery`) or run `sync --finalize-pending` (which triggers `ExternalFinalize`), not to delete.
- **[Restore from memory.db alone loses DB-authoritative state on fresh re-init]** programmatic links, non-import assertions, `raw_data`, and `contradictions` created via MCP don't survive a "restore vault, delete memory.db, re-ingest" cycle. *Mitigation:* documented; users who need full fidelity back up `memory.db` alongside the vault directory (the recommended pattern).
- **[Schema break]** existing `memory.db` files need re-ingest. *Mitigation:* zero users; `quaid init` is the happy path on fresh installs.

## Migration Plan

Zero users = no data migration burden. For developer-local brains:

1. v5 schema replaces v4 entirely. `quaid init` on a fresh directory creates v5.
2. `src/schema.sql` is updated to v5 DDL; `src/core/db.rs::{open,init}` preflight stored schema metadata (`quaid_config.schema_version`, falling back to legacy `config.version`) before any v5 DDL runs. If an older schema is found, error with instructions to re-init from the source vault.
3. We deliberately do not write a v4 → v5 migration. Developers re-run `quaid init` + `quaid collection add <vault-path>`.

Planned final-state deletions (NOT part of the current foundation slice):

- `src/commands/import.rs` removed
- `src/core/migrate.rs::import_dir()` removed; file split into `reconciler.rs` (walk + stat-diff) and `writer.rs` (page upsert + FTS)
- `ingest_log` table dropped from schema; `migrate.rs` helpers `is_already_ingested` / `record_ingest` / `refresh_ingest_source` removed



**Decision (landed):** `quaid import` is removed from the binary. `src/commands/import.rs`, `src/core/migrate.rs::import_dir()`, and the `ingest_log` table are gone. The removal shipped in the same change as the user-facing docs / skills updates, preserving the intended sequencing requirement and leaving collections plus single-file ingest as the only supported ingest surfaces.

**Why this is correct, not a regression:**

- Quaid is **pre-release software**. There has never been a tagged public release with users in the wild. There are **zero users**. There are zero `memory.db` files outside this developer's machine.
- **On the `v0.9.x` strings in `README.md`:** these are developer-preview build labels for the author's own local testing, NOT advertised shipped releases. `v0.9.4` in the README is the author's phase-3-complete snapshot tag, not a product version with installed users. Repeated Codex rounds have mis-read these strings as evidence of a deployed user base; that inference is wrong, and this bullet is the canonical rebuttal. The `README.md` status label is being updated in this change to make the pre-release nature unambiguous to future review rounds.
- A "migration path" presumes data to preserve. There is no such data. A migrator would migrate from empty to empty, modulo the developer's own testing memory which they can recreate by re-ingesting their own notes.
- A compatibility shim would force us to maintain two ingest entry points (`import` AND `collection add`) with subtly different semantics — exactly the "two ways to do it" pathology this change exists to eliminate.
- A deprecation-with-warning cycle presumes a user base that warns would reach. There is no such base.

**Criteria that would re-open this decision:** (a) a tagged public release has shipped to install URLs referenced outside this repo AND (b) at least one external user has reported a non-empty `memory.db` they want to preserve AND (c) a user survey shows a migration need. **NONE of these conditions hold today or in any foreseeable window covered by this change.** A repository README with a "v0.9.x" status string is NOT evidence of condition (a) — a public release requires external users actually running that build against real brains, which does not exist. If all three become true in the future, it is the subject of a separate OpenSpec change, not this one.

**Alternatives already considered and rejected across multiple rounds:** a compat-shim that delegated to `collection add` (rejected round 9); a deprecation cycle with warnings (rejected round 9); an in-place v4→v5 upgrader (rejected round 9 and 11); an export/import mechanism for v4 DB-only state (rejected round 13 — there is no v4 DB-only state that matters because there are no v4 brains in the wild).


## Open Questions

- **Watcher mode auto-detection:** should we attempt native first and downgrade to poll on error, or require explicit `watcher_mode=poll` for known-flaky filesystems? *Current plan:* auto-detect with downgrade, log a warning when downgrading.
- **Debounce window default:** 1.5s chosen to coalesce Obsidian bulk saves (which fire events over ~500ms). Real-world tuning may move this. *Current plan:* 1.5s default, expose via `QUAID_WATCH_DEBOUNCE_MS`.
- **Embedding worker concurrency cap:** `min(cpus, 4)` is a guess. May need tuning against real BGE-small throughput. *Current plan:* expose via `QUAID_EMBEDDING_CONCURRENCY`.
- **`memory_collections` MCP tool shape — RESOLVED; adds tagged discriminator + restore advisory.** The tool returns an object per collection with the following frozen field set (all fields required in the response schema; nullable where noted): `name: string`; `root_path: string | null` (null when `state != 'active'` — even though storage is non-null, the API surfaces null so callers do not attempt to open a detached/restoring root); `state: "active" | "detached" | "restoring"`; `writable: boolean`; `is_write_target: boolean`; `page_count: integer`; `last_sync_at: string | null` (ISO-8601); `embedding_queue_depth: integer`; `ignore_parse_errors: Array<{ code: string, line: integer | null, raw: string | null, message: string }> | null` (task 13.6 surfaces line-level `"parse_error"` entries only, with `line`/`raw` populated; null when clean and also when the only pending ignore diagnostic is the deferred stable-absence refusal. The broader canonical tagged-union arm from task 3.7 — `"file_stably_absent_but_clear_not_confirmed"` with `line`/`raw` null — keeps the same object shape but is surfaced to `memory_collections` later in task 17.5aa5; until then it remains operator-visible via WARN log and `quaid collection info`); `needs_full_sync: boolean` (— set when a remap/restore has occurred and full-hash reconciliation is required); `recovery_in_progress: boolean` (— true while serve startup recovery is actively hashing; queued-but-not-yet-running recovery is encoded as `needs_full_sync: true AND recovery_in_progress: false` — there is NO separate `recovery_scheduled` field); `integrity_blocked: null | "manifest_tampering" | "manifest_incomplete_escalated" | "duplicate_uuid" | "unresolvable_trivial_content"` (— REPLACES the /83 boolean; null means the collection is NOT in a terminal blocking state, a string value names the specific blocking cause. `"manifest_tampering"` ← `integrity_failed_at IS NOT NULL` (operator runs `restore-reset`); `"manifest_incomplete_escalated"` ← `pending_manifest_incomplete_at` aged past `QUAID_MANIFEST_INCOMPLETE_ESCALATION_SECS` (operator runs `restore-reset`); `"duplicate_uuid"` ← `reconcile_halted_at IS NOT NULL AND reconcile_halt_reason = 'duplicate_uuid'` (operator strips duplicate `quaid_id` frontmatter manually, then runs `reconcile-reset`); `"unresolvable_trivial_content"` ← `reconcile_halted_at IS NOT NULL AND reconcile_halt_reason = 'unresolvable_trivial_content'` (operator runs `migrate-uuids` offline or `restore`, then `reconcile-reset`). Null for collections in normal operation OR in-flight recoverable states like plain pending-finalize / within-window manifest-incomplete. Agents observing a non-null `integrity_blocked` SHALL (i) stop retrying against this collection, (ii) surface the specific string value to the user, AND (iii) recommend the operator command mapped per-cause above — this closes the trust-boundary gap where a single boolean forced agents to shell out to `quaid collection info` to pick the right reset command. Backwards-compatible evaluation: agents checking truthiness (`if (integrity_blocked)`) still branch correctly; stricter schemas see the type widen from `boolean` to `null | string` and SHOULD update their handling. Precedence when multiple causes co-exist: `manifest_tampering` > `manifest_incomplete_escalated` > `duplicate_uuid` > `unresolvable_trivial_content` (named left-to-right so the most-severe identity-corruption signal wins); the CLI `info` command still surfaces the full per-cause column set so operators see every concurrent cause. The "boolean + shell out to info" contract is superseded by this discriminator; `restore_in_progress: boolean` (— true when `state='restoring'` AND the command has passed Phase 2 stability but has not yet cleared Tx-B; false otherwise. Agents SHALL surface `restore_in_progress=true` to the user as a "do not edit this vault" advisory: the old root is on a read-only mount for the duration of the destructive step so external writes fail loudly rather than succeed; the advisory signals that the vault is mid-capture and operators should wait for completion before interacting with the tree. The flag is NOT merely a restatement of `state='restoring'` — pre-Phase-2 restore windows set `state='restoring'` but the staging walk is not yet consuming user bytes, so the advisory fires specifically for the narrow "about to commit destructively" window. `recovery_in_progress` covers a different semantic (full-hash reconciliation progress after a completed remap/restore); `restore_in_progress` covers mid-flight restore specifically). This is the single authoritative schema for task 13.6; [tasks.md:231](tasks.md#L231) (task 13.6 definition) and all spec scenarios reference it rather than restating subsets. The later 17.5aa5 expansion preserves the field shape while widening the surfaced `ignore_parse_errors.code` set. The audit invariant in task 17.17 enforces that no other artifact lists a strict subset of these fields as the tool's complete response shape.
- **Ignore pattern precedence — RESOLVED (fail-closed absence added /27).** `.quaidignore` is the sole source of truth for user-authored patterns; `collections.ignore_patterns` is a cached mirror. Earlier drafts proposed last-writer-wins keyed by file mtime vs. DB `updated_at`; Codex flagged this as a confidentiality-boundary weak arbiter (clock skew, restore/remap, editor temp-file renames, DB/file divergence can revive stale patterns and re-index excluded files). New contract: **`.quaidignore` on disk is authoritative**; `collections.ignore_patterns` is derived from it on every successful atomic parse and SHALL NOT be consulted when the file is present. The sync is one-way and transactional: (a) the atomic-parse path (task 3.5/3.6) reads `.quaidignore`, validates every line; if fully valid, updates `collections.ignore_patterns` to exactly what the file said (cached mirror); if any line fails, leaves `collections.ignore_patterns` UNCHANGED (last-known-good), records errors in `ignore_parse_errors`, NO reconciliation runs. (b) The reconciler/walk reads patterns from `collections.ignore_patterns` (not from the file on every walk — the cached mirror is kept fresh by the watcher on every `.quaidignore` write). (c) CLI `ignore add|remove|clear --confirm` is **dry-run first, file-write second, mirror-refresh last**: the CLI computes the proposed `.quaidignore` contents in memory, runs the same atomic-parse validator against the proposed contents BEFORE writing anything, and refuses on validation failure with NO disk mutation and NO DB mutation. Only on a fully-valid dry-run does the CLI write `.quaidignore` to disk. The DB mirror (`collections.ignore_patterns`) is refreshed EXCLUSIVELY by `reload_patterns()` — invoked by the watcher's self-observed event when serve is running, or by the next `quaid serve` startup when serve is not. The CLI NEVER writes the mirror directly (enforced by code-audit test 17.5qq9). This sequence makes malformed CLI input impossible to persist and keeps file+mirror divergence impossible by construction. (d) **Absent-file behavior is three-way:** *no-prior-mirror* — `collections.ignore_patterns IS NULL` AND the file is absent → mirror stays NULL, reconciler applies defaults only (safe because no user patterns ever existed); *prior-mirror + opt-out UNSET* — mirror EXISTS and file is absent → `RefusedAbsenceUnconfirmed`, mirror UNCHANGED, `ignore_parse_errors = file_stably_absent_but_clear_not_confirmed`, WARN log, NO reconciliation (default behavior); *prior-mirror + `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1` OR explicit `quaid collection ignore clear <name> --confirm`* — mirror cleared, reconciliation runs. The fail-closed default exists because a transient delete / editor crash / sync-glitch is indistinguishable from user intent, and confidentiality patterns (`private/**`) are too sensitive to arbitrate by absence alone. (e) On `collection add` with a vault that lacks `.quaidignore`, no file is created (the user opts in by saving one). The collection operates with defaults-only until a file exists. There is NO mtime-based reconciliation — timestamps are not consulted for precedence because timestamps are unreliable across restore/remap, clock skew, and editor rename cycles. Last-writer-wins is formally rejected. This closes the confidentiality-boundary concern and the "absent-file auto-clear" regression surface.

## Execution order

Dependency graph of the 17 task sections:

```
§1 Schema v5
  └──> §2 Collection model ──> §3 Ignore patterns
                             └─> §4 File state + stat-diff
                                    ├──> §5 Reconciler ──> §5a UUID lifecycle
                                    │      └──> §6 Watcher pipeline ──> §7 Dedup set
                                    │                                └─> §8 Embedding queue
                                    └──> §9 `quaid collection` commands
                                           ├──> §10 `quaid init`
                                           ├──> §11 `quaid serve` integration
                                           │      └──> §12 `memory_put` write-through
                                           │             └──> §13 Slug parsing MCP/CLI
                                           │                    └──> §14 `quaid stats`
                                           └──> §15 Remove legacy ingest ──> §16 Docs
§17 Tests run against every preceding section.
§18 Follow-up stubs are independent.
```

Recommended PR slicing:

- **PR 1 — schema + collections foundation (§1, §2).** Lands v5 schema, `Collection` model, `parse_slug`, `op_kind`, fs-safety primitives. No behavior change visible to users until §9 lands.
- **PR 2 — stat/reconcile/watcher core (§4, §5, §5a, §6, §7).** Reconciler, UUID lifecycle, watcher pipeline, dedup set. Usable from tests; not yet wired into serve.
- **PR 3 — serve + writes (§8, §11, §12).** Embedding queue/worker, serve session/owner lease, RCRT, `memory_put` rename-before-commit. Live sync is functional.
- **PR 4 — CLI surface + ignore (§3, §9, §10, §13, §14).** Collection commands, ignore handling, slug routing across MCP/CLI, stats.
- **PR 5 — removal + docs + tests (§15, §16, §17).** Drop `quaid import`, update docs, land the full test matrix. §15 MUST ship in the same change as §16.

## Environment variables

| Name | Default | Purpose |
|------|---------|---------|
| `QUAID_WATCH_DEBOUNCE_MS` | `1500` | Debounce window coalescing filesystem events per collection. |
| `QUAID_FULL_HASH_AUDIT_DAYS` | `7` | Interval for the background full-hash audit that catches residual drift. |
| `QUAID_QUARANTINE_TTL_DAYS` | `30` | Auto-sweep TTL for quarantined pages with NO DB-only state. |
| `QUAID_RAW_IMPORTS_KEEP` | `10` | Per-page cap on retained inactive `raw_imports` rows. |
| `QUAID_RAW_IMPORTS_TTL_DAYS` | `90` | Age threshold for inactive `raw_imports` GC. |
| `QUAID_RAW_IMPORTS_KEEP_ALL` | unset | When `1`, disables inactive-row GC (forensic/full-history mode). |
| `QUAID_EMBEDDING_CONCURRENCY` | `min(cpus, 4)` | Bounded concurrency for the embedding worker. |
| `QUAID_RENAME_MIN_BYTES` | `64` | Minimum file size for content-hash rename inference. |
| `QUAID_RELOAD_HANDSHAKE_TIMEOUT_SECS` | `30` | Wall-clock timeout for the online restore/remap handshake. |
| `QUAID_DEFERRED_RETRY_SECS` | `30` | Max wait for RCRT to pick up an owned collection after a supervisor exits. |
| `QUAID_MANIFEST_INCOMPLETE_ESCALATION_SECS` | `1800` | Duration after which `pending_manifest_incomplete_at` escalates to `integrity_failed_at`. |
| `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE` | unset | When `1`, treating `.quaidignore` as absent clears the mirror instead of fail-closed. |
| `QUAID_RESTORE_STABILITY_MAX_ITERS` | `5` | Max Phase-2 stability retry iterations before `CollectionUnstableError`. |

Spec requirements and task bullets that reference any of these variables SHALL use the names exactly as listed here; the table is the single source of truth.

## Deferred hardening

Candidates the dev team MAY simplify at implementation time without changing the shipped user-visible behavior. Each is documented in specs today because it closes a real correctness gap, but none is user-observable in a single-user zero-user deployment. Simplification decisions belong in the implementing PR, not in this change:

- **`restore_command_start_time_unix_ns` + PID-start-time short-circuit.** Specs describe a same-host PID-liveness probe that uses `(pid, start_time)` to detect PID reuse and short-circuit the fresh-heartbeat defer gate. A correct simpler implementation drops `restore_command_start_time_unix_ns` and relies on the wall-clock heartbeat gate alone. Trade-off: after an originator crash on the same host whose PID is immediately reused by an unrelated process, recovery waits out the heartbeat TTL (~60s) instead of short-circuiting. Acceptable pre-release.
- **`FinalizeCaller::SupervisorRetry`.** Specs note this variant exists in the helper API "for test harnesses only." If tests can be written against `StartupRecovery`/`ExternalFinalize`/`RestoreOriginator`, omit the variant.
- **Four-way `integrity_blocked` discriminator.** Specs enumerate manifest-tampering, manifest-incomplete escalation, duplicate-UUID, and unresolvable-trivial-content as distinct blocking causes with a single `integrity_failed_at` column plus cause-specific diagnostics. A single `quaid collection repair` command covering every cause is sufficient; cause-specific CLI subcommands are not required.
- **Grep-based spec-consistency CI.** Prior drafts included a set of regex-based invariants (old task 17.17) enforcing prose conventions across the spec pack. Dropped during compaction. If drift returns, reintroduce as part of a dedicated consistency change.

`QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE` is NOT in this list — the prior-mirror fail-closed default is load-bearing confidentiality protection and stays in scope.

## Review history

Reviews on this change produced 86 rounds of adversarial pressure between initial proposal and final design. Load-bearing decisions (rename-before-commit sequencing, link-provenance quarantine predicate, two-phase restore with recoverable Tx-B, lease-based handshake, file-authoritative `.quaidignore`, opt-in UUID write-back, byte-exact restore invariant) are documented above and in the three capability specs. The prior round-by-round attribution in prose has been removed as scar tissue; the decisions themselves stand on their stated rationale.
