## ADDED Requirements

### Requirement: Novelty check wired into ingest pipeline
`src/commands/ingest.rs` SHALL call `check_novelty(new_content, existing_page, conn)` before
writing a page update. If the function returns `Ok(false)` (not novel), ingest SHALL skip
the write, print a warning to stderr, and return `Ok(())`. The `--force` flag SHALL bypass
the novelty check and proceed unconditionally. The `#![allow(dead_code)]` suppression in
`src/core/novelty.rs` SHALL be removed once the wiring is in place.

#### Scenario: Near-duplicate content is silently skipped
- **WHEN** `quaid ingest note.md` is called and the new content is ≥ 85% Jaccard-similar
  to the existing compiled_truth
- **THEN** no database write occurs; stderr shows "Skipping ingest: content not novel (slug: <slug>)"

#### Scenario: Clearly new content proceeds normally
- **WHEN** `quaid ingest note.md` is called and the new content has < 85% Jaccard overlap
- **THEN** the page is upserted as in Phase 1

#### Scenario: --force bypasses novelty check
- **WHEN** `quaid ingest note.md --force` is called with near-duplicate content
- **THEN** the page is upserted without any novelty check warning

#### Scenario: Novelty check is skipped for new pages (no prior content)
- **WHEN** `quaid ingest note.md` is called and no page with that slug exists yet
- **THEN** the novelty check is NOT performed and the page is created normally

### Requirement: Knowledge gaps — log and list
`src/core/gaps.rs` SHALL implement:
- `log_gap(query, context, confidence_score, conn)` — inserts a row into `knowledge_gaps`
  with `query_hash = sha256(query)`, `sensitivity = 'internal'`, `query_text = NULL`.
- `list_gaps(resolved, limit, conn)` — returns gaps filtered by `resolved_at IS NULL`
  (default) or all if `resolved = true`.
- `resolve_gap(id, resolved_by_slug, conn)` — sets `resolved_at` and `resolved_by_slug`.

`memory_query` and `quaid query` SHALL call `log_gap` automatically when `hybrid_search`
returns fewer than 2 results or all scores are below 0.3.

`src/commands/gaps.rs` SHALL implement `run(db, limit, resolved, json)` to list gaps.

#### Scenario: Low-confidence query auto-logs a gap
- **WHEN** `quaid query "who invented quantum socks"` returns 0 results
- **THEN** a row is inserted into `knowledge_gaps` with the query hash; stderr shows
  "Knowledge gap logged"

#### Scenario: gaps command lists unresolved gaps
- **WHEN** `quaid gaps` is called and there are 2 unresolved gaps
- **THEN** stdout shows 2 entries with their query hashes and detected_at timestamps

#### Scenario: gaps --resolved includes resolved gaps
- **WHEN** `quaid gaps --resolved` is called
- **THEN** all gaps (resolved and unresolved) are listed

#### Scenario: Duplicate gap query is idempotent
- **WHEN** the same query triggers gap logging twice
- **THEN** only one gap row exists (ON CONFLICT(query_hash) DO NOTHING or equivalent)
