## ADDED Requirements

### Requirement: Pages support an ADD-only supersede chain via `superseded_by`
The system SHALL add a nullable `superseded_by INTEGER REFERENCES pages(id)` column to the `pages` table. A page with `superseded_by IS NULL` SHALL be the head of its chain (the latest version). A page with a non-null `superseded_by` SHALL be a historical (non-head) version. New page writes that supersede an existing page SHALL set the new page's frontmatter `supersedes: <prior_slug>` and SHALL update the prior page's `superseded_by` to point to the new page's `id`. The system SHALL NOT mutate a non-head page's content or frontmatter outside of explicit history-rebuild operations.

#### Scenario: Fresh v8 schema includes the `superseded_by` column
- **WHEN** `quaid init` creates a fresh v8 database
- **THEN** `PRAGMA table_info(pages)` reports a `superseded_by` column with type `INTEGER` and a foreign-key reference to `pages(id)`

#### Scenario: Writing a successor updates both ends of the chain
- **WHEN** page `B` is written with frontmatter `supersedes: <slug-of-A>` and page `A` exists as a head
- **THEN** page `B` is inserted with `superseded_by = NULL` and page `A` is updated to `superseded_by = B.id`, and page `A`'s `content` and `frontmatter` are otherwise unchanged

#### Scenario: A page can have only one direct successor
- **WHEN** an attempt is made to write a second page that supersedes an already-superseded page `A`
- **THEN** the write is rejected (the system requires the caller to supersede the current head, not a historical version)

### Requirement: Partial index makes head-only filtering free
The system SHALL create a partial index `idx_pages_supersede_head ON pages(type, superseded_by) WHERE superseded_by IS NULL` so that "select head pages of a given page type" is a single indexed lookup. A second guarded partial index `idx_pages_session ON pages(json_extract(IIF(json_valid(frontmatter), frontmatter, '{}'), '$.session_id')) WHERE json_valid(frontmatter) AND json_extract(IIF(json_valid(frontmatter), frontmatter, '{}'), '$.session_id') IS NOT NULL` SHALL exist to support session-scoped queries used by extraction (proposal #2) and supersede lookups.

#### Scenario: Head-only index exists on a fresh v8 database
- **WHEN** a fresh v8 database is initialized
- **THEN** both `idx_pages_supersede_head` and `idx_pages_session` are present with their documented partial-index `WHERE` clauses

### Requirement: Retrieval defaults to head-only and exposes history via `include_superseded`
The system's retrieval surfaces SHALL filter to head pages (`superseded_by IS NULL`) by default. Specifically: `hybrid_search`, `progressive_retrieve`, the `memory_search` MCP tool, the `memory_query` MCP tool, the `quaid search` CLI, and the `quaid query` CLI SHALL apply the head-only predicate. Each surface SHALL accept an `include_superseded` parameter (default `false`); when `true`, historical pages SHALL be included in candidate selection and ordering.

#### Scenario: Default search returns only head pages
- **WHEN** pages `A` (superseded) and `B` (current head, supersedes A) match a search query
- **THEN** `memory_search` and `memory_query` return `B` and not `A`, and `quaid search` and `quaid query` likewise omit `A`

#### Scenario: `--include-superseded` exposes the chain
- **WHEN** the same query is issued with `include_superseded: true` (or the CLI `--include-superseded` flag)
- **THEN** both `A` and `B` appear in the candidate set and the response indicates which is the head

#### Scenario: `progressive_retrieve` applies the head-only filter before token-budget expansion
- **WHEN** `progressive_retrieve` runs against a corpus that contains a multi-step supersede chain
- **THEN** only head pages are eligible for inclusion in the retrieved budget unless `include_superseded` is set

### Requirement: `memory_get` is unfiltered and exposes supersede metadata
The system's `memory_get` MCP tool SHALL return the requested page regardless of head/non-head status. The response SHALL include a `superseded_by` field naming the slug of the immediate successor when the page is non-head, and SHALL include `supersedes` (the slug of the page this page replaces) when present in frontmatter, so that callers can walk the chain from any starting point.

#### Scenario: Getting a superseded page returns it with its successor pointer
- **WHEN** `memory_get` is called with the slug of page `A`, where `A` is superseded by `B`
- **THEN** the response includes `A`'s content and frontmatter, plus a `superseded_by: "<slug-of-B>"` field

#### Scenario: Getting a head page returns it with `superseded_by` null
- **WHEN** `memory_get` is called with the slug of page `B`, where `B` is the head of a chain
- **THEN** the response includes `B`'s content and `superseded_by: null`

### Requirement: `memory_graph` exposes the chain as `superseded_by` edges
The system's `memory_graph` MCP tool SHALL include `superseded_by` as a navigable edge type so that supersede chains are walkable in graph traversals. Each non-head page SHALL produce one outgoing `superseded_by` edge to its immediate successor; the relationship SHALL be distinct from `links`-table edges and SHALL not be conflated with user-authored typed links.

#### Scenario: Graph view of a chain produces ordered edges
- **WHEN** `memory_graph` is called for the slug of the chain head `C` with `depth >= 2`, where `C` supersedes `B` which supersedes `A`
- **THEN** the response contains `superseded_by` edges from `A → B` and from `B → C` and identifies `C` as the head

### Requirement: File-edit-aware supersede preserves history on user edits to extracted facts
The Phase 4 vault watcher SHALL detect user edits to files under `<vault>/extracted/**/*.md` (or its namespace-scoped equivalent). When the watcher observes a content-hash change for an existing page whose `type` is one of `decision`, `preference`, `fact`, or `action_item`, the system SHALL: (a) write the prior version of the page as a new archived page with `superseded_by` pointing to the edited file's page row and slug `<original-slug>--archived-<timestamp>`, and (b) treat the edited file's new content as a new head with frontmatter `supersedes: <archived_slug>` and `corrected_via: file_edit`. Whitespace-only edits SHALL be a no-op. The archived page SHALL exist only in the database by default; when `corrections.history_on_disk = true`, the archived content SHALL also be written to `<vault>/extracted/_history/<original-slug>--<timestamp>.md`.

#### Scenario: Editing an extracted preference produces an archived predecessor and a new head
- **WHEN** a user edits `<vault>/extracted/preferences/foo.md` to change its body, while the existing page is the head of its chain
- **THEN** an archived page exists in the database with the prior content and `superseded_by` pointing to the edited file's page, and the edited file's page is the new head with frontmatter `supersedes: <archived_slug>` and `corrected_via: file_edit`

#### Scenario: Whitespace-only edits do not produce a new head
- **WHEN** a user saves `<vault>/extracted/preferences/foo.md` with only whitespace differences (e.g. a trailing newline)
- **THEN** no archived page is written and the existing page row is unchanged

#### Scenario: Disk-level history is written when opted in
- **WHEN** `corrections.history_on_disk = true` and a user edit triggers an archive
- **THEN** a file `<vault>/extracted/_history/<original-slug>--<timestamp>.md` is created with the prior content

#### Scenario: Editing a non-extracted file does not invoke the supersede handler
- **WHEN** a user edits a file outside `extracted/**` (e.g. a regular note or a conversation file)
- **THEN** the file-edit-aware supersede handler does not fire and the page is updated through the normal vault-sync path

### Requirement: Conversation-memory schema baseline remains v8 without automatic migration
The system SHALL keep `SCHEMA_VERSION` (and the persisted `quaid_config.schema_version`) at 8 for the remainder of this change. The landed first slice already introduced `pages.superseded_by`, `idx_pages_supersede_head ON pages(type, superseded_by) WHERE superseded_by IS NULL`, the guarded `idx_pages_session` partial index, and the `extraction_queue` table. There SHALL NOT be an automatic pre-v8 → v8 migration; existing pre-v8 databases SHALL be rejected by the existing schema-mismatch behaviour with the standard re-init guidance.

#### Scenario: Fresh v8 init succeeds and reports schema_version 8
- **WHEN** `quaid init` is run against a non-existent database path
- **THEN** the resulting database has `quaid_config.schema_version = 8` and contains all four new schema artefacts (`pages.superseded_by`, `idx_pages_supersede_head`, `idx_pages_session`, `extraction_queue` + `idx_extraction_queue_pending`)

#### Scenario: Existing pre-v8 database is rejected
- **WHEN** the v8 binary opens an existing pre-v8 database
- **THEN** the command fails with the existing schema-mismatch error and does not mutate the database
