# Changelog

All notable changes to Quaid are tracked here. Pre-1.0, schema and
response-shape changes may break compatibility between minor versions —
each entry below calls out the migration implications.

## v0.23.0 — code-review remediation (Waves 0–4)

This release lands the full 26-area code-review remediation: structural
retrieval-quality fixes, namespace-aware page identity, a versioned schema
migration ladder, security hardening, daemon and vault-sync robustness, an
opt-in outbound redaction layer, two new MCP tools (the registry is now **26
tools**), and broad correctness, performance, and test-effectiveness work.

**Breaking for consumers:** the `memory_query` / `memory_search` responses are
now an object envelope (was a bare array), all conflict errors carry one
canonical `ConflictError:` prefix, and conversation capture moved to format v2.
The embedding pooling/instruction changes alter every stored vector — a one-time
`quaid embed --all` re-embed is required after upgrading. See **Migration**.

### Added

- **Two new MCP tools (24 → 26).** `memory_gap_resolve` (resolve a logged
  knowledge gap by the page that answered it) and `memory_rehydrate` (reverse
  this session's outbound redaction tokens such as `<EMAIL_1>` back to their
  original values — the read-side counterpart to the new `redact` option) join
  the stdio registry.
- **Namespace-aware MCP surface + `core::pages` identity resolver.** A single
  sanctioned `core::pages::resolve*` path now keys page identity by
  `(collection, namespace, slug)`; the MCP surface, reconciler reingest, and a
  backfill migration were threaded through it. A source-audit test fails on any
  new namespace-blind `slug = ?` lookup.
- **Opt-in outbound secret scrubbing (redaction phase 1).** Read-chokepoint
  redaction (`redact` option on `memory_query`/`memory_search`/`memory_get`)
  scrubs secrets before results leave the process, for cloud-LLM contexts.
- **Rerank plumbing, MCP `hops`, and gap-loop fixes.** Optional rerank stage,
  multi-hop neighbourhood expansion on reads, and a corrected gap-logging
  heuristic that no longer logs phantom gaps on strong hits.
- **Contradiction detection & resolution surface.** Heuristic contradiction
  detection is exposed through the surface rather than requiring an explicit
  `## Assertions` section.
- **`quaid setup --register-mcp` + installer onboarding.** First-run MCP client
  registration and clearer installer guidance.
- **`quaid skills extract` + skills refresh.** Materialize embedded skills to
  disk for editing; refreshed skill content and resolution doctor.
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
- **CI benchmark/release gates + docs-truth grep gates.** Mini-bench retrieval
  gate, release-asset/version verification, and docs gates that pin
  current-release version, MCP tool-surface, and schema-version claims to the
  code.

### Changed

- **MCP read envelope (BREAKING).** `memory_query` and `memory_search` now
  return `{ "results": [...], "pending_embedding_jobs"?: N }` instead of a bare
  array. Clients must read `.results`.
- **Embedding quality (re-embed required).** The BGE path now uses CLS pooling
  (was mean) on both queries and passages, applies the BGE query-instruction
  prefix asymmetrically (queries only), caps chunks, and records an embedder
  version. This materially changes stored vectors and improves semantic recall.
- **Retrieval fusion.** Recalibrated fusion `k`/relevance floor, tiered OR
  blending, numeric-alias symmetry (e.g. `$75K` ↔ `75000`), and `memory_search`
  parity with `memory_query`.
- **Two-phase KNN vector retrieval** with vec0 cleanup and a CLI embed drain,
  for correctness and scale on larger stores.
- **Conflict message prefix (BREAKING).** All JSON-RPC `-32009` conflict
  messages now carry the single canonical `ConflictError: ` prefix (previously
  a mix of `conflict: `, `Conflict: `, and `ConflictError: `). The error code is
  unchanged; consumers pattern-matching the old spellings should match
  `ConflictError`.
- **Hot-path cost** is now proportional to change size per vault event.
- **Layering.** Page reads moved out of `commands` into `core::pages`; `put`
  uses `BEGIN IMMEDIATE` to fix cross-process write contention.
- The formerly scattered open-time `ensure_*` schema patches are consolidated
  into one idempotent current-version maintenance step shared by `open` and
  `quaid migrate`. Future DDL changes must land as new migration-registry
  rungs with a `SCHEMA_VERSION` bump, not as unversioned open-time patches.

### Fixed

- **CLI ↔ MCP dispatch parity.** `quaid call` / `quaid pipe` now route
  `memory_correct` and `memory_correct_continue`, matching the 26-tool MCP
  registry. A parity test pins the dispatcher to the registry.
- **Global `--json` flag.** `--json` is now honoured by `put`, `ingest`,
  `export`, `extract`, `embed`, `link`, `link-close`, `unlink`, `tags`,
  `timeline-add`, `compact`, and `config`; `quaid --json status` and
  `quaid status --json` are equivalent. `put` keeps stdout JSON-clean by
  routing its embedding-drain note to stderr.
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
- **Conversation extraction atomicity & idempotency.** Session close and SLM
  fact extraction are now atomic and safely re-runnable.
- **SLM extraction quality.** Phi-3 chat template, EOS-token union, fenced-JSON
  recovery, F16 weight load, and a runner gate.
- **Custom-model safety.** Unpinned custom-model downloads are refused; the
  loader never silently falls back to the default model.
- **Daemon lifecycle.** State-aware launchd start, atomic dead-PID daemon
  claim, and a clean exit on supervisor death.
- **Vault-sync robustness.** Pages whose vault file vanished or became
  unparseable are quarantined instead of destroyed; the heartbeat is isolated
  and staleness handling is fail-safe. Restore validates DB-sourced paths and
  uses fd-pinned durable writes.
- **Retrieval / data-fidelity correctness cluster** plus a supersede-cycle
  guard.
- **Security hardening.** `Origin`/`Host` validation on the opt-in HTTP/SSE
  transport; conversation turn-boundary and metadata markers are escaped behind
  capture format v2.

### Migration

- **Re-embed after upgrading.** The CLS-pooling, query-instruction, and chunking
  changes alter every stored vector. Run `quaid embed --all` once after
  installing v0.23.0 so semantic search reflects the new embeddings.
- **MCP consumers** must read the `{ "results": [...] }` envelope from
  `memory_query` / `memory_search`, match `ConflictError:` on `-32009`, and
  update conversation readers for capture format v2.
- Schema v9 databases (v0.20.x / v0.21.x) can be upgraded in place with
  `quaid migrate` instead of export + re-ingest. v0.22.x (v10) databases are
  unaffected; running `quaid migrate` on them is a no-op.

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
