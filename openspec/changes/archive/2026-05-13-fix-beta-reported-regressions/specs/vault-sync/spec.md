## ADDED Requirements

### Requirement: Recoverable frontmatter does not reduce round-trip export completeness
Collection ingest and export SHALL preserve every page whose frontmatter can be losslessly represented or safely normalized by known coercions. A recoverable field such as scalar `related` SHALL NOT cause collection attach, sync, or export to skip the page.

#### Scenario: Scalar related page imports and exports
- **WHEN** a collection contains a markdown page with frontmatter `related: some-slug`
- **THEN** `quaid collection add` ingests the page
- **AND** `quaid export` includes the page in the exported directory
- **AND** the exported frontmatter remains parseable on re-import

#### Scenario: Round-trip count preserves imported pages
- **WHEN** a collection import reports N successfully ingested pages
- **THEN** a round-trip export of that collection writes N page files unless a page is explicitly excluded by documented ignore/delete behavior

#### Scenario: Invalid unrecoverable graph metadata is isolated
- **WHEN** a page contains unrecoverable graph metadata in one optional frontmatter field but otherwise valid page content
- **THEN** the system logs or reports the field-level problem using existing diagnostics
- **AND** the page itself remains importable and exportable
