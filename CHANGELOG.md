# Changelog

All notable changes to Quaid are tracked here. Pre-1.0, schema and
response-shape changes may break compatibility between minor versions —
each entry below calls out the migration implications.

## v0.22.2 — conversation turn delimiter fix

### Fixed

- **Conversation capture stability.** `memory_add_turn` and
  `memory_close_session` now handle conversation content that contains Markdown
  horizontal rules (`---`). New conversation day-files use an unambiguous
  Quaid turn boundary marker while the parser remains compatible with existing
  legacy turn files.

### Migration

- No schema migration. Existing v0.22.x databases remain compatible.

## v0.22.1 — beta regression fixes

### Fixed

- **Vault import / export parity.** `related:` frontmatter now accepts either a
  scalar string or a YAML list, so Obsidian/DAB-style scalar relationships no
  longer abort collection attach or create export deltas.
- **PARA type inference.** Collection ingest now recognizes both singular and
  plural PARA folder names (`project(s)`, `area(s)`, `resource(s)`,
  `archive(s)`) after numeric-prefix stripping.
- **MCP runtime shutdown.** `quaid serve` and foreground `quaid daemon run`
  handle SIGTERM/SIGINT by dropping owned runtime workers and unregistering
  their `serve_sessions` rows without touching unrelated processes.
- **Embedding batch stability.** Bulk `quaid embed` scans pages in bounded
  batches (default 32) and adds `--batch-size N` validation to reduce first-run
  memory pressure on larger vaults while preserving idempotent reruns.

### Migration

- No schema migration. Existing v0.22.0 databases remain compatible.

## v0.22.0 — knowledge graph layer (pre-release v9 → v10)

### Schema

- **v10 schema reset.** Bumped `SCHEMA_VERSION`, `config.version`, and
  `quaid_config.schema_version` to `10`. No v9 → v10 data migration is
  provided; existing v9 databases continue to fail with the existing
  schema-mismatch / re-init message. `quaid init` is the only supported
  path to v10.
- **Derived-edge columns and constraints on `links`:**
  - new `edge_weight REAL NOT NULL DEFAULT 1.0`
  - `source_kind` CHECK constraint extended to allow `'frontmatter'` and
    `'entity_pattern'` (the latter reserved for a follow-on change)
  - new partial unique index `idx_links_unique_derived_edge` on
    `(from_page_id, to_page_id, relationship, source_kind)` where
    `source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern')`;
    `source_kind = 'programmatic'` retains duplicate-temporal-row freedom

### Config

New seeded `config` keys (defaults shown):

| Key | Default |
| --- | ------- |
| `graph_depth` | `0` |
| `graph_distance_decay` | `0.5` |
| `graph_expansion_max` | `50` |
| `edge_weight_frontmatter` | `1.0` |
| `edge_weight_entity_pattern` | `0.7` |
| `edge_weight_wikilink` | `0.5` |

### Behavior

- **Structured frontmatter.** Page frontmatter is parsed and stored as
  structured JSON (`pages.frontmatter` is a full JSON object). Arrays and
  objects survive `export → re-import` round-trips.
- **Frontmatter link autowiring.** Every page write produces derived
  `links` rows with `source_kind = 'frontmatter'` from the canonical
  `links:`, `parent:`, `children:`, and `related:` fields. String
  shorthand defaults to `relationship = 'related'`. Stale derived edges
  are deleted on re-write. Unresolved targets are skipped and logged once
  to `knowledge_gaps`.
- **Wikilink autowiring.** Body `[[slug]]` patterns produce derived
  `wiki_link` rows synchronised with the source page on every write.
- **Entity-pattern extraction (assertions only).** `quaid graph
  extract-entities` runs regex patterns from
  `~/.quaid/entity-patterns.yaml` (or the embedded defaults `works_at`,
  `founded`, `invested_in`, `acquired`, `leads`) over every page and
  writes matches to `assertions` with `asserted_by='agent'` and
  `confidence=pattern.weight`. Per Decision 11, this change does **not**
  insert durable `links` rows with `source_kind='entity_pattern'`.
- **Graph-aware retrieval.** `hybrid_search` walks outbound active edges
  from the FTS5 + vector top-K when `config.graph_depth > 0`. The seeded
  default is `0` until the benchmark gate below has published passing
  DAB §4 / MSMARCO numbers. Expanded
  candidates score
  `parent_score × edge_weight × (graph_distance_decay ^ hops)` and
  participate in progressive token-budget pruning.
- **CLI `--hops N`.** `quaid query --hops N` and `quaid search --hops N`
  override `config.graph_depth` for a single invocation.
- **`quaid graph extract-entities`.** Opt-in backfill command iterates
  all pages and writes assertions; idempotent on re-run.

### MCP / CLI response shape

- **`memory_graph` and `quaid graph <slug>` now include `paths`.** The
  response carries a `paths` map keyed by reachable slug, where each
  value is the list of `(from_slug, relationship, to_slug)` triples
  describing the first path found to that node during BFS. The path for
  the root slug is empty. This is a pre-release response-shape change
  with no compatibility mode.

### Acceptance gate

- A release shipping graph-aware retrieval as default-on must pass the
  DAB §4 (≥ 35/50, ≥ +8 points over bge-small baseline) and MSMARCO P@5
  (≥ +5 points) gates documented in `benchmarks/graph_retrieval.md`. This
  release ships autowiring + path-output features with `graph_depth = 0`
  because the representative DAB §4 / MSMARCO measurements have not yet
  been recorded.

### Documentation

- New `docs/graph.md` summarises frontmatter link syntax, tag behavior,
  entity-pattern override YAML, graph search knobs, and path explanations.
- New `benchmarks/graph_retrieval.md` encodes the acceptance procedure.
- Website reference updates: `memory_graph` response includes `paths`;
  graph config keys documented under `reference/configuration`.
