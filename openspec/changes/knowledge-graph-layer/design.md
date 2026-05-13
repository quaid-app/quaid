## Context

Quaid already exposes graph reads (`memory_graph`, `memory_backlinks`) and has an N-hop BFS in `src/core/graph.rs`. The `links` table supports typed relationships (`relationship`), provenance (`source_kind` ∈ {`wiki_link`, `programmatic`}), and temporal validity (`valid_from`, `valid_until`). What is missing is a durable write path that keeps derived edges in sync with page content.

Current constraints that shape this change:

- `src/core/links.rs::extract_links()` can parse body wikilinks, but it is not wired into the production write paths.
- `src/core/markdown.rs::parse_frontmatter()` returns `JsonMap<String, JsonValue>` (already structured via `serde_json::to_value`) and scalar helpers are in `src/core/types.rs`. This is already implemented as of the v9 baseline; tasks 2.1–2.5 are pre-checked.
- `src/core/search.rs::hybrid_search` is lexical + vector today. `src/core/progressive.rs` can walk outbound links after initial retrieval, but the hybrid ranking layer does not use graph proximity or edge weights.
- Existing schema-migration policy rejects old schema versions. This is pre-release software with no users to migrate, so this change updates the canonical v10 schema directly and does not introduce a v9 → v10 migration path.

## Goals / Non-Goals

**Goals:**

- Preserve structured frontmatter values while keeping ergonomic scalar access for `slug`, `title`, `type`, `wing`, and `memory_id`. (Already implemented as of v9 baseline via `pub type Frontmatter = JsonMap<String, JsonValue>` and helpers in `src/core/types.rs`; tasks 2.1–2.5 are pre-checked.)
- Every page write/import syncs derived edges from frontmatter and body wikilinks without a manual step.
- Frontmatter tags sync to the `tags` table only; tags do not become graph edges in this change.
- Regex entity extraction produces additional `entity_pattern` edges when both endpoints resolve to pages, otherwise text assertions.
- `hybrid_search` and `progressive_retrieve` consult graph proximity when ranking/expanding, with depth and weight knobs exposed via config and CLI flags.
- Re-ingest is idempotent for derived edges and assertions: identical input → identical derived edge/assertion set.
- DAB §4 Semantic improves by ≥ 8 points; MSMARCO P@5 improves by ≥ 5 points, both measured against the bge-small baseline.

**Non-Goals:**

- Automatic schema migration, rollback, or graph backfill for v9 databases. Stale pre-release DBs should be re-initialized and re-imported.
- LLM-based entity extraction. Entity extraction remains regex only under the airgapped-binary rule.
- Inferring relationships across collections or across temporal validity windows. Edges are resolved in the source collection unless explicitly specified otherwise in a later change.
- Graph mutation tools. Existing typed `memory_link` / CLI link commands remain the manual write surfaces.
- Tags-as-soft-edges and per-relationship weights. Both remain follow-up levers if benchmark results plateau.

## Decisions

### Decision 1 — Preserve structured frontmatter as JSON values, not scalar-only strings.

**Why:** Frontmatter is the graph/tag source of truth. The old `HashMap<String, String>` approach silently dropped YAML arrays/objects, making `links: [{target: ...}]`, `children: [...]`, and `tags: [...]` unparseable after initial raw read and breaking export/re-import semantics. The fix is a structured frontmatter representation that stores the full YAML mapping as JSON in `pages.frontmatter`, plus helper accessors for scalar fields used by existing code.

**Status:** Already implemented as of the v9 baseline. `pub type Frontmatter = JsonMap<String, JsonValue>` with `frontmatter_get`, `frontmatter_get_str`, and `frontmatter_get_string` helpers exists in `src/core/types.rs`; `parse_frontmatter()` in `src/core/markdown.rs` already uses `serde_json::to_value`. Tasks 2.1–2.5 are pre-checked.

**Implementation shape:** Add `expand_frontmatter_edges` and `expand_frontmatter_tags` consumers that read the already-structured `Frontmatter` type (tasks 3.x). No changes to `parse_frontmatter()` or `Page.frontmatter` are needed.

### Decision 2 — Extend `links.source_kind`; do not add `edge_source`.

**Why:** The existing `links` table already has a provenance column. We extend the `source_kind` CHECK constraint to include `frontmatter` alongside `wiki_link` and `programmatic`. `entity_pattern` is reserved in the schema for a follow-on change that adds explicit source-page provenance and proven retraction semantics (see Decision 11). A separate `edge_source` column would duplicate state and create ambiguity in readers.

**Schema:** `source_kind TEXT NOT NULL DEFAULT 'programmatic' CHECK(source_kind IN ('wiki_link', 'programmatic', 'frontmatter', 'entity_pattern'))` and `edge_weight REAL NOT NULL DEFAULT 1.0`. The `entity_pattern` value is present in the CHECK constraint for schema completeness but no write path uses it in this change.

### Decision 3 — This is a pre-release schema reset, not a migration.

**Why:** There are no users to migrate. The repo already has a no-auto-migration policy for schema mismatches, and adding migration/rollback machinery would create complexity for a pre-release-only database shape. Update `src/schema.sql` and schema version constants directly from the current v9 baseline to v10. Existing v9 dev DBs fail with the current schema-mismatch message and should be recreated with `quaid init` then repopulated with `quaid import`.

**Consequence:** There is no migration backfill. Existing pages get frontmatter/wikilink edges when re-imported or written under v10.

### Decision 4 — Frontmatter edge syntax is typed objects with string shorthand; tags are labels only.

**Why:** The canonical `links:` shape is a list of `{target, type, valid_from, valid_until}` objects. Bare strings under `links:` are accepted as shorthand for `{target: <string>, type: related}`. `parent:`, `children:`, and `related:` are convenience fields with fixed relationship types. `tags:` syncs labels into the `tags` table and never writes graph edges.

**Accepted shapes:**

- `links: [companies/brex]`
- `links: [{target: companies/brex, type: founded, valid_from: 2017-01-01}]`
- `parent: programs/yc-w17`
- `children: [companies/brex, companies/scale]`
- `related: [people/alice, companies/acme]`
- `tags: [fintech, yc-w17]` or `tags: fintech, yc-w17`

### Decision 5 — Source-kind weights start global and configurable.

**Why:** Frontmatter is the strongest signal, entity-pattern extraction is medium-confidence, and wikilinks are weaker navigational references. The initial defaults are simple and benchmark-tunable: frontmatter `1.0`, entity-pattern `0.7`, wikilink `0.5`.

**Config keys:** Store `edge_weight_frontmatter`, `edge_weight_entity_pattern`, and `edge_weight_wikilink` in the existing `config` table.

### Decision 6 — Derived edges are unique; programmatic temporal history is not constrained.

**Why:** Derived edges must be idempotent under re-ingest, but manual/programmatic links may intentionally contain multiple temporal intervals for the same `(from, to, relationship)`. A global unique index would break that manual history.

**Index:** Add a partial unique index on `(from_page_id, to_page_id, relationship, source_kind)` only for `source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern')`.

**Upsert semantics:** Derived-edge upserts replace `valid_from`, `valid_until`, `edge_weight`, and `context` for the same key. Frontmatter and wikilink sync delete stale derived rows for the source page when the source no longer contains the edge. Programmatic links continue to use the existing manual link behavior.

### Decision 7 — Entity-pattern resolution is role-aware and collection-local.

**Why:** A raw surface like `Alice` normalizes to `alice`, but the actual page is likely `people/alice`; `Brex` is likely `companies/brex`. Entity extraction needs a resolver beyond bare `resolve_slug()`.

**Resolver order:** For each captured surface, infer a role/type hint from the pattern (`founded`: subject person/company, object company; `works_at`: subject person, object company; `acquired`: subject company, object company; `invested_in`: subject person/company, object company; `leads`: subject person, object project/company). Then try, within the source collection:

1. exact slug after `resolve_slug(surface)`;
2. role-prefixed slug candidates such as `people/<slug>` or `companies/<slug>`;
3. case-insensitive exact title match constrained by role hint;
4. unique slug basename match constrained by role hint.

If exactly one page resolves for both endpoints, write a `links` row from the resolved subject page to the resolved object page. If either endpoint is unresolved or ambiguous, write an assertion only and do not pollute the graph.

### Decision 8 — Entity-pattern output goes to `assertions` only in this change; durable `entity_pattern` links are deferred.

**Why (updated per Nibbler's review gate, 2026-05-12):** The original design routed resolved entity-pattern matches to `links`. Nibbler blocked this pending (a) explicit source-page provenance on `links` rows (so a retract/re-sync by the source page is unambiguous) and (b) proven retraction semantics (so stale derived rows are cleaned up when the source page changes). Neither is implemented in this change. The safer option is to write all entity-pattern matches to `assertions` only, regardless of whether both endpoints resolve to pages. This preserves the extracted knowledge in a queryable form without polluting the durable graph with edges that cannot be reliably retracted.

**Assertion routing:** Entity assertions use `(subject, predicate, object)`, `asserted_by = 'agent'`, `confidence = pattern.weight`, and evidence/source context from the page where the match was found. Duplicate assertions are prevented by checking `(page_id, subject, predicate, object)` before insert.

**Deferred:** A follow-on change will add `source_page_id` provenance to `links` and wire `entity_pattern` edges through the derived-edge sync owner (task 4.1) with proven retraction. Until that change lands, `source_kind = 'entity_pattern'` is reserved in the schema CHECK constraint but no write path inserts rows with that value.

### Decision 9 — Graph expansion is layered onto `hybrid_search`, not a parallel pipeline.

**Why:** Search stays one entry point. `hybrid_search` first produces FTS5 + vector candidates, then a bounded graph-expansion pass walks outward from those candidates and scores expansions by `(parent_score) × edge_weight × distance_decay^hops`. This treats the graph as a recall booster and re-ranker around already relevant pages rather than an independent result source.

**Bounds:** Default depth is 1. CLI/config allow 0–3. Expansion caps new candidates via `graph_expansion_max` and caps visited nodes via the graph module's `MAX_NODES` safety limit.

### Decision 10 — Graph path output changes the pre-release contract.

**Why:** Path explanations make navigational graph results auditable and are required by the retrieval/graph UX. Because Quaid is pre-release and has no compatibility obligations yet, `GraphResult` can grow a `paths` field and `memory_graph` can return the new shape without version negotiation. The MCP handler target for this change is `src/mcp/tools/links.rs` (decomposed from `src/mcp/server.rs` in PR #183, v0.21.0).

### Decision 11 — Single derived-edge sync owner and task wave order.

**Why:** Multiple write paths (put, ingest, import, vault sync, MCP) must all call the same derived-edge sync logic. Without a single owner, each path diverges independently. The canonical owner is the `upsert_derived_edge` + `sync_frontmatter_edges` / `sync_wikilink_edges` primitives in `src/core/links.rs`. Every write path calls these primitives; none re-implements edge logic inline.

**Wave order:**
1. **Schema + config** (tasks 1.x): bump schema to v10, extend `source_kind`, add `edge_weight`, partial unique index, seed config defaults.
2. **Structured frontmatter** (tasks 2.1–2.5 pre-checked, task 2.6): already implemented at v9 baseline; verify and add remaining unit tests.
3. **Frontmatter edge and tag expansion types** (tasks 3.x): `FrontmatterLink`, `expand_frontmatter_edges`, `expand_frontmatter_tags` — pure parsing, no DB writes yet.
4. **Derived-edge sync primitive** (tasks 4.x): `upsert_derived_edge`, `sync_frontmatter_edges`, `sync_wikilink_edges` in `src/core/links.rs` — the single write owner.
5. **Write-path wiring** (tasks 5.x): wire the sync primitives into `commands/put.rs`, `commands/ingest.rs`, `src/core/migrate.rs`, `src/core/vault_sync/mod.rs`, and `src/mcp/tools/pages.rs`.
6. **Entity-pattern extraction** (tasks 6.x, 7.x, 8.x): regex config, resolver, extraction loop, assertions-only routing, opt-in backfill command.
7. **Graph-aware retrieval and path output** (tasks 9.x, 10.x): `expand_graph`, updated `hybrid_search`, `progressive_retrieve`, `--hops` flag, `GraphResult.paths`, `src/mcp/tools/links.rs` output schema.
8. **Tests, benchmarks, docs** (tasks 11.x, 12.x, 13.x).

## Risks / Trade-offs

- **Risk: structured frontmatter consumers break.** `Page.frontmatter` is already `JsonMap<String, JsonValue>` at v9 baseline; scalar helpers are in place. → Mitigation: edge-expansion consumers are new code; existing consumers are unchanged. Remaining exposure is in write paths that need to call the new sync primitives.
- **Risk: regex false positives pollute assertions.** Unresolved/ambiguous matches still write assertions; duplicates are deduplicated. → Mitigation: role-aware resolver reduces false matches; confidence field lets downstream consumers filter by weight.
- **Risk: entity-pattern `links` routing deferred until provenance + retraction land.** All entity patterns go to assertions only in this change. → Mitigation: assertions are queryable and preserve extracted knowledge; a follow-on change will add durable `entity_pattern` edges once retraction semantics are proven.
- **Risk: graph-aware ranking degrades broad queries.** → Mitigation: depth defaults to 1, `graph_expansion_max` caps additions, `graph_depth = 0` disables the feature, and benchmark gates include broad as well as navigational queries.
- **Risk: entity extraction destabilizes import time.** → Mitigation: regexes compile once, extraction has a 5 ms per-page budget, and over-budget pages skip remaining patterns and log a gap instead of failing the write.
- **Trade-off: edge weights are source-level, not relationship-level.** This keeps v1 simple. If DAB §4 plateaus, per-relationship weights are the next obvious lever.

## Pre-release Schema Plan

1. Update `src/schema.sql` to v10 directly: extend `source_kind`, add `edge_weight`, add the partial unique index for derived edges, and seed graph config defaults.
2. Bump `SCHEMA_VERSION`, `config.version`, and `quaid_config.schema_version` expectations to 10.
3. Do not implement v9 → v10 migration or rollback. Existing v9 databases remain incompatible by design.
4. Re-import fixture/dev vaults to populate structured frontmatter, tags, wikilinks, and frontmatter edges.
5. Keep entity-pattern backfill opt-in via `quaid graph extract-entities` because it is potentially expensive and heuristic.

## Open Questions

- **Question 1 — Per-relationship weights.** Should `founded` outrank `related` independent of source? Defer to DAB measurement.
- **Question 2 — Tags as soft edges.** Two pages sharing a tag are arguably 2-hop neighbours. Defer; this may swamp real edges.
- **Question 3 — Wikilink demotion.** Some users use wikilinks as inline references. Instrument first, then decide whether parenthetical/footnote wikilinks should be downweighted further.
- **Question 4 — Multi-collection edges.** Out of scope for this change. Collection-local resolution keeps writes deterministic.
