## Why

Quaid can consume excessive resident memory when extraction or correction loads the local SLM, with observed usage around 19 GiB for a tiny database. The current implementation loads Phi-3.5 Mini as `F32`, can create one SLM runner per HTTP/SSE session, and has process-global registries that grow with session/path churn.

## What Changes

- Load extraction/correction SLM weights with a memory-aware dtype instead of forcing `F32`, preferring native lower precision when supported.
- Share one process-wide lazy SLM runner across extraction and MCP correction tools so concurrent HTTP/SSE sessions cannot load duplicate model copies.
- Make extraction status and model status report realistic resident-memory estimates and loaded dtype/device information.
- Bound or evict process-global runtime registries used for idle conversation tracking and vault write locks.
- Add regression tests around single-load behavior, dtype selection, registry eviction, and memory/status reporting.
- Leave sqlite-vec storage and graph expansion unchanged in this proposal because the investigated database size does not support them as the root cause.

## Capabilities

### New Capabilities

- `runtime-memory-footprint`: Defines memory bounds, shared runtime ownership, dtype selection, and registry cleanup behavior for long-lived Quaid processes.

### Modified Capabilities

- `slm-runtime`: The SLM runtime must use a process-wide shared lazy runner and memory-aware dtype selection instead of per-server `F32` runners.
- `extraction-worker`: Idle session tracking must remove closed or abandoned sessions and expose no unbounded growth path during normal worker operation.
- `correction-dialogue`: Correction tools must use the shared SLM runtime instead of constructing per-MCP-session model runners.
- `extraction-control-cli`: Status output must report realistic model memory/dtype/runtime state so operators can diagnose memory usage.

## Impact

- Affected code: `src/core/conversation/slm.rs`, `src/core/conversation/extractor.rs`, `src/core/conversation/idle_close.rs`, `src/core/vault_sync/write_lock.rs`, `src/mcp/server.rs`, `src/mcp/http.rs`, `src/mcp/tools/conversation.rs`, `src/commands/extraction.rs`, and model status helpers.
- Affected APIs: No breaking MCP or CLI shape changes. Human-readable status output gains additional fields; JSON output, if present, gains additive fields only.
- Dependencies: No new external dependencies expected.
- User-facing: Extraction/correction should no longer load multiple Phi copies in one process, and default SLM memory should be dramatically lower on supported hardware/model files.
