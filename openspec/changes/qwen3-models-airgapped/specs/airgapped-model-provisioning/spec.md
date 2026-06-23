## ADDED Requirements

### Requirement: Airgapped means local-only inference, not embedded weights
"Airgapped" SHALL denote a privacy and operational property — all inference runs locally, with no cloud inference, no API keys, and no user data leaving the machine — and SHALL NOT denote that model weights are compiled into the binary. The only network access Quaid may perform SHALL be an explicit, user-triggered model download (and integrity verification). After the required models are cached, the system SHALL operate fully offline.

#### Scenario: No data leaves the machine during normal operation
- **WHEN** a user runs `quaid query`, `quaid serve`, ingestion, or extraction against cached models
- **THEN** no request is made to any network endpoint and no user content is transmitted off-device

#### Scenario: Airgapped does not require embedded weights
- **WHEN** the binary is built for the airgapped (privacy) posture
- **THEN** the embedding and extraction model weights are NOT compiled into the binary, and the binary instead resolves them from the local model cache (fetching once if absent)

### Requirement: Default models are auto-provisioned on first use
On first use that requires a model (first semantic operation for the embedder, first extraction enable/job for the SLM), if the configured default model is absent from the local cache, the system SHALL automatically download, integrity-verify, and hook up that model with progress output — including under the airgapped posture — rather than requiring a separate manual download step. Provisioning SHALL be idempotent: a model already present and verified is reused without re-downloading.

#### Scenario: First semantic use provisions the default embedding model
- **WHEN** a fresh install runs its first `quaid query`/`quaid embed` with the default `Qwen3-Embedding-0.6B` not yet cached and network available
- **THEN** the model is downloaded with progress, verified, cached, and used to produce embeddings, with no separate manual pull required

#### Scenario: Enabling extraction provisions the default SLM
- **WHEN** `quaid extraction enable` runs with the default `Qwen3-4B-Instruct-2507` q4_K_M GGUF not yet cached and network available
- **THEN** the GGUF is downloaded with progress, verified, cached, and `extraction.enabled` is flipped to `true`

#### Scenario: Provisioning is skipped when already cached
- **WHEN** a model required for an operation is already present and passes integrity verification
- **THEN** the operation proceeds immediately with no network access

### Requirement: Provisioning is explicit, verified, and offline-tolerant after first fetch
Model provisioning SHALL be triggered only by an explicit user action or first-use of a feature the user invoked, never as a hidden side effect of an unrelated request, and SHALL verify integrity before use. When network is unavailable and a required model is not cached, the system SHALL fail with an actionable error naming the model and the manual fallback, and SHALL NOT degrade silently to a non-semantic fallback without surfacing the cause.

#### Scenario: Offline first-use without a cached model fails actionably
- **WHEN** a required default model is absent and the network is unreachable
- **THEN** the command exits with a non-zero status and an error that names the missing model and the `quaid model pull <alias>` fallback

#### Scenario: Integrity failure aborts hook-up
- **WHEN** a downloaded model file fails its integrity check
- **THEN** the partial download is discarded, the model is not hooked up, and the error names the cause and recovery step

### Requirement: Default models remain user-substitutable
The operator SHALL be able to override both defaults without rebuilding: the embedding model via `QUAID_MODEL` / `--model`, and the extraction model via its model alias/repo configuration. A substituted model SHALL be provisioned and verified through the same path as the defaults.

#### Scenario: Operator overrides the embedding model
- **WHEN** `QUAID_MODEL` or `--model` selects a non-default embedding model
- **THEN** that model is resolved, provisioned on first use, and used for embeddings in place of the default

#### Scenario: Operator overrides the extraction model
- **WHEN** the extraction model alias/repo is set to a non-default value and extraction is enabled
- **THEN** that model is provisioned and loaded in place of the default SLM
