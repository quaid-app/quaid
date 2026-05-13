## ADDED Requirements

### Requirement: Collection ingest preserves PARA type inference
During collection add, sync, and watcher ingest, the system SHALL assign each page a stable type using explicit frontmatter first and then the existing PARA path/content inference. Graph/frontmatter autowire processing SHALL NOT overwrite a derived page type with the fallback `concept`.

#### Scenario: PARA path types remain distributed
- **WHEN** a collection contains pages under project, area, resource, and archive paths
- **THEN** collection ingest stores pages with the corresponding `project`, `area`, `resource`, and `archive` types
- **AND** pages without explicit or inferred PARA signals use `concept`

#### Scenario: Explicit type wins over path fallback
- **WHEN** a page frontmatter contains `type: project`
- **THEN** the stored page type is `project` even if graph autowire finds relationship fields

#### Scenario: Graph autowire does not collapse type
- **WHEN** a page with inferred type `resource` also contains `links`, `parent`, `children`, `related`, or `tags` frontmatter
- **THEN** graph/tag sync runs without changing the stored page type to `concept`
