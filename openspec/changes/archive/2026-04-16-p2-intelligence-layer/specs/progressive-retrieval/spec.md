## ADDED Requirements

### Requirement: Progressive retrieval with token-budget gating
`src/core/progressive.rs` SHALL implement `progressive_retrieve(initial_results, budget, depth, conn)`
that takes an initial `Vec<SearchResult>` from `hybrid_search`, approximates token counts as
`len(compiled_truth) / 4`, and expands the result set by following outbound links from the top
results until the cumulative token budget is exhausted or `depth` hops are reached (max 3).
The returned `Vec<SearchResult>` SHALL be ordered by relevance score (initial results first,
then expansion results by link distance).

#### Scenario: Budget exhausted before depth cap
- **WHEN** initial results total 3500 tokens, budget is 4000, and expansion would add 1000 more tokens
- **THEN** progressive_retrieve returns the initial results plus partial expansion up to the budget;
  no result that would exceed the budget is included

#### Scenario: Depth cap reached before budget exhaustion
- **WHEN** budget is very large (e.g. 100_000) and depth is 1
- **THEN** only direct neighbours of initial results are added; no second-hop expansion occurs

#### Scenario: Empty initial results return empty vec
- **WHEN** initial_results is empty
- **THEN** progressive_retrieve returns an empty Vec without error

#### Scenario: Duplicate pages from expansion are deduplicated
- **WHEN** two initial results both link to the same page
- **THEN** that page appears exactly once in the expanded result set

### Requirement: memory_query depth flag triggers progressive retrieval
`memory_query` (both CLI and MCP) SHALL accept `--depth auto` (CLI) or `"depth": "auto"` (MCP).
When depth is `auto`, the query pipeline calls `progressive_retrieve` after `hybrid_search`
using the `default_token_budget` value from the `config` table.
When depth is absent or `0`, behaviour is unchanged from Phase 1 (no expansion).

#### Scenario: memory_query --depth auto expands results
- **WHEN** `quaid query "who runs Acme" --depth auto` is called against a memory where
  companies/acme links to people/alice
- **THEN** the output includes both the companies/acme result and the people/alice expansion

#### Scenario: memory_query without depth is unchanged
- **WHEN** `quaid query "who runs Acme"` is called (no --depth flag)
- **THEN** behaviour is identical to Phase 1: only direct hybrid_search results are returned

### Requirement: Palace room classification
`src/core/palace.rs::derive_room` SHALL be implemented to return the first `##`-level heading
from the content, lowercased and converted to kebab-case (spaces → hyphens, strip non-alphanumeric
except hyphens). Pages with no `##` heading SHALL return `""` (unchanged from Phase 1 behaviour).

#### Scenario: Room derived from first h2 heading
- **WHEN** compiled_truth begins with `## Current Role\n...`
- **THEN** `derive_room` returns `"current-role"`

#### Scenario: No h2 heading returns empty string
- **WHEN** compiled_truth has no `##` heading
- **THEN** `derive_room` returns `""`

#### Scenario: derive_room updates are persisted on put and ingest
- **WHEN** a page is written with `memory_put` or ingested with `quaid ingest`
- **THEN** the `room` column in the `pages` table reflects the value of `derive_room(compiled_truth)`
