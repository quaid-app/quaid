## MODIFIED Requirements

### Requirement: Conversation file format uses frontmatter and ordered turn blocks
The system SHALL render conversation files with YAML frontmatter containing `type: conversation`, `session_id`, `date`, `started_at`, `status` (`open` or `closed`), `last_extracted_at`, and `last_extracted_turn` (integer cursor, default `0`). The body SHALL consist of turn blocks separated by horizontal rules (`---`). Each turn block SHALL begin with a heading `## Turn <N> · <role> · <timestamp>` and contain the turn's `content` followed by an optional metadata code fence using the canonical info string ````json turn-metadata```` so ordinary trailing JSON code fences remain content. The cursor `last_extracted_turn` SHALL track the highest turn ordinal that has been processed by extraction and SHALL be updated only by extraction (proposal #2), not by `memory_add_turn`.

Canonical conversation files SHALL also remain safe to re-ingest through the watcher: the resulting page content SHALL NOT produce blank embedding chunks or empty embedding inputs solely because the rendered body begins with a separator newline after frontmatter.

#### Scenario: Canonical single-turn day-file does not create a blank embedding chunk
- **WHEN** a conversation day-file created by `memory_add_turn` is re-ingested through the watcher with a single turn in its body
- **THEN** the resulting page chunks are all non-blank
- **AND** embedding refresh does not fail with `input text is empty`
