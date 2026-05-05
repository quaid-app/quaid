# namespace-isolation Specification

## Purpose
TBD - created by archiving change namespace-isolation. Update Purpose after archive.
## Requirements
### Requirement: Namespace create
The system SHALL allow callers to create a named namespace within a database. A namespace ID SHALL be a non-empty string of at most 128 characters containing only alphanumerics, hyphens, and underscores. An optional TTL in hours MAY be provided and is stored but not enforced at query time.

#### Scenario: Create namespace via CLI
- **WHEN** a caller runs `quaid namespace create <id>`
- **THEN** a row is inserted into the `namespaces` table with the given `id` and `created_at` timestamp, and the command exits 0

#### Scenario: Create namespace with TTL via CLI
- **WHEN** a caller runs `quaid namespace create <id> --ttl <hours>`
- **THEN** the `namespaces` row is stored with `ttl_hours = <hours>`

#### Scenario: Create namespace via MCP
- **WHEN** a caller invokes `memory_namespace_create(id, ttl_hours?)`
- **THEN** the namespace is created and the tool returns success

#### Scenario: Create with invalid ID
- **WHEN** a caller provides a namespace ID that is empty or contains disallowed characters
- **THEN** the system returns a `NamespaceError::InvalidId` and exits non-zero

### Requirement: Namespace list
The system SHALL allow callers to list all existing namespaces.

#### Scenario: List namespaces via CLI
- **WHEN** a caller runs `quaid namespace list`
- **THEN** all rows from the `namespaces` table are printed, one per line

#### Scenario: List namespaces as JSON
- **WHEN** a caller runs `quaid namespace list --json`
- **THEN** a JSON array of namespace objects is printed

### Requirement: Namespace destroy
The system SHALL allow callers to destroy a namespace by ID. Destroying a namespace SHALL delete all `pages` rows whose `namespace` column matches the given ID and then remove the namespace record.

#### Scenario: Destroy existing namespace via CLI
- **WHEN** a caller runs `quaid namespace destroy <id>` for an existing namespace
- **THEN** all pages scoped to that namespace are deleted, the `namespaces` row is removed, and the command exits 0

#### Scenario: Destroy existing namespace via MCP
- **WHEN** a caller invokes `memory_namespace_destroy(id)`
- **THEN** namespace pages and the namespace record are deleted; tool returns success

#### Scenario: Destroy non-existent namespace
- **WHEN** a caller attempts to destroy a namespace ID that does not exist
- **THEN** the system returns `NamespaceError::NotFound` and exits non-zero

### Requirement: Namespace-scoped write
The system SHALL allow callers to write a page into a specific namespace using `--namespace <id>` on the CLI `put` and `collection add` commands, or via the `namespace` parameter on the `memory_put` MCP tool.

#### Scenario: Write page into namespace via CLI
- **WHEN** a caller runs `quaid put <slug> --namespace <id> < page.md`
- **THEN** the page is stored with `namespace = <id>` and does not appear in global-scope queries

#### Scenario: Write page into namespace via MCP
- **WHEN** a caller invokes `memory_put(content, namespace=<id>)`
- **THEN** the page is stored with `namespace = <id>`

#### Scenario: Write page without namespace
- **WHEN** a caller writes a page without specifying a namespace
- **THEN** the page is stored with `namespace = ''` (global scope) and behaviour is identical to pre-v0.16.0

### Requirement: Namespace-scoped read
The system SHALL allow callers to read, query, search, and list pages scoped to a specific namespace using `--namespace <id>` on CLI commands or via the `namespace` parameter on MCP tools.

#### Scenario: Query scoped to namespace
- **WHEN** a caller runs `quaid query "<text>" --namespace <id>`
- **THEN** only pages with `namespace = <id>` are returned; pages in other namespaces or in global scope are not included

#### Scenario: Query without namespace returns global only
- **WHEN** a caller runs `quaid query "<text>"` without `--namespace`
- **THEN** only global-scope pages (`namespace = ''`) are returned; namespace-scoped pages are not included

#### Scenario: MCP read scoped to namespace
- **WHEN** a caller invokes `memory_query(query, namespace=<id>)`
- **THEN** only pages with `namespace = <id>` are returned

#### Scenario: MCP read without namespace returns global only
- **WHEN** a caller invokes `memory_query(query)` without a `namespace` field
- **THEN** only global-scope pages are returned; this MUST be identical to pre-v0.16.0 behavior

### Requirement: Backward compatibility
The system SHALL preserve all pre-v0.16.0 CLI and MCP behaviors when no namespace is specified. Existing databases SHALL be automatically migrated on first open without user intervention.

#### Scenario: Legacy database opens after upgrade
- **WHEN** a database created before v0.16.0 (with old `UNIQUE(collection_id, slug)` constraint) is opened by v0.16.0+
- **THEN** the database is rebuilt in-place to support the new `(collection_id, namespace, slug)` constraint; all existing pages are preserved with `namespace = ''`; the open succeeds

#### Scenario: Existing queries unchanged
- **WHEN** an existing caller invokes any CLI or MCP read command without a namespace parameter
- **THEN** the results are identical to pre-v0.16.0 for pages stored in global scope

### Requirement: Namespace isolation guarantee
Pages written to a namespace SHALL NOT appear in queries scoped to a different namespace. The isolation MUST apply to all retrieval paths: FTS5 full-text search, vector search, hybrid search, and progressive retrieval.

#### Scenario: Write to namespace A, query namespace B returns nothing
- **WHEN** pages are written to namespace A and a query is issued for namespace B (with no matching pages)
- **THEN** zero results are returned

#### Scenario: Progressive retrieval respects namespace for source pages
- **WHEN** a progressive retrieval traverses links from a source page in namespace A
- **THEN** the source page filter uses the same namespace predicate as target pages, preventing slug collision with namespace B pages

