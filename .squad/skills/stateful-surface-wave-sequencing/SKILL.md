---
name: "stateful-surface-wave-sequencing"
description: "Sequence stateful features from storage contract to request path to watcher/mutation closure"
domain: "planning, sequencing, architecture"
confidence: "high"
source: "earned"
---

## Context

Use this when one change spans an on-disk format, synchronous write APIs, background queueing, and watcher-driven mutation/history behavior. These changes are easy to over-parallelize and then re-churn because every later surface depends on the file/path contract.

## Patterns

1. **Freeze the storage contract first.** Land types, parser/render rules, path layout, and root resolution before public MCP/CLI surfaces depend on them.
2. **Bundle the request path together.** Pair the writer, queue semantics, and public add/close tools in one wave so latency, durability, and collapse behavior are proven as one contract.
3. **Isolate watcher or correction mutators last.** File-edit/archive/history-preservation seams deserve their own wave after the capture path is already green.
4. **Move path-selection config early.** If config changes the root or namespace layout, settle it before appenders or watchers hard-code paths.
5. **Keep follow-on runtime out of the foundation wave.** If a later proposal owns workers, daemons, or model runtime, do not leak that scope into the plumbing batch.

## Reviewer guidance

- Put the contract reviewer on Wave 1 if render shape or path semantics might drift from spec.
- Put the test/perf reviewer on the request-path wave before making latency or concurrency claims.
- Put an adversarial watcher reviewer on the final mutation wave before any code lands there.

## Anti-patterns

- Shipping public tool wiring before file/path shape is frozen
- Landing queue enqueue now and retry/lease truth later if the same public surface already depends on it
- Mixing watcher mutation work into the first public write wave
- Letting a follow-on worker/runtime proposal leak into the foundations batch
