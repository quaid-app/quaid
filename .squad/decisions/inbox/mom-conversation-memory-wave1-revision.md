## Mom — conversation-memory foundations Wave 1 revision

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Use explicit ownership and explicit sentinels for the Wave 1 seams: queue completion/failure must be bound to the current dequeue attempt, same-session turn appends must hold a per-session cross-process file lock, and rendered turn metadata must use an explicit `json turn-metadata` fence instead of being inferred from any trailing JSON block.

## Why

- Lease expiry reuses the same queue row, so `job_id` alone cannot prove the caller still owns the live claim.
- A process-local mutex is not enough for file-backed turn ordinals; the serialization proof has to hold when two OS processes race the same session.
- Trailing JSON content is valid user content. If metadata is inferred from shape alone, the canonical parser strips real content.

## Evidence

- `src/core/conversation/queue.rs` now rejects `mark_done` / `mark_failed` when the caller's attempt no longer matches the live `running` row.
- `src/core/conversation/turn_writer.rs` now pairs the existing in-process mutex with a per-session cross-process file lock, and `tests/conversation_turn_capture.rs` proves the second process blocks until the first releases it.
- `src/core/conversation/format.rs` now renders metadata with ` ```json turn-metadata`, and tests prove a bare trailing JSON fence remains content.
