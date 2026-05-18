## MODIFIED Requirements

### Requirement: Phi-3.5 Mini is the default in-process SLM, lazy-loaded
The system SHALL load a configurable small local model in-process inside `quaid serve` for fact extraction and MCP correction. The default model SHALL be `phi-3.5-mini` (alias for `microsoft/Phi-3.5-mini-instruct` or equivalent quantised variant). The model SHALL be lazy-loaded on the first extraction or correction call that needs SLM inference; daemons running with `extraction.enabled = false` SHALL NOT load the model merely because `quaid serve` starts or `memory_add_turn` is called. Once loaded, the model SHALL remain resident in a process-wide shared runtime until process exit, fatal runtime disablement, or a request for a different alias. The model SHALL be invoked via `candle-transformers` using the same inference stack already used for embeddings, and the load dtype SHALL follow the `runtime-memory-footprint` dtype-selection contract.

#### Scenario: Daemon with extraction disabled does not load the model
- **WHEN** `quaid serve` starts with `extraction.enabled = false`
- **THEN** the model file is not opened, the model weights are not loaded into memory, and `quaid extraction status` reports the model as not loaded

#### Scenario: First extraction job triggers model load
- **WHEN** `quaid serve` starts with `extraction.enabled = true` and the first extraction job arrives
- **THEN** the model is loaded into the shared runtime before the job is processed, and subsequent extraction jobs reuse the loaded model without reloading

#### Scenario: First correction call triggers model load
- **WHEN** `memory_correct` is called before any extraction job has loaded the model
- **THEN** the model is loaded into the shared runtime before correction inference, and later correction or extraction calls for the same alias reuse that loaded model

#### Scenario: Model alias is configurable
- **WHEN** `extraction.model_alias` is set to `gemma-3-1b` and the daemon is restarted
- **THEN** the next extraction or correction load uses the Gemma 3 1B model from the local model cache

#### Scenario: Shared runtime does not duplicate model weights per MCP session
- **WHEN** multiple MCP sessions in the same process invoke SLM-backed tools with the same model alias
- **THEN** all sessions use the same loaded runner and do not create per-session model copies
