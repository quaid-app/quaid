## MODIFIED Requirements

### Requirement: Interface and default
The embedding model SHALL be selectable via the `QUAID_MODEL` environment variable (primary) or the `--model` CLI flag (alternative), with precedence `--model` > `QUAID_MODEL` > default. The default model SHALL be `Qwen3-Embedding-0.6B` (HuggingFace `Qwen/Qwen3-Embedding-0.6B`, 1024 dimensions).

#### Scenario: Default resolves to Qwen3-Embedding-0.6B
- **WHEN** neither `--model` nor `QUAID_MODEL` is set
- **THEN** the resolved model is `Qwen/Qwen3-Embedding-0.6B` at 1024 dimensions

#### Scenario: Flag overrides environment variable
- **WHEN** both `--model large` and `QUAID_MODEL=base` are provided
- **THEN** `large` wins per the documented precedence

### Requirement: Alias Resolution
Short aliases SHALL expand to HuggingFace model IDs. The default embedding model is a Qwen3 decoder embedder using last-token pooling; the BGE family remains available as opt-in aliases. Any unrecognized string SHALL be used as-is as a HuggingFace model ID.

| Alias | HuggingFace Model ID | Dimensions | Pooling / query | Notes |
|-------|---------------------|-----------|-----------------|-------|
| `qwen3-0.6b` (default) | Qwen/Qwen3-Embedding-0.6B | 1024 | last-token + `Instruct:\nQuery:` | Default |
| `small` | BAAI/bge-small-en-v1.5 | 384 | CLS + BGE prefix | Opt-in |
| `base` | BAAI/bge-base-en-v1.5 | 768 | CLS + BGE prefix | Opt-in |
| `large` | BAAI/bge-large-en-v1.5 | 1024 | CLS + BGE prefix | Opt-in |
| `m3` | BAAI/bge-m3 | 1024 | CLS, no prefix | Multilingual, opt-in |
| Any other string | Used as-is as HuggingFace model ID | From model config | per-arch | Warning: no SHA-256 pin |

#### Scenario: Default alias resolves to the Qwen3 embedder
- **WHEN** the default model is resolved
- **THEN** it maps to `Qwen/Qwen3-Embedding-0.6B` at 1024 dimensions with last-token pooling and the instruction-aware query format

#### Scenario: BGE alias still resolves
- **WHEN** `QUAID_MODEL=large` is set
- **THEN** it maps to `BAAI/bge-large-en-v1.5` at 1024 dimensions with CLS pooling and the BGE query prefix

### Requirement: Airgapped Channel Behaviour
The airgapped (privacy) posture SHALL NOT force a specific embedded model. The configured model — default or substituted — SHALL be provisioned on first use via the local model cache (downloading once if absent), even under the airgapped posture, consistent with the `airgapped-model-provisioning` capability. A non-default selection SHALL be honored rather than silently overridden.

#### Scenario: Airgapped honors a non-default model
- **WHEN** an airgapped install sets `QUAID_MODEL` to a non-default embedding model
- **THEN** the selected model is provisioned and used; the system does not silently fall back to a different model

#### Scenario: Airgapped provisions the default on first use
- **WHEN** an airgapped install with no cached model performs its first semantic operation with network available
- **THEN** the default `Qwen3-Embedding-0.6B` is downloaded, verified, and used

### Requirement: SHA-256 Integrity Verification
Curated model selections (the default `Qwen3-Embedding-0.6B` and the retained BGE aliases) SHALL be integrity-verified against pinned digests before use. Custom (unrecognized) model IDs SHALL skip hash verification with a logged warning, subject to the existing custom-model download policy.

#### Scenario: Curated model is integrity-verified
- **WHEN** the default or a curated alias is downloaded
- **THEN** its files are verified against pinned digests before the model is hooked up

#### Scenario: Custom model warns and skips verification
- **WHEN** an unrecognized model ID is provisioned under the custom-model policy
- **THEN** a warning is logged that integrity verification is skipped
