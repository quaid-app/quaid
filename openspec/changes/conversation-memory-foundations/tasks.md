## 1. Landed v8 baseline slice (already in repo)

> **Truth note (Leela, 2026-05-04T07:22:12.881+08:00):** Tasks `1.1`–`1.8` and `2.1` describe the conversation-memory plumbing that is already landed in the live v8 baseline. Remaining implementation work for this change starts at `2.2`; it does not introduce another schema-version bump.

- [x] 1.1 Keep `src/schema.sql` aligned with the landed v8 baseline by retaining `pages.superseded_by INTEGER REFERENCES pages(id)` (nullable)
- [x] 1.2 Keep partial index `idx_pages_supersede_head ON pages(type, superseded_by) WHERE superseded_by IS NULL`
- [x] 1.3 Keep the guarded partial index `idx_pages_session ON pages(json_extract(IIF(json_valid(frontmatter), frontmatter, '{}'), '$.session_id')) WHERE json_valid(frontmatter) AND json_extract(IIF(json_valid(frontmatter), frontmatter, '{}'), '$.session_id') IS NOT NULL`
- [x] 1.4 Keep the landed `extraction_queue` table with columns and CHECK constraints per spec, plus `idx_extraction_queue_pending ON (status, scheduled_for) WHERE status = 'pending'`
- [x] 1.5 Keep the landed config defaults in the existing `config` table: `memory.location='vault-subdir'`, `corrections.history_on_disk='false'`, `extraction.max_retries='3'`
- [x] 1.6 Keep `SCHEMA_VERSION`, `quaid_config.schema_version`, and schema-version tests at `8` (the live baseline for this change)
- [x] 1.7 Verify no pre-v8 → v8 migration or rollback path is added; existing pre-v8 DBs continue to fail with the schema-mismatch/re-init message
- [x] 1.8 Unit tests: fresh v8 schema continues to expose the landed artefacts; pre-v8 DB rejected at open; foreign-key reference on `superseded_by` enforced; CHECK constraints on `extraction_queue.trigger_kind` and `.status` enforced

## 2. ADD-only supersede chain — page-level support

> **Truth note (Mom, 2026-05-04T07:22:12.881+08:00):** Professor's slice-2 rejection is now repaired for `2.3` and `2.5`: Unix write-through preflights `supersedes` before sentinel/tempfile/rename work starts, and tests now prove a rejected non-head supersede leaves the vault and active raw source bytes unchanged while returning the typed supersede conflict.
>
> **Truth note (Bender, 2026-05-04T07:22:12.881+08:00):** The remaining concurrent-contender hole in `2.2`/`2.3` is now closed in `src/commands/put.rs`: different successor slugs must stage the successor row and claim the current head inside the same still-open write transaction before sentinel/tempfile/rename work begins, while the later transactional reconcile remains as a race backstop. The new deterministic concurrent proof blocks the winner mid-claim and verifies the loser returns `SupersedeConflictError` without creating vault bytes, activating raw-import ownership, or leaving recovery-sentinel residue.

- [x] 2.1 Keep `superseded_by: Option<i64>` on `Page` (or equivalent) in `src/core/types.rs` as landed baseline plumbing for the remaining supersede work
- [x] 2.2 Update page write/upsert paths so a write with frontmatter `supersedes: <slug>` resolves the prior slug to its page id, sets the new page's row, and updates the prior page's `superseded_by` atomically (single transaction)
- [x] 2.3 Reject writes that attempt to supersede an already-superseded (non-head) page; return a typed error to the caller
- [x] 2.4 Update `src/core/migrate.rs` import path to round-trip `superseded_by` correctly via frontmatter `supersedes`
- [x] 2.5 Unit tests: head/non-head invariants, atomic two-end update, rejection of non-head supersede, multi-step chain (A → B → C) integrity, frontmatter round-trip

## 3. Head-only retrieval default + `include_superseded` opt-in

> **Truth note (Bender, 2026-05-04T07:22:12.881+08:00):** The landed `3.5` `memory_get` seam preserves the stored page UUID in returned `frontmatter.quaid_id` even when a later update omits any frontmatter UUID, matching rendered page output instead of leaking the sparse stored frontmatter map.

- [x] 3.1 Add `include_superseded: bool` (default `false`) to `hybrid_search` in `src/core/search.rs`; apply `superseded_by IS NULL` predicate when not set
- [x] 3.2 Add the same parameter and predicate to `progressive_retrieve` in `src/core/progressive.rs`, applied before token-budget expansion
- [x] 3.3 Plumb `include_superseded` through `memory_search` and `memory_query` MCP tools in `src/mcp/server.rs`
- [x] 3.4 Add `--include-superseded` flag to `quaid search` (`src/commands/search.rs`) and `quaid query` (`src/commands/query.rs`)
- [x] 3.5 Update `memory_get` to return the page regardless of head status and include `superseded_by: <slug-or-null>` in the response
- [x] 3.6 Update `memory_graph` to expose `superseded_by` as a navigable edge type distinct from `links`-table edges
- [x] 3.7 Tests: head-only default in `tests/supersede_chain.rs` (search, query, progressive, graph); `--include-superseded` exposes chain; `memory_get` returns non-head with successor pointer

## 4. Conversation file format

- [ ] 4.1 Add `Turn`, `ConversationFile` types to `src/core/types.rs` (frontmatter struct + ordered turn blocks)
- [ ] 4.2 Implement `src/core/conversation/format.rs` with `parse(path) -> ConversationFile`, `render(file) -> String`, and a turn-block round-trip helper
- [ ] 4.3 Define the canonical render shape: frontmatter (`type`, `session_id`, `date`, `started_at`, `status`, `last_extracted_at`, `last_extracted_turn`) + turn blocks (`## Turn N · role · timestamp` with optional metadata fence)
- [ ] 4.4 Implement multi-day continuation: given a `session_id` and a new turn timestamp, locate the most recent prior day-file (if any) and compute the next ordinal as `MAX(ordinal across all day-files) + 1`
- [ ] 4.5 Namespace-aware path resolution: `<vault>/<namespace>/conversations/<YYYY-MM-DD>/<session-id>.md` when namespaces are in use
- [ ] 4.6 Unit tests: parse-render round-trip, frontmatter cursor preservation, ordinal continuation across day-files, namespace path nesting, malformed turn block produces actionable parse error

## 5. Turn writer (`memory_add_turn` request path)

- [ ] 5.1 Implement `src/core/conversation/turn_writer.rs::append_turn(session_id, role, content, timestamp, metadata, namespace?) -> Result<TurnWriteResult>`
- [ ] 5.2 The append: locate or create the target day-file, compute the next ordinal, render the new turn block, append + fsync, return the assigned ordinal and conversation path
- [ ] 5.3 On first turn for a new session, create the file with full conversation frontmatter (`status: open`, `started_at: <turn timestamp>`, `last_extracted_turn: 0`)
- [ ] 5.4 Treat `metadata` as an opaque object preserved verbatim in the rendered turn block
- [ ] 5.5 Property tests: durability (the appended turn is observable on disk before the function returns); concurrent appends to different sessions do not interfere; concurrent appends to the same session serialise (single-writer file lock or equivalent)

## 6. Extraction queue (storage + enqueue)

- [ ] 6.1 Implement `src/core/conversation/queue.rs::enqueue(session_id, conversation_path, trigger_kind, scheduled_for) -> Result<()>` with UPSERT-collapse semantics on `(session_id, status='pending')`
- [ ] 6.2 Encode the precedence rules: `session_close` overrides any later `debounce` (collapses to earlier `scheduled_for` and `trigger_kind = 'session_close'`); `debounce` extends `scheduled_for` forward but does not override `session_close`
- [ ] 6.3 Implement `dequeue() -> Option<Job>` that selects the earliest `pending` row with `scheduled_for <= now()`, atomically transitions to `running`, and returns the job — safe under concurrent dequeues
- [ ] 6.4 Implement `mark_done(job_id)`, `mark_failed(job_id, err)` with `attempts` accounting and the `extraction.max_retries` cap (default 3) before transitioning to `failed`
- [ ] 6.5 Implement lease expiry: `running` rows whose `scheduled_for + lease_expiry_seconds` has passed (default 300s) become re-eligible for dequeue with `attempts += 1`
- [ ] 6.6 Tests: UPSERT-collapse under burst, `session_close` precedence, scheduled_for ordering on dequeue, concurrent-dequeue safety, retry/fail transitions, lease expiry recovery, persistence across simulated daemon restart

## 7. `memory_add_turn` MCP tool

- [ ] 7.1 Register `memory_add_turn` in `src/mcp/server.rs` with input schema `{session_id, role, content, timestamp?, metadata?}` and output `{turn_id, conversation_path, extraction_scheduled_at}`
- [ ] 7.2 Wire the tool to `turn_writer::append_turn` followed by `queue::enqueue(..., 'debounce', now + extraction.debounce_ms)` when `extraction.enabled = true`
- [ ] 7.3 When `extraction.enabled = false`, skip enqueue and return `extraction_scheduled_at: null`
- [ ] 7.4 Map errors: write conflicts on closed sessions return `ConflictError`; unwritable vault returns `ConfigError`
- [ ] 7.5 Latency test (`tests/turn_latency.rs`): p95 < 50 ms for 100 sequential calls on representative SSD hardware
- [ ] 7.6 End-to-end test: call `memory_add_turn` three times for a new session; verify file created, three turn blocks present in order, queue contains exactly one collapsed `pending` row

## 8. `memory_close_session` MCP tool

- [ ] 8.1 Register `memory_close_session` with input `{session_id}` and output `{closed_at, extraction_triggered, queue_position}`
- [ ] 8.2 Locate the most recent day-file for the session in the active namespace; update its frontmatter `status` to `closed` and persist
- [ ] 8.3 Enqueue an immediate `session_close` job (`scheduled_for = now`) so any debounced job is overridden
- [ ] 8.4 Idempotent re-close: if `status` is already `closed`, return the original `closed_at` without modifying the file
- [ ] 8.5 Return `NotFoundError` for unknown `session_id` in the active namespace
- [ ] 8.6 Tests: close transitions status, immediate enqueue overrides debounce, idempotent re-close returns original timestamp, unknown session returns NotFoundError

## 9. `memory_close_action` MCP tool

- [ ] 9.1 Register `memory_close_action` with input `{slug, status, note?}` and output `{updated_at, version}`
- [ ] 9.2 Validate the target page is `type: action_item`; otherwise return `KindError`
- [ ] 9.3 Update the page's frontmatter `status` in place using the existing optimistic-concurrency machinery (read version, write with `expected_version`, return `ConflictError` on mismatch)
- [ ] 9.4 If `note` is provided, append it to the page body
- [ ] 9.5 Tests: open → done transition increments version, KindError on non-action_item, ConflictError on concurrent writer

## 10. File-edit-aware supersede handler

- [ ] 10.1 Implement `src/core/conversation/file_edit.rs::handle_extracted_edit(prior_page, new_content) -> Result<()>` that is invoked from the existing vault-watcher hook when a content-hash change is observed for a file under `<vault>/extracted/**/*.md`
- [ ] 10.2 The handler: write the prior version as a new page with slug `<original-slug>--archived-<timestamp>`, `superseded_by = <new-page-id>`, content equal to prior file content, frontmatter equal to prior frontmatter
- [ ] 10.3 The new (edited) page becomes a head with frontmatter `supersedes: <archived_slug>` and `corrected_via: file_edit`
- [ ] 10.4 No-op on whitespace-only changes (compare normalized content hash)
- [ ] 10.5 Restrict to `type ∈ {decision, preference, fact, action_item}`; edits to other page types (notes, conversations, etc.) bypass the handler and use the regular vault-sync path
- [ ] 10.6 When `corrections.history_on_disk = true`, write the archived content to `<vault>/extracted/_history/<original-slug>--<timestamp>.md`
- [ ] 10.7 Tests (`tests/file_edit_supersede.rs`): edit produces archive + new head with chain pointers; whitespace-only edit is a no-op; non-extracted edit bypasses; opt-in disk history writes the file; archived page is non-head and recoverable via `--include-superseded`

## 11. Vault layout configuration

- [ ] 11.1 Add `memory.location` config key with values `vault-subdir` (default) and `dedicated-collection`
- [ ] 11.2 Path resolver picks the conversation/extracted root based on the config value and the active namespace
- [ ] 11.3 When `dedicated-collection` is selected, ensure the collection exists (create on first use) and writes go to it instead of the user's main vault
- [ ] 11.4 Tests: subdir mode writes to main vault; dedicated-collection mode writes to the configured collection; both modes honour namespace nesting

## 12. End-to-end integration tests

- [ ] 12.1 `tests/conversation_turn_capture.rs`: full flow — add turn, verify file, verify queue, verify ingestion
- [ ] 12.2 Multi-day session test: add turns spanning midnight; verify two day-files exist with continuing ordinals and independent cursors
- [ ] 12.3 Namespace isolation test: add turns under two namespaces with the same session_id; verify two distinct files and no cross-namespace bleed
- [ ] 12.4 Supersede chain test: write A, write B with `supersedes: A`, write C with `supersedes: B`; verify chain is walkable via `memory_graph` and head-only retrieval returns only C
- [ ] 12.5 File-edit supersede test: extract path manually creates a fact page (since extraction worker lands in proposal #2, this test simulates by writing a fact via existing page-write surfaces); user edits the file; verify chain integrity
