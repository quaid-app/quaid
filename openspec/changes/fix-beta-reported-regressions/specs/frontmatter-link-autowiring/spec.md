## MODIFIED Requirements

### Requirement: Frontmatter `parent`, `children`, and `related` fields produce fixed relationship types
The system SHALL parse `parent:` as a single string, `children:` as a list of strings, and `related:` as either a list of strings or a single string coerced to a one-item list. Each resolvable value SHALL produce a `links` row with `source_kind = 'frontmatter'`, `edge_weight = config.edge_weight_frontmatter`, and `relationship` equal to `'parent'`, `'child'`, or `'related'` respectively. Unresolvable values SHALL follow the existing unresolved-target gap behavior without aborting otherwise valid page ingest.

#### Scenario: `parent` field produces a single typed edge
- **WHEN** a page is written with frontmatter `parent: programs/yc-w17`
- **THEN** a `links` row exists from the page to `programs/yc-w17` with `relationship = 'parent'` and `source_kind = 'frontmatter'`

#### Scenario: `children` field produces one edge per entry
- **WHEN** a page is written with frontmatter `children: [companies/brex, companies/scale]`
- **THEN** the `links` table contains exactly two rows from that page with `relationship = 'child'` and `source_kind = 'frontmatter'`

#### Scenario: Scalar `related` field is coerced to one edge
- **WHEN** a page is written with frontmatter `related: karpathy-llm-wiki-workflow-breakdown`
- **THEN** ingest treats it as `related: [karpathy-llm-wiki-workflow-breakdown]`
- **AND** collection attach does not fail with a list-of-strings validation error

#### Scenario: List `related` field remains supported
- **WHEN** a page is written with frontmatter `related: [companies/brex, companies/scale]`
- **THEN** the `links` table contains exactly two rows from that page with `relationship = 'related'` and `source_kind = 'frontmatter'`
