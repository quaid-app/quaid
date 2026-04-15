## ADDED Requirements

### Requirement: MCP stdio server
`gbrain serve` SHALL start an MCP server on stdio using the `rmcp` crate. The server
SHALL expose exactly 5 Phase 1 tools: `brain_get`, `brain_put`, `brain_query`,
`brain_search`, `brain_list`. Each tool SHALL delegate to the same core functions used
by the CLI. The server SHALL run until the stdio stream is closed.

#### Scenario: Server starts and lists tools
- **WHEN** `gbrain serve` is started and an MCP `initialize` + `tools/list` request is sent
- **THEN** the server responds with the 5 tool definitions in the MCP protocol format

#### Scenario: Server handles tool calls
- **WHEN** a valid `tools/call` request for `brain_get` is received with `{"slug": "people/alice"}`
- **THEN** the server returns the page content in the MCP response format

#### Scenario: Server exits on stdin close
- **WHEN** the stdin stream is closed (client disconnects)
- **THEN** the server exits cleanly with code 0

### Requirement: brain_get tool
`brain_get` SHALL accept `{"slug": "<slug>"}` and return the page content as rendered
markdown including frontmatter. If the page does not exist, it SHALL return a JSON-RPC
error with an appropriate error code.

#### Scenario: Get existing page via MCP
- **WHEN** `brain_get({"slug": "people/alice"})` is called and the page exists
- **THEN** the response contains the rendered page markdown

#### Scenario: Get non-existent page via MCP
- **WHEN** `brain_get({"slug": "people/nobody"})` is called and the page does not exist
- **THEN** the response is a JSON-RPC error with code `-32001` (not found)

### Requirement: brain_put tool
`brain_put` SHALL accept `{"slug": "<slug>", "content": "<markdown>", "expected_version": <N>}`.
`expected_version` is optional. If provided, OCC is enforced. The response SHALL always
include the resulting `version` on success.

#### Scenario: Create page via MCP
- **WHEN** `brain_put({"slug": "people/bob", "content": "# Bob\n..."})` is called
  and the page does not exist
- **THEN** the page is created and the response includes `{"version": 1}`

#### Scenario: OCC conflict via MCP
- **WHEN** `brain_put({"slug": "people/alice", "content": "...", "expected_version": 1})`
  is called and the stored version is `2`
- **THEN** the response is a JSON-RPC error with code `-32009` and data `{"current_version": 2}`

### Requirement: brain_query tool
`brain_query` SHALL accept `{"query": "<text>", "limit": <N>, "wing": "<wing>"}` (limit
and wing are optional, defaults: limit=10, wing=None) and return hybrid search results
as a JSON array of `{"slug": "...", "summary": "...", "score": ...}` objects.

#### Scenario: Semantic query via MCP
- **WHEN** `brain_query({"query": "AI researchers I know"})` is called
- **THEN** the response contains up to 10 result objects ordered by relevance score

#### Scenario: Wing-filtered query via MCP
- **WHEN** `brain_query({"query": "machine learning", "wing": "projects"})` is called
- **THEN** only pages with `wing = 'projects'` appear in results

### Requirement: brain_search tool
`brain_search` SHALL accept `{"query": "<text>", "limit": <N>, "wing": "<wing>"}` and
return FTS5 keyword search results as a JSON array (same format as `brain_query`).

#### Scenario: FTS keyword search via MCP
- **WHEN** `brain_search({"query": "Series A fundraising"})` is called
- **THEN** the response contains FTS5 BM25-ranked results in JSON format

### Requirement: brain_list tool
`brain_list` SHALL accept `{"wing": "<wing>", "type": "<type>", "limit": <N>}` (all
optional) and return a JSON array of pages matching the filters.

#### Scenario: List all pages via MCP
- **WHEN** `brain_list({})` is called
- **THEN** a JSON array of up to 50 pages (default limit) is returned

#### Scenario: List filtered by type via MCP
- **WHEN** `brain_list({"type": "person"})` is called
- **THEN** only pages with `type = 'person'` are returned

### Requirement: MCP error codes
The MCP server SHALL use the following JSON-RPC error codes for application errors:
- `-32001`: Not found (page does not exist)
- `-32002`: Parse error (invalid markdown / bad input)
- `-32003`: Database error (unexpected SQLite error)
- `-32009`: OCC conflict (expected_version mismatch)

#### Scenario: Consistent error code mapping
- **WHEN** any tool call results in a `core::OccError`
- **THEN** the JSON-RPC response uses error code `-32009` with the current version in the error data
