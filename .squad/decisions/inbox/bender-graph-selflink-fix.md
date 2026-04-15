# Bender: Graph self-link suppression fix

- **Date:** 2026-04-16
- **Scope:** `src/core/graph.rs`, `src/commands/graph.rs`, `tests/graph.rs`
- **Commit:** a1d1593
- **Trigger:** Nibbler graph slice rejection (`nibbler-graph-final.md`)

## Decision

Self-links (`from_page_id == to_page_id`) are suppressed at two layers:

1. **Core BFS**: skip edges where target equals current source during traversal. Self-link edges never enter `GraphResult`.
2. **Text renderer**: defense-in-depth filter drops any edge where `from == to` before tree rendering.

## Rationale

- The `active_path` cycle check happened to suppress self-links in text output, but this was accidental — not an intentional contract enforcement.
- Nibbler correctly identified that this left the task 2.2 invariant ("root can never appear as its own neighbour") enforced by coincidence, not by design.
- Two-layer defense ensures the contract holds even if future refactors change the cycle suppression mechanism.

## Reviewer lockout

- Scruffy is locked out of the graph artifact per Nibbler's rejection. Bender took ownership.
- This fix is scoped to the self-link issue only; all other approved behaviors (outbound-only traversal, parent-aware tree, cycle suppression, edge deduping, temporal filtering) are preserved.

## Test evidence

- 3 new unit tests + 1 new integration test + 1 strengthened integration test
- All 14 unit + 9 integration graph tests pass
