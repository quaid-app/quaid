# graph-aware-retrieval Specification

## Purpose
TBD - created by archiving change knowledge-graph-layer. Update Purpose after archive.
## Requirements
### Requirement: `hybrid_search` consults graph proximity when ranking candidates
After computing the FTS5 + vector top-K candidate set, the system SHALL run a bounded graph-expansion pass when graph depth is greater than 0. For each candidate, the expansion SHALL walk up to `config.graph_depth` hops outward over currently active `links` rows and add reachable pages as additional candidates. Each expanded candidate's score SHALL equal `(parent_score) × edge_weight × distance_penalty`, where `distance_penalty = config.graph_distance_decay ^ hops` (default `0.5`).

#### Scenario: 1-hop expansion adds graph neighbours to the candidate set
- **WHEN** a query produces a top-K with one candidate `alice`, `alice` has a `founded` edge to `brex` with `edge_weight = 1.0`, and `graph_depth = 1`
- **THEN** the final result set contains `brex` with score `alice.score × 1.0 × 0.5`

#### Scenario: Graph expansion respects `graph_depth`
- **WHEN** `graph_depth = 1` and `alice → brex → fintech-investor` is a 2-hop chain
- **THEN** `fintech-investor` is NOT added to the candidate set from this expansion

#### Scenario: Graph expansion respects active-only temporal filter
- **WHEN** an edge `alice → brex` has `valid_until = '2020-01-01'` in the past and the query runs today
- **THEN** the expansion does NOT add `brex` via that edge

#### Scenario: Edge weight differentiates source kinds
- **WHEN** `alice → brex` has `source_kind = 'frontmatter'` (`edge_weight = 1.0`) and `alice → scale` has `source_kind = 'wiki_link'` (`edge_weight = 0.5`), both 1 hop, both with the same parent score
- **THEN** `brex` ranks above `scale` in the expanded result set

### Requirement: Graph expansion is bounded
The graph-expansion pass SHALL cap the number of newly added candidates at `config.graph_expansion_max` (default `50`) per query. The total nodes visited during expansion SHALL be capped by the existing graph safety cap (`MAX_NODES = 1000`).

#### Scenario: Expansion stops at the per-query cap
- **WHEN** a candidate would expand to 200 reachable nodes within `graph_depth`
- **THEN** at most `graph_expansion_max` (default 50) are added to the result set, prioritized by score

#### Scenario: Expansion does not exceed `MAX_NODES`
- **WHEN** the cumulative visited-node count across expansions reaches `MAX_NODES`
- **THEN** further expansion stops and the existing top-K plus already-added expansions are returned

### Requirement: `graph_depth = 0` disables graph expansion
When `config.graph_depth` is `0`, `hybrid_search` SHALL return the FTS5 + vector top-K unchanged with no graph-expansion pass.

#### Scenario: Disabled expansion yields baseline behaviour
- **WHEN** `graph_depth = 0` and a query runs
- **THEN** the result set matches the v9 `hybrid_search` output, aside from unrelated schema columns that are not part of `SearchResult`

### Requirement: CLI and config exposure
The system SHALL expose graph-expansion knobs via:
- `config` table keys `graph_depth`, `graph_distance_decay`, `graph_expansion_max`, `edge_weight_frontmatter`, `edge_weight_entity_pattern`, `edge_weight_wikilink`.
- A `--hops N` flag on `quaid query` and `quaid search` that overrides `graph_depth` for a single invocation.

#### Scenario: Config keys read at query time
- **WHEN** the `config` table sets `graph_depth = 2` and a query runs without a CLI flag
- **THEN** the expansion runs to depth 2

#### Scenario: CLI flag overrides config
- **WHEN** the `config` table sets `graph_depth = 1` and the user invokes `quaid query --hops 3 "fintech investments"`
- **THEN** the expansion runs to depth 3 for this invocation only

#### Scenario: Config defaults populated at init
- **WHEN** `quaid init` creates a fresh v10 database
- **THEN** the `config` table contains `graph_depth = 0`, `graph_distance_decay = 0.5`, `graph_expansion_max = 50`, `edge_weight_frontmatter = 1.0`, `edge_weight_entity_pattern = 0.7`, and `edge_weight_wikilink = 0.5`
- **AND** graph expansion remains opt-in until the documented DAB §4 and MSMARCO benchmark gates publish passing numbers

### Requirement: Graph read surfaces expose path explanations
The `memory_graph` MCP tool and the `quaid graph <slug>` CLI SHALL include the path used to reach each expanded node in the result, expressed as a list of `(from_slug, relationship, to_slug)` triples. This is a pre-release response-shape change and SHALL NOT require backward-compatible output negotiation.

#### Scenario: Path returned for a 2-hop reachable node
- **WHEN** `quaid graph alice --depth 2` is invoked and `alice → brex → fintech-investor` is a valid active path
- **THEN** the result for `fintech-investor` includes a path field listing two triples: `(alice, founded, brex)` and `(brex, related, fintech-investor)`

#### Scenario: Path empty for the root slug
- **WHEN** `quaid graph alice --depth 2` is invoked
- **THEN** the result for `alice` itself contains an empty path list

#### Scenario: MCP graph output includes paths
- **WHEN** `memory_graph` returns JSON for a graph with reachable nodes
- **THEN** the JSON contains the existing `nodes` and `edges` fields plus a `paths` field keyed by reachable slug

### Requirement: Acceptance is gated on benchmark improvements
A release shipping graph-aware retrieval SHALL NOT pass retrieval acceptance unless the DAB §4 Semantic / Hybrid score improves by at least 8 points over a reproducible bge-small baseline (target: ≥ 35/50) AND MSMARCO P@5 improves by at least 5 points over a reproducible bge-small baseline.

#### Scenario: DAB §4 below threshold blocks retrieval acceptance
- **WHEN** a release candidate scores 32/50 on DAB §4 Semantic / Hybrid
- **THEN** the retrieval acceptance gate fails and the release is blocked or graph expansion remains disabled by default

#### Scenario: Both benchmarks meet thresholds → release passes the retrieval gate
- **WHEN** a release candidate scores 36/50 on DAB §4 and MSMARCO P@5 improves by 6 points over baseline
- **THEN** the retrieval-acceptance gate for this capability is satisfied

