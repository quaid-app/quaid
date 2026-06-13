# Schema Migration Spec

**Change:** The legacy configuration table renamed to `quaid_config`. `SCHEMA_VERSION` bumped. Default DB directory and filename updated. This is a **breaking schema change** per the breaking-schema-changes skill.

## DDL change

```sql
-- After (current)
CREATE TABLE quaid_config (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
```

The legacy configuration table name no longer appears in any source file. All references in `src/core/db.rs` and any other Rust source files must use `quaid_config`.

## SCHEMA_VERSION

The `SCHEMA_VERSION` constant in `src/core/db.rs` must be incremented by 1 from its previous value.

## Default paths

| Before | After |
|--------|-------|
| `~/<legacy-dir>/` | `~/.quaid/` |
| `~/<legacy-dir>/brain.db` | `~/.quaid/memory.db` |

The `dirs::home_dir()` lookup in `src/core/db.rs` (or wherever the default path is constructed) must reflect the new directory and filename.

## Migration policy

**No automatic migration on open.** Existing databases at a different schema version are never upgraded implicitly. On detecting a `SCHEMA_VERSION` mismatch (including detecting the old schema without a `quaid_config` table), a plain open must return a clear, non-zero error:

```
Error: database schema version mismatch.
  Found version N, expected M.
  To migrate: export your data with the previous binary version, then re-ingest it with the current workflow:
    quaid init ~/.quaid/memory.db
    quaid collection add migrated <export-directory>
```

No fallback, no silent upgrade on open, no legacy configuration table alias reading. The only supported in-place upgrade path is the explicit `quaid migrate` command below.

## Explicit migration: `quaid migrate`

`quaid migrate [path]` (path defaults to `--db` / `QUAID_DB` / `~/.quaid/memory.db`) upgrades an existing database in place by walking the versioned migration registry in `src/core/db.rs`:

- The registry (`MIGRATIONS`) maps each target schema version to a step function; a step upgrades a database from `target - 1` to `target`. New DDL changes MUST land as a new registry rung together with a `SCHEMA_VERSION` bump — never as unversioned open-time patches.
- Before the first rung runs, a byte copy of the database file is written to `<path>.bak` (after a WAL checkpoint). In-memory databases cannot be migrated.
- Each rung runs inside its own transaction and bumps both `quaid_config.schema_version` and the legacy `config.version` mirror to the rung's target version; a `PRAGMA foreign_key_check` runs after each rung commits.
- After the ladder, the idempotent current-version maintenance step runs (the consolidated former `ensure_*` patches: `pages_au` trigger repair, namespace schema, collection owner columns, serve-session columns, collection name guards, raw-import hash backfill), then `PRAGMA integrity_check` and row-count sanity assertions must pass (`pages` count unchanged; `links` may only shrink via derived-edge dedup). On failure the command exits non-zero and names the `.bak` backup.
- An already-current database is a no-op: maintenance still runs, no backup is written, exit code 0.
- A database whose stored version is newer than the binary's `SCHEMA_VERSION` is refused with the version-mismatch error; the remediation is upgrading the binary.
- A database whose version has no contiguous registered ladder path (anything below v9) is refused with the export/re-ingest guidance above.

### Registered ladder rungs

| Target version | Delta |
|----------------|-------|
| 10 | `links` table rebuild (12-step): `source_kind` CHECK extended with `'frontmatter'`/`'entity_pattern'`, new `edge_weight REAL NOT NULL DEFAULT 1.0`; derived-edge dedup then partial unique index `idx_links_unique_derived_edge`; v10 graph config seeds (`graph_depth`, `graph_distance_decay`, `graph_expansion_max`, `edge_weight_frontmatter`, `edge_weight_entity_pattern`, `edge_weight_wikilink`) |

## Invariants

1. `src/schema.sql` must contain `quaid_config` and must NOT contain the legacy table name.
2. `SCHEMA_VERSION` in `src/core/db.rs` must be greater than the version in the last release tag.
3. The DDL change, `SCHEMA_VERSION` bump, and all test fixture updates must land in a single atomic commit.
4. `cargo test` must pass in that commit with no schema-related failures.
5. The default DB path used when no `--db` / `QUAID_DB` is provided must be `~/.quaid/memory.db`.
6. Every schema change to `src/schema.sql` ships with a matching rung in the `MIGRATIONS` registry (or an explicit, documented decision that no in-place path is supported for that bump).

## Validation

- A search for the legacy configuration table name in `src/` returns zero matches.
- A search for the legacy default directory path in `src/` returns zero matches.
- A search for `brain.db` in `src/` returns zero matches.
- `cargo test` → green.
