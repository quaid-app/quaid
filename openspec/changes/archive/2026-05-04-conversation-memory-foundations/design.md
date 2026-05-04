## Context

This proposal lands the foundations layer for Phase 5 of `docs/roadmap_v3.md`. The full Phase 5 design is captured in `docs/superpowers/specs/2026-05-03-phase5-conversation-memory-design.md`; this proposal carves out the parts that can ship without an SLM. The follow-on proposal (`slm-extraction-and-correction`) layers Phi-3.5 extraction, fact writing, and the `memory_correct` correction dialogue on top of these foundations.

Current state shaping this change:

- Phase 4 already provides multi-collection vault sync with a live filesystem watcher (`#137` namespace isolation already shipped). Conversation files and extracted-fact files can ride the existing watcher rather than introducing a new one.
- The `pages` table is the single page store. Existing page types (for example `project`) are unaffected; this change adds `conversation`, `decision`, `preference`, `fact`, and `action_item` to the planned surface, with this proposal only writing `conversation` pages directly. Fact pages are produced in proposal #2.
- The repo is already on schema v8. The landed first slice from this change already added `pages.superseded_by`, `idx_pages_supersede_head` on `pages.type`, the guarded `idx_pages_session`, `extraction_queue`, config defaults, and `Page.superseded_by`; remaining work starts at task `2.2` and does not introduce another schema-version bump.
- The repo's no-auto-migration policy applies. The product is pre-release; existing dev databases re-init under the standard schema-mismatch message.
- LoCoMo and LongMemEval are at 0.1% / 0.0% today. The benchmark lift bet is in proposal #2 (the SLM does the actual fact extraction); this proposal only ensures the input plumbing and storage shape can support it.

## Goals / Non-Goals

**Goals:**

- Accept conversation turns from any caller through `memory_add_turn`, with synchronous response and a request-path latency budget that keeps p95 < 50 ms.
- Define a markdown-on-disk conversation file format that Obsidian users can read and edit naturally (per the brainstorm decision: turns live in the vault, not in a SQL-only `conversation_turns` table).
- Provide an extraction job queue with UPSERT-collapse semantics so a flurry of turn-adds debounces to a single pending job per session.
- Introduce an ADD-only supersede chain that all four fact page types (and any other page type) can use, plus the head-only retrieval default that makes "latest in chain" the natural answer.
- Make user edits to extracted-fact files in Obsidian preserve history rather than silently overwrite the prior page.
- Cleanly defer Phi-3.5, the extraction worker, fact-page writing, and the `memory_correct` correction dialogue to proposal #2.

**Non-Goals:**

- The extraction worker itself, the SLM prompt, JSON parsing, dedup-vs-supersede-vs-coexist resolution, fact-page writing. All in proposal #2.
- Phi-3.5 download lifecycle, `quaid extraction enable`, `quaid model pull`. All in proposal #2.
- The `memory_correct` and `memory_correct_continue` MCP tools and the `correction_sessions` table. All in proposal #2.
- Cross-namespace fact recall. Namespace isolation is a hard product boundary by design (`#137`).
- Synchronous fact extraction on the request path. Deliberate choice — `memory_close_session` exists precisely so a caller that wants "extract before I move on" can force a flush.
- A further schema-version bump beyond the already-landed v8 baseline. Remaining work in this change rides the current v8 schema.
- DAB §8 Conversation Memory benchmark gate. Tracked under proposal #2.

## Decisions

### Decision 1 — Turns are markdown files in the vault, not a SQL-only table.

The brainstorm explicitly chose this shape: turns are appended to per-session markdown files under `<vault>/conversations/YYYY-MM-DD/<session-id>.md`, ingested by the existing Phase 4 watcher into the `pages` table. The DB is the derived index; the filesystem is the source of truth. Trade-off: marginally higher write cost (file append + fsync vs single SQL insert) in exchange for portability, Obsidian compatibility, and trivial re-extraction (re-read the file, re-run the SLM). The p95 < 50 ms latency budget is comfortably achievable on SSD.

### Decision 2 — One file per session per day, with turn ordinals continuing across day-files.

The vault grouping is `<YYYY-MM-DD>/<session-id>.md`. Sessions that span midnight produce one file per day. Turn ordinals continue (turn 47 on day 1, turn 48 on day 2 in a different file) — they do not restart at 1. Per-file cursors (`last_extracted_turn` in frontmatter) are independent; the canonical "what's been extracted across the whole session" is `MAX(last_extracted_turn)` over the session's day-files.

Why per-day files at all: bounded file size (long-running multi-day sessions don't grow into one massive file), natural alignment with Obsidian daily-notes patterns, and simpler concurrency story (no two agents racing to append to the same file across days). Why continuing ordinals: turn IDs need to be globally unique within a session for source-turn references on extracted facts to be unambiguous.

Trade-off: extraction's lookback context window does not naturally cross day boundaries. We accept this for v1; cross-day "yes, that decision still stands" references are rare in practice and Phase 6 entity extraction will compensate when it lands.

### Decision 3 — Extraction queue UPSERTs collapse to one pending row per session.

The queue's enqueue operation is keyed on `(session_id, status='pending')`. A burst of `memory_add_turn` calls yields one queue row whose `scheduled_for` is pushed forward each time, debouncing naturally. A `session_close` enqueue overrides any later `debounce` by collapsing onto the same pending row with the earlier `scheduled_for` and the `session_close` `trigger_kind`. Concurrent `running` rows for the same session can coexist with new `pending` rows (a new burst doesn't have to wait for an already-running extraction to finish before being scheduled).

Trade-off considered: a separate `pending_turns` log per session that the worker drains. Rejected — adds a second source-of-truth and double-bookkeeping for what amounts to "is there extraction work pending for this session." The single-row-per-session collapse is enough.

### Decision 4 — ADD-only supersede chain via a `superseded_by` column on `pages`.

The chain lives in the `pages` table itself: a nullable `superseded_by INTEGER REFERENCES pages(id)`. A head page has `superseded_by IS NULL`; a non-head page points at its successor. New pages that supersede an existing page set their `supersedes` frontmatter and update the prior page's `superseded_by`. No new `fact_versions` table, no per-page version side-table.

Why this shape: reuses an indexed scalar lookup that's free at query time (one partial index, `idx_pages_supersede_head`), and a recursive CTE handles "walk the chain". A separate version table would duplicate page rows for marginal benefit. Mutating in place was rejected during brainstorm because it loses the temporal data LoCoMo-style queries need.

Type-specific structured frontmatter keys (`about`, `chose`, `what`) — defined in proposal #2's fact-writing surface — drive supersede detection. This proposal makes the chain itself work for any page type; proposal #2 wires extraction into it.

### Decision 5 — `memory_close_action` is the only in-place mutation on fact page types.

Action items have lifecycle (`open` → `done` / `cancelled`), not just supersede. Modelling lifecycle as a supersede chain ("status: open" → "status: done" → "status: cancelled") would be confusing and bloat the chain with mechanical state changes. `memory_close_action` is therefore special: it updates `status` in place via the existing optimistic-concurrency path and bumps `version`. Direct file edits to `extracted/action-items/*.md` still go through file-edit-aware supersede (Decision 7), but `memory_close_action` is the supported programmatic path for routine `open → done` transitions on `type: action_item` pages.

### Decision 6 — Head-only is the retrieval default; opt into history.

`hybrid_search`, `progressive_retrieve`, `memory_search`, `memory_query`, the `quaid search` CLI, and the `quaid query` CLI all default to filtering by `superseded_by IS NULL`. Each surface accepts `include_superseded` (default `false`); when true, both heads and non-heads are eligible. `memory_get` is unfiltered (raw fetch by slug regardless of head status) and exposes `superseded_by` in its response so callers can detect non-head pages and walk the chain.

Why head-only as default: the most common LoCoMo-style query is "what is currently true," not "what was true at some past point." Forcing every caller to pass `include_superseded: false` would be tedious and error-prone. The explicit `--include-superseded` opt-in handles audit, debugging, and historical queries cleanly.

### Decision 7 — File-edit-aware supersede preserves history on Obsidian edits.

The vault watcher already fires on file changes. We add a small handler for changes under `<vault>/extracted/**/*.md` (or its namespace-scoped equivalent): on a content-hash change, write the prior version as a new archived page with `superseded_by` pointing at the edited file's page, slug `<original-slug>--archived-<timestamp>`. The edited file becomes the new head with `supersedes: <archived_slug>` and `corrected_via: file_edit`.

Why this is non-optional: without it, every Obsidian user who fixes a typo silently overwrites the prior page (vault sync updates in place, version bumps, and the original is gone). That's exactly the corruption the ADD-only model is meant to prevent. The handler closes that gap so the supersede chain remains canonical regardless of how the correction arrived.

By default, the archived page lives only in the database (not on disk) to keep `extracted/` clean. Power users who want disk-level history flip `corrections.history_on_disk = true`, which writes the archive content to `<vault>/extracted/_history/<slug>--<timestamp>.md`.

Whitespace-only edits are a no-op so opening, viewing, and re-saving a file in Obsidian doesn't trigger spurious archives.

### Decision 8 — `memory.location = vault-subdir` is the default; `dedicated-collection` is an opt-in.

The brainstorm chose subdirectories in the user's main vault as the default — natural for Obsidian users, supports linking from extracted facts to existing notes, doesn't require a second vault to be set up at install time. `dedicated-collection` is preserved as a config alternative for users who want agent memory isolated from their notes vault. In this wave, the shipped resolver and tests use that choice for conversation-file placement only; extracted-fact root routing remains follow-on work. Multi-collection support (Phase 4) still makes both eventual layouts straightforward; the only schema implication already landed here is that this proposal records the choice in the `config` table.

### Decision 9 — Remaining work rides the landed v8 schema baseline.

Pre-release product, no users to migrate, existing no-auto-migration policy. The first plumbing slice of this change is already present in the live repo at schema v8: `pages.superseded_by`, `idx_pages_supersede_head` on `pages.type`, the guarded `idx_pages_session`, `extraction_queue` + `idx_extraction_queue_pending`, config defaults, and `Page.superseded_by`. Remaining work in this proposal keeps that v8 shape intact rather than bumping the schema again. Pre-v8 dev databases are still rejected with the standard schema-mismatch error and must be re-initialised.

## Risks / Trade-offs

| Risk | Mitigation |
|---|---|
| File append + fsync exceeds the 50 ms p95 budget on slow disks. | Latency budget is documented as "representative SSD"; spinning-disk users get a slower experience. We accept this. The async path (queue + extraction) absorbs any extraction-side cost so only the fsync sits on the request path. |
| Session-id collision across distinct contexts merges unrelated turns into one file. | Session ids are namespace-local; we document caller responsibility for uniqueness. We don't enforce — that's the caller's job. |
| Multi-day session lookback window doesn't cross day boundaries. | Documented limitation. Cross-day "as we discussed yesterday" references are rare in practice. Phase 6 entity extraction will partially compensate. |
| User edits a conversation file (not an extracted fact) and breaks frontmatter parsing. | Vault sync already handles parse errors gracefully; the watcher logs and skips. The cursor is recalculated on next successful parse. |
| Pre-v8 dev DBs still require re-init against the landed v8 baseline. | Pre-release policy; documented. The schema-mismatch error message already includes re-init guidance. |
| `extraction_queue` grows unbounded over time. | A janitor in `quaid serve` (introduced under proposal #2 alongside the worker) prunes `done` rows older than N days. For this proposal we ship the table; pruning lands with the worker. |
| File-edit-aware supersede misfires on whitespace edits. | Whitespace-only edits are explicitly a no-op (Decision 7). The hash check uses normalized content. |

## Migration Plan

This change now builds on an already-landed v8 baseline, so there is no new schema-version migration step in the remaining work:

1. Keep `SCHEMA_VERSION` and `quaid_config.schema_version` at 8 in `src/core/db.rs`.
2. Keep `src/schema.sql` aligned with the landed v8 conversation-memory baseline: `pages.superseded_by`, `idx_pages_supersede_head` on `pages.type`, the guarded `idx_pages_session`, `extraction_queue`, and the related config defaults.
3. Existing pre-v8 dev databases fail at open time with the standard schema-mismatch message, which already includes the `quaid init` + `quaid import` re-init recipe.
4. There is no backfill step in the remaining work: rows already written under v8 stay as-is, and older dev databases still re-init instead of migrating.
5. Rollback of the remaining work does not change the version boundary; any future schema bump would need its own proposal.

## Open Questions

None. The brainstorm covered the v3 design questions and the resulting spec doc resolves them.
