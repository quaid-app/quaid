## ADDED Requirements

### Requirement: Default entity-pattern set ships embedded in the binary
The system SHALL include a default set of regex-based entity patterns covering the relationships `works_at`, `founded`, `invested_in`, `acquired`, and `leads`. These defaults SHALL be embedded in the binary so a fresh install needs no external config to extract entities.

#### Scenario: Default patterns active on a fresh install
- **WHEN** `quaid init` creates a new database and a page is ingested whose `compiled_truth` contains `Alice founded Brex`
- **THEN** the system attempts entity extraction against the default `founded` pattern without any user-supplied config file

#### Scenario: Default pattern list covers the five seed relationships
- **WHEN** the binary is built and inspected by tests
- **THEN** the loaded default pattern set contains at least one entry per relationship in {`works_at`, `founded`, `invested_in`, `acquired`, `leads`}

### Requirement: User-overridable entity patterns at `~/.quaid/entity-patterns.yaml`
When a command that performs entity extraction starts, the system SHALL load `~/.quaid/entity-patterns.yaml` if it exists and use its contents in place of the embedded defaults. Each entry SHALL specify `regex` (a Rust-compatible regex with exactly two capture groups for subject and object), `relationship` (a relationship-type string), optional `subject_type` / `object_type` role hints, and optional `weight` (a float between 0.0 and 1.0; defaults to `config.edge_weight_entity_pattern`). Read-only commands that do not run extraction SHALL NOT need to load this file.

#### Scenario: User-supplied file overrides defaults
- **WHEN** `~/.quaid/entity-patterns.yaml` contains a single valid pattern with `relationship: founded`
- **THEN** the active pattern set for extraction contains exactly that one pattern; the embedded defaults are not loaded

#### Scenario: Per-pattern weight overrides the global default
- **WHEN** a user pattern specifies `weight: 0.4` and matches a page
- **THEN** the resulting edge or assertion is recorded with weight/confidence `0.4`, not the global `config.edge_weight_entity_pattern` default

#### Scenario: Malformed pattern file fails the extraction command before ingesting pages
- **WHEN** `~/.quaid/entity-patterns.yaml` is syntactically invalid YAML or a regex fails to compile
- **THEN** the write/extraction command exits with a non-zero status and an error message identifying the offending entry before mutating pages

### Requirement: Extraction runs at write time within a 5 ms-per-page budget
For every page write or ingest, the system SHALL run the entity-pattern set against the page's `compiled_truth`. Total extraction time per page SHALL be budgeted to 5 ms wall-clock from the start of extraction. The extractor SHALL check the deadline between patterns; if the budget is exceeded, remaining patterns SHALL be skipped and a `knowledge_gap` row recorded with the page slug and budget-overrun context.

#### Scenario: Extraction completes within budget on a typical page
- **WHEN** a page with `compiled_truth` of approximately 1 KB is ingested
- **THEN** the entity extractor processes the page in under 5 ms wall-clock and records every match

#### Scenario: Pages over budget are partially processed and logged
- **WHEN** a pathological page causes pattern matching to exceed 5 ms after the third pattern
- **THEN** the remaining patterns are skipped, and a `knowledge_gap` row is recorded with context indicating budget exhaustion

#### Scenario: No LLM calls during extraction
- **WHEN** entity extraction runs for any page
- **THEN** no network requests are issued and no embedding or inference function is invoked from the extraction code path

### Requirement: Entity surfaces resolve through a role-aware collection-local resolver
For each regex match producing a `(subject, object)` pair, the system SHALL resolve each surface within the source page's collection using relationship-derived or pattern-provided role hints. Resolution SHALL try exact slug normalization, role-prefixed slug candidates, case-insensitive exact title match, and unique slug-basename match. A surface SHALL resolve only if exactly one page matches.

#### Scenario: Bare person and company names resolve to typed slugs
- **WHEN** a page in the default collection contains `Alice founded Brex`, and pages `people/alice` and `companies/brex` exist
- **THEN** the subject resolves to `people/alice` and the object resolves to `companies/brex`

#### Scenario: Ambiguous entity surface does not create a graph edge
- **WHEN** a matched surface `Acme` could resolve to more than one page in the source collection
- **THEN** the match is treated as unresolved for graph purposes and no `entity_pattern` edge is inserted for that match

### Requirement: Extraction results route to `assertions` only in this change

For each regex match producing a `(subject, object)` pair, the system SHALL insert an `assertions` row regardless of whether endpoints resolve to pages. Resolved endpoint information SHOULD be recorded in the evidence/context field. The system SHALL NOT insert `links` rows with `source_kind = 'entity_pattern'` in this change. (Durable `entity_pattern` edges in `links` are deferred to a follow-on change that adds source-page provenance and proven retraction semantics.)

`Assertions` rows use `(subject, predicate, object)`, `asserted_by = 'agent'`, and `confidence = pattern.weight`. Duplicate assertions are prevented by checking `(page_id, subject, predicate, object)` before insert.

#### Scenario: Match with both endpoints resolving to pages → assertion only (no edge)
- **WHEN** page text matches a `founded` pattern producing `(alice, brex)`, and both pages `people/alice` and `companies/brex` exist
- **THEN** an `assertions` row is inserted with `(subject='alice', predicate='founded', object='brex')`; **no `links` row** with `source_kind = 'entity_pattern'` is added in this change

#### Scenario: Object does not resolve to a page → assertion only
- **WHEN** page text matches a `founded` pattern producing `(alice, xyz-corp)` and no page resolves for `xyz-corp`
- **THEN** an `assertions` row is inserted with `(subject='alice', predicate='founded', object='xyz-corp')` and no `links` row is added

#### Scenario: Subject does not resolve → assertion only
- **WHEN** page text matches a `works_at` pattern producing `(unknown-person, brex)` and `unknown-person` has no page
- **THEN** an `assertions` row is inserted and no `links` row is added

### Requirement: Extraction is idempotent under re-ingest
Re-ingesting the same page SHALL NOT produce duplicate edges or duplicate assertions from entity patterns. Duplicate prevention for edges relies on the derived-edge partial unique key. Duplicate prevention for assertions SHALL check `(page_id, subject, predicate, object)` before insertion.

#### Scenario: Re-ingesting an unchanged page produces no new assertions
- **WHEN** a page that produced one entity-pattern assertion on first ingest is re-ingested with identical content
- **THEN** the assertion count for that page from this code path remains unchanged

### Requirement: Backfill via opt-in command
The system SHALL provide a `quaid graph extract-entities` command that re-runs entity extraction across every existing page. This command SHALL NOT run as part of schema initialization or schema mismatch handling.

#### Scenario: Opt-in backfill processes existing pages
- **WHEN** `quaid graph extract-entities` runs against a database with 1000 pages
- **THEN** every page's `compiled_truth` is processed through the active pattern set, and the resulting edges and assertions are written, respecting idempotency

#### Scenario: Schema reset does not run entity extraction automatically
- **WHEN** a fresh v10 schema is initialized or a v9 database is rejected for schema mismatch
- **THEN** `entity_pattern` rows are NOT inserted by schema handling alone; only writes/re-ingest or explicit `quaid graph extract-entities` create them
