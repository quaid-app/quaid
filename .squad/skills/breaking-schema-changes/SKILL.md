---
name: "breaking-schema-changes"
description: "How to safely plan and execute a breaking schema migration in a Rust SQLite codebase"
domain: "architecture, schema migration, execution planning"
confidence: "high"
source: "earned — vault-sync-engine breakdown (2026-04-22)"
---

## Context

Applies when a new OpenSpec change introduces a breaking DDL change (schema version bump, table drops, column type changes, FK changes) in a Rust binary that embeds its schema via `include_str!("schema.sql")`. GigaBrain uses `rusqlite` with `SCHEMA_VERSION` constant in `db.rs`. The pattern applies to any SQLite-backed Rust project with the same structure.

## Patterns

### 1. Schema update must be atomic with test fixture updates

Never commit a schema DDL change alone. The schema is `include_str!()` in `db.rs` — as soon as it changes, every test that calls `db::open()` or creates a test DB will fail. The only valid commit is one that simultaneously:

- Updates `src/schema.sql` (new DDL)
- Updates `src/core/db.rs` `SCHEMA_VERSION` constant
- Updates all test fixtures (or test helper DB creation helpers) to match the new schema
- Verifies `cargo test` passes in the same commit

### 2. Version detection before any v5 work

Implement `db::open()` version detection as the very first task. On detecting an older schema version, return an explicit error with re-init instructions. Users must be directed to an escape route (export, then re-init) BEFORE the first PR that bumps SCHEMA_VERSION is merged.

### 3. Identify every callsite that touches modified tables

Before writing any new code, enumerate every file that does INSERT/SELECT/UPDATE on tables with breaking changes (`pages`, `links`, `assertions` in the vault-sync case). Use `grep` to find them. This is the integration collision list. Fix them in the same Wave as the schema change, not afterward.

### 4. Never remove the only working path mid-sprint

When a new module replaces an old one (e.g., `reconciler.rs` replacing `migrate.rs::import_dir()`), the new module must be complete and tested before the old one is removed. Never merge a PR that removes the only functional ingest/write/sync path while the replacement is still incomplete.

### 5. raw_imports / strict rotation invariants need explicit audits

When a schema change tightens a data invariant (e.g., "exactly one `is_active=1` row per page"), add a post-ingest assertion in tests (`SELECT COUNT(*) WHERE is_active=1 = 1`) and do an explicit callsite audit of every existing INSERT path before writing new code. Existing code may not respect the new invariant.

### 6. Composite key changes are cascading

Changing a unique key from `UNIQUE(slug)` to `UNIQUE(collection_id, slug)` cascades to every query, index hint, and ORM-style struct that assumes the old uniqueness contract. Map all affected code before starting.

### 7. Prove the bootstrap crash window reopens cleanly

When schema bootstrap and runtime metadata hydration are separate steps, matching the seeded version number is necessary but not sufficient. Add a regression for the exact crash-partial state that can exist between those steps (for Quaid: current embedded DDL + legacy `config.version` written, but `quaid_config` still empty) and prove the normal open path succeeds afterward. A preflight-only test can hide a real reopen failure.

Only auto-repair that window when the database is still bootstrap-fresh (default collection only, no user rows in mutable tables). If `embedding_models` already has an active row, treat that registry row as the authoritative model hint for reconstructing `quaid_config`; never fall back to the legacy `config.embedding_model` value once model selection became runtime-configurable.

## Examples

```
# Find all files that INSERT into pages:
rg "INSERT.*INTO.*pages" src/ --type rust

# Find all files that reference SCHEMA_VERSION:
rg "SCHEMA_VERSION" src/ --type rust

# Find all files that use page slug as unique identifier:
rg "\.slug" src/ --type rust | head -50
```

## Anti-Patterns

- **Commit schema DDL separately from test fixture updates.** This breaks CI immediately and wastes review cycles.
- **Remove the old ingest path before the new one is tested.** Creates a window where the product cannot ingest at all.
- **Tighten invariants (rotation, unique constraints) without auditing existing callsites.** New invariant + old code = silent data corruption on upgrade.
- **Post-implementation adversarial review of security-sensitive surfaces.** IPC socket protocols, peer verification, sentinel lifecycle — these need adversarial review BEFORE implementation begins, not as a post-merge QA pass.
- **Landing a breaking schema change while other active branches touch the same tables.** Always clear in-flight branches that touch schema-adjacent code before starting a version bump.
