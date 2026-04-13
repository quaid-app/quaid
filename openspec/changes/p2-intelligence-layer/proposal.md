---
id: p2-intelligence-layer
title: "Phase 2: Intelligence Layer"
status: proposed
type: feature
phase: 2
owner: fry
reviewers: [leela, professor, nibbler, mom]
created: 2026-04-13
depends_on: p1-core-storage-cli
---

# Phase 2: Intelligence Layer

## What

Add cross-reference, temporal reasoning, and memory-consolidation capabilities:

- Temporal links with validity windows (`brain_link`, `brain_link_close`, `--temporal` backlinks)
- N-hop graph neighbourhood traversal (`brain_graph`, `gbrain graph`)
- Assertions table with provenance + heuristic contradiction detection (`gbrain check`)
- Progressive retrieval with full token-budget gating (`brain_query` with `--depth auto`)
- Novelty checking for Tier 2–4 gating (reject near-duplicates on ingest)
- Work-context entity types: `decision`, `commitment`, `action_item`
- Palace wing filtering validated against benchmarks (room-level deferred)
- Full MCP write surface with optimistic concurrency (version check on `brain_put`)
- Optional person template enrichment sections for Tier 1 contacts

## Why

Phase 1 gives you search. Phase 2 gives you knowledge. The graph traversal, contradiction detection, and novelty checking are the features that separate GigaBrain from a glorified FTS5 wrapper. They implement the core ideas from MemPalace, OMNIMEM, and agentmemory research cited in the spec.

## Dependencies

Phase 2 work does NOT begin until the Phase 1 ship gate is fully signed off.

## Scope

- `src/core/graph.rs` — N-hop BFS over links table with temporal filtering
- `src/commands/graph.rs`
- `src/core/assertions.rs` — contradiction detection via SQL assertion comparison
- `src/commands/check.rs`
- Full progressive retrieval in `brain_query`
- Full MCP write surface: `brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`, `brain_check`, `brain_timeline`, `brain_tags`
- Optimistic concurrency: enforce `expected_version` on `brain_put`
- Novelty check integration in `src/commands/ingest.rs`
- Person template enrichment sections as optional frontmatter keys

## Reviewer Gates

- **Professor**: graph BFS correctness, OCC conflict protocol
- **Nibbler**: OCC abuse (stale-version exploits, parallel writer races), contradiction evasion
- **Mom**: temporal link edge cases, cyclic graph queries, zero-hop graph
- **Bender**: ingest conflicting sources → contradiction detected; parallel writers → correct OCC
- **Kif**: palace wing filtering benchmark (wing-level filter reduces latency without precision drop)
