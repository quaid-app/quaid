# vault-sync Specification

## Purpose
TBD - created by archiving change vault-sync-engine. Update Purpose after archive.
## Requirements
### Requirement: File state tracking for stat-diff

The system SHALL maintain a `file_state` row for every indexed file with fields `(collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256, last_seen_at, last_full_hash_at)`. The primary key SHALL be `(collection_id, relative_path)` with additional indexes on `sha256` (for rename lookups) and `last_full_hash_at` (for audit scheduling). The `ctime_ns` and `inode` columns exist so reconciliation can detect content drift that preserves `(mtime, size)` (backup-restore, `git checkout`, rsync with `--times`, in-place truncate-and-rewrite of identical size, etc.) because on POSIX kernels update `ctime` on any inode modification and neither `ctime` nor `inode` can be set to an arbitrary past value by user-space tools.

#### Scenario: file_state upsert on ingest captures the full metadata set

- **WHEN** a file is ingested (via walk, watcher, or `memory_put`)
- **THEN** the corresponding `file_state` row is inserted or updated with current mtime_ns, ctime_ns, size_bytes, inode, and sha256
- **AND** `last_full_hash_at` is set to the current time (a full-hash was just performed)

#### Scenario: file_state removed on hard-delete

- **WHEN** a page is hard-deleted as part of watcher or reconciler processing
- **THEN** the associated `file_state` row is deleted in the same transaction

### Requirement: Cold-start reconciliation (with ctime/inode invalidation)

On `quaid serve` startup and on `quaid collection sync <name>`, the system SHALL walk each attached collection's root path and reconcile the filesystem against `file_state`. Files SHALL be skipped without reading content ONLY when ALL of the following match the stored `file_state` row: `mtime_ns`, `ctime_ns`, `size_bytes`, and `inode`. If ANY of these four fields differs, the file SHALL be re-hashed and compared to the stored `sha256`; only those whose `sha256` also differs SHALL be re-indexed.

The `ctime_ns` + `inode` signals close the common "preserved mtime/size" drift paths:

- **`git checkout` / `git stash pop`**: creates a new inode (different `inode` value), or overwrites in place updating `ctime`.
- **rsync / cp `--preserve=times`**: may preserve `mtime` but still touches `ctime` on the destination.
- **Backup-restore tools**: create new inodes on extract.
- **In-place truncate-and-rewrite of identical size**: updates `ctime`.

In every case, at least one of `ctime_ns` or `inode` will diverge from the stored value, triggering a re-hash. We accept that there remain narrow adversarial cases (e.g., explicit filesystem-level mtime+ctime spoofing, which requires privileges on most systems) as out-of-scope for the fast path; the periodic full-hash audit (separate requirement below) catches those.

#### Scenario: Unchanged vault is sub-second

- **WHEN** `quaid serve` starts against a 10,000-file vault where no files have changed since last run
- **AND** stat of every file yields matching `(mtime_ns, ctime_ns, size_bytes, inode)` against `file_state`
- **THEN** the reconciliation phase completes in well under one second on local SSD
- **AND** no files are re-hashed or re-indexed

#### Scenario: Modified file is re-indexed

- **WHEN** an existing file's bytes are changed (normal save — updates `mtime` and `ctime`)
- **THEN** the next reconciliation reads the file, computes sha256, detects a hash mismatch, updates `pages`, updates `file_state` (including new `ctime_ns`), and enqueues an embedding job

#### Scenario: mtime+size preserved but ctime or inode changed → still re-hashed

- **WHEN** an external tool (backup restore, rsync with `--times`, `git checkout`, in-place truncate-and-rewrite) modifies a file while preserving `(mtime, size)`
- **AND** the kernel nonetheless updates `ctime_ns` (for any content or metadata mutation) or the tool recreated the file yielding a different `inode`
- **THEN** reconciliation detects the `ctime_ns` or `inode` mismatch against `file_state`, re-hashes the file, and if sha256 differs, re-indexes
- **AND** the page content is brought current without waiting for some later edit to "unstick" stat-diff

#### Scenario: New file is ingested

- **WHEN** a new `.md` file appears under the vault root and matches no ignore pattern
- **THEN** the next reconciliation parses it, inserts a `pages` row, inserts a `file_state` row, and enqueues an embedding job

### Requirement: DB-only state predicate for quarantine classification

The system SHALL determine whether a page has "DB-only state" (state that cannot be reconstructed from page markdown on re-ingest) using the following explicit predicate, evaluated against schema fields that actually exist:

A page `p` has DB-only state **if any of the following are true**:

1. There EXISTS a row in `links` where (`from_page_id = p.id` OR `to_page_id = p.id`) AND `source_kind = 'programmatic'`. The `links.source_kind` column SHALL be added in the v5 schema with values `'wiki_link'` (extracted from markdown body via `extract_links()` at ingest — re-derivable) and `'programmatic'` (created via the `memory_link` MCP tool — DB-only). Ingest SHALL set `source_kind = 'wiki_link'` on every extracted link; `memory_link` SHALL set `source_kind = 'programmatic'`. The default value on the column is `'programmatic'` so any code path that forgets to set it errs on the side of preservation.
2. There EXISTS a row in `assertions` where `page_id = p.id` AND `asserted_by != 'import'`. The existing `asserted_by` column already has the needed values (`'agent'`, `'manual'`, `'enrichment'` = programmatic; `'import'` = re-derivable from content). `check_assertions()` at ingest time SHALL set `asserted_by = 'import'`; `memory_check` MCP tool writes SHALL use `'agent'` or `'manual'`.
3. There EXISTS a row in `raw_data` where `page_id = p.id` (any `raw_data` row is external-API-derived sidecar content that cannot be reconstructed from markdown).
4. There EXISTS a row in `contradictions` where `page_id = p.id` OR `other_page_id = p.id`. Contradictions are created by `memory_check` and by explicit operator actions; they record inconsistencies between pages and are NOT reconstructible from markdown. **Schema note:** v5 sets both `contradictions.page_id` AND `contradictions.other_page_id` to `ON DELETE CASCADE` (the v4 schema had `other_page_id ON DELETE SET NULL`, which would have left half-broken contradiction rows on page deletion — inconsistent with the quarantine-preservation intent). Under v5, the quarantine predicate protects any page referenced on EITHER side from the hard-delete path (the sweep's `has_db_only_state()` returns TRUE for both). CASCADE only fires on explicit `--purge` paths, where the user has stated full-removal intent. Unresolved contradictions (`resolved_at IS NULL`) and resolved ones are BOTH preserved — the history matters for audit even after resolution.
5. There EXISTS a row in `knowledge_gaps` where `page_id = p.id`. **Schema note:** `knowledge_gaps.page_id` is a new column added in v5 — `INTEGER NULL REFERENCES pages(id) ON DELETE CASCADE`, indexed via `idx_knowledge_gaps_page ON knowledge_gaps(page_id) WHERE page_id IS NOT NULL`. The column is nullable because slug-less gap logging (agents reporting general research gaps not tied to a specific page) remains supported; those rows do NOT affect `has_db_only_state()` because they do not reference a page. Slug-bound `memory_gap` invocations populate `page_id` with the resolved page; those rows are DB-authoritative audit history of what the agent asked and what wasn't answerable against THAT page, and cannot be reconstructed from page markdown. Added in (expanded in after Codex flagged the column was missing from the current schema). Unresolved gaps (`resolved_at IS NULL`) and resolved gaps are BOTH preserved. Because this is pre-release software with zero users (see the pre-release banner in proposal.md), there is no migration of existing v4 rows — `memory_gap` writes under v5 populate the column from the start.

Conversely, a page has ONLY vault-derivable state when NONE of the five predicates above hold — in particular, when all `links` involving the page are `source_kind = 'wiki_link'`, all `assertions` for the page are `asserted_by = 'import'`, no `raw_data` rows reference the page, no `contradictions` rows reference the page as either `page_id` or `other_page_id`, AND no `knowledge_gaps` rows reference the page.

**Schema-audit rule:** any future page-scoped table that is NOT reconstructible from markdown MUST be added to this predicate. Implementation guidance: when adding such a table, update `has_db_only_state()` in `src/core/quarantine.rs`, add a corresponding test under `17.5aa`, and add the table to the `quarantine export` JSON output so exported DB-only state survives a discard.

#### Scenario: Predicate correctly classifies a page with only wiki links

- **WHEN** a page is linked only by `wiki_link` rows in `links` (every `[[...]]` in its body and in bodies that reference it), has only `import`-authored assertions from `check_assertions`, and has no `raw_data`
- **THEN** the DB-only-state predicate returns FALSE; on file delete the page is hard-deleted

#### Scenario: Predicate correctly classifies a page with a programmatic link

- **WHEN** a user or agent invokes `memory_link("work::a", "work::b", relationship="depends_on")` creating a row in `links` with `source_kind = 'programmatic'`
- **THEN** both page `work::a` and page `work::b` now match the predicate (each has a programmatic link referencing it) — file deletion of either quarantines instead of hard-deletes

#### Scenario: Predicate correctly classifies a page with `raw_data`

- **WHEN** an enrichment pipeline has stored external API response in `raw_data` for a page
- **THEN** the predicate returns TRUE; file deletion quarantines rather than hard-deletes

#### Scenario: Predicate correctly classifies a page with a `contradictions` row

- **WHEN** `memory_check` has recorded a contradiction between page A and page B, creating a `contradictions` row with `page_id = A.id` and `other_page_id = B.id`
- **THEN** `has_db_only_state(A.id)` returns TRUE AND `has_db_only_state(B.id)` returns TRUE — each referenced page is protected from hard-delete
- **AND** deleting the file for either page quarantines instead of hard-deleting, preserving the contradiction record
- **AND** this holds regardless of whether `resolved_at` is NULL (unresolved) or non-NULL (resolved) — historical contradiction records are preserved across disk deletes

#### Scenario: Predicate correctly classifies a page with a `knowledge_gaps` row

- **WHEN** `memory_gap` has recorded a gap referencing page P (creating a `knowledge_gaps` row with `page_id = P.id`), and P has NO programmatic links, NO non-import assertions, NO `raw_data`, NO `contradictions`
- **THEN** `has_db_only_state(P.id)` returns TRUE because of the `knowledge_gaps` branch alone — the page is protected from hard-delete on file removal
- **AND** deleting the file for P quarantines instead of hard-deleting; the `knowledge_gaps` row survives the quarantine intact
- **AND** this holds regardless of whether `resolved_at` is NULL (unresolved gap) or non-NULL (resolved gap) — both encode audit history of what agents could not answer
- **AND** `quaid collection quarantine export` SHALL include `knowledge_gaps` rows referencing the page in the JSON output so a subsequent `discard` does not silently erase that history

### Requirement: Provenance maintenance on ingest and MCP writes

The system SHALL populate `links.source_kind` on every `INSERT INTO links` operation, and SHALL populate `assertions.asserted_by` consistent with the source. Code paths that insert links or assertions without setting these fields SHALL fail a CHECK constraint or unit test rather than falling back to ambiguous defaults.

#### Scenario: Ingest extracts wiki_link rows

- **WHEN** a page's markdown body contains `See [[people/alice]] for details` and the page is ingested
- **THEN** the resulting `links` row has `source_kind = 'wiki_link'`

#### Scenario: `memory_link` inserts programmatic rows

- **WHEN** an MCP client invokes `memory_link` to create a typed relationship
- **THEN** the resulting `links` row has `source_kind = 'programmatic'`

#### Scenario: `check_assertions` marks assertions as `import`

- **WHEN** `check_assertions()` extracts a heuristic assertion during ingest
- **THEN** the resulting `assertions` row has `asserted_by = 'import'`

#### Scenario: `memory_check` marks assertions as `agent` or `manual`

- **WHEN** an MCP client invokes `memory_check` to register a contradiction with supersession
- **THEN** the resulting `assertions` row has `asserted_by ∈ {'agent', 'manual'}` as appropriate

### Requirement: Deletion behavior driven by the DB-only predicate

The reconciler and watcher pipelines SHALL use the DB-only predicate (above) to decide between hard-delete and quarantine when a file is removed from disk.

#### Scenario: Deleted file with only vault-derivable state is hard-deleted

- **WHEN** an indexed file is removed from disk, no file with the same sha256 appears elsewhere in the vault during the debounce window, AND `has_db_only_state(page_id)` returns FALSE (ALL FIVE branches empty: all links `wiki_link`, all assertions `import`, no `raw_data`, no `contradictions`, no `knowledge_gaps`)
- **THEN** the next reconciliation deletes the corresponding `pages` row, `file_state` row, embeddings, and embedding jobs in a single transaction
- **AND** `wiki_link` rows in `links` and `import` rows in `assertions` cascade via `ON DELETE CASCADE` without data loss — they can be re-derived from any restored vault content

#### Scenario: Deleted file with DB-only state is quarantined, not hard-deleted

- **WHEN** an indexed file is removed from disk, no file with the same sha256 appears elsewhere in the vault, AND `has_db_only_state(page_id)` returns TRUE (at least one of the five branches is non-empty)
- **THEN** instead of hard-deleting, the reconciliation sets `pages.quarantined_at` to the current timestamp, deletes the `file_state` row (the file is gone from disk), but preserves the `pages` row and all five DB-only state categories (programmatic links, non-import assertions, `raw_data`, `contradictions`, `knowledge_gaps`)
- **AND** the quarantined page is excluded from default `memory_search`, `memory_list`, `memory_get`, and MCP results; it remains accessible via `quaid collection quarantine list` and explicit `include_quarantined` flags
- **AND** the decision is logged at INFO with the slug and the specific DB-only state categories that triggered quarantine (e.g., `programmatic_links=2 non_import_assertions=1 raw_data=0 contradictions=1 knowledge_gaps=3`)

#### Scenario: Non-active collection is skipped

- **WHEN** a collection's `state` is `'detached'` or `'restoring'` (i.e., `state != 'active'`) — regardless of whether the stored `root_path` still points at a readable directory
- **THEN** reconciliation is skipped for that collection and other collections proceed normally. The gate is `state != 'active'`, NOT `root_path IS NULL` — `collections.root_path` is `NOT NULL` by schema (task 1.1) and retains the last-known absolute path even while the collection is detached or restoring, so testing for NULL would be impossible and testing for "not accessible" would incorrectly allow reconciliation against a stale readable tree after a failed restore. The MCP `memory_collections` tool presents `root_path: null` when `state != 'active'` (presentation-layer masking per task 13.6), but that nullability is an API convenience, not a storage or gate signal. An additional safety check: if `state = 'active'` but `open(root_path, O_DIRECTORY | O_NOFOLLOW)` fails (disk unmounted, user deleted the directory out-of-band), serve logs `reconcile_skipped_root_unreadable collection=<N> root_path=<P>` at ERROR and transitions the collection to `detached` (a WriteAdmin-equivalent state change) before skipping; the next `quaid collection sync --remap-root` or user-triggered re-attach restores `active`.

### Requirement: Full-hash reconciliation on remap, fresh-attach, and first-use-after-detach

After any operation that could silently invalidate the mapping between `file_state` and on-disk bytes — specifically `quaid collection sync <name> --remap-root`, `quaid collection restore`, the first `quaid serve` session after a collection has been detached for any reason, and cross-machine `memory.db` moves — the system SHALL perform a UUID-first full-hash reconciliation pass that reads every file (ignoring `mtime`/`ctime`/`inode` stat-diff short-circuit) and reconciles identity and content using the priority defined in "Rename detection" (UUID match first, native event second, content-hash uniqueness fallback third, quarantine+create last). Stat-diff is an optimization for steady-state operation on the same filesystem; it is NOT sufficient after operations that can import fresh inodes or stat values from another system.

**Critical invariant — identity preserved across path changes.** Remap and fresh-attach DO NOT assume that on-disk `relative_path` values correspond to prior `file_state.relative_path` entries. Users who move a vault may reorganize directories; the reconciler SHALL resolve identity by `quaid_id` frontmatter UUID before considering `relative_path` at all. Specifically, the full-hash reconciler under remap/fresh-attach operates as follows:

1. **Walk the new root** collecting every `.md` file's `(on_disk_relative_path, quaid_id?, sha256)` tuple. `quaid_id` is read from frontmatter (may be absent for files added externally after memory's last view of them).
2. **Build a UUID index** from the new root's `quaid_id` values. For every `pages` row with a non-null `uuid`, look up the matching tuple in the walk results.
3. **UUID match → identity preserved, path updated in place.** If a `pages.uuid = X` has exactly one walk-result match, update that page's `file_state.relative_path` to the observed on-disk path (which may differ from the prior path). Re-hash the file; if content drifted, re-ingest the page's markdown-derived state (content/FTS/embeddings queue) but KEEP `pages.id` unchanged — programmatic links (`source_kind='programmatic'`), non-import assertions, `raw_data`, and `contradictions` survive intact.
4. **Native rename events** — N/A in the remap/fresh-attach context (we're running a one-shot reconciliation, not consuming a watcher stream).
5. **Content-hash uniqueness fallback** — for walk-results that have NO `quaid_id` AND no UUID match in step 3, AND for prior `file_state` entries whose UUID is absent or does not match any on-disk file: if exactly one orphan walk-result has sha256 = H AND exactly one orphan `file_state` entry has sha256 = H AND `size > 64 bytes` AND the **body after frontmatter is non-empty** (/82 canonical-resolver guard — files whose non-frontmatter content is empty or whitespace-only are treated as trivial and do NOT participate in content-hash fallback; this matches Phase 4's `UnresolvableTrivialContentError` behavior so the reconciler and the verifier agree), infer a rename — update `file_state.relative_path`, preserve `pages.id`, and ensure `pages.uuid` is populated. **Round-83 no-reconcile-write-back contract**: the reconciler SHALL NEVER enqueue a `quaid_id` frontmatter self-write during remap/fresh-attach/watcher reconciliation — identity is tracked via the DB-only `pages.uuid` column. Frontmatter mutation is reserved for the explicit opt-in surfaces in task 5a.5 (`quaid collection add --write-quaid-id`, `quaid collection migrate-uuids`, user-initiated `memory_put`). The pre-"already opted into write-back" conditional self-write branch is REMOVED from this step because it depended on a collection-level write-back mode the schema does not define; any rewrite-during-reconcile behavior reopens silent vault rewrites with git/sync churn.
6. **Unresolved on-disk files** → create new pages with fresh UUIDs stored in `pages.uuid`. **Round-83 no-reconcile-write-back contract applies here too**: NO frontmatter self-write is enqueued. The file's bytes stay byte-identical; identity is tracked via the DB `pages.uuid` column + content-hash fallback. Operators who want `quaid_id` persisted to frontmatter run `quaid collection migrate-uuids` as an explicit one-shot pass (task 9.2b) — never as a reconcile side-effect.
7. **Unresolved prior `file_state` entries** (page's UUID not found on disk, no content-hash match) → apply the DB-only-state predicate: pages with purely vault-derivable state are hard-deleted; pages with DB-only state are quarantined (per the "Deletion behavior driven by the DB-only predicate" requirement).

This procedure preserves identity across any directory reorganization the user performs during a vault move, which is the case Codex flagged: a user rearranging folders on a new machine would otherwise trigger bulk delete/create churn and lose programmatic links/assertions/raw_data/contradictions for every renamed file. Under this requirement those pages survive with `pages.id`, `links`, `assertions`, `raw_data`, and `contradictions` intact; only their `file_state.relative_path` moves to reflect the new layout.

**Performance note:** the UUID index is an in-memory HashMap for the duration of the reconciliation pass; lookups are O(1). For a 10k-file vault with all UUIDs present, UUID resolution is effectively free; the dominant cost is the full-file hash pass, which we already incur under stat-invalidation.

#### Scenario: Remap-root with directory reorganization preserves page identity

- **WHEN** a user runs `quaid collection sync work --remap-root /new/path` and the new layout has reorganized directories (e.g., `notes/` was renamed to `thoughts/`, `people/` was split into `people-work/` and `people-personal/`, a dozen files were moved deeper into subdirectories), but every markdown file still carries its original `quaid_id` in frontmatter
- **THEN** the reconciler walks `/new/path`, builds the UUID index, and for every `pages.uuid = X` matches it against the walk tuple carrying `quaid_id = X` regardless of the file's on-disk path
- **AND** every matched `pages.id` is preserved (no delete, no new page created); `file_state.relative_path` is updated in place to the observed new location
- **AND** every programmatic link, non-import assertion, `raw_data` row, and `contradictions` row survives; the collection's programmatic state is identical before and after the remap
- **AND** only files that had NO `quaid_id` or whose UUID cannot be matched in the new tree fall through to the content-hash fallback or the "create new page" path
- **AND** the log emits `remap_identity_preserved collection=work uuid_matched=N path_changed=M content_changed=K created=J hard_deleted=L quarantined=Q` for operator visibility

#### Scenario: Remap-root without directory reorganization is a no-op for identity

- **WHEN** `quaid collection sync work --remap-root /new/path` is run and the new layout is identical to the old (files at same relative paths, same bytes — just the root moved)
- **THEN** every `pages.uuid` matches a walk-result; every `file_state.relative_path` is already correct and is NOT rewritten (the reconciler notes "no change" and skips the UPDATE); content hashes all match stored `sha256`; no re-ingest
- **AND** the reconciler completes quickly (dominant cost is the full-hash pass)

#### Scenario: Fresh attach after cross-machine `memory.db` move — UUID-first reconciliation

- **WHEN** `memory.db` is moved to a new machine (possibly with directory reorganization) and a collection's `root_path` is set for the first time on the new machine via `sync --remap-root` or `collection restore`
- **THEN** the reconciler performs the UUID-first full-hash walk; pages with matching `quaid_id` in frontmatter preserve identity regardless of path; pages whose UUID is not found anywhere on disk are quarantined (if DB-only state) or hard-deleted (if purely vault-derivable), never silently resurrected as a fresh create-and-delete pair
- **AND** `last_full_hash_at` is updated for every matched file on completion

#### Scenario: First serve after detach — same UUID-first reconciliation

- **WHEN** a collection transitions from `detached` back to `active` (e.g., a missing mount reappears)
- **THEN** the reconciler runs the same UUID-first full-hash pass; drift during the detach window is resolved by UUID match when possible, content-hash uniqueness when UUID isn't present, and the standard quarantine/hard-delete rules otherwise
- **AND** no identity churn occurs for files that moved during the detach window as long as frontmatter UUIDs are intact

#### Scenario: Remap — file lacks `quaid_id`, identity preserved without write-back

- **WHEN** a file in the new layout has no `quaid_id` (e.g., created externally during the detach window) and the content-hash fallback infers a rename from an existing page
- **THEN** the reconciler preserves `pages.id`, updates `file_state.relative_path` to the new location, and ensures `pages.uuid` is populated; NO frontmatter self-write is scheduled by default (user bytes stay unchanged). To persist `quaid_id` to the file itself, the user may run `quaid collection migrate-uuids <name>` or attach with `--write-quaid-id`.

### Requirement: Periodic full-hash audit

The system SHALL run a background full-hash audit that rehashes every indexed file and reconciles any sha256 drift at a configurable interval. The default interval SHALL be 7 days (`QUAID_FULL_HASH_AUDIT_DAYS`). The audit SHALL be rate-limited so it does not saturate I/O on active systems (e.g., chunked across hours, or throttled via a configurable rate). Users SHALL be able to trigger an on-demand audit via `quaid collection audit <name>`.

#### Scenario: Scheduled audit catches mtime+size+ctime-preserving drift

- **WHEN** the background audit runs and encounters a file whose `(mtime_ns, ctime_ns, size_bytes, inode)` still matches `file_state` but whose actual bytes have silently changed (e.g., a rare adversarial filesystem-level operation, or a bug in an external tool that spoofs ctime)
- **THEN** the audit detects the sha256 mismatch, re-indexes the page, and updates `file_state`
- **AND** `last_full_hash_at` is updated to the current time

#### Scenario: Audit scheduling respects last_full_hash_at

- **WHEN** the audit runs and a file's `last_full_hash_at` is less than `QUAID_FULL_HASH_AUDIT_DAYS` old
- **THEN** the file is skipped in this audit cycle (it has been hashed recently enough)
- **AND** the audit spreads hashing work across multiple cycles so a small subset of the vault is hashed per day instead of the entire vault in one batch

#### Scenario: On-demand audit via CLI

- **WHEN** a user runs `quaid collection audit work`
- **THEN** the system performs a full-hash walk of the `work` collection immediately, reports `checked=N drifted=N fixed=N`, and updates `last_full_hash_at` on every file

### Requirement: Durable page identity via `quaid_id` frontmatter UUID (write-back is opt-in)

Every indexed page SHALL have a durable UUID stored as a column (`pages.uuid`). The UUID MAY ALSO be persisted to the page's frontmatter under the key `quaid_id` for cross-machine rename-stability, but persistence to the file is OPT-IN and NOT the default. This UUID is the primary identity signal for rename detection; content-hash and native-event heuristics cover the cases where no frontmatter UUID exists.

**Round-22 rationale — attach is NEVER a silent bulk rewrite.** Earlier drafts mandated that every ingested page whose frontmatter lacked `quaid_id` have the UUID written back to disk automatically on first ingest. Against a real vault (thousands of notes, git-backed, synced via iCloud/Dropbox, or partly read-only), that converted "attach a vault" into "rewrite every file," creating (a) git-dirty storms, (b) sync-tool conflicts, (c) unpredictable failures on permission-limited files, and (d) surprise for users who expected indexing to be read-only. Round-22 inverts the default:

- **Default attach is read-only.** `quaid collection add <name> <path>` and the reconciler's initial walk SHALL NOT write any `quaid_id` into frontmatter. Every page still gets a `pages.uuid` (generated server-side on first ingest and stored in the DB), and identity tracking works via the content-hash + UUID-column path for files that already have `quaid_id` in frontmatter.
- **Write-back is opt-in.** Users who want cross-machine rename-stability (the scenario where `memory.db` is not carried alongside the vault) enable write-back via either (a) the `--write-quaid-id` flag on `quaid collection add`, which runs write-back for every file during the initial walk, OR (b) the one-shot `quaid collection migrate-uuids <name>` command, which scans the collection and writes `quaid_id` into any file's frontmatter that lacks it. Both commands honor read-only files (log `uuid_write_back_skipped path=<P> reason=permission_denied` at WARN and continue), honor a `--dry-run` flag to preview the set of files that would be modified, and are treated as WriteAdmin per task 2.3 (they take the `CollectionRestoringError` interlock).
- **`memory_put` always writes `quaid_id`.** Any page written via `memory_put` SHALL include the `quaid_id` in its frontmatter at step 12's SQLite tx (which rotates `raw_imports` with the on-disk bytes that carry the UUID) — that's a write the user explicitly initiated, not a silent bulk mutation. The self-write dedup discipline (step 8 of the canonical 13-step rename-before-commit sequence) applies, and the recovery sentinel from step 5 protects durability on post-rename failures.
- **Watcher-observed external edits do NOT write back.** If the watcher observes a user-authored edit to a file without `quaid_id`, the reconciler ingests the new content and updates `pages.uuid` if needed, but does NOT inject `quaid_id` into the user's freshly-saved bytes. That would be a surprise mutation of what the user just typed.
- **`quaid collection add` classifies the root via a capability probe, but default attach supports read-only vaults.** Before inserting the `collections` row, the command attempts to create and unlink a tempfile at the root (e.g., via `openat(root_fd, ".quaid-probe-<uuid>", O_CREAT|O_EXCL|O_NOFOLLOW)` followed by `unlinkat`). The probe outcome is recorded in `collections.writable` (1 = writable, 0 = read-only). **Default attach SHALL succeed even when the probe fails with `EROFS` or `EACCES`** — the collection is attached as read-only with `writable=0`; `quaid collection info` surfaces the flag; the watcher and default reconciler run normally because they only read. Only when the user passed `--write-target` or `--write-quaid-id` (flags that explicitly promise to mutate the root) does a probe failure become a hard `RootNotWritableError` — those flags are incompatible with a read-only root by definition. `ENOSPC` on the probe is logged at WARN and recorded as `writable=1` with a hint to watch for free-space issues; disk-full is transient and does not disqualify the root. The `collection_owners` lease and all read-path behavior applies unchanged to read-only collections.

#### Scenario: Ingest reads existing UUID from frontmatter

- **WHEN** a markdown file is ingested and its frontmatter contains `quaid_id: 019da37c-7a18-7e4d-9c12-abcdef012345`
- **THEN** the resulting `pages.uuid` is set to that value
- **AND** no re-write of the file is performed (the UUID already exists)

#### Scenario: Default attach — file lacks `quaid_id`, NO rewrite

- **WHEN** `quaid collection add work /path` is invoked WITHOUT `--write-quaid-id` and a file in the vault has frontmatter lacking `quaid_id`
- **THEN** the system generates a server-side UUIDv7 and stores it in `pages.uuid`; rows in `links`, `assertions`, `contradictions`, `knowledge_gaps`, `raw_data` all reference that `pages.id` as normal
- **AND** NO write-back to disk is attempted; `file_state.sha256` reflects the original bytes the user had; `git status` is clean (if the vault is git-backed); sync tools see no file activity from the attach
- **AND** rename detection for files without `quaid_id` uses content-hash uniqueness (already specified as the fallback path) — cross-machine moves still work for uniquely-hashed files; very short notes or template-derived files may need the opt-in migration for full rename-stability
- **AND** the summary reports `indexed=N uuid_writeback=0 (use --write-quaid-id to persist UUIDs into frontmatter)` so the user sees the tradeoff

#### Scenario: Opt-in write-back during attach via `--write-quaid-id`

- **WHEN** `quaid collection add work /path --write-quaid-id` is invoked
- **THEN** during the initial walk, for every file whose frontmatter lacks `quaid_id`, the system generates a UUIDv7, stores it in `pages.uuid`, AND writes the file back via the canonical 13-step rename-before-commit self-write sequence (recovery sentinel at step 5, tempfile + atomic rename + fsync, dedup entry inserted at step 8, SQLite tx at step 12)
- **AND** write-backs honor file permissions: if a file is read-only (`EACCES`, `EROFS`), the UUID is still stored in `pages.uuid` but the file write is skipped with `uuid_write_back_skipped path=<P> reason=permission_denied` logged at WARN; attach completes with a summary noting the skipped count
- **AND** `--dry-run` prints the list of files that would be rewritten without performing any write
- **AND** all writes are serialized via the collection's per-slug mutex and interlock with the restore-state check
- **AND** the command honors the live-owner routing contract (`agent-writes/spec.md` — "CLI write routing when serve owns the collection", scope expansion): if the target collection has a live `collection_owners` lease, the attach command refuses BEFORE the initial walk's UUID write-back step with `ServeOwnsCollectionError`; the operator must stop serve (or detach online) before re-attaching with `--write-quaid-id`

#### Scenario: One-shot migration via `quaid collection migrate-uuids`

- **WHEN** `quaid collection migrate-uuids work` is invoked on a collection attached without `--write-quaid-id`
- **THEN** the command scans the collection, finds every file lacking `quaid_id` in frontmatter, and writes the UUID back using the same rename-before-commit self-write discipline as `--write-quaid-id`
- **AND** the command is classified as `WriteAdmin` per task 2.3 — it takes the `CollectionRestoringError` interlock from task 11.8 and fails fast if the collection is in `state = 'restoring'`
- **AND** the command honors the live-owner routing contract (`agent-writes/spec.md` — "CLI write routing when serve owns the collection", scope expansion): it refuses with `ServeOwnsCollectionError` BEFORE the scan begins if any live `collection_owners` lease exists for the target collection. `migrate-uuids` is Refuse-mode by default — no proxy fallback — because bulk frontmatter rewrites against a live watcher trade no correctness for amplified IPC and event-queue pressure. Operator contract: stop serve, run `migrate-uuids`, restart serve
- **AND** `--dry-run` previews; read-only files are skipped with WARN; the summary reports `migrated=N skipped_readonly=M already_had_uuid=K`

#### Scenario: Default attach on a read-only root succeeds with `writable=0`

- **WHEN** `quaid collection add ro-vault /some/readonly/path` is invoked WITHOUT `--write-target` and WITHOUT `--write-quaid-id`, and the root directory is read-only (mounted `ro`, locked by permissions, snapshot-mounted, container-restricted, etc.)
- **THEN** the capability probe (`openat` + `unlinkat` of a `.quaid-probe-<uuid>` file) fails with `EROFS` or `EACCES`
- **AND** attach SUCCEEDS: the `collections` row is inserted with `writable = 0`; the initial walk indexes every matching `.md` file (generating `pages.uuid` server-side); NO file bytes on disk are modified; NO `.quaid-probe-*` artifact is left behind
- **AND** `quaid collection info ro-vault` reports `writable=false` prominently; the watcher runs normally and reconciles external edits; `memory_put` / `quaid put` / `migrate-uuids` / CLI ignore mutations / `--write-quaid-id` operations refuse with `CollectionReadOnlyError`
- **AND** the user may later run `quaid collection sync ro-vault --recheck-writable` (e.g., after remounting read-write) to re-run the probe and flip the flag to `writable=1` without re-attaching

#### Scenario: `--write-target` or `--write-quaid-id` on a read-only root refuses attach

- **WHEN** `quaid collection add <name> <ro-path> --write-target` OR `quaid collection add <name> <ro-path> --write-quaid-id` is invoked and the capability probe fails with `EROFS` / `EACCES`
- **THEN** the command errors with `RootNotWritableError` naming the specific errno and the flag that cannot be honored
- **AND** NO `collections` row is inserted; NO walk is performed; the user is directed to either (a) fix permissions and retry, (b) attach without the flag to get a read-only collection, or (c) use `migrate-uuids` later once the root becomes writable

#### Scenario: UUID survives rename regardless of content

- **WHEN** a page with `pages.uuid = X` is renamed on disk (by any mechanism — `mv`, Finder, Obsidian, git checkout) while `quaid serve` is down
- **AND** the renamed file still contains `quaid_id: X` in frontmatter (because `render_page()` preserves it on every write)
- **THEN** the next reconciliation identifies the file by its UUID match against `pages.uuid = X` and updates `file_state.relative_path` to the new path
- **AND** `pages.id` is unchanged; all programmatic links, non-import assertions, `raw_data`, and `contradictions` remain intact

#### Scenario: UUID collision (accidental or adversarial) halts reconcile with `DuplicateUuidError`

- **WHEN** reconciliation finds two or more files with the same `quaid_id` in the same collection (e.g., user duplicated a file via copy-and-paste without clearing the UUID, or two branches were merged with overlapping frontmatter UUIDs)
- **THEN** the reconciler HALTS with `DuplicateUuidError` naming the uuid value AND all colliding paths; no auto-rewrite is performed; NO `pages.uuid` rebind is attempted; NO new UUID is minted for the duplicate; NO frontmatter self-write fires. The canonical identity resolver is fail-stop under duplicate-UUID conditions because auto-rewrite under ambiguous ownership can silently corrupt page identity and drop DB-only state (/82 correction — the pre).
- **AND** the operator resolves manually: inspect the colliding files, pick which one retains the original `quaid_id` (typically the page at its original `file_state.relative_path`), strip the `quaid_id` frontmatter line from every OTHER colliding file via an external text editor, then re-run the reconcile / `quaid collection sync`. On the next pass the retained file matches via UUID and the stripped files enter the canonical resolver's stage-(b) content-hash uniqueness branch (either matching a prior `raw_imports` entry OR creating a fresh `pages` row with a newly-minted UUID written back via `--write-quaid-id`/`migrate-uuids`).
- **AND** the `DuplicateUuidError` is emitted IDENTICALLY from Phase 4 (`/new/path` verification for `--remap-root`) and from post-attach `full_hash_reconcile` —.17(n) `resolver_unification`, both artifacts use the same canonical resolver and the same fail-stop branches. A `quaid collection info` output after the halt reports `reconcile_halted=DuplicateUuidError uuid=<X> paths=<comma-separated>` so the operator has full diagnostic context.
- **AND** the log is `reconcile_halted_duplicate_uuid collection=<name> uuid=<X> paths=<P1>,<P2>,...` at ERROR (replaces the pre).

### Requirement: Rename detection with UUID-first, then native events, then uniqueness-guarded hash

The system SHALL resolve rename detection in the following priority order:

1. **UUID match.** If a `new` file (disk-observed path with no `file_state` row) contains a `quaid_id` matching `pages.uuid` for a `missing` entry (path recorded in `file_state` but absent from disk), this is a rename regardless of content. Update `file_state.relative_path` in place; preserve `pages.id`, links, assertions, `raw_data`, and `contradictions`. This rule applies BEFORE content-hash inference and is immune to the "duplicate content" pathology.

2. **Native rename event.** When the filesystem backend emits a true rename event (FSEvents `Rename` with paired IDs, or inotify `IN_MOVED_FROM`/`IN_MOVED_TO` with matching cookies), the system honors it directly. This is typically fastest because it requires no content read.

3. **Content-hash inference** (fallback for edge cases, e.g., file moved before UUID was persisted to disk, or corrupted frontmatter). Applies only when UUID resolution fails AND no native rename event is available. The system infers a rename only when ALL of the following hold:
 - Exactly one `missing` entry in the current batch has sha256 = H.
 - Exactly one `new` entry in the current batch has sha256 = H.
 - The file size is strictly greater than a minimum threshold (default 64 bytes, configurable via `QUAID_RENAME_MIN_BYTES`), AND sha256 ≠ sha256 of empty content.

4. **Quarantine + fresh create.** If none of the above resolve the pair, quarantine the `missing` side (per the quarantine rules below) and create a fresh page for the `new` side with a newly-generated UUID. This path is now RARE — it only triggers when the file has no `quaid_id` yet (e.g., brand-new file) AND the content-hash rules don't apply.

#### Scenario: UUID match honored even for duplicate-content files

- **WHEN** the user renames `templates/meeting.md` (content: empty template with `quaid_id: X`) to `notes/2026-04-19-meeting.md`, and another file at `templates/archived/meeting.md` has identical bytes but a DIFFERENT `quaid_id`
- **THEN** reconciliation matches on `quaid_id = X` and updates `file_state.relative_path` for the existing page without regard to the identical-content file
- **AND** programmatic links, non-import assertions, `raw_data`, and `contradictions` associated with the page remain intact
- **AND** `pages.id` is unchanged

The decision — native-honored, inferred-pair, or quarantine+create — SHALL be logged at INFO level for each ambiguous case to support debugging.

#### Scenario: Native rename honored regardless of content

- **WHEN** the filesystem backend emits a native rename event from `notes/old.md` to `notes/new.md`
- **THEN** the system updates `file_state.relative_path` in place on the existing page
- **AND** the page's `pages.id` is unchanged (links and backlinks remain intact)
- **AND** no content hash comparison is performed

#### Scenario: Inferred rename during reconciler walk with unique hash

- **WHEN** the user moves `notes/old-name.md` to `notes/new-name.md` between serve restarts
- **AND** no other file in the vault has the same sha256
- **AND** the file size exceeds the minimum threshold
- **THEN** reconciliation infers the rename, updates `file_state.relative_path`, preserves `pages.id` and backlinks
- **AND** logs the inference at INFO

#### Scenario: UUID match wins over duplicate-content ambiguity

- **WHEN** the user deletes `templates/meeting.md` (which contained `quaid_id: X`) and creates `notes/2026-04-19-meeting.md` with the same content AND the same `quaid_id: X` in its frontmatter (because `render_page()` preserved the UUID across the move)
- **AND** another file at `templates/archived/meeting.md` also has the same sha256 (a duplicate copy) but a DIFFERENT `quaid_id`
- **THEN** the UUID-first rule matches `quaid_id = X` at the new path, identifies this as a rename, updates `file_state.relative_path` for the existing page
- **AND** `pages.id` is unchanged; all programmatic links, non-import assertions, `raw_data`, and `contradictions` remain intact
- **AND** the content-hash uniqueness rules are NOT consulted (UUID already resolved it)

#### Scenario: Rename with no UUID on disk — content-hash uniqueness required

- **WHEN** the user renames `notes/old.md` to `notes/new.md` while `quaid serve` is down, and the file has no `quaid_id` in frontmatter yet (brand-new file ingested but UUID write-back failed, or pre-existing file with corrupted frontmatter)
- **AND** no other file in the vault has the same sha256
- **AND** the file size exceeds the minimum threshold
- **THEN** the content-hash fallback succeeds: reconciliation infers the rename, updates `file_state.relative_path`, preserves `pages.id`
- **AND** at the same time, it writes the missing `quaid_id` to the file via self-write, so future renames will resolve via UUID match

#### Scenario: Rename fallback refused — no UUID AND non-unique hash

- **WHEN** neither UUID match nor native rename event resolves a pair, AND the content-hash uniqueness rules also fail (non-unique hash or trivial content)
- **THEN** the system quarantines the `missing` side (preserves `pages` row and DB-only state) and creates a new page for the `new` side with a newly-generated UUID
- **AND** the quarantine is logged at INFO; backlinks are NOT reassigned; user can resolve via `quaid collection quarantine restore` if the move was intentional
- **AND** this path is now RARE in practice because most files will have a `quaid_id` after their first ingest

#### Scenario: Watcher rename via paired Remove/Create applies the same priority order

- **WHEN** a rename occurs while the watcher is running on a backend that emits `Remove` then `Create` (rather than a native paired rename event)
- **THEN** the watcher holds the `Remove` in a pending-delete set during the debounce window
- **AND** at flush time applies the full priority order: UUID match first; then native-event (N/A here since we're in the Remove+Create path); then content-hash uniqueness; then quarantine+create
- **AND** because most renames preserve the `quaid_id` in frontmatter (the user didn't strip it), UUID match resolves the pair immediately

### Requirement: Quarantine state, TTL, and resolution

The system SHALL support a quarantine lifecycle for pages whose backing file has been removed but whose identity should be preserved pending reconciliation. A page in quarantine SHALL be excluded from default reads and search results and SHALL retain its `pages` row AND ALL rows in all FIVE DB-only-state categories: `links` (including wiki-links so re-ingest can cleanly rebuild them later), `assertions` (both import and non-import), `raw_data`, `contradictions`, and `knowledge_gaps`. The `pages.quarantined_at` TIMESTAMP column SHALL be NULL for active pages and non-NULL for quarantined pages.

#### Scenario: Quarantined pages hidden from default queries

- **WHEN** a page is quarantined and an agent calls `memory_search`, `memory_query`, `memory_list`, `memory_get` (with the quarantined slug), `memory_graph`, `memory_timeline`, or `memory_backlinks` without an explicit `include_quarantined` parameter
- **THEN** the quarantined page does NOT appear in results
- **AND** `memory_get` returns `NotFoundError` for the quarantined slug (not a stale page body)
- **AND** FTS queries join against `pages.quarantined_at IS NULL` so quarantined content does not surface via keyword search

#### Scenario: Explicit include_quarantined surfaces quarantined pages

- **WHEN** an agent calls `memory_search(query, include_quarantined=true)` or an equivalent flag on `memory_list` / `memory_get`
- **THEN** quarantined pages appear in results; each result includes a `quarantined: true` and `quarantined_at` field so the caller knows to handle it carefully

#### Scenario: Restoring a quarantined page via CLI (fd-relative, absent-target, default-refuses-collision)

- **WHEN** a user runs `quaid collection quarantine restore <collection>::<slug> <relative-path>`
- **THEN** the system reuses the fd-relative path-safety infrastructure from `memory_put` (the "Path-traversal and symlink-escape rejection via fd-relative path resolution" requirement in agent-writes): (1) parse-time rejection of `..`, absolute paths, empty segments, NUL bytes in `<relative-path>`; (2) `walk_to_parent(root_fd, relative_path, create_dirs=true)` → trusted `parent_fd`; any `ELOOP` on a component returns `SymlinkEscapeError`; (3) `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)` to check the target
- **AND** the quarantine restore DEFAULT-REFUSES collisions: if the target file already exists on disk (regular file or symlink), the command errors with `QuarantineRestoreTargetOccupiedError` naming the target path; no mutation occurs. This prevents the failure where a duplicate-content rename was quarantined as the "old" side and the "new" side is still living at its path — blindly writing to the new path would have overwritten a current note
- **AND** the DEFAULT-REFUSES behavior also covers the "already-indexed" case: if a `file_state` row exists for `(collection_id, relative_path)` pointing at a DIFFERENT page, the command errors with `QuarantineRestorePathOwnedError` naming the owning page's slug; no filesystem or DB mutation occurs
- **AND** when the target is absent AND no conflicting `file_state` row exists, the command writes via the rename-before-commit sequence used by `memory_put`: create tempfile with `O_CREAT | O_EXCL | O_NOFOLLOW` under `parent_fd`, write bytes (sourced from the page's active `raw_imports.raw_bytes`), `fsync`, defense-in-depth re-stat of the target name (symlink check), atomic `renameat` within `parent_fd`, `fsync(parent_fd)`, stat the renamed target for full `(mtime_ns, ctime_ns, size_bytes, inode)`, then a SINGLE SQLite tx that inserts the `file_state` row with the full stat tuple, clears `pages.quarantined_at`, enqueues an `embedding_jobs` row if embeddings were absent, and logs the restore at INFO
- **AND** an explicit `--force-overwrite` flag MAY be provided to override the collision refusal. When passed: (i) the command STILL runs the same fd-relative walk and symlink-at-target check (symlink targets are ALWAYS refused — `--force-overwrite` cannot punch through the symlink guard); (ii) if a conflicting `file_state` row exists for a DIFFERENT page, the command STILL refuses unless a prior `quaid collection quarantine export` was performed on the conflicting page OR the user ALSO passes `--force-export-conflict` (two-flag override makes the destructive action maximally explicit); (iii) the overwritten content is NOT preserved anywhere — `--force-overwrite` is a destructive last resort, audit-logged at WARN
- **AND** if the restore succeeds, a subsequent `quaid collection restore` (collection-wide) for this collection will materialize this page's bytes from `raw_imports` (the same rows that were used to source the restore); byte-exact fidelity holds

#### Scenario: Quarantine restore refuses occupied target (default)

- **WHEN** a user runs `quaid collection quarantine restore work::notes/old-meeting notes/old-meeting` and `<work_root>/notes/old-meeting.md` already exists on disk (possibly from a duplicate-content rename that triggered the quarantine)
- **THEN** the command errors with `QuarantineRestoreTargetOccupiedError` identifying the occupied target path and suggesting either a different `<relative-path>` or the `--force-overwrite` flag
- **AND** no filesystem or DB mutation occurs
- **AND** the quarantined page remains quarantined

#### Scenario: Quarantine restore refuses path already owned by another page

- **WHEN** a user runs `quaid collection quarantine restore work::notes/A <relative-path>` but `file_state` already has a row at `(work.id, <relative-path>)` bound to a different page (e.g., the user attempted to restore over a legitimate current note's slot)
- **THEN** the command errors with `QuarantineRestorePathOwnedError` identifying the owning page's slug
- **AND** no mutation occurs
- **AND** the user can choose a different path for restoration or use `--force-overwrite --force-export-conflict` after exporting the conflicting page

#### Scenario: Quarantine restore with symlink-at-target refuses even under `--force-overwrite`

- **WHEN** an attacker plants a symlink at the restore target path and the user runs `quaid collection quarantine restore <slug> <relative-path> --force-overwrite`
- **THEN** the defense-in-depth `fstatat(parent_fd, target_name, AT_SYMLINK_NOFOLLOW)` detects the symlink
- **AND** the command returns `SymlinkEscapeError` regardless of `--force-overwrite` — the symlink guard is never punched through
- **AND** the tempfile (if created) is `unlinkat`-cleaned; no DB mutation

#### Scenario: Quarantine restore honors the write-path crash semantics

- **WHEN** `quaid collection quarantine restore` completes the atomic rename but the SQLite commit fails (same failure mode as `memory_put` task 12.4)
- **THEN** the handler sets `collections.needs_full_sync = 1`, returns an error to the user; the vault holds the restored bytes; the recovery task re-ingests from disk within 1 second — the page ends up un-quarantined and indexed
- **AND** no ghost state: `raw_imports` bytes (still from the pre-restore active row) match what the recovery task writes based on the actual disk content

#### Scenario: Discarding a quarantined page via CLI

- **WHEN** a user runs `quaid collection quarantine discard <collection>::<slug>` with `--force` (or after a prior `export` for pages with DB-only state)
- **THEN** the system hard-deletes the `pages` row, cascading through `links`, `assertions`, `raw_data`, `contradictions`, `knowledge_gaps`, `embeddings`, `file_state` (already absent), and removes the quarantine entry — cascades rely on the v5 schema's `ON DELETE CASCADE` on every page-scoped FK per task 1.1

#### Scenario: Automatic quarantine sweep after TTL — DB-only predicate returns FALSE

- **WHEN** `quaid serve` starts up or a daily sweep task fires, a quarantined page's `quarantined_at` is older than `QUAID_QUARANTINE_TTL_DAYS` (default 30), AND the shared `has_db_only_state(page_id)` helper returns FALSE — meaning ALL FIVE branches are empty: NO programmatic `links` (`source_kind = 'programmatic'`), NO non-import `assertions` (`asserted_by != 'import'`), NO `raw_data` rows, NO `contradictions` rows (referenced as either `page_id` or `other_page_id`), AND NO `knowledge_gaps` rows (referenced via `knowledge_gaps.page_id`)
- **THEN** the system hard-deletes that page (everything attached is vault-derivable state that can be reconstructed from any restored vault content)
- **AND** the discard is logged at INFO with the slug, original quarantine timestamp, and TTL

#### Scenario: Automatic quarantine sweep SKIPS pages where DB-only predicate returns TRUE

- **WHEN** a quarantined page's `quarantined_at` is older than the TTL, BUT `has_db_only_state(page_id)` returns TRUE (at least one of the five branches is non-empty: programmatic link, non-import assertion, `raw_data` row, `contradictions` row referencing this page as either `page_id` or `other_page_id`, or `knowledge_gaps` row referencing this page)
- **THEN** the auto-sweep SHALL NOT hard-delete the page
- **AND** the page remains in quarantine indefinitely (preserving its DB-only state) until a user explicitly acts via `quaid collection quarantine discard` (which requires an explicit `--force` flag for pages with DB-only state) or `quaid collection quarantine export` (dumps DB-only state to a JSON file, after which a subsequent `discard` is allowed without `--force`)
- **AND** `quaid collection info <name>` surfaces the count of "quarantined pages with DB-only state awaiting user action" so backlogs are visible
- **AND** the sweep logs the skip at DEBUG on each cycle for auditability

#### Scenario: CLI discard of quarantined page with DB-only state requires `--force`

- **WHEN** a user runs `quaid collection quarantine discard <collection>::<slug>` and `has_db_only_state(page_id)` returns TRUE for the page (ANY of the five branches: programmatic `links`, non-import `assertions`, `raw_data`, `contradictions`, or `knowledge_gaps`)
- **THEN** the command errors with a message describing the DB-only state that will be lost (counts per branch — e.g. `programmatic_links=2 non_import_assertions=1 raw_data=0 contradictions=1 knowledge_gaps=3`), instructs the user to either pass `--force` to proceed anyway or run `quaid collection quarantine export` first
- **AND** no deletion occurs

#### Scenario: Quarantine export dumps DB-only state as JSON

- **WHEN** a user runs `quaid collection quarantine export <collection>::<slug> <output-path>`
- **THEN** the system writes a JSON file containing: `page_id`, `slug`, `collection`, `quarantined_at`, full page content (so the export is self-contained), every programmatic `links` row (source/target slugs resolved to strings), every programmatic `assertions` row with supersession chain, every `raw_data` row, every `contradictions` row where the page appears as `page_id` or `other_page_id` (including resolved ones, for historical audit), every `knowledge_gaps` row where `page_id = <page.id>` (including resolved gaps — the audit history matters), any `tags`, and any `timeline_entries` associated with the page. The export MUST include ALL FIVE DB-only-state branches; omitting `knowledge_gaps` would cause the post-export `discard` relaxation to silently erase gap history — rejected in.
- **AND** after successful export, the discard-force requirement is relaxed — `quaid collection quarantine discard <slug>` without `--force` succeeds if the user has exported the page since it entered quarantine (tracked via an `exported_at` column on `pages` or a separate `quarantine_exports` table)
- **AND** if export fails (I/O error, target-path error), the discard-force requirement is not relaxed

#### Scenario: Quarantine survives process restart

- **WHEN** a page is quarantined, `quaid serve` is killed, and restarted later (within the TTL)
- **THEN** the quarantine persists in `pages.quarantined_at`; the page remains hidden from default queries; the auto-sweep checks the TTL on every startup and on a daily timer

#### Scenario: File reappears during TTL window

- **WHEN** a quarantined page's sha256 matches a file newly appearing anywhere in the same collection during reconciliation (within the TTL)
- **THEN** the system MAY treat this as a latent rename and offer to restore — BUT SHALL NOT auto-restore unless the uniqueness rules (exactly one match on both sides, non-trivial content) are satisfied
- **AND** a log-only notice is emitted recommending `quaid collection quarantine restore <slug> <path>` if the match is ambiguous

### Requirement: `.quaidignore` watched as a control file — all-or-nothing parse with last-known-good preservation

The per-collection file watcher SHALL treat `<collection_root>/.quaidignore` as a control file distinct from indexable content. When the watcher observes any write, rename, or delete affecting `.quaidignore`, it SHALL:

1. Parse the new pattern set as an atomic unit: every non-empty, non-comment line SHALL be validated via `globset::Glob::new` BEFORE any effect is applied. If ANY line fails to parse, the reload SHALL be rejected wholesale.
2. **On fully-valid parse** — in a SQLite tx, update `collections.ignore_patterns` to the validated file contents (user patterns only — the file-authoritative contract; built-in defaults are NOT merged into the stored mirror, they are applied at reconciler-query time); clear `collections.ignore_parse_errors` (set to NULL). Commit. Then invoke an immediate reconciliation pass using the user-pattern-plus-defaults effective set computed in code.
3. **On any parse failure** — SHALL NOT modify `collections.ignore_patterns`; the LAST-KNOWN-GOOD effective pattern set remains in force. Record the failure in `collections.ignore_parse_errors` using the canonical tagged-union shape (, task 3.7): `[{"code": "parse_error", "line": N, "raw": "<line contents>", "message": "<globset error>"},...]`. Log at WARN. DO NOT trigger reconciliation. The previously-active exclusions remain active until the file is fixed.
4. During reconciliation (only runs on fully-valid reloads), any `file_state` row whose `relative_path` now matches an ignore pattern is treated as a "missing-from-indexable-set" case: the DB-only-state predicate runs on the corresponding page; pages with purely vault-derivable state are hard-deleted; pages with DB-only state are quarantined. Any on-disk file that is no longer ignored is ingested as `new`.
5. Surface parse errors prominently: log `ignore_reload_rejected collection=<name> error_count=<N> errors=[(line=<L>, msg=<M>),...]` at WARN; include the `parse_errors` JSON in `memory_collections` MCP output (`ignore_parse_errors` field); include a formatted rendering in `quaid collection info <name>` output. Fully-valid reloads log `ignore_reload_applied collection=<name> patterns=<N> hard_deleted=<N> quarantined=<N> re_ingested=<N>` at INFO.

The watcher SHALL NOT emit `pages` row inserts for a `.quaidignore` write (it is not an indexable page).

**Atomic all-or-nothing rationale:** a partial-parse fail-open — applying the valid subset and dropping the broken lines — is a confidentiality regression under normal editor behavior. An editor's intermediate save of `.quaidignore` (truncated write, mid-edit garbage, an accidental delete of a line) could drop a protective `private/**` pattern, trigger immediate reconciliation, and ingest previously-excluded files into FTS and embeddings before the user noticed the typo. The last-known-good stance closes that race: once a protective pattern has been applied, ONLY a fully-valid new file can deactivate it. The operator sees the WARN log + `memory_collections` error surface + `quaid collection info` rendering and knows to fix the file; during the fix-window, prior exclusions stay in effect.

**Empty-file and absent-file semantics:** an `.quaidignore` file that EXISTS on disk but contains only whitespace/comments parses cleanly to "zero user patterns" — that is a deliberate user edit (typically the `ignore clear --confirm` happy path) and triggers reconciliation to re-ingest formerly-excluded files. **Absent-file semantics are three-way** and differ from empty-file: (a) **fresh attach / no prior mirror** (`collections.ignore_patterns IS NULL`) — absent file parses as zero patterns, defaults-only applies, reconciliation walks normally (no last-known-good exists to preserve); (b) **prior mirror EXISTS + `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE` unset (default)** — absent-past-debounce is fail-closed: `RefusedAbsenceUnconfirmed`, mirror UNCHANGED, `ignore_parse_errors = file_stably_absent_but_clear_not_confirmed`, WARN log, NO reconciliation; (c) **prior mirror + `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1` OR explicit `quaid collection ignore clear <name> --confirm`** — mirror cleared, reconciliation re-ingests previously-excluded files. The fail-closed default exists because a transient delete / editor crash / sync-glitch is indistinguishable from user intent, and confidentiality patterns (`private/**`) are too sensitive to arbitrate by absence alone. To remove exclusions, users must either (i) save a fully-valid `.quaidignore` with the desired contents, (ii) save an empty `.quaidignore` (a valid parse of zero user patterns), or (iii) run `ignore clear --confirm`. Deleting the file is NOT a valid "clear" path under default settings.

**CLI path equivalence (file-only, dry-run first):** `quaid collection ignore add|remove|clear --confirm` SHALL follow a single authoritative sequence: (1) compute the proposed `.quaidignore` contents in memory from the current file + the requested transformation; (2) **atomic-parse dry-run** — validate every line via `globset::Glob::new` BEFORE writing any state; (3) on validation failure, exit non-zero with the parse error naming the offending line; NO file write, NO `collections.ignore_patterns` mutation, NO serve state touched; (4) on validation success, write ONLY `.quaidignore` to disk via tempfile+rename+fsync; `collections.ignore_patterns` is refreshed EXCLUSIVELY by `reload_patterns()` (invoked by the watcher's self-observed event OR by the next `quaid serve` startup). The CLI SHALL NOT write `collections.ignore_patterns` directly — that column is `reload_patterns`'s sole write surface (enforced by code-audit test 17.5qq9). This closes the two-way-sync divergence risk.

#### Scenario: Fully-valid `.quaidignore` write → apply + immediate reconcile

- **WHEN** the watcher for collection `work` receives a `Write` event for `<work_root>/.quaidignore` containing only valid glob patterns (e.g., adds `private/**`), and pages under `private/` are currently indexed
- **THEN** the atomic parse succeeds; `collections.ignore_patterns` is updated; `collections.ignore_parse_errors` is cleared (set to NULL); reconciliation runs
- **AND** pages under `private/` are hard-deleted or quarantined per the DB-only predicate
- **AND** subsequent `memory_search` calls do not return content from `private/` pages

#### Scenario: `.quaidignore` delete → fail-closed (last-known-good preserved by default)

- **WHEN** the user deletes `<collection_root>/.quaidignore` and no replacement appears within the debounce quiet-period (2s); `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE` is UNSET (default)
- **THEN** the watcher treats stable absence as fail-closed per the collections spec: `collections.ignore_patterns` is UNCHANGED (last-known-good exclusions, including any `private/**`-style patterns, remain in force); `collections.ignore_parse_errors` records `file_stably_absent_but_clear_not_confirmed`; NO reconciliation runs; previously-excluded files stay excluded
- **AND** a WARN log `ignore_file_absent_refused collection=<name>` is emitted, instructing the user how to confirm the clear
- **AND** to actually drop user patterns the user MUST run `quaid collection ignore clear <name> --confirm` (which writes an empty `.quaidignore` through the normal atomic-parse path) OR save a new `.quaidignore` with the desired contents
- **AND** this is the critical confidentiality fail-closed path: an `rm`, sync-tool delete, editor crash, or remote-filesystem hiccup cannot silently unignore previously-protected content

#### Scenario: Any invalid line rejects the whole reload (last-known-good preserved) — privacy fail-closed

- **WHEN** `.quaidignore` is written with a mix of valid and invalid lines — e.g., a user intended to keep `private/**` and `secret/**` but an editor saved an intermediate state where `secret/**` was corrupted into `**]` (unmatched bracket)
- **THEN** atomic parse detects the invalid line and REJECTS the entire reload: `collections.ignore_patterns` is UNCHANGED (the previously-applied `private/**` AND `secret/**` remain in force), `collections.ignore_parse_errors` is set to JSON recording the invalid line, a `ignore_reload_rejected collection=<name> error_count=1 errors=[(line=2, msg=...)]` WARN log is emitted
- **AND** NO reconciliation runs; no previously-excluded files are re-ingested; files under `secret/` stay excluded from search/MCP despite the current on-disk `.quaidignore` having a broken line for them
- **AND** `memory_collections` MCP responses and `quaid collection info <name>` surface the parse error so the user sees exactly which line to fix
- **AND** this is the critical confidentiality fail-closed path: an editor glitch cannot silently unignore previously-protected content

#### Scenario: Empty or whitespace-only `.quaidignore` — valid parse, removes user patterns intentionally

- **WHEN** the user deliberately empties `.quaidignore` (saves a file with zero non-comment non-whitespace lines)
- **THEN** the atomic parse succeeds with zero user patterns; `collections.ignore_patterns` is updated to the empty user-pattern set (stored as NULL or an empty pattern list per the file-authoritative contract — user patterns only, defaults applied in code at query time); `collections.ignore_parse_errors` is cleared
- **AND** reconciliation runs with the built-in-defaults-only set; files that were previously excluded by user patterns but do NOT match any built-in default are ingested as `new`
- **AND** this is distinct from the parse-failure path: the user explicitly removed all user exclusions (a valid state), and the system honors that intent

#### Scenario: Parse errors clear when a subsequent edit is fully valid

- **WHEN** a prior reload left `collections.ignore_parse_errors` populated (last-known-good patterns still active), and the user edits `.quaidignore` to a fully-valid set of patterns
- **THEN** the reload succeeds atomically; `collections.ignore_patterns` is updated to the new set; `collections.ignore_parse_errors` is set to NULL in the same tx
- **AND** reconciliation runs with the new valid patterns; `memory_collections` and `quaid collection info` reflect the cleared state

#### Scenario: `.quaidignore` is not indexed as a page

- **WHEN** the watcher processes events for `.quaidignore`
- **THEN** no `pages` row is inserted or updated for the file itself; only the ignore-pattern reload side effect occurs

#### Scenario: CLI `ignore add|remove` rejects invalid edits before persisting

- **WHEN** a user runs `quaid collection ignore add work "**]"` (a malformed glob) while `quaid serve` is running on `work`
- **THEN** the CLI validates the would-be resulting pattern set via the same atomic parse, detects the invalid pattern, returns a non-zero exit code with an error identifying the offending pattern
- **AND** NEITHER `collections.ignore_patterns` NOR `<work_root>/.quaidignore` is mutated; the serve process observes no change (no watcher event fires)

#### Scenario: CLI `ignore add` with a valid pattern is equivalent to an external edit — file-only writer contract

- **WHEN** a user runs `quaid collection ignore add work "archive/**"` (a valid glob) while `quaid serve` is running on `work`
- **THEN** the CLI writes ONLY `<work_root>/.quaidignore` (via fd-relative tempfile → fsync → rename → fsync parent); the CLI SHALL NOT write `collections.ignore_patterns` directly
- **AND** the watcher observes the file event within its debounce window, runs the atomic parse, and refreshes `collections.ignore_patterns` as the cached mirror — the DB update is a side-effect of the watcher path, not of the CLI
- **AND** the outcome is identical to the Obsidian-edit case: pages under `archive/` are hard-deleted or quarantined per the DB-only predicate within the debounce + reconciliation window
- **AND** code-audit test 17.5qq9 enforces that only `reload_patterns()` in `src/core/ignore.rs` writes `collections.ignore_patterns` — any other writer fails the build

### Requirement: `raw_imports` rotation on every content-changing write

Every content-changing write to a page SHALL rotate the page's `raw_imports` active row to reflect the current bytes. The rotation SHALL happen in the SAME SQLite transaction as the `pages` / `file_state` update. Rotation is required for:

- Initial ingest (first time a file appears in a collection — first active row inserted).
- Reconciler / watcher re-ingest after detecting changed bytes (sha256 drift).
- `memory_put` from an MCP client (the SINGLE SQLite tx of the rename-before-commit write sequence — task 12.1 step 12).
- UUID write-back self-write (the file content changed: `quaid_id` was inserted).

The rotation body SHALL be:

```sql
UPDATE raw_imports SET is_active = 0 WHERE page_id = ? AND is_active = 1;
INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path, created_at)
VALUES (?, ?, 1, ?, ?, ?);
```

After rotation, `COUNT(*) FROM raw_imports WHERE page_id = ? AND is_active = 1` equals 1.

Prior inactive rows SHALL be preserved for audit and historical export; they are never selected by restore.

#### Scenario: Reconciler re-ingest rotates raw_imports

- **WHEN** a file is edited externally and the reconciler detects a sha256 drift against `file_state`
- **THEN** in the same tx that updates `pages` and `file_state`, the prior active raw_imports row is marked inactive and a new row is inserted with the new bytes and `is_active = 1`
- **AND** a subsequent `quaid collection restore` writes the NEW bytes, not the pre-edit bytes

#### Scenario: `memory_put` single-tx rotates raw_imports (rename-before-commit)

- **WHEN** `memory_put` executes its first SQLite transaction (pages/file_state/embedding_jobs upsert)
- **THEN** the same tx performs the raw_imports rotation with the new tempfile bytes
- **AND** because rotation is part of the SINGLE SQLite tx that runs AFTER rename succeeds (rename-before-commit, task 12.1), a rename failure leaves raw_imports UNCHANGED (rotation never happened) and a post-rename DB-commit failure triggers `needs_full_sync` recovery, which re-ingests from disk and installs a fresh rotation matching the on-disk bytes — the restore view always equals the actual on-disk state

#### Scenario: Opt-in UUID self-write rotates raw_imports

- **WHEN** the opt-in UUID write-back path (invoked via `quaid collection add --write-quaid-id`, `quaid collection migrate-uuids`, or `memory_put`) rewrites a file with `quaid_id` inserted into frontmatter
- **THEN** raw_imports is rotated to the new bytes (which now include `quaid_id`) in the same tx as the self-write's DB update
- **AND** a subsequent restore writes a file that contains `quaid_id`, preserving rename-detection identity through disaster recovery
- **AND** the default read-only path does NOT enter this scenario — for pages ingested without the opt-in flag, raw_imports holds the user's original bytes and restore returns them verbatim

#### Scenario: Invariant — at most one active row per page

- **WHEN** any number of content-changing writes have occurred on a page
- **THEN** `SELECT COUNT(*) FROM raw_imports WHERE page_id = ? AND is_active = 1` equals 1
- **AND** the partial index `idx_raw_imports_active` enforces correct constant-time lookup for restore

### Requirement: Live file watcher per collection

The system SHALL run one file watcher per attached collection inside `quaid serve`. The watcher SHALL use a native backend (FSEvents on macOS, inotify on Linux) when available and SHALL fall back to polling when native setup fails or when `collections.watcher_mode = 'poll'`.

#### Scenario: Native watcher delivers events

- **WHEN** `quaid serve` is running with a native-mode collection and a user saves a `.md` file in the vault
- **THEN** within the configured debounce window (default 1.5s), the file is indexed and visible to FTS search

#### Scenario: Polling fallback on native failure

- **WHEN** native watcher setup returns an error during `quaid serve` startup
- **THEN** the system logs a warning and retries with `notify::PollWatcher`; the collection's effective `watcher_status` reports `polling`

#### Scenario: Ignore patterns respected

- **WHEN** a user saves a file at `.obsidian/workspace.md`
- **THEN** the watcher receives the event but the event is dropped during filtering; no indexing occurs

#### Scenario: Panic isolation

- **WHEN** the pipeline for one collection panics
- **THEN** only that collection's watcher task restarts (with exponential backoff); other collections continue serving unaffected

### Requirement: Overflow-triggered immediate full reconciliation (no silent event drop)

The watcher pipeline SHALL NOT use event-dropping as its steady-state backpressure strategy. When the per-collection event queue reaches its bounded capacity, the system SHALL (a) set `collections.needs_full_sync = 1` in a brief SQLite transaction for that collection, (b) log the overflow at WARN level with the collection name and queue size, (c) continue accepting subsequent events normally (the flag is a signal, not a pause), and (d) trigger an immediate full-hash reconciliation of that collection via a dedicated recovery task. Recovery starts within one second of the overflow, not on the next scheduled audit cycle, so the window of index staleness is bounded to the reconciliation duration (seconds for a typical vault) rather than days.

The `needs_full_sync` flag MAY additionally be set by: process-level panics that restart the watcher; detected channel disconnects; or `quaid` issuing an explicit mark via a debug CLI (`quaid collection mark-dirty <name>` for recovery of suspected state drift).

MCP callers SHALL be able to see this state via `memory_collections` (includes `needs_full_sync` and `recovery_in_progress` fields) and `quaid collection info <name>` so agents can reason about eventually-consistent reads.

#### Scenario: Burst overflows the watcher queue — flag set, immediate recovery scheduled

- **WHEN** the watcher channel for collection `work` reaches its capacity (e.g., bulk operation generates more events than the channel can hold)
- **THEN** the system sets `collections.needs_full_sync = 1` for `work` in a brief tx
- **AND** logs `watcher_overflow collection=work queue_len=<cap>` at WARN
- **AND** schedules an immediate full-hash reconciliation task for `work` that begins execution within one second
- **AND** continues processing incoming events normally (no pause, no further drops unless the queue hits capacity again while recovery is pending — in which case the flag stays set)

#### Scenario: Full reconciliation completes — flag cleared, state eventually consistent

- **WHEN** the recovery task completes for a dirty collection
- **THEN** `collections.needs_full_sync` is cleared in a tx on successful completion
- **AND** the reconciliation stats are logged: `recovery_complete collection=work walked=N modified=N new=N missing=N`
- **AND** subsequent reads see fully-reconciled state

#### Scenario: Agents can observe eventually-consistent state via MCP

- **WHEN** an agent queries `memory_collections` while a collection has `needs_full_sync = 1`
- **THEN** the response is represented entirely within the frozen 13-field schema (/83/84 — task 13.6 / [design.md §Open Questions `memory_collections` MCP tool shape](../../design.md)): `needs_full_sync: true` plus `recovery_in_progress: true` when a recovery worker is actively hashing, OR `recovery_in_progress: false` when recovery is queued but not yet running. `integrity_blocked` is additionally exposed — `null` in this in-progress-recovery scenario (not a terminal block), letting agents distinguish normal recovery from operator-reset-required states. A non-null string value (`"manifest_tampering" | "manifest_incomplete_escalated" | "duplicate_uuid" | "unresolvable_trivial_content"`) names the specific terminal cause and maps to the correct reset command per `collections/spec.md`. `restore_in_progress: false` in this scenario because it is reserved for the narrow mid-restore destructive window, not generic recovery. The queued-but-not-running recovery case is encoded by `needs_full_sync: true AND recovery_in_progress: false` — there is NO separate `recovery_scheduled` field (prior drafts invented one; fix normalizes on the 2-boolean encoding). Agents distinguish "idle" from "queued" from "running" via the pair `(needs_full_sync, recovery_in_progress)`: `(false, false)` idle; `(true, false)` dirty and queued; `(true, true)` dirty and recovery worker actively running; `(false, true)` is not a reachable state (the worker clears `needs_full_sync` only on clean completion so a transient `(false, true)` would only appear in the same tx as the flag-clear)
- **AND** `memory_stats` includes a summary line like "1 collection awaiting recovery" (counts rows where `needs_full_sync = 1`, regardless of `recovery_in_progress`)
- **AND** reads against the collection still succeed (no blocking) but return whatever state currently exists; the flag pair is advisory so agents can defer high-stakes decisions until recovery completes

#### Scenario: Recovery runs independently of the periodic audit

- **WHEN** a collection is flagged `needs_full_sync` and the periodic audit schedule has most files "recently-audited" (within `QUAID_FULL_HASH_AUDIT_DAYS`)
- **THEN** the recovery task still runs a FULL hash walk of the collection, ignoring per-file `last_full_hash_at` (this is a dirty-flag recovery, not a scheduled audit); `last_full_hash_at` is refreshed on completion for every file
- **AND** the periodic audit's timing is unaffected

### Requirement: Debounced batch processing

The watcher SHALL coalesce events per path within a configurable debounce window (default 1500ms, configurable via `QUAID_WATCH_DEBOUNCE_MS`). Multiple events on the same file within the window SHALL collapse to a single effective state. When events stop arriving for the debounce duration, the batch SHALL be flushed through the indexing pipeline in a single transaction.

#### Scenario: Bulk save coalesces

- **WHEN** a user saves 50 files in rapid succession (e.g., Obsidian bulk-rename)
- **THEN** the watcher collects all events into a single debounced batch
- **AND** the indexing transaction processes them together
- **AND** FTS rows for all 50 files are queryable after the batch commits

#### Scenario: Rapid edits coalesce to latest

- **WHEN** a user saves the same file three times within 500ms
- **THEN** only the final state is indexed
- **AND** exactly one embedding job is enqueued for the page

### Requirement: Two-tier indexing

File content changes SHALL be split into a synchronous tier (FTS + metadata updates) and a deferred tier (embedding generation). The synchronous tier SHALL commit in the same SQLite transaction as the `file_state` upsert. The deferred tier SHALL enqueue an `embedding_jobs` row drained by a background worker with bounded concurrency.

#### Scenario: FTS fresh immediately

- **WHEN** a file is saved and the debounce window elapses
- **THEN** `memory_search` via FTS returns the page in results using keywords from its content

#### Scenario: Embeddings catch up asynchronously

- **WHEN** a file is saved and indexed at Tier 1
- **THEN** an `embedding_jobs` row is inserted with state `pending`
- **AND** the embedding worker subsequently processes the job, writes to `page_embeddings_vec_*` and `page_embeddings`, and deletes the job row

#### Scenario: Hybrid search always finds the page via FTS lane

- **WHEN** a file is saved and its embedding has not yet been generated
- **THEN** `memory_query` using hybrid search returns the page because the FTS lane is fresh, even though the vector lane does not yet have its embedding

### Requirement: Embedding queue crash recovery

The `embedding_jobs` queue SHALL persist across restarts. On `quaid serve` startup, rows in state `processing` SHALL be reset to `pending` with `attempts` incremented. Rows SHALL transition to `failed` with an error message after 3 failed attempts.

#### Scenario: Worker restart resumes queue

- **WHEN** `quaid serve` is killed while the embedding worker has 10 jobs in flight and 50 pending
- **THEN** on the next startup, the 10 in-flight jobs are reset to pending and processed alongside the 50 pending

#### Scenario: Persistent failure reaches terminal state

- **WHEN** a specific page's embedding fails 3 times in a row
- **THEN** the job's state becomes `failed` with an error message; the worker moves on; `quaid collection info` surfaces a failed-job count

### Requirement: Live-serve coordination for restore/remap (serve session ownership + rebind)

`quaid collection restore` and `quaid collection sync --remap-root` change `collections.root_path` and therefore invalidate the trusted `root_fd` and watcher state held by any running `quaid serve` process for that collection. The system SHALL implement explicit coordination between these commands and a live serve process so a restore/remap NEVER leaves serve pinned to a stale root. Two mechanisms cover the two valid usage patterns:

**1. Serve ownership — single-owner per-collection lease.** Because AGENTS.md mandates `Single writer`, the serve-ownership model SHALL enforce at most ONE live owner per collection via a transactional lease rather than an unconstrained sessions table. Schema: a `serve_sessions` table with columns `(session_id TEXT PRIMARY KEY, pid INTEGER NOT NULL, host TEXT NOT NULL, started_at TEXT NOT NULL, heartbeat_at TEXT NOT NULL)` — tracks heartbeat liveness — PLUS a `collection_owners` table `(collection_id INTEGER PRIMARY KEY REFERENCES collections(id) ON DELETE CASCADE, session_id TEXT NOT NULL REFERENCES serve_sessions(session_id) ON DELETE CASCADE, acquired_at TEXT NOT NULL)` that makes ownership exclusive per collection. The `PRIMARY KEY` on `collection_id` enforces one owner at a time.

At startup `quaid serve` SHALL run ONE SQLite tx per collection that attempts an exclusive claim: (a) sweep stale `serve_sessions` rows (`heartbeat_at < now() - 15s`) AND cascade-delete their `collection_owners` rows; (b) INSERT its own `serve_sessions` row with a fresh UUID `session_id`; (c) attempt `INSERT INTO collection_owners (collection_id, session_id, acquired_at) VALUES (?, ?, now())`. If the INSERT fails with a PK conflict, the collection already has a live owner — serve SHALL refuse to attach the collection (log `serve_refused_collection_owned collection=<N> owner_session=<S> owner_pid=<P>` at ERROR, continue with other collections that ARE free, exit non-zero if NO collections could be claimed). This prevents two serve processes from watching the same collection simultaneously. Heartbeat refresh every 5 seconds (configurable via `QUAID_SERVE_HEARTBEAT_SECS`); a session is "live" when `heartbeat_at > now() - 15s` (three heartbeat intervals). At shutdown (SIGTERM, SIGINT, or clean exit) serve DELETEs its `serve_sessions` row which cascades to drop its `collection_owners` rows. On crash the row ages past the liveness threshold within 15s and the next `quaid serve` sweep reclaims it.

**1a. Command coordination with the owner lease.** Restore/remap/purge commands SHALL capture `expected_session_id` from `collection_owners` for the target collection (NOT from arbitrary `serve_sessions` rows). Because `collection_owners.collection_id` is a primary key, there is exactly ONE owning session per collection at any time — no "which serve to coordinate with" ambiguity. If `collection_owners` has no row for the collection, there is no live owner; offline mode applies. The handshake helper (task 9.7a) re-reads `collection_owners` on every poll (in addition to `serve_sessions.heartbeat_at`) — if the owning session changes mid-handshake (e.g., the original owner crashed and a fresh serve claimed the collection), the command aborts with `ServeOwnershipChangedError` and runs the abort-path resume-generation bump.

**1b. Startup contention handling.** A second `quaid serve` that starts while another is live observes a PK conflict on `collection_owners` and refuses the claim per (1). The second serve logs the collision and exits non-zero if it cannot claim any collection; the user is directed to stop the running serve via SIGTERM (`kill <pid>` or `pkill -TERM quaid` per task 9.7e — `quaid serve --stop` is NOT an implemented subcommand). This is the single-writer enforcement — multiple serve processes cannot silently split-memory the same collection.

**2. Command behavior (lease-based ownership resolution).** `quaid collection restore`, `quaid collection sync --remap-root`, AND `quaid collection remove --purge` SHALL, before mutating `collections.root_path`, `collections.state`, or cascading deletes:

- Read `collection_owners` for the target collection to resolve the owning session (NOT `serve_sessions` in the aggregate — the PK on `collection_owners.collection_id` guarantees exactly one owner, so there is no ambiguity to resolve).
- If `collection_owners` has NO row for the collection, there is no owner; proceed immediately (offline mode).
- If `collection_owners` has a row AND the referenced `serve_sessions.heartbeat_at` has aged past the 15s liveness threshold, the owner is stale. The command SHALL NOT silently adopt the stale lease; instead, it SHALL run the sweep (DELETE stale `serve_sessions` row → CASCADE drops the `collection_owners` row) in a tx, re-read `collection_owners` (now empty), and proceed as offline.
- If `collection_owners` has a row AND its referenced session is live (`heartbeat_at > now() - 15s`), the command SHALL select between two explicit behaviors:
 - **Default (no flag): refuse.** Return `ServeOwnsCollectionError` with a message naming the owning session's `pid` and `host` (joined from `serve_sessions`), instructing the user to stop serve via SIGTERM (`kill <pid>` or `pkill -TERM quaid`) and retry. `quaid serve --stop` is NOT an implemented subcommand per task 9.7e; all operator guidance uses SIGTERM consistently. No mutation occurs.
 - **`--online` flag: coordinate.** Perform the online rebinding handshake described next.

All three commands capture `expected_session_id = collection_owners.session_id` (the single owner) for use as the handshake key in (a)–(d) below. No path reasons about "any live session" in `serve_sessions` — every ownership check goes through `collection_owners`.

**3. Online rebinding handshake — lease-based ack bound to `(session_id, reload_generation)`.** When `--online` is passed and a live session exists, the handshake SHALL use a release acknowledgement that is bound to the exact serve session and the exact generation bump the command is waiting for. A bare timestamp is NOT sufficient: a delayed write from an earlier timed-out handshake, or from a serve instance that is racing shutdown/restart, could satisfy a later command even when the current owner has not released. The ack SHALL name both the releaser and the request it is releasing.

Schema additions on `collections` (beyond `reload_generation` and `watcher_released_at`):

- `watcher_released_session_id TEXT NULL` — session_id of the serve that wrote the ack (must equal the session_id the command captured).
- `watcher_released_generation INTEGER NULL` — the generation value that was current when the ack was written (must equal the generation the command bumped to).

A handshake completes if and only if **all three** of these fields match the command's captured expectation:

- `watcher_released_session_id = <expected_session_id>`
- `watcher_released_generation = <cmd_reload_generation>`
- `watcher_released_at IS NOT NULL` (as a "has been set" signal; the timestamp itself is informational only, never a liveness test)

Matching on any two of the three is insufficient; a stale ack from a prior generation of the same session, or from a different session that happens to race, is rejected by construction.

- (a) **Command captures expectation.** Before any state mutation, command reads `collection_owners` for the target collection and captures `expected_session_id = collection_owners.session_id`; it then verifies the referenced `serve_sessions` row is live (`heartbeat_at > now() - 15s`). If `collection_owners` has no row OR the referenced session is stale (sweep runs per §2), the command takes the offline path (no handshake needed). The captured value is the ONLY session whose ack will be accepted for this handshake instance, and it is resolved from the single-owner lease — never from the aggregate `serve_sessions` table.
- (b) **Command opens handshake.** In a single tx, command atomically: sets `collections.state = 'restoring'`; computes `cmd_reload_generation = collections.reload_generation + 1` and writes it back; clears `watcher_released_session_id`, `watcher_released_generation`, and `watcher_released_at` (NULL, NULL, NULL); sets `pending_command_heartbeat_at = now()` (— command liveness marker for startup auto-recovery gating). The `reload_generation INTEGER NOT NULL DEFAULT 0` column exists so serve can detect state changes via a lightweight poll without inter-process signalling. Clearing the ack triple ensures any leftover ack from a prior handshake is wiped before the new one starts. **Command heartbeat:** while waiting in step (d), the command SHALL refresh `pending_command_heartbeat_at = now()` every 5s (configurable via `QUAID_COMMAND_HEARTBEAT_SECS`) via a brief fresh-connection tx. On successful completion OR abort, the command SHALL NULL this field as part of the final state transition.
- (c) **Serve releases.** Serve's per-collection supervisor task polls `collections.state` and `collections.reload_generation` every 250ms (configurable via `QUAID_RELOAD_POLL_MS`). On observing `state = 'restoring'` with a strictly greater `reload_generation` than the last-observed value, serve SHALL: stop the watcher, flush any in-flight debounce batch, drain outstanding embedding jobs for that collection (best-effort within 5s), close `root_fd`, release the per-collection dedup set and per-slug mutex map, AND THEN in a single tx write the full ack triple: `watcher_released_session_id = <own session_id>`, `watcher_released_generation = <observed reload_generation at release time>`, `watcher_released_at = now()`. The session_id written is the serve's OWN session_id — serve never writes an ack on behalf of a different session, so foreign-session acks are impossible by construction. **Round-54/55: after writing the ack, the per-collection supervisor EXITS cleanly** — it removes its own entry from the process-global `supervisor_handles: DashMap<CollectionId, JoinHandle>` registry and returns. The collection is now owned by RCRT (task 9.7d) for the entire `restoring` phase. The supervisor does NOT continue polling for `state = 'active'` — that observation and reattach is RCRT's responsibility under the single-flight mutex.
- (d) **Command waits on the lease.** Command polls every 100ms (timeout default 30s via `QUAID_RELOAD_HANDSHAKE_TIMEOUT_SECS`) for the exact match: `watcher_released_session_id = <expected_session_id> AND watcher_released_generation = <cmd_reload_generation> AND watcher_released_at IS NOT NULL`. An ack with either a different `session_id` (foreign serve) or a different `generation` (stale from an earlier handshake) is ignored — polling continues until a matching ack lands or the timeout expires. Concurrently, on every poll, command also re-reads `serve_sessions` for `expected_session_id`: if that row has disappeared or its `heartbeat_at` has aged past the 15s liveness threshold, command aborts early with `ServeDiedDuringHandshakeError` (no point waiting for an ack from a dead session). On plain timeout, command aborts with `ServeHandshakeTimeoutError`. **Abort-path resume semantics (/54/55 fix — prevents serve from being stranded with its watcher torn down).** In EVERY abort case the command SHALL, in a SINGLE durable SQLite tx via a FRESH connection: (i) revert `state` to its prior value (`active` if the prior `root_path` is readable, `detached` otherwise); (ii) keep `root_path` unchanged (no finalize ran); (iii) clear the ack triple; (iv) **bump `reload_generation` ONE MORE TIME to `cmd_reload_generation + 1`**. The generation bump on abort is mandatory — without it, RCRT's sweep (which detects "owned collection without a live supervisor handle in the registry") would see `state='active'` at the SAME generation as when the supervisor released, and a future audit or test cannot distinguish "never had a supervisor in the first place" from "lost its supervisor during a handshake". Bumping again forces a clean ordering signal that a resume is expected. RCRT's next sweep observes the owned `state='active'` collection with NO live supervisor handle → acquires the per-collection single-flight mutex → opens fresh `root_fd` against the unchanged prior `root_path` → starts a new watcher → runs `full_hash_reconcile` → spawns a new per-collection supervisor task → registers its handle. The command logs `restore_abort_resumed collection=<N> prior_generation=<cmd_reload_generation> resume_generation=<cmd_reload_generation+1>` at WARN before returning. Additionally clean up any staging directory and return non-zero. Serve is NOT required to restart — RCRT's periodic sweep (default `QUAID_DEFERRED_RETRY_SECS=30s`) recovers the collection automatically. The error message directs the user to SIGTERM the serve process (`kill <pid>` or `pkill -TERM quaid`) ONLY if they want to force an immediate resume instead of waiting for the next RCRT sweep. `quaid serve --stop` is NOT an implemented subcommand per task 9.7e.
- (e) **Command proceeds once lease is granted — mode-specific post-release flow.** After the lease matches, the command's behavior DIVERGES by operation mode:
 - **Restore mode** (`quaid collection restore`): runs the normal restore flow (staging, verify, atomic rename, Tx-A + Tx-B via `finalize_pending_restore(..., FinalizeCaller::RestoreOriginator)` — per task 9.7 (i–k)). `run_tx_b` flips `state = 'active'`, SETS `needs_full_sync = 1`, bumps `reload_generation`, clears the ack triple, and NULLs all pending/integrity/command columns. Writes become legal only AFTER RCRT's attach-completion tx clears `needs_full_sync = 0` — between Tx-B commit and RCRT attach the write-gate (§4) refuses all mutating tools because Tx-B's `DELETE FROM file_state` leaves the restored tree with NO matching `file_state` rows, so `memory_put`'s canonical external-create precondition (`stat` succeeds + no `file_state` row → `ExternalCreate` → `ConflictError`) would misclassify every page. Reads remain available throughout. Expected write-block latency: up to `QUAID_DEFERRED_RETRY_SECS` (default 30s).
 - **Remap mode** (`quaid collection sync --remap-root`): runs the non-destructive re-attach flow (see "Remap via `sync --remap-root`" scenario). The command's DB-update tx leaves `state = 'restoring'` with `needs_full_sync = 1`, bumps `reload_generation`, clears the ack triple, AND does NOT flip to `'active'`. Remap blocks writes until RCRT's UUID-first `full_hash_reconcile` completes because pre-existing `file_state` paths are stale against the new (possibly reorganized) vault — a write resolving by slug-relative path could create a duplicate-UUID collision with the real moved file. `state='active'` is flipped only by RCRT's attach-completion tx after the reconcile commits.
 - **Remove mode** (`quaid collection remove --purge`): cascades the DELETE after the handshake; no `state='active'` transition applies because the row is gone.
- (f) **RCRT attaches — mode-specific handoff.** The attach handoff always runs under the per-collection single-flight mutex (`Arc<Mutex<()>>` keyed by `collection_id`), acquires a fresh `root_fd`, runs `full_hash_reconcile` (UUID-first), starts a new watcher, spawns a new per-collection supervisor, and registers its `JoinHandle` in `supervisor_handles`. Mode differences:
 - **Restore mode:** state is already `'active'` AND `needs_full_sync = 1` when the command exits (set by Tx-B via RestoreOriginator). RCRT's next sweep observes the owned active collection with no live supervisor handle and invokes the attach handoff: acquires the per-collection single-flight mutex, opens fresh `root_fd` against the new `root_path`, runs `full_hash_reconcile` (which repopulates `file_state` from the new tree), THEN in ONE attach-completion tx `UPDATE collections SET needs_full_sync = 0, reload_generation = reload_generation + 1 WHERE id = ? AND needs_full_sync = 1` (clears the write-gate), THEN spawns the new watcher + per-collection supervisor and registers the `JoinHandle`. Logs `rcrt_attach collection=<N> root=<P> reason=online_active` at WARN. Writes become legal at the moment the attach-completion tx commits.
 - **Remap mode:** state remains `'restoring'` with `needs_full_sync=1` after the command's DB-update tx. RCRT's next sweep observes this signature (`state='restoring' AND needs_full_sync=1 AND pending_root_path IS NULL AND pending_manifest_incomplete_at IS NULL AND integrity_failed_at IS NULL AND restore_command_id IS NULL` — distinct from pending-finalize restore AND from terminal integrity-blocked rows) and routes to the remap attach path: acquires the mutex, opens fresh `root_fd`, runs `full_hash_reconcile` WITH STATE STILL `'restoring'` (so concurrent writes remain interlocked during reconcile), then in the attach-completion tx flips `state='active'` + clears `needs_full_sync=0` + bumps `reload_generation` again (second bump — distinct from the command's initial bump). Only after this tx commits does RCRT spawn the supervisor. Writes become legal only at the moment of this final state flip. Logs `rcrt_attach collection=<N> root=<P> reason=remap_post_reconcile` at WARN.

 The newly-spawned supervisor (in both modes) resumes the standard `(state, reload_generation)` poll for future handshakes. If another attach path races to the same collection, the single-flight mutex serializes them; the later acquirer sees a live handle in the registry and exits without duplicating work. **Expected attach latency:** restore mode up to `QUAID_DEFERRED_RETRY_SECS` (default 30s); remap mode is `QUAID_DEFERRED_RETRY_SECS + full_hash_reconcile_duration` (the reconcile duration scales with vault size).

 **Read/write availability — unified across restore and remap post-finalize:**
 - **Reads remain available throughout every phase** — `restoring` state AND the post-Tx-B / post-remap-DB-update pre-attach window. Reads operate on DB rows (FTS, vec, `pages`) and do NOT require an open `root_fd` or live watcher; the data is already in SQLite and Tx-B / the remap DB-update tx do NOT delete `pages`, only `file_state`.
 - **Writes are blocked until RCRT's attach-completion tx commits, clearing `needs_full_sync = 0`.** Two gate conditions combine (OR, not AND): `state = 'restoring'` OR `needs_full_sync = 1`. The gate is enforced by the §4 write-interlock (task 11.8) which returns `CollectionRestoringError` for any `WriteCreate` / `WriteUpdate` / `WriteAdmin` tool invoked against a collection satisfying either condition. This covers:
 - **Pre-Tx-B restore AND pre-DB-update-tx remap**: `state = 'restoring'` is the gate (command is mid-handshake or mid-restore).
 - **Post-Tx-B restore pre-attach window** (fix — state is `'active'` but `needs_full_sync = 1` set by `run_tx_b`): `needs_full_sync = 1` is the gate. Reason: Tx-B also `DELETE FROM file_state WHERE collection_id = ?` in the same tx, so the post-rename tree on disk has NO matching `file_state` rows. `memory_put`'s canonical external-create precondition (`stat` succeeds on target parent + target name AND no `file_state` row for the path → classify as `ExternalCreate` → return `ConflictError`) would misclassify EVERY restored page as an unindexed external create. Blocking writes via the `needs_full_sync = 1` gate keeps the ExternalCreate guard honest without a restore-specific carve-out.
 - **Post-remap-DB-update pre-attach window**: `state = 'restoring' AND needs_full_sync = 1` — BOTH gates hold because remap's DB-update tx leaves state at `'restoring'` (for reasons in the "Remap via `sync --remap-root`" scenario). Either gate alone would suffice; both are set for belt-and-suspenders and for observability parity with restore's window.
 - **Watcher-overflow reconcile window**: `state = 'active' AND needs_full_sync = 1` — the watcher set the flag because events were dropped, the generic recovery worker will clear it within 1–2s. The gate briefly blocks writes during this fast recovery; reads stay available.
 - **RCRT's attach-completion tx opens the write-gate**: for both the `reason=online_active` (post-Tx-B restore) and `reason=remap_post_reconcile` (post-DB-update remap) branches, RCRT's attach-completion tx includes `UPDATE collections SET needs_full_sync = 0...` so writes become legal at the moment the tx commits — by which point `file_state` has been fully repopulated by `full_hash_reconcile` and the supervisor handle is registered.

 The 30s attach latency is acceptable because (i) reads stay available throughout, (ii) writes are gated honestly via `needs_full_sync` so no ExternalCreate false-positives can corrupt agent flows, (iii) the only observable effect is a brief write-block immediately after command completion, and (iv) users who need lower write-availability latency can tighten `QUAID_DEFERRED_RETRY_SECS` to e.g. 1–5s with negligible overhead for the single-digit expected collection count per memory.
- (g) **Serve startup rule (do-not-impersonate).** On `quaid serve` startup, when initializing a collection whose `state = 'restoring'` is observed, serve SHALL NOT write `watcher_released_*` fields. It SHALL treat the collection as detached for the duration of the restoring state and rely on the command owner to drive the handshake to completion or abort. This guarantees that only the session that was live when the command captured `expected_session_id` is eligible to write an accepted ack — if that session dies mid-handshake, no successor session can impersonate it.

**4. Write-path interlocking.** While a collection is in `state = 'restoring'` OR `needs_full_sync = 1` (OR — either condition alone triggers the gate), ALL MCP and CLI tools classified as `WriteCreate`, `WriteUpdate`, or `WriteAdmin` by task 2.3 SHALL return `CollectionRestoringError` before any fd-walk, tempfile write, `.quaidignore` mutation, or DB mutation. The second gate (`needs_full_sync = 1`) covers the post-Tx-B-restore and post-remap-DB-update pre-attach windows (where `file_state` was deleted and not yet repopulated by RCRT's reconcile), as well as the watcher-overflow reconcile window. Writing through an incomplete `file_state` would misclassify restored pages as `ExternalCreate` under `memory_put`'s canonical precondition and produce spurious `ConflictError`s. The authoritative list (no tool may skip the check): `memory_put` / `quaid put`, `memory_check`, `memory_raw`, `memory_link`, `memory_link_close` mutate mode, **`memory_gap` WITH a slug** (slug-bound form; the slug-less memory-wide form is Read-classified per task 1.1c and does NOT take this interlock — it cannot race a restore because it resolves no collection), `quaid collection ignore add`, `quaid collection ignore remove`, **`quaid collection ignore clear --confirm` (WriteAdmin — explicit fail-closed clear MUST take the interlock to prevent a mid-handshake clear-against-old-root from becoming effective against the new root after finalize)**, `quaid collection migrate-uuids` (WriteAdmin — opt-in UUID write-back), `quaid collection add --write-quaid-id` when applied to an existing collection in restoring state (typically blocked at the unique-name level, but interlock applies to any subpath that reaches collection-level mutation), and any future collection-level admin mutator. Read ops (`memory_get`, `memory_search`, `memory_query`, `memory_list`, `memory_backlinks`, `memory_graph`, `memory_timeline`, `memory_tags`, `memory_link_close` lookup mode, `memory_gap` WITHOUT a slug, `quaid collection ignore list`, `quaid collection info`) remain available during `restoring`. The interlock is enforced at the command entry point via the shared `check_writable(collection_id)` helper, keyed off the same polled `state` read. This prevents (a) page mutations that would race the rename/finalize, and (b) the confidentiality-regression case where an ignore mutation applied against the OLD root becomes effective against the NEW root after finalize.

**5. Concurrent command refusal.** A second `restore` or `sync --remap-root` invocation against a collection already in `state = 'restoring'` SHALL error immediately with `RestoreInProgressError` — the in-progress command owns the collection for the duration of its run. No interleaving is attempted.

#### Scenario: Restore / remap refused when collection is dirty

- **WHEN** a user runs `quaid collection restore work /new_path` OR `quaid collection sync work --remap-root /new_path` (online or offline) against a collection where `is_collection_dirty(collection_id)` returns TRUE — the durable dirty predicate is `collections.needs_full_sync = 1` OR ANY `<write_id>.needs_full_sync` sentinel file exists under `<memory_data_dir>/recovery/<collection_id>/` (broadening — per the agent-writes spec, the sentinel is the PRIMARY durable dirty signal when a `memory_put` crashes post-rename pre-commit; the fresh-connection `needs_full_sync` UPDATE may legitimately fail while the sentinel is guaranteed durable by the write sequence's step 5)
- **THEN** the command refuses upfront with `CollectionDirtyError` naming the collection AND listing both sub-signals independently (`needs_full_sync=<bool> sentinel_count=<N> recovery_in_progress=<flag> last_sync_at=<timestamp>`) so the operator can distinguish which dirty-signal path they are waiting on
- **AND** NO staging directory is created, NO `collections.state` mutation runs, NO handshake is attempted (the preflight guard runs BEFORE ownership resolution)
- **AND** the error message instructs the operator to wait for RCRT's sweep / the generic recovery worker to complete (which reconciles AND unlinks any sentinels as part of post-reconcile cleanup), OR to run `quaid collection sync work` (the read-only reconcile path) to clear both signals, OR to restart `quaid serve` so the startup recovery path runs
- **AND** once the dirty predicate returns FALSE (both `needs_full_sync = 0` AND no sentinels), the SAME command succeeds normally (the guard does NOT require any destructive intervention; it is a gate, not a terminal state)
- **AND Round-77/78 post-release drift capture + filesystem stability check**: internal dirty signals (`needs_full_sync`, sentinels) catch in-process failure modes; they do NOT catch an external editor or sync tool that wrote directly to `collections.root_path` between the watcher's release and the `raw_imports` read / remap DB-update. Two-phase defense:
 - **Phase 1 — drift capture**: restore and remap SHALL — after handshake release (online) OR offline-lease acquire (offline) AND BEFORE raw_imports is consumed / the remap DB-update runs — open a fresh `root_fd` against the OLD `collections.root_path` and invoke `full_hash_reconcile(collection_id, root_fd, mode=synchronous_drift_capture)` bypassing the normal `state='active'` gate (authorized because the command owns the collection via lease / `restore_command_id`). This ingests newer-on-disk bytes into fresh `raw_imports` / `pages` / `file_state` rows. For **restore**, the captured drift becomes the authoritative `raw_imports` content that step (e) materializes into the new target. For **remap**, drift has nowhere to propagate (remap is non-destructive re-attach to an already-existing `/new/path`; it does NOT copy captured content into the new tree). The correction replaces the earlier "remap adopts old-root drift" claim with **refusal semantics**: if remap's reconcile reports ANY non-zero drift (`pages_updated > 0` OR `pages_added > 0` OR quarantines), abort with `RemapDriftConflictError` naming the counts; the operator MUST EITHER verify `/new/path` already contains those edits (then re-run remap — the drift is now in DB so a second pass will see zero drift) OR use `quaid collection restore` instead (which materializes raw_imports into a fresh tree, guaranteeing no edit is lost). Log `remap_drift_refused collection=<N> pages_updated=<P> pages_added=<A> pages_quarantined=<Q>` at ERROR. Log `restore_drift_captured collection=<N> pages_updated=<P> pages_added=<A> pages_quarantined=<Q>` at WARN for restore when non-zero.
 - **Phase 2 — filesystem stability check**: after Phase 1 reconcile commits (restore) OR after Phase 1 reports zero drift (remap), the command captures two successive stat-only snapshots `snap1`, `snap2` of every file under the old root (`(relative_path, mtime_ns, ctime_ns, size_bytes, inode)` tuples — cheap, no content hashing). If `snap1 == snap2`, the tree is stable-enough-for-now and the command continues. If the snapshots differ, an external writer is still active: re-invoke Phase 1 reconcile to ingest the new diff, then capture `snap3` and compare against `snap2`; retry up to `QUAID_RESTORE_STABILITY_MAX_ITERS` times (default 5). On persistent instability, abort with `CollectionUnstableError` — the writer is too busy for a reliable capture; the operator must quiesce external writers and retry. For remap, if a retry iteration captures non-zero drift, fall back to `RemapDriftConflictError`. Log `restore_aborted_unstable` / `remap_aborted_unstable collection=<N> iters=<I>` at WARN.
 - **Phase 3 — pre-destruction fence**: IMMEDIATELY before the destructive step (restore's Tx-A, remap's DB-update tx), the command takes ONE final stat-only walk `snap_fence` and compares against the last stable snapshot (`snap_final`). If they differ, a write landed AFTER Phase 2 proved stability but BEFORE the destructive step — abort via the abort-path resume sequence, log `restore_aborted_fence_drift` / `remap_aborted_fence_drift` at WARN, return `CollectionUnstableError`. This minimizes (does NOT eliminate) the residual window.
 - **Phase 4 (remap only) — `/new/path` manifest verification + new-root stability fence**: remap additionally verifies `/new/path` contents against the authoritative `raw_imports` BEFORE the DB-update tx. Scope matches the rest of the remap/reconcile model:
 - **Required page set**: ONLY active-indexable pages (`quarantined_at IS NULL` AND NOT filtered by `.quaidignore`). Quarantined pages intentionally have no backing file and are EXCLUDED from the required set.
 - **Allowed-but-not-required files**: paths matching `.quaidignore` patterns on `/new/path` (re-read at remap time), the built-in ignore set (`.obsidian/**`, `node_modules/**`, `_templates/**`, etc.), AND user-defined patterns — these files MAY exist without a `pages` row and are NOT "extra".
 - **Identity resolution**: Phase 4 SHALL invoke the SAME `resolve_page_identity(...)` helper that `full_hash_reconcile` uses. The canonical contract (single source of truth): (a) `quaid_id` frontmatter UUID match against `pages.uuid`; (b) content-hash uniqueness with `size > 64` AND non-empty-body-after-frontmatter guards. The earlier "same `file_state.relative_path`" shortcut is REMOVED — relying on it green-lights remaps that the post-attach reconciler then churns. Pages whose `quaid_id` is absent AND content is trivial (`size <= 64` OR empty body) are unresolvable under this contract; Phase 4 fails with `UnresolvableTrivialContentError` naming the affected pages and directing the operator to `migrate-uuids` or `restore`. Invariant 17.17(n) `resolver_unification` enforces that any resolver rule used by Phase 4 MUST also appear in `full_hash_reconcile`'s spec.
 - **Pass criteria**: (i) every active-indexable page resolves to exactly one file on `/new/path` via the canonical resolver, (ii) that file's sha256 matches the page's authoritative `raw_imports.raw_bytes`, (iii) every non-ignored file on `/new/path` resolves to exactly one page (truly-untracked files count as "extra"). Quarantined pages and ignored files are EXCLUDED from both sides of the required↔present bijection.
 - **Full-tree new-root stability fence**: the verifier captures `newroot_snap_pre` during its content walk including (i) the full file-set WITHOUT pre-filtering by `.quaidignore` (so newly-added files CAN be detected), (ii) per-file stat tuples, (iii) sha256 of `/new/path/.quaidignore`. Immediately before the DB-update tx, the verifier re-captures `newroot_snap_fence` via a FULL tree walk (not just re-statting pre-known files). Drift of any kind — file-set membership change (`added`/`removed`), per-file tuple change, OR `.quaidignore` sha256 change — aborts with `NewRootUnstableError` naming the drift type and sample. This mirrors the old-root Phase 3 fence but broadened (correction — a stat-only walk over pre-known files misses new additions and ignore-policy shifts); the same residual-microsecond window applies and is disclosed identically. The full-tree readdir adds latency for large vaults — the tradeoff is "longer remap latency vs silent adoption of an inconsistent tree"; the spec chooses the former.
 Any pass-criteria failure aborts with `NewRootVerificationFailedError` naming counts (`missing=<M> mismatch=<X> extra=<E>`) and sampled diffs. The operator receives a machine-verified answer — not a silent post-attach quarantine cascade from a partial manual sync.
 - **Residual-race refusal: the ship-ready contract requires kernel-enforced no-write on the old root.** The sub-millisecond window between Phase 3's `snap_fence` fstat and the destructive SQLite commit is NOT fenced by POSIX primitives. To eliminate silent data-loss paths, restore and remap SHALL run ONLY when a kernel-enforced no-write guarantee holds on the old root:
 - **(a) Old root is on a read-only mount.** The command `statvfs`es the old `root_path` and observes `ST_RDONLY` (Linux `statvfs().f_flag & ST_RDONLY`) or `MNT_RDONLY` (macOS `statfs().f_flags & MNT_RDONLY`). A read-only mount makes external writes impossible by construction — the race window is closed by the kernel. The command runs the full Phase 1 → Phase 4 capture/stability/fence pipeline for defense-in-depth, but the residual-race class is structurally eliminated. On Linux, operators MAY obtain the RO guarantee cheaply via `mount --bind -o ro <old_root> <old_root>`; on macOS, the supported path is a loopback read-only mount or running recovery on a machine where no writers exist. The preflight SHALL log `restore_ro_mount_verified collection=<N> mount_flags=<F>` at INFO.
 - **(b) Writable mount → refuse.** On any writable mount (no `ST_RDONLY`/`MNT_RDONLY`), the command SHALL refuse with `CollectionLacksWriterQuiescenceError` naming the two acceptance paths: (i) remount the old root read-only (Linux: `mount --bind -o ro`; macOS: loopback RO or APFS snapshot mount), or (ii) run the recovery from a quiesced environment where no external writers can touch the old root. No `--writers-quiesced` / `--unsafe-accept-residual-race` override exists. Operator-asserted quiescence on a writable mount was explicitly removed: it permitted a nominally-successful command to silently drop a concurrent write, and no audit log or ERROR banner is an adequate substitute for refusing the run.

 **Ship-ready contract:** restore and remap proceed only under (a). Automated/unattended contexts (CI pipelines, scheduled recovery scripts, operator-runbook green paths) see the same binary gate. The `--online` flag remains ORTHOGONAL: `--online` coordinates with a live `quaid serve`; the RO-mount precondition still applies.
- **AND Round-76 TOCTOU recheck (tertiary defense on internal signals)**: AFTER Phase 2 stability AND BEFORE raw_imports is consumed / the remap DB-update runs, restore and remap re-evaluate `is_collection_dirty(collection_id)` on a fresh SQLite connection AND re-scan the sentinel directory. A TRUE result at this point means a SECOND-order dirty event (e.g., a watcher-overflow or sentinel drop by another Quaid command — rare but possible). If TRUE, abort via the standard abort-path resume sequence (revert state, keep root_path, clear ack triple, NULL heartbeat, bump reload_generation as RCRT's ordering marker, stop heartbeat tasks, drop offline lease), log `restore_aborted_dirty_recheck` / `remap_aborted_dirty_recheck` at WARN, return `CollectionDirtyError` non-zero.
- **AND** plain `quaid collection sync work` (without `--remap-root`) is EXEMPT from this guard because it IS the reconcile path (its purpose is to clear both signals); `quaid collection sync work --finalize-pending` is also EXEMPT because its helper operates on `pending_root_path` (the new, just-renamed tree) rather than the dirty `root_path` (the old tree)
- **Rationale:** a dirty collection means on-disk bytes at the old root may be NEWER than `pages` / `raw_imports`. Restore materializes `raw_imports.raw_bytes` into a fresh vault; remap DELETEs `file_state` and re-reconciles against the new root. Running either in a dirty window would silently discard the pending on-disk edits. This is a data-loss path, not a transient consistency issue.

#### Scenario: Restore / remap refuses when trivial-content pages lack frontmatter `quaid_id`

- **WHEN** an operator runs `quaid collection restore work /new_path` (or `quaid collection sync work --remap-root /new_path`) and the collection contains one or more pages whose `uuid` is not mirrored into the file's frontmatter `quaid_id` AND whose body is trivial (≤ 64 bytes after frontmatter OR empty)
- **THEN** the UUID-migration preflight (runs before the RO-mount gate) refuses with `UuidMigrationRequiredError`, naming the count of affected pages and up to 5 sample paths, and directing the operator to run `quaid collection migrate-uuids <name>` before retrying
- **AND** NO RO-mount check, NO staging, NO Phase 1 reconcile, and NO state mutation is attempted
- **AND** running `quaid collection migrate-uuids work` (which writes frontmatter `quaid_id` for every affected page, offline) followed by the original restore/remap command succeeds because every page now has either UUID-anchored identity OR content-hash-unique identity
- **AND** this gate closes the silent-identity-loss path for short/template notes that have neither a frontmatter UUID nor content-hash uniqueness: under the old behavior, Phase 4 would fail with `UnresolvableTrivialContentError` mid-restore; the preflight now catches it up-front against DB state without a filesystem walk

#### Scenario: Restore on a writable mount — refused

- **WHEN** a user runs `quaid collection restore work /new_path` (or `quaid collection sync work --remap-root /new_path`) against a collection whose old `root_path` is on a writable mount
- **THEN** the preflight `statvfs(old_root)` observes the mount is NOT `ST_RDONLY` / `MNT_RDONLY`; the command refuses with `CollectionLacksWriterQuiescenceError`
- **AND** NO staging directory is created, NO `collections.state` mutation runs, NO handshake is attempted
- **AND** the error message names the old mount's writability and lists the two acceptance paths: (i) remount the old root read-only (Linux `mount --bind -o ro <old_root> <old_root>`, macOS loopback RO or APFS snapshot mount), or (ii) run recovery from a quiesced environment where no external writers can touch the old root
- **AND** the error message explicitly notes that no `--writers-quiesced` / `--unsafe-accept-residual-race` override exists — operator-asserted quiescence on a writable mount was removed because it permitted silent data loss

#### Scenario: Restore against a read-only old root — proceeds as the ship-ready path

- **WHEN** the old `root_path` is on a mount that reports `ST_RDONLY` / `MNT_RDONLY` (e.g., APFS snapshot mount, Linux `mount --bind -o ro` loopback, read-only network share for recovery from a failing disk)
- **THEN** the quiescence gate accepts precondition (a); the residual-race window is closed by the kernel for the duration of the command because external writes cannot succeed against the old root
- **AND** the command proceeds with Phase 1 reconcile, Phase 2 stability, Phase 3 fence, and Tx-A / rename normally, logging `restore_ro_mount_verified collection=<N> mount_flags=<F>` at INFO
- **AND** on successful Tx-A/Tx-B the summary line reports `restored=N byte_exact=N pending_finalize=<bool>` without any unsafe-mode field

#### Scenario: Offline restore — no live serve session, no coordination needed

- **WHEN** no `quaid serve` session is live (heartbeat stale or row absent) and a user runs `quaid collection restore work /path`
- **THEN** the command proceeds immediately without the `--online` handshake, since there is no running process that could hold a stale `root_fd`
- **AND** the restore completes with the normal sequence (stage, verify, atomic rename, update `root_path`, state → `active`)

#### Scenario: Restore with live serve and no `--online` flag — refused

- **WHEN** `quaid serve` is running (heartbeat fresh) and a user runs `quaid collection restore work /path` WITHOUT `--online`
- **THEN** the command errors with `ServeOwnsCollectionError` identifying the session's `pid` and `host`
- **AND** no `collections.state` change; no staging directory; no mutation

#### Scenario: Restore with live serve and `--online` — handshake succeeds (ack lease matches on session_id + generation)

- **WHEN** `quaid serve` is running as `session_id = S` with `root_fd` open on `<old_root>`, and a user runs `quaid collection restore work /new_path --online`
- **THEN** the command resolves ownership via `collection_owners` for the target collection and captures `expected_session_id = collection_owners.session_id = S` (the single-owner lease — NOT an arbitrary `serve_sessions` row); it verifies `serve_sessions.heartbeat_at > now() - 15s` for that session. Then in one tx it sets `state = 'restoring'`, computes `cmd_reload_generation = reload_generation + 1` and writes it back, clears `watcher_released_session_id`, `watcher_released_generation`, `watcher_released_at`, and sets `pending_command_heartbeat_at = now()`
- **AND** within the next 250ms serve's per-collection supervisor observes the state change, stops the watcher, flushes the debounce batch, closes `root_fd`, writes the ack triple in one tx (`watcher_released_session_id = S`, `watcher_released_generation = cmd_reload_generation`, `watcher_released_at = now()`), AND EXITS (removes its entry from the process-global `supervisor_handles` registry per task 11.7's contract)
- **AND** the command's poll matches `(watcher_released_session_id = S) AND (watcher_released_generation = cmd_reload_generation) AND (watcher_released_at IS NOT NULL)` within the timeout window and proceeds with staging + atomic rename + `finalize_pending_restore(..., FinalizeCaller::RestoreOriginator { command_id })` which runs Tx-B via `run_tx_b`: updates `root_path` to `/new_path` + state → `active` + bumps `reload_generation` again + clears the ack triple + NULLs all pending/integrity/originator-identity columns (`pending_root_path`, `pending_restore_manifest`, `integrity_failed_at`, `pending_manifest_incomplete_at`, `pending_command_heartbeat_at`, `restore_command_id`, `restore_command_pid`, `restore_command_host`, `restore_command_start_time_unix_ns`)
- **AND** RCRT's next sweep (task 9.7d — the sole runtime reattach actor under the /55/56/57 single-actor contract, sweeping every `QUAID_DEFERRED_RETRY_SECS`, default 30s) observes the owned collection with `state='active'` and no live `supervisor_handles` entry; under the per-collection single-flight mutex it opens a fresh `root_fd` against `/new_path`, runs `full_hash_reconcile`, starts a new watcher, spawns a new per-collection supervisor, and registers the new `JoinHandle`; logs `rcrt_attach collection=<N> root=/new_path reason=online_active` at WARN — no serve restart required, expected reattach latency ≤ `QUAID_DEFERRED_RETRY_SECS`
- **AND** subsequent MCP reads/writes on that collection operate against the new root

#### Scenario: Stale ack from a prior handshake generation is ignored

- **WHEN** a prior `--online` handshake had timed out at generation `N`, leaving `watcher_released_session_id = S, watcher_released_generation = N, watcher_released_at = <old_time>` briefly visible before the aborting command cleared the triple
- **AND** a later `--online` handshake opens at generation `N+1` with `expected_session_id = S`
- **AND** a spurious delayed write from the crashed prior-handshake code path lands, writing `watcher_released_generation = N` (not `N+1`)
- **THEN** the command's poll SHALL NOT accept the stale ack — `watcher_released_generation = N` does not equal `cmd_reload_generation = N+1`, so the match fails
- **AND** the command either observes a subsequent correct ack at generation `N+1` or times out and aborts — at no point does it proceed against serve state that has not actually released for THIS handshake

#### Scenario: Foreign-session ack is ignored

- **WHEN** session `S` owns the collection via `collection_owners` and is live when the command captures `expected_session_id = S` from that lease, opening a handshake at generation `N+1`
- **AND** session `S` dies before writing the ack; a new serve process comes up as session `T` and initializes the collection; per the startup rule, session `T` observes `state = 'restoring'` and does NOT write an ack
- **AND** (hypothetically) a race or bug causes session `T` to write an ack triple with `watcher_released_session_id = T`
- **THEN** the command's poll SHALL NOT accept the ack — `watcher_released_session_id = T` does not equal `expected_session_id = S`, so the match fails
- **AND** the command detects via its per-poll liveness re-read that session `S` has disappeared or gone stale, and aborts with `ServeDiedDuringHandshakeError` rather than waiting for the (impossible) matching ack

#### Scenario: Serve dies mid-handshake — command aborts promptly, does not wait for the full timeout

- **WHEN** the command has opened a handshake (state = 'restoring', generation bumped) and is polling for the ack, AND `serve_sessions` for `expected_session_id` is DELETEd (clean shutdown) or its `heartbeat_at` ages past 15s (crash)
- **THEN** on the next poll cycle (within 100ms), the command detects the missing/stale liveness row and aborts with `ServeDiedDuringHandshakeError`
- **AND** the abort reverts `state` to its prior value, clears the ack triple, cleans up any staging directory, and returns non-zero
- **AND** the 30s handshake timeout is NOT waited out — the liveness check is the fast path for this failure mode

#### Scenario: Online handshake times out — command aborts AND bumps resume generation so serve rebinds to prior root

- **WHEN** `quaid collection restore --online` is invoked at generation `N+1`, serve observes `state='restoring'` at `N+1`, stops the watcher, closes `root_fd`, begins draining embedding jobs, but its supervisor is stuck past the 30s handshake timeout
- **THEN** the command's poll-loop hits timeout and enters the abort path
- **AND** in a SINGLE durable SQLite tx via a FRESH connection, the command: (i) reverts `state` to its prior value (`active`, since `root_path` is unchanged and still readable); (ii) keeps `root_path` unchanged; (iii) clears the ack triple; (iv) bumps `reload_generation` to `N+2`. The generation bump is an **ordering marker** for RCRT's sweep, NOT a poll trigger for the (now-exited) supervisor — it ensures a clean audit trail ordering "state transitioned after the release"
- **AND** RCRT's next sweep (within `QUAID_DEFERRED_RETRY_SECS`) observes the owned collection with `state='active'` and no live `supervisor_handles` entry; under the per-collection single-flight mutex it opens a fresh `root_fd` against the unchanged `root_path`, runs `full_hash_reconcile`, starts a new watcher, and spawns a new per-collection supervisor — logs `rcrt_attach collection=<N> root=<P> reason=abort_resume` at WARN
- **AND** the command removes any staging directory, logs `restore_abort_resumed collection=<N> prior_generation=N+1 resume_generation=N+2` at WARN, returns `ServeHandshakeTimeoutError` non-zero
- **AND** at NO point is serve left with a torn-down watcher and no rebind signal — RCRT's continuous sweep is the positive rebind trigger under the /55/56/57 single-actor contract

#### Scenario: Serve-died abort ALSO bumps resume generation (defense-in-depth)

- **WHEN** the command detects serve-died during the handshake (`ServeDiedDuringHandshakeError` path) AND the abort runs
- **THEN** the same abort-path resume semantics apply: revert `state`, keep `root_path`, clear ack triple, bump `reload_generation` to `cmd_reload_generation + 1`
- **AND** the resume-generation bump is defensive in this case — the original serve that saw `state='restoring'` is already dead, so there is no supervisor to rebind. But if a successor `quaid serve` starts later, its startup path observes the collection at `state='active'` at a generation STRICTLY GREATER than any generation that could have been observed by a live supervisor, guaranteeing the successor does a fresh full-hash reconcile on attach (it won't skip the rebind because "last observed generation" is stored per-supervisor-session and a fresh serve starts with no prior observation). This also guarantees the new serve cannot be confused by any stale in-flight work from the dead serve's generation `N+1`.

#### Scenario: Writes against a collection in `restoring` state are refused

- **WHEN** a collection is in `state = 'restoring'` and an agent calls `memory_put("work::notes/x", content)`
- **THEN** the call returns `CollectionRestoringError` before any fd-walk, tempfile write, or DB mutation
- **AND** the agent is expected to retry after observing `collections.state = 'active'` via `memory_collections` or `quaid collection info`

#### Scenario: Two concurrent `--online` restores against the same collection — second refused

- **WHEN** a restore is in progress (`state = 'restoring'`) and a second `quaid collection restore <same_name>` invocation starts
- **THEN** the second invocation errors with `RestoreInProgressError`; no mutation; no staging

#### Scenario: Remap via `sync --remap-root` — non-destructive re-attach to an existing vault at a new path

- **WHEN** a user runs `quaid collection sync work --remap-root /new/path` where `/new/path` is an ALREADY-EXISTING directory holding the vault contents (typical use case: the vault was moved to a new path out-of-band, e.g., after restoring from backup, migrating machines, or a `mv` operation). Remap is fundamentally a re-attach-at-new-path operation, NOT a fresh materialization of the vault — that is `quaid collection restore`'s responsibility. Remap SHALL NOT stage or rename any directories; it updates `collections.root_path` and runs `full_hash_reconcile` against the existing tree with UUID-first identity resolution per the adjacent "Remap root for detached collections (identity preserved via UUID)" requirement.
- **AND** against a live serve session without `--online`, the command errors with `ServeOwnsCollectionError` (same error shape as restore).
- **AND** with `--online`, the handshake protocol proceeds identically to restore through the release phase: the command captures `expected_session_id` from `collection_owners` (NOT `serve_sessions`), sets `state='restoring'` as a "do-not-touch" marker for the duration of the remap, bumps `cmd_reload_generation`, NULLs the ack triple and sets `pending_command_heartbeat_at`; the per-collection supervisor observes the release, writes the ack triple, AND EXITS (removes its `supervisor_handles` entry per task 11.7's contract). Then remap DIVERGES from restore — it does NOT stage/verify/rename and does NOT populate `pending_root_path` / `pending_restore_manifest` / `restore_command_id`. Instead the command validates that `/new/path` exists and is a readable directory (refuses otherwise), then in ONE tx: `UPDATE collections SET root_path = '/new/path', reload_generation = reload_generation + 1, watcher_released_session_id = NULL, watcher_released_generation = NULL, watcher_released_at = NULL, pending_command_heartbeat_at = NULL, needs_full_sync = 1 WHERE id = ?` AND `DELETE FROM file_state WHERE collection_id = ?` (file_state from the prior root is meaningless). **Round-71 write-safety correction:** `state` REMAINS `'restoring'` after this tx — it is NOT flipped to `'active'` by the command. The command exits successfully with a message indicating that UUID-first reconciliation is pending. Flipping to `'active'` prematurely would allow `memory_put` through the `CollectionRestoringError` interlock (task 11.8) while the new-root `file_state` is empty and slug-path topology is stale; a write resolving by slug-relative-path against a reorganized tree could create a duplicate-UUID collision with the real moved file once RCRT's reconciler discovers it. Keeping `state='restoring'` blocks all mutating writes until the state transition is safe. The remap tx does NOT touch any pending/integrity/command columns — they are guaranteed NULL pre-remap by the preflight blocking-state guard (task 9.5 addition) which refuses remap against terminal integrity states OR plain pending-finalize.
- **AND** RCRT's next sweep observes the owned collection in `state='restoring'` with `needs_full_sync=1` AND `pending_root_path IS NULL` AND `pending_manifest_incomplete_at IS NULL` AND `integrity_failed_at IS NULL` AND `restore_command_id IS NULL` (the remap-post-reconcile-pending signature — distinct from restore's pending-finalize signature AND distinct from terminal integrity-blocked rows, whose `pending_manifest_incomplete_at IS NOT NULL` OR `integrity_failed_at IS NOT NULL` must refuse the remap branch) and routes to the single-flight attach handoff specifically for this case: acquires the per-collection single-flight mutex, opens fresh `root_fd` against `/new/path`, runs `full_hash_reconcile` (required: the UUID-first procedure resolves identity against the new tree; stat fields from the prior root are meaningless) WITH THE STATE STILL `restoring` (so concurrent writes remain interlocked during the reconcile pass), then in ONE attach-completion tx: `UPDATE collections SET state = 'active', needs_full_sync = 0, reload_generation = reload_generation + 1 WHERE id = ?` (second generation bump so observers see a distinct "reconcile complete" transition from the initial "remap DB-update" transition). Only AFTER this tx commits does RCRT spawn the new per-collection supervisor and register its `JoinHandle` in `supervisor_handles`. Writes become legal at the exact moment `state='active'` lands, by which point `file_state` is fully populated with UUID-resolved per-page paths. Logs `rcrt_attach collection=work root=/new/path reason=remap_post_reconcile` at WARN. There is NO "serve rebinds itself" path — attach is exclusively RCRT's responsibility under the /55/56/57/58/59/60/61/63/71 single-actor contract. Expected attach latency: up to `QUAID_DEFERRED_RETRY_SECS + full_hash_reconcile_duration` (the reconcile duration scales with vault size; 30s default sweep + reconcile time).
- **AND** remap does NOT use `finalize_pending_restore` at any point because it does not produce a pending-finalize state — the one-tx update flips state cleanly without needing the two-phase recoverable-finalize dance that restore requires. This distinction keeps remap simple and non-destructive: restore materializes bytes from `raw_imports` into a sibling staging dir and atomically renames onto an absent target (the case where crash-after-rename recovery matters); remap adopts an already-existing tree (the case where there's no rename to recover).
- **AND** integration test asserts `supervisor_handles.get(&collection_id)` is `None` between the supervisor's exit (after ack) and RCRT's attach (after the remap tx commits); at no point do two supervisor handles coexist for the same collection.

#### Scenario: Serve crashes mid-handshake — command aborts and FULLY unwinds state

- **WHEN** the command has set `state = 'restoring'` at generation `N+1` with `expected_session_id = S` and is polling for the ack, serve `S` crashes without writing the ack, and serve's heartbeat row goes stale within 15s
- **THEN** the command detects the stale `serve_sessions` row on its next liveness re-read (within 100ms after the 15s liveness window closes) and aborts with `ServeDiedDuringHandshakeError`
- **AND** the abort commits a SINGLE durable SQLite tx via a FRESH connection (so it succeeds even if the command's main connection is in a broken state) that (i) reverts `state` to its prior value — `active` if the prior `root_path` is still a readable directory, `detached` otherwise, (ii) NULLs `pending_root_path` (it was never set since Tx-A did not run — NULLing is a no-op but asserted explicitly for idempotency), (iii) clears the ack triple (`watcher_released_session_id`, `watcher_released_generation`, `watcher_released_at` = NULL); (iv) also removes any staging directory `<target>.quaid-restoring-*/` as best-effort cleanup
- **AND** when a subsequent `quaid serve` starts (new session `T`), it observes the collection in its reverted state (NOT `restoring`), attaches normally, and starts its watcher/supervisor. Successor-session impersonation is impossible because the ack triple is NULL and `expected_session_id = S ≠ T`.

#### Scenario: Serve crashes AND command ALSO crashes before committing the revert — time-gated startup orphan recovery

- **WHEN** serve `S` crashes, the command detects the stale session and begins aborting, but the command process is killed (SIGKILL, OOM, etc.) BEFORE its abort-tx commits the state revert
- **AND** the collection is left with `state = 'restoring'`, `pending_root_path = NULL` (Tx-A never ran — rename was not attempted), ack triple possibly NULL or stale, AND `pending_command_heartbeat_at` was last refreshed by the dead command
- **THEN** auto-recovery of the "aborted handshake orphan" branch is GATED on proving the command is dead. It SHALL run ONLY when the new serve has (a) already SUCCESSFULLY claimed ownership of the collection via `collection_owners` (which requires the prior owner's `serve_sessions` row to have been swept as stale — proving the prior serve session is dead), AND (b) observed `collections.pending_command_heartbeat_at IS NULL OR pending_command_heartbeat_at < now() - (2 * QUAID_RELOAD_HANDSHAKE_TIMEOUT_SECS)` (default 60s) — proving no live command could still be waiting to write its abort revert. Only after BOTH conditions hold does serve invoke `finalize_pending_restore(collection_id, FinalizeCaller::StartupRecovery { session_id: <own_session_id> })` with EXPLICIT caller identity (per invariant 17.17(k) — the zero-arg shorthand is forbidden), which then enters the `OrphanRecovered` branch.
- **AND** the recovery path reverts `state` to `active` if the prior `root_path` is a readable directory, `detached` otherwise; clears the ack triple; NULLs `pending_command_heartbeat_at`; removes any `<root_path>.quaid-restoring-*/` staging dirs under the collection's parent directory (best-effort); logs `restore_orphan_recovered collection=<N> state=<reverted_state>` at WARN
- **AND** if the gate fails (ownership claim succeeded but `pending_command_heartbeat_at` is fresh), the new serve logs `restore_orphan_deferred collection=<N> command_heartbeat_age_s=<A>` at INFO and treats the collection as detached for the duration of `restoring`. The live command is expected to either complete its abort-revert (state transitions out of `restoring`) or its heartbeat will eventually stall, at which point a later recovery cycle succeeds.
- **AND** because `collection_owners` has a PK on `collection_id`, a second serve that has NOT claimed ownership can NEVER run this recovery path — it cannot even observe collections it does not own beyond read-only inspection. This closes the race where a fresh serve could roll back a live `--online` restore that was mid-handshake against another serve's session.

#### Scenario: Stale serve_session rows swept at startup

- **WHEN** `quaid serve` starts and finds `serve_sessions` rows with `heartbeat_at < now() - 15s`
- **THEN** the stale rows are DELETEd in a sweep tx before inserting the new session row
- **AND** a WARN log records the cleanup (`serve_session_stale_sweep count=<N>`) for operator visibility

### Requirement: Remap root for detached collections (identity preserved via UUID)

The system SHALL provide a mechanism to re-attach a detached collection to a new local path via `quaid collection sync <name> --remap-root <new-path>`. After remap, reconciliation SHALL proceed against the new root using the UUID-first full-hash procedure defined in the "Full-hash reconciliation on remap, fresh-attach, and first-use-after-detach" requirement. `file_state.relative_path` values are NOT blindly reinterpreted against the new root — they are resolved per-page via `quaid_id` frontmatter matching, so directory reorganizations during the move do not churn `pages.id` or drop DB-only state.

#### Scenario: Remap with reorganized layout preserves `pages.id` and DB-only state

- **WHEN** a user runs `quaid collection sync work --remap-root ~/Documents/work-vault-on-new-machine` and the new machine has reorganized directories (files moved between subdirectories, some renamed)
- **THEN** `collections.root_path` is updated to the new path
- **AND** the full-hash reconciler runs the UUID-first procedure: pages whose `quaid_id` appears in the new tree preserve `pages.id`; `file_state.relative_path` is updated to the observed on-disk path
- **AND** programmatic links, non-import assertions, `raw_data`, and `contradictions` all survive the remap intact
- **AND** files with no `quaid_id` fall through to content-hash uniqueness or create-new; unresolved prior pages quarantine (if DB-only state) or hard-delete (if purely vault-derivable) per the standard rules

#### Scenario: Remap on first attach after move

- **WHEN** `memory.db` is moved to a new machine and `quaid serve` is started before any remap
- **THEN** the collection is reported detached; reconciliation is skipped; `quaid collection info` instructs the user to run `sync --remap-root`

### Requirement: Cross-machine-sync extension point

The schema and reconciler design SHALL preserve an extension point for future subtree-hash (Merkle) optimization without requiring schema changes to existing tables. The `file_state.sha256` column SHALL be content-addressable so that future inter-machine reconciliation can compare `(collection_name, relative_path, sha256)` tuples between brains.

#### Scenario: `file_state.sha256` is populated with a content-addressable sha256 for every tracked file

- **WHEN** any write path that inserts or updates a `file_state` row runs (`memory_put` rename-before-commit step 12, reconciler ingest, watcher Create/Modify handler, `full_hash_reconcile`, quarantine restore)
- **THEN** `file_state.sha256` is populated with the sha256 of the file's bytes as they landed on disk — the same bytes returned by reading the file via `openat(parent_fd, target_name, O_RDONLY | O_NOFOLLOW)` at that moment
- **AND** the sha256 is computed over the raw bytes (not `render_page()` output, not frontmatter-stripped content) — the content-addressable property is preserved so future inter-memory reconciliation can compare `(collection_name, relative_path, sha256)` tuples and resolve matching pages without re-hashing

#### Scenario: Schema extension for subtree/Merkle hashes does not require migrating existing tables

- **WHEN** a future change adds subtree-hash or Merkle-tree optimization for cross-machine sync
- **THEN** the addition SHALL be expressible as a NEW table (e.g., `file_state_subtree_hash` keyed on `collection_id + path_prefix`) rather than ALTERing any existing column in `pages` / `file_state` / `collections`
- **AND** `file_state.sha256` remains the authoritative per-file content hash used by the reconciler's identity resolver — the Merkle extension SHALL be a cache layered on top of that authoritative column, not a replacement for it
- **AND** this change does not block or pre-specify any specific Merkle algorithm — it only guarantees that the per-file sha256 contract is stable enough to build on

