## ADDED Requirements

### Requirement: validate command with modular checks

`quaid validate` SHALL run integrity checks on `memory.db` with the following flags:
- `--links`: check link interval non-overlap, temporal ordering (`valid_from <= valid_until`),
  and referential integrity (from_page_id/to_page_id reference existing pages)
- `--assertions`: check assertion dedup (no duplicate subject+predicate+object with
  overlapping validity), supersession chain validity (supersedes_id references exist),
  dangling references (asserted page_id exists)
- `--embeddings`: check exactly one active embedding model, all chunks reference active model,
  all vec_rowids resolve in the active model's vec table
- `--all`: run all checks
- No flags: equivalent to `--all`

Exit code 0 on clean check, exit code 1 on violations. `--json` outputs structured violation
array.

#### Scenario: Clean memory validation
- **WHEN** `quaid validate --all` is run on a consistent memory
- **THEN** exit code is 0 and output says "All checks passed"

#### Scenario: Broken link detected
- **WHEN** a link references a page_id that does not exist in `pages`
- **THEN** `quaid validate --links` reports the violation with the link ID and dangling page_id

#### Scenario: Stale embedding detected
- **WHEN** a chunk's vec_rowid does not resolve in the active model's vec table
- **THEN** `quaid validate --embeddings` reports the stale chunk

### Requirement: call command for raw MCP tool invocation

`quaid call <TOOL> <JSON>` SHALL invoke the named MCP tool handler directly, deserialize
the JSON input, call the tool function, and print the result to stdout. This does not start
the MCP server. The output SHALL be the tool's JSON result.

#### Scenario: Successful tool call
- **WHEN** `quaid call memory_get '{"slug":"people/alice"}'` is executed and the page exists
- **THEN** stdout contains the page content as JSON, exit code 0

#### Scenario: Unknown tool
- **WHEN** `quaid call unknown_tool '{}'` is executed
- **THEN** stderr contains an error message, exit code 1

### Requirement: pipe mode for JSONL streaming

`quaid pipe` SHALL read one JSON object per line from stdin. Each object has the shape
`{"tool": "<tool_name>", "input": {...}}`. For each line, the tool is invoked and the
result is written as one JSON object per line to stdout. Errors are JSON objects with
an `"error"` field. The pipe continues until EOF.

#### Scenario: Batch tool invocation
- **WHEN** stdin contains 3 JSONL lines with valid tool calls
- **THEN** stdout contains 3 JSONL lines with results

#### Scenario: Error in batch
- **WHEN** one line in the batch has an unknown tool
- **THEN** that line's output has an `"error"` field, other lines succeed normally

### Requirement: --json flag on all commands

Every CLI command that produces output SHALL support the `--json` flag for structured JSON
output. Commands that already support `--json` are unaffected. Commands that currently ignore
the flag SHALL be updated.

#### Scenario: validate --json
- **WHEN** `quaid validate --all --json` is run
- **THEN** output is a JSON object with `{"checks": [...], "violations": [...], "passed": true/false}`

#### Scenario: skills doctor --json
- **WHEN** `quaid skills doctor --json` is run
- **THEN** output is a JSON array of skill objects with `name`, `source`, `hash`, `shadowed` fields

### Requirement: memory_gap MCP tool

`memory_gap` SHALL log a knowledge gap with privacy-safe defaults. It accepts `query` (string)
and `context` (string). It stores `query_hash = sha256(query)`, `sensitivity = 'internal'`,
`query_text = NULL`. Returns the gap ID.

#### Scenario: Gap logged
- **WHEN** `memory_gap` is called with a query
- **THEN** a row is inserted in `knowledge_gaps` with `query_text = NULL`, `sensitivity = 'internal'`

#### Scenario: Duplicate gap
- **WHEN** `memory_gap` is called with the same query twice
- **THEN** the second call is a no-op (idempotent on query_hash)

### Requirement: memory_gaps MCP tool

`memory_gaps` SHALL list knowledge gaps. Accepts optional `resolved` (bool, default false)
and `limit` (int, default 20, max 1000). Returns JSON array of gap objects.

#### Scenario: List unresolved gaps
- **WHEN** `memory_gaps` is called with no arguments
- **THEN** returns up to 20 unresolved gaps ordered by creation date

### Requirement: memory_stats MCP tool

`memory_stats` SHALL return memory statistics as a JSON object with fields: `page_count`,
`link_count`, `assertion_count`, `contradiction_count`, `gap_count`, `embedding_count`,
`active_model`, `db_size_bytes`.

#### Scenario: Stats retrieved
- **WHEN** `memory_stats` is called
- **THEN** returns a JSON object with all statistic fields populated

### Requirement: memory_raw MCP tool

`memory_raw` SHALL insert a row into the `raw_data` table. Accepts `slug` (string),
`source` (string), and `data` (JSON object). Returns the row ID. If the page referenced
by slug does not exist, returns error code `-32001`.

#### Scenario: Raw data stored
- **WHEN** `memory_raw` is called with a valid slug, source, and data
- **THEN** a row is inserted in `raw_data` and the row ID is returned

#### Scenario: Unknown slug
- **WHEN** `memory_raw` is called with a slug that has no corresponding page
- **THEN** error code `-32001` is returned
