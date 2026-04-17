## Model Mismatch Detection Spec

### When It Fires

On every DB open (any command that opens brain.db), after reading `brain_config`:

1. Resolve the requested model from `--model` flag or `GBRAIN_MODEL` env (default: `small`).
2. Read `model_id` and `embedding_dim` from `brain_config`.
3. If the requested model's resolved ID does not match the stored `model_id`: **error and exit**.

### Error Message Format

```
Error: Model mismatch

  This brain.db was initialized with: BAAI/bge-small-en-v1.5 (384 dimensions)
  You requested:                       BAAI/bge-large-en-v1.5 (1024 dimensions)

  Embedding dimensions are incompatible. Options:
    1. Use the original model:   GBRAIN_MODEL=small gbrain <command>
    2. Re-initialize the DB:     rm ~/brain.db && gbrain init   (data will be lost)
```

### When It Does Not Fire

- Requested model matches stored model: proceed normally.
- `brain_config` table is missing (pre-v0.9.2 DB): emit a deprecation warning, treat as `small`, continue.
- Airgapped build with non-small model requested: warn only, continue with embedded BGE-small (see model-selection spec).

### Commands Affected

All commands that open brain.db: `query`, `search`, `get`, `put`, `import`, `export`, `stats`, `check`, `gaps`, `gap`, `graph`, `link`, `links`, `tags`, `timeline`, `validate`, `serve`.

Commands that do not open brain.db: `version`, `help`. These are not affected.
