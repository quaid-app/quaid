# Graph layer

Quaid's `links` table is a typed temporal knowledge graph. The graph layer
spans three slices: how edges are **created** (frontmatter autowiring +
wikilinks + entity-pattern matches), how they are **traversed** (the
`memory_graph` MCP tool and `quaid graph` CLI), and how they **influence
retrieval** (`hybrid_search` graph expansion).

This page is the developer-facing summary. The full normative contract
lives in the OpenSpec change `knowledge-graph-layer` (specs:
`frontmatter-link-autowiring`, `entity-pattern-extraction`,
`graph-aware-retrieval`).

## Frontmatter link syntax

Every page write parses YAML frontmatter and produces derived `links` rows
with `source_kind = 'frontmatter'`. Two shapes are accepted:

```yaml
---
title: Alice
type: person
# Canonical object form
links:
  - target: companies/brex
    type: founded
    valid_from: 2017-01-01
    valid_until: 2024-12-31
# String shorthand (relationship defaults to `related`)
  - companies/scale
# Fixed-relationship convenience fields
parent: programs/yc-w17
children:
  - companies/brex
related:
  - people/bob
# or, for a single related page:
# related: people/carol
tags: [fintech, yc-w17]
---
```

Behavior:

| Field | Edge `relationship` | `source_kind` | Notes |
| ----- | ------------------- | ------------- | ----- |
| `links:` object form | `type` field, default `related` | `frontmatter` | Temporal fields supported |
| `links:` string form | `related` | `frontmatter` | – |
| `parent:` | `parent` | `frontmatter` | Single string |
| `children:` | `child` | `frontmatter` | YAML list |
| `related:` | `related` | `frontmatter` | YAML string or list |
| `tags:` | – | – | Populates `tags` table only |

Slug normalization (`resolve_slug`) is applied to every target. Unresolved
targets are skipped and logged once to `knowledge_gaps` (de-duplicated).

## Body wikilinks

`[[slug]]` patterns in `compiled_truth` or `timeline` create derived rows
with `source_kind = 'wiki_link'` and `edge_weight = config.edge_weight_wikilink`
(default `0.5`). Removed wikilinks are removed from `links` on re-write.

## Entity-pattern extraction (assertions-only)

`quaid graph extract-entities` runs regex patterns from
`~/.quaid/entity-patterns.yaml` (or the embedded defaults: `works_at`,
`founded`, `invested_in`, `acquired`, `leads`) over every page and writes
matches as rows in `assertions` with `asserted_by = 'agent'` and
`confidence = pattern.weight`. Per the design Decision 11, this change
does **not** insert durable `links` rows with `source_kind = 'entity_pattern'`;
edge-promotion semantics are deferred to a follow-on change.

The extractor has a 5 ms per-page deadline; pages that exceed it log a
`knowledge_gap` and skip remaining patterns. The extractor uses no
embedding, inference, or network calls.

## Derived-edge idempotency

A partial unique index on `(from_page_id, to_page_id, relationship, source_kind)`
where `source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern')` makes
derived writes idempotent:

- Re-ingesting an unchanged page produces no duplicate edges.
- Changing `valid_from`/`valid_until` updates the existing row in place.
- Removing a link from frontmatter deletes the corresponding derived row
  on the next write of the source page.

The uniqueness constraint does **not** apply to `source_kind = 'programmatic'`,
so manual `quaid link` calls with overlapping `(from, to, relationship)` and
different temporal ranges remain representable.

## Graph-aware retrieval

When `config.graph_depth > 0`, `hybrid_search` walks outbound active edges
from the FTS5 + vector top-K and adds reachable pages as additional
candidates. Each expanded candidate's score is:

```
expanded_score = parent_score × edge_weight × (graph_distance_decay ^ hops)
```

Bounds:

- per-query newly added candidates capped at `config.graph_expansion_max`
  (default `50`),
- total visited nodes capped at `MAX_NODES = 1000`,
- only currently active edges are walked (temporal filter).

CLI: `quaid query --hops N` and `quaid search --hops N` override
`config.graph_depth` for a single invocation.

## Graph config keys

Seeded by `quaid init` into the `config` table:

| Key | Default | Effect |
| --- | ------- | ------ |
| `graph_depth` | `0` | Hops walked during retrieval expansion (`0` disables it). |
| `graph_distance_decay` | `0.5` | Multiplied per hop into the candidate score. |
| `graph_expansion_max` | `50` | Cap on newly added candidates per query. |
| `edge_weight_frontmatter` | `1.0` | Default weight for `frontmatter` rows. |
| `edge_weight_entity_pattern` | `0.7` | Reserved for the entity-pattern edge follow-on. |
| `edge_weight_wikilink` | `0.5` | Default weight for `wiki_link` rows. |

Edit with `quaid config set graph_depth 0` (or via the `config` table
directly).

## `memory_graph` / `quaid graph` path output

The `memory_graph` MCP response and `quaid graph <slug>` CLI output include
a `paths` field keyed by reachable slug. Each value is a list of
`(from_slug, relationship, to_slug)` triples describing the first path
found to that node during BFS. The path for the root slug is an empty
list. This is a pre-release response-shape change with no compatibility
mode.

## Acceptance gate

Graph-aware retrieval changes the default ranking behaviour. The OpenSpec
change carries a benchmark acceptance gate documented in
[`benchmarks/graph_retrieval.md`](../benchmarks/graph_retrieval.md): DAB §4
Semantic/Hybrid must improve ≥ 8 points (target ≥ 35/50) and MSMARCO P@5
must improve ≥ 5 points over a reproducible bge-small baseline before
`graph_depth` can ship default-on. Until those numbers are recorded,
autowiring and `memory_graph` path output ship with retrieval expansion off
by default.
