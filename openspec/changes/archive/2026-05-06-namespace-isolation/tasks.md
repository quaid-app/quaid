## 1. Schema and migration

- [x] 1.1 Add `namespace TEXT NOT NULL DEFAULT ''` column to `pages` table in `src/schema.sql`
- [x] 1.2 Add `namespaces` table (`id TEXT PRIMARY KEY, ttl_hours REAL, created_at TEXT`) to `src/schema.sql`
- [x] 1.3 Update UNIQUE constraint on `pages` from `(collection_id, slug)` to `(collection_id, namespace, slug)`
- [x] 1.4 Add `idx_pages_namespace` index for efficient namespace filtering
- [x] 1.5 Implement `pages_needs_namespace_unique_rebuild()` detection query in `src/core/db.rs`
- [x] 1.6 Implement full table rebuild (CREATE + INSERT SELECT + DROP + RENAME) in `src/core/db.rs` for legacy databases
- [x] 1.7 Apply `ALTER TABLE pages ADD COLUMN namespace` idempotently on DB open for databases missing the column
- [x] 1.8 Create `namespaces` table and `idx_pages_namespace` idempotently on DB open

## 2. Core namespace module

- [x] 2.1 Create `src/core/namespace.rs` with `NamespaceError` enum (InvalidId, NotFound, AlreadyExists)
- [x] 2.2 Implement `validate_optional_namespace(id: Option<&str>)` — validates format, returns error on invalid chars or excessive length
- [x] 2.3 Implement `create_namespace(id, ttl_hours, db)` — inserts into `namespaces` table
- [x] 2.4 Implement `destroy_namespace(id, db)` — deletes scoped pages then removes namespace record; returns `NotFound` if absent
- [x] 2.5 Implement `list_namespaces(db)` — returns all rows from `namespaces` table
- [x] 2.6 Export `namespace` module from `src/core/mod.rs`

## 3. Namespace-aware search paths

- [x] 3.1 Add `search_fts_canonical_with_namespace(query, namespace, db)` in `src/core/fts.rs`
- [x] 3.2 Add `hybrid_search_canonical_with_namespace(query, namespace, db)` in `src/core/search.rs`
- [x] 3.3 Add `progressive_retrieve_with_namespace(results, budget, depth, namespace, db)` in `src/core/progressive.rs`
- [x] 3.4 Fix `outbound_neighbours` in `src/core/progressive.rs` to apply namespace predicate to source page (p1) as well as target page (p2), preventing duplicate-slug collisions across namespaces

## 4. CLI namespace commands

- [x] 4.1 Create `src/commands/namespace.rs` with `namespace create`, `namespace list`, `namespace destroy` subcommands
- [x] 4.2 Support `--ttl <hours>` on `namespace create`
- [x] 4.3 Support `--json` output on `namespace list` and `namespace destroy`
- [x] 4.4 Wire `namespace` subcommand into `src/commands/mod.rs` and `src/main.rs`

## 5. CLI namespace flag on existing commands

- [x] 5.1 Add `--namespace <id>` flag to `src/commands/query.rs`; omitted flag defaults to global scope (`""`)
- [x] 5.2 Add `--namespace <id>` flag to `src/commands/search.rs`; omitted flag defaults to global scope
- [x] 5.3 Add `--namespace <id>` flag to `src/commands/get.rs`; omitted flag defaults to global scope
- [x] 5.4 Add `--namespace <id>` flag to `src/commands/put.rs`; omitted flag defaults to global scope
- [x] 5.5 Add `--namespace <id>` flag to `src/commands/list.rs`; omitted flag defaults to global scope
- [x] 5.6 Add `--namespace <id>` flag to `src/commands/collection.rs` (`collection add`); omitted defaults to global scope

## 6. MCP namespace support

- [x] 6.1 Add optional `namespace` parameter to `memory_put` handler in `src/mcp/server.rs`; absent param defaults to `""`
- [x] 6.2 Add optional `namespace` parameter to `memory_query` handler; absent param defaults to `""`
- [x] 6.3 Add optional `namespace` parameter to `memory_search` handler; absent param defaults to `""`
- [x] 6.4 Add optional `namespace` parameter to `memory_list` handler; absent param defaults to `""`
- [x] 6.5 Implement new MCP tool `memory_namespace_create(id, ttl_hours?)` in `src/mcp/server.rs`
- [x] 6.6 Implement new MCP tool `memory_namespace_destroy(id)` in `src/mcp/server.rs`

## 7. Tests

- [x] 7.1 Add `tests/namespace_isolation.rs`: write to namespace A, query namespace B returns nothing, global query returns global pages
- [x] 7.2 Add unit tests in `src/commands/namespace.rs` covering create/list/destroy with plain and JSON output (≥6 tests, achieving >85% line coverage)
- [x] 7.3 Add additional tests for TTL create, list JSON, and destroy-with-pages paths
- [x] 7.4 Add unit tests for `src/core/namespace.rs` covering error paths: validate_namespace_id (empty, too-long, invalid chars, valid), destroy NotFound, validate_optional_namespace invalid propagation, create without TTL (≥8 tests, achieving >95% patch coverage)
- [x] 7.5 Add `tests/namespace_isolation.rs` test for legacy UNIQUE constraint rebuild (`open_rebuilds_legacy_pages_unique_constraint_for_namespaces`)
- [x] 7.6 Add `codecov.yml` with 80% patch threshold to handle untestable serde error paths

## 8. Backward-compatibility fixes (bundled in PR #141)

- [x] 8.1 Fix CLI read commands: omitted `--namespace` passes `Some("")` (global scope) not `None`; ensures no cross-namespace bleed for existing callers
- [x] 8.2 Fix MCP read tools: absent `namespace` field defaults to `""` (global scope) not all-namespaces
- [x] 8.3 Fix `src/core/db.rs`: add `pages_needs_namespace_unique_rebuild()` and full table rebuild path for legacy databases
- [x] 8.4 Fix `src/core/progressive.rs`: apply namespace predicate to source page (p1) in `outbound_neighbours` query
- [x] 8.5 Add `#[allow(dead_code)]` to legacy API shims in `src/commands/namespace.rs` to satisfy `clippy --all-targets`
- [x] 8.6 Remove duplicate unique index from `src/schema.sql` (table UNIQUE constraint already creates the index; explicit `CREATE UNIQUE INDEX` was redundant)
