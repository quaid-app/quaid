## Why

Quaid currently hard-codes BGE-small-en-v1.5 as its only embedding model. Users with more capable hardware or higher recall requirements have no way to trade binary size and latency for quality. The BAAI BGE family offers a well-maintained progression from small (384d, fast) to large/m3 (1024d, higher recall), and the existing online-model channel already has the HuggingFace download infrastructure to support them. This change makes model selection a first-class, user-controlled option without altering the default behaviour for any existing user.

## What Changes

- Add a `QUAID_MODEL` environment variable and a `--model` global CLI flag that accept shorthand aliases (`small`, `base`, `large`, `m3`) or any full HuggingFace model ID.
- Add a `quaid_config` key-value table to the schema, written at `quaid init`, recording the active model ID and embedding dimension.
- On DB open, validate the requested model against the stored model. Mismatch returns a clear, actionable error message before any operation proceeds.
- Update the online-model channel to resolve MODEL_ID and EMBEDDING_DIMENSIONS at runtime from the selected model. Airgapped channel is unchanged (always BGE-small embedded).
- Maintain ~90% test coverage on all new code.
- Update README.md, CLAUDE.md, and affected skills/ SKILL.md files to document the new interface.

## Capabilities

### New Capabilities
- `configurable-embedding-model`: Users can select BGE-small, BGE-base, BGE-large, or BGE-m3 (or any HuggingFace model ID) via `QUAID_MODEL` env var or `--model` flag. Default is unchanged (`small` / BAAI/bge-small-en-v1.5).
- `memory-config-schema`: A `quaid_config` key-value table in memory.db records the active model and embedding dimension at init time, enabling validation on subsequent opens.
- `model-mismatch-detection`: Opening a DB initialized with a different model returns a clear error before any read or write proceeds, preventing silent dimension mismatches.

### Modified Capabilities
- `online-model-download`: Extends existing HuggingFace download/cache infrastructure to support additional BGE model IDs beyond BGE-small. SHA-256 pins for standard BGE family; warning-only for custom IDs.

### Unchanged Capabilities
- `airgapped-channel`: The embedded-model build is unaffected. Always uses BGE-small-en-v1.5 at compile time. The `--model` flag is a no-op (with a clear warning) in airgapped builds.

## Non-Goals

- Supporting non-BGE or non-HuggingFace embedding models in this change.
- Automatic model migration (re-embedding an existing DB under a new model). Users must `rm memory.db && quaid init` to switch models.
- GPU/Metal acceleration (CPU-only, consistent with current candle usage).
- Airgapped builds with embedded large models (binary size makes this impractical).

## Impact

- `src/core/inference.rs`: MODEL_ID, EMBEDDING_DIMENSIONS become runtime values; model resolution logic added
- `src/core/db.rs`: quaid_config table init, model validation on open
- `src/schema.sql`: quaid_config table DDL
- `src/main.rs`: `--model` global flag via clap
- `Cargo.toml`: no new dependencies (all within existing candle/reqwest stack)
- `README.md`, `CLAUDE.md`, `skills/` SKILL.md files: documentation updates
- Test coverage: new unit + integration tests for model resolution, quaid_config, and mismatch detection
