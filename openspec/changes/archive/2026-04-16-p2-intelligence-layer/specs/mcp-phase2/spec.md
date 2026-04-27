## ADDED Requirements

### Requirement: memory_link MCP tool
`src/mcp/server.rs` SHALL expose a `memory_link` tool that creates a typed temporal link
between two pages. Input: `from_slug`, `to_slug`, `relationship`, optional `valid_from`,
optional `valid_until`. Delegates to `commands::link::run`. Returns success text or
`-32001` if either slug is not found.

#### Scenario: Successful link creation
- **WHEN** `memory_link` is called with `from_slug="people/alice"`, `to_slug="companies/acme"`,
  `relationship="works_at"`, `valid_from="2024-01"`
- **THEN** a link row is inserted and the tool returns a success content with a confirmation message

#### Scenario: Unknown from_slug returns not-found error
- **WHEN** `memory_link` is called with `from_slug="people/ghost"` which does not exist
- **THEN** the tool returns `ErrorCode(-32001)` with "page not found" in the message

### Requirement: memory_link_close MCP tool
`src/mcp/server.rs` SHALL expose a `memory_link_close` tool that closes a link by ID.
Input: `link_id` (integer), `valid_until` (date string). Delegates to `commands::link::close`.
Returns success text or `-32001` if link_id is not found.

#### Scenario: Successful link close
- **WHEN** `memory_link_close` is called with a valid `link_id` and `valid_until="2025-06"`
- **THEN** the link's `valid_until` is updated and the tool returns confirmation

#### Scenario: Unknown link_id returns not-found error
- **WHEN** `memory_link_close` is called with `link_id=99999`
- **THEN** the tool returns `ErrorCode(-32001)` with "link not found" in the message

### Requirement: memory_backlinks MCP tool
`src/mcp/server.rs` SHALL expose a `memory_backlinks` tool that returns inbound links for a page.
Input: `slug`, optional `temporal` filter (`"active"` | `"all"`, default `"active"`).
Delegates to `commands::link::backlinks`. Returns JSON array of link objects.

#### Scenario: Lists inbound links for a known page
- **WHEN** `memory_backlinks` is called with `slug="companies/acme"`
- **THEN** the tool returns a JSON array of link objects with `from_slug`, `relationship`,
  `valid_from`, `valid_until` fields

#### Scenario: Unknown slug returns not-found error
- **WHEN** `memory_backlinks` is called with `slug="nobody/ghost"`
- **THEN** the tool returns `ErrorCode(-32001)`

### Requirement: memory_graph MCP tool
`src/mcp/server.rs` SHALL expose a `memory_graph` tool that returns the N-hop neighbourhood
graph for a page. Input: `slug`, optional `depth` (integer, default 2, max 10), optional
`temporal` (`"active"` | `"all"`, default `"active"`). Delegates to `core::graph::neighborhood_graph`.
Returns JSON `{"nodes": [...], "edges": [...]}`.

#### Scenario: Graph result returned as JSON
- **WHEN** `memory_graph` is called with `slug="people/alice"`, `depth=2`
- **THEN** the tool returns a JSON object with `nodes` and `edges` arrays

#### Scenario: Unknown slug returns not-found error
- **WHEN** `memory_graph` is called with `slug="people/ghost"`
- **THEN** the tool returns `ErrorCode(-32001)`

### Requirement: memory_check MCP tool
`src/mcp/server.rs` SHALL expose a `memory_check` tool that detects contradictions.
Input: optional `slug` (if absent, checks all pages). Delegates to `core::assertions::check_assertions`.
Returns JSON array of contradiction objects.

#### Scenario: Check returns detected contradictions
- **WHEN** `memory_check` is called with `slug="people/alice"` and there are contradictions
- **THEN** the tool returns a JSON array of contradiction objects

#### Scenario: Clean page returns empty array
- **WHEN** `memory_check` is called with `slug="people/alice"` and no contradictions exist
- **THEN** the tool returns an empty JSON array `[]`

### Requirement: memory_timeline MCP tool
`src/mcp/server.rs` SHALL expose a `memory_timeline` tool that reads timeline entries for a page.
Input: `slug`, optional `limit` (default 20). Delegates to `commands::timeline::run`.
Returns JSON array of timeline entry strings.

#### Scenario: Timeline entries returned
- **WHEN** `memory_timeline` is called with `slug="people/alice"` and timeline entries exist
- **THEN** the tool returns a JSON array of entry strings ordered by date DESC

#### Scenario: Unknown slug returns not-found error
- **WHEN** `memory_timeline` is called with `slug="nobody/ghost"`
- **THEN** the tool returns `ErrorCode(-32001)`

### Requirement: memory_tags MCP tool
`src/mcp/server.rs` SHALL expose a `memory_tags` tool that reads, adds, or removes tags.
Input: `slug`, optional `add` (array of tag strings), optional `remove` (array of tag strings).
If both `add` and `remove` are absent, the tool lists current tags.
Delegates to `commands::tags`. Returns JSON array of current tags after the operation.

#### Scenario: List tags for a page
- **WHEN** `memory_tags` is called with `slug="people/alice"` and no add/remove
- **THEN** the tool returns a JSON array of current tag strings

#### Scenario: Add a tag
- **WHEN** `memory_tags` is called with `slug="people/alice"`, `add=["investor"]`
- **THEN** "investor" is inserted; the tool returns the updated tag list

#### Scenario: Remove a tag
- **WHEN** `memory_tags` is called with `slug="people/alice"`, `remove=["investor"]`
- **THEN** "investor" is deleted; the tool returns the updated tag list (without "investor")

#### Scenario: Unknown slug returns not-found error
- **WHEN** `memory_tags` is called with `slug="nobody/ghost"`
- **THEN** the tool returns `ErrorCode(-32001)`
