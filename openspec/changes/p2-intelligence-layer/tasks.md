---
change: p2-intelligence-layer
owner: fry
branch: phase2/p2-intelligence-layer
reviewers: [professor, nibbler, mom, bender]
depends_on: p1-core-storage-cli (gate passed)
---

# Phase 2 Tasks

All tasks execute on branch `phase2/p2-intelligence-layer`.
Run `cargo test` after every group. `cargo clippy -- -D warnings` and `cargo fmt --check` must
stay clean throughout. Do NOT start until Professor and Nibbler have signed off the Phase 1 gate.

OCC on `brain_put` is already complete — do not re-implement.

---

## Group 1 — Graph Core (`src/core/graph.rs`)

- [x] 1.1  Define `TemporalFilter` enum (`Active`, `All`) and `GraphNode` / `GraphEdge` / `GraphResult` structs in `src/core/graph.rs`. Derive `Serialize` on all output types.
- [x] 1.2  Implement `neighborhood_graph(slug: &str, depth: u32, filter: TemporalFilter, conn: &Connection) -> Result<GraphResult, GraphError>` using iterative BFS with `HashSet<i64>` visited set. Hard-cap depth at 10. **Contract: outbound links only** — inbound reachability is the domain of `gbrain backlinks`.
- [x] 1.3  Add temporal filter SQL: `Active` clause `WHERE (l.valid_from IS NULL OR l.valid_from <= date('now')) AND (l.valid_until IS NULL OR l.valid_until >= date('now'))` — excludes both past-closed and future links; `TemporalFilter::All` omits the clause.
- [x] 1.4  Add `GraphError` enum (`PageNotFound`, `Sqlite(rusqlite::Error)`) using `thiserror`. Ensure `PageNotFound` is returned when the root slug doesn't exist.
- [x] 1.5  Write unit tests: zero-hop returns root only; single-hop returns direct outbound neighbours; cycle between A↔B terminates without infinite loop; temporal Active filter excludes past-closed links; temporal Active filter excludes future-dated links; `All` filter includes past-closed links; unknown slug returns `PageNotFound`.

## Group 2 — Graph CLI (`src/commands/graph.rs`)

- [x] 2.1  Implement `run(db: &Connection, slug: &str, depth: u32, temporal: &str, json: bool) -> Result<()>` calling `core::graph::neighborhood_graph`. Map `temporal` string to `TemporalFilter` (default `"active"`).
- [x] 2.2  Human-readable output: print root slug, then indented lines `  → <to_slug> (<relationship>)` for each **outbound** edge. Root can never appear as its own neighbour.
- [x] 2.3  JSON output: emit `{"nodes": [...], "edges": [...]}` via `serde_json::to_string_pretty`.
- [x] 2.4  Wire `graph::run` into `src/main.rs` dispatch (currently has a `todo!`).
- [x] 2.5  Write CLI integration tests: human output asserts root slug present and correct `→ <to> (<rel>)` format via `run_to` writer capture; JSON output asserts valid JSON with `nodes` and `edges` keys plus correct output shapes via `run_to`; unknown slug exits with non-zero code; temporal `all` flag includes closed links.

## Group 3 — Assertions Core (`src/core/assertions.rs`)

- [x] 3.1  Define `Triple { subject, predicate, object }` and `AssertionError` in `src/core/assertions.rs`.
- [x] 3.2  Implement `extract_assertions(page: &Page, conn: &Connection) -> Result<usize, AssertionError>`: DELETE existing assertions for `page.id`, apply regex patterns over `compiled_truth` sentences, INSERT new `assertions` rows with `confidence = 0.8`, `asserted_by = 'agent'`. Return count of inserted rows.
- [x] 3.3  Implement at least 3 regex patterns: "X works at Y", "X is a Y", "X founded Y". Document patterns with inline comments.
- [x] 3.4  Implement `check_assertions(slug: &str, conn: &Connection) -> Result<Vec<Contradiction>, AssertionError>`: query assertions for the page plus assertions sharing the same `subject`; detect (subject, predicate) pairs with 2+ different objects and overlapping validity; insert into `contradictions` table if no unresolved row exists for that pair; return the new contradiction rows.
- [x] 3.5  Write unit tests: extract_assertions inserts expected triples; re-indexing replaces prior triples; page with no patterns inserts zero rows; check_assertions detects same-page conflict; check_assertions detects cross-page conflict; resolved contradiction is not duplicated.

## Group 4 — Check CLI (`src/commands/check.rs`)

- [x] 4.1  Implement `run(db: &Connection, slug: Option<String>, all: bool, check_type: Option<String>, json: bool) -> Result<()>`. When `all`, iterate all pages via `SELECT slug FROM pages`; for each, call `extract_assertions` then `check_assertions`.
- [x] 4.2  Human-readable output: one line per contradiction `[slug] ↔ [other_slug]: <description>`. Summary line: "N contradiction(s) found."
- [x] 4.3  JSON output: array of objects `{ page_slug, other_page_slug, type, description, detected_at }`.
- [x] 4.4  Wire `check::run` into `src/main.rs` dispatch (currently has a `todo!`).
- [x] 4.5  Write tests: single-page check finds existing contradiction; `--all` processes multiple pages; JSON is valid; `--slug` for non-existent page returns error.

## Group 5 — Progressive Retrieval (`src/core/progressive.rs`)

- [ ] 5.1  Implement `progressive_retrieve(initial: Vec<SearchResult>, budget: usize, depth: u32, conn: &Connection) -> Result<Vec<SearchResult>, SearchError>`. Token approximation: `len(page.compiled_truth) / 4`. Hard cap depth at 3. Dedup by slug using `HashSet<String>`.
- [ ] 5.2  Expansion loop: for each result in the current frontier (starting with `initial`), fetch the page's outbound links, retrieve linked pages, add to result list if budget permits.
- [ ] 5.3  Read `default_token_budget` from the `config` table in `query.rs` and pass it to `progressive_retrieve` when `--depth auto` is specified.
- [ ] 5.4  Add `--depth` arg to CLI `gbrain query` (already has a placeholder clap arg with `/// Phase 2: deferred` comment — remove the comment and wire it).
- [ ] 5.5  Add `depth` field to `BrainQueryInput` MCP struct (optional string, `"auto"` triggers expansion).
- [ ] 5.6  Write unit tests: budget exhausted before depth cap stops expansion; depth cap stops expansion before budget; empty initial returns empty; duplicates from expansion are deduplicated; zero depth returns initial results unchanged.

## Group 6 — Novelty Check Wiring (`src/commands/ingest.rs`)

- [ ] 6.1  In `ingest.rs`, after resolving the slug and before the `INSERT ... ON CONFLICT` upsert, check if the page exists. If it does and `--force` is false, call `check_novelty(new_content, &existing_page, conn)`. If `Ok(false)`, print to stderr "Skipping ingest: content not novel (slug: <slug>)" and return `Ok(())`.
- [ ] 6.2  Remove `#![allow(dead_code)]` from `src/core/novelty.rs`.
- [ ] 6.3  Write tests: near-duplicate content (Jaccard ≥ 0.85) is skipped; distinct content proceeds; `--force` bypasses the check; first-time ingest (no prior page) skips the novelty check.

## Group 7 — Palace Room Filtering (`src/core/palace.rs`)

- [ ] 7.1  Replace the stub body of `derive_room(content: &str) -> String` with: find the first line matching `^## (.+)`, lowercase it, replace spaces with hyphens, strip non-`[a-z0-9-]` characters. Return `""` if no `##` heading found.
- [ ] 7.2  Remove the `#![allow(dead_code)]` attribute from `palace.rs` if still present.
- [ ] 7.3  Update `src/commands/put.rs`, `src/commands/ingest.rs`, and `src/mcp/server.rs` (brain_put handler) to pass `derive_room(&compiled_truth)` instead of the current `palace::derive_room(&compiled_truth)` stub (no call-site change needed — verify the result is actually non-empty for headed content).
- [ ] 7.4  Write tests: h2 heading produces kebab-case room; no heading returns `""`; heading with special characters is cleaned; second h2 heading is ignored.

## Group 8 — Knowledge Gaps (`src/core/gaps.rs` + `src/commands/gaps.rs`)

- [ ] 8.1  Implement `log_gap(query: &str, context: &str, confidence_score: Option<f64>, conn: &Connection) -> Result<(), GapsError>` in `src/core/gaps.rs`: insert into `knowledge_gaps` with `query_hash = sha256_hex(query)`, `sensitivity = 'internal'`, `query_text = NULL`. Use `INSERT OR IGNORE` to be idempotent on the same query hash.
- [ ] 8.2  Implement `list_gaps(resolved: bool, limit: usize, conn: &Connection) -> Result<Vec<KnowledgeGap>, GapsError>`.
- [ ] 8.3  Implement `resolve_gap(id: i64, resolved_by_slug: &str, conn: &Connection) -> Result<(), GapsError>`.
- [ ] 8.4  In `src/commands/query.rs` and `src/mcp/server.rs` (brain_query handler), after `hybrid_search`, if `results.len() < 2` or all scores < 0.3, call `log_gap`. On success print to stderr "Knowledge gap logged."
- [ ] 8.5  Implement `run(db: &Connection, limit: u32, resolved: bool, json: bool) -> Result<()>` in `src/commands/gaps.rs` calling `core::gaps::list_gaps`.
- [ ] 8.6  Wire `gaps::run` into `src/main.rs` dispatch (currently has a `todo!`).
- [ ] 8.7  Write tests: `log_gap` inserts a row; duplicate query is idempotent; `list_gaps` returns only unresolved by default; `resolve_gap` sets resolved_at; low-result query auto-logs gap.

## Group 9 — MCP Phase 2 Write Surface (`src/mcp/server.rs`)

- [ ] 9.1  Add `BrainLinkInput` struct and `brain_link` tool method. Delegate to `commands::link::run`. Map anyhow errors to `ErrorCode(-32001)` for page-not-found, `-32003` for other db errors.
- [ ] 9.2  Add `BrainLinkCloseInput` struct and `brain_link_close` tool method. Delegate to `commands::link::close`. Return `-32001` if link not found.
- [ ] 9.3  Add `BrainBacklinksInput` struct and `brain_backlinks` tool method. Delegate to `commands::link::backlinks`. Return JSON array. Validate slug with `validate_slug`.
- [ ] 9.4  Add `BrainGraphInput` struct and `brain_graph` tool method. Delegate to `core::graph::neighborhood_graph`. Validate slug. Return JSON `{"nodes": [...], "edges": [...]}`. Cap depth at 10.
- [ ] 9.5  Add `BrainCheckInput` struct and `brain_check` tool method. Delegate to `core::assertions::check_assertions` (single slug) or iterate all pages (no slug). Return JSON array of contradictions.
- [ ] 9.6  Add `BrainTimelineInput` struct and `brain_timeline` tool method. Delegate to `commands::timeline::run` (JSON mode). Validate slug. Default limit 20, max 1000.
- [ ] 9.7  Add `BrainTagsInput` struct and `brain_tags` tool method. Delegate to `commands::tags`. If both `add` and `remove` are absent, list tags. Return JSON array of current tags after operation.
- [ ] 9.8  Update `get_info_enables_tools_capability` test (or add a new test) to reference the 7 new Phase 2 tool method signatures, confirming they compile and are accessible.
- [ ] 9.9  Write MCP tests: `brain_link` with unknown from_slug returns -32001; `brain_link_close` with unknown id returns -32001; `brain_backlinks` returns link array; `brain_graph` returns nodes+edges JSON; `brain_check` on clean page returns `[]`; `brain_timeline` on unknown slug returns -32001; `brain_tags` list/add/remove round-trip.

## Group 10 — Phase 2 Ship Gate

- [ ] 10.1  `cargo test` — all tests pass (target: 200+ unit tests).
- [ ] 10.2  `cargo clippy -- -D warnings` — zero warnings.
- [ ] 10.3  `cargo fmt --check` — clean.
- [ ] 10.4  Manual smoke test: `gbrain graph people/alice --depth 2`, `gbrain check --all`, `gbrain gaps`, `gbrain query "test" --depth auto`.
- [ ] 10.5  Phase 1 round-trip tests (`tests/roundtrip_semantic.rs`, `tests/roundtrip_raw.rs`) still pass with no regressions.
- [ ] 10.6  Professor review: `src/core/graph.rs` BFS correctness, `src/core/progressive.rs` budget logic, OCC protocol unchanged.
- [ ] 10.7  Nibbler review: MCP Phase 2 write surface adversarial check (link injection, graph depth abuse, contradiction table poisoning).
- [ ] 10.8  Mom review: temporal link edge cases (valid_from > valid_until rejected by schema CHECK, zero-hop graph, null valid_from).
- [ ] 10.9  Bender sign-off: ingest novelty-skip scenario, contradictions round-trip (ingest conflicting pages → `gbrain check` detects).
