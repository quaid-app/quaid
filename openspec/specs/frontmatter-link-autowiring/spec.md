# frontmatter-link-autowiring Specification

## Purpose
TBD - created by archiving change knowledge-graph-layer. Update Purpose after archive.
## Requirements
### Requirement: Structured frontmatter values are preserved
The system SHALL parse YAML frontmatter as a structured mapping, preserve scalar, sequence, and mapping values in `pages.frontmatter`, and expose scalar helper accessors for existing fields such as `slug`, `title`, `type`, `wing`, and `memory_id`. Structured fields SHALL survive import → export → re-import without being silently dropped.

#### Scenario: Sequence and mapping values are stored in frontmatter JSON
- **WHEN** a page is written with frontmatter `links: [{target: companies/brex, type: founded}]` and `tags: [fintech, yc-w17]`
- **THEN** the persisted `pages.frontmatter` JSON contains a `links` array with an object entry and a `tags` array with both tag strings

#### Scenario: Structured frontmatter round-trips through export
- **WHEN** a page with structured `links:` and `tags:` frontmatter is exported and re-imported
- **THEN** the re-imported page retains equivalent structured frontmatter values and does not collapse them to lossy scalar strings

### Requirement: Frontmatter `links` array produces typed graph edges
On every page write or ingest, the system SHALL parse the `links:` frontmatter field and create one derived `links`-table row per resolvable entry. Each entry MAY be a string (treated as `{target: <string>, type: 'related'}`) or an object with `target` (required), `type` (optional, default `'related'`), `valid_from` (optional, ISO-8601 date), and `valid_until` (optional, ISO-8601 date). Edges produced from this field SHALL have `source_kind = 'frontmatter'` and `edge_weight = config.edge_weight_frontmatter` (default `1.0`).

#### Scenario: Object-form link entry creates a typed edge with temporal validity
- **WHEN** a page is written with frontmatter `links: [{target: companies/brex, type: founded, valid_from: 2017-01-01}]`
- **THEN** the `links` table contains exactly one row with `relationship = 'founded'`, `source_kind = 'frontmatter'`, `valid_from = '2017-01-01'`, `valid_until = NULL`, and `edge_weight = 1.0`

#### Scenario: String-form link entry defaults to relationship `related`
- **WHEN** a page is written with frontmatter `links: [companies/brex]`
- **THEN** the `links` table contains exactly one row with `relationship = 'related'`, `source_kind = 'frontmatter'`, both temporal fields `NULL`, and `edge_weight = 1.0`

#### Scenario: Edge resolves target slug via `resolve_slug` normalization
- **WHEN** a page is written with frontmatter `links: [{target: 'Companies/Brex Inc'}]`
- **THEN** the `to_page_id` resolves to the page whose slug matches `companies/brex-inc` after slug normalization

#### Scenario: Edge to a non-existent target page is dropped and logged
- **WHEN** a page is written with frontmatter `links: [{target: companies/does-not-exist}]` and no page exists at that slug in the source collection
- **THEN** no row is inserted into `links` for that entry, and a `knowledge_gap` row is recorded with context containing the unresolved target

### Requirement: Frontmatter `parent`, `children`, and `related` fields produce fixed relationship types
The system SHALL parse `parent:` (single string), `children:` (list of strings), and `related:` (list of strings) frontmatter fields. Each resolvable value SHALL produce a `links` row with `source_kind = 'frontmatter'`, `edge_weight = config.edge_weight_frontmatter`, and `relationship` equal to `'parent'`, `'child'`, or `'related'` respectively.

#### Scenario: `parent` field produces a single typed edge
- **WHEN** a page is written with frontmatter `parent: programs/yc-w17`
- **THEN** a `links` row exists from the page to `programs/yc-w17` with `relationship = 'parent'` and `source_kind = 'frontmatter'`

#### Scenario: `children` field produces one edge per entry
- **WHEN** a page is written with frontmatter `children: [companies/brex, companies/scale]`
- **THEN** the `links` table contains exactly two rows from that page with `relationship = 'child'` and `source_kind = 'frontmatter'`

### Requirement: Body-content wikilinks produce soft graph edges
The system SHALL extract `[[slug]]` patterns from page body content and create derived `links` rows with `source_kind = 'wiki_link'` and `edge_weight = config.edge_weight_wikilink` (default `0.5`). Wikilink-derived rows SHALL be synced on write so removed wikilinks remove their derived rows for the source page.

#### Scenario: Wikilink in compiled truth produces a soft edge
- **WHEN** a page's `compiled_truth` contains `[[companies/brex]]`
- **THEN** a `links` row exists with `relationship = 'related'`, `source_kind = 'wiki_link'`, and `edge_weight = 0.5`

#### Scenario: Wikilink target lookup honours slug normalization
- **WHEN** a page's `compiled_truth` contains `[[Companies/Brex Inc]]`
- **THEN** the edge target resolves to the page with slug `companies/brex-inc` if it exists; otherwise no edge row is inserted

### Requirement: Frontmatter `tags` populate the `tags` table only, not `links`
The system SHALL parse frontmatter `tags:` entries from a YAML list or comma-separated scalar string and sync them into the `tags` table for the page. Tags SHALL NOT produce rows in the `links` table.

#### Scenario: Tags do not create edges
- **WHEN** a page is written with frontmatter `tags: [fintech, yc-w17]`
- **THEN** the `tags` table contains two rows for that page, and the `links` table contains zero new rows from this field

#### Scenario: Removed tags are removed from the tags table on re-ingest
- **WHEN** a page first ingests with `tags: [fintech, yc-w17]`, then re-ingests with `tags: [fintech]`
- **THEN** the `tags` table contains `fintech` and no longer contains `yc-w17` for that page

### Requirement: Derived edge writes are idempotent without constraining programmatic history
The system SHALL enforce uniqueness for derived graph edges using a partial unique index on `(from_page_id, to_page_id, relationship, source_kind)` where `source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern')`. The system SHALL NOT apply this uniqueness constraint to `source_kind = 'programmatic'`, so manual temporal link history remains representable.

#### Scenario: Re-ingesting an unchanged frontmatter link produces no duplicate edge
- **WHEN** a page with frontmatter `links: [{target: companies/brex, type: founded}]` is ingested twice in succession
- **THEN** the `links` table contains exactly one matching `frontmatter` row after both ingests

#### Scenario: Re-ingesting with an updated date replaces the temporal range
- **WHEN** a page first ingests with `links: [{target: companies/brex, type: founded, valid_from: 2017-01-01}]`, then re-ingests with `valid_from: 2017-02-01`
- **THEN** the `links` table contains exactly one row for that derived edge key, with `valid_from = '2017-02-01'`

#### Scenario: Removing a frontmatter link removes the derived edge on re-ingest
- **WHEN** a page first ingests with `links: [{target: companies/brex}]`, then re-ingests with empty `links`
- **THEN** the `frontmatter` row for that source page and target is deleted

#### Scenario: Programmatic temporal duplicates remain allowed
- **WHEN** two manual `programmatic` links with the same `(from_page_id, to_page_id, relationship)` but different temporal ranges are inserted
- **THEN** both rows are allowed to exist

### Requirement: Pre-release v9→v10 schema reset adds graph edge metadata
The system SHALL bump the canonical schema from v9 to v10 without implementing a v9 → v10 data migration. Fresh v10 databases SHALL include `links.edge_weight REAL NOT NULL DEFAULT 1.0`, an extended `source_kind` CHECK constraint with `'frontmatter'` and `'entity_pattern'` (the latter reserved for a follow-on change), the derived-edge partial unique index, and config defaults for graph depth and edge weights. Existing v9 databases SHALL continue to be rejected by the existing schema-mismatch behavior.

#### Scenario: Fresh v10 schema accepts derived source kinds
- **WHEN** a fresh v10 database is initialized
- **THEN** an `INSERT INTO links (..., source_kind) VALUES (..., 'frontmatter')` succeeds and `INSERT ... 'invalid_kind'` fails the CHECK constraint

#### Scenario: Existing v9 database is not migrated automatically
- **WHEN** the v10 binary opens an existing v9 database
- **THEN** the command fails with the existing schema-mismatch error and does not mutate the database

#### Scenario: Config defaults populated at init
- **WHEN** `quaid init` creates a fresh v10 database
- **THEN** the `config` table contains `graph_depth = 0`, `graph_distance_decay = 0.5`, `graph_expansion_max = 50`, `edge_weight_frontmatter = 1.0`, `edge_weight_entity_pattern = 0.7`, and `edge_weight_wikilink = 0.5`
- **AND** graph retrieval expansion remains opt-in until the benchmark gate passes

