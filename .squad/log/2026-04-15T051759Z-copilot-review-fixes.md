# Session Log: Copilot Review Fixes

**Timestamp:** 2026-04-15T051759Z
**Session:** copilot-review-fixes
**Agent:** Fry
**Commit:** 1ec9f76

## What Happened

Fry systematically addressed all 14 Copilot-generated PR review comments on `phase1/p1-core-storage-cli`.

## Changes Applied

| # | File(s) | Fix | Rationale |
|---|---------|-----|-----------|
| 1 | `src/mcp/server.rs` | `brain_query` limit 50→10 | Spec compliance for default window |
| 2 | `src/core/migrate.rs` | `INSERT OR REPLACE` → UPSERT | Preserve rowid/created_at on conflict |
| 3 | `src/core/migrate.rs` | Manual `BEGIN`/`COMMIT` → `unchecked_transaction()` | Rollback-on-drop, errors reported |
| 4 | `src/core/migrate.rs` | `read_to_string` → `fs::read()` | True raw-bytes SHA-256 for idempotency |
| 5 | `src/commands/ingest.rs` | `INSERT OR REPLACE` → UPSERT | Data preservation |
| 6 | `src/commands/ingest.rs` | Consolidate `import_hashes` → `ingest_log` | Single idempotency source |
| 7 | `src/core/migrate.rs` | Same consolidation | Match ingest.rs |
| 8 | `src/commands/ingest.rs` | `read_to_string` → `fs::read()` | Raw-bytes hash consistency |
| 9 | `src/commands/timeline.rs` | Query `timeline_entries` table + fallback | Structured data preference |
| 10 | `src/commands/put.rs` | `.ok()` → explicit `QueryReturnedNoRows` | Error clarity |
| 11 | `src/commands/config.rs` | `.ok()` / `Err(_)` → explicit match | Error clarity |
| 12 | `src/commands/embed.rs` | Remove static hash-placeholder warning | Reduce noise (T14 shim documented) |

## Testing

- ✅ All 152 tests pass
- ✅ Clippy clean
- No regressions

## Context

This work closes Copilot's review comments from PR #12. All fixes are focused on correctness, data integrity, and alignment with spec.

## Next

Ready for merge to `main`.
