## MODIFIED Requirements

### Requirement: `quaid extraction` subcommand controls runtime extraction state
The system SHALL provide a `quaid extraction` CLI subcommand group with the following children:

- `quaid extraction enable` - flips `extraction.enabled` to `true` AND eagerly downloads the configured model (delegating to `slm-runtime`'s contract).
- `quaid extraction disable` - flips `extraction.enabled` to `false` without removing model files.
- `quaid extraction status` - prints a human-readable summary of model state, queue depth, active sessions, last-extraction-at per session, and recent failed jobs.

`quaid extraction status` SHALL report realistic SLM runtime diagnostics. When the model is loaded, the output SHALL include the configured alias, loaded/not-loaded/runtime-disabled state, selected runtime dtype, source safetensor dtype summary when known, device, and estimated resident weight bytes. When the model is not loaded, the output SHALL avoid optimistic static memory estimates unless they are derived from cached safetensor metadata. If metadata is unavailable, the memory estimate SHALL be `unknown`.

#### Scenario: `enable` followed by `status` shows the loaded state
- **WHEN** `quaid extraction enable` runs successfully and then `quaid extraction status` runs after an extraction has loaded the model
- **THEN** the status output reports `Extraction enabled: yes`, the model alias, loaded runtime state, selected dtype, device, and a non-zero resident weight estimate

#### Scenario: `status` reports queue depth and active sessions
- **WHEN** the queue contains 2 pending jobs, 0 running, and 1 failed job from the last 24h, and 3 sessions have received turns within the idle window
- **THEN** `quaid extraction status` lists those counts and lists each active session with idle duration

#### Scenario: Not-loaded status does not under-report Phi memory
- **WHEN** `quaid extraction status` runs for `phi-3.5-mini` before the SLM runtime has loaded
- **THEN** the status output either reports a metadata-derived weight estimate for the cached model or reports `unknown`, but it does not print the previous hard-coded `~2.0 GiB` estimate

#### Scenario: F32 fallback is visible
- **WHEN** lower-precision SLM loading falls back to `DType::F32`
- **THEN** `quaid extraction status` includes the selected dtype `F32` and the fallback reason
