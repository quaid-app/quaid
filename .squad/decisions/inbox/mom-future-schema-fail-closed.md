# Mom — future schema mismatch must fail closed

- **Date:** 2026-05-05
- **Scope:** `src/core/db.rs` schema-version gate

## Decision

Treat **any** schema-version mismatch as a hard stop at open time, not just older databases.

## Why

Allowing `schema_version > SCHEMA_VERSION` lets an older binary attach to a newer database shape and do normal open work against an unsupported schema. That is a fail-open seam, not a compatibility feature.

## Required proof

- Preflight/open rejects `schema_version != SCHEMA_VERSION`
- Regression seeds a future version (currently `10`) and proves open/init refuse before creating current-version tables or rewriting stored version metadata
