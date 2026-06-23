## MODIFIED Requirements

### Requirement: Qwen3-4B-Instruct-2507 is the default in-process SLM, lazy-loaded
The system SHALL load a configurable small local model in-process inside `quaid serve` for fact extraction. The default model SHALL be `Qwen3-4B-Instruct-2507` distributed as a q4_K_M GGUF. The model SHALL be run in-process via `candle-transformers`' `quantized_qwen3` (GGUF / `QMatMul`) — no external runtime, daemon, or HTTP service. The model SHALL be lazy-loaded on the first extraction job after daemon start; daemons running with `extraction.enabled = false` SHALL NOT load the model. Once loaded, the model SHALL remain resident for the daemon's lifetime. Generation SHALL be greedy (temperature 0) and the context window SHALL be capped explicitly (default 8K) so the model's large trained context does not over-allocate the KV cache. Strict-JSON output SHALL remain best-effort: greedy decode followed by `serde_json` parsing and the existing tolerant-recovery path, NOT a hard grammar/constrained-decoding guarantee.

#### Scenario: Daemon with extraction disabled does not load the model
- **WHEN** `quaid serve` starts with `extraction.enabled = false`
- **THEN** the model file is not opened, the model weights are not loaded into memory, and `quaid extraction status` reports the model as not loaded

#### Scenario: First extraction job triggers model load
- **WHEN** `quaid serve` starts with `extraction.enabled = true` and the first extraction job arrives
- **THEN** the GGUF is loaded into memory before the job is processed, and subsequent jobs reuse the loaded model without reloading

#### Scenario: Context window is bounded
- **WHEN** an extraction prompt plus generation would exceed the configured context cap (default 8K)
- **THEN** the runner bounds the context/KV allocation to the cap rather than the model's full trained context

#### Scenario: Model alias is configurable
- **WHEN** the extraction model alias is set to another supported model and the daemon is restarted
- **THEN** the next extraction load uses that model from the local model cache

#### Scenario: Malformed JSON is recovered best-effort, not guaranteed
- **WHEN** the model emits JSON with surrounding commentary or minor malformation
- **THEN** the existing recovery path attempts to parse/repair it; a hard well-formedness guarantee is NOT claimed

### Requirement: Eager model download via `quaid extraction enable`
The system SHALL provide a `quaid extraction enable` CLI subcommand that flips `extraction.enabled` to `true` AND eagerly downloads the configured GGUF model with progress UI. If the download fails (network unreachable, integrity check failure, etc.), the config flag SHALL remain `false` and the command SHALL exit with an actionable error. The system SHALL provide a complementary `quaid extraction disable` subcommand that flips the flag to `false` without removing model files. The system SHALL also provide a `quaid model pull <alias>` subcommand for manual or CI workflows that downloads a model without flipping the extraction flag.

#### Scenario: Successful enable downloads the model and flips the flag
- **WHEN** `quaid extraction enable` runs in an environment with network access and the configured GGUF is not yet cached
- **THEN** the model is downloaded with progress output, integrity is verified, `extraction.enabled` becomes `true` in `quaid_config`, and the command exits with status `0`

#### Scenario: Failed download leaves the flag unflipped
- **WHEN** `quaid extraction enable` runs but the download fails (offline, mirror unavailable, integrity check failure)
- **THEN** `extraction.enabled` remains `false`, the command exits with non-zero status, and the error message names the cause and the manual fallback (`quaid model pull <alias>`)

#### Scenario: Manual model pull does not flip the flag
- **WHEN** `quaid model pull <default-extraction-alias>` runs successfully
- **THEN** the GGUF is downloaded into the local cache, but `extraction.enabled` is unchanged from its prior value

#### Scenario: `disable` does not delete model files
- **WHEN** `quaid extraction disable` runs while the model is cached on disk
- **THEN** `extraction.enabled` becomes `false`, but the model files remain in the cache for future re-enable
