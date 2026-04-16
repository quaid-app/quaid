## ADDED Requirements

### Requirement: MCP server registers Phase 3 tools

The MCP server SHALL register four new tools: `brain_gap`, `brain_gaps`, `brain_stats`,
`brain_raw`. These tools bring the total MCP surface to 16 tools. All new tools SHALL
follow the same error code conventions as Phase 2 tools (`-32001` for not-found, `-32003`
for DB errors).

#### Scenario: Tool discovery
- **WHEN** an MCP client connects and requests tool listing
- **THEN** all 16 tools are listed: brain_get, brain_put, brain_query, brain_search,
  brain_list, brain_link, brain_link_close, brain_backlinks, brain_graph, brain_check,
  brain_timeline, brain_tags, brain_gap, brain_gaps, brain_stats, brain_raw

#### Scenario: brain_gap input validation
- **WHEN** `brain_gap` is called with an empty query string
- **THEN** error code `-32602` (invalid params) is returned

#### Scenario: brain_raw slug validation
- **WHEN** `brain_raw` is called with a slug containing invalid characters
- **THEN** error code `-32602` (invalid params) is returned

### Requirement: brain_gap privacy-safe defaults

`brain_gap` SHALL NOT accept a `sensitivity` parameter. Sensitivity is always `'internal'`
at creation. The raw query text is NOT stored — only `sha256(query)` is stored in `query_hash`.
`query_text` remains NULL until a separate approval flow populates it.

#### Scenario: Privacy enforcement
- **WHEN** `brain_gap` is called with any input
- **THEN** the stored row has `sensitivity = 'internal'` and `query_text = NULL`
