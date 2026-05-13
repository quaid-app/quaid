## Wave order (Decision 11 — single derived-edge sync owner)

Implementation must proceed in this order. No wave may start before the previous wave's primitives are in place.

| Wave | Tasks | Owner primitive |
|------|-------|-----------------|
| 0 — Schema + config | 1.x | `src/schema.sql`, `src/core/db.rs` |
| 1 — Frontmatter baseline verification | 2.6 | `src/core/types.rs`, `src/core/markdown.rs` (pre-checked 2.1–2.5) |
| 2 — Edge + tag expansion types | 3.x | `src/core/links.rs` (parsing only, no DB writes) |
| 3 — Derived-edge sync primitives | 4.x | `src/core/links.rs` (`upsert_derived_edge`, `sync_frontmatter_edges`, `sync_wikilink_edges`) |
| 4 — Write-path wiring | 5.x | `commands/put.rs`, `commands/ingest.rs`, `src/core/migrate.rs`, `src/core/vault_sync/mod.rs`, `src/mcp/tools/pages.rs` |
| 5 — Entity-pattern extraction | 6.x, 7.x, 8.x | `src/core/entities.rs` (assertions only — see Decision 11) |
| 6 — Graph-aware retrieval + path output | 9.x, 10.x | `src/core/search.rs`, `src/core/progressive.rs`, `src/core/graph.rs`, `src/mcp/tools/links.rs` |
| 7 — Tests, benchmarks, docs | 11.x, 12.x, 13.x | `tests/`, `docs/` |

## 1. Pre-release schema reset to v10

- [x] 1.1 Update `src/schema.sql`: add `links.edge_weight REAL NOT NULL DEFAULT 1.0`
- [x] 1.2 Update `links.source_kind` CHECK constraint to allow `('wiki_link', 'programmatic', 'frontmatter', 'entity_pattern')`
- [x] 1.3 Add partial unique index `idx_links_unique_derived_edge` on `(from_page_id, to_page_id, relationship, source_kind)` where `source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern')`
- [x] 1.4 Seed graph config defaults in the existing `config` table: `graph_depth='1'`, `graph_distance_decay='0.5'`, `graph_expansion_max='50'`, `edge_weight_frontmatter='1.0'`, `edge_weight_entity_pattern='0.7'`, `edge_weight_wikilink='0.5'`
- [x] 1.5 Bump `SCHEMA_VERSION`, `config.version`, `quaid_config.schema_version`, and schema-version tests to `10` (baseline is v9)
- [x] 1.6 Verify no v9 → v10 migration or rollback path is added; existing v9 DBs must continue to fail with the schema-mismatch/re-init message
- [x] 1.7 Unit tests: fresh v10 schema accepts `frontmatter`/`entity_pattern` in CHECK constraint, rejects invalid `source_kind`, preserves multiple duplicate-key `programmatic` temporal links, and enforces uniqueness for derived sources

## 2. Structured frontmatter parsing and persistence

> **Scope note (Leela, 2026-05-12):** `Frontmatter` is already `pub type Frontmatter = JsonMap<String, JsonValue>` with scalar helpers in `src/core/types.rs`, and `parse_frontmatter()` in `src/core/markdown.rs` already uses `serde_json::to_value`. Tasks 2.1–2.5 are pre-checked as of the v9 baseline. Task 2.6 (unit tests) remains open to verify round-trip correctness against the new derived-edge write paths.

- [x] 2.1 Add `FrontmatterDocument` (or equivalent) in `src/core/markdown.rs` / `src/core/types.rs` that preserves full structured YAML values and exposes scalar helpers
  > **Pre-checked:** `pub type Frontmatter = JsonMap<String, JsonValue>` and helpers already exist in `src/core/types.rs`.
- [x] 2.2 Keep or replace `parse_frontmatter` with a compatibility wrapper; route write paths through the structured parser
  > **Pre-checked:** `parse_frontmatter()` already uses `serde_json::to_value` in `src/core/markdown.rs`.
- [x] 2.3 Store full structured frontmatter JSON in `pages.frontmatter` instead of scalar-only maps
  > **Pre-checked:** `pages.frontmatter` already stores `JsonMap` at v9 baseline.
- [x] 2.4 Update scalar consumers (`slug`, `title`, `type`, `wing`, `memory_id`) to use helper accessors instead of direct `HashMap<String, String>` assumptions
  > **Pre-checked:** `frontmatter_get_str` / `frontmatter_get_string` helpers already in place.
- [x] 2.5 Update `render_page` to render structured frontmatter deterministically, including arrays and objects
  > **Pre-checked:** `render_page` already uses structured frontmatter at v9 baseline.
- [x] 2.6 Unit tests: arrays/objects are not skipped, scalar helpers preserve existing behavior, and structured frontmatter survives import → export → re-import

## 3. Frontmatter edge and tag expansion

- [x] 3.1 Add `FrontmatterLink { target, relationship, valid_from, valid_until }` type
- [x] 3.2 Implement `expand_frontmatter_edges(frontmatter: &FrontmatterDocument) -> Result<Vec<FrontmatterLink>>`
- [x] 3.3 Support canonical `links:` object form, string shorthand, `parent:`, `children:`, and `related:` fixed relationship fields
- [x] 3.4 Reject malformed link entries with an actionable parse error in validate/write paths
- [x] 3.5 Implement `expand_frontmatter_tags(frontmatter: &FrontmatterDocument) -> Vec<String>` supporting YAML lists and comma-separated scalar strings
- [x] 3.6 Unit tests: object-form links, string-form links, mixed lists, temporal fields, parent/children/related, malformed entries, list tags, scalar tags, and tags-not-edges behavior

## 4. Derived edge upsert and sync

- [x] 4.1 Implement `upsert_derived_edge(conn, from_page_id, to_page_id, relationship, source_kind, edge_weight, valid_from, valid_until, context)` for `wiki_link`, `frontmatter`, and `entity_pattern`
- [x] 4.2 Use the partial-index conflict target for derived upserts; update `valid_from`, `valid_until`, `edge_weight`, and `context` on conflict
- [x] 4.3 Implement `sync_frontmatter_edges(conn, page_id, collection_id, edges)` that upserts incoming `frontmatter` edges and deletes stale `frontmatter` rows for the source page
- [x] 4.4 Implement `sync_wikilink_edges(conn, page_id, collection_id, compiled_truth, timeline)` that extracts body `[[slug]]` references, upserts `wiki_link` edges, and deletes stale `wiki_link` rows for the source page
- [x] 4.5 Add unresolved-target logging with de-duplication so repeated writes do not flood `knowledge_gaps`
- [x] 4.6 Unit tests: idempotency, stale deletion, temporal replacement, edge-weight replacement, unresolved target gap logging, and programmatic link history unaffected

## 5. Wire structured side-table sync into write paths

- [ ] 5.1 Wire structured frontmatter persistence and side-table sync into `commands/put.rs`
- [ ] 5.2 Wire structured frontmatter persistence and side-table sync into single-file ingest (`commands/ingest.rs`)
- [ ] 5.3 Wire structured frontmatter persistence and side-table sync into directory import/export (`src/core/migrate.rs` import/export helpers)
- [ ] 5.4 Wire structured frontmatter persistence and side-table sync into vault sync / reconciler write paths (`src/core/vault_sync/mod.rs` — page insert callsites at ~lines 3483, 4255, 4304)
- [ ] 5.5 Wire structured frontmatter persistence and side-table sync into MCP `memory_put` (`src/mcp/tools/pages.rs`)
- [ ] 5.6 Keep frontmatter edges, wikilink edges, tags, raw imports, file state, and embedding jobs transactionally consistent with the page write where practical
- [ ] 5.7 Integration tests: write/import pages with frontmatter links, wikilinks, and tags; verify pages, links, tags, and exports remain in sync across re-ingest

## 6. Entity-pattern config and resolver

- [ ] 6.1 Add `EntityPattern { regex, relationship, subject_type, object_type, weight }` and `EntityMatch` types
- [ ] 6.2 Embed default pattern YAML via `include_str!`; cover `works_at`, `founded`, `invested_in`, `acquired`, and `leads`
- [ ] 6.3 Implement `load_patterns()` for extraction commands: prefer `~/.quaid/entity-patterns.yaml`, fall back to embedded defaults, compile regexes once, reject malformed patterns before page mutation
- [ ] 6.4 Implement role/type hint defaults by relationship, with user pattern hints overriding defaults
- [ ] 6.5 Implement collection-local `resolve_entity_surface(surface, role_hint, source_collection_id, conn)` using exact slug, role-prefixed slug, case-insensitive title, and unique basename strategies
- [ ] 6.6 Unit tests: defaults load, user overrides defaults, malformed YAML/regex fails before mutation, wrong capture-group count rejected, role-prefixed slug resolution, title resolution, basename resolution, and ambiguity returns unresolved

## 7. Entity-pattern extraction and routing

- [ ] 7.1 Implement `extract_entities(page_content, patterns, deadline) -> Vec<EntityMatch>` with a 5 ms page-level deadline checked between patterns
- [ ] 7.2 Record a `knowledge_gap` when extraction exceeds budget and skip remaining patterns for that page
- [ ] 7.3 Implement `route_entity_matches(conn, source_page_id, source_collection_id, matches)`
  > **Scope note (Leela, 2026-05-12 — Nibbler blocker resolved):** Per Decision 11, all entity-pattern matches are routed to `assertions` only in this change. Durable `entity_pattern` edges in `links` require source-page provenance and proven retraction semantics, which are deferred to a follow-on change.
- [ ] 7.4 Route ALL entity-pattern matches (resolved or not) to `assertions` with `asserted_by='agent'`, `confidence=pattern.weight`, and dedup on `(page_id, subject, predicate, object)`; do NOT insert `links` rows with `source_kind='entity_pattern'` in this change
- [ ] 7.5 Route unresolved or ambiguous matches to `assertions` with `asserted_by='agent'`, `confidence=pattern.weight`, and dedup on `(page_id, subject, predicate, object)` (same routing as resolved matches in this change)
- [ ] 7.6 Wire extraction after page writes; failures should not corrupt page writes, and malformed pattern config should fail before page mutation
- [ ] 7.7 Add static/debug validation that `src/core/entities.rs` does not call embedding/inference or network APIs
- [ ] 7.8 Unit tests: budget enforcement, no-LLM/no-inference proof, assertions-only routing (no `entity_pattern` links rows), ambiguity handling, idempotent re-ingest, and pattern weight propagation to assertion confidence

## 8. `quaid graph extract-entities` opt-in backfill command

- [ ] 8.1 Add `extract-entities` subcommand under `quaid graph` without adding automatic schema migration/backfill behavior
- [ ] 8.2 Iterate all pages, run `extract_entities` + `route_entity_matches` per page, and report progress/summary counts
- [ ] 8.3 Integration test: 100-page fixture, command writes expected edges/assertions and is idempotent on re-run

## 9. Graph-aware retrieval

- [ ] 9.1 Add `expand_graph(conn, candidates, depth, max_added, distance_decay) -> Vec<SearchResult>` in `src/core/search.rs` or a shared graph-ranking module
- [ ] 9.2 Expansion walks currently active outbound links only, uses `edge_weight`, applies `distance_decay^hops`, respects max-added and max-visited caps, and deduplicates against initial candidates
- [ ] 9.3 Update `hybrid_search` / `hybrid_search_canonical` to call graph expansion when effective depth > 0
- [ ] 9.4 Update `progressive_retrieve` so graph-expanded candidates participate in token-budget pruning like regular candidates
- [ ] 9.5 Add `--hops N` to `quaid query` and `quaid search`; CLI value overrides `config.graph_depth` for that invocation
- [ ] 9.6 Unit tests: 1-hop expansion, depth bound, active temporal filter, source weight ordering, `graph_expansion_max`, `MAX_NODES`, deduplication, and depth=0 baseline behavior

## 10. Graph path explanations

- [ ] 10.1 Extend `GraphResult` with `paths: HashMap<String, Vec<(String, String, String)>>` keyed by reachable slug
- [ ] 10.2 Update `neighborhood_graph` BFS to track the first path used to reach each node; root path is empty
- [ ] 10.3 Update `memory_graph` output schema in `src/mcp/tools/links.rs`; no compatibility mode required pre-release
- [ ] 10.4 Update `quaid graph` text and JSON rendering to include paths
- [ ] 10.5 Integration test: `quaid graph alice --depth 2` returns expected path triples for a known 2-hop fixture

## 11. Roundtrip and integration tests

- [ ] 11.1 Extend `tests/roundtrip_semantic.rs`: structured frontmatter arrays/objects survive export → re-import with equivalent JSON values and equivalent derived edge sets
- [ ] 11.2 Add `tests/graph_autowire.rs`: covers all scenarios in `specs/frontmatter-link-autowiring/spec.md`
- [ ] 11.3 Add `tests/entity_extraction.rs`: covers all scenarios in `specs/entity-pattern-extraction/spec.md`
- [ ] 11.4 Add `tests/graph_retrieval.rs`: covers all scenarios in `specs/graph-aware-retrieval/spec.md`
- [ ] 11.5 Update existing tests that assume scalar-only `frontmatter` or unrestricted derived duplicate links

## 12. Benchmark gating

- [ ] 12.1 Add a reproducible DAB §4 bge-small baseline measurement task before graph-aware retrieval is enabled by default; record numerics in `docs/benchmarks/`
- [ ] 12.2 Add a post-change DAB §4 measurement task; target ≥ 8 point improvement and ≥ 35/50 score
- [ ] 12.3 Add MSMARCO P@5 baseline + post-change measurement task; target ≥ 5 point improvement
- [ ] 12.4 Wire benchmark checks into release acceptance; if thresholds miss, keep graph expansion disabled by default while retaining autowiring/graph read improvements

## 13. Documentation

- [ ] 13.1 Update `CHANGELOG.md` with v9→v10 pre-release schema reset, no migration path, new config keys, `--hops`, `quaid graph extract-entities`, and graph path output
- [ ] 13.2 Update `CLAUDE.md` / `AGENTS.md` key-file notes for `src/core/entities.rs`, structured frontmatter, and v10 schema
- [ ] 13.3 Add `docs/graph.md` covering frontmatter link syntax, tag behavior, entity-pattern override YAML, graph search knobs, and path explanations
- [ ] 13.4 Update website/reference docs for `memory_graph` response shape and graph config keys
