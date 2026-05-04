## Why

Quaid stores documents but has no way to ingest agent conversation turns or model facts that change over time. Per the v3 roadmap (`docs/roadmap_v3.md`, Phase 5), this is the highest-impact gap on the product — LoCoMo and LongMemEval benchmark scores are 0.1% and 0.0% respectively, while comparable systems like Mem0 v3 sit near 90%. Closing that gap requires conversation ingestion plus structured fact storage with proper supersede semantics. This proposal lands the **foundations / plumbing** for that work — turn capture, conversation files, the extraction job queue, and an ADD-only supersede chain that any page type can use. A follow-on proposal (`slm-extraction-and-correction`) layers Phi-3.5 SLM extraction, fact writing, and the `memory_correct` correction dialogue on top of these foundations. Issues `#105`, `#135`, `#137`.

## What Changes

- Baseline truth: the live repo already carries the first schema/plumbing slice from this change at schema v8 — `pages.superseded_by`, `idx_pages_supersede_head` on `pages.type`, the guarded `idx_pages_session`, `extraction_queue`, the related config defaults, and `Page.superseded_by`. Remaining work in this change starts at task `2.2` and builds on that landed baseline rather than introducing another schema-version bump.
- Add a new `memory_add_turn` MCP tool that accepts conversation turns from any caller, appends them to a per-session markdown file in the vault, and enqueues a debounced extraction job. Synchronous response, target p95 < 50 ms.
- Add `memory_close_session` (force a final extraction flush + mark session closed) and `memory_close_action` (lifecycle update for `type: action_item` pages — the only in-place mutation on the new fact page types).
- Define the conversation file format under `<vault>/conversations/YYYY-MM-DD/<session-id>.md`: frontmatter (`type`, `session_id`, `started_at`, `status`, `last_extracted_turn` cursor) plus turn blocks. Sessions that span midnight produce one file per day with turn ordinals continuing across files.
- Add an `extraction_queue` table for the background extraction job pipeline. UPSERT-collapse on `(session_id, status='pending')` so 20 turns in 10 seconds debounce to a single job. **Storage and enqueue logic only — the worker that runs the SLM is in the follow-on proposal.**
- Introduce an **ADD-only supersede chain** for any page type: new `superseded_by` column on `pages`, partial index `idx_pages_supersede_head` for head-only queries, head-only default applied to `hybrid_search`, `progressive_retrieve`, `memory_search`, `memory_query`, and a new `--include-superseded` flag (CLI) / `include_superseded` parameter (MCP) to expose history when needed. `memory_get` is unfiltered and returns `superseded_by` when applicable. `memory_graph` exposes the chain as a `superseded_by` edge type.
- Add a **file-edit-aware supersede handler** to the existing Phase 4 vault watcher: when a user edits a file under `<vault>/extracted/**/*.md` (or its namespace-scoped equivalent), the prior version is preserved as a new archived page (DB-only by default, written to `<vault>/extracted/_history/` when `corrections.history_on_disk = true`) and the edited file becomes a new head with `supersedes: <archived_id>` and `corrected_via: file_edit`. Without this, every Obsidian edit silently destroys history.
- Default vault layout for Phase 5 output: `memory.location = vault-subdir` routes conversation files under the user's main vault in this wave. `memory.location = dedicated-collection` is supported as an alternative for users who want conversation memory isolated. Namespace-aware (`#137`) — conversation paths nest under the namespace directory when namespaces are in use. Extracted-fact root routing under this config remains follow-on work.
- **BREAKING (pre-release baseline already landed)**: the repo remains on schema v8 for this change. No additional automatic migration is introduced; existing pre-v8 dev databases still re-init under the existing no-auto-migration policy.

## Capabilities

### New Capabilities

- `conversation-turn-capture`: Appending conversation turns to per-session vault files via `memory_add_turn`, the `memory_close_session` and `memory_close_action` tools, the conversation file format (frontmatter cursor + turn blocks), multi-day session continuation, and namespace-aware path layout.
- `extraction-queue`: SQLite-backed background job queue that backs extraction work. Defines the `extraction_queue` schema, UPSERT-collapse enqueue semantics, debounce-versus-immediate trigger kinds, and the queue invariants the worker (delivered in the follow-on proposal) will rely on.
- `add-only-supersede-chain`: Supersede chain semantics for any page type. Defines the `superseded_by` column, head-only default in retrieval, `--include-superseded` exposure, supersede edge type in `memory_graph`, and the file-edit-aware supersede handler that archives prior file versions on user edit.

### Modified Capabilities

None. The retrieval surfaces (`hybrid_search`, `progressive_retrieve`) are not formally specced today — head-only filtering and `--include-superseded` are introduced as part of the new `add-only-supersede-chain` capability rather than as deltas to a non-existent spec, mirroring the precedent in `retrieval-quality-rerank`.

## Impact

- **Code**:
  - `src/core/conversation/turn_writer.rs` (new): append-with-fsync, enqueue.
  - `src/core/conversation/queue.rs` (new): UPSERT-collapse enqueue, dequeue/poll, retry/fail accounting.
  - `src/core/conversation/format.rs` (new): conversation file parser/writer, cursor management, turn-block round-trip.
  - `src/core/conversation/file_edit.rs` (new): vault-watcher hook for `extracted/**/*.md` edits, archive-page write.
  - `src/core/db.rs`: keep the landed v8 baseline aligned while remaining work builds on `superseded_by`, the head/session indices, `extraction_queue`, and config defaults.
  - `src/core/types.rs`: `Turn`, `ConversationFile`, queue-job types; extend `Page` with `superseded_by`.
  - `src/core/search.rs`: head-only predicate in `hybrid_search`; `include_superseded` plumb-through.
  - `src/core/progressive.rs`: same head-only filter applied before token-budget expansion.
  - `src/core/migrate.rs`: import recognizes `type: conversation` and the four fact page types for namespace classification.
  - `src/mcp/server.rs`: register `memory_add_turn`, `memory_close_session`, `memory_close_action`; add `include_superseded` to `memory_search` / `memory_query` / `memory_graph`; expose `superseded_by` in `memory_get` results.
  - `src/commands/search.rs`, `src/commands/query.rs`: `--include-superseded` flag.
  - `src/schema.sql`: embedded DDL updated to v8.
- **Schema**: live baseline stays at v8. The landed first slice already added `pages.superseded_by INTEGER REFERENCES pages(id)`, partial index `idx_pages_supersede_head ON pages(type, superseded_by) WHERE superseded_by IS NULL`, guarded partial index `idx_pages_session ON pages(json_extract(IIF(json_valid(frontmatter), frontmatter, '{}'), '$.session_id')) WHERE json_valid(frontmatter) AND json_extract(IIF(json_valid(frontmatter), frontmatter, '{}'), '$.session_id') IS NOT NULL`, and `extraction_queue` with `idx_extraction_queue_pending`.
- **Config**: New keys in the existing mutable `config` table — `memory.location` (`vault-subdir` default, `dedicated-collection` alternative for conversation-file placement in this wave), `corrections.history_on_disk` (`false` default).
- **Migration**: No new migration in the remaining work. The landed first slice already established v8; existing pre-v8 dev databases re-init under the existing no-auto-migration policy.
- **Tests**:
  - `tests/conversation_turn_capture.rs`: append + fsync invariants, multi-day session continuation, ordinal continuity, frontmatter cursor round-trip, namespace path nesting.
  - `tests/extraction_queue.rs`: UPSERT-collapse semantics, scheduled_for advancement, retry/fail accounting, idempotent re-enqueue, drain order.
  - `tests/supersede_chain.rs`: head-only default, `--include-superseded` exposes chain, multi-step chains query correctly, `memory_graph` includes `superseded_by` edges.
  - `tests/file_edit_supersede.rs`: user edit produces archived breadcrumb + new head, history preserved (DB-only default; `extracted/_history/` when opted in), no-op on whitespace-only edits.
  - `tests/turn_latency.rs`: `memory_add_turn` p95 < 50 ms on representative hardware.
  - Extend roundtrip tests to cover the new `type: conversation` page type.
- **Dependencies**: No new runtime dependencies. Reuses existing `rusqlite`, `serde`, `serde_yaml`, `notify` (for vault watcher).
- **Performance**: `memory_add_turn` request path is one file append + fsync + one queue UPSERT; p95 target < 50 ms on SSD. Head-only retrieval filtering is one indexed predicate; expected zero measurable regression versus the live v8 baseline. File-edit handler runs only when an `extracted/**` file changes; cost is amortised across vault sync's existing change-detection loop.
