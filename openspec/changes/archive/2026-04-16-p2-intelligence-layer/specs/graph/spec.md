## ADDED Requirements

### Requirement: N-hop graph neighbourhood traversal
`src/core/graph.rs` SHALL implement `neighborhood_graph(slug, depth, temporal_filter, conn)`
that performs an iterative BFS over the `links` table, returning a `GraphResult` containing
a deduplicated node list and an edge list. Depth SHALL be capped at 10 regardless of the
caller-supplied argument. A `HashSet<i64>` visited set SHALL prevent cycles.

#### Scenario: Single-hop neighbourhood from a known page
- **WHEN** `neighborhood_graph("people/alice", 1, TemporalFilter::Active, conn)` is called
- **THEN** the result contains `people/alice` as the root node plus all pages reachable via
  one active outbound link from alice, with the edge list showing (from_slug, to_slug, relationship)

#### Scenario: Zero-hop returns only the root page
- **WHEN** `neighborhood_graph("people/alice", 0, TemporalFilter::Active, conn)` is called
- **THEN** the result contains exactly one node (`people/alice`) and an empty edge list

#### Scenario: Cyclic graph does not loop forever
- **WHEN** pages A → B and B → A exist and `neighborhood_graph("a", 10, TemporalFilter::All, conn)` is called
- **THEN** the function returns without panic; each of A and B appears exactly once in the node list

#### Scenario: Temporal filter excludes closed links by default
- **WHEN** a link from alice to acme has `valid_until = '2020-01-01'` (past) and
  `neighborhood_graph("people/alice", 1, TemporalFilter::Active, conn)` is called
- **THEN** acme does NOT appear in the result node list

#### Scenario: All-history filter includes closed links
- **WHEN** the same past-closed link exists and `neighborhood_graph("people/alice", 1, TemporalFilter::All, conn)` is called
- **THEN** acme appears in the result node list

#### Scenario: Non-existent root slug returns a not-found error
- **WHEN** `neighborhood_graph("people/ghost", 1, TemporalFilter::Active, conn)` is called
- **THEN** the function returns `Err(GraphError::PageNotFound)`

### Requirement: Graph CLI command
`src/commands/graph.rs` SHALL implement `run(db, slug, depth, temporal, json)` that calls
`neighborhood_graph` and prints output. Text output shows nodes as a tree with edge annotations.
JSON output emits `{"nodes": [...], "edges": [...]}`.

#### Scenario: Human-readable graph output
- **WHEN** `quaid graph people/alice --depth 2` is called
- **THEN** stdout shows the root slug, then indented reachable slugs with their relationship label

#### Scenario: JSON graph output
- **WHEN** `quaid graph people/alice --depth 2 --json` is called
- **THEN** stdout is a valid JSON object with `nodes` array (each with `slug`, `type`, `title`)
  and `edges` array (each with `from`, `to`, `relationship`, `valid_from`, `valid_until`)

#### Scenario: Unknown page exits with an error message
- **WHEN** `quaid graph nobody/ghost` is called
- **THEN** stderr shows "page not found: nobody/ghost" and exit code is non-zero
