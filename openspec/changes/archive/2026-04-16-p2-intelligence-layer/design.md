## Context

Phase 1 delivered a static binary with full CRUD, FTS5 + vector hybrid search, ingest pipeline, and
5 core MCP tools. The codebase already has stub modules for every Phase 2 concern:

| Module | Status |
|--------|--------|
| `src/core/graph.rs` | Pure stub — `// TODO` |
| `src/core/assertions.rs` | Pure stub — `// TODO` |
| `src/core/progressive.rs` | Pure stub — `// TODO` |
| `src/core/gaps.rs` | Pure stub — `// TODO` |
| `src/core/novelty.rs` | Logic complete; NOT wired into ingest |
| `src/core/palace.rs` | `derive_wing` done; `derive_room` always returns `""` |
| `src/commands/graph.rs` | `todo!` stub |
| `src/commands/check.rs` | `todo!` stub |
| `src/commands/gaps.rs` | `todo!` stub |
| `src/commands/link.rs` | Fully implemented (create, close, backlinks, unlink) |
| `src/mcp/server.rs` | 5 core tools live; Phase 2 tools absent |

Schema (`src/schema.sql`) already defines `links`, `assertions`, `contradictions`,
`knowledge_gaps`, and `timeline_entries` — no DDL changes required in Phase 2.

OCC on `memory_put` is **already complete** (shipped in the SG-6 final fix). Do not re-implement.

**Stakeholders:** Fry (implementer), Professor (graph/OCC review), Nibbler (adversarial review),
Mom (temporal edge cases), Bender (integration testing), Kif (wing-filter benchmark).

---

## Goals / Non-Goals

**Goals:**

1. Implement `src/core/graph.rs` — N-hop BFS over the `links` table with temporal filtering.
2. Implement `src/core/assertions.rs` — extract subject/predicate/object triples and write
   detected contradictions to the `contradictions` table.
3. Implement `src/core/progressive.rs` — token-budget gating for multi-hop result expansion.
4. Implement `src/core/gaps.rs` — list, log, and resolve knowledge gaps.
5. Wire `src/core/novelty.rs` into `src/commands/ingest.rs` (logic exists; plumbing missing).
6. Implement `src/core/palace.rs::derive_room` — room classification from heading structure.
7. Expose Phase 2 MCP tools: `memory_link`, `memory_link_close`, `memory_backlinks`,
   `memory_graph`, `memory_check`, `memory_timeline`, `memory_tags`.
8. Implement `src/commands/graph.rs` and `src/commands/check.rs` CLI commands.
9. Implement `src/commands/gaps.rs` CLI command.
10. Wire `--depth` progressive retrieval into `memory_query`.
11. Pass Phase 2 ship gate (Professor + Nibbler sign-off).

**Non-Goals:**

- GPU inference or CUDA support (Phase 3).
- External API enrichment (Phase 3).
- Multi-tenant or auth layer (never in this product).
- BEIR benchmarks (Phase 3).
- Changing the database schema — all tables are already defined.
- Re-implementing OCC on `memory_put` (already done).
- Full room-level palace filtering benchmark (Kif validates in Phase 3).

---

## Decisions

### D1 — Graph traversal: iterative BFS with visited set, not recursive

**Rationale:** Recursive BFS on an arbitrary user graph risks stack overflow on deep or cyclic
graphs. An iterative BFS with an explicit `HashSet<i64>` visited set is safe, predictable, and
easy to bound at compile time. Depth limit of 10 hops enforced as a safety cap regardless of
`--depth` argument.

**Alternatives considered:**
- Recursive DFS: rejected — unbounded stack depth on cyclic graphs.
- SQLite recursive CTE: rejected — sqlite-vec extension interferes with certain CTE patterns;
  Rust BFS keeps the logic testable and debuggable outside SQL.

### D2 — Graph temporal filtering: "active at query time" by default, `--all` flag for full history

**Rationale:** A AI memory's default graph view should reflect the current world.
Links with `valid_until < now()` represent closed relationships (e.g., former employer).
The default filter is `WHERE valid_until IS NULL OR valid_until >= date('now')`.
`--all` removes the filter to expose full temporal history.

### D3 — Assertions: heuristic triple extraction, not an LLM call

**Rationale:** Phase 2 is offline-first. Triple extraction uses regex patterns over
compiled_truth sentences to extract (subject, predicate, object) tuples. This is deterministic,
fast, and testable. Semantic contradiction detection requiring an LLM is deferred to a future
"enrichment" phase.

**What "contradiction" means in Phase 2:** Two assertions on the same page share the same
(subject, predicate) but have different objects and overlapping validity windows.
Cross-page contradictions (same subject, same predicate, different objects on different pages)
are also detected.

**Alternatives considered:**
- spaCy NER pipeline: not available in a static Rust binary.
- Claude API for triple extraction: requires network; violates offline-first constraint.

### D4 — Progressive retrieval: token budget from `config` table, expandable per call

**Rationale:** The `config` table already has `default_token_budget = 4000`. Progressive
retrieval uses this as the base. `memory_query` with `--depth auto` expands results by following
outbound links of top-ranked results until the budget is consumed or depth 3 is reached.
Token count is approximated as `len(content) / 4` (industry standard proxy).

**Alternatives considered:**
- Hard-coded 4000-token budget: rejected — config table exists precisely to make this tunable.
- Full BFS expansion: rejected — combinatorial explosion on dense graphs; depth cap prevents it.

### D5 — Novelty check: wire into ingest as a pre-write guard, bypassed with `--force`

**Rationale:** `check_novelty` is already implemented with Jaccard + cosine thresholds.
Wiring it into `src/commands/ingest.rs` before the `INSERT/UPDATE` prevents near-duplicate
content from inflating the vector index. `--force` flag (already exists in `ingest.rs`)
bypasses the check for explicit re-ingestion.

**Note on placeholder embeddings:** Until T14 (candle forward-pass) is complete, the cosine
path uses hash-indexed embeddings. The Jaccard path is always semantic. This is the same
trade-off documented in the Phase 1 T14 blocker.

### D6 — Palace room: derive from first `##`-level heading in compiled_truth

**Rationale:** The "room" concept from the MemPalace architecture maps naturally to the
first semantic section of a page. Using the first `##` heading (lowercased, kebab-cased)
as the room name is deterministic and human-readable. Pages with no `##` heading use
room `""` (unchanged from Phase 1 behavior).

**Alternatives considered:**
- ML classification into a fixed taxonomy: requires model inference on every write; deferred.
- First paragraph noun extraction: too noisy without NER.

### D7 — MCP Phase 2 tools: same `QuaidServer` impl block, new `#[tool]` methods

**Rationale:** `server.rs` already uses `rmcp`'s `#[tool(tool_box)]` macro on `impl QuaidServer`.
Adding Phase 2 tools as new methods on the same impl block is the natural pattern — no
architectural change needed. Input structs follow the same `#[derive(Debug, Deserialize, schemars::JsonSchema)]`
pattern as Phase 1.

### D8 — Knowledge gaps: log on low-confidence query results, expose via `quaid gaps`

**Rationale:** The `knowledge_gaps` schema stores `query_hash` (always) and `query_text`
(only after approval). `memory_gap` logs a gap when `hybrid_search` returns fewer than
`min_results` (default 2) or all scores are below `confidence_threshold` (default 0.3).
The `gaps` CLI command lists unresolved gaps with their hashes; resolution links them to
the page that answered the question.

---

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| Cyclic graph → infinite BFS loop | Visited set + hard depth cap of 10 |
| Heuristic triple extraction produces false contradictions | `confidence` field defaults to 0.8 for heuristic; manual assertions use 1.0; UI shows confidence |
| Novelty check with hash-indexed embeddings has poor recall | Jaccard path handles exact/near-exact duplicates; cosine path will improve when T14 completes |
| Room classification from headings is noisy for headingless pages | Falls back to `""` — same as Phase 1; no regression |
| Progressive retrieval budget overrun | Token approximation (`len/4`) is a ceiling estimate; hard cap at `budget * 1.5` before truncation |
| MCP Phase 2 tools expose write surface to concurrent clients | Links/assertions are append-only; contradiction detection is read-only; no new OCC surface needed |

---

## Migration Plan

No schema changes. No data migration. All tables used in Phase 2 (`links`, `assertions`,
`contradictions`, `knowledge_gaps`) were created at `quaid init` in Phase 1.

Deployment steps:
1. Implement and test each module independently (see tasks.md).
2. Run `cargo test` after each group — no partial-state binary should ship.
3. Professor reviews graph.rs BFS and progressive.rs budget logic before MCP wiring.
4. Nibbler runs adversarial review of MCP Phase 2 write surface before ship gate.
5. Ship gate: `cargo test` (all pass) + MCP Phase 2 tools connect + `quaid graph` works + no
   regressions on Phase 1 round-trip tests.

Rollback: Phase 2 is additive (no schema changes, no Phase 1 behavior modified). Rolling back
means reverting to Phase 1 binary — the DB is forward/backward compatible.

---

## Open Questions

1. **Room taxonomy**: Should rooms be drawn from a fixed vocabulary or freeform headings?
   *Current decision*: freeform (D6). Revisit in Phase 3 if palace benchmarks show precision drop.

2. **Assertion confidence thresholds**: The 0.8 default for heuristic triples is a guess.
   Kif should establish a recall/precision curve in Phase 3 benchmarks.

3. **`memory_gap` auto-logging**: Should every low-confidence query auto-log a gap, or require
   an explicit `memory_gap` call? *Current decision*: auto-log when `len(results) < 2`.
   May produce noise on sparse brains — monitor and tune in Phase 3.

4. **Progressive retrieval depth cap**: Is 3 hops enough? The spec says `--depth auto`.
   Current design caps at 3 with the token budget as the primary brake. Revisit if users
   report incomplete context on deep relationship chains.
