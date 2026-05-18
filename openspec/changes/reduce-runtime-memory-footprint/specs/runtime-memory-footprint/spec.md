## ADDED Requirements

### Requirement: Process-wide SLM residency is bounded
The system SHALL use a single process-wide lazy SLM runtime for production extraction and MCP correction paths. The runtime SHALL hold at most one loaded model runner at a time for the active process. Concurrent callers requesting the same alias SHALL share the same loaded runner instead of constructing duplicate model instances.

#### Scenario: HTTP correction sessions share one loaded runner
- **WHEN** two HTTP/SSE MCP sessions in the same process invoke `memory_correct` using the same configured model alias
- **THEN** both sessions use the same process-wide SLM runtime and the model weights are loaded no more than once

#### Scenario: Extraction and correction share one loaded runner
- **WHEN** the extraction worker has already loaded the configured model alias and an MCP session later invokes `memory_correct`
- **THEN** the correction call reuses the already-loaded runner instead of loading another copy of the same model

#### Scenario: Alias change replaces the loaded runner
- **WHEN** a process has loaded alias `phi-3.5-mini` and a later SLM call requests alias `gemma-3-1b`
- **THEN** the runtime drops the old runner before loading the new alias and still holds at most one loaded runner

### Requirement: SLM dtype selection is memory-aware
The system SHALL select the SLM load dtype from cached safetensor metadata instead of always forcing `F32`. Header inspection SHALL read only safetensor metadata, not tensor payload bytes. The selected dtype and any fallback reason SHALL be available through runtime diagnostics.

#### Scenario: BF16 weights load as BF16
- **WHEN** all floating-point tensors in the selected SLM safetensor files declare dtype `BF16`
- **THEN** the system passes `DType::BF16` to the Candle VarBuilder

#### Scenario: F16 weights load as F16
- **WHEN** all floating-point tensors in the selected SLM safetensor files declare dtype `F16`
- **THEN** the system passes `DType::F16` to the Candle VarBuilder

#### Scenario: F32 weights remain F32
- **WHEN** all floating-point tensors in the selected SLM safetensor files declare dtype `F32`
- **THEN** the system passes `DType::F32` to the Candle VarBuilder and records no lower-precision fallback

#### Scenario: Mixed weights fall back with diagnostic
- **WHEN** the selected SLM safetensor files contain mixed floating-point dtypes
- **THEN** the system loads with `DType::F32` and records a diagnostic reason naming the mixed dtype set

#### Scenario: Lower precision backend failure falls back once
- **WHEN** the system selects `DType::BF16` or `DType::F16` but Candle model construction fails for that dtype
- **THEN** the system retries model construction once with `DType::F32` and exposes the fallback reason in runtime diagnostics

### Requirement: Process-global registries do not grow without bound during normal operation
The system SHALL remove process-global idle-tracker and write-lock registry entries when their work is complete or provably stale. Registry cleanup SHALL preserve concurrency safety and SHALL NOT remove a lock entry while another thread can still be waiting on that lock.

#### Scenario: Write lock entry is removed after an uncontended write
- **WHEN** `with_write_slug_lock` completes for a write target with no other waiters or cloned lock handles
- **THEN** the target's `slug_writes` registry entry is removed before the function returns

#### Scenario: Write lock entry remains while contended
- **WHEN** one thread completes a guarded write while another thread has already obtained a clone of the same per-target lock
- **THEN** the registry entry remains until it is safe for a later caller to remove it

#### Scenario: Closed idle session is not retained
- **WHEN** an idle-tracked session is closed successfully or discovered to be already closed
- **THEN** the session entry is removed from `IDLE_TRACKERS`, and the database entry is removed when it has no remaining sessions

#### Scenario: Stale idle tracker entry is pruned
- **WHEN** an idle-tracked session is older than the configured stale threshold and no active conversation file can be resolved for it
- **THEN** idle scanning prunes the tracker entry instead of retaining it indefinitely
