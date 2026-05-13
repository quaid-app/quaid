## Why

Quaid already has typed links and `memory_graph` / `memory_backlinks` read surfaces, but the graph is sparse in practice because derived edges are not written automatically. DAB §4 Semantic / Hybrid has been stuck at 19–31/50 across nine consecutive releases — the chronic top-line drag on overall grade. A self-wiring graph is the foundation that unblocks Epic 2 (retrieval quality) and gives every retrieval improvement a populated signal to compound against. Reference implementations (GBrain v0.12) report +5% precision, +11% recall, +28% on graph queries, and −53% noisy results from this single architectural change.

## What Changes

- Preserve structured YAML frontmatter instead of reducing everything to scalar strings. Existing scalar consumers keep helper accessors, while graph/tag extraction can read arrays and objects directly.
- Auto-create and sync typed graph edges from frontmatter (`links:`, `related:`, `parent:`, `children:`) on write/ingest, including temporal validity (`valid_from`, `valid_until`).
- Keep frontmatter `tags:` as labels only: sync them into the `tags` table, but do not turn tags into graph edges.
- Add a production wikilink write path: body `[[slug]]` references become soft `wiki_link` edges with lower weight than explicit frontmatter edges.
- Add a zero-LLM entity-extraction pass at write time: regex-driven detection of `works_at`, `founded`, `invested_in`, `acquired`, and `leads` relationships, configurable via `~/.quaid/entity-patterns.yaml`, and budgeted at < 5 ms per page. In this change, **all entity-pattern match outputs are routed to `assertions` only** regardless of endpoint resolution; durable `entity_pattern` edges in `links` are deferred to a follow-on change that adds explicit source-page provenance to the `links` table and proven retraction semantics.
- Extend retrieval to fuse semantic similarity with graph proximity: score = `(parent semantic/lexical score) × edge_weight × distance_penalty`, with configurable depth (0–3 hops).
- Extend `quaid graph <slug>` and the `memory_graph` MCP read tool to include path explanations. This is a pre-release contract change; no backward compatibility is required.
- **Pre-release schema reset**: update the canonical schema to v10 directly (baseline is v9). There is no v9 → v10 automatic migration, rollback, or backfill path. Existing development databases should be re-initialized and re-imported under the current no-auto-migration policy.

## Capabilities

### New Capabilities

- `frontmatter-link-autowiring`: Parsing of frontmatter link fields (`links`, `related`, `parent`, `children`) and body wikilinks into typed `links`-table rows on every write/ingest, with temporal validity and idempotency under re-ingest.
- `entity-pattern-extraction`: Configurable regex-based extraction of relationship triples (`works_at`, `founded`, `invested_in`, `acquired`, `leads`) from page content at write time, with role-aware entity resolution, a per-page time budget, no LLM calls, and user-overridable patterns. All matches are routed to `assertions` in this change; durable `links` from entity patterns require follow-on source-page provenance work.
- `graph-aware-retrieval`: Multi-hop graph traversal layered onto `hybrid_search` and `progressive_retrieve`, scoring candidates by semantic similarity × edge weight × distance penalty, with configurable depth.

### Modified Capabilities

- `memory_graph` / `quaid graph`: graph read output now includes path explanations for reachable nodes. This intentionally changes the pre-release response shape.
- `page-frontmatter`: page writes preserve structured frontmatter values so arrays/objects round-trip and can drive derived side tables.

## Impact

- **Code**: `src/core/markdown.rs` and `src/core/types.rs` (structured frontmatter already implemented; verification and edge-expansion consumers to add), `src/core/links.rs` (derived edge expansion/upsert + wikilinks), `src/commands/tags.rs` or shared tag helpers (frontmatter tag sync), `src/core/entities.rs` (regex extraction + entity resolution), `src/core/graph.rs` (path tracking), `src/core/search.rs` and `src/core/progressive.rs` (graph-aware ranking), `src/commands/` (new `--hops` flags and graph subcommand), `src/mcp/tools/links.rs` (graph output schema).
- **Schema**: Existing `links` table remains the graph store. Extend `source_kind` to `('wiki_link', 'programmatic', 'frontmatter', 'entity_pattern')`, add `edge_weight REAL NOT NULL DEFAULT 1.0`, and add a partial unique index for derived sources only: `(from_page_id, to_page_id, relationship, source_kind) WHERE source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern')`.
- **Config**: New keys in the existing mutable `config` table: `graph_depth`, `graph_distance_decay`, `graph_expansion_max`, `edge_weight_frontmatter`, `edge_weight_entity_pattern`, `edge_weight_wikilink`.
- **Migration**: None. Bump `SCHEMA_VERSION` / `config.version` / `quaid_config.schema_version` to v10 and rely on the existing schema-mismatch behavior for stale dev databases.
- **Tests**: Extend roundtrip tests for structured frontmatter and derived edge equivalence. Add `tests/graph_autowire.rs`, `tests/entity_extraction.rs`, and `tests/graph_retrieval.rs` for autowiring, entity extraction, path output, and graph-aware ranking.
- **Benchmarks**: DAB §4 Semantic should improve by ≥ 8 points (target ≥ 35/50). MSMARCO P@5 should improve by ≥ 5 points versus the bge-small baseline. Acceptance remains measured against reproducible baselines.
- **Dependencies**: No new runtime dependencies. `regex` and `serde_yaml` already exist.
- **Performance**: Ingest path adds ≤ 5 ms per page for entity extraction and ≤ 1 ms per page for frontmatter/wikilink edge expansion. Retrieval path adds one bounded BFS-style expansion per query (depth ≤ 3); measured cost must remain inside the standard latency budget.
