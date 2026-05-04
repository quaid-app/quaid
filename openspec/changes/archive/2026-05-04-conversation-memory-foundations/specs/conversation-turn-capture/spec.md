## ADDED Requirements

### Requirement: `memory_add_turn` MCP tool appends turns to a per-session vault file
The system SHALL expose an MCP tool `memory_add_turn` that accepts `{session_id: string, role: "user"|"assistant"|"system"|"tool", content: string, timestamp?: ISO-8601, metadata?: object}` and SHALL append the turn to a vault-resident markdown file at `<vault>/conversations/<YYYY-MM-DD>/<session-id>.md`, where `<YYYY-MM-DD>` is derived from the turn's timestamp (server `now` when the timestamp is omitted). The append SHALL be durable: the file SHALL exist on disk with the new turn block before the call returns. Concurrent appends for the same session SHALL serialize across processes so ordinals remain unique and ordered. The tool SHALL return synchronously with `{turn_id, conversation_path, extraction_scheduled_at}`. The implementation SHALL NOT invoke any SLM on the request path.

#### Scenario: First turn for a session creates the conversation file
- **WHEN** `memory_add_turn` is called with `session_id="s1"`, `role="user"`, `content="hi"`, `timestamp="2026-05-03T09:14:22Z"` and no file exists yet
- **THEN** the file `<vault>/conversations/2026-05-03/s1.md` is created with conversation frontmatter and a single turn block, and the response includes `turn_id="s1:1"` and `conversation_path="conversations/2026-05-03/s1.md"`

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

### Requirement: Conversation file format uses frontmatter and ordered turn blocks
The system SHALL render conversation files with YAML frontmatter containing `type: conversation`, `session_id`, `date`, `started_at`, `status` (`open` or `closed`), `last_extracted_at`, and `last_extracted_turn` (integer cursor, default `0`). The body SHALL consist of turn blocks separated by horizontal rules (`---`). Each turn block SHALL begin with a heading `## Turn <N> · <role> · <timestamp>` and contain the turn's `content` followed by an optional metadata code fence using the canonical info string ````json turn-metadata```` so ordinary trailing JSON code fences remain content. The cursor `last_extracted_turn` SHALL track the highest turn ordinal that has been processed by extraction and SHALL be updated only by extraction (proposal #2), not by `memory_add_turn`.

#### Scenario: Conversation file frontmatter contains required keys at creation
- **WHEN** a new conversation file is created by the first `memory_add_turn` call
- **THEN** its frontmatter contains `type: conversation`, the supplied `session_id`, `date` matching the file's directory, `started_at` matching the first turn's timestamp, `status: open`, `last_extracted_turn: 0`, and `last_extracted_at: null`

#### Scenario: Turn block round-trips through parse-render
- **WHEN** a conversation file with N turn blocks is parsed and re-rendered
- **THEN** the resulting file is byte-identical to the input under the canonical render rules (whitespace-normalised), and ordinal numbering, role values, and timestamps are preserved

### Requirement: Multi-day sessions continue turn ordinals across day-boundary files
When `memory_add_turn` is called for a `session_id` whose most recent turn lives in a previous day's file, the system SHALL create or append to a file under the new day's directory, and SHALL assign the new turn an ordinal that is one greater than the highest ordinal observed for that session across all of its day-files. Turn ordinals SHALL be globally unique within a session, never restarting at 1 for a continuing session.

#### Scenario: Session that crosses midnight writes a new day-file with continuing ordinals
- **WHEN** turns 1..47 exist in `conversations/2026-05-03/s1.md` and a new turn for `session_id="s1"` arrives with timestamp `2026-05-04T00:01:00Z`
- **THEN** a file `conversations/2026-05-04/s1.md` is created (or appended to) and the new turn's ordinal is `48`, not `1`

#### Scenario: Per-file cursor is independent across day-files
- **WHEN** day-1's file has `last_extracted_turn: 47` and day-2's file is freshly created
- **THEN** day-2's frontmatter `last_extracted_turn` is `0` and the two files' cursors advance independently

### Requirement: Conversation file paths are namespace-aware
When namespace isolation (`#137`) is in use, the system SHALL nest conversation files under the namespace directory: `<vault>/<namespace>/conversations/<YYYY-MM-DD>/<session-id>.md`. The `session_id` SHALL be namespace-local; identical `session_id` values in different namespaces SHALL produce distinct files and SHALL NOT merge.

#### Scenario: Same session_id in two namespaces stores in two distinct files
- **WHEN** `memory_add_turn` is called with `session_id="main"` in namespace `alpha` and again with `session_id="main"` in namespace `beta`
- **THEN** the two turns are appended to `<vault>/alpha/conversations/<date>/main.md` and `<vault>/beta/conversations/<date>/main.md` respectively, and neither file contains both turns

### Requirement: `memory_close_session` MCP tool flushes extraction and marks the session closed
The system SHALL expose an MCP tool `memory_close_session` that accepts `{session_id: string}` and SHALL: (a) update the most recent open day-file for the session to `status: closed` in frontmatter, (b) enqueue an immediate (non-debounced) extraction job (proposal #2 supplies the worker), and (c) return `{closed_at, extraction_triggered, queue_position}`. Re-closing an already-closed session SHALL be idempotent and return the original `closed_at`.

#### Scenario: Closing an open session marks the latest file as closed
- **WHEN** `memory_close_session` is called for a session whose latest file has `status: open`
- **THEN** that file's frontmatter is updated to `status: closed` and the response includes the `closed_at` timestamp

#### Scenario: Re-closing an already-closed session is idempotent
- **WHEN** `memory_close_session` is called twice for the same `session_id`
- **THEN** the second call returns the same `closed_at` timestamp as the first and does not modify the file again

#### Scenario: Closing an unknown session returns `NotFoundError`
- **WHEN** `memory_close_session` is called with a `session_id` for which no conversation file exists in the active namespace
- **THEN** the tool returns a `NotFoundError`

### Requirement: `memory_close_action` MCP tool updates an action_item's lifecycle in place
The system SHALL expose an MCP tool `memory_close_action` that accepts `{slug: string, status: "done"|"cancelled", note?: string}` and SHALL update the page's `status` frontmatter field in place using the existing optimistic-concurrency machinery. This SHALL be the only in-place mutation supported on the new fact page types; all other content changes SHALL go through the supersede chain. If the page is not `type: action_item`, the tool SHALL return `KindError`.

#### Scenario: Closing an open action item updates status in place
- **WHEN** `memory_close_action` is called with `{slug: "ship-phase5", status: "done"}` and that slug's page is `type: action_item, status: open`
- **THEN** the page's frontmatter `status` becomes `done`, the page's version increments, and the response includes the new `version`

#### Scenario: Closing a non-action page returns `KindError`
- **WHEN** `memory_close_action` is called with a slug whose page is `type: preference`
- **THEN** the tool returns `KindError` and the page is unchanged

#### Scenario: Optimistic concurrency clash returns `ConflictError`
- **WHEN** `memory_close_action` reads version `v` and another writer increments to `v+1` before the close commits
- **THEN** the tool returns `ConflictError` and the action item is unchanged

### Requirement: `memory_add_turn` enqueues extraction non-blockingly
On a successful turn append, `memory_add_turn` SHALL enqueue a debounced extraction job for the session via the `extraction-queue` capability and SHALL return without awaiting any extraction work. When `extraction.enabled = false`, the implementation MAY skip enqueue and return `extraction_scheduled_at: null` in the response. The request-path latency budget SHALL allow `memory_add_turn` p95 < 50 ms on representative SSD hardware (one append + fsync + one queue UPSERT, no SLM call).

#### Scenario: Enqueue is decoupled from caller latency
- **WHEN** `memory_add_turn` is called and the extraction queue holds a slow or failing job for the same session
- **THEN** the call still returns within the latency budget and the queue's prior state is not visible in the response

#### Scenario: Enqueue is skipped when extraction is disabled
- **WHEN** `memory_add_turn` is called with `extraction.enabled = false` configured
- **THEN** no row is inserted into `extraction_queue` and the response's `extraction_scheduled_at` is `null`
