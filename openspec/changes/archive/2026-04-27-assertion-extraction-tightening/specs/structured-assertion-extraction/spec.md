## ADDED Requirements

### Requirement: Assertion extraction scoped to structured zones only
`extract_from_content` SHALL only extract agent assertions from the content inside an
explicit `## Assertions` (case-insensitive) heading section and from structured frontmatter
fields (`is_a`, `works_at`, `founded`). General prose body text SHALL NOT be scanned.

#### Scenario: Page without Assertions section produces zero assertions
- **WHEN** a page has no `## Assertions` heading in its `compiled_truth`
- **THEN** `extract_from_content` returns an empty assertion list for that page

#### Scenario: Page with Assertions section extracts only from that section
- **WHEN** a page has a `## Assertions` section containing `is_a: researcher`
- **THEN** exactly one assertion triple is extracted for that page

#### Scenario: Page with frontmatter is_a field produces assertion
- **WHEN** a page has `is_a: founder` in its frontmatter
- **THEN** an assertion triple is extracted via the frontmatter path without regex

#### Scenario: Prose body text not extracted as assertion
- **WHEN** a page's `compiled_truth` contains the phrase "simpler algorithm than RRF"
  outside any `## Assertions` section
- **THEN** no assertion triple is extracted from that phrase

### Requirement: Minimum object-length guard filters noise assertions
Agent-extracted assertion triples SHALL be discarded if the `object` field is fewer than
6 characters, preventing noise matches like `is_a: it` or `is_a: the`.

#### Scenario: Short object discarded
- **WHEN** a regex match produces a triple with `object = "it"` (2 chars)
- **THEN** the triple is discarded before insertion

#### Scenario: Valid-length object retained
- **WHEN** a regex match produces a triple with `object = "researcher"` (10 chars)
- **THEN** the triple is retained and inserted
