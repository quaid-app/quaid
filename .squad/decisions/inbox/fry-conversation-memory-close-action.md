# Fry decision — conversation memory close action

- Timestamp: 2026-05-04T07:22:12.881+08:00
- Change: conversation-memory-foundations
- Scope: tasks 9.1-9.5

## Decision

Keep `memory_close_action` on the narrow MCP contract `{slug, status, note?}` and prove optimistic-concurrency conflicts with an internal pre-write test seam instead of widening the public tool schema.

## Why

- The OpenSpec slice only commits to slug-based action closure.
- Collection-aware slug resolution already gives the handler the routing it needs.
- The pre-write seam gives a deterministic conflict proof without adding user-visible knobs.
