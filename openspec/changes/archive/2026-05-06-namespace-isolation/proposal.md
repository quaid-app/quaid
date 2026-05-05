## Why

Quaid collections provide logical grouping but not true isolation. Running multi-agent workloads, benchmark suites (LongMemEval, BEAM), or per-user deployments against a shared database caused memory bleed between sessions, requiring one database file per context — expensive both in init time and disk I/O. Per-namespace isolation delivers context separation as a lightweight DB primitive rather than a file-system workaround.

**Status:** complete — shipped in v0.16.0 (PR #141, merge commit e0ffa8a, closes issue #137)

## What Changes

- **Schema v6**: `namespace TEXT NOT NULL DEFAULT ''` column added to `pages`; new `namespaces` table (`id, ttl_hours, created_at`); UNIQUE constraint updated to `(collection_id, namespace, slug)`; `idx_pages_namespace` index; safe `ALTER TABLE` migration for existing databases
- `--namespace <id>` flag added to CLI commands: `query`, `search`, `get`, `put`, `list`, `collection add`
- New CLI commands: `quaid namespace create <id> [--ttl <hours>]`, `quaid namespace list`, `quaid namespace destroy <id>`
- MCP `namespace` optional parameter on `memory_put`, `memory_query`, `memory_search`, `memory_list`
- New MCP tools: `memory_namespace_create(id, ttl_hours?)`, `memory_namespace_destroy(id)`
- `src/core/namespace.rs`: `NamespaceError`, `create_namespace`, `destroy_namespace`, `list_namespaces`, `validate_optional_namespace`
- Namespace-aware search variants: `hybrid_search_canonical_with_namespace`, `search_fts_canonical_with_namespace`, `progressive_retrieve_with_namespace`
- Backward compatibility: omitted `--namespace` / omitted MCP `namespace` param defaults to global scope (empty namespace); all existing queries work unchanged
- **Follow-up fixes bundled in PR #141**: omitted CLI/MCP namespace reads default to global-only (not all namespaces); legacy `UNIQUE(collection_id, slug)` DBs auto-rebuild for namespace support on open; progressive source-page namespace filter fixed to prevent duplicate-slug collisions

## Capabilities

### New Capabilities

- `namespace-isolation`: Per-namespace memory isolation — create, query, and destroy named contexts within a single database; backward-compatible with global (empty-namespace) scope

### Modified Capabilities

- (none — namespace is an additive column; existing spec behaviors are unchanged in the default global scope)

## Impact

- `src/schema.sql` — v6 DDL with namespace column, namespaces table, updated UNIQUE constraint, new index
- `src/core/db.rs` — migration logic, legacy constraint detection and rebuild
- `src/core/namespace.rs` — new module
- `src/core/search.rs`, `src/core/fts.rs`, `src/core/progressive.rs` — namespace-aware variants
- `src/commands/namespace.rs` — new command module
- `src/commands/query.rs`, `search.rs`, `get.rs`, `put.rs`, `list.rs`, `collection.rs` — `--namespace` flag
- `src/mcp/server.rs` — namespace param on existing tools, two new tools
- `src/main.rs` — namespace subcommand wiring
- `tests/namespace_isolation.rs` — integration test
- No breaking changes to existing CLI or MCP surface; empty-namespace behaviour preserved
