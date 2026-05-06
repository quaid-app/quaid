## ADDED Requirements

### Requirement: Phi-3.5 Mini is the default in-process SLM, lazy-loaded
The system SHALL load a configurable small local model in-process inside `quaid serve` for fact extraction. The default model SHALL be `phi-3.5-mini` (alias for `microsoft/Phi-3.5-mini-instruct` or equivalent quantised variant). The model SHALL be lazy-loaded on the first extraction job after daemon start; daemons running with `extraction.enabled = false` SHALL NOT load the model. Once loaded, the model SHALL remain resident for the daemon's lifetime. The model SHALL be invoked via `candle-transformers` using the same inference stack already used for embeddings.

#### Scenario: Daemon with extraction disabled does not load the model
- **WHEN** `quaid serve` starts with `extraction.enabled = false`
- **THEN** the model file is not opened, the model weights are not loaded into memory, and `quaid extraction status` reports the model as not loaded

#### Scenario: First extraction job triggers model load
- **WHEN** `quaid serve` starts with `extraction.enabled = true` and the first extraction job arrives
- **THEN** the model is loaded into memory before the job is processed, and subsequent jobs reuse the loaded model without reloading

#### Scenario: Model alias is configurable
- **WHEN** `extraction.model_alias` is set to `gemma-3-1b` and the daemon is restarted
- **THEN** the next extraction load uses the Gemma 3 1B model from the local model cache

### Requirement: Eager model download via `quaid extraction enable`
The system SHALL provide a `quaid extraction enable` CLI subcommand that flips `extraction.enabled` to `true` AND eagerly downloads the configured model with progress UI. If the download fails (network unreachable, integrity check failure, etc.), the config flag SHALL remain `false` and the command SHALL exit with an actionable error. The system SHALL provide a complementary `quaid extraction disable` subcommand that flips the flag to `false` without removing model files. The system SHALL also provide a `quaid model pull <alias>` subcommand for manual or CI workflows that downloads a model without flipping the extraction flag.

#### Scenario: Successful enable downloads the model and flips the flag
- **WHEN** `quaid extraction enable` runs in an environment with network access and the configured model is not yet cached
- **THEN** the model is downloaded with progress output, the model integrity is verified, `extraction.enabled` becomes `true` in `quaid_config`, and the command exits with status `0`

#### Scenario: Failed download leaves the flag unflipped
- **WHEN** `quaid extraction enable` runs but the download fails (offline, mirror unavailable, integrity check failure)
- **THEN** `extraction.enabled` remains `false`, the command exits with non-zero status, and the error message names the cause and the manual fallback (`quaid model pull <alias>`)

#### Scenario: Manual model pull does not flip the flag
- **WHEN** `quaid model pull phi-3.5-mini` runs successfully
- **THEN** the model is downloaded into the local cache, but `extraction.enabled` is unchanged from its prior value

#### Scenario: `disable` does not delete model files
- **WHEN** `quaid extraction disable` runs while the model is cached on disk
- **THEN** `extraction.enabled` becomes `false`, but the model files remain in the cache for future re-enable

### Requirement: Daemon never silently downloads the model
The system SHALL NOT download a model file as a side effect of any other command (`quaid serve`, `memory_add_turn`, `quaid extract`, etc.). If `extraction.enabled = true` but the model file is missing from the local cache at runtime, the daemon SHALL log an error, mark `extraction.enabled` runtime-disabled (without changing the persisted config), and skip extraction work. The user SHALL re-run `quaid extraction enable` to recover.

#### Scenario: Missing model with extraction enabled does not auto-download
- **WHEN** `quaid serve` starts with `extraction.enabled = true` and the cached model file has been deleted
- **THEN** the daemon logs an actionable error, runtime-disables extraction in memory, continues serving non-extraction MCP tools normally, and `quaid extraction status` reports the runtime-disabled state

#### Scenario: `memory_add_turn` request does not trigger a model fetch
- **WHEN** the daemon is in the runtime-disabled state above and `memory_add_turn` is called
- **THEN** the call appends the turn and skips enqueueing extraction (returning `extraction_scheduled_at: null`); no model download is initiated

### Requirement: SLM panic isolation does not crash the daemon
The system SHALL invoke SLM inference within a `catch_unwind` boundary (or equivalent mechanism). If inference panics, the system SHALL: (a) mark the in-flight extraction job as a retriable failure (counting toward `extraction.max_retries`), (b) flip extraction to runtime-disabled in memory only (without changing the persisted `extraction.enabled`), (c) log the panic with sufficient detail for triage, and (d) leave `quaid serve` running so non-extraction MCP tools continue to function. Recovery SHALL require an explicit user action — `quaid extraction enable` (which re-validates the model and re-loads it).

#### Scenario: SLM panic does not crash `quaid serve`
- **WHEN** an SLM inference call panics during extraction
- **THEN** the panic is caught, the daemon process remains alive, `memory_search` and other MCP tools continue to respond, and the failing job is marked failed with attempts incremented

#### Scenario: Recovery requires explicit user action
- **WHEN** the daemon is in the panic-induced runtime-disabled state
- **THEN** subsequent extraction jobs are skipped (no further SLM calls) until `quaid extraction enable` is run again, after which the model is reloaded and extraction resumes
