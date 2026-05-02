## Context

Quaid stores all pages in a single SQLite file shared across all operations. Before v0.16.0, isolation was only available by pointing different consumers at separate `.db` files — one database per isolated context. This worked but was prohibitively expensive for benchmark suites (LongMemEval: 500 questions × full DB init = 3–6 hours) and inconvenient for multi-agent production deployments.

The existing `pages` schema had a `(collection_id, slug)` UNIQUE constraint. Namespace support requires extending this to `(collection_id, namespace, slug)` so the same slug can exist in multiple namespaces. The migration must be safe for existing databases that carry the old constraint.

## Goals / Non-Goals

**Goals:**
- Add `namespace` as a first-class, lightweight isolation primitive in the single-DB model
- Preserve full backward compatibility: the empty string (`""`) namespace is the global scope; all callers that omit `--namespace` or omit the MCP `namespace` param continue to see global pages only
- Sub-1 second namespace create and destroy (no data copying, no re-init)
- Namespace-aware variants of all search and retrieval paths
- Safe schema migration: detect and rebuild old `UNIQUE(collection_id, slug)` constraint automatically on DB open

**Non-Goals:**
- Cross-database namespace bridging
- Namespace-level access control / encryption
- Namespace TTL enforcement at query time (TTL is stored but expiry is caller-enforced)
- Changing existing FTS5 or vector table schemas beyond namespace filtering predicates

## Decisions

### Namespace as an empty-string default (not NULL)

**Decision:** `namespace TEXT NOT NULL DEFAULT ''`. The empty string is the global scope sentinel.

**Alternatives considered:**
- *NULL as sentinel* — NULL doesn't compose cleanly with UNIQUE constraints; every index and JOIN would require `IS NULL` / `COALESCE` gymnastics.
- *Separate `is_global` boolean* — redundant column; the empty string already encodes this.

**Rationale:** Empty string in a `NOT NULL` column is SQLite-idiomatic for "no namespace". Every predicate simplifies to `WHERE namespace = ?` with `''` as the global value.

### UNIQUE constraint rebuild for legacy databases

**Decision:** On DB open, detect the old `UNIQUE(collection_id, slug)` constraint via `pages_needs_namespace_unique_rebuild()`. If present, perform full `CREATE + INSERT SELECT + DROP + RENAME` table rebuild.

**Alternatives considered:**
- *`ALTER TABLE ADD COLUMN` only* — can't remove the old UNIQUE constraint in SQLite without a table rebuild.
- *Require users to run `quaid migrate`* — breaks the "just works" upgrade contract.

**Rationale:** SQLite does not support `DROP CONSTRAINT`. The rebuild is the only safe path. It is gated behind a detection query so it only runs once on the first open after upgrade.

### Omitted namespace = global scope for reads

**Decision:** When `--namespace` is omitted on CLI reads or `namespace` is absent from MCP read calls, the default is global scope (`""`), not "all namespaces". Explicit namespace pass-through is required to read namespace-scoped pages.

**Alternatives considered:**
- *All-namespaces default* — surprising for existing callers; makes query results non-deterministic as namespaces accumulate.
- *Require explicit `--namespace ""` for global* — breaks every existing invocation.

**Rationale:** Preserves the exact existing query semantics. Callers that want multi-namespace results must opt in explicitly.

### Namespace-aware search variants (not a flag on existing functions)

**Decision:** New functions (`hybrid_search_canonical_with_namespace`, `search_fts_canonical_with_namespace`, `progressive_retrieve_with_namespace`) rather than adding an `Option<&str>` parameter to existing functions.

**Rationale:** Avoids changing the signature of already-stable functions in hot paths. The new variants compose with the existing ones through a thin dispatch layer. This also makes it easy to audit which paths are namespace-aware.

## Risks / Trade-offs

- **[Legacy DB rebuild time]** Databases with very large `pages` tables may take several seconds to rebuild → Mitigation: run in a WAL transaction; the detection query only fires once.
- **[Duplicate slug collisions after rebuild]** After the rebuild, the same `(collection_id, slug)` can appear in multiple namespaces — this is intentional, but callers that assumed slug uniqueness across a collection must now filter by namespace → Mitigation: the `progressive_retrieve_with_namespace` fix addresses the primary caller; documented in proposal.
- **[TTL not enforced at query time]** Expired namespaces return results until explicitly destroyed → Mitigation: documented limitation; callers are expected to call `namespace destroy` at session end.

## Migration Plan

1. On DB open, `db.rs` calls `pages_needs_namespace_unique_rebuild()`.
2. If the old constraint is detected, the full table rebuild executes automatically inside a WAL transaction.
3. After rebuild, `ALTER TABLE pages ADD COLUMN namespace TEXT NOT NULL DEFAULT ''` is applied if not already present.
4. `namespaces` table is created if absent (idempotent `CREATE TABLE IF NOT EXISTS`).
5. `idx_pages_namespace` index is created if absent.
6. No user action required. No `quaid migrate` command needed.
