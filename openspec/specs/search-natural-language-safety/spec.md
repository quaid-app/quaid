# search-natural-language-safety Specification

## Purpose
TBD - created by archiving change fts5-search-robustness. Update Purpose after archive.
## Requirements
### Requirement: quaid search sanitizes natural-language input by default
`quaid search` SHALL apply `sanitize_fts_query` to the query before passing it to
`search_fts` unless the `--raw` flag is explicitly provided.

#### Scenario: Question mark in query
- **WHEN** user runs `quaid search "what is CLARITY?"`
- **THEN** the query executes without error and returns results (or empty list)

#### Scenario: Apostrophe in query
- **WHEN** user runs `quaid search "it's a stablecoin"`
- **THEN** the query executes without error and returns results (or empty list)

#### Scenario: Percent sign in query
- **WHEN** user runs `quaid search "50% fee reduction"`
- **THEN** the query executes without error and returns results (or empty list)

#### Scenario: Dotted version number in query
- **WHEN** user runs `quaid search "gpt-5.4 codex model"`
- **THEN** the query executes without error and returns results (or empty list)

### Requirement: quaid search --raw preserves expert FTS5 syntax
When `--raw` is provided, `quaid search` SHALL pass the query verbatim to `search_fts`
without sanitization, enabling full FTS5 operator syntax.

#### Scenario: Expert FTS5 query via --raw
- **WHEN** user runs `quaid search --raw '"exact phrase" AND rust*'`
- **THEN** the query is passed unsanitized to FTS5 and matched using expert FTS5 semantics

### Requirement: quaid search --json returns valid JSON on error
When both `--json` and `--raw` are active and `search_fts` returns an error (invalid FTS5),
`quaid search` SHALL output `{"error": "<message>"}` to stdout rather than propagating
the error to stderr with no stdout output.

#### Scenario: Invalid FTS5 with --raw --json
- **WHEN** user runs `quaid search --raw --json "?invalid"`
- **THEN** stdout contains valid JSON `{"error": "..."}` describing the parse failure

### Requirement: MCP memory_search sanitizes natural-language input
The MCP `memory_search` tool handler SHALL apply `sanitize_fts_query` to the `query`
parameter before passing it to `search_fts`.

#### Scenario: Agent sends punctuated query
- **WHEN** an MCP client calls `memory_search` with query `"what is clarity?"`
- **THEN** the tool executes without error and returns a valid JSON-RPC response

