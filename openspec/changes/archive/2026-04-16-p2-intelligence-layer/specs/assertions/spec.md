## ADDED Requirements

### Requirement: Triple extraction from page content
`src/core/assertions.rs` SHALL implement `extract_assertions(page, conn)` that applies
regex-based heuristics over `compiled_truth` sentences to produce (subject, predicate, object)
triples and write them to the `assertions` table with `confidence = 0.8` and
`asserted_by = 'agent'`. Already-stored assertions for the page SHALL be replaced
(DELETE + INSERT) to avoid stale triples after a page update.

#### Scenario: Extract a works-at assertion
- **WHEN** compiled_truth contains "Alice works at Acme Corp"
- **THEN** an assertion `(subject="Alice", predicate="works_at", object="Acme Corp")` is inserted

#### Scenario: Re-indexing a page replaces prior assertions
- **WHEN** extract_assertions is called a second time after the page content has changed
- **THEN** the old assertions for that page_id are deleted before new ones are inserted;
  the total count equals the number of triples found in the new content

#### Scenario: Page with no recognisable triples inserts zero rows
- **WHEN** compiled_truth contains only prose with no detectable subject-predicate-object patterns
- **THEN** no rows are inserted into the assertions table and the function returns Ok

### Requirement: Contradiction detection
`src/core/assertions.rs` SHALL implement `check_assertions(slug, conn)` that queries
the `assertions` table for the given page plus any page sharing the same subject token.
For each (subject, predicate) pair with two or more different objects and overlapping
validity windows, a row is inserted into the `contradictions` table (if no unresolved
row for that pair already exists).

#### Scenario: Same-page contradiction detected
- **WHEN** a page has two assertions with `subject="Alice"`, `predicate="employer"`,
  `object="Acme"` (valid_until NULL) and `object="Beta Corp"` (valid_until NULL)
- **THEN** `check_assertions` inserts a row into `contradictions` with `type = 'assertion_conflict'`

#### Scenario: Cross-page contradiction detected
- **WHEN** page A asserts `(Alice, employer, Acme)` and page B asserts `(Alice, employer, Beta Corp)`,
  both with overlapping validity windows
- **THEN** `check_assertions` for either page detects the conflict and inserts one contradiction row

#### Scenario: Resolved contradiction is not duplicated
- **WHEN** a contradiction already exists with `resolved_at IS NOT NULL` for the same
  (subject, predicate) pair
- **THEN** `check_assertions` does not insert a duplicate row

### Requirement: Check CLI command
`src/commands/check.rs` SHALL implement `run(db, slug, all, check_type, json)`.
When `--all` is passed it iterates every page, calls `extract_assertions` then
`check_assertions`. When `--slug` is given it processes only that page.
Output lists each detected contradiction with page slug, type, and description.

#### Scenario: Check a single page
- **WHEN** `quaid check --slug people/alice` is called
- **THEN** stdout lists any unresolved contradictions for alice, or "No contradictions found" if clean

#### Scenario: Full memory scan
- **WHEN** `quaid check --all` is called
- **THEN** every page is processed; summary line shows "N contradiction(s) found across M pages"

#### Scenario: JSON output
- **WHEN** `quaid check --all --json` is called
- **THEN** stdout is a JSON array where each element has `page_slug`, `other_page_slug`,
  `type`, `description`, `detected_at`
