# Changelog

All notable changes to Quaid are tracked here. Pre-1.0, schema and
response-shape changes may break compatibility between minor versions —
each entry below calls out the migration implications.

## Unreleased

### Added

- **Versioned schema migration ladder + `quaid migrate`.** New explicit
  `quaid migrate [path]` command upgrades older databases in place by walking
  a versioned migration registry in `src/core/db.rs`. The first registered
  rung migrates schema v9 (v0.20.x / v0.21.x) databases to v10: `links`
  table rebuild (extended `source_kind` CHECK plus the `edge_weight` column),
  derived-edge dedup with the `idx_links_unique_derived_edge` partial unique
  index, and the v10 graph config seeds. The run writes a `<db>.bak` backup
  first, applies each step in its own transaction (bumping
  `quaid_config.schema_version` and the legacy `config.version` mirror per
  step), and verifies `PRAGMA integrity_check` plus row-count sanity
  afterwards. Plain opens remain fail-closed on any schema-version mismatch.

### Changed

- The formerly scattered open-time `ensure_*` schema patches are consolidated
  into one idempotent current-version maintenance step shared by `open` and
  `quaid migrate`. Future DDL changes must land as new migration-registry
  rungs with a `SCHEMA_VERSION` bump, not as unversioned open-time patches.

### Migration

- Schema v9 databases (v0.20.x / v0.21.x) can now be upgraded in place with
  `quaid migrate` instead of export + re-ingest. v0.22.x (v10) databases are
  unaffected; running `quaid migrate` on them is a no-op.
### Fixed

- **CLI ↔ MCP dispatch parity.** `quaid call` / `quaid pipe` now route
  `memory_correct` and `memory_correct_continue`, matching the 24-tool MCP
  registry. A parity test pins the dispatcher to the registry.
- **Global `--json` flag.** `--json` is now honoured by `put`, `ingest`,
  `export`, `extract`, `embed`, `link`, `link-close`, `unlink`, `tags`,
  `timeline-add`, `compact`, and `config`; `quaid --json status` and
  `quaid status --json` are equivalent.
- **Dead flags implemented.** `quaid export --raw [--import-id <id>]` now
  performs a byte-exact restore from `raw_imports`; `quaid embed --all`
  forces a re-embed (bypassing the unchanged-hash skip) while `--stale`
  keeps the skip.
- **`quaid status` exit code 3.** A failed service-manager probe
  (`launchctl`/`systemctl` error) now exits 3 as documented, instead of
  being misreported as "not installed" (exit 2). JSON output gains a
  `daemon.probe_failed` field.
- **Schema-mismatch message accuracy.** Opening a database with an older
  (e.g. v9, shipped with quaid v0.20.x-v0.21.x) or newer schema version
  no longer claims the file predates the Quaid rename; the message now
  distinguishes older/newer/legacy cases and instructs backing up the old
  file before `quaid init` (the previous remediation dead-looped).

### Changed

- **Conflict message prefix.** All JSON-RPC `-32009` conflict messages now
  carry the single canonical `ConflictError: ` prefix (previously a mix of
  `conflict: `, `Conflict: `, and `ConflictError: `). The JSON-RPC error
  code is unchanged; consumers pattern-matching the old spellings should
  match `ConflictError` instead.

## v0.22.6 — namespaced search and numeric FTS fixes

### Fixed

- **Namespaced `memory_search` results.** `memory_search` (and `memory_query`)
  called with a `namespace` now return results for pages extracted from
  sessions stored under that namespace, instead of an empty array (#212).
- **FTS5 numeric aliasing.** Bare numeric query tokens are expanded at query
  time into grouped and abbreviated aliases, so a search for `75000` now
  matches content written as `75,000` or `$75K` (#196). Implicit-AND semantics
  between top-level tokens are preserved.

### Migration

- No schema migration. Numeric aliasing is query-time only — existing FTS
  indexes do not need to be rebuilt.

## v0.22.5 — extraction warning fixes

### Fixed

- **Blank embedding chunks from conversation day-files.** Single-turn
  conversation day-files no longer enqueue empty embedding chunks when the
  vault watcher re-ingests the canonical file (#217).
- **SLM extraction parser hardening.** Quote-delimited and prompt-echo
  wrappers around the extraction JSON envelope now fail closed, while
  commentary-wrapped `{"facts":[...]}` output still recovers instead of
  retry-failing the extraction worker.

### Migration

- No schema migration. Existing v0.22.x databases remain compatible.

## v0.22.4 — playground, shutdown handling, model cache management

### Added

- **Playground UI webapp.** A local web playground under `playground/` for
  exercising search, conversation capture, and extraction against a quaid
  database.
- **Graceful shutdown signal handling.** The serve/daemon runtime arms a
  dedicated SIGTERM/SIGINT shutdown signal for orderly worker termination.
- **Online model cache management.** `quaid model` gains cache inspection and
  validation: cache directory/key retrieval, required-file verification for
  online embedding model caches, and cleanup of temporary download files when
  hash verification fails (see `docs/model-cache.md`).

### Fixed

- **Vault-sync handshake timeout.** The watcher handshake timeout is now
  configurable via an environment variable instead of a fixed value.

### Migration

- No schema migration. Existing v0.22.x databases remain compatible.

## v0.22.3 — post-release bug fixes

### Fixed

- **First-run collection defaults.** Fresh `quaid init` now provisions a writable
  default collection at `~/.quaid/vault`, so MCP conversation capture can write
  turns before any manual `collection add`. Existing databases with configured
  write-target roots are preserved; legacy unconfigured defaults are repaired on
  open.
- **Synthetic benchmark fallback.** `make bench` / DAB-oriented benchmark flows
  now generate the synthetic corpus automatically when the DAB corpus is absent,
  so fresh clones no longer fail before the benchmark harness can run.
- **Phi-3 config compatibility.** SLM model loading now normalizes
  `rope_scaling` object payloads before `Phi3Config` deserialization, fixing a
  main-branch regression in the conversation extraction path.

### Migration

- No schema migration. Existing v0.22.x databases remain compatible; only
  unconfigured default write-target states are conditionally bootstrapped.

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
