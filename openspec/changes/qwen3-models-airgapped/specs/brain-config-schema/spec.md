## MODIFIED Requirements

### Requirement: Required Keys (written at `quaid init`)
`quaid init` SHALL write the embedding-model identity keys into `quaid_config`. The default values SHALL reflect the `Qwen3-Embedding-0.6B` embedding model at 1024 dimensions.

| Key | Example Value | Description |
|-----|--------------|-------------|
| `model_id` | `Qwen/Qwen3-Embedding-0.6B` | Full HuggingFace model ID |
| `model_alias` | `qwen3-0.6b` | Shorthand alias, or `custom` if a full ID was provided |
| `embedding_dim` | `1024` | Integer embedding dimension for this model |
| `schema_version` | (current) | Current schema version for migration tracking |

#### Scenario: Default init writes Qwen3 embedding config
- **WHEN** `quaid init` runs with no model override on a new database
- **THEN** `model_id` = `Qwen/Qwen3-Embedding-0.6B`, `model_alias` = `qwen3-0.6b`, and `embedding_dim` = `1024` are written

#### Scenario: Custom model override is recorded
- **WHEN** `quaid init` runs with a custom embedding model selected
- **THEN** `model_id` records the full ID, `model_alias` is `custom`, and `embedding_dim` reflects that model's dimension

### Requirement: Init Behaviour
`quaid init` SHALL write the config keys on first creation using the resolved (default or overridden) embedding model, and SHALL be idempotent when `quaid_config` is already populated. As a pre-release breaking change, no automatic migration is provided for databases created before this change: their embeddings are dimension-incompatible (384 vs 1024) and a fresh `quaid init` plus full re-embed is expected.

#### Scenario: Fresh init writes config keys
- **WHEN** `quaid init` runs against a new database
- **THEN** all required `quaid_config` keys are written using the resolved embedding model

#### Scenario: Idempotent on populated config
- **WHEN** `quaid init` runs against a database whose `quaid_config` is already populated
- **THEN** it is a no-op and does not overwrite existing keys

#### Scenario: Pre-change database is not auto-migrated
- **WHEN** a database created before this change is opened
- **THEN** the dimension/model mismatch is surfaced (per model-mismatch detection) and re-initialization is required; no silent in-place migration is attempted
