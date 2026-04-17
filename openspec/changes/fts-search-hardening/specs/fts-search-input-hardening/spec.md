## FTS Search Input Hardening Spec

### Requirement: CLI search accepts natural-language punctuation

The `gbrain search` command SHALL accept ordinary natural-language punctuation without surfacing
FTS5 parser failures to the user.

#### Scenario: question-mark query succeeds

- **WHEN** a user runs `gbrain search "what is CLARITY?"`
- **THEN** the command exits successfully
- **AND** it returns normal search output or `No results found.`
- **AND** it does not print an FTS5 syntax or parse error

#### Scenario: apostrophe and percent queries succeed

- **WHEN** a user runs `gbrain search "it's a stablecoin"` or `gbrain search "50% fee reduction"`
- **THEN** the command exits successfully
- **AND** it does not print an FTS5 syntax or parse error

### Requirement: CLI JSON search returns valid JSON for dotted version tokens

The `gbrain search --json` command SHALL return valid JSON when the query contains dotted
version-number-style tokens.

#### Scenario: dotted version query in JSON mode

- **WHEN** a user runs `gbrain --json search "gpt-5.4 codex model"`
- **THEN** the command exits successfully
- **AND** stdout is valid JSON
- **AND** it does not emit an FTS5 parse failure

### Requirement: MCP brain_search applies the same hardening contract

The MCP `brain_search` tool SHALL apply the same natural-language-safe input handling as the CLI
search command.

#### Scenario: MCP search with punctuation succeeds

- **WHEN** a client calls `brain_search` with `query = "what is CLARITY?"`
- **THEN** the tool call succeeds
- **AND** the returned text content is valid JSON
- **AND** the error message `invalid search query` is not returned

#### Scenario: MCP search with dotted version token succeeds

- **WHEN** a client calls `brain_search` with `query = "gpt-5.4 codex model"`
- **THEN** the tool call succeeds
- **AND** the returned text content is valid JSON

### Requirement: empty-after-sanitize queries fail soft

Queries that normalize to no searchable terms SHALL return empty results, not an error.

#### Scenario: punctuation-only query

- **WHEN** a user or MCP client submits a query like `???***`
- **THEN** the operation succeeds
- **AND** the result set is empty
