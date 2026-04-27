## ADDED Requirements

### Requirement: MCP server registers Phase 3 tools

The MCP server SHALL register four new tools: `memory_gap`, `memory_gaps`, `memory_stats`,
`memory_raw`. These tools bring the total MCP surface to 16 tools. All new tools SHALL
follow the same error code conventions as Phase 2 tools (`-32001` for not-found, `-32003`
for DB errors).

#### Scenario: Tool discovery
- **WHEN** an MCP client connects and requests tool listing
- **THEN** all 16 tools are listed: memory_get, memory_put, memory_query, memory_search,
  memory_list, memory_link, memory_link_close, memory_backlinks, memory_graph, memory_check,
  memory_timeline, memory_tags, memory_gap, memory_gaps, memory_stats, memory_raw

#### Scenario: memory_gap input validation
- **WHEN** `memory_gap` is called with an empty query string
- **THEN** error code `-32602` (invalid params) is returned

#### Scenario: memory_raw slug validation
- **WHEN** `memory_raw` is called with a slug containing invalid characters
- **THEN** error code `-32602` (invalid params) is returned

### Requirement: memory_gap privacy-safe defaults

`memory_gap` SHALL NOT accept a `sensitivity` parameter. Sensitivity is always `'internal'`
at creation. The raw query text is NOT stored — only `sha256(query)` is stored in `query_hash`.
`query_text` remains NULL until a separate approval flow populates it.

#### Scenario: Privacy enforcement
- **WHEN** `memory_gap` is called with any input
- **THEN** the stored row has `sensitivity = 'internal'` and `query_text = NULL`
