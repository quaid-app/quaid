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

**No automatic migration.** Existing databases created with a previous binary version are incompatible with `quaid`. On detecting a `SCHEMA_VERSION` mismatch (including detecting the old schema without a `quaid_config` table), the binary must return a clear, non-zero error:

```
Error: database schema version mismatch.
  Found version N, expected M.
  To migrate: export your data with the previous binary version, then run:
    quaid init ~/.quaid/memory.db
    quaid import <export-directory>
```

No fallback, no silent upgrade, no legacy configuration table alias reading.

## Invariants

1. `src/schema.sql` must contain `quaid_config` and must NOT contain the legacy table name.
2. `SCHEMA_VERSION` in `src/core/db.rs` must be greater than the version in the last release tag.
3. The DDL change, `SCHEMA_VERSION` bump, and all test fixture updates must land in a single atomic commit.
4. `cargo test` must pass in that commit with no schema-related failures.
5. The default DB path used when no `--db` / `QUAID_DB` is provided must be `~/.quaid/memory.db`.

## Validation

- A search for the legacy configuration table name in `src/` returns zero matches.
- A search for the legacy default directory path in `src/` returns zero matches.
- A search for `brain.db` in `src/` returns zero matches.
- `cargo test` → green.
