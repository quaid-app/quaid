## MODIFIED Requirements

### Requirement: `memory_add_turn` MCP tool appends turns to a per-session vault file
The system SHALL expose an MCP tool `memory_add_turn` that accepts `{session_id: string, role: "user"|"assistant"|"system"|"tool", content: string, timestamp?: ISO-8601, metadata?: object}` and SHALL append the turn to a vault-resident markdown file at `<vault>/conversations/<YYYY-MM-DD>/<session-id>.md`, where `<YYYY-MM-DD>` is derived from the turn's timestamp (server `now` when the timestamp is omitted). The append SHALL be durable: the file SHALL exist on disk with the new turn block before the call returns. Concurrent appends for the same session SHALL serialize across processes so ordinals remain unique and ordered. The tool SHALL return synchronously with `{turn_id, conversation_path, extraction_scheduled_at}`. The implementation SHALL NOT invoke any SLM on the request path.

On a fresh initialized database with default settings, callers SHALL NOT need to manually create or attach a collection before first use. `memory_add_turn` SHALL succeed by writing under the default writable write-target rooted at `~/.quaid/vault`.

#### Scenario: First turn for a session creates the conversation file
- **WHEN** `memory_add_turn` is called with `session_id="s1"`, `role="user"`, `content="hi"`, `timestamp="2026-05-03T09:14:22Z"` and no file exists yet
- **THEN** the file `<vault>/conversations/2026-05-03/s1.md` is created with conversation frontmatter and a single turn block, and the response includes `turn_id="s1:1"` and `conversation_path="conversations/2026-05-03/s1.md"`

#### Scenario: Fresh initialized DB works without manual collection bootstrap
- **WHEN** a user has a freshly initialized database with no manual `collection add` and calls `memory_add_turn` for a new session
- **THEN** the call succeeds and writes to `~/.quaid/vault/conversations/<YYYY-MM-DD>/<session-id>.md` via the default writable write-target
- **AND** no `ConfigError` is returned for missing writable collection root

#### Scenario: Subsequent turn appends to the same file with the next ordinal
- **WHEN** `memory_add_turn` is called twice in succession for the same `session_id` on the same calendar day
- **THEN** the second call appends a second turn block to the same file with `turn_id="<session>:2"` and the file contains both turn blocks in order

#### Scenario: Append is durable before the call returns
- **WHEN** `memory_add_turn` returns successfully
- **THEN** the appended turn content is observable on disk by an independent file read

#### Scenario: Same-session concurrent appends serialize across processes
- **WHEN** two processes append turns concurrently for the same `session_id`
- **THEN** exactly one append holds the session lock at a time, and the resulting file contains both turns with distinct ordinals in append order

#### Scenario: Caller-supplied metadata is preserved verbatim in the turn block
- **WHEN** `memory_add_turn` is called with `metadata={"tool_name": "bash", "importance": "high"}`
- **THEN** the rendered turn block in the conversation file preserves the metadata fields and they survive a parse-then-render round-trip
