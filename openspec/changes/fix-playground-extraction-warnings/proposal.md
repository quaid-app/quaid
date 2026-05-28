## Why

The playground currently surfaces two avoidable warnings for the simplest conversation-memory path: a single-turn conversation can enqueue an empty embedding chunk when the watcher re-ingests the canonical day-file, and the extraction prompt is loose enough that Phi-3.5-mini can answer a bare preference statement with non-JSON text. Both failures show up in normal playground use and make the feature look broken even though the underlying user intent is straightforward.

## What Changes

- Stop canonical conversation day-files from producing blank embedding chunks during watcher-driven re-ingest.
- Keep the tighter extraction prompt contract for single-turn preference windows and harden parsing so commentary-wrapped `{"facts":[...]}` output still recovers instead of failing the worker.
- Add regression coverage for both warning paths.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `conversation-turn-capture`: canonical conversation day-files remain ingestable without generating blank embedding inputs.
- `fact-extraction-schema`: simple one-turn preference extraction now tolerates chatty-but-recoverable SLM wrappers around the required JSON envelope instead of retry-failing the worker.

## Impact

- Affected areas: `src/core/chunking.rs`, `src/core/conversation/extractor.rs`, `src/core/conversation/slm.rs`, and targeted regression tests under `tests/`.
- Affected surfaces: playground runtime logs, watcher-driven embedding refresh, extraction worker prompt behavior.
- Compatibility: no CLI or schema changes; this is a correctness fix to existing behavior.
