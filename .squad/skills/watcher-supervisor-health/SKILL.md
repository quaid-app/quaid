# Watcher Supervisor Health

Use this when a filesystem watcher needs both crash recovery and a truthful operator-facing health surface.

## Pattern

1. Keep runtime-only watcher facts on the watcher state itself (`mode`, `last_event_at`, backoff/crash metadata, queue depth source).
2. Treat crash state as real state (`Crashed`), not as absence.
3. On disconnect/crash, clear the live watcher handle, record backoff, and let the supervisor decide when restart is allowed.
4. Publish a **snapshot** of live watcher health into an in-process registry after each supervisor loop.
5. Surface that snapshot only on operator-facing views that actually share the process; if the caller cannot see the live registry, return `null` rather than inventing health.

## Why

Watcher health is easy to lie about. If you persist guessed values or add fake "inactive" enums, operators cannot tell the difference between "not running here" and "running but sick." Snapshotting live runtime state keeps the health surface truthful while preserving a narrow contract.

## Quaid example

- `src/core/vault_sync.rs` owns `WatcherMode = Native | Poll | Crashed`, overflow recovery, and crash/backoff restart state.
- `src/core/vault_sync.rs` publishes watcher health snapshots into the supervisor registry.
- `src/commands/collection.rs` reads that snapshot for `quaid collection info` only; `memory_collections` stays unchanged in v0.10.0.
