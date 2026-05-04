---
name: "occ-conflict-proof-hook"
description: "Prove optimistic concurrency races in Rust MCP handlers with a deterministic pre-write seam"
domain: "testing, rust, mcp"
confidence: "high"
source: "earned from conversation-memory-foundations memory_close_action closure"
---

## Context

When an MCP handler reads a page, mutates content, and writes back with `expected_version`, race tests become flaky if they depend on thread timing alone.

## Pattern

1. Keep the public handler thin and delegate to an internal implementation helper.
2. Let the helper accept a test-only `before_write` callback that runs after the read/mutation step but before the OCC write.
3. In the conflict test, use that callback to land a competing write first.
4. Assert the handler returns `ConflictError` and the stored page matches the competing winner, not the stale caller.

## Anti-patterns

- Sleep-based race tests
- Adding public debug parameters just to force OCC conflicts
- Asserting source-text ordering instead of observable winner state
